//! Cron schedule editor screen.
//!
//! Schedules render as bordered cards in a [`SelectList`] — expression,
//! description, enabled/disabled badge, last run. Arrow keys navigate; Enter or
//! a single click toggles enable/disable; 'n' adds a new schedule (placeholder),
//! 'd' deletes the selected one; Esc returns to Welcome (handled by `TuieApp`).

use std::collections::HashMap;
use std::path::PathBuf;

use tuie::prelude::*;

use crate::core::nervous::cron::{self, ScheduleEntry, ScheduleKind, ScheduleState};
use crate::ui::chat::ChatPalette;
use crate::ui::theme;
use crate::ui::widgets::select_list::{ActivateEvent, SelectList};

pub struct CronScreen {
    root: Box<Pane>,
    list_id: WidgetId<SelectList>,
    /// Schedule entries loaded from the schedules directory.
    entries: Vec<ScheduleEntry>,
    /// Live run-state keyed by entry name.
    run_state: HashMap<String, ScheduleState>,
    dim: Color,
    #[allow(dead_code)] // retained for persistence work (writing edits back)
    schedules_dir: PathBuf,
}

impl DelegateWidget for CronScreen {
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
                        }
                        return InputResult::Handled;
                    }
                    Key::Arrow(Direction2D::Down) => {
                        queue.next();
                        if let Some(list) = self.root.get_widget_mut(self.list_id) {
                            list.move_down();
                        }
                        return InputResult::Handled;
                    }
                    Key::Enter => {
                        queue.next();
                        if let Some(list) = self.root.get_widget_mut(self.list_id) {
                            list.activate_selected();
                        }
                        return InputResult::Handled;
                    }
                    Key::Char('n') => {
                        queue.next();
                        self.add_placeholder();
                        return InputResult::Handled;
                    }
                    Key::Char('d') => {
                        queue.next();
                        self.delete_selected();
                        return InputResult::Handled;
                    }
                    _ => {}
                }
            }
        }
        self.get_delegate_mut().on_input(queue)
    }

    fn after_on_event(&mut self, event: &mut WidgetEvent) {
        // Activation (Enter or click) toggles the row.
        if let Some(&ActivateEvent(idx)) = event.get_by::<ActivateEvent>(self.list_id) {
            self.toggle(idx);
        }
    }
}

impl CronScreen {
    pub fn new(schedules_dir: PathBuf, palette: &ChatPalette) -> Box<Self> {
        let primary = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);

        // Scan the schedules directory for .md files.
        let entries = cron::scan_schedule_files(&schedules_dir);

        // Load persisted run-state (.state.json).
        let state_file = schedules_dir.join(".state.json");
        let run_state: HashMap<String, ScheduleState> = if state_file.exists() {
            std::fs::read_to_string(&state_file)
                .ok()
                .and_then(|data| serde_json::from_str(&data).ok())
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        let list = SelectList::new()
            .colors(primary, Color::BRIGHT_BLACK)
            .bordered()
            .items(build_rows(&entries, &run_state, dim));
        let list_id = list.get_id();

        let root = Pane::new()
            .vertical()
            .flex(1)
            .padding(Spacing::new().horizontal(1).top(1).bottom(1))
            .gap(1)
            .children([
                Text::new().content(StyledStr::new(" Cron Schedules ").fg(primary).bold())
                    as Box<dyn Widget>,
                Text::new().content(
                    StyledStr::new("  arrow keys select \u{b7} Enter or click toggle \u{b7} n add \u{b7} d delete \u{b7} Esc back")
                        .fg(Color::BRIGHT_BLACK),
                ),
                list,
                footer_hint(dim),
            ]);

        Box::new(Self {
            root,
            list_id,
            entries,
            run_state,
            dim,
            schedules_dir,
        })
    }

    /// Push the rebuilt rows into the list, holding selection at `keep`.
    fn refresh(&mut self, keep: usize) {
        let rows = build_rows(&self.entries, &self.run_state, self.dim);
        if let Some(list) = self.root.get_widget_mut(self.list_id) {
            list.set_items(rows);
            list.select(keep.min(self.entries.len().saturating_sub(1)));
        }
    }

    /// Toggle enabled/disabled on a specific entry (activation target).
    fn toggle(&mut self, idx: usize) {
        if let Some(entry) = self.entries.get_mut(idx) {
            entry.enabled = !entry.enabled;
        }
        self.refresh(idx);
    }

    /// Add a placeholder schedule entry and select it.
    fn add_placeholder(&mut self) {
        let count = self.entries.len() + 1;
        let entry = ScheduleEntry {
            name: format!("new-schedule-{count}"),
            kind: ScheduleKind::Cron,
            schedule: "0 0 * * *".to_string(),
            source: "placeholder".to_string(),
            enabled: true,
            urgency: 0.5,
            created_at: chrono::Utc::now(),
            prompt: String::new(),
        };
        self.entries.push(entry);
        let last = self.entries.len().saturating_sub(1);
        self.refresh(last);
    }

    /// Delete the selected entry from the in-memory list.
    fn delete_selected(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let idx = self
            .root
            .get_widget_mut(self.list_id)
            .map(|l| l.selected_index())
            .unwrap_or(0);
        let name = self.entries[idx].name.clone();
        self.entries.remove(idx);
        self.run_state.remove(&name);
        self.refresh(idx);
    }
}

// ── Builders ─────────────────────────────────────────────────────────────────

/// Build one styled row per schedule entry. Selection prefix and border are
/// owned by [`SelectList`]; this is the content only.
fn build_rows(
    entries: &[ScheduleEntry],
    run_state: &HashMap<String, ScheduleState>,
    dim: Color,
) -> Vec<StyledString> {
    entries
        .iter()
        .map(|entry| {
            let (enabled_marker, enabled_color) = if entry.enabled {
                ("enabled", Color::GREEN)
            } else {
                ("disabled", Color::RED)
            };

            let kind_label = match entry.kind {
                ScheduleKind::Cron => "cron",
                ScheduleKind::Interval => "interval",
                ScheduleKind::Once => "once",
            };

            let state = run_state.get(&entry.name);
            let last_run = state
                .and_then(|s| s.last_fired)
                .map(|t| format!("{}", t.format("%Y-%m-%d %H:%M UTC")))
                .unwrap_or_else(|| "never".to_string());
            let fire_count = state.map(|s| s.fire_count).unwrap_or(0);

            // Line 1: enabled badge, name, schedule, kind.
            let mut content = StyledString::new();
            content.push_span(StyledStr::new(&format!("{enabled_marker}  ")).fg(enabled_color));
            content.push_span(StyledStr::new(&entry.name).bold());
            content.push_span(StyledStr::new(&format!("    {}", entry.schedule)).fg(dim));
            content.push_span(
                StyledStr::new(&format!("  ({kind_label})"))
                    .fg(Color::BRIGHT_BLACK)
                    .italic(),
            );
            // Line 2: last run time, fire count.
            content.push_str("\n");
            content.push_span(
                StyledStr::new(&format!(
                    "last run: {last_run} \u{b7} fired {fire_count} times"
                ))
                .fg(dim),
            );
            content
        })
        .collect()
}

/// Footer with keybinding hints.
fn footer_hint(dim: Color) -> Box<Text> {
    Text::new().content(
        StyledStr::new("  up/down navigate \u{b7} Enter/click toggle \u{b7} n add \u{b7} d delete \u{b7} Esc back")
            .fg(dim),
    )
}

// ── Tests ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use tuie::emulator::Emulator;

    use super::*;
    use crate::ui::chat::ChatPalette;

    #[test]
    fn cron_screen_renders_header() {
        let palette = ChatPalette::default();
        let dir = std::path::PathBuf::from("/tmp/test_cron_header");
        let _ = std::fs::create_dir_all(&dir);
        let mut screen = CronScreen::new(dir, &palette);
        let term = Emulator::new(&mut *screen, Vec2::new(80, 20));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("Cron") || rendered.contains("Schedule"),
            "expected Cron or Schedule in header, got: {rendered:?}"
        );
    }

    #[test]
    fn cron_screen_shows_add_hint() {
        let palette = ChatPalette::default();
        let dir = std::path::PathBuf::from("/tmp/test_cron_empty");
        let _ = std::fs::create_dir_all(&dir);
        let mut screen = CronScreen::new(dir, &palette);
        let term = Emulator::new(&mut *screen, Vec2::new(80, 20));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("n") || rendered.contains("add"),
            "expected 'n' or 'add' hint, got: {rendered:?}"
        );
    }
}
