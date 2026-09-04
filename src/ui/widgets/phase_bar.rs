//! Phase bar widget — spinner, turn status, elapsed time, queued count.

use tuie::prelude::*;

/// Spinner animation frames.
pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// What the agent is currently doing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PhaseKind {
    Idle,
    Thinking,
    RunningTool,
    Streaming,
    Interrupted,
    Subconscious,
}

/// Status bar showing the current turn phase.
pub struct PhaseBar {
    text: Box<Text>,
}

impl DelegateWidget for PhaseBar {
    tuie::delegate_widget!(text);
    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl PhaseBar {
    pub fn new() -> Box<Self> {
        Box::new(Self { text: Text::new() })
    }

    /// Set the phase display from the current state.
    #[allow(clippy::too_many_arguments)] // phase styling params passed individually
    pub fn set_phase(
        &mut self,
        kind: PhaseKind,
        spinner_idx: usize,
        elapsed_secs: u64,
        tool_count: u32,
        queued: usize,
        quiet_secs: u64,
        style: Style,
        dim_style: Style,
    ) {
        let glyph = SPINNER[spinner_idx % SPINNER.len()];
        let tool_label: String;
        let (glyph_char, label): (&str, &str) = match kind {
            PhaseKind::Idle | PhaseKind::Thinking => (glyph, "Thinking"),
            PhaseKind::RunningTool => {
                tool_label = format!("Running tool · {} tools used", tool_count.max(1));
                (glyph, tool_label.as_str())
            }
            PhaseKind::Streaming => (glyph, "Streaming"),
            PhaseKind::Interrupted => ("×", "Interrupted"),
            PhaseKind::Subconscious => (glyph, "Subconscious"),
        };

        let mut content = StyledString::new();
        let line = if quiet_secs >= 5 {
            let wait = if quiet_secs >= 120 {
                format!("still waiting {}s...", quiet_secs)
            } else {
                format!("waiting {}s...", quiet_secs)
            };
            format!(" {glyph_char} {label}  ·  {}s  ·  {wait}", elapsed_secs)
        } else {
            format!(" {glyph_char} {label}  ·  {}s", elapsed_secs)
        };

        content.push_str(&line);
        let len = content.as_ref().len();
        content.style_range(0..len, |s| *s = style);

        if queued > 0 {
            let extra = format!("  ·  {} queued", queued);
            content.push_str(&extra);
            let q_start = len;
            content.style_range(q_start..content.as_ref().len(), |s| *s = dim_style.italic());
        }

        self.text.set_content(content);
        self.text.dirty_layout();
    }

    /// Append the subconscious stream output below the normal spinner line.
    ///
    /// The live `subconscious_current` line is rendered brightest at the bottom
    /// with a "⟡ " prefix. Older history lines from `subconscious_stream` fade
    /// upward — each older line is styled dimmer and more italic than the one
    /// below, creating a gradient fade. Capped at 5 total lines.
    ///
    /// Call after `set_phase` when phase is `Subconscious` and stream data exists.
    pub fn set_subconscious_stream(
        &mut self,
        live_line: &str,
        history: &[String],
        style: Style,
        dim_style: Style,
    ) {
        let has_live = !live_line.is_empty();
        let cap_lines = 4usize;
        let mut entries: Vec<&str> = Vec::with_capacity(1 + cap_lines);
        if has_live {
            entries.push(live_line);
        }
        let history_take = cap_lines.min(history.len());
        for s in history.iter().rev().take(history_take) {
            entries.push(s.as_str());
        }
        entries.reverse();

        if entries.is_empty() {
            return;
        }
        let entry_count = entries.len();

        let mut content = StyledString::new();
        // Spinner glyph line — the existing phase bar header.
        let spinner_line = " \u{280B} Subconscious"; // ⠋
        content.push_str(spinner_line);
        let spinner_len = content.as_ref().len();
        content.style_range(0..spinner_len, |s| *s = style);
        content.push_str("\n");

        for (slot_idx, body) in entries.iter().enumerate() {
            let is_newest = slot_idx + 1 == entry_count;
            let t = if entry_count <= 1 {
                1.0
            } else {
                slot_idx as f32 / (entry_count - 1) as f32
            };

            let line_style = if is_newest {
                style
            } else if t < 0.34 {
                dim_style.italic()
            } else if t < 0.67 {
                dim_style
            } else {
                style.italic()
            };

            let prefix = if is_newest { " \u{27E1} " } else { "   " };
            let clipped = clip_line(body, 80);
            content.push_str(&format!("{prefix}{clipped}"));
            let line_start = content.as_ref().len() - prefix.len() - clipped.len();
            content.style_range(line_start..content.as_ref().len(), |s| *s = line_style);
            if slot_idx + 1 < entry_count {
                content.push_str("\n");
            }
        }

        self.text.set_content(content);
        self.text.dirty_layout();
    }

    /// Clear the phase bar (hide it).
    pub fn clear(&mut self) {
        self.text.set_content("");
    }
}

/// Clip a line to `width` chars, appending `…` when truncated.
fn clip_line(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    let mut out = String::with_capacity(width + 1);
    for c in s.chars().take(width.saturating_sub(1)) {
        out.push(c);
    }
    out.push('\u{2026}'); // …
    out
}
