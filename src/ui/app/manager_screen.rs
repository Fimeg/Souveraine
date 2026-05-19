use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use ratatui_image::{Resize, StatefulImage};

use super::{App, AgentCard, short_id};
use crate::ui::presence::Posture;
use crate::core::config::ConsciousnessConfig;

impl App {
    pub(super) async fn refresh_agent_cards(&mut self) {
        let cfg = self.config.read().await.clone();
        self.agent_cards = Self::fetch_agent_cards(cfg).await;
    }

    /// Standalone fetch so it can be called without &mut self during init.
    pub(super) async fn fetch_agent_cards(cfg: ConsciousnessConfig) -> Vec<AgentCard> {
        use crate::backend::Backend;
        let Ok(local) = crate::backend::LocalBackend::new(cfg).await else { return vec![] };
        let Ok(list) = local.list_agents().await else { return vec![] };
        let inv = local.server_agents();
        let mut cards = Vec::new();
        for a in &list {
            let glyph = inv.seed_id(&a.id)
                .map(|s| s.glyph())
                .unwrap_or_else(|_| "◇◆".to_string());
            let pubkey_prefix = inv.seed_id(&a.id)
                .map(|s| s.public_key_hex()[..16].to_string())
                .unwrap_or_else(|_| "—".to_string());
            let instance_count = inv.instance_count(&a.id).await.unwrap_or(0);
            let lifetime_secs = inv.lifetime_active_seconds(&a.id).await.unwrap_or(0);
            let uptime_pct = if lifetime_secs > 0 {
                let days = ((instance_count.max(1)) as f64 * 30.0).max(1.0);
                let pct = (lifetime_secs as f64 / (days * 86400.0)) * 100.0;
                pct.min(99.0) as u8
            } else { 0 };
            let mem_count = local.server_agents().memory_repo(&a.id)
                .status()
                .map(|s| s.file_count)
                .unwrap_or(0);
            cards.push(AgentCard {
                id: a.id.clone(),
                name: a.name.clone(),
                description: a.description.clone().unwrap_or_default(),
                glyph,
                pubkey_prefix,
                instance_count,
                uptime_pct,
                memory_count: mem_count,
                created: "Feb 2025 · TBD date from server".to_string(),
            });
        }
        cards.sort_by(|a, b| a.name.cmp(&b.name));
        cards
    }

    /// Render the agent manager — Letta-style card deck. Each card has:
    ///   • a status badge (ACTIVE / PRIMARY) in the top-right
    ///   • a scale-to-fit portrait photo occupying the top ~55% of the card
    ///   • a dark metadata block below the photo, holding:
    ///       — seed glyph row + instance count
    ///       — agent name with `[AGENT]` tag
    ///       — agent id prefix as a path-style monospace breadcrumb
    ///       — a stats row (files / uptime / active duration placeholder)
    ///   • the primary agent gets a cyan accent border and bold weight
    ///
    /// Cards without a portrait file fall back to the half-block silhouette
    /// in the image slot so the grid stays geometrically uniform.
    ///
    /// `&mut self` is required because `StatefulImage` re-encodes the
    /// per-card protocol on each render to match the current cell area.
    pub(super) fn draw_agent_cards_mut(&mut self, frame: &mut Frame) {
        use crate::ui::portrait;
        let area = frame.size();
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);
        let bg = Block::default().style(Style::default().bg(palette.bg));
        frame.render_widget(bg, area);

        // ── Header strip ──────────────────────────────────────────────
        let header = Paragraph::new(Line::from(vec![
            Span::styled("  Agent Manager   ", Style::default()
                .fg(palette.agent_primary)
                .add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{} agents  ·  manage, monitor, deploy", self.agent_cards.len()),
                Style::default().fg(palette.agent_dim),
            ),
        ])).alignment(Alignment::Center);
        let header_area = Rect { x: area.x, y: area.y + 1, width: area.width, height: 1 };
        frame.render_widget(header, header_area);

        if self.agent_cards.is_empty() {
            let empty = Paragraph::new("\n\n(no agents found — run `souveraine init`)")
                .style(Style::default().fg(palette.agent_dim))
                .alignment(Alignment::Center);
            frame.render_widget(empty, area);
            return;
        }

        // ── Grid math ─────────────────────────────────────────────────
        // Letta shows 4 cards across; we pick the column count based on
        // available width so terminals down to ~50 cols still get usable
        // cards. Each card is taller than wide (portrait-style).
        let pad_x: u16 = 2;
        let pad_y: u16 = 1;
        let min_card_w: u16 = 22;
        let max_card_w: u16 = 32;
        let n: u16 = self.agent_cards.len() as u16;
        // Pick cols so card_w ∈ [min, max], preferring more cols on wider screens.
        let mut cols: u16 = 4;
        loop {
            let avail = area.width.saturating_sub((cols + 1) * pad_x);
            let cw = avail / cols.max(1);
            if cw >= min_card_w || cols == 1 { break; }
            cols -= 1;
        }
        cols = cols.min(n).max(1);
        self.manager_cols = cols as usize;
        // Clamp selection to valid range in case cards changed since last draw.
        self.manager_selected = self.manager_selected.min(self.agent_cards.len().saturating_sub(1));
        let avail = area.width.saturating_sub((cols + 1) * pad_x);
        let card_w = (avail / cols).min(max_card_w).max(min_card_w);
        // Card height: image area (target ~ card_w / 2 + 2, so a 24-wide card
        // gets 14 image rows) + 6 rows of metadata + 2 rows of border/badge.
        let image_h: u16 = (card_w / 2 + 3).max(8);
        let meta_h: u16 = 7;
        let card_h: u16 = image_h + meta_h + 2; // +2 for top/bottom border
        let grid_w = cols * card_w + (cols.saturating_sub(1)) * pad_x;
        let grid_x = area.x + area.width.saturating_sub(grid_w) / 2;
        let grid_y = area.y + 3;

        // Snapshot plans first so we can hold `&mut self.card_images` per card
        // without overlapping the immutable borrow of `self.agent_cards`.
        let manager_selected = self.manager_selected;
        struct Plan {
            card_area: Rect,
            image_area: Rect,
            badge_area: Rect,
            meta_area: Rect,
            agent_id: String,
            name: String,
            glyph: String,
            pubkey: String,
            instance_count: i64,
            uptime_pct: u8,
            memory_count: usize,
            is_primary: bool,
            is_selected: bool,
        }
        let plans: Vec<Plan> = self.agent_cards
            .iter()
            .enumerate()
            .filter_map(|(idx, card)| {
                let col = (idx as u16) % cols;
                let row = (idx as u16) / cols;
                let cx = grid_x + col * (card_w + pad_x);
                let cy = grid_y + row * (card_h + pad_y);
                if cy + card_h >= area.y + area.height.saturating_sub(2) {
                    return None;
                }
                let card_area = Rect { x: cx, y: cy, width: card_w, height: card_h };
                // Inner area inside the rounded border.
                let inner_w = card_w.saturating_sub(2);
                let inner_x = cx + 1;
                let image_y = cy + 1;
                let image_area = Rect { x: inner_x, y: image_y, width: inner_w, height: image_h };
                // Badge floats in the top-right corner of the image area,
                // overlaid as text spans (no separate widget).
                let badge_w: u16 = 10.min(inner_w);
                let badge_area = Rect {
                    x: inner_x + inner_w.saturating_sub(badge_w),
                    y: image_y,
                    width: badge_w,
                    height: 1,
                };
                let meta_area = Rect {
                    x: inner_x,
                    y: image_y + image_h,
                    width: inner_w,
                    height: meta_h,
                };
                Some(Plan {
                    card_area,
                    image_area,
                    badge_area,
                    meta_area,
                    agent_id: card.id.clone(),
                    name: card.name.clone(),
                    glyph: card.glyph.clone(),
                    pubkey: card.pubkey_prefix.clone(),
                    instance_count: card.instance_count,
                    uptime_pct: card.uptime_pct,
                    memory_count: card.memory_count,
                    is_primary: card.name.eq_ignore_ascii_case(&self.agent_pref),
                    is_selected: idx == manager_selected,
                })
            })
            .collect();

        // ── Render each card ──────────────────────────────────────────
        for p in plans {
            let accent = if p.is_primary {
                palette.agent_primary
            } else if p.instance_count > 0 {
                Color::Rgb(120, 220, 160) // active: green (semantic — keep)
            } else {
                palette.agent_dim
            };
            let border_color = if p.is_selected {
                palette.agent_primary
            } else if p.is_primary {
                palette.agent_primary
            } else {
                palette.agent_dim
            };

            // Card background fill (lifts the card off the screen).
            let (cr, cg, cb) = match palette.bg { Color::Rgb(r, g, b) => (r, g, b), _ => (16, 18, 28) };
            let card_bg = Block::default().style(Style::default().bg(Color::Rgb(cr.saturating_add(6), cg.saturating_add(6), cb.saturating_add(6))));
            frame.render_widget(card_bg, p.card_area);

            // Border — gold when cursor is here, violet for primary, dim otherwise.
            let border_modifier = if p.is_selected || p.is_primary {
                Modifier::BOLD
            } else {
                Modifier::DIM
            };
            let border = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border_color).add_modifier(border_modifier));
            frame.render_widget(border, p.card_area);

            // Image — scale-to-fit so the whole photo is visible. The
            // letterbox space inherits the card_bg above, which reads as
            // a clean dark frame.
            if let Some(proto) = self.card_images.get_mut(&p.agent_id) {
                frame.render_stateful_widget(
                    StatefulImage::default().resize(Resize::Fit(None)),
                    p.image_area,
                    proto,
                );
            } else {
                portrait::render(frame.buffer_mut(), p.image_area, &self.presence);
            }

            // Top-right badge: PRIMARY (with ★) or ACTIVE (with •) or muted.
            let (badge_text, badge_fg) = if p.is_primary {
                ("★ PRIMARY ", Color::Rgb(245, 230, 110)) // gold (semantic — keep)
            } else if p.instance_count > 0 {
                ("• ACTIVE  ", Color::Rgb(120, 220, 160)) // green (semantic — keep)
            } else {
                (" idle     ", palette.agent_dim)
            };
            let badge_para = Paragraph::new(Line::from(vec![
                Span::styled(badge_text, Style::default()
                    .fg(badge_fg)
                    .bg(palette.bg)
                    .add_modifier(Modifier::BOLD)),
            ])).alignment(Alignment::Right);
            frame.render_widget(badge_para, p.badge_area);

            // Metadata block — slightly darker inset under the photo.
            let (mr, mg, mb) = match palette.bg { Color::Rgb(r, g, b) => (r.saturating_sub(4), g.saturating_sub(4), b.saturating_sub(4)), _ => (12, 14, 22) };
            let meta_bg = Block::default().style(Style::default().bg(Color::Rgb(mr, mg, mb)));
            frame.render_widget(meta_bg, p.meta_area);

            let instance_label = if p.instance_count == 1 {
                "1 instance".to_string()
            } else {
                format!("{} instances", p.instance_count)
            };
            let path = format!("agents/{}", short_id(&p.agent_id));

            // Compose 7 lines into the meta_area:
            //   0: spacer
            //   1: glyph row + instance count
            //   2: name + [AGENT]
            //   3: path-style id
            //   4: separator rule
            //   5: stats (files / uptime / commits placeholder)
            //   6: action hint
            let meta_lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(format!(" {} ", p.glyph),
                        Style::default().fg(accent).add_modifier(Modifier::BOLD)),
                    Span::styled(instance_label,
                        Style::default().fg(palette.agent_dim)),
                ]),
                Line::from(vec![
                    Span::styled(format!(" {} ", p.name),
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("[AGENT]",
                        Style::default().fg(palette.agent_dim)
                            .bg(Color::Rgb(cr, cg, cb))),
                ]),
                Line::from(vec![
                    Span::styled(format!(" {} ", path),
                        Style::default().fg(palette.agent_dim)),
                ]),
                Line::from(Span::styled(
                    "─".repeat(p.meta_area.width as usize),
                    Style::default().fg(palette.agent_dim).add_modifier(Modifier::DIM),
                )),
                Line::from(vec![
                    Span::styled(" Files  ",
                        Style::default().fg(palette.agent_dim)),
                    Span::styled(format!("{:<5}", p.memory_count),
                        Style::default().fg(Color::White)),
                    Span::styled("Uptime  ",
                        Style::default().fg(palette.agent_dim)),
                    Span::styled(format!("{}%", p.uptime_pct),
                        Style::default().fg(Color::Rgb(120, 220, 160))), // green (semantic — keep)
                ]),
                Line::from(vec![
                    Span::styled(" key ",
                        Style::default().fg(palette.agent_dim)),
                    Span::styled(p.pubkey.chars().take(12).collect::<String>(),
                        Style::default().fg(palette.agent_dim)),
                ]),
            ];
            let meta_para = Paragraph::new(meta_lines);
            frame.render_widget(meta_para, p.meta_area);
        }

        // ── Footer ────────────────────────────────────────────────────
        let footer = Paragraph::new("↑↓←→ navigate  •  Enter select  •  f favorite  •  Esc back")
            .style(Style::default().fg(palette.agent_dim))
            .alignment(Alignment::Center);
        let footer_area = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(2),
            width: area.width,
            height: 1,
        };
        frame.render_widget(footer, footer_area);
    }
}
