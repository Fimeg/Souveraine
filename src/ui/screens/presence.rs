#![allow(dead_code)] // WIP scaffolding not yet wired
//! Presence mode screen — voice recording, playback, and atmosphere display.
//!
//! Shows the current atmosphere preset, voice recording controls, a live
//! audio level meter, and a log of recent voice transcripts. Key bindings:
//! R → record, P → play, S → stop, Esc → return to Welcome.
//!
//! The screen owns a [`MicCapture`] and [`VoicePlayer`]; the meter bar updates
//! every 50 ms via [`tuie::schedule`] while recording or playing.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use tuie::prelude::*;

use crate::ui::atmosphere::Atmosphere;
use crate::ui::chat::ChatPalette;
use crate::ui::presence::Posture;
use crate::ui::theme;
use crate::ui::voice::{MicCapture, VoicePlayer, LEVEL_CHARS};

/// Maximum number of recent transcripts to keep.
const MAX_RECENT: usize = 8;

/// Meter bar width in characters for the level display.
const METER_WIDTH: usize = 40;

// ── PresenceScreen widget ──────────────────────────────────────────────────────

pub struct PresenceScreen {
    root: Box<Pane>,

    // Widget IDs for dynamic updates.
    atmosphere_name_id: WidgetId<Text>,
    posture_status_id: WidgetId<Text>,
    voice_status_id: WidgetId<Text>,
    voice_meter_id: WidgetId<Text>,
    voice_transcript_id: WidgetId<Text>,
    recent_id: WidgetId<Text>,

    // Visual state.
    palette: ChatPalette,
    atmosphere: Atmosphere,
    posture: Posture,

    // Voice state.
    mic: Option<MicCapture>,
    player: Option<VoicePlayer>,
    waveform: Vec<f32>,
    last_transcript: Option<String>,
    recent_transcripts: Vec<String>,

    // Frame counter for blink / breath.
    tick: u64,

    // Set to true when the user presses Esc — parent reads and pops the screen.
    pub should_exit: Rc<Cell<bool>>,
}

impl DelegateWidget for PresenceScreen {
    tuie::delegate_widget!(root);

    fn override_is_focusable(&self) -> bool {
        true
    }

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        use tuie::input::key::Key;
        use tuie::input::trigger::Trigger;

        if let Some(event) = queue.peek() {
            if let Trigger::Key(key) = &event.chord.trigger {
                match key {
                    Key::Char('r') | Key::Char('R') => {
                        queue.next();
                        self.start_recording();
                        return InputResult::Handled;
                    }
                    Key::Char('p') | Key::Char('P') => {
                        queue.next();
                        self.start_playback();
                        return InputResult::Handled;
                    }
                    Key::Char('s') | Key::Char('S') => {
                        queue.next();
                        self.stop_all();
                        return InputResult::Handled;
                    }
                    Key::Esc => {
                        queue.next();
                        self.stop_all();
                        self.should_exit.set(true);
                        return InputResult::Handled;
                    }
                    _ => {}
                }
            }
        }
        InputResult::Rejected
    }
}

impl PresenceScreen {
    pub fn new(
        palette: &ChatPalette,
        atmosphere: Atmosphere,
        posture: Posture,
    ) -> (Box<Self>, Rc<Cell<bool>>) {
        let should_exit = Rc::new(Cell::new(false));
        let primary = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);
        let tool = theme::to_tuie_color(palette.tool_accent);

        // ── Header ──────────────────────────────────────────────────────────
        let header = Text::new().content(StyledStr::new(" Presence Mode ").fg(primary).bold());

        // ── Atmosphere section ──────────────────────────────────────────────
        let atmo_name_text = Text::new().content(
            StyledString::new()
                .span(StyledStr::new(" Preset ").fg(dim))
                .span(
                    StyledStr::new(atmosphere_display_name(atmosphere))
                        .fg(theme::to_tuie_color(atmosphere.primary()))
                        .bold(),
                ),
        );
        let atmosphere_name_id = atmo_name_text.get_id();

        let posture_text = Text::new().content(
            StyledString::new()
                .span(StyledStr::new(" Posture ").fg(dim))
                .span(StyledStr::new(posture_label(posture)).fg(posture_color(posture, palette))),
        );
        let posture_status_id = posture_text.get_id();

        let atmosphere_section = Pane::new()
            .vertical()
            .gap(0)
            .bordered()
            .border_style(Style::new().fg(dim).dim())
            .children([atmo_name_text as Box<dyn Widget>, posture_text]);

        // ── Voice section ──────────────────────────────────────────────────
        let voice_status_text = Text::new().content(
            StyledString::new()
                .span(StyledStr::new(" Status ").fg(dim))
                .span(StyledStr::new(" Idle").fg(dim)),
        );
        let voice_status_id = voice_status_text.get_id();

        let voice_meter_text = Text::new().content(meter_bar(0.0, dim, dim));
        let voice_meter_id = voice_meter_text.get_id();

        let voice_transcript_text = Text::new().content(
            StyledString::new()
                .span(StyledStr::new(" Transcript ").fg(dim))
                .span(StyledStr::new(" (none)").fg(dim).italic()),
        );
        let voice_transcript_id = voice_transcript_text.get_id();

        let voice_section = Pane::new()
            .vertical()
            .gap(0)
            .bordered()
            .border_style(Style::new().fg(dim).dim())
            .children([
                voice_status_text as Box<dyn Widget>,
                voice_meter_text,
                voice_transcript_text,
            ]);

        // ── Recent recordings section ──────────────────────────────────────
        let recent_text = Text::new().content(
            StyledString::new().span(
                StyledStr::new(" (no recent voice recordings) ")
                    .fg(dim)
                    .italic(),
            ),
        );
        let recent_id = recent_text.get_id();

        let recent_section = Pane::new()
            .vertical()
            .flex(1)
            .min_height(3)
            .bordered()
            .border_style(Style::new().fg(dim).dim())
            .children([
                Text::new().content(StyledStr::new(" Recent ").fg(tool).bold()) as Box<dyn Widget>,
                recent_text,
            ]);

        // ── Footer ─────────────────────────────────────────────────────────
        let footer = Text::new()
            .content(StyledStr::new(" R Record    P Play    S Stop    Esc Return ").fg(dim));

        // ── Root ───────────────────────────────────────────────────────────
        let root = Pane::new()
            .vertical()
            .gap(1)
            .padding(Spacing::new().horizontal(1).top(1).bottom(1))
            .children([
                header as Box<dyn Widget>,
                atmosphere_section,
                voice_section,
                recent_section,
                footer,
            ]);

        let this = Box::new(Self {
            root,
            atmosphere_name_id,
            posture_status_id,
            voice_status_id,
            voice_meter_id,
            voice_transcript_id,
            recent_id,
            palette: *palette,
            atmosphere,
            posture,
            mic: None,
            player: None,
            waveform: Vec::new(),
            last_transcript: None,
            recent_transcripts: Vec::new(),
            tick: 0,
            should_exit: should_exit.clone(),
        });
        (this, should_exit)
    }

    // ── Activation (called by TuieApp after the widget is in the tree) ─────

    /// Start the poll loop that advances voice state and redraws the meter.
    pub fn activate(&self) {
        self.schedule_poll();
    }

    /// Open the audio output device lazily so playback can work.
    pub fn ensure_player(&mut self) {
        if self.player.is_none() {
            match VoicePlayer::new() {
                Ok(p) => {
                    tracing::info!("VoicePlayer opened in presence screen");
                    self.player = Some(p);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to open audio output in presence screen");
                }
            }
        }
    }

    // ── Setters (called by TuieApp when posture / atmosphere change) ───────

    pub fn set_palette(&mut self, palette: ChatPalette) {
        self.palette = palette;
    }

    pub fn set_atmosphere(&mut self, atmosphere: Atmosphere, posture: Posture) {
        self.atmosphere = atmosphere;
        self.posture = posture;
        self.refresh_atmosphere_section();
        self.refresh_posture_section();
    }

    // ── Voice commands ─────────────────────────────────────────────────────

    fn start_recording(&mut self) {
        // Stop anything that's already running.
        self.stop_all();

        match MicCapture::start(16_000) {
            Ok(cap) => {
                self.mic = Some(cap);
                self.waveform.clear();
                self.posture = Posture::Listening;
                tracing::info!("presence screen — mic capture started");
                self.update_voice_ui(" Recording");
            }
            Err(e) => {
                tracing::warn!(error = %e, "presence screen — mic capture failed");
                self.update_voice_ui(" Mic unavailable");
                self.last_transcript = Some(format!("[mic unavailable — {}]", e));
                self.push_recent_transcript();
            }
        }
    }

    fn start_playback(&mut self) {
        // Playback replays the last TTS audio if available.
        // In the initial implementation, voice synthesis happens upstream
        // in the App; the screen shows what's been set.
        self.ensure_player();

        if let Some(ref player) = self.player {
            if player.is_speaking() {
                // Already playing — restart from the beginning by stopping first.
                player.stop();
            }
            // Playback of stored bytes is handled by the App layer.
            // Here we indicate playback readiness.
            self.update_voice_ui(" Playing");
        } else {
            self.update_voice_ui(" No audio device");
        }
    }

    fn stop_all(&mut self) {
        if let Some(cap) = self.mic.take() {
            let samples = cap.stop_and_take();
            let sample_count = samples.len();
            if sample_count > 0 {
                tracing::info!(
                    samples = sample_count,
                    "presence screen — mic capture stopped"
                );
                // Store the raw sample count as a transcript marker for now;
                // actual STT transcription runs through the App's voice pipeline.
                self.last_transcript = Some(format!(
                    "[{} samples captured — awaiting transcription]",
                    sample_count
                ));
                self.push_recent_transcript();
            } else {
                self.last_transcript = Some("[empty recording]".to_string());
                self.push_recent_transcript();
            }
        }

        self.posture = Posture::Idle;
        self.waveform.clear();
        self.update_voice_ui(" Idle");
        self.update_meter(0.0);
    }

    // ── Poll loop ──────────────────────────────────────────────────────────

    fn schedule_poll(&self) {
        let screen_id = self.get_id();
        tuie::schedule(
            screen_id,
            Duration::from_millis(50),
            |this: &mut PresenceScreen| {
                this.poll_tick();
                this.schedule_poll();
            },
        );
    }

    fn poll_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);

        // Read mic level and update waveform.
        if let Some(ref mic) = self.mic {
            let level = mic.current_level();
            self.waveform.push(level);
            if self.waveform.len() > 128 {
                self.waveform.remove(0);
            }
            self.update_meter(level);
        }

        // Check playback state.
        let was_playing = self.posture == Posture::Speaking;
        let still_playing = self
            .player
            .as_ref()
            .map(|p| p.is_speaking())
            .unwrap_or(false);
        if was_playing && !still_playing {
            self.posture = Posture::Idle;
            self.update_voice_ui(" Idle");
            self.update_meter(0.0);
        }
    }

    // ── UI update helpers ──────────────────────────────────────────────────

    fn update_voice_ui(&mut self, status: &str) {
        let dim = theme::to_tuie_color(self.palette.agent_dim);
        let status_color = match self.posture {
            Posture::Listening => theme::to_tuie_color(self.atmosphere.secondary()),
            Posture::Speaking => theme::to_tuie_color(self.atmosphere.primary()),
            _ => dim,
        };

        if let Some(w) = self.root.get_widget_mut(self.voice_status_id) {
            w.set_content(
                StyledString::new()
                    .span(StyledStr::new(" Status ").fg(dim))
                    .span(StyledStr::new(status).fg(status_color)),
            );
        }

        if let Some(w) = self.root.get_widget_mut(self.voice_transcript_id) {
            let label = if self.posture == Posture::Listening {
                " Listening for "
            } else if self.posture == Posture::Speaking {
                " Playback "
            } else {
                " Transcript "
            };
            let text = self.last_transcript.as_deref().unwrap_or(" (none)");
            w.set_content(
                StyledString::new()
                    .span(StyledStr::new(label).fg(dim))
                    .span(StyledStr::new(text).fg(status_color)),
            );
        }
    }

    fn update_meter(&mut self, level: f32) {
        let dim = theme::to_tuie_color(self.palette.agent_dim);
        let primary = theme::to_tuie_color(self.palette.agent_primary);

        let (fg, bg) = if level < 0.3 {
            (dim, dim)
        } else if level < 0.6 {
            (theme::to_tuie_color(self.atmosphere.secondary()), dim)
        } else {
            (primary, dim)
        };

        if let Some(w) = self.root.get_widget_mut(self.voice_meter_id) {
            w.set_content(meter_bar(level, fg, bg));
        }
    }

    fn push_recent_transcript(&mut self) {
        if let Some(ref text) = self.last_transcript {
            self.recent_transcripts.push(text.clone());
            if self.recent_transcripts.len() > MAX_RECENT {
                self.recent_transcripts.remove(0);
            }
        }
        self.refresh_recent_section();
    }

    fn refresh_atmosphere_section(&mut self) {
        let dim = theme::to_tuie_color(self.palette.agent_dim);
        let atmo_color = theme::to_tuie_color(self.atmosphere.primary());

        if let Some(w) = self.root.get_widget_mut(self.atmosphere_name_id) {
            w.set_content(
                StyledString::new()
                    .span(StyledStr::new(" Preset ").fg(dim))
                    .span(
                        StyledStr::new(atmosphere_display_name(self.atmosphere))
                            .fg(atmo_color)
                            .bold(),
                    ),
            );
        }
    }

    fn refresh_posture_section(&mut self) {
        let dim = theme::to_tuie_color(self.palette.agent_dim);

        if let Some(w) = self.root.get_widget_mut(self.posture_status_id) {
            w.set_content(
                StyledString::new()
                    .span(StyledStr::new(" Posture ").fg(dim))
                    .span(
                        StyledStr::new(posture_label(self.posture))
                            .fg(posture_color(self.posture, &self.palette)),
                    ),
            );
        }
    }

    fn refresh_recent_section(&mut self) {
        let dim = theme::to_tuie_color(self.palette.agent_dim);

        if let Some(w) = self.root.get_widget_mut(self.recent_id) {
            if self.recent_transcripts.is_empty() {
                w.set_content(
                    StyledString::new().span(
                        StyledStr::new(" (no recent voice recordings) ")
                            .fg(dim)
                            .italic(),
                    ),
                );
            } else {
                let mut content = StyledString::new();
                for (i, t) in self.recent_transcripts.iter().rev().enumerate() {
                    if i > 0 {
                        content.push_span(StyledStr::new("\n"));
                    }
                    let clipped: String = if t.len() > 72 {
                        t.chars().take(69).chain("...".chars()).collect()
                    } else {
                        t.clone()
                    };
                    content.push_span(StyledStr::new("  ").fg(dim));
                    content.push_span(StyledStr::new(&clipped).fg(dim));
                    content.push_span(StyledStr::new(" ").fg(dim));
                }
                w.set_content(content);
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a horizontal meter bar string like `[████████░░░░░░░░░░]` with colour.
fn meter_bar(level: f32, fg: Color, bg: Color) -> StyledString {
    let level = level.clamp(0.0, 1.0);
    let filled = ((level * METER_WIDTH as f32).round() as usize).min(METER_WIDTH);
    let empty = METER_WIDTH.saturating_sub(filled);

    let mut s = StyledString::new();
    s.push_span(StyledStr::new("[").fg(bg));

    // Filled portion uses the level character at the appropriate density.
    if filled > 0 {
        let block_idx = (level * 7.0).round() as usize;
        let ch = LEVEL_CHARS[block_idx.min(7)];
        let filled_str = ch.to_string().repeat(filled);
        s.push_span(StyledStr::new(&filled_str).fg(fg));
    }
    if empty > 0 {
        let empty_str = "\u{2591}".repeat(empty);
        s.push_span(StyledStr::new(&empty_str).fg(bg));
    }
    s.push_span(StyledStr::new("]").fg(bg));
    s
}

fn atmosphere_display_name(atm: Atmosphere) -> &'static str {
    use Atmosphere::*;
    match atm {
        Default => "Default",
        MintTea => "Mint Tea",
        TherapeuticBlue => "Therapeutic Blue",
        LavenderCalm => "Lavender Calm",
        WarmAmber => "Warm Amber",
        PeachSunset => "Peach Sunset",
        AutumnBrowns => "Autumn Browns",
        // A blend is unnameable by construction — `from_name` cannot make one
        // and `from_posture` never returns one — so this only ever renders
        // mid-transition, and saying so is more use than a colour triple.
        Custom { .. } => "Blending",
        NeonGlow => "Neon Glow",
        AuroraBorealis => "Aurora Borealis",
        CherryBlossom => "Cherry Blossom",
        OceanDepths => "Ocean Depths",
        MidnightGalaxy => "Midnight Galaxy",
        TwilightMist => "Twilight Mist",
        ForestGreens => "Forest Greens",
    }
}

fn posture_label(posture: Posture) -> &'static str {
    match posture {
        Posture::Idle => "Idle",
        Posture::Alert => "Alert",
        Posture::Thinking => "Thinking",
        Posture::Processing => "Processing",
        Posture::Affectionate => "Affectionate",
        Posture::Straining => "Straining",
        Posture::Yawning => "Yawning",
        Posture::Listening => "Listening",
        Posture::Speaking => "Speaking",
    }
}

fn posture_color(posture: Posture, palette: &ChatPalette) -> Color {
    match posture {
        Posture::Idle => theme::to_tuie_color(palette.agent_dim),
        Posture::Alert => theme::to_tuie_color(palette.user_accent),
        Posture::Thinking => Color::Rgb(120, 150, 200),
        Posture::Processing => theme::to_tuie_color(palette.agent_primary),
        Posture::Affectionate => Color::Rgb(220, 150, 170),
        Posture::Straining => Color::Rgb(200, 120, 100),
        Posture::Yawning => Color::Rgb(160, 145, 130),
        Posture::Listening => theme::to_tuie_color(palette.agent_primary),
        Posture::Speaking => theme::to_tuie_color(palette.agent_primary),
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use tuie::emulator::Emulator;

    use super::*;
    use crate::ui::atmosphere::Atmosphere;
    use crate::ui::chat::ChatPalette;
    use crate::ui::presence::Posture;

    #[test]
    fn presence_screen_renders_header() {
        let palette = ChatPalette::default();
        let (mut screen, _should_exit) =
            PresenceScreen::new(&palette, Atmosphere::Default, Posture::Idle);
        let term = Emulator::new(&mut *screen, Vec2::new(80, 20));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("Presence"),
            "expected 'Presence' in header, got: {rendered:?}"
        );
    }

    #[test]
    fn presence_screen_shows_keybindings() {
        let palette = ChatPalette::default();
        let (mut screen, _should_exit) =
            PresenceScreen::new(&palette, Atmosphere::Default, Posture::Idle);
        let term = Emulator::new(&mut *screen, Vec2::new(80, 20));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("R") || rendered.contains("Record") || rendered.contains("Esc"),
            "expected keybinding hints (R, Record, or Esc) in output, got: {rendered:?}"
        );
    }
}
