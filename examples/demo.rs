//! Demo of sexy terminal UI effects
//! Run with: cargo run --example demo

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute,
    style::{Color, ResetColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use std::io::{self, Write};
use std::time::{Duration, Instant};
use tokio::time::sleep;

pub struct Animator {
    start_time: Instant,
}

impl Animator {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
        }
    }

    pub fn breathe(&self, speed_ms: u64) -> f32 {
        let elapsed = self.start_time.elapsed().as_millis() as f64;
        let cycle = (elapsed / speed_ms as f64) * 2.0 * std::f64::consts::PI;
        ((cycle.sin() + 1.0) / 2.0) as f32
    }
}

pub async fn typewrite(text: &str, wpm: u64) {
    let delay_ms = 60000 / (wpm * 5);
    for ch in text.chars() {
        print!("{}", ch);
        io::stdout().flush().unwrap();
        sleep(Duration::from_millis(delay_ms)).await;
    }
}

pub fn gradient(text: &str, start_hue: f32) -> String {
    text.chars()
        .enumerate()
        .map(|(i, ch)| {
            let hue = (start_hue + i as f32 * 3.0) % 360.0;
            let (r, g, b) = hsl_to_rgb(hue, 0.8, 0.6);
            format!("\x1b[38;2;{};{};{}m{}\x1b[0m", r, g, b, ch)
        })
        .collect()
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = match h {
        _ if h < 60.0 => (c, x, 0.0),
        _ if h < 120.0 => (x, c, 0.0),
        _ if h < 180.0 => (0.0, c, x),
        _ if h < 240.0 => (0.0, x, c),
        _ if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

pub fn breathing_color(base: (u8, u8, u8), intensity: f32) -> (u8, u8, u8) {
    let factor = 0.8 + (intensity * 0.4);
    (
        (base.0 as f32 * factor).min(255.0) as u8,
        (base.1 as f32 * factor).min(255.0) as u8,
        (base.2 as f32 * factor).min(255.0) as u8,
    )
}

pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const WAVE: &[&str] = &[
    "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█", "▇", "▆", "▅", "▄", "▃", "▂",
];

#[tokio::main]
async fn main() {
    let mut stdout = io::stdout();

    execute!(stdout, Hide, Clear(ClearType::All)).unwrap();

    let animator = Animator::new();

    // Demo 1: Gradient
    execute!(stdout, MoveTo(5, 2)).unwrap();
    println!("{}", gradient("✨ Souveraine ✨", 30.0));

    // Demo 2: Typing
    execute!(stdout, MoveTo(5, 4)).unwrap();
    print!("Ani: ");
    io::stdout().flush().unwrap();
    typewrite("Color and pop!", 100).await;
    println!();

    // Demo 3: Breathing heart
    execute!(stdout, MoveTo(5, 6)).unwrap();
    print!("Breathing: ");
    for _ in 0..20 {
        let breathe = animator.breathe(500);
        let (r, g, b) = breathing_color((255, 100, 200), breathe);
        execute!(stdout, SetForegroundColor(Color::Rgb { r, g, b })).unwrap();
        print!("♥");
        io::stdout().flush().unwrap();
        sleep(Duration::from_millis(50)).await;
        execute!(stdout, MoveTo(16, 6)).unwrap();
    }

    execute!(stdout, ResetColor).unwrap();
    println!();

    // Demo 4: Spinner
    execute!(stdout, MoveTo(5, 8)).unwrap();
    print!("Loading: ");
    for i in 0..20 {
        execute!(
            stdout,
            SetForegroundColor(Color::Rgb {
                r: 100,
                g: 200,
                b: 255
            })
        )
        .unwrap();
        print!("{}", SPINNER[i % SPINNER.len()]);
        io::stdout().flush().unwrap();
        sleep(Duration::from_millis(80)).await;
        execute!(stdout, MoveTo(14, 8)).unwrap();
    }

    execute!(stdout, ResetColor).unwrap();
    println!(" ✓");

    // Cleanup
    execute!(stdout, Show, ResetColor).unwrap();
    println!("\n✨ Demo complete! ✨");
}
