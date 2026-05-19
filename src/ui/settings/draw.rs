use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use super::types::{Category, FieldLoc, SettingsMode, PanelFocus};
use super::view::SettingsView;

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(frame: &mut Frame, view: &SettingsView) {
    let area = frame.size();
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5), Constraint::Length(3)])
        .split(area);

    // Header
    draw_header(frame, vert[0], view);
    // Body: two panels
    if area.width >= 80 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
            .split(vert[1]);
        draw_category_list(frame, body[0], view);
        draw_field_panel(frame, body[1], view);
    } else {
        // Narrow terminal: full-width field panel with category switcher hint.
        draw_field_panel(frame, vert[1], view);
    }
    // Footer
    draw_footer(frame, vert[2], view);
}

fn draw_header(frame: &mut Frame, area: Rect, view: &SettingsView) {
    let title = if view.dirty { " Settings (unsaved) " } else { " Settings " };
    let header = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(view.palette.agent_primary));
    frame.render_widget(header, area);
}

fn draw_category_list(frame: &mut Frame, area: Rect, view: &SettingsView) {
    let categories = Category::all();
    let focused = view.focus == PanelFocus::Categories;
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    for (i, cat) in categories.iter().enumerate() {
        let selected = i == view.category_idx;
        let (prefix, style) = if selected && focused {
            (">", Style::default().fg(view.palette.agent_primary).add_modifier(Modifier::BOLD))
        } else if selected {
            (">", Style::default().fg(view.palette.agent_primary).add_modifier(Modifier::DIM))
        } else {
            (" ", Style::default().fg(view.palette.agent_dim))
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {}  {}", prefix, cat.label()), style),
        ]));
    }

    let border_style = if focused {
        Style::default().fg(view.palette.agent_primary)
    } else {
        Style::default().fg(view.palette.agent_dim)
    };

    let body = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Categories ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style),
        );
    frame.render_widget(body, area);
}

fn draw_field_panel(frame: &mut Frame, area: Rect, view: &SettingsView) {
    let cat = view.selected_category();
    let fields = view.fields_for_category(cat);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    let p = &view.palette;
    for (i, (loc, value)) in fields.iter().enumerate() {
        let is_selected = i == view.field_idx && view.focus == PanelFocus::Fields;
        let key_style = if is_selected {
            Style::default().fg(p.agent_primary).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.agent_dim)
        };

        let mut spans = vec![
            Span::styled(format!("  {:25}", loc.label()), key_style),
        ];

        // If this field is being edited, show the buffer with cursor.
        if let SettingsMode::Editing { loc: edit_loc, buffer, cursor } = &view.mode {
            if *edit_loc == *loc {
                let before = &buffer[..*cursor];
                let after = &buffer[*cursor..];
                spans.push(Span::styled(
                    before.to_string(),
                    Style::default().fg(p.agent_primary).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    '▏'.to_string(),
                    Style::default().fg(Color::White).add_modifier(Modifier::SLOW_BLINK),
                ));
                spans.push(Span::styled(
                    after.to_string(),
                    Style::default().fg(p.agent_primary).add_modifier(Modifier::BOLD),
                ));
                lines.push(Line::from(spans));
                continue;
            }
        }

        spans.extend(value.display_spans(is_selected, &view.palette));
        if loc.applies_live() {
            spans.push(Span::styled("  ◆", Style::default().fg(Color::Rgb(120, 200, 120))));
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ◆ applies live · other changes take effect on restart",
        Style::default().fg(p.agent_dim).add_modifier(Modifier::ITALIC),
    )));

    let focused = view.focus == PanelFocus::Fields;
    let border_color = if view.dirty {
        let (r, g, b) = match p.agent_primary { Color::Rgb(r, g, b) => (r, g, b), _ => (255, 140, 66) };
        Color::Rgb(r, (g as u16 + 40).min(255) as u8, b.saturating_sub(20))
    } else if focused {
        p.agent_primary
    } else {
        p.agent_dim
    };
    let body = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(format!(" {} ", cat.label()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border_color)),
        );
    frame.render_widget(body, area);
}

fn draw_footer(frame: &mut Frame, area: Rect, view: &SettingsView) {
    let msg: String = match &view.mode {
        SettingsMode::Browse => {
            let base = match view.focus {
                PanelFocus::Categories => " ↑↓ navigate · → or Enter select · y save · Ctrl+S save & quit · Esc back",
                PanelFocus::Fields if area.width >= 80 => " ↑↓ field · ←→ cycle/focus · Enter edit · y save · Ctrl+S save & quit · Esc back",
                PanelFocus::Fields => " ↑↓ · ←→ cycle · Enter · y · Ctrl+S · Esc",
            };
            if matches!(view.selected_category(), Category::Bifrost) {
                if view.models_fetching {
                    format!("{base} · fetching models…")
                } else if view.available_models.is_empty() {
                    format!("{base} · [r] fetch models")
                } else {
                    format!("{base} · [r] refresh  {} models", view.available_models.len())
                }
            } else {
                base.to_string()
            }
        }
        SettingsMode::Editing { .. } => " Enter confirm · Tab confirm+next · Esc cancel".to_string(),
        SettingsMode::ConfirmDiscard => " Unsaved changes — y discard & quit · Esc cancel".to_string(),
        SettingsMode::Status { msg, is_error } => {
            if *is_error {
                return draw_error(frame, area, msg);
            } else {
                return draw_saved(frame, area, msg);
            }
        }
    };

    let footer = Paragraph::new(format!(" {msg}"))
        .style(Style::default().fg(view.palette.agent_dim));
    frame.render_widget(footer, area);
}

fn draw_saved(frame: &mut Frame, area: Rect, msg: &str) {
    let text = Paragraph::new(format!(" ✓ {msg}"))
        .style(Style::default().fg(Color::Rgb(120, 200, 120)));
    frame.render_widget(text, area);
}

fn draw_error(frame: &mut Frame, area: Rect, msg: &str) {
    let text = Paragraph::new(format!(" ✗ {msg}"))
        .style(Style::default().fg(Color::Rgb(220, 100, 100)));
    frame.render_widget(text, area);
}
