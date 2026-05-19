use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph, Wrap},
    Frame,
};

use ratatui_image::{Resize, StatefulImage};

use super::App;

impl App {
    pub(super) fn draw_welcome_mut(&mut self, frame: &mut Frame) {
        let bg = Block::default().style(Style::default().bg(Color::Black));
        frame.render_widget(bg, frame.size());

        if frame.size().width >= 100 {
            self.draw_welcome_wide(frame);
        } else {
            self.draw_welcome_stacked(frame);
        }

        if let Some(err) = &self.chat_error {
            let area = frame.size();
            let err_para = Paragraph::new(format!(" chat connect failed: {} ", err))
                .style(Style::default().fg(Color::Rgb(220, 100, 100)))
                .alignment(Alignment::Center);
            let row = Rect {
                x: area.x,
                y: area.y + area.height.saturating_sub(2),
                width: area.width,
                height: 1,
            };
            frame.render_widget(err_para, row);
        }
    }

    pub(super) fn welcome_menu_items() -> Vec<(&'static str, &'static str, bool)> {
        vec![
            ("💬 Chat",       "Talk with your agent",   true),
            ("📅 Schedule",   "Cron jobs & tasks",      true),
            ("⚙️  Settings",   "Configure",              true),
            ("🛋️  Therapy",    "Agent therapy session",  false),
            ("⏰ Agent Time", "Give your agent time",   false),
        ]
    }

    pub(super) fn build_menu_list(&self, title: &str, palette: &crate::ui::chat::ChatPalette) -> List<'static> {
        let items: Vec<ListItem> = Self::welcome_menu_items()
            .into_iter()
            .enumerate()
            .map(|(i, (label, desc, available))| {
                let selected = i == self.menu_selected;
                let label_style = if !available {
                    Style::default().fg(palette.tool_dim)
                } else if selected {
                    Style::default()
                        .fg(palette.agent_primary)
                        .bg(palette.bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                let desc_style = Style::default().fg(palette.agent_dim);
                let mut spans = vec![
                    Span::styled(format!(" {} ", label), label_style),
                    Span::styled(format!("- {}", desc), desc_style),
                ];
                if !available {
                    spans.push(Span::styled(
                        "  (coming soon)",
                        Style::default()
                            .fg(palette.agent_dim)
                            .add_modifier(Modifier::ITALIC),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();

        List::new(items).block(
            Block::default()
                .title(format!(" {} ", title))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(palette.agent_primary).add_modifier(Modifier::DIM)),
        )
    }

    pub(super) fn render_stat_cards(&self, frame: &mut Frame, area: Rect) {
        let atm = self.presence.atmosphere;
        let cards = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ])
            .split(area);

        let energy_color = match self.agent_status.energy {
            0..=30 => Color::Red,
            31..=60 => Color::Yellow,
            _ => Color::Green,
        };
        let energy = Gauge::default()
            .block(
                Block::default()
                    .title(" Energy ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded),
            )
            .gauge_style(Style::default().fg(energy_color).bg(atm.bg_tint()))
            .percent(self.agent_status.energy as u16)
            .label(format!("{}%", self.agent_status.energy));
        frame.render_widget(energy, cards[0]);

        let mood = Paragraph::new(format!("\n◌\n\n{}", self.agent_status.mood))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title(" State ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(atm.secondary())),
            );
        frame.render_widget(mood, cards[1]);

        let memory_label = match &self.agent_status.last_commit {
            Some(c) => format!("\n💾\n\n{} files\n{}", self.agent_status.memory_commits, c),
            None => format!("\n💾\n\n{} files", self.agent_status.memory_commits),
        };
        let memory = Paragraph::new(memory_label)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title(" Memory ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(atm.secondary())),
            );
        frame.render_widget(memory, cards[2]);

        let agents_card = Paragraph::new(format!(
            "\n👥\n\n{} agent{}\non {}",
            self.agent_status.agent_count,
            if self.agent_status.agent_count == 1 { "" } else { "s" },
            self.agent_status.mode,
        ))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .title(" Backend ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(atm.secondary())),
        );
        frame.render_widget(agents_card, cards[3]);
    }

    pub(super) fn render_recent_activity(&self, frame: &mut Frame, area: Rect) {
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);
        let text = if self.agent_status.recent_activity.is_empty() {
            "(no recent activity — open Chat to begin)".to_string()
        } else {
            self.agent_status.recent_activity.join("\n")
        };
        let para = Paragraph::new(text)
            .style(Style::default().fg(palette.agent_dim))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(" Recent Activity ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(palette.agent_primary).add_modifier(Modifier::DIM)),
            );
        frame.render_widget(para, area);
    }

    pub(super) fn render_card_image_cover(
        &mut self,
        frame: &mut Frame,
        agent_id: &str,
        area: Rect,
    ) {
        let Some(picker) = self.image_picker.as_ref() else { return };
        let Some(raw) = self.raw_card_images.get(agent_id).cloned() else {
            if let Some(proto) = self.card_images.get_mut(agent_id) {
                frame.render_stateful_widget(
                    StatefulImage::default().resize(Resize::Crop(None)),
                    area,
                    proto,
                );
            }
            return;
        };

        let key = format!("{}:{}x{}", agent_id, area.width, area.height);

        if !self.cover_protocols.contains_key(&key) {
            let fs = picker.font_size();
            let target_px_w = area.width as u32 * fs.width as u32;
            let target_px_h = area.height as u32 * fs.height as u32;

            let sx = target_px_w as f64 / raw.width() as f64;
            let sy = target_px_h as f64 / raw.height() as f64;
            let scale = sx.max(sy);
            let scaled_w = (raw.width() as f64 * scale).round() as u32;
            let scaled_h = (raw.height() as f64 * scale).round() as u32;
            let scaled = raw.resize_exact(scaled_w, scaled_h, image::imageops::FilterType::Lanczos3);

            let cropped = scaled.crop_imm(0, 0, target_px_w.min(scaled_w), target_px_h.min(scaled_h));

            let proto = picker.new_resize_protocol(cropped);
            self.cover_protocols.insert(key.clone(), (area.width, area.height, proto));
        }

        if let Some((_, _, proto)) = self.cover_protocols.get_mut(&key) {
            frame.render_stateful_widget(
                StatefulImage::default().resize(Resize::Fit(None)),
                area,
                proto,
            );
        }
    }

    pub(super) fn render_portrait_card(&mut self, frame: &mut Frame, area: Rect) {
        use crate::ui::portrait;
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);
        let border_col = if self.presence.subconscious_active {
            palette.surfacing
        } else {
            palette.agent_primary
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_col).add_modifier(Modifier::DIM));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height < 4 || inner.width < 4 {
            return;
        }

        let photo_h = inner.height.saturating_sub(1);
        let portrait_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: photo_h,
        };

        let active_id = self.agent_id_by_name(&self.presence.name)
            .or_else(|| self.agent_id_by_name(&self.agent_pref));

        let rgp_rendered = if let Some(ref mut g) = self.rgp_portrait {
            if g.is_active() {
                g.apply_posture(self.presence.posture);
                g.render(portrait_area, frame.buffer_mut());
                true
            } else { false }
        } else { false };

        let rendered = if rgp_rendered {
            true
        } else {
            active_id.as_ref().and_then(|id| {
                let picker = self.image_picker.as_ref()?;

                if self.raw_card_images.contains_key(id) {
                    self.render_card_image_cover(frame, id, portrait_area);
                    return Some(true);
                }

                let assets_dir = Self::agent_assets_dir(id)?;
                let key = crate::ui::expressions::ExpressionKey::from_presence(&self.presence);
                if let Some(proto) = self.expression_cache.resolve(id, key, picker, &assets_dir) {
                    frame.render_stateful_widget(
                        StatefulImage::default().resize(Resize::Scale(None)),
                        portrait_area,
                        proto,
                    );
                    return Some(true);
                }

                None
            }).is_some()
        };
        if !rendered {
            let scale = (portrait_area.width / portrait::PORTRAIT_W)
                .min((2 * portrait_area.height) / portrait::PORTRAIT_H)
                .max(1);
            portrait::render_scaled(frame.buffer_mut(), portrait_area, &self.presence, scale);
        }

        let glyph = if self.presence.subconscious_active { "◈" } else { "·" };
        let name_area = Rect {
            x: inner.x,
            y: inner.y + photo_h,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {} ", glyph), Style::default().fg(border_col)),
                Span::styled(
                    self.presence.name.clone(),
                    Style::default().fg(border_col).add_modifier(Modifier::BOLD),
                ),
            ]))
            .alignment(Alignment::Center),
            name_area,
        );
    }

    fn draw_welcome_wide(&mut self, frame: &mut Frame) {
        let area = frame.size();
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);

        let outer = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(20),
                Constraint::Length(1),
            ])
            .split(area);

        let (tr, tg, _tb) = match palette.agent_primary { Color::Rgb(r, g, b) => (r, g, b), _ => (255, 140, 66) };
        let breathe = self.presence.animator.breathe(3000);
        let glow = (tg as f32 * 0.6 + breathe * 40.0) as u8;
        let title = Paragraph::new(vec![
            Line::from(Span::styled(
                "S O U V E R A I N E",
                Style::default()
                    .fg(Color::Rgb(tr, glow.max(tr / 3), tr / 4))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "La souveraineté de la conscience",
                Style::default().fg(palette.agent_dim),
            )),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(title, outer[0]);

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(outer[1]);

        self.render_portrait_card(frame, body[0]);

        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6),
                Constraint::Min(6),
                Constraint::Length(9),
            ])
            .split(body[1]);

        self.render_stat_cards(frame, right[0]);
        self.render_recent_activity(frame, right[1]);
        let menu = self.build_menu_list("Menu", &palette);
        frame.render_widget(menu, right[2]);

        let footer = Paragraph::new(
            "↑↓ Navigate • Enter select • a Add • i Inspect • p Presence • q Quit",
        )
        .style(Style::default().fg(palette.agent_dim))
        .alignment(Alignment::Center);
        frame.render_widget(footer, outer[2]);
    }

    fn draw_welcome_stacked(&mut self, frame: &mut Frame) {
        let area = frame.size();
        let palette = crate::ui::chat::ChatPalette::from_atmosphere(self.presence.atmosphere);

        let avatar_card_w: u16 = (area.width * 50 / 100).min(48).max(28);
        let photo_h: u16 = (avatar_card_w / 2 + 2).clamp(10, 18);
        let avatar_card_h: u16 = photo_h + 2;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(6),
                Constraint::Length(avatar_card_h),
                Constraint::Min(5),
                Constraint::Length(9),
                Constraint::Length(1),
            ])
            .split(area);

        let (tr, tg, _tb) = match palette.agent_primary { Color::Rgb(r, g, b) => (r, g, b), _ => (255, 140, 66) };
        let breathe = self.presence.animator.breathe(3000);
        let glow = (tg as f32 * 0.6 + breathe * 40.0) as u8;
        let title = Paragraph::new(vec![
            Line::from(Span::styled(
                "S O U V E R A I N E",
                Style::default()
                    .fg(Color::Rgb(tr, glow.max(tr / 3), tr / 4))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "La souveraineté de la conscience",
                Style::default().fg(palette.agent_dim),
            )),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(title, chunks[0]);

        self.render_stat_cards(frame, chunks[1]);

        if chunks[2].width >= avatar_card_w {
            let card_x = chunks[2].x + (chunks[2].width - avatar_card_w) / 2;
            let card_area = Rect {
                x: card_x,
                y: chunks[2].y,
                width: avatar_card_w,
                height: avatar_card_h.min(chunks[2].height),
            };
            self.render_portrait_card(frame, card_area);
        }

        self.render_recent_activity(frame, chunks[3]);
        let menu = self.build_menu_list("Menu", &palette);
        frame.render_widget(menu, chunks[4]);

        let footer = Paragraph::new(
            "↑↓ • Enter • a Add • i Inspect • p Presence • q Quit",
        )
        .style(Style::default().fg(palette.agent_dim))
        .alignment(Alignment::Center);
        frame.render_widget(footer, chunks[5]);
    }
}
