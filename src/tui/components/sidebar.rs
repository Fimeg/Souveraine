use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};
use crate::tui::state::TuiState;

/// Render the sidebar with agent vitals
pub fn render_sidebar(frame: &mut Frame, state: &dyn TuiState, area: Rect) {
    let agent = state.agent_status();
    
    // Create sections
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),   // Agent header
            Constraint::Length(3),     // Energy gauge
            Constraint::Length(3),     // Archivist pressure
            Constraint::Length(2),     // N+1 counter
            Constraint::Min(0),        // Remaining space
        ])
        .split(area);
    
    // Agent name and mood
    let header_text = vec![
        Line::from(vec![
            Span::styled("👤 ", Style::default()),
            Span::styled(agent.name.clone(), Style::default().fg(Color::White).bold()),
        ]),
        Line::from(vec![
            Span::styled("Mood: ", Style::default().fg(Color::Gray)),
            Span::styled(agent.mood.clone(), Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("Memory: ", Style::default().fg(Color::Gray)),
            Span::styled(agent.memory_commits.to_string(), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("Tasks: ", Style::default().fg(Color::Gray)),
            Span::styled(agent.pending_tasks.to_string(), Style::default().fg(Color::Green)),
        ]),
    ];
    
    let header = Paragraph::new(header_text)
        .block(Block::default().title("Agent").borders(Borders::ALL));
    frame.render_widget(header, chunks[0]);
    
    // Energy gauge
    let energy_gauge = Gauge::default()
        .block(Block::default().title("Energy").borders(Borders::ALL))
        .gauge_style(Style::default().fg(Color::Rgb(255, 140, 66)))
        .percent(agent.energy as u16);
    frame.render_widget(energy_gauge, chunks[1]);
    
    // Archivist pressure gauge
    let pressure = state.archivist_pressure();
    let pressure_color = if pressure > 0.8 {
        Color::Red
    } else if pressure > 0.5 {
        Color::Yellow
    } else {
        Color::Green
    };
    
    let pressure_gauge = Gauge::default()
        .block(Block::default().title("Memory Pressure").borders(Borders::ALL))
        .gauge_style(Style::default().fg(pressure_color))
        .percent((pressure * 100.0) as u16);
    frame.render_widget(pressure_gauge, chunks[2]);
    
    // N+1 counter
    let n1 = state.n1_count();
    let n1_color = if n1 > 0 { Color::Yellow } else { Color::DarkGray };
    let n1_text = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("N+1 Cycles: ", Style::default().fg(Color::Gray)),
            Span::styled(n1.to_string(), Style::default().fg(n1_color).bold()),
        ]),
    ]);
    frame.render_widget(n1_text, chunks[3]);
}
