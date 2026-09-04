//! ext-session-lock-v1 client: acquire the compositor lock, render the
//! holding surface, and support the abandoned-lock handoff.
//!
//! **This daemon locks. It does not unlock.** There is no PIN pad and no PAM
//! conversation here, deliberately, removed 2026-07-29.
//!
//! What was here was a second lockscreen: it authenticated, called
//! `unlock_and_destroy()`, and returned `Unlocked` — and then the shell, whose
//! `GlobalStates.screenLocked` is a separate bool, re-registered, was told
//! `must_lock=true`, and locked again. The protocol has a lock directive and
//! no unlock directive, so the fallback could open the compositor's lock and
//! had no way to tell the session it had. Casey hit it exactly that way: "it
//! logs in, and then qs says Locked still."
//!
//! It also was not a last resort in practice. It is raised whenever
//! `shell_alive` is false, and that flag lied for 90 minutes on 2026-07-29
//! because a scene reload's re-registration was refused and the shell gave up
//! (fixed in 45fbbea). So the "fallback" was the first thing reached, on a bad
//! signal, to do a job it could not finish. That is the fallback shape the
//! doctrine forbids: a second implementation of a job, on an unreliable
//! trigger, with different and worse behaviour than the first.
//!
//! What remains is the part that was never a fallback: something must hold the
//! session locked before any shell exists, or there is a window where the panel
//! is live and unlocked (`LOCK-DPMS-LESSONS.md` §1). A locked session with no
//! shell now stays locked and says so. Recovery is the shell coming back, or
//! the user rebooting — both louder and more honest than a keypad that cannot
//! hand control back.
//!
//! One lock session = one Wayland connection. Releasing to the shell is
//! `SessionOutcome::Released`: the connection is dropped WITHOUT unlocking,
//! which leaves the compositor holding the session locked until the shell's
//! own lock takes over (`misc:allow_session_lock_restore`).

use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::mpsc::{Receiver, Sender};

use anyhow::{bail, Context, Result};
use tracing::info;
use wayland_client::{
    delegate_noop,
    protocol::{
        wl_buffer::{self, WlBuffer},
        wl_compositor::WlCompositor,
        wl_keyboard::{self, WlKeyboard},
        wl_output::WlOutput,
        wl_pointer::{self, WlPointer},
        wl_registry::{self, WlRegistry},
        wl_seat::{self, WlSeat},
        wl_shm::{self, WlShm},
        wl_shm_pool::WlShmPool,
        wl_surface::WlSurface,
        wl_touch::{self, WlTouch},
    },
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self, ExtSessionLockV1},
};

use crate::sessiond::draw::{self, Mood, Scene};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    /// Told to hand off: connection dropped, session still locked.
    Released,
    /// The compositor refused or revoked the lock (another locker active).
    Denied,
}

/// Messages into a running lock session. External commands and the auth
/// worker's verdict share one channel; the pipe write wakes the poll loop.
pub enum Msg {
    Release,
}

pub struct LockController {
    tx: Sender<Msg>,
    wake: OwnedFd,
}

impl LockController {
    pub fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        // A single byte; the loop drains the pipe. Failure means the loop
        // is gone, which the caller observes via the join handle anyway.
        let _ = nix_write(&self.wake, b"x");
    }

    pub fn try_clone(&self) -> Result<LockController> {
        Ok(LockController {
            tx: self.tx.clone(),
            wake: self.wake.try_clone()?,
        })
    }
}

fn nix_write(fd: &OwnedFd, buf: &[u8]) -> std::io::Result<usize> {
    let n = unsafe { libc::write(fd.as_raw_fd(), buf.as_ptr() as *const _, buf.len()) };
    if n < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

pub fn channel() -> Result<(LockController, Receiver<Msg>, OwnedFd)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut fds = [0i32; 2];
    // SAFETY: pipe2 fills two fds on success.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    if rc != 0 {
        bail!("pipe2 failed: {}", std::io::Error::last_os_error());
    }
    // SAFETY: fresh fds owned exactly once each.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    Ok((LockController { tx, wake: write }, rx, read))
}

struct SurfaceCtx {
    surface: WlSurface,
    lock_surface: ExtSessionLockSurfaceV1,
    output_name: u32,
    width: i32,
    height: i32,
    configured: bool,
}

#[derive(Default)]
struct SeatDevices {
    touch: Option<WlTouch>,
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
}

struct LockState {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    lock_mgr: Option<ExtSessionLockManagerV1>,
    lock: Option<ExtSessionLockV1>,
    locked: bool,
    finished: bool,
    outputs: Vec<(u32, WlOutput)>,
    surfaces: Vec<SurfaceCtx>,
    seats: HashMap<u32, (WlSeat, SeatDevices)>,
    mood: Mood,
    dirty: bool,
    pointer_pos: (f64, f64),
    pointer_surface: Option<u32>, // protocol id of the entered wl_surface
}

impl LockState {
    fn new() -> Self {
        Self {
            compositor: None,
            shm: None,
            lock_mgr: None,
            lock: None,
            locked: false,
            finished: false,
            outputs: Vec::new(),
            surfaces: Vec::new(),
            seats: HashMap::new(),
            mood: Mood::Holding,
            dirty: false,
            pointer_pos: (0.0, 0.0),
            pointer_surface: None,
        }
    }

    /// Input reaches a locked session with no shell and goes nowhere, on
    /// purpose. There is nothing to type into: this daemon cannot unlock.
    /// Kept as a no-op rather than deleted so the surface still repaints on
    /// touch — a screen that ignores you entirely is indistinguishable from a
    /// hung one, and the point is to be legible, not silent.
    fn press(&mut self) {
        self.dirty = true;
    }

    fn surface_size_by_proto_id(&self, id: u32) -> Option<(i32, i32)> {
        self.surfaces
            .iter()
            .find(|s| s.surface.id().protocol_id() == id)
            .map(|s| (s.width, s.height))
    }
}

/// Run one full lock session on the current thread. Blocks until release or
/// denial. There is no unlock outcome — this daemon does not unlock.
pub fn run(rx: Receiver<Msg>, wake_read: OwnedFd) -> Result<SessionOutcome> {
    let conn = Connection::connect_to_env().context("connecting to Wayland display")?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let display = conn.display();
    display.get_registry(&qh, ());

    let mut state = LockState::new();
    queue.roundtrip(&mut state).context("initial roundtrip")?;

    let (Some(_), Some(_)) = (&state.compositor, &state.shm) else {
        bail!("compositor is missing wl_compositor/wl_shm");
    };
    let Some(mgr) = &state.lock_mgr else {
        bail!("compositor does not advertise ext-session-lock-v1");
    };

    let lock = mgr.lock(&qh, ());
    state.lock = Some(lock);
    queue.roundtrip(&mut state).context("lock roundtrip")?;

    // Wait for the compositor's verdict: exactly one of locked/finished is
    // guaranteed by the protocol, but NOT promptly. At boot Hyprland defers
    // the ack until it is the active DRM session — and the splash can hold
    // DRM for many seconds before its watchdog fails open. One dispatch was
    // far too impatient (observed: gave up 0.7s into boot, and the whole
    // lock-before-shell guarantee silently degraded to the legacy path).
    // Wait up to 60s, dispatching as events arrive.
    let verdict_deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !state.locked && !state.finished {
        if std::time::Instant::now() >= verdict_deadline {
            bail!("compositor never acknowledged the session lock (60s)");
        }

        // Draw *while* waiting, not after. A spec-honouring compositor sends
        // `locked` only once a lock frame is on every output, so the surfaces
        // have to exist and be painted before the verdict can arrive — see
        // `ensure_surfaces`. Doing this after the wait deadlocks against
        // exactly the compositor the protocol describes.
        ensure_surfaces(&mut state, &qh);
        if state.dirty {
            redraw_all(&mut state, &qh)?;
            state.dirty = false;
        }

        // The handoff must not wait out the verdict. This loop used to be deaf
        // to the control channel for its whole 60s budget, so a shell that
        // registered during it got `lock session did not release in time`
        // (server.rs waits 5s) and then asked the compositor for a lock this
        // thread was still holding — TASK-48's crash, from the daemon side.
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Release => {
                    info!("releasing session lock to shell (before the verdict)");
                    return Ok(SessionOutcome::Released);
                }
            }
        }

        queue
            .blocking_dispatch(&mut state)
            .context("waiting for locked")?;
    }
    if state.finished {
        return Ok(SessionOutcome::Denied);
    }
    info!("session lock acquired");

    loop {
        if state.finished {
            info!("lock revoked by compositor (another locker?)");
            return Ok(SessionOutcome::Denied);
        }

        ensure_surfaces(&mut state, &qh);

        if state.dirty {
            redraw_all(&mut state, &qh)?;
            state.dirty = false;
        }

        // Drain control messages before sleeping. Release is the only one
        // left and it returns, so there is nothing to loop back for.
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Release => {
                    info!("releasing session lock to shell (connection drop, stays locked)");
                    return Ok(SessionOutcome::Released);
                }
            }
        }

        queue.flush().context("flush")?;
        if queue.dispatch_pending(&mut state).context("dispatch")? > 0 {
            continue;
        }

        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let wl_fd = guard.connection_fd().as_raw_fd();
        let mut fds = [
            libc::pollfd {
                fd: wl_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake_read.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: fds array outlives the call.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err).context("poll");
        }
        if fds[1].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 64];
            // Drain the wake pipe; messages are picked up next iteration.
            loop {
                let n = unsafe {
                    libc::read(wake_read.as_raw_fd(), buf.as_mut_ptr() as *mut _, buf.len())
                };
                if n <= 0 {
                    break;
                }
            }
        }
        if fds[0].revents & libc::POLLIN != 0 {
            match guard.read() {
                Ok(_) => {
                    queue
                        .dispatch_pending(&mut state)
                        .context("dispatch after read")?;
                }
                Err(wayland_client::backend::WaylandError::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e).context("reading wayland events"),
            }
        }
        // Guard dropped here if we didn't read — that cancels cleanly.
    }
}

fn ensure_surfaces(state: &mut LockState, qh: &QueueHandle<LockState>) {
    // Gated on the lock *object*, not on the `locked` event, and the
    // difference is a deadlock.
    //
    // ext-session-lock-v1 is explicit: "The locked event must not be sent
    // until a new 'locked' frame has been presented on all outputs." So a
    // compositor that honours the spec is waiting for exactly the surfaces
    // this function refuses to create until it has heard from the compositor.
    // Neither side can move.
    //
    // Hyprland hides it by acking `locked` before any lock surface exists,
    // which is why this stood for months. Measured against viewtop on blueline
    // 2026-08-02: sessiond sat in `blocking_dispatch` waiting for a verdict
    // that could not arrive, the shell's `shell_ready` handoff timed out
    // behind it, and the session crash-looped every eleven seconds.
    let (Some(compositor), Some(lock)) = (&state.compositor, &state.lock) else {
        return;
    };
    let existing: Vec<u32> = state.surfaces.iter().map(|s| s.output_name).collect();
    let mut created = Vec::new();
    for (name, output) in &state.outputs {
        if existing.contains(name) {
            continue;
        }
        let surface = compositor.create_surface(qh, ());
        let lock_surface = lock.get_lock_surface(&surface, output, qh, ());
        created.push(SurfaceCtx {
            surface,
            lock_surface,
            output_name: *name,
            width: 0,
            height: 0,
            configured: false,
        });
    }
    state.surfaces.extend(created);
}

fn redraw_all(state: &mut LockState, qh: &QueueHandle<LockState>) -> Result<()> {
    let scene = Scene { mood: state.mood };
    let Some(shm) = state.shm.clone() else {
        return Ok(());
    };
    for ctx in &mut state.surfaces {
        if !ctx.configured || ctx.width <= 0 || ctx.height <= 0 {
            continue;
        }
        draw_frame(&shm, ctx, &scene, qh)?;
    }
    Ok(())
}

fn draw_frame(
    shm: &WlShm,
    ctx: &SurfaceCtx,
    scene: &Scene,
    qh: &QueueHandle<LockState>,
) -> Result<()> {
    let (w, h) = (ctx.width, ctx.height);
    let stride = w * 4;
    let size = (stride * h) as usize;

    // SAFETY: fresh memfd, sized before map, unmapped after the pixel write.
    let fd = unsafe {
        let raw = libc::memfd_create(b"sessiond-frame\0".as_ptr() as *const _, libc::MFD_CLOEXEC);
        if raw < 0 {
            bail!("memfd_create: {}", std::io::Error::last_os_error());
        }
        let fd = OwnedFd::from_raw_fd(raw);
        if libc::ftruncate(fd.as_raw_fd(), size as libc::off_t) != 0 {
            bail!("ftruncate: {}", std::io::Error::last_os_error());
        }
        fd
    };
    // SAFETY: mapping the region we just sized; checked for MAP_FAILED.
    unsafe {
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd.as_raw_fd(),
            0,
        );
        if ptr == libc::MAP_FAILED {
            bail!("mmap: {}", std::io::Error::last_os_error());
        }
        let pixels = std::slice::from_raw_parts_mut(ptr as *mut u32, (w * h) as usize);
        draw::render(pixels, w, h, scene);
        libc::munmap(ptr, size);
    }

    let pool = shm.create_pool(fd.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(0, w, h, stride, wl_shm::Format::Argb8888, qh, ());
    // The buffer keeps the pool's storage alive; the pool object itself can go.
    pool.destroy();

    ctx.surface.attach(Some(&buffer), 0, 0);
    ctx.surface.damage_buffer(0, 0, w, h);
    ctx.surface.commit();
    Ok(())
}

// ---- Dispatch impls ----

impl Dispatch<WlRegistry, ()> for LockState {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_compositor" => {
                    state.compositor =
                        Some(registry.bind::<WlCompositor, _, _>(name, version.min(4), qh, ()));
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind::<WlShm, _, _>(name, 1, qh, ()));
                }
                "wl_seat" => {
                    let seat = registry.bind::<WlSeat, _, _>(name, version.min(5), qh, ());
                    state.seats.insert(name, (seat, SeatDevices::default()));
                }
                "wl_output" => {
                    let output = registry.bind::<WlOutput, _, _>(name, version.min(2), qh, ());
                    state.outputs.push((name, output));
                }
                "ext_session_lock_manager_v1" => {
                    state.lock_mgr =
                        Some(registry.bind::<ExtSessionLockManagerV1, _, _>(name, 1, qh, ()));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                state.outputs.retain(|(n, _)| *n != name);
                state.surfaces.retain(|s| s.output_name != name);
                state.seats.remove(&name);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtSessionLockV1, ()> for LockState {
    fn event(
        state: &mut Self,
        _: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => state.locked = true,
            ext_session_lock_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, ()> for LockState {
    fn event(
        state: &mut Self,
        surface: &ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            surface.ack_configure(serial);
            if let Some(ctx) = state
                .surfaces
                .iter_mut()
                .find(|s| s.lock_surface.id() == surface.id())
            {
                ctx.width = width as i32;
                ctx.height = height as i32;
                ctx.configured = true;
                state.dirty = true;
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for LockState {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            let entry = state.seats.values_mut().find(|(s, _)| s.id() == seat.id());
            let Some((seat, devices)) = entry else { return };
            if caps.contains(wl_seat::Capability::Touch) && devices.touch.is_none() {
                devices.touch = Some(seat.get_touch(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Pointer) && devices.pointer.is_none() {
                devices.pointer = Some(seat.get_pointer(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Keyboard) && devices.keyboard.is_none() {
                devices.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<WlTouch, ()> for LockState {
    fn event(
        state: &mut Self,
        _: &WlTouch,
        event: wl_touch::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_touch::Event::Down { surface, x, y, .. } => {
                let _ = (surface, x, y);
                state.press();
            }
            wl_touch::Event::Up { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, ()> for LockState {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_surface = Some(surface.id().protocol_id());
                state.pointer_pos = (surface_x, surface_y);
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_surface = None;
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_pos = (surface_x, surface_y);
            }
            wl_pointer::Event::Button {
                state: WEnum::Value(st),
                ..
            } => match st {
                wl_pointer::ButtonState::Pressed => state.press(),
                wl_pointer::ButtonState::Released => {}
                _ => {}
            },
            _ => {}
        }
    }
}

impl Dispatch<WlKeyboard, ()> for LockState {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(st),
                ..
            } => match st {
                wl_keyboard::KeyState::Pressed => {
                    let _ = key;
                    state.press();
                }
                wl_keyboard::KeyState::Released => {}
                _ => {}
            },
            // Keymap carries an fd we must not leak; OwnedFd drops it here.
            wl_keyboard::Event::Keymap { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<WlBuffer, ()> for LockState {
    fn event(
        _: &mut Self,
        buffer: &WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            buffer.destroy();
        }
    }
}

delegate_noop!(LockState: ignore WlCompositor);
delegate_noop!(LockState: ignore WlSurface);
delegate_noop!(LockState: ignore WlShm);
delegate_noop!(LockState: ignore WlShmPool);
delegate_noop!(LockState: ignore WlOutput);
delegate_noop!(LockState: ignore ExtSessionLockManagerV1);
