//! Voice level meter — a single-row block-character waveform widget.
//!
//! Renders `▁▂▃▄▅▆▇█` blocks proportional to the current mic peak level.
//! Color follows the atmosphere's primary accent. Fixed-height single line;
//! placed below the portrait in the Presence layout.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

/// Block characters for the waveform — 8 levels, index 0 = quietest.
const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Exported level-to-char mapping for the rolling waveform in the presence pane.
pub const LEVEL_CHARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// A single-row waveform bar driven by a peak level.
///
/// The bar fills `area.width` cells; each cell shows the block character
/// that maps to the current level. A `threshold` parameter dims cells
/// beyond the active portion for a more tactile "bar" feel.
pub struct VoiceMeter {
    /// Current level 0.0..1.0
    pub level: f32,
    /// Accent color — set from `Atmosphere::primary()`.
    pub color: Color,
}

impl VoiceMeter {
    pub fn new(level: f32, color: Color) -> Self {
        Self { level: level.clamp(0.0, 1.0), color }
    }
}

impl Widget for VoiceMeter {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let y = area.y;
        let block_idx = (self.level * 7.0).round() as usize;
        let block_char = BLOCKS[block_idx.min(7)];

        // Draw active bar up to `active_cols` cells, dim the rest.
        let active_cols = ((self.level * area.width as f32).round() as u16).min(area.width);

        for x in area.x..area.x + area.width {
            let cell = buf.get_mut(x, y);
            cell.set_char(block_char);
            if x < area.x + active_cols {
                cell.set_style(Style::default().fg(self.color));
            } else {
                // Dim fill — faint block at lowest level so the bar has extent.
                cell.set_char('▁');
                cell.set_style(Style::default().fg(Color::Rgb(40, 40, 55)));
            }
        }
    }
}
