use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use super::wrap::wrap_words;
use super::{BtwState, ChatState, Overlay, SPINNER};

pub fn draw_btw_pane(f: &mut Frame, state: &ChatState, area: Rect) {
    let pane_w = (area.width * 3 / 4)
        .max(40)
        .min(area.width.saturating_sub(6));
    let pane_h = (area.height * 3 / 5)
        .max(12)
        .min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(pane_w)) / 2;
    let y = (area.height.saturating_sub(pane_h)) / 2;
    let pane_area = Rect {
        x,
        y,
        width: pane_w,
        height: pane_h,
    };

    f.render_widget(Clear, pane_area);

    let inner_w = pane_w.saturating_sub(4) as usize;

    let (title, body, border_color) = match &state.btw_state {
        BtwState::Forking { question } => {
            let spinner = SPINNER[(state.tick as usize / 2) % SPINNER.len()];
            (
                format!(" btw — {} ", question.chars().take(40).collect::<String>()),
                vec![Line::from(vec![Span::styled(
                    format!(" {} forking...", spinner),
                    Style::default().fg(state.palette.agent_dim),
                )])],
                state.palette.agent_primary,
            )
        }
        BtwState::Streaming {
            question,
            response_so_far,
        } => {
            let truncated: String = response_so_far.chars().take(800).collect();
            let wrapped = wrap_text(&truncated, inner_w);
            let q_label = question.chars().take(40).collect::<String>();
            (
                format!(" btw — {} ", q_label),
                wrapped,
                state.palette.tool_accent,
            )
        }
        BtwState::Complete {
            question,
            response,
            forked_id,
        } => {
            let truncated: String = response.chars().take(800).collect();
            let wrapped = wrap_text(&truncated, inner_w);
            let q_label = question.chars().take(40).collect::<String>();
            let fork_label = if forked_id.is_empty() {
                String::new()
            } else {
                format!(" fork: {}", &forked_id[..forked_id.len().min(8)])
            };
            (
                format!(" btw — {} {}", q_label, fork_label),
                {
                    let mut lines = wrapped;
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "[esc] dismiss  ·  [j] jump to fork",
                        Style::default().fg(state.palette.agent_dim),
                    )));
                    lines
                },
                state.palette.surfacing,
            )
        }
        BtwState::Error { question, error } => {
            let q_label = question.chars().take(40).collect::<String>();
            (
                format!(" btw — {} ", q_label),
                vec![Line::from(Span::styled(
                    format!(" Error: {}", error),
                    Style::default().fg(state.palette.compaction),
                ))],
                state.palette.compaction,
            )
        }
        BtwState::Idle => return,
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        );

    let para = Paragraph::new(body)
        .block(block)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false });
    f.render_widget(para, pane_area);
}

pub fn draw_overlay(f: &mut Frame, state: &ChatState, full_area: Rect, input_area: Rect) {
    match &state.overlay {
        Overlay::None => {}
        Overlay::SlashComplete { selected, matches } => {
            let count = matches.len().min(8);
            let height = count as u16 + 2;
            let width = 40u16.min(full_area.width.saturating_sub(4));
            let x = input_area.x + 1;
            let y = input_area.y.saturating_sub(height);
            let area = Rect {
                x,
                y,
                width,
                height,
            };

            f.render_widget(Clear, area);

            let items: Vec<Line<'static>> = matches
                .iter()
                .enumerate()
                .take(count)
                .map(|(i, cmd)| {
                    let sel = i == *selected;
                    let sel_fg = state.palette.agent_primary;
                    let sel_bg = state.palette.bg;
                    let style = if sel {
                        Style::default()
                            .fg(sel_fg)
                            .bg(sel_bg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    let hint_style = if sel {
                        Style::default().fg(state.palette.agent_dim).bg(sel_bg)
                    } else {
                        Style::default().fg(state.palette.agent_dim)
                    };
                    Line::from(vec![
                        Span::styled(format!(" {} ", cmd.name), style),
                        Span::styled(format!(" {}", cmd.hint), hint_style),
                    ])
                })
                .collect();

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.palette.agent_dim));
            let para = Paragraph::new(items).block(block);
            f.render_widget(para, area);
        }
        Overlay::ConversationPicker {
            selected,
            conversations,
        } => {
            let count = conversations.len();
            let visible = count.min(12);
            let height = visible as u16 + 4;
            let width = (full_area.width * 3 / 4)
                .max(40)
                .min(full_area.width.saturating_sub(4));
            let x = (full_area.width.saturating_sub(width)) / 2;
            let y = (full_area.height.saturating_sub(height)) / 2;
            let area = Rect {
                x,
                y,
                width,
                height,
            };

            f.render_widget(Clear, area);

            let inner_width = (width as usize).saturating_sub(4);
            let mut lines: Vec<Line<'static>> = Vec::new();
            lines.push(Line::from(Span::styled(
                " Conversations — ↑↓ select · Enter resume · n new · Esc dismiss",
                Style::default()
                    .fg(state.palette.agent_dim)
                    .add_modifier(Modifier::ITALIC),
            )));

            let scroll_offset = if *selected >= visible {
                selected + 1 - visible
            } else {
                0
            };
            for (i, conv) in conversations
                .iter()
                .enumerate()
                .skip(scroll_offset)
                .take(visible)
            {
                let sel = i == *selected;
                let short_id = &conv.id[..8.min(conv.id.len())];
                let summary = conv.summary.as_deref().unwrap_or("(no summary)");
                let label = format!(" {} · {} msgs · {}", short_id, conv.message_count, summary,);
                let truncated = if label.chars().count() > inner_width {
                    let mut s: String = label.chars().take(inner_width.saturating_sub(1)).collect();
                    s.push('…');
                    s
                } else {
                    label
                };

                let style = if sel {
                    Style::default()
                        .fg(state.palette.agent_primary)
                        .bg(state.palette.bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                lines.push(Line::from(Span::styled(truncated, style)));
            }

            let block = Block::default()
                .title(Span::styled(
                    " Resume ",
                    Style::default()
                        .fg(state.palette.agent_primary)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(state.palette.agent_primary));
            let para = Paragraph::new(lines).block(block);
            f.render_widget(para, area);
        }
    }
}

pub fn draw_esc_overlay(f: &mut Frame, state: &ChatState, area: Rect) {
    let overlay_w = 28.min(area.width.saturating_sub(4));
    let overlay_h = 7;
    let ox = area.x + (area.width - overlay_w) / 2;
    let oy = area.y + (area.height.saturating_sub(overlay_h)) / 2;
    let overlay_area = Rect {
        x: ox,
        y: oy,
        width: overlay_w,
        height: overlay_h,
    };

    f.render_widget(Clear, overlay_area);

    let pal = &state.palette;
    let lines = vec![
        Line::from(Span::styled(
            " Turn in progress ",
            Style::default()
                .fg(pal.surfacing)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  [i]  Raise hand",
            Style::default().fg(pal.agent_primary),
        )),
        Line::from(Span::styled(
            "  [m]  Menu",
            Style::default().fg(pal.user_accent),
        )),
        Line::from(Span::styled(
            "  [c]  Cancel",
            Style::default().fg(pal.agent_dim),
        )),
    ];

    let para = Paragraph::new(lines).alignment(Alignment::Left).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(pal.agent_dim)),
    );
    f.render_widget(para, overlay_area);
}

fn wrap_text(text: &str, max_width: usize) -> Vec<Line<'static>> {
    wrap_words(text, max_width.max(1))
        .into_iter()
        .map(|line| Line::from(Span::styled(line, Style::default().fg(Color::White))))
        .collect()
}
