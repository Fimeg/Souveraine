//! Agent manager screen — scrollable list of agent processes with status info.
//!
//! Each known agent is a row with name, instance count, status, uptime %, and
//! memory file count, in a [`SelectList`]. Arrow keys navigate; Enter or a
//! single click selects; k=kill, r=restart, Esc=back.
//!
//! Ported from `src/ui/app/manager_screen.rs` (ratatui → tuie).

use std::cell::Cell;
use std::rc::Rc;

use tuie::prelude::*;

use crate::ui::chat::ChatPalette;
use crate::ui::theme;
use crate::ui::widgets::select_list::{ActivateEvent, SelectList};

// ── Agent process info ───────────────────────────────────────────────────────────

/// A single row in the manager list — flattened from the old `AgentCard`.
#[derive(Debug, Clone)]
pub struct AgentProcessInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 4-glyph SeedID badge (e.g. "◇◆◇◆").
    pub glyph: String,
    /// Running instance count (≈ process count).
    pub instance_count: i64,
    /// Lifetime uptime percentage, capped at 99.
    pub uptime_pct: u8,
    /// Number of files in the agent's memory repo.
    pub memory_count: usize,
    /// First 8 hex chars of the pubkey.
    pub pubkey_prefix: String,
}

// ── ManagerScreen widget ────────────────────────────────────────────────────────

pub struct ManagerScreen {
    root: Box<Pane>,
    list_id: WidgetId<SelectList>,
    /// All known agents.
    agents: Vec<AgentProcessInfo>,
    /// Shadow of the list's selection, kept current for k/r and selected_agent.
    selected: usize,
    /// Shared with TuieApp — set to Some(index) when a row is activated.
    pub selection: Rc<Cell<Option<usize>>>,
    /// Shared flag — set to true when Esc is pressed (caller pops this screen).
    pub back_pressed: Rc<Cell<bool>>,
    /// Shared with TuieApp — set to Some(index) when 'k' is pressed.
    pub kill_requested: Rc<Cell<Option<usize>>>,
    /// Shared with TuieApp — set to Some(index) when 'r' is pressed.
    pub restart_requested: Rc<Cell<Option<usize>>>,
}

impl DelegateWidget for ManagerScreen {
    tuie::delegate_widget!(root);

    fn override_is_focusable(&self) -> bool {
        true
    }

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        use tuie::input::key::Key;
        use tuie::input::trigger::Trigger;

        if let Some(event) = queue.peek() {
            if let Trigger::Key(key) = &event.chord.trigger {
                match key {
                    Key::Arrow(Direction2D::Up) => {
                        queue.next();
                        if let Some(list) = self.root.get_widget_mut(self.list_id) {
                            list.move_up();
                            self.selected = list.selected_index();
                        }
                        return InputResult::Handled;
                    }
                    Key::Arrow(Direction2D::Down) => {
                        queue.next();
                        if let Some(list) = self.root.get_widget_mut(self.list_id) {
                            list.move_down();
                            self.selected = list.selected_index();
                        }
                        return InputResult::Handled;
                    }
                    Key::Enter => {
                        queue.next();
                        if !self.agents.is_empty() {
                            if let Some(list) = self.root.get_widget_mut(self.list_id) {
                                list.activate_selected();
                            }
                        }
                        return InputResult::Handled;
                    }
                    Key::Char('k') => {
                        queue.next();
                        if !self.agents.is_empty() {
                            self.kill_requested.set(Some(self.selected));
                        }
                        return InputResult::Handled;
                    }
                    Key::Char('r') => {
                        queue.next();
                        if !self.agents.is_empty() {
                            self.restart_requested.set(Some(self.selected));
                        }
                        return InputResult::Handled;
                    }
                    Key::Esc => {
                        queue.next();
                        self.back_pressed.set(true);
                        return InputResult::Handled;
                    }
                    _ => {}
                }
            }
        }
        self.get_delegate_mut().on_input(queue)
    }

    fn after_on_event(&mut self, event: &mut WidgetEvent) {
        if let Some(&ActivateEvent(idx)) = event.get_by::<ActivateEvent>(self.list_id) {
            if !self.agents.is_empty() {
                self.selected = idx;
                self.selection.set(Some(idx));
            }
        }
    }
}

impl ManagerScreen {
    /// Create an empty manager screen. Call [`refresh`](Self::refresh) to
    /// populate the list.
    pub fn new(palette: &ChatPalette) -> (Box<Self>, Rc<Cell<Option<usize>>>, Rc<Cell<bool>>, Rc<Cell<Option<usize>>>, Rc<Cell<Option<usize>>>) {
        let selection = Rc::new(Cell::new(None));
        let back_pressed = Rc::new(Cell::new(false));
        let kill_requested = Rc::new(Cell::new(None));
        let restart_requested = Rc::new(Cell::new(None));

        let dim = theme::to_tuie_color(palette.agent_dim);
        let primary = theme::to_tuie_color(palette.agent_primary);

        let header = Text::new().content(StyledStr::new(" Agent Manager ").fg(primary).bold());
        let col_guide = Text::new().content(
            StyledStr::new("   name              pid   status    uptime   memory").fg(dim),
        );

        // Empty to start — rebuilt by refresh().
        let list = SelectList::new()
            .colors(primary, dim)
            .items(Vec::new());
        let list_id = list.get_id();

        let footer = Text::new()
            .content(StyledStr::new(" Enter/click=select  k=kill  r=restart  Esc=back").fg(dim));

        let root = Pane::new()
            .vertical()
            .flex(1)
            .padding(Spacing::new().horizontal(1).top(1).bottom(1))
            .gap(0)
            .children([
                header as Box<dyn Widget>,
                col_guide,
                list,
                footer,
            ]);

        let sel = selection.clone();
        let back = back_pressed.clone();
        let kill = kill_requested.clone();
        let restart = restart_requested.clone();
        let this = Box::new(Self {
            root,
            list_id,
            agents: Vec::new(),
            selected: 0,
            selection,
            back_pressed,
            kill_requested,
            restart_requested,
        });

        (this, sel, back, kill, restart)
    }

    /// Replace the agent list and rebuild rows.
    pub fn refresh(&mut self, agents: Vec<AgentProcessInfo>, palette: &ChatPalette) {
        self.agents = agents;
        self.selected = self.selected.min(self.agents.len().saturating_sub(1));
        let rows = build_rows(&self.agents, palette);
        if let Some(list) = self.root.get_widget_mut(self.list_id) {
            list.set_items(rows);
            list.select(self.selected);
        }
    }

    /// Return the currently selected agent, if any.
    #[allow(dead_code)] // public accessor; not yet consumed by TuieApp
    pub fn selected_agent(&self) -> Option<&AgentProcessInfo> {
        self.agents.get(self.selected)
    }
}

/// Build one styled row per agent. Selection prefix/tint owned by [`SelectList`].
fn build_rows(agents: &[AgentProcessInfo], palette: &ChatPalette) -> Vec<StyledString> {
    let dim = theme::to_tuie_color(palette.agent_dim);
    let green = Color::Rgb(120, 220, 160);

    agents
        .iter()
        .map(|agent| {
            let status = if agent.instance_count > 0 { "active" } else { "idle  " };
            let status_color = if agent.instance_count > 0 { green } else { dim };

            let mut content = StyledString::new();
            content.push_span(StyledStr::new(&format!("{:<16}", truncate_str(&agent.name, 16))).bold());
            content.push_span(StyledStr::new("  ").fg(dim));
            let pid_str = if agent.instance_count > 0 {
                format!("{:>5}", agent.instance_count)
            } else {
                format!("{:>5}", "\u{2014}")
            };
            content.push_span(StyledStr::new(&pid_str).fg(dim));
            content.push_span(StyledStr::new("  ").fg(dim));
            content.push_span(StyledStr::new(&format!("{:<7}", status)).fg(status_color));
            content.push_span(StyledStr::new("  ").fg(dim));
            let uptime_str = format!("{:>3}%", agent.uptime_pct);
            content.push_span(StyledStr::new(&uptime_str).fg(if agent.uptime_pct > 0 { green } else { dim }));
            content.push_span(StyledStr::new("   ").fg(dim));
            content.push_span(StyledStr::new(&format!("{:>5} files", agent.memory_count)).fg(dim));
            content
        })
        .collect()
}

/// Truncate a string to at most `max_len` characters, appending "…" if cut.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}\u{2026}", truncated)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use tuie::emulator::Emulator;

    use super::*;
    use crate::ui::chat::ChatPalette;

    #[test]
    fn manager_screen_renders_header() {
        let palette = ChatPalette::default();
        let (mut screen, _selection, _back, _kill, _restart) = ManagerScreen::new(&palette);
        let term = Emulator::new(&mut *screen, Vec2::new(80, 20));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("Agent Manager"),
            "expected 'Agent Manager' in header, got: {rendered:?}"
        );
    }

    #[test]
    fn manager_screen_empty_state() {
        let palette = ChatPalette::default();
        let (mut screen, _selection, _back, _kill, _restart) = ManagerScreen::new(&palette);
        let term = Emulator::new(&mut *screen, Vec2::new(80, 20));
        let rendered = term.get_snapshot_text();
        assert!(
            !rendered.trim().is_empty(),
            "empty manager screen should still render column guide and footer"
        );
        assert!(
            rendered.contains("name") || rendered.contains("Enter"),
            "expected column guide or footer hint, got: {rendered:?}"
        );
    }

    #[test]
    fn manager_screen_shows_agents() {
        let palette = ChatPalette::default();
        let (mut screen, _selection, _back, _kill, _restart) = ManagerScreen::new(&palette);
        let agents = vec![AgentProcessInfo {
            id: "a1".into(),
            name: "TestBot".into(),
            description: "test".into(),
            glyph: "◇◆◇◆".into(),
            instance_count: 1,
            uptime_pct: 99,
            memory_count: 10,
            pubkey_prefix: "abcdef01".into(),
        }];
        screen.refresh(agents, &palette);
        let term = Emulator::new(&mut *screen, Vec2::new(80, 20));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("TestBot"),
            "expected 'TestBot' in rendered agent list, got: {rendered:?}"
        );
    }
}
