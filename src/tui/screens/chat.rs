use crate::tui::state::{TuiState, Screen};
use crate::tui::components::{messages, input, sidebar};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Borders, Clear},
    Frame,
};

/// Render the chat screen - conversation interface
pub fn render_chat_screen(frame: &mut Frame, state: &dyn TuiState, area: Rect) {
    // Split into main chat area and sidebar
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(40),      // Chat area
            Constraint::Length(28),   // Sidebar
        ])
        .split(area);
    
    let chat_area = chunks[0];
    let sidebar_area = chunks[1];
    
    // Split chat area into messages and input
    let chat_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),      // Messages
            Constraint::Length(5),    // Input
        ])
        .split(chat_area);
    
    // Render messages
    messages::render_messages(frame, state, chat_chunks[0]);
    
    // Render input (create a temporary InputArea from state)
    // In real implementation, this would be stored in App
    // input::render_input(frame, state, input_area, chat_chunks[1]);
    
    // Render sidebar
    sidebar::render_sidebar(frame, state, sidebar_area);
}

/// Chat screen input handling
pub fn handle_chat_input(state: &mut dyn TuiState, key: crossterm::event::KeyCode) -> Option<String> {
    // Returns Some(message) when user submits
    // Returns None for other actions
    use crossterm::event::KeyCode;
    
    match key {
        KeyCode::Enter => {
            // Submit message
            // state.submit_input()
            None
        }
        KeyCode::Char(c) => {
            // state.input_push_char(c);
            None
        }
        KeyCode::Backspace => {
            // state.input_backspace();
            None
        }
        KeyCode::Left => {
            // state.input_move_cursor_left();
            None
        }
        KeyCode::Right => {
            // state.input_move_cursor_right();
            None
        }
        KeyCode::Up => {
            // History up
            None
        }
        KeyCode::Down => {
            // History down
            None
        }
        KeyCode::Esc => {
            // Cancel / back to dashboard
            // state.set_screen(Screen::Dashboard);
            None
        }
        _ => None,
    }
}
