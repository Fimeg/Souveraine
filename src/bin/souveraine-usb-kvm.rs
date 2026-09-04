use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use gadgetry_most_foul::function::custom::{Custom, Interface};
use gadgetry_most_foul::Class;
use gud_gadget::{DisplayMode, Event, GUD_PIXEL_FORMAT_RGB565};
use softbuffer::{Context as SoftContext, Surface};
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

#[derive(Debug, Parser)]
#[command(about = "Receive a GUD display into a Wayland KVM surface")]
struct Args {
    #[arg(long, default_value = "/run/souveraine-usb/ffs/gud")]
    ffs_dir: PathBuf,
    #[arg(long, default_value = "/run/souveraine-usb/keyboard")]
    keyboard: PathBuf,
    #[arg(long, default_value = "/run/souveraine-usb/pointer")]
    pointer: PathBuf,
    #[arg(long, default_value = "/run/souveraine-usb/gud.ready")]
    ready_file: PathBuf,
    #[arg(long, default_value_t = 1080)]
    width: u32,
    #[arg(long, default_value_t = 1920)]
    height: u32,
    #[arg(long, default_value = "USB KVM")]
    title: String,
}

#[derive(Debug)]
enum UserEvent {
    Frame,
    GudStopped(String),
}

struct Frame {
    width: u32,
    height: u32,
    rgb565: Vec<u8>,
    xrgb8888: Vec<u32>,
    generation: u64,
}

impl Frame {
    fn new(width: u32, height: u32) -> Self {
        let pixels = width as usize * height as usize;
        Self {
            width,
            height,
            rgb565: vec![0; pixels * 2],
            xrgb8888: vec![0xff10_1010; pixels],
            generation: 0,
        }
    }

    fn convert_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        let end_x = x.saturating_add(width).min(self.width);
        let end_y = y.saturating_add(height).min(self.height);
        for row in y..end_y {
            for column in x..end_x {
                let pixel = row as usize * self.width as usize + column as usize;
                let byte = pixel * 2;
                let raw = u16::from_le_bytes([self.rgb565[byte], self.rgb565[byte + 1]]);
                let r = u32::from((raw >> 11) & 0x1f) * 255 / 31;
                let g = u32::from((raw >> 5) & 0x3f) * 255 / 63;
                let b = u32::from(raw & 0x1f) * 255 / 31;
                self.xrgb8888[pixel] = 0xff00_0000 | (r << 16) | (g << 8) | b;
            }
        }
        self.generation = self.generation.wrapping_add(1);
    }
}

enum HidReport {
    Keyboard([u8; 8]),
    Pointer([u8; 4]),
}

fn spawn_hid_writer(keyboard: PathBuf, pointer: PathBuf) -> mpsc::Sender<HidReport> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for report in rx {
            let (path, bytes): (&Path, &[u8]) = match &report {
                HidReport::Keyboard(bytes) => (&keyboard, bytes),
                HidReport::Pointer(bytes) => (&pointer, bytes),
            };
            let result = OpenOptions::new()
                .write(true)
                .open(path)
                .and_then(|mut endpoint| endpoint.write_all(bytes));
            if let Err(error) = result {
                warn!(path = %path.display(), %error, "HID report was not delivered");
            }
        }
    });
    tx
}

fn display_mode(width: u32, height: u32) -> DisplayMode {
    let hsync_start = width.saturating_add(8);
    let hsync_end = hsync_start.saturating_add(8);
    let htotal = width.saturating_add(40);
    let vsync_start = height.saturating_add(3);
    let vsync_end = vsync_start.saturating_add(5);
    let vtotal = height.saturating_add(40);
    DisplayMode {
        clock: htotal.saturating_mul(vtotal).saturating_mul(60) / 1000,
        hdisplay: width as u16,
        hsync_start: hsync_start as u16,
        hsync_end: hsync_end as u16,
        htotal: htotal as u16,
        vdisplay: height as u16,
        vsync_start: vsync_start as u16,
        vsync_end: vsync_end as u16,
        vtotal: vtotal as u16,
        flags: 0,
    }
}

fn spawn_gud(
    ffs_dir: PathBuf,
    ready_file: PathBuf,
    frame: Arc<Mutex<Frame>>,
    proxy: EventLoopProxy<UserEvent>,
    running: Arc<AtomicBool>,
    width: u32,
    height: u32,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let (mut pixel_data, pixel_endpoint) = gud_gadget::PixelDataEndpoint::new();
            let mut gud = Custom::builder()
                .with_interface(
                    Interface::new(Class::vendor_specific(Class::VENDOR_SPECIFIC, 0), "GUD")
                        .with_endpoint(pixel_endpoint),
                )
                .existing(&ffs_dir)
                .with_context(|| format!("initializing {}", ffs_dir.display()))?;

            if let Some(parent) = ready_file.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&ready_file, b"ready\n")
                .with_context(|| format!("writing {}", ready_file.display()))?;
            info!(ffs = %ffs_dir.display(), "GUD FunctionFS responder ready");

            let mode = display_mode(width, height);
            while running.load(Ordering::Relaxed) {
                let Some(event) = gud.event_timeout(Duration::from_millis(100))? else {
                    continue;
                };
                let Some(event) = gud_gadget::event(event)? else {
                    continue;
                };
                match event {
                    Event::GetDescriptor(request) => {
                        request.send_descriptor(width, height, width, height)?;
                    }
                    Event::GetPixelFormats(request) => {
                        request.send_pixel_formats(&[GUD_PIXEL_FORMAT_RGB565])?;
                    }
                    Event::GetDisplayModes(request) => request
                        .send_modes(&[display_mode(mode.hdisplay.into(), mode.vdisplay.into())])?,
                    Event::Buffer(update) => {
                        let dirty = (update.x, update.y, update.width, update.height);
                        let mut frame = frame.lock().unwrap_or_else(|e| e.into_inner());
                        let pitch = frame.width as usize * 2;
                        pixel_data.recv_buffer(update, &mut frame.rgb565, pitch, 2)?;
                        frame.convert_rect(dirty.0, dirty.1, dirty.2, dirty.3);
                        let _ = proxy.send_event(UserEvent::Frame);
                    }
                }
            }
            Ok(())
        })();
        let _ = std::fs::remove_file(&ready_file);
        if let Err(error) = result {
            error!(%error, "GUD responder stopped");
            let _ = proxy.send_event(UserEvent::GudStopped(error.to_string()));
        }
    })
}

struct App {
    title: String,
    frame: Arc<Mutex<Frame>>,
    window: Option<Arc<Window>>,
    context: Option<SoftContext<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    rendered_generation: u64,
    rendered_size: Option<(u32, u32)>,
    hid: mpsc::Sender<HidReport>,
    keys: BTreeSet<u8>,
    modifiers: u8,
    pointer_buttons: u8,
    cursor: Option<PhysicalPosition<f64>>,
    touch: Option<TouchTrack>,
    running: Arc<AtomicBool>,
}

struct TouchTrack {
    id: u64,
    last: PhysicalPosition<f64>,
    travel: f64,
}

impl App {
    fn keyboard_report(&self) {
        let mut report = [0u8; 8];
        report[0] = self.modifiers;
        for (slot, key) in self.keys.iter().take(6).enumerate() {
            report[slot + 2] = *key;
        }
        let _ = self.hid.send(HidReport::Keyboard(report));
    }

    fn pointer_report(&self, x: i32, y: i32, wheel: i32) {
        let mut x = x;
        let mut y = y;
        let mut wheel = wheel;
        loop {
            let dx = x.clamp(-127, 127) as i8;
            let dy = y.clamp(-127, 127) as i8;
            let dw = wheel.clamp(-127, 127) as i8;
            let _ = self.hid.send(HidReport::Pointer([
                self.pointer_buttons,
                dx as u8,
                dy as u8,
                dw as u8,
            ]));
            x -= i32::from(dx);
            y -= i32::from(dy);
            wheel -= i32::from(dw);
            if x == 0 && y == 0 && wheel == 0 {
                break;
            }
        }
    }

    fn set_mouse_button(&mut self, button: MouseButton, pressed: bool) {
        let mask = match button {
            MouseButton::Left => 1,
            MouseButton::Right => 2,
            MouseButton::Middle => 4,
            _ => return,
        };
        if pressed {
            self.pointer_buttons |= mask;
        } else {
            self.pointer_buttons &= !mask;
        }
        self.pointer_report(0, 0, 0);
    }

    fn draw(&mut self) -> Result<()> {
        let Some(window) = &self.window else {
            return Ok(());
        };
        let Some(surface) = &mut self.surface else {
            return Ok(());
        };
        let size = window.inner_size();
        let width = NonZeroU32::new(size.width.max(1)).unwrap();
        let height = NonZeroU32::new(size.height.max(1)).unwrap();
        surface
            .resize(width, height)
            .map_err(|error| anyhow!(error.to_string()))?;

        let frame = self.frame.lock().unwrap_or_else(|e| e.into_inner());
        let size_key = (size.width, size.height);
        if frame.generation == self.rendered_generation && self.rendered_size == Some(size_key) {
            return Ok(());
        }
        let mut buffer = surface
            .buffer_mut()
            .map_err(|error| anyhow!(error.to_string()))?;
        for y in 0..size.height {
            let source_y = y as usize * frame.height as usize / size.height.max(1) as usize;
            for x in 0..size.width {
                let source_x = x as usize * frame.width as usize / size.width.max(1) as usize;
                buffer[y as usize * size.width as usize + x as usize] =
                    frame.xrgb8888[source_y * frame.width as usize + source_x];
            }
        }
        self.rendered_generation = frame.generation;
        self.rendered_size = Some(size_key);
        buffer
            .present()
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(())
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let frame = self.frame.lock().unwrap_or_else(|e| e.into_inner());
        let attributes = WindowAttributes::default()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(frame.width / 2, frame.height / 2));
        drop(frame);
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let window = Arc::new(window);
                match SoftContext::new(window.clone()).and_then(|context| {
                    Surface::new(&context, window.clone()).map(|surface| (context, surface))
                }) {
                    Ok((context, surface)) => {
                        self.context = Some(context);
                        self.surface = Some(surface);
                        window.request_redraw();
                        self.window = Some(window);
                    }
                    Err(error) => {
                        error!(%error, "could not create KVM drawing surface");
                        event_loop.exit();
                    }
                }
            }
            Err(error) => {
                error!(%error, "could not create KVM window");
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Frame => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            UserEvent::GudStopped(reason) => {
                error!(%reason, "closing KVM surface");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.draw() {
                    error!(%error, "KVM redraw failed");
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(_) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                if let Some(mask) = modifier_for(code) {
                    if pressed {
                        self.modifiers |= mask
                    } else {
                        self.modifiers &= !mask
                    }
                } else if let Some(key) = hid_key(code) {
                    if pressed {
                        self.keys.insert(key);
                    } else {
                        self.keys.remove(&key);
                    }
                }
                self.keyboard_report();
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(last) = self.cursor.replace(position) {
                    self.pointer_report(
                        (position.x - last.x).round() as i32,
                        (position.y - last.y).round() as i32,
                        0,
                    );
                }
            }
            WindowEvent::CursorLeft { .. } => self.cursor = None,
            WindowEvent::MouseInput { state, button, .. } => {
                self.set_mouse_button(button, state == ElementState::Pressed);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let wheel = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * 3.0).round() as i32,
                    MouseScrollDelta::PixelDelta(position) => (position.y / 24.0).round() as i32,
                };
                self.pointer_report(0, 0, wheel);
            }
            WindowEvent::Touch(touch) => match touch.phase {
                TouchPhase::Started => {
                    self.touch = Some(TouchTrack {
                        id: touch.id,
                        last: touch.location,
                        travel: 0.0,
                    });
                }
                TouchPhase::Moved => {
                    let Some(track) = &mut self.touch else { return };
                    if track.id != touch.id {
                        return;
                    }
                    let dx = touch.location.x - track.last.x;
                    let dy = touch.location.y - track.last.y;
                    track.last = touch.location;
                    track.travel += dx.abs() + dy.abs();
                    self.pointer_report(dx.round() as i32, dy.round() as i32, 0);
                }
                TouchPhase::Ended | TouchPhase::Cancelled => {
                    if self
                        .touch
                        .take()
                        .is_some_and(|track| track.id == touch.id && track.travel < 12.0)
                    {
                        self.pointer_buttons |= 1;
                        self.pointer_report(0, 0, 0);
                        self.pointer_buttons &= !1;
                        self.pointer_report(0, 0, 0);
                    }
                }
            },
            WindowEvent::Focused(false) => {
                self.keys.clear();
                self.modifiers = 0;
                self.pointer_buttons = 0;
                self.keyboard_report();
                self.pointer_report(0, 0, 0);
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.running.store(false, Ordering::Relaxed);
        self.keys.clear();
        self.modifiers = 0;
        self.pointer_buttons = 0;
        self.keyboard_report();
        self.pointer_report(0, 0, 0);
    }
}

fn modifier_for(code: KeyCode) -> Option<u8> {
    Some(match code {
        KeyCode::ControlLeft => 0x01,
        KeyCode::ShiftLeft => 0x02,
        KeyCode::AltLeft => 0x04,
        KeyCode::SuperLeft => 0x08,
        KeyCode::ControlRight => 0x10,
        KeyCode::ShiftRight => 0x20,
        KeyCode::AltRight => 0x40,
        KeyCode::SuperRight => 0x80,
        _ => return None,
    })
}

fn hid_key(code: KeyCode) -> Option<u8> {
    Some(match code {
        KeyCode::KeyA => 0x04,
        KeyCode::KeyB => 0x05,
        KeyCode::KeyC => 0x06,
        KeyCode::KeyD => 0x07,
        KeyCode::KeyE => 0x08,
        KeyCode::KeyF => 0x09,
        KeyCode::KeyG => 0x0a,
        KeyCode::KeyH => 0x0b,
        KeyCode::KeyI => 0x0c,
        KeyCode::KeyJ => 0x0d,
        KeyCode::KeyK => 0x0e,
        KeyCode::KeyL => 0x0f,
        KeyCode::KeyM => 0x10,
        KeyCode::KeyN => 0x11,
        KeyCode::KeyO => 0x12,
        KeyCode::KeyP => 0x13,
        KeyCode::KeyQ => 0x14,
        KeyCode::KeyR => 0x15,
        KeyCode::KeyS => 0x16,
        KeyCode::KeyT => 0x17,
        KeyCode::KeyU => 0x18,
        KeyCode::KeyV => 0x19,
        KeyCode::KeyW => 0x1a,
        KeyCode::KeyX => 0x1b,
        KeyCode::KeyY => 0x1c,
        KeyCode::KeyZ => 0x1d,
        KeyCode::Digit1 => 0x1e,
        KeyCode::Digit2 => 0x1f,
        KeyCode::Digit3 => 0x20,
        KeyCode::Digit4 => 0x21,
        KeyCode::Digit5 => 0x22,
        KeyCode::Digit6 => 0x23,
        KeyCode::Digit7 => 0x24,
        KeyCode::Digit8 => 0x25,
        KeyCode::Digit9 => 0x26,
        KeyCode::Digit0 => 0x27,
        KeyCode::Enter => 0x28,
        KeyCode::Escape => 0x29,
        KeyCode::Backspace => 0x2a,
        KeyCode::Tab => 0x2b,
        KeyCode::Space => 0x2c,
        KeyCode::Minus => 0x2d,
        KeyCode::Equal => 0x2e,
        KeyCode::BracketLeft => 0x2f,
        KeyCode::BracketRight => 0x30,
        KeyCode::Backslash => 0x31,
        KeyCode::Semicolon => 0x33,
        KeyCode::Quote => 0x34,
        KeyCode::Backquote => 0x35,
        KeyCode::Comma => 0x36,
        KeyCode::Period => 0x37,
        KeyCode::Slash => 0x38,
        KeyCode::CapsLock => 0x39,
        KeyCode::F1 => 0x3a,
        KeyCode::F2 => 0x3b,
        KeyCode::F3 => 0x3c,
        KeyCode::F4 => 0x3d,
        KeyCode::F5 => 0x3e,
        KeyCode::F6 => 0x3f,
        KeyCode::F7 => 0x40,
        KeyCode::F8 => 0x41,
        KeyCode::F9 => 0x42,
        KeyCode::F10 => 0x43,
        KeyCode::F11 => 0x44,
        KeyCode::F12 => 0x45,
        KeyCode::PrintScreen => 0x46,
        KeyCode::ScrollLock => 0x47,
        KeyCode::Pause => 0x48,
        KeyCode::Insert => 0x49,
        KeyCode::Home => 0x4a,
        KeyCode::PageUp => 0x4b,
        KeyCode::Delete => 0x4c,
        KeyCode::End => 0x4d,
        KeyCode::PageDown => 0x4e,
        KeyCode::ArrowRight => 0x4f,
        KeyCode::ArrowLeft => 0x50,
        KeyCode::ArrowDown => 0x51,
        KeyCode::ArrowUp => 0x52,
        _ => return None,
    })
}

fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    anyhow::ensure!(
        args.width <= u16::MAX.into() && args.height <= u16::MAX.into(),
        "display is too large for GUD mode fields"
    );

    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    let frame = Arc::new(Mutex::new(Frame::new(args.width, args.height)));
    let running = Arc::new(AtomicBool::new(true));
    let gud = spawn_gud(
        args.ffs_dir,
        args.ready_file,
        frame.clone(),
        event_loop.create_proxy(),
        running.clone(),
        args.width,
        args.height,
    );
    let mut app = App {
        title: args.title,
        frame,
        window: None,
        context: None,
        surface: None,
        rendered_generation: u64::MAX,
        rendered_size: None,
        hid: spawn_hid_writer(args.keyboard, args.pointer),
        keys: BTreeSet::new(),
        modifiers: 0,
        pointer_buttons: 0,
        cursor: None,
        touch: None,
        running,
    };
    event_loop.run_app(&mut app)?;
    app.running.store(false, Ordering::Relaxed);
    let _ = gud.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_dirty_rect_becomes_xrgb8888() {
        let mut frame = Frame::new(2, 1);
        frame.rgb565 = [0x00, 0xf8, 0xe0, 0x07].to_vec();
        frame.convert_rect(0, 0, 2, 1);
        assert_eq!(frame.xrgb8888, [0xffff_0000, 0xff00_ff00]);
        assert_eq!(frame.generation, 1);
    }

    #[test]
    fn physical_keys_use_usb_hid_usage_ids() {
        assert_eq!(hid_key(KeyCode::KeyA), Some(0x04));
        assert_eq!(hid_key(KeyCode::Digit0), Some(0x27));
        assert_eq!(hid_key(KeyCode::F12), Some(0x45));
        assert_eq!(hid_key(KeyCode::Delete), Some(0x4c));
        assert_eq!(modifier_for(KeyCode::ControlLeft), Some(0x01));
        assert_eq!(modifier_for(KeyCode::AltRight), Some(0x40));
    }
}
