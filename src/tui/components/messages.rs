use crate::tui::state::{DisplayMessage, MessageRole, ToolStatus, TuiState};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

/// Render the full message list for the chat screen
pub fn render_messages(frame: &mut Frame, state: &dyn TuiState, area: Rect) {
    let messages = state.messages();
    let agent_name = &state.agent_status().name;
    let mut text_lines: Vec<Line> = Vec::new();

    for msg in messages {
        let msg_lines = render_message(msg, area.width, agent_name);
        text_lines.extend(msg_lines);
        text_lines.push(Line::from("")); // spacing between messages
    }
    
    // Add streaming indicator if active
    if state.is_streaming() && !state.streaming_text().is_empty() {
        let streaming_lines = render_streaming_text(state.streaming_text(), area.width);
        text_lines.extend(streaming_lines);
    }
    
    let paragraph = Paragraph::new(text_lines)
        .wrap(Wrap { trim: true })
        .scroll((state.scroll_offset() as u16, 0));
    
    frame.render_widget(paragraph, area);
}

/// Render a single message based on its role
fn render_message(msg: &DisplayMessage, width: u16, agent_name: &str) -> Vec<Line> {
    match msg.role {
        MessageRole::User => render_user_message(msg, width),
        MessageRole::Assistant => render_assistant_message(msg, width, agent_name),
        MessageRole::Reasoning => render_reasoning_block(msg, width, false),
        MessageRole::ToolResult => render_tool_result(msg, width),
        MessageRole::System => render_system_message(msg, width),
    }
}

/// User message - right aligned, blue/cyan styling
fn render_user_message(msg: &DisplayMessage, width: u16) -> Vec<Line> {
    let style = Style::default().fg(Color::Cyan);
    let mut lines = vec![Line::from(vec![
        Span::styled("You ", style.add_modifier(Modifier::BOLD)),
    ])];
    
    for line in msg.content.lines() {
        lines.push(Line::from(Span::styled(line.to_string(), style)));
    }
    
    lines
}

/// Assistant message - left aligned, white with tool chips
fn render_assistant_message(msg: &DisplayMessage, width: u16, agent_name: &str) -> Vec<Line> {
    let name = agent_name;
    let header_style = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{} ", name), header_style),
    ])];
    
    // Render markdown content
    let content_lines = render_markdown(&msg.content, width.saturating_sub(2));
    lines.extend(content_lines);
    
    // Add tool call chips if present
    if !msg.tool_calls.is_empty() {
        lines.push(Line::from(""));
        lines.extend(render_tool_chips(&msg.tool_calls, width));
    }
    
    lines
}

/// Render tool call chips: "tools: search_files · read_file · +2 more"
fn render_tool_chips(tools: &[ToolCallDisplay], width: u16) -> Vec<Line> {
    if tools.is_empty() {
        return Vec::new();
    }
    
    const TOOL_SEPARATOR: &str = " · ";
    let label = if tools.len() == 1 { "tool:" } else { "tools:" };
    
    let prefix_style = Style::default().fg(Color::DarkGray);
    let separator_style = Style::default().fg(Color::DarkGray);
    let name_style = Style::default().fg(Color::Cyan);
    let running_style = Style::default().fg(Color::Yellow);
    let error_style = Style::default().fg(Color::Red);
    
    let mut spans = vec![
        Span::styled(format!("  {} ", label), prefix_style),
    ];
    
    let mut current_width = 2 + label.len() + 1;
    let max_width = width as usize - 4;
    let mut shown = 0;
    
    for (idx, tool) in tools.iter().enumerate() {
        let separator_width = if shown == 0 { 0 } else { TOOL_SEPARATOR.len() };
        let remaining = tools.len().saturating_sub(idx + 1);
        let more_label = if remaining > 0 {
            format!("{}+{} more", TOOL_SEPARATOR, remaining)
        } else {
            String::new()
        };
        
        let required = separator_width + tool.name.len() + more_label.len();
        
        if current_width + required <= max_width {
            if shown > 0 {
                spans.push(Span::styled(TOOL_SEPARATOR, separator_style));
                current_width += separator_width;
            }
            
            let style = match tool.status {
                ToolStatus::Running => running_style,
                ToolStatus::Error => error_style,
                _ => name_style,
            };
            
            spans.push(Span::styled(&tool.name, style));
            current_width += tool.name.len();
            shown += 1;
        } else {
            break;
        }
    }
    
    if shown < tools.len() {
        let remaining = tools.len() - shown;
        let more_text = if shown == 0 {
            format!("+{} more", remaining)
        } else {
            format!("{}+{} more", TOOL_SEPARATOR, remaining)
        };
        spans.push(Span::styled(more_text, separator_style));
    }
    
    vec![Line::from(spans)]
}

/// Reasoning block - collapsible thinking indicator (DeepSeek style)
fn render_reasoning_block(msg: &DisplayMessage, width: u16, is_expanded: bool) -> Vec<Line> {
    let icon = if is_expanded { "▼" } else { "▶" };
    let mut lines = vec![Line::from(vec![
        Span::styled(icon, Style::default().fg(Color::Yellow)),
        Span::styled(" Thinking...", Style::default().fg(Color::DarkGray).italic()),
    ])];
    
    if is_expanded {
        let thinking_style = Style::default().fg(Color::DarkGray).italic();
        for line in msg.content.lines() {
            lines.push(Line::from(Span::styled(
                format!("  {}", line),
                thinking_style,
            )));
        }
    }
    
    lines
}

/// Tool result - monospace output with border
fn render_tool_result(msg: &DisplayMessage, width: u16) -> Vec<Line> {
    let style = Style::default().fg(Color::Gray);
    let mut lines = vec![
        Line::from(Span::styled("  ┌─ Tool Output ─┐", style.dim())),
    ];
    
    for line in msg.content.lines().take(10) {
        lines.push(Line::from(vec![
            Span::styled("  │ ", style.dim()),
            Span::styled(line.to_string(), style),
        ]));
    }
    
    if msg.content.lines().count() > 10 {
        lines.push(Line::from(Span::styled("  │ ... (truncated)", style.dim())));
    }
    
    lines.push(Line::from(Span::styled("  └─────────────────┘", style.dim())));
    lines
}

/// System message - centered, dimmed
fn render_system_message(msg: &DisplayMessage, width: u16) -> Vec<Line> {
    let style = Style::default().fg(Color::DarkGray).dim();
    msg.content
        .lines()
        .map(|line| Line::from(Span::styled(line.to_string(), style)).alignment(Alignment::Center))
        .collect()
}

/// Streaming text with cursor indicator
fn render_streaming_text(text: &str, width: u16) -> Vec<Line> {
    let mut lines = render_markdown(text, width.saturating_sub(2));
    
    // Add blinking cursor to last line
    if let Some(last) = lines.last_mut() {
        last.spans.push(Span::styled("▌", Style::default().fg(Color::Green)));
    }
    
    lines
}

/// Simple markdown parser (no deps)
fn render_markdown(content: &str, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut code_content = String::new();
    
    for line in content.lines() {
        if line.starts_with("```") {
            if in_code_block {
                let code_lines = render_code_block(&code_content);
                lines.extend(code_lines);
                code_content.clear();
                in_code_block = false;
            } else {
                in_code_block = true;
            }
        } else if in_code_block {
            code_content.push_str(line);
            code_content.push('\n');
        } else {
            lines.push(render_markdown_line(line));
        }
    }
    
    if in_code_block && !code_content.is_empty() {
        let code_lines = render_code_block(&code_content);
        lines.extend(code_lines);
    }
    
    lines
}

fn render_markdown_line(line: &str) -> Line<'static> {
    // Basic: bold **text**
    if let Some(start) = line.find("**") {
        if let Some(end) = line[start+2..].find("**") {
            let before = &line[..start];
            let bold = &line[start+2..start+2+end];
            let after = &line[start+2+end+2..];
            
            return Line::from(vec![
                Span::raw(before.to_string()),
                Span::styled(bold.to_string(), Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(after.to_string()),
            ]);
        }
    }
    
    Line::from(line.to_string())
}

fn render_code_block(content: &str) -> Vec<Line<'static>> {
    let style = Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC);
    let mut lines = vec![Line::from(Span::styled("  ┌─ Code ─┐", style))];
    
    for line in content.lines().take(8) {
        lines.push(Line::from(vec![
            Span::styled("  │ ", style),
            Span::styled(line.to_string(), style),
        ]));
    }
    
    if content.lines().count() > 8 {
        lines.push(Line::from(Span::styled("  │ ...", style)));
    }
    
    lines.push(Line::from(Span::styled("  └────────┘", style)));
    lines
}
