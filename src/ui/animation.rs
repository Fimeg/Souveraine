use std::time::Duration;
use tokio::time::Instant;
use std::io::{stdout, Write};

/// Animation system for sexy terminal effects
pub struct Animator {
    start_time: Instant,
}

impl Animator {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
        }
    }
    
    /// Breathing glow effect (sine wave)
    /// Returns intensity 0.0-1.0 based on time
    pub fn breathe(&self, speed_ms: u64) -> f32 {
        let elapsed = self.start_time.elapsed().as_millis() as f64;
        let cycle = (elapsed / speed_ms as f64) * 2.0 * std::f64::consts::PI;
        ((cycle.sin() + 1.0) / 2.0) as f32
    }
    
    /// Pulsing between two colors
    pub fn pulse_color(&self, color1: (u8, u8, u8), color2: (u8, u8, u8), speed_ms: u64) -> (u8, u8, u8) {
        let t = self.breathe(speed_ms);
        (
            (color1.0 as f32 * (1.0 - t) + color2.0 as f32 * t) as u8,
            (color1.1 as f32 * (1.0 - t) + color2.1 as f32 * t) as u8,
            (color1.2 as f32 * (1.0 - t) + color2.2 as f32 * t) as u8,
        )
    }
}

/// Typing animation for responses
pub async fn typewrite(text: &str, wpm: u64) {
    // 120 WPM = ~10 chars/sec = 100ms per char
    let delay_ms = 60000 / (wpm * 5); // chars per word ~5
    
    for ch in text.chars() {
        print!("{}", ch);
        stdout().flush().unwrap();
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

/// Gradient text across a string
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

/// Smooth spinner using braille patterns
pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Wave animation for progress
pub const WAVE: &[&str] = &["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█", "▇", "▆", "▅", "▄", "▃", "▂"];

/// Persona signature colors
pub mod colors {
    use ratatui::style::Color;

    // Ani: Warm, gentle, inviting
    pub const ANI_PRIMARY: Color = Color::Rgb(255, 140, 66);    // Warm orange
    pub const ANI_SECONDARY: Color = Color::Rgb(255, 200, 150); // Light peach
    pub const ANI_DIM: Color = Color::Rgb(180, 120, 80);        // Muted brown-orange

    // Jean-Luc: Cool, precise, technical
    pub const JEANLUC_PRIMARY: Color = Color::Rgb(66, 133, 244);   // Blue
    pub const JEANLUC_SECONDARY: Color = Color::Rgb(150, 200, 255); // Light blue
    pub const JEANLUC_DIM: Color = Color::Rgb(80, 100, 140);       // Steel

    // Eione: Creative, flowing, purple
    pub const EIONE_PRIMARY: Color = Color::Rgb(155, 89, 182);    // Purple
    pub const EIONE_SECONDARY: Color = Color::Rgb(200, 150, 220);  // Light purple
    pub const EIONE_DIM: Color = Color::Rgb(120, 80, 140);         // Muted

    // Subconscious surfacing: Gray, dim, italic
    pub const SUBCONSCIOUS: Color = Color::Rgb(128, 128, 128);
}

/// Get breathing color for active element
pub fn breathing_color(base: (u8, u8, u8), intensity: f32) -> (u8, u8, u8) {
    // Shift brightness slightly
    let factor = 0.8 + (intensity * 0.4); // 0.8 to 1.2
    (
        (base.0 as f32 * factor).min(255.0) as u8,
        (base.1 as f32 * factor).min(255.0) as u8,
        (base.2 as f32 * factor).min(255.0) as u8,
    )
}

/// HSL to RGB conversion for gradients
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    
    let (r1, g1, b1) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_gradient() {
        let text = "Ani";
        let result = gradient(text, 30.0); // Orange-ish start
        assert!(result.contains("\x1b[38;2;")); // Contains ANSI color code
    }
    
    #[test]
    fn test_breathe() {
        let animator = Animator::new();
        let val = animator.breathe(1000);
        assert!(val >= 0.0 && val <= 1.0);
    }
}
