#![allow(dead_code, clippy::type_complexity)] // WIP scaffolding; builder return tuple
//! Unified agent screen — master–detail view replacing the old AgentsScreen
//! and ManagerScreen.
//!
//! Wide terminals (>=100 cols): agent list on the left, selected agent's
//! portrait and vitals on the right, updating live as you arrow through.
//! Narrow terminals: list stacked above detail.
//!
//! Keys: arrow keys move, Enter talk, k kill, r restart, f pin primary,
//! i toggle inspect, Esc back.

use std::cell::Cell;
use std::rc::Rc;

use tuie::prelude::*;

use crate::ui::chat::ChatPalette;
use crate::ui::theme;
use crate::ui::tuie_app::AgentSummary;
use crate::ui::widgets::portrait::Portrait;
use crate::ui::widgets::responsive::Responsive;
use crate::ui::widgets::select_list::{ActivateEvent, SelectList};

/// Terminal width at or above which the wide (side-by-side) layout is used.
const WIDE_BREAKPOINT: u16 = 100;

// ── Detail pane widget IDs ─────────────────────────────────────────────────

/// Typed widget IDs for one copy of the detail pane. Each [`Responsive`] layout
/// (wide / narrow) gets its own set so both stay addressable regardless of
/// which layout is currently visible.
struct DetailIds {
    /// Sub-pane holding the portrait — cleared and rebuilt on agent change.
    portrait_pane: WidgetId<Pane>,
    name: WidgetId<Text>,
    glyph: WidgetId<Text>,
    description: WidgetId<Text>,
    primary_badge: WidgetId<Text>,
    instances: WidgetId<Text>,
    uptime: WidgetId<Text>,
    files: WidgetId<Text>,
    pubkey: WidgetId<Text>,
    activity_header: WidgetId<Text>,
    activity_lines: WidgetId<Text>,
}

// ── AgentsScreen ───────────────────────────────────────────────────────────

pub struct AgentsScreen {
    root: Box<Pane>,
    /// Selection lists — one per Responsive layout, selection mirrored between them.
    list_wide_id: WidgetId<SelectList>,
    list_narrow_id: WidgetId<SelectList>,
    /// Detail panes for each layout variant.
    detail_wide: DetailIds,
    detail_narrow: DetailIds,
    /// All known agents.
    agents: Vec<AgentSummary>,
    /// Currently selected index.
    selected: usize,
    palette: ChatPalette,
    primary_color: Color,
    /// Whether the inspect overlay is toggled on.
    inspecting: bool,
    // Signals shared with TuieApp.
    selection: Rc<Cell<Option<usize>>>,
    back_pressed: Rc<Cell<bool>>,
    kill_requested: Rc<Cell<Option<usize>>>,
    restart_requested: Rc<Cell<Option<usize>>>,
    pin_requested: Rc<Cell<Option<usize>>>,
}

impl DelegateWidget for AgentsScreen {
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
                        self.move_up();
                        return InputResult::Handled;
                    }
                    Key::Arrow(Direction2D::Down) => {
                        queue.next();
                        self.move_down();
                        return InputResult::Handled;
                    }
                    Key::Enter => {
                        queue.next();
                        self.activate();
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
                    Key::Char('f') => {
                        queue.next();
                        if !self.agents.is_empty() {
                            self.pin_requested.set(Some(self.selected));
                        }
                        return InputResult::Handled;
                    }
                    Key::Char('i') => {
                        queue.next();
                        self.inspecting = !self.inspecting;
                        self.refresh_detail();
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
        // Check both lists — only the visible one fires, but we don't track which.
        if let Some(&ActivateEvent(idx)) = event.get_by::<ActivateEvent>(self.list_wide_id) {
            self.selected = idx;
            self.refresh_detail();
            self.selection.set(Some(idx));
        }
        if let Some(&ActivateEvent(idx)) = event.get_by::<ActivateEvent>(self.list_narrow_id) {
            self.selected = idx;
            self.refresh_detail();
            self.selection.set(Some(idx));
        }
    }
}

impl AgentsScreen {
    /// Create the unified agent screen.
    ///
    /// Returns the screen widget and five shared signal cells:
    /// (selection, back_pressed, kill_requested, restart_requested, pin_requested)
    pub fn new(
        agents: Vec<AgentSummary>,
        palette: &ChatPalette,
    ) -> (
        Box<Self>,
        Rc<Cell<Option<usize>>>,
        Rc<Cell<bool>>,
        Rc<Cell<Option<usize>>>,
        Rc<Cell<Option<usize>>>,
        Rc<Cell<Option<usize>>>,
    ) {
        let selection = Rc::new(Cell::new(None));
        let back_pressed = Rc::new(Cell::new(false));
        let kill_requested = Rc::new(Cell::new(None));
        let restart_requested = Rc::new(Cell::new(None));
        let pin_requested = Rc::new(Cell::new(None));

        let primary = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);

        let rows = build_rows(&agents, palette);
        // Clone the first agent's data so the detail pane owns its content
        // independently from the `agents` Vec (which is moved into `self` later).
        let first_agent = agents.first().cloned();

        // ── Wide layout: list left, detail right ──────────────────────────
        let list_wide = build_list(&rows, palette);
        let list_wide_id = list_wide.get_id();
        let (detail_wide_pane, detail_wide_ids) = build_detail_pane(palette, first_agent.as_ref());

        let list_col = Pane::new()
            .vertical()
            .width(34)
            .bordered()
            .border_style(Style::new().fg(dim).dim())
            .gap(0)
            .children([
                Text::new().content(" Agents ".fg(primary).bold()) as Box<dyn Widget>,
                list_wide,
            ]);

        let wide = Pane::new()
            .horizontal()
            .flex(1)
            .gap(2)
            .children([list_col as Box<dyn Widget>, detail_wide_pane]);

        // ── Narrow layout: list top, detail bottom ───────────────────────
        let list_narrow = build_list(&rows, palette);
        let list_narrow_id = list_narrow.get_id();
        let (detail_narrow_pane, detail_narrow_ids) =
            build_detail_pane(palette, first_agent.as_ref());

        let narrow_list_col = Pane::new()
            .vertical()
            .flex(1)
            .bordered()
            .border_style(Style::new().fg(dim).dim())
            .gap(0)
            .children([
                Text::new().content(" Agents ".fg(primary).bold()) as Box<dyn Widget>,
                list_narrow,
            ]);

        let narrow = Pane::new()
            .vertical()
            .flex(1)
            .gap(1)
            .children([narrow_list_col as Box<dyn Widget>, detail_narrow_pane]);

        let responsive = Responsive::new(WIDE_BREAKPOINT, wide, narrow);

        // ── Root ─────────────────────────────────────────────────────────
        let root = Pane::new()
            .vertical()
            .flex(1)
            .padding(Spacing::new().horizontal(1).top(1).bottom(1))
            .gap(0)
            .children([
                Text::new().content(" Agents ".fg(primary).bold()) as Box<dyn Widget>,
                Text::new().content(
                    StyledStr::new(
                        "  \u{2190}\u{2192} select · Enter talk · k kill · r restart · f pin · i inspect · Esc back",
                    )
                    .fg(dim),
                ) as Box<dyn Widget>,
                responsive as Box<dyn Widget>,
            ]);

        let mut this = Self {
            root,
            list_wide_id,
            list_narrow_id,
            detail_wide: detail_wide_ids,
            detail_narrow: detail_narrow_ids,
            agents,
            selected: 0,
            palette: *palette,
            primary_color: primary,
            inspecting: false,
            selection,
            back_pressed,
            kill_requested,
            restart_requested,
            pin_requested,
        };

        // Show the first agent's detail immediately.
        this.refresh_detail();

        let sel = this.selection.clone();
        let back = this.back_pressed.clone();
        let kill = this.kill_requested.clone();
        let restart = this.restart_requested.clone();
        let pin = this.pin_requested.clone();

        (Box::new(this), sel, back, kill, restart, pin)
    }

    // ── Selection navigation ─────────────────────────────────────────────

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.apply_selection();
            self.refresh_detail();
        }
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.agents.len() {
            self.selected += 1;
            self.apply_selection();
            self.refresh_detail();
        }
    }

    /// Mirror the current selection into both layouts' lists so the choice
    /// survives a resize that switches layouts.
    fn apply_selection(&mut self) {
        let sel = self.selected;
        if let Some(list) = self.root.get_widget_mut(self.list_wide_id) {
            list.select(sel);
        }
        if let Some(list) = self.root.get_widget_mut(self.list_narrow_id) {
            list.select(sel);
        }
    }

    fn activate(&mut self) {
        if !self.agents.is_empty() {
            // Activate the wide list — the event fires regardless of which
            // layout is currently visible.
            if let Some(list) = self.root.get_widget_mut(self.list_wide_id) {
                list.activate_selected();
            }
        }
    }

    // ── Detail refresh ───────────────────────────────────────────────────

    /// Refresh both detail panes to show the currently selected agent.
    fn refresh_detail(&mut self) {
        if let Some(agent) = self.agents.get(self.selected).cloned() {
            self.refresh_detail_ids(
                &DetailIdsRef {
                    portrait_pane: self.detail_wide.portrait_pane,
                    name: self.detail_wide.name,
                    glyph: self.detail_wide.glyph,
                    description: self.detail_wide.description,
                    primary_badge: self.detail_wide.primary_badge,
                    instances: self.detail_wide.instances,
                    uptime: self.detail_wide.uptime,
                    files: self.detail_wide.files,
                    pubkey: self.detail_wide.pubkey,
                    activity_header: self.detail_wide.activity_header,
                    activity_lines: self.detail_wide.activity_lines,
                },
                &agent,
            );
            self.refresh_detail_ids(
                &DetailIdsRef {
                    portrait_pane: self.detail_narrow.portrait_pane,
                    name: self.detail_narrow.name,
                    glyph: self.detail_narrow.glyph,
                    description: self.detail_narrow.description,
                    primary_badge: self.detail_narrow.primary_badge,
                    instances: self.detail_narrow.instances,
                    uptime: self.detail_narrow.uptime,
                    files: self.detail_narrow.files,
                    pubkey: self.detail_narrow.pubkey,
                    activity_header: self.detail_narrow.activity_header,
                    activity_lines: self.detail_narrow.activity_lines,
                },
                &agent,
            );
        }
    }

    fn refresh_detail_ids(&mut self, ids: &DetailIdsRef, agent: &AgentSummary) {
        let primary = self.primary_color;
        let dim = theme::to_tuie_color(self.palette.agent_dim);
        let green = Color::Rgb(120, 220, 160);

        // Portrait — clear and rebuild.
        self.rebuild_portrait(ids.portrait_pane, agent);

        // Name + glyph header.
        if let Some(t) = self.root.get_widget_mut(ids.name) {
            let mut content = StyledString::new();
            content.push_span(StyledStr::new(&agent.glyph).fg(primary));
            content.push_span(
                StyledStr::new(&format!("  {}", agent.name))
                    .fg(primary)
                    .bold(),
            );
            t.set_content(content);
        }

        // Description.
        if let Some(t) = self.root.get_widget_mut(ids.description) {
            let desc = if agent.description.is_empty() {
                "(no description)"
            } else {
                &agent.description
            };
            t.set_content(StyledStr::new(desc).fg(dim).italic());
        }

        // Primary badge.
        if let Some(t) = self.root.get_widget_mut(ids.primary_badge) {
            if agent.is_primary {
                t.set_content(StyledStr::new("\u{2605}  PRIMARY").fg(primary).bold());
            } else {
                t.set_content(StyledStr::new(""));
            }
        }

        // Instances.
        if let Some(t) = self.root.get_widget_mut(ids.instances) {
            let mut content = StyledString::new();
            content.push_span(StyledStr::new("  instances  ").fg(dim));
            let count_color = if agent.instance_count > 0 {
                primary
            } else {
                dim
            };
            content.push_span(StyledStr::new(&format!("{}", agent.instance_count)).fg(count_color));
            t.set_content(content);
        }

        // Uptime bar + percentage.
        if let Some(t) = self.root.get_widget_mut(ids.uptime) {
            let mut content = StyledString::new();
            content.push_span(StyledStr::new("  uptime      ").fg(dim));
            crate::ui::widgets::stats::push_bar(
                &mut content,
                agent.uptime_pct as f32 / 100.0,
                10,
                green,
            );
            let pct_color = if agent.uptime_pct > 0 { green } else { dim };
            content.push_span(StyledStr::new(&format!(" {}%", agent.uptime_pct)).fg(pct_color));
            t.set_content(content);
        }

        // Memory files.
        if let Some(t) = self.root.get_widget_mut(ids.files) {
            let mut content = StyledString::new();
            content.push_span(StyledStr::new("  files       ").fg(dim));
            content.push_span(StyledStr::new(&format!("{}", agent.memory_count)).fg(primary));
            t.set_content(content);
        }

        // Pubkey prefix.
        if let Some(t) = self.root.get_widget_mut(ids.pubkey) {
            let mut content = StyledString::new();
            content.push_span(StyledStr::new("  pubkey      ").fg(dim));
            let prefix = if agent.pubkey_prefix.is_empty() {
                "\u{2014}"
            } else {
                &agent.pubkey_prefix
            };
            content.push_span(StyledStr::new(prefix).fg(dim));
            t.set_content(content);
        }

        // Activity header.
        if let Some(t) = self.root.get_widget_mut(ids.activity_header) {
            if agent.recent_activity.is_empty() {
                t.set_content(StyledStr::new(""));
            } else {
                t.set_content(StyledStr::new("  Activity").fg(dim).bold());
            }
        }

        // Activity lines.
        if let Some(t) = self.root.get_widget_mut(ids.activity_lines) {
            let mut content = StyledString::new();
            let max = if self.inspecting {
                agent.recent_activity.len()
            } else {
                3.min(agent.recent_activity.len())
            };
            for line in agent.recent_activity.iter().take(max) {
                content.push_span(StyledStr::new(&format!("  \u{00b7} {}\n", line)).fg(dim));
            }
            if self.inspecting && agent.recent_activity.len() > 3 {
                content.push_span(
                    StyledStr::new(&format!(
                        "  ... {} more (i to collapse)\n",
                        agent.recent_activity.len() - 3
                    ))
                    .fg(dim),
                );
            } else if !self.inspecting && agent.recent_activity.len() > 3 {
                content.push_span(
                    StyledStr::new(&format!(
                        "  ... {} more (i to expand)\n",
                        agent.recent_activity.len() - 3
                    ))
                    .fg(dim),
                );
            }
            t.set_content(content);
        }
    }

    /// Clear the portrait sub-pane and insert a new [`Portrait`] for `agent`.
    fn rebuild_portrait(&mut self, pane_id: WidgetId<Pane>, agent: &AgentSummary) {
        let color = self.primary_color;
        let portrait = Portrait::new(Some(&agent.id), &agent.name, color);

        if let Some(pane) = self.root.get_widget_mut(pane_id) {
            pane.clear();
            pane.add_child(portrait);
        }
    }
}

// ── DetailIdsRef helper ────────────────────────────────────────────────────

/// Borrow-free snapshot of [`DetailIds`] so `refresh_detail_ids` can be
/// called for both wide and narrow without splitting `&mut self`.
struct DetailIdsRef {
    portrait_pane: WidgetId<Pane>,
    name: WidgetId<Text>,
    glyph: WidgetId<Text>,
    description: WidgetId<Text>,
    primary_badge: WidgetId<Text>,
    instances: WidgetId<Text>,
    uptime: WidgetId<Text>,
    files: WidgetId<Text>,
    pubkey: WidgetId<Text>,
    activity_header: WidgetId<Text>,
    activity_lines: WidgetId<Text>,
}

// ── Build helpers ──────────────────────────────────────────────────────────

/// Build one styled row per agent for the [`SelectList`].
fn build_rows(agents: &[AgentSummary], palette: &ChatPalette) -> Vec<StyledString> {
    let primary = theme::to_tuie_color(palette.agent_primary);
    let dim = theme::to_tuie_color(palette.agent_dim);
    let green = Color::Rgb(120, 220, 160);

    agents
        .iter()
        .map(|a| {
            let mut row = StyledString::new();
            // Name.
            let name = truncate_str(&a.name, 20);
            row.push_span(StyledStr::new(&format!("{:<20}", name)).bold());
            // Primary marker.
            if a.is_primary {
                row.push_span(StyledStr::new("\u{2605} ").fg(primary));
            } else {
                row.push_span(StyledStr::new("  "));
            }
            // Status.
            if a.instance_count > 0 {
                row.push_span(StyledStr::new("active").fg(green));
            } else {
                row.push_span(StyledStr::new("idle  ").fg(dim));
            }
            // Instance count.
            if a.instance_count > 0 {
                row.push_span(StyledStr::new(&format!("  {} up", a.instance_count)).fg(dim));
            }
            row
        })
        .collect()
}

/// Build a [`SelectList`] pre-filled with the given rows.
fn build_list(rows: &[StyledString], palette: &ChatPalette) -> Box<SelectList> {
    let primary = theme::to_tuie_color(palette.agent_primary);
    let dim = theme::to_tuie_color(palette.agent_dim);
    SelectList::new().colors(primary, dim).items(rows.to_vec())
}

/// Build one detail pane — a bordered column with the selected agent's info.
///
/// If `agent` is `Some`, the fields are pre-populated with that agent's data
/// so the initial render is correct before any `set_content` calls. If `None`
/// (empty agent list), the fields start empty.
fn build_detail_pane(
    palette: &ChatPalette,
    agent: Option<&AgentSummary>,
) -> (Box<Pane>, DetailIds) {
    let primary = theme::to_tuie_color(palette.agent_primary);
    let dim = theme::to_tuie_color(palette.agent_dim);
    let green = Color::Rgb(120, 220, 160);

    // Helper: build a "key  value" styled string.
    let kv = |key: &str, val: &str, val_color: Color| -> StyledString {
        let mut s = StyledString::new();
        s.push_span(StyledStr::new(&format!("  {:<12}", key)).fg(dim));
        s.push_span(StyledStr::new(val).fg(val_color));
        s
    };

    // Portrait — height(1) temporarily to avoid flex layout issues.
    let portrait_pane = Pane::new()
        .height(1)
        .x_place(Place::Center)
        .y_place(Place::Center);
    let portrait_pane_id = portrait_pane.get_id();

    let default_agent = AgentSummary::default();

    // Build initial content from agent or defaults.
    let ag = agent.unwrap_or(&default_agent);

    let mut name_content = StyledString::new();
    name_content.push_span(StyledStr::new(&ag.glyph).fg(primary));
    name_content.push_span(StyledStr::new(&format!("  {}", ag.name)).fg(primary).bold());
    let name = Text::new().content(name_content);
    let name_id = name.get_id();

    let glyph = Text::new().content(StyledStr::new(&ag.glyph).fg(primary));
    let glyph_id = glyph.get_id();

    let desc = if ag.description.is_empty() {
        "(no description)"
    } else {
        &ag.description
    };
    let description = Text::new().content(StyledStr::new(desc).fg(dim).italic());
    let description_id = description.get_id();

    let pb = if ag.is_primary {
        StyledStr::new("\u{2605}  PRIMARY")
            .fg(primary)
            .bold()
            .into()
    } else {
        StyledString::new()
    };
    let primary_badge = Text::new().content(pb);
    let primary_badge_id = primary_badge.get_id();

    let instances = Text::new().content(kv("instances", &ag.instance_count.to_string(), primary));
    let instances_id = instances.get_id();

    let mut uptime_content = StyledString::new();
    uptime_content.push_span(StyledStr::new("  uptime      ").fg(dim));
    crate::ui::widgets::stats::push_bar(
        &mut uptime_content,
        ag.uptime_pct as f32 / 100.0,
        10,
        green,
    );
    let pct_color = if ag.uptime_pct > 0 { green } else { dim };
    uptime_content.push_span(StyledStr::new(&format!(" {}%", ag.uptime_pct)).fg(pct_color));
    let uptime = Text::new().content(uptime_content);
    let uptime_id = uptime.get_id();

    let files = Text::new().content(kv("files", &ag.memory_count.to_string(), primary));
    let files_id = files.get_id();

    let pk = if ag.pubkey_prefix.is_empty() {
        "\u{2014}"
    } else {
        &ag.pubkey_prefix
    };
    let pubkey = Text::new().content(kv("pubkey", pk, dim));
    let pubkey_id = pubkey.get_id();

    let ah = if ag.recent_activity.is_empty() {
        StyledString::new()
    } else {
        let mut s = StyledString::new();
        s.push_span(StyledStr::new("  Activity").fg(dim).bold());
        s
    };
    let activity_header = Text::new().content(ah);
    let activity_header_id = activity_header.get_id();

    let mut al_content = StyledString::new();
    for line in ag.recent_activity.iter().take(3) {
        al_content.push_span(StyledStr::new(&format!("  \u{00b7} {}\n", line)).fg(dim));
    }
    if ag.recent_activity.len() > 3 {
        al_content.push_span(
            StyledStr::new(&format!(
                "  ... {} more (i to expand)\n",
                ag.recent_activity.len() - 3
            ))
            .fg(dim),
        );
    }
    let activity_lines = Text::new().content(al_content);
    let activity_lines_id = activity_lines.get_id();

    let section = Pane::new()
        .vertical()
        .flex(1)
        .bordered()
        .border_style(Style::new().fg(dim).dim())
        .gap(0)
        .children([
            portrait_pane as Box<dyn Widget>,
            name as Box<dyn Widget>,
            description as Box<dyn Widget>,
            primary_badge as Box<dyn Widget>,
            instances as Box<dyn Widget>,
            uptime as Box<dyn Widget>,
            files as Box<dyn Widget>,
            pubkey as Box<dyn Widget>,
            activity_header as Box<dyn Widget>,
            activity_lines as Box<dyn Widget>,
        ]);

    let section_id = section.get_id();
    let _ = section_id;

    (
        section,
        DetailIds {
            portrait_pane: portrait_pane_id,
            name: name_id,
            glyph: glyph_id,
            description: description_id,
            primary_badge: primary_badge_id,
            instances: instances_id,
            uptime: uptime_id,
            files: files_id,
            pubkey: pubkey_id,
            activity_header: activity_header_id,
            activity_lines: activity_lines_id,
        },
    )
}

/// Truncate a string to at most `max_len` chars, appending "…" if cut.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}\u{2026}", truncated)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use tuie::emulator::Emulator;

    use super::*;
    use crate::ui::chat::ChatPalette;

    fn sample_agents() -> Vec<AgentSummary> {
        vec![
            AgentSummary {
                id: "a1".into(),
                name: "Alice".into(),
                description: "Primary agent".into(),
                glyph: "◇◆◇◆".into(),
                instance_count: 2,
                uptime_pct: 87,
                memory_count: 142,
                pubkey_prefix: "abcd1234ef01".into(),
                is_primary: true,
                atmosphere: None,
                recent_activity: vec!["updated index".into(), "merged pass".into()],
            },
            AgentSummary {
                id: "b2".into(),
                name: "Bob".into(),
                description: "".into(),
                glyph: "◆◇◆◇".into(),
                instance_count: 0,
                uptime_pct: 0,
                memory_count: 3,
                pubkey_prefix: "".into(),
                is_primary: false,
                atmosphere: None,
                recent_activity: vec![],
            },
        ]
    }

    #[test]
    fn renders_header_and_hints() {
        let palette = ChatPalette::default();
        let agents = sample_agents();
        let (mut screen, ..) = AgentsScreen::new(agents, &palette);
        let term = Emulator::new(&mut *screen, Vec2::new(120, 30));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("Agents"),
            "expected 'Agents' in header, got: {rendered:?}"
        );
        assert!(
            rendered.contains("talk") || rendered.contains("Enter"),
            "expected key hints, got: {rendered:?}"
        );
    }

    #[test]
    fn shows_agent_names() {
        let palette = ChatPalette::default();
        let agents = sample_agents();
        let (mut screen, ..) = AgentsScreen::new(agents, &palette);
        let term = Emulator::new(&mut *screen, Vec2::new(120, 30));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("Alice"),
            "expected 'Alice' in agent list, got: {rendered:?}"
        );
        assert!(
            rendered.contains("Bob"),
            "expected 'Bob' in agent list, got: {rendered:?}"
        );
    }

    #[test]
    fn shows_primary_badge() {
        let palette = ChatPalette::default();
        let agents = sample_agents();
        let (mut screen, ..) = AgentsScreen::new(agents, &palette);
        let term = Emulator::new(&mut *screen, Vec2::new(120, 30));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("PRIMARY"),
            "expected 'PRIMARY' badge in detail pane, got: {rendered:?}"
        );
    }

    #[test]
    fn narrow_layout_renders() {
        let palette = ChatPalette::default();
        let agents = sample_agents();
        let (mut screen, ..) = AgentsScreen::new(agents, &palette);
        // 80 cols triggers narrow (stacked) layout.
        let term = Emulator::new(&mut *screen, Vec2::new(80, 30));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("Alice"),
            "narrow layout should still show agent names, got: {rendered:?}"
        );
    }

    #[test]
    fn escape_sets_back_signal() {
        let palette = ChatPalette::default();
        let agents = sample_agents();
        let (mut screen, _sel, back, _kill, _restart, _pin) = AgentsScreen::new(agents, &palette);
        assert!(!back.get());

        let chord = chord_macro::chord!(Esc);
        let event = tuie::widget::input::InputEvent::from_chord(chord);
        let events = [event];
        let mut queue = tuie::widget::input::InputQueue::new(&events, false);
        let _ = screen.on_input(&mut queue);
        assert!(back.get(), "Esc should set back_pressed signal");
    }

    #[test]
    fn arrow_down_changes_selection() {
        let palette = ChatPalette::default();
        let agents = sample_agents();
        let (mut screen, ..) = AgentsScreen::new(agents, &palette);
        assert_eq!(screen.selected, 0);

        let chord = chord_macro::chord!(Down);
        let event = tuie::widget::input::InputEvent::from_chord(chord);
        let events = [event];
        let mut queue = tuie::widget::input::InputQueue::new(&events, false);
        let _ = screen.on_input(&mut queue);
        assert_eq!(screen.selected, 1);
    }

    #[test]
    fn arrow_up_at_top_stays() {
        let palette = ChatPalette::default();
        let agents = sample_agents();
        let (mut screen, ..) = AgentsScreen::new(agents, &palette);
        assert_eq!(screen.selected, 0);

        let chord = chord_macro::chord!(Up);
        let event = tuie::widget::input::InputEvent::from_chord(chord);
        let events = [event];
        let mut queue = tuie::widget::input::InputQueue::new(&events, false);
        let _ = screen.on_input(&mut queue);
        assert_eq!(screen.selected, 0, "up at top should stay at 0");
    }

    #[test]
    fn empty_agent_list_renders() {
        let palette = ChatPalette::default();
        let (mut screen, ..) = AgentsScreen::new(vec![], &palette);
        let term = Emulator::new(&mut *screen, Vec2::new(120, 20));
        let rendered = term.get_snapshot_text();
        assert!(
            rendered.contains("Agents"),
            "empty screen should still render header, got: {rendered:?}"
        );
    }
}
