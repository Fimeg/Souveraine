use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::tui::state::{TuiState, Screen};

/// Multi-line input area with history support
pub struct InputArea {
    pub content: String,
    pub cursor_pos: usize,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub suggestions: Vec<String>,
    pub suggestion_selected: usize,
}

impl InputArea {
    pub fn new() -> Self {
        Self {
            content: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_index: None,
            suggestions: Vec::new(),
            suggestion_selected: 0,
        }
    }
    
    pub fn push_char(&mut self, c: char) {
        self.content.insert(self.cursor_pos, c);
        self.cursor_pos += 1;
        self.update_suggestions();
    }
    
    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
            self.content.remove(self.cursor_pos);
            self.update_suggestions();
        }
    }
    
    pub fn delete(&mut self) {
        if self.cursor_pos < self.content.len() {
            self.content.remove(self.cursor_pos);
            self.update_suggestions();
        }
    }
    
    pub fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }
    
    pub fn move_cursor_right(&mut self) {
        if self.cursor_pos < self.content.len() {
            self.cursor_pos += 1;
        }
    }
    
    pub fn move_cursor_start(&mut self) {
        self.cursor_pos = 0;
    }
    
    pub fn move_cursor_end(&mut self) {
        self.cursor_pos = self.content.len();
    }
    
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        
        let new_index = match self.history_index {
            None => self.history.len() - 1,
            Some(i) if i > 0 => i - 1,
            Some(_) => 0,
        };
        
        self.history_index = Some(new_index);
        self.content = self.history[new_index].clone();
        self.cursor_pos = self.content.len();
    }
    
    pub fn history_down(&mut self) {
        if let Some(i) = self.history_index {
            if i + 1 < self.history.len() {
                self.history_index = Some(i + 1);
                self.content = self.history[i + 1].clone();
                self.cursor_pos = self.content.len();
            } else {
                self.history_index = None;
                self.content.clear();
                self.cursor_pos = 0;
            }
        }
    }
    
    pub fn submit(&mut self) -> String {
        let content = self.content.clone();
        if !content.is_empty() {
            self.history.push(content.clone());
        }
        self.content.clear();
        self.cursor_pos = 0;
        self.history_index = None;
        self.suggestions.clear();
        content
    }
    
    pub fn clear(&mut self) {
        self.content.clear();
        self.cursor_pos = 0;
    }
    
    fn update_suggestions(&mut self) {
        // TODO: Implement based on input mode
        self.suggestions.clear();
        if self.content.starts_with('/') {
            self.suggestions = vec![
                "/agent".to_string(),
                "/model".to_string(),
                "/clear".to_string(),
            ];
        }
    }
}

/// Render input area with prompt, multi-line support, and suggestions
pub fn render_input(
    frame: &mut Frame,
    state: &dyn TuiState,
    input_area: &InputArea,
    area: Rect,
) {
    let next_num = state.messages().len() + 1;
    
    // Determine prompt based on mode
    let (prompt, prompt_color) = if state.is_processing() {
        ("… ", Color::DarkGray)
    } else if input_area.content.starts_with('/') {
        ("/ ", Color::Yellow)
    } else {
        ("> ", Color::Blue)
    };
    
    let full_prompt = format!("{}{} ", next_num, prompt);
    
    // Wrap input text
    let wrapped = wrap_input(&input_area.content, &full_prompt, area.width as usize);
    
    // Create paragraph
    let mut text_lines: Vec<Line> = Vec::new();
    for (i, line) in wrapped.iter().enumerate() {
        let mut spans = vec![
            Span::styled(&line.prefix, Style::default().fg(prompt_color)),
        ];
        
        // Add cursor if this is the cursor line
        if i == wrapped.cursor_line {
            let (before, after) = line.content.split_at(line.cursor_in_line);
            spans.push(Span::raw(before.to_string()));
            spans.push(Span::styled("▌", Style::default().fg(Color::Green)));
            spans.push(Span::raw(after.to_string()));
        } else {
            spans.push(Span::raw(line.content.clone()));
        }
        
        text_lines.push(Line::from(spans));
    }
    
    let paragraph = Paragraph::new(text_lines)
        .block(Block::default().borders(Borders::ALL).title("Input"));
    
    frame.render_widget(paragraph, area);
    
    // Render suggestions below input
    if !input_area.suggestions.is_empty() {
        let suggestion_area = Rect {
            x: area.x,
            y: area.y + area.height,
            width: area.width,
            height: (input_area.suggestions.len() as u16).min(3),
        };
        
        let suggestion_text: Vec<Line> = input_area.suggestions
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let style = if i == input_area.suggestion_selected {
                    Style::default().bg(Color::Blue).fg(Color::White)
                } else {
                    Style::default().fg(Color::Gray)
                };
                Line::from(Span::styled(s.clone(), style))
            })
            .collect();
        
        let suggestion_para = Paragraph::new(suggestion_text)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(suggestion_para, suggestion_area);
    }
}

struct WrappedLine {
    prefix: String,
    content: String,
    cursor_in_line: usize,
}

struct WrappedInput {
    lines: Vec<WrappedLine>,
    cursor_line: usize,
}

fn wrap_input(content: &str, prefix: &str, width: usize) -> WrappedInput {
    let available_width = width.saturating_sub(prefix.len());
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut cursor_line = 0;
    let mut cursor_in_line = 0;
    let mut current_width = 0;
    
    // Simple word wrap
    for word in content.split_whitespace() {
        let word_width = word.len() + if current_line.is_empty() { 0 } else { 1 };
        
        if current_width + word_width > available_width && !current_line.is_empty() {
            lines.push(WrappedLine {
                prefix: if lines.is_empty() { prefix.to_string() } else { "   ".to_string() },
                content: current_line.clone(),
                cursor_in_line: if lines.len() == cursor_line { cursor_in_line } else { 0 },
            });
            current_line.clear();
            current_width = 0;
        }
        
        if !current_line.is_empty() {
            current_line.push(' ');
            current_width += 1;
        }
        current_line.push_str(word);
        current_width += word.len();
    }
    
    // Add remaining content
    if !current_line.is_empty() || lines.is_empty() {
        lines.push(WrappedLine {
            prefix: if lines.is_empty() { prefix.to_string() } else { "   ".to_string() },
            content: current_line,
            cursor_in_line,
        });
    }
    
    WrappedInput {
        lines,
        cursor_line,
    }
}
