use crate::core::config::AgentStatus;
use std::time::Instant;

/// Trait for TUI state consumed by the shared renderer.
/// 
/// This trait allows components to render without knowing the full App struct.
/// It abstracts all state needed for the UI, enabling:
/// - Testable components (mock TuiState)
/// - Easier refactoring (change App without touching UI)
/// - Component reuse across screens
pub trait TuiState {
    // ========== Messages ==========
    fn messages(&self) -> &[DisplayMessage];
    fn streaming_text(&self) -> &str;
    fn is_streaming(&self) -> bool;
    fn has_errors(&self) -> bool;
    
    // ========== Input ==========
    fn input(&self) -> &str;
    fn cursor_pos(&self) -> usize;
    fn input_history(&self) -> &[String];
    fn input_history_index(&self) -> Option<usize>;
    
    // ========== Agent Status ==========
    fn agent_status(&self) -> &AgentStatus;
    fn n1_count(&self) -> usize;
    fn archivist_pressure(&self) -> f32;  // 0.0 - 1.0
    
    // ========== Connection ==========
    fn is_connected(&self) -> bool;
    fn is_processing(&self) -> bool;  // Waiting for the provider
    fn connection_error(&self) -> Option<&str>;
    
    // ========== Screen State ==========
    fn current_screen(&self) -> Screen;
    fn screen_transition(&self) -> Option<&ScreenTransition>;
    
    fn scroll_offset(&self) -> usize;
}

/// Message display type - rendered in chat
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub tool_calls: Vec<ToolCallDisplay>,
    pub timestamp: Instant,
    pub is_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    Reasoning,      // DeepSeek <thinking> blocks
    ToolResult,     // Tool execution output
    System,         // Status messages
}

#[derive(Debug, Clone)]
pub struct ToolCallDisplay {
    pub name: String,
    pub status: ToolStatus,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToolStatus {
    Pending,
    Running,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Screen {
    Splash,
    Dashboard,
    Chat,
    Settings,
}

#[derive(Debug, Clone)]
pub struct ScreenTransition {
    pub from: Screen,
    pub to: Screen,
    pub start_time: Instant,
    pub duration_ms: u64,
}

impl ScreenTransition {
    pub fn progress(&self) -> f32 {
        let elapsed = self.start_time.elapsed().as_millis() as f64;
        let t = (elapsed / self.duration_ms as f64).min(1.0);
        // Ease out cubic: smooth deceleration
        1.0 - (1.0 - t).powi(3)
    }
    
    pub fn is_complete(&self) -> bool {
        self.start_time.elapsed().as_millis() > self.duration_ms
    }
}
