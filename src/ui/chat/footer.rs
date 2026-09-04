use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::ChatState;

pub fn draw_footer(f: &mut Frame, state: &ChatState, area: Rect) {
    let pressure_pct = (state.pressure * 100.0) as u16;
    let ctx_display = match state.context_limit {
        Some(limit) => format!("ctx {}% of {}", pressure_pct, limit),
        None => format!("ctx {}%", pressure_pct),
    };
    let cockpit_hint = if state.cockpit {
        "Tab close cockpit"
    } else {
        "Tab cockpit"
    };
    let tool_hint = if state.tool_cards_expanded {
        "CTRL+T collapse tools"
    } else {
        "CTRL+T expand tools"
    };
    let mut spans = vec![
        Span::styled(
            format!(" Esc menu · Enter send · {cockpit_hint} · {tool_hint} "),
            Style::default().fg(state.palette.agent_dim),
        ),
        Span::raw("│  "),
        Span::styled(
            format!("conv {}", short(&state.conversation_id)),
            Style::default().fg(state.palette.agent_dim),
        ),
        Span::raw("  │  "),
        Span::styled(ctx_display, Style::default().fg(state.palette.agent_dim)),
    ];
    if state.scroll > 0 {
        spans.push(Span::raw("  │  "));
        spans.push(Span::styled(
            format!("↓ {} below", state.scroll),
            Style::default().fg(state.palette.agent_primary),
        ));
    }
    if state
        .copy_flash
        .map(|t| t.elapsed().as_millis() < 1600)
        .unwrap_or(false)
    {
        spans.push(Span::raw("  │  "));
        spans.push(Span::styled(
            "⧉ copied",
            Style::default()
                .fg(state.palette.tool_accent)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let footer = Line::from(spans);
    f.render_widget(Paragraph::new(footer).alignment(Alignment::Center), area);
}

pub fn short(s: &str) -> String {
    if s.len() <= 8 {
        s.to_string()
    } else {
        s[..8].to_string()
    }
}
