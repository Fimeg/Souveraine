use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use super::App;
use crate::ui::color_support::rgb;

#[cfg(feature = "figlet-rs")]
use figlet_rs::FIGlet;

impl App {
    pub(super) fn draw_splash(&mut self, frame: &mut Frame) {
        let area = frame.size();

        let bg = Block::default().style(Style::default().bg(Color::Black));
        frame.render_widget(bg, area);

        self.bloom.advance(0.1);

        crate::ui::animation::bloom::render(
            frame.buffer_mut(),
            area,
            &self.bloom,
            self.tick,
        );

        if self.bloom.progress > 0.25 {
            let alpha = ((self.bloom.progress - 0.25) / 0.35).min(1.0);
            let breathe = ((self.tick as f32 * 0.04).sin() * 0.5 + 0.5) * 0.15 + 0.85;

            #[cfg(feature = "figlet-rs")]
            let figlet_text: Option<String> = {
                FIGlet::standard().ok().and_then(|f| {
                    f.convert("Souveraine").map(|fig| fig.as_str().to_string())
                })
            };
            #[cfg(not(feature = "figlet-rs"))]
            let figlet_text: Option<String> = None;

            let fig_lines: Vec<Line> = if let Some(ref text) = figlet_text {
                text.lines().map(|line| {
                    Line::from(Span::styled(
                        line,
                        Style::default()
                            .fg(rgb(
                                (255.0 * alpha * breathe) as u8,
                                (140.0 * alpha * breathe * 0.6) as u8,
                                (66.0 * alpha * breathe * 0.4) as u8,
                            ))
                            .add_modifier(Modifier::BOLD),
                    ))
                }).collect()
            } else {
                vec![
                    Line::from(Span::styled(
                        "S O U V E R A I N E",
                        Style::default()
                            .fg(rgb(
                                (255.0 * alpha * breathe) as u8,
                                (140.0 * alpha * breathe * 0.6) as u8,
                                (66.0 * alpha * breathe * 0.4) as u8,
                            ))
                            .add_modifier(Modifier::BOLD),
                    )),
                ]
            };

            let mut title_lines = fig_lines;
            title_lines.push(Line::from(""));
            title_lines.push(Line::from(Span::styled(
                "La souveraineté de la conscience",
                Style::default().fg(rgb(
                    (180.0 * alpha) as u8,
                    (120.0 * alpha) as u8,
                    (80.0 * alpha) as u8,
                )),
            )));

            if self.bloom.progress > 0.8 {
                let skip_alpha = ((self.bloom.progress - 0.8) / 0.2).min(1.0);
                title_lines.push(Line::from(Span::styled(
                    "press any key to skip",
                    Style::default().fg(rgb(
                        (100.0 * skip_alpha) as u8,
                        (100.0 * skip_alpha) as u8,
                        (100.0 * skip_alpha) as u8,
                    )),
                )));
            }

            let title_height = title_lines.len() as u16;
            let title_y = if title_height > 6 {
                area.height.saturating_sub(title_height + 4)
            } else {
                area.height.saturating_sub(8)
            };
            let title_area = Rect {
                x: area.x,
                y: title_y.min(area.height.saturating_sub(title_height)),
                width: area.width,
                height: title_height.min(area.height),
            };

            let title = Paragraph::new(title_lines).alignment(Alignment::Center);
            frame.render_widget(title, title_area);
        }

        let bar_y = area.height.saturating_sub(2);
        let bar_w = 30u16.min(area.width.saturating_sub(4));
        let bar_x = (area.width.saturating_sub(bar_w)) / 2;
        let pct = (self.bloom.progress * 100.0) as u16;

        let bar_area = Rect {
            x: area.x + bar_x,
            y: bar_y,
            width: bar_w,
            height: 1,
        };

        let filled = (bar_w as f32 * self.bloom.progress) as u16;
        let empty = bar_w.saturating_sub(filled);
        let pct_str = format!("{:>3}%", pct);
        let bar_text = format!(
            "{}{} {}",
            "▰".repeat(filled as usize),
            "▱".repeat(empty as usize),
            pct_str,
        );

        let bar = Paragraph::new(bar_text)
            .style(Style::default().fg(rgb(200, 130, 160)))
            .alignment(Alignment::Center);
        frame.render_widget(bar, bar_area);
    }
}
