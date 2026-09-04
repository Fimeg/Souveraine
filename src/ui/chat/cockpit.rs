use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use super::{ChatState, CockpitEntry, CockpitLayout};
use ratatui::style::Color;

pub fn draw_cockpit(f: &mut Frame, state: &ChatState, area: Rect) {
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    *state.cockpit_layout.borrow_mut() = CockpitLayout {
        thinking: panes[0],
        subconscious: panes[1],
    };

    let thinking_window = panes[0].height.saturating_sub(2) as usize;
    let thinking_max = state.thinking.len().saturating_sub(thinking_window);
    let thinking_scroll = (state.thinking_scroll.get() as usize).min(thinking_max);
    state.thinking_scroll.set(thinking_scroll as u16);
    let thinking_entries: Vec<&String> = state
        .thinking
        .iter()
        .rev()
        .skip(thinking_scroll)
        .take(thinking_window)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut thinking_view: Vec<Line<'static>> = Vec::with_capacity(thinking_entries.len() * 2);
    for (i, t) in thinking_entries.iter().enumerate() {
        if t.starts_with("────") {
            if i > 0 {
                thinking_view.push(Line::from(""));
            }
            thinking_view.push(Line::from(Span::styled(
                t.to_string(),
                Style::default().fg(state.palette.agent_dim),
            )));
            thinking_view.push(Line::from(""));
        } else {
            thinking_view.push(Line::from(Span::styled(
                format!("· {}", t),
                Style::default().fg(state.palette.agent_dim),
            )));
        }
    }
    let thinking_title = if thinking_scroll > 0 {
        format!(" thinking ↑{} ", thinking_scroll)
    } else {
        " thinking ".to_string()
    };
    let thinking = Paragraph::new(thinking_view)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.palette.agent_dim))
                .title(Span::styled(
                    thinking_title,
                    Style::default()
                        .fg(state.palette.agent_dim)
                        .add_modifier(Modifier::BOLD),
                )),
        );
    f.render_widget(thinking, panes[0]);

    let visible_height = panes[1].height.saturating_sub(2) as usize;
    let sub_max = state.cockpit_log.len().saturating_sub(visible_height);
    let sub_scroll = (state.subconscious_scroll.get() as usize).min(sub_max);
    state.subconscious_scroll.set(sub_scroll as u16);
    let visible_entries: Vec<&CockpitEntry> = state
        .cockpit_log
        .iter()
        .rev()
        .skip(sub_scroll)
        .take(visible_height)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let entry_count = visible_entries.len();
    let log_view: Vec<Line<'static>> = visible_entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let base = entry.color(&state.palette);
            let dim = if entry_count > 1 {
                let age = 1.0 - (i as f32 / (entry_count - 1) as f32);
                0.4 + 0.6 * (1.0 - age)
            } else {
                1.0
            };
            let (r, g, b) = match base {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (200, 200, 200), // fallback gray for non-RGB colors
            };
            let fg = Color::Rgb(
                (r as f32 * dim) as u8,
                (g as f32 * dim) as u8,
                (b as f32 * dim) as u8,
            );
            Line::from(vec![
                Span::styled(
                    format!(" {} ", entry.prefix()),
                    Style::default().fg(fg).add_modifier(Modifier::BOLD),
                ),
                Span::styled(entry.text.clone(), Style::default().fg(fg)),
            ])
        })
        .collect();
    let sub_title = if sub_scroll > 0 {
        format!(" subconscious ↑{} ", sub_scroll)
    } else {
        " subconscious ".to_string()
    };
    let subconscious = Paragraph::new(log_view).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(state.palette.surfacing))
            .title(Span::styled(
                sub_title,
                Style::default()
                    .fg(state.palette.surfacing)
                    .add_modifier(Modifier::BOLD),
            )),
    );
    f.render_widget(subconscious, panes[1]);
}
