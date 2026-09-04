#![allow(deprecated)] // legacy ratatui render path, pending removal at tuie parity
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use ratatui_image::{Resize, StatefulImage};

use super::{clip_to, App};
use crate::ui::presence::Posture;

impl App {
    pub(super) fn format_age(&self, created: &str) -> String {
        use chrono::NaiveDate;
        if let Ok(d) = NaiveDate::parse_from_str(created, "%Y-%m-%d") {
            let now = chrono::Local::now().naive_local().date();
            let delta = now - d;
            let days = delta.num_days();
            let years = days / 365;
            let months = (days % 365) / 30;
            let rem_days = (days % 365) % 30;
            format!("{:02}y:{:02}m:{:02}d", years, months, rem_days)
        } else {
            "—:—:—".to_string()
        }
    }

    /// Full-height presence column — portrait fills the terminal, metadata
    /// and stats render as HUD overlays on top of the image. Waveform and
    /// Vocal Recall sit at the very bottom.
    pub(super) fn draw_presence_mode_mut(&mut self, frame: &mut Frame) {
        use crate::ui::portrait;
        let area = frame.size();
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);

        // ── Background ───────────────────────────────────────────────
        let bg = Block::default().style(Style::default().bg(palette.bg));
        frame.render_widget(bg, area);

        // ── Split: portrait fills most, voice bar at the bottom ──────
        let vchunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(5)])
            .split(area);

        let portrait_chunk = vchunks[0];
        let voice_chunk = vchunks[1];

        // ── Active agent lookup ──────────────────────────────────────
        let active_id = self
            .agent_id_by_name(&self.presence.name)
            .or_else(|| self.agent_id_by_name(&self.agent_pref));
        let agent_card = self.agent_cards.iter().find(|c| {
            Some(c.name.as_str()) == active_id.as_deref()
                || c.name.eq_ignore_ascii_case(&self.presence.name)
        });

        // Hoist card data out of agent_card before the render block
        // (which needs &mut self), so the immutable borrow on agent_cards
        // doesn't conflict.
        let card_created = agent_card.map(|c| c.created.clone());
        let card_mem_count = agent_card.map(|c| c.memory_count).unwrap_or(0);
        let card_uptime = agent_card.map(|c| c.uptime_pct).unwrap_or(0);
        let card_instances = agent_card.map(|c| c.instance_count).unwrap_or(0);
        // agent_card consumed by the .map() chain above — immutable borrow
        // on self.agent_cards is released.

        // ═══════════════════════════════════════════════════════════════
        // PORTRAIT: fill the full panel area (minus border). Cover-fill
        // scaling in render_card_image_cover handles aspect ratio and
        // keeps the face visible via top-anchored crop. No manual
        // aspect-ratio guesstimate — the cover-fill math is pixel-exact.
        // ═══════════════════════════════════════════════════════════════
        let inner_w = portrait_chunk.width.saturating_sub(2);
        let inner_h = portrait_chunk.height.saturating_sub(2);
        let photo_area = Rect {
            x: portrait_chunk.x + 1,
            y: portrait_chunk.y + 1,
            width: inner_w,
            height: inner_h,
        };
        let cell_w = photo_area.width;
        let cell_h = photo_area.height;

        // Double-line border, posture-aware color.
        let border_color = crate::ui::presence::posture_border(&self.presence);
        let frame_style = if self.presence.subconscious_active {
            Style::default().fg(border_color)
        } else {
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::DIM)
        };
        let double_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(frame_style);
        frame.render_widget(double_block, portrait_chunk);

        // Tier 0: RGP 3D portrait (ratty terminal only).
        let rgp_rendered = if let Some(ref mut g) = self.rgp_portrait {
            if g.is_active() {
                g.apply_posture(self.presence.posture);
                g.render(photo_area, frame.buffer_mut());
                true
            } else {
                false
            }
        } else {
            false
        };

        // Tiers 1-3: card_image (cover-fill) → expression cache → half-block.
        let rendered = if rgp_rendered {
            true
        } else if let Some(ref id) = active_id {
            let picker = self.image_picker.as_ref();

            // Tier 1: cover-fill. Always fills the full area, top-crops for
            // the face. Cached per area so subsequent frames are cheap.
            if picker.is_some() && self.raw_card_images.contains_key(id) {
                self.render_card_image_cover(frame, id, photo_area);
                true
            // Tier 2: expression frames — only if expressions/ dir exists.
            // Use Scale (proportional upscale) not Crop (native clip).
            } else if let Some(p) = picker {
                match Self::agent_assets_dir(id) {
                    Some(dir) => {
                        let key =
                            crate::ui::expressions::ExpressionKey::from_presence(&self.presence);
                        if let Some(proto) = self.expression_cache.resolve(id, key, p, &dir) {
                            frame.render_stateful_widget(
                                StatefulImage::default().resize(Resize::Scale(None)),
                                photo_area,
                                proto,
                            );
                            true
                        } else {
                            false
                        }
                    }
                    None => false,
                }
            } else {
                false
            }
        } else {
            false
        };
        if !rendered {
            let scale = (cell_w / portrait::PORTRAIT_W)
                .min((2 * cell_h) / portrait::PORTRAIT_H)
                .max(1);
            portrait::render_scaled(frame.buffer_mut(), photo_area, &self.presence, scale);
        }

        // ═══════════════════════════════════════════════════════════════
        // HUD OVERLAY: rendered on top of the portrait area bottom
        // ═══════════════════════════════════════════════════════════════
        let p = &self.presence;
        let (badge_icon, badge_color) = match p.posture {
            Posture::Processing => ("⚡", palette.agent_primary),
            Posture::Thinking => ("◔", Color::Rgb(120, 150, 200)),
            Posture::Alert => ("◉", palette.agent_primary),
            Posture::Affectionate => ("♥", Color::Rgb(220, 150, 170)),
            Posture::Straining => ("⚠", Color::Rgb(200, 120, 100)),
            Posture::Yawning => ("💤", Color::Rgb(160, 145, 130)),
            Posture::Listening => ("◉", palette.agent_dim),
            Posture::Speaking => ("◉", palette.agent_primary),
            Posture::Idle => ("◌", palette.agent_dim),
        };

        // Bottom 4 rows of the portrait chunk become the HUD panel.
        let hud_top = portrait_chunk.y + portrait_chunk.height.saturating_sub(5);
        let hud_area = Rect {
            x: portrait_chunk.x,
            y: hud_top,
            width: portrait_chunk.width,
            height: 5.min(portrait_chunk.height.saturating_sub(2)),
        };

        // Semi-transparent background bar.
        let (hud_r, hud_g, hud_b) = match palette.bg {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => (4, 4, 10),
        };
        let hud_bg = Block::default().style(Style::default().bg(Color::Rgb(
            hud_r.saturating_sub(2),
            hud_g.saturating_sub(2),
            hud_b.saturating_sub(2),
        )));
        frame.render_widget(hud_bg, hud_area);

        let age_str = card_created
            .as_ref()
            .map(|c| self.format_age(c))
            .unwrap_or_else(|| "—:—:—".to_string());
        let (commits, uptime, instances, mem_count) = (
            card_mem_count as u32,
            card_uptime,
            card_instances,
            card_mem_count,
        );

        let hud_inner = Rect {
            x: hud_area.x + 2,
            y: hud_area.y + 1,
            width: hud_area.width.saturating_sub(4),
            height: hud_area.height.saturating_sub(2),
        };

        let hud_lines = vec![
            // Row 1: Name + posture badge
            Line::from(vec![
                Span::styled(
                    format!(" {} ", badge_icon),
                    Style::default().fg(badge_color),
                ),
                Span::styled(
                    &p.name,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    match p.posture {
                        Posture::Listening => "  Listening",
                        Posture::Speaking => "  Speaking",
                        Posture::Processing => "  Processing",
                        Posture::Thinking => "  Thinking",
                        Posture::Alert => "  Alert",
                        Posture::Affectionate => "  Affectionate",
                        Posture::Straining => "  Straining",
                        Posture::Yawning => "  Yawning",
                        Posture::Idle => "",
                    },
                    Style::default().fg(badge_color).add_modifier(Modifier::DIM),
                ),
            ]),
            // Row 2: AGE
            Line::from(vec![
                Span::styled(" AGE  ", Style::default().fg(palette.agent_dim)),
                Span::styled(age_str, Style::default().fg(palette.agent_primary)),
            ]),
            // Row 3: STATS grid
            Line::from(vec![
                Span::styled(" STATS", Style::default().fg(palette.agent_dim)),
                Span::raw("  "),
                Span::styled(
                    format!("C {}", commits),
                    Style::default().fg(Color::Rgb(160, 200, 140)),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("U {}%", uptime),
                    Style::default().fg(Color::Rgb(120, 220, 160)),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("I {}", instances),
                    Style::default().fg(Color::Rgb(160, 180, 220)),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("M {}", mem_count),
                    Style::default().fg(Color::Rgb(140, 200, 180)),
                ),
                if self
                    .rgp_portrait
                    .as_ref()
                    .map(|g| g.is_active())
                    .unwrap_or(false)
                {
                    Span::styled("  3D", Style::default().fg(Color::Rgb(220, 180, 255)))
                } else {
                    Span::raw("")
                },
            ]),
            // Row 4: Mood / outfit
            Line::from(vec![
                Span::styled(" MOOD ", Style::default().fg(palette.agent_dim)),
                Span::styled(&p.mood, Style::default().fg(palette.agent_primary)),
                Span::raw("  ·  "),
                Span::styled(
                    p.outfit.as_deref().unwrap_or("default"),
                    Style::default().fg(palette.agent_dim),
                ),
            ]),
        ];

        frame.render_widget(
            Paragraph::new(hud_lines).alignment(Alignment::Left),
            hud_inner,
        );

        // ═══════════════════════════════════════════════════════════════
        // VOICE BAR: transcript, waveform, Vocal Recall controls
        // ═══════════════════════════════════════════════════════════════
        let voice_area = voice_chunk;
        let is_listening = p.posture == Posture::Listening;
        let is_speaking = p.posture == Posture::Speaking;
        let has_recent_tts = self.voice_last_tts_text.is_some();

        // Sub-layout: transcript (1), waveform (1), controls (rest).
        let voice_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(voice_area);

        let transcript_area = voice_rows[0];
        let wave_area = voice_rows[1];
        let hint_area = voice_rows[2];

        // ── Transcript row ─────────────────────────────────────────
        // Shows what was said (STT) or what she said (TTS text).
        let transcript = self
            .voice_last_transcript
            .as_deref()
            .filter(|t| !t.is_empty());
        let tts_display = self
            .voice_last_tts_text
            .as_deref()
            .filter(|t| !t.is_empty());

        let transcript_line = if is_listening {
            transcript
                .map(|t| format!("‹ {} ›", t))
                .unwrap_or_else(|| " listen  ".to_string())
        } else if is_speaking {
            tts_display
                .map(|t| clip_to(t, voice_area.width.saturating_sub(6) as usize))
                .map(|c| format!("» {} «", c))
                .unwrap_or_else(|| " speak  ".to_string())
        } else if let Some(t) = tts_display {
            let clip = clip_to(t, voice_area.width.saturating_sub(6) as usize);
            format!("» {} «", clip)
        } else if let Some(t) = transcript {
            let clip = clip_to(t, voice_area.width.saturating_sub(6) as usize);
            format!("‹ {} ›", clip)
        } else {
            String::new()
        };

        let transcript_color = if is_listening {
            palette.agent_primary
        } else if is_speaking {
            self.presence.atmosphere.primary()
        } else {
            palette.agent_dim
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                transcript_line,
                Style::default()
                    .fg(transcript_color)
                    .add_modifier(Modifier::DIM),
            ))),
            transcript_area,
        );

        // ── Waveform row ───────────────────────────────────────────
        if is_listening {
            let level = self
                .voice_capture
                .as_ref()
                .map(|c| c.current_level())
                .unwrap_or(0.0);
            let is_recording = level > 0.05;
            let rec_glyph = if is_recording && self.tick.is_multiple_of(2) {
                "● REC"
            } else {
                "  rec"
            };

            let mut wave_spans: Vec<Span> = Vec::new();
            let bar_w = (voice_area.width.saturating_sub(10)).min(128) as usize;
            wave_spans.push(Span::styled(
                format!(" {} ", rec_glyph),
                Style::default().fg(if is_recording {
                    Color::Rgb(220, 60, 60)
                } else {
                    palette.agent_dim
                }),
            ));

            let wf_len = self.voice_waveform.len();
            if bar_w > 0 && wf_len > 0 {
                let step = (wf_len as f32 / bar_w as f32).max(1.0);
                for i in 0..bar_w {
                    let idx = ((i as f32) * step) as usize;
                    let sample = self.voice_waveform.get(idx).copied().unwrap_or(0.0);
                    let ch = crate::ui::voice::LEVEL_CHARS[(sample * 7.0).round() as usize];
                    let b = (60.0 + sample * 195.0) as u8;
                    wave_spans.push(Span::styled(
                        ch.to_string(),
                        Style::default().fg(Color::Rgb(b / 2, b, b / 3)),
                    ));
                }
            }
            frame.render_widget(Paragraph::new(Line::from(wave_spans)), wave_area);
        } else if is_speaking {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    " ♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪",
                    Style::default().fg(self.presence.atmosphere.primary()),
                ))),
                wave_area,
            );
        } else {
            let ghost: String = "▁▂▃▄▅▆▇█▇▆▅▄▃▂"
                .chars()
                .flat_map(|c| std::iter::repeat_n(c, 3))
                .take(voice_area.width as usize)
                .collect();
            let (ghost_r, ghost_g, ghost_b) = match palette.bg {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (40, 44, 60),
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    ghost,
                    Style::default().fg(Color::Rgb(
                        ghost_r.saturating_add(30),
                        ghost_g.saturating_add(30),
                        ghost_b.saturating_add(36),
                    )),
                ))),
                wave_area,
            );
        }

        // ── Controls row ───────────────────────────────────────────
        if is_listening {
            let hint = Paragraph::new(Line::from(Span::styled(
                " Space → send  ·  Esc → cancel",
                Style::default().fg(palette.agent_dim),
            )))
            .alignment(Alignment::Center);
            frame.render_widget(hint, hint_area);
        } else if is_speaking || has_recent_tts {
            let recall = vec![
                Span::styled(" r ⟲ ", Style::default().fg(palette.tool_accent)),
                Span::raw("Replay  "),
                Span::styled(" g ↻ ", Style::default().fg(palette.agent_primary)),
                Span::raw("Regen  "),
                Span::styled(" s 💾 ", Style::default().fg(palette.tool_accent)),
                Span::raw("Save  ·  "),
                Span::styled("Space to speak", Style::default().fg(palette.agent_dim)),
            ];
            frame.render_widget(
                Paragraph::new(Line::from(recall)).alignment(Alignment::Center),
                hint_area,
            );
        } else {
            let hint = " Space to speak  ·  Esc to leave";
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    hint,
                    Style::default().fg(palette.agent_dim),
                )))
                .alignment(Alignment::Center),
                hint_area,
            );
        }
    }
}
