#![allow(dead_code)] // WIP scaffolding not yet wired
//! Welcome screen — the dashboard shown after splash.
//!
//! Composes the brand title, stat cards (live data), portrait, recent-activity
//! pane and menu into Souveraine's dashboard. It offers **two viewable modes**,
//! switched on terminal width by a [`Responsive`] container:
//!
//! * **Wide** (≥ [`WIDE_BREAKPOINT`] cols) — portrait on the left, a column of
//!   stats / activity / menu on the right, the way a cockpit spreads out.
//! * **Stacked** (narrow) — everything in a single vertical column: stats,
//!   portrait, activity, menu.
//!
//! Both arrangements carry their own menu; the selected index is mirrored into
//! both so navigating, then resizing across the breakpoint, keeps your place.
//!
//! Flourishes: the title breathes (a gentle scheduled colour pulse) and the
//! portrait's border takes on the "surfacing" colour while the subconscious is
//! active.

use std::cell::Cell;
use std::rc::Rc;

use tuie::prelude::*;

use crate::ui::chat::ChatPalette;
use crate::ui::theme;
use crate::ui::tuie_app::AgentStatus;
use crate::ui::widgets::brand_title::BrandTitle;
use crate::ui::widgets::portrait::Portrait;
use crate::ui::widgets::responsive::Responsive;
use crate::ui::widgets::select_list::{ActivateEvent, SelectList};
use crate::ui::widgets::stats::breathe_color;

/// Terminal width (in columns) at or above which the wide layout is used.
pub const WIDE_BREAKPOINT: u16 = 100;

pub struct WelcomeScreen {
    root: Box<Pane>,
    /// Menu in the wide arrangement.
    menu_wide_id: WidgetId<SelectList>,
    /// Menu in the stacked arrangement.
    menu_narrow_id: WidgetId<SelectList>,
    title_id: WidgetId<BrandTitle>,
    /// Selected menu index, mirrored into both menus.
    selected: usize,
    menu_len: usize,
    /// Base title colour the breathing pulse modulates around.
    primary: Color,
    /// Breathing-animation phase counter.
    tick: u64,
    /// Shared with TuieApp — set to Some(menu_index) when Enter is pressed.
    pub menu_action: Rc<Cell<Option<usize>>>,
}

impl DelegateWidget for WelcomeScreen {
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
                        self.move_menu_up();
                        return InputResult::Handled;
                    }
                    Key::Arrow(Direction2D::Down) => {
                        queue.next();
                        self.move_menu_down();
                        return InputResult::Handled;
                    }
                    Key::Enter => {
                        queue.next();
                        self.choose(self.selected);
                        return InputResult::Handled;
                    }
                    _ => {}
                }
            }
        }
        self.get_delegate_mut().on_input(queue)
    }

    fn after_on_event(&mut self, event: &mut WidgetEvent) {
        // A single click on either arrangement's menu activates that row.
        for id in [self.menu_wide_id, self.menu_narrow_id] {
            if let Some(&ActivateEvent(idx)) = event.get_by::<ActivateEvent>(id) {
                self.selected = idx;
                self.apply_selection();
                self.choose(idx);
            }
        }
    }
}

impl WelcomeScreen {
    pub fn new(
        palette: &ChatPalette,
        status: &AgentStatus,
    ) -> (Box<Self>, Rc<Cell<Option<usize>>>) {
        let menu_action = Rc::new(Cell::new(None));
        let primary = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);

        let title = BrandTitle::new(primary, dim);
        let title_id = title.get_id();

        let items = menu_items();
        let menu_len = items.len();

        // ── Wide arrangement: portrait | (stats / activity / menu) ───────────
        let menu_wide = make_menu(palette);
        let menu_wide_id = menu_wide.get_id();

        let right = Pane::new().vertical().gap(2).children([
            build_stat_row(palette, status) as Box<dyn Widget>,
            section("Recent Activity", build_activity(palette, status), dim)
                .flex(1)
                .min_height(3),
            section("Menu", menu_wide, primary).flex(1).min_height(5),
        ]);

        let wide = Pane::new()
            .horizontal()
            .gap(2)
            .flex(1)
            .min_height(0)
            .children([
                build_portrait_pane(palette, status).flex(1).min_width(22),
                Pane::new().flex(2).min_width(36).children([right]),
            ]);

        // ── Stacked arrangement: stats / portrait / activity / menu ──────────
        let menu_narrow = make_menu(palette);
        let menu_narrow_id = menu_narrow.get_id();

        let narrow = Pane::new()
            .vertical()
            .gap(1)
            .flex(1)
            .min_height(0)
            .children([
                build_stat_row(palette, status) as Box<dyn Widget>,
                build_portrait_pane(palette, status).flex(1).min_height(8),
                // Cap activity height so it doesn't push the menu off-screen
                // in short terminals.
                section("Recent Activity", build_activity(palette, status), dim)
                    .flex(1)
                    .min_height(3)
                    .max_height(10),
                section("Menu", menu_narrow, primary).min_height(6),
            ]);

        let body = Responsive::new(WIDE_BREAKPOINT, wide, narrow)
            .flex(1)
            .min_height(0);

        let root = Pane::new()
            .vertical()
            .gap(1)
            .padding(Spacing::new().horizontal(1).top(1).bottom(1))
            .children([title as Box<dyn Widget>, body, build_footer(palette)]);

        let action = menu_action.clone();
        let this = Box::new(Self {
            root,
            menu_wide_id,
            menu_narrow_id,
            title_id,
            selected: 0,
            menu_len,
            primary,
            tick: 0,
            menu_action,
        });
        (this, action)
    }

    /// Start (and keep) the breathing-title animation.
    ///
    /// Self-reschedules on a timer; once this screen leaves the widget tree the
    /// scheduled callback can no longer resolve our id, so the loop ends on its
    /// own. Call once, after the screen is installed as the active widget.
    pub fn schedule_breathe(&self) {
        let id = self.get_id();
        tuie::schedule(
            id,
            std::time::Duration::from_millis(90),
            |w: &mut WelcomeScreen| {
                w.breathe();
                w.schedule_breathe();
            },
        );
    }

    fn breathe(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        let color = breathe_color(self.primary, self.tick);
        if let Some(t) = self.root.get_widget_mut(self.title_id) {
            t.set_primary_color(color);
        }
    }

    pub fn move_menu_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.apply_selection();
        }
    }

    pub fn move_menu_down(&mut self) {
        if self.selected + 1 < self.menu_len {
            self.selected += 1;
            self.apply_selection();
        }
    }

    /// Mirror the current selection into both arrangements' menus so the choice
    /// survives a resize that switches layouts.
    fn apply_selection(&mut self) {
        let sel = self.selected;
        if let Some(m) = self.root.get_widget_mut(self.menu_wide_id) {
            m.select(sel);
        }
        if let Some(m) = self.root.get_widget_mut(self.menu_narrow_id) {
            m.select(sel);
        }
    }

    /// Activate a menu entry if it is available (the signal `TuieApp` drains).
    fn choose(&mut self, idx: usize) {
        if menu_items().get(idx).map(|i| i.available).unwrap_or(false) {
            self.menu_action.set(Some(idx));
        }
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_menu_label(&self) -> Option<String> {
        menu_items().get(self.selected).map(|i| i.label.to_string())
    }
}

// ── Breathing colour ─────────────────────────────────────────────────────────
//
// `breathe_color` now lives in `crate::ui::widgets::stats` so the unified agent
// screen can share it.

// ── Sub-widget builders ──────────────────────────────────────────────────────

/// One dashboard menu entry.
struct MenuItem {
    label: &'static str,
    description: &'static str,
    available: bool,
}

fn menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem {
            label: "Chat",
            description: "Talk with your agent",
            available: true,
        },
        MenuItem {
            label: "Agents",
            description: "Select an agent",
            available: true,
        },
        MenuItem {
            label: "Schedule",
            description: "Cron jobs & tasks",
            available: true,
        },
        MenuItem {
            label: "Settings",
            description: "Configure",
            available: true,
        },
    ]
}

/// Menu rows as content for [`SelectList`] (selection prefix/tint owned by it).
fn menu_rows() -> Vec<StyledString> {
    menu_items()
        .iter()
        .map(|item| {
            let mut row = StyledString::new();
            let label = format!("{:<12}", item.label);
            if item.available {
                row.push_span(StyledStr::new(&label).bold());
                row.push_span(
                    StyledStr::new(&format!("- {}", item.description)).fg(Color::BRIGHT_BLACK),
                );
            } else {
                row.push_span(StyledStr::new(&label).fg(Color::BRIGHT_BLACK));
                row.push_span(
                    StyledStr::new(&format!("- {}  (coming soon)", item.description))
                        .fg(Color::BRIGHT_BLACK)
                        .italic(),
                );
            }
            row
        })
        .collect()
}

fn make_menu(palette: &ChatPalette) -> Box<SelectList> {
    let primary = theme::to_tuie_color(palette.agent_primary);
    let dim = theme::to_tuie_color(palette.agent_dim);
    SelectList::new().colors(primary, dim).items(menu_rows())
}

/// A bordered pane with a coloured title bar at the top.
fn section(title: &str, content: Box<dyn Widget>, border_color: Color) -> Box<Pane> {
    Pane::new()
        .vertical()
        .bordered()
        .border_style(Style::new().fg(border_color).dim())
        .children([
            Text::new().content(format!(" {} ", title).fg(border_color).bold()),
            content,
        ])
}

/// Portrait inside a bordered pane whose border surfaces the subconscious
/// state — "surfacing" colour while active, primary otherwise.
fn build_portrait_pane(palette: &ChatPalette, status: &AgentStatus) -> Box<Pane> {
    let primary = theme::to_tuie_color(palette.agent_primary);
    let border = if status.subconscious_active {
        theme::to_tuie_color(palette.surfacing)
    } else {
        primary
    };

    let portrait = Portrait::new(status.agent_id.as_deref(), &status.name, primary);

    let glyph = if status.subconscious_active {
        "\u{25C8}"
    } else {
        "\u{00B7}"
    };
    let mut name_line = StyledString::new();
    name_line.push_span(StyledStr::new(&format!("{glyph} ")).fg(border));
    name_line.push_span(StyledStr::new(&status.name).fg(border).bold());

    Pane::new()
        .vertical()
        .bordered()
        .border_style(Style::new().fg(border).dim())
        .children([
            Pane::new()
                .flex(1)
                .min_height(0)
                .x_place(Place::Center)
                .y_place(Place::Center)
                .children([portrait]) as Box<dyn Widget>,
            Text::new().content(name_line).center(),
        ])
}

fn build_stat_row(palette: &ChatPalette, status: &AgentStatus) -> Box<dyn Widget> {
    let primary = theme::to_tuie_color(palette.agent_primary);
    let dim = theme::to_tuie_color(palette.agent_dim);

    let energy_color = match status.energy {
        0..=30 => Color::RED,
        31..=60 => Color::YELLOW,
        _ => Color::GREEN,
    };

    let bar = {
        let filled = (status.energy as usize).div_ceil(20); // 0..5
        let filled = filled.min(5);
        format!(
            "{}{}",
            "\u{25B0}".repeat(filled),
            "\u{25B1}".repeat(5 - filled),
        )
    };
    let energy_value = format!("\u{26A1} {}%\n{}", status.energy, bar);

    let mood_glyph = if status.subconscious_active {
        "\u{25CC}"
    } else {
        "\u{00B7}"
    };
    let mood_value = format!("{mood_glyph}\n{}", status.mood);

    let memory_value = match &status.last_commit {
        Some(c) => {
            let short = if c.len() > 7 { &c[..7] } else { c };
            format!("\u{1F4BE} {} files\n{}", status.memory_commits, short)
        }
        None => format!("\u{1F4BE} {} files", status.memory_commits),
    };

    let backend_value = format!(
        "\u{1F465} {} agent{}\non {}",
        status.agent_count,
        if status.agent_count == 1 { "" } else { "s" },
        status.mode,
    );

    Pane::new().horizontal().gap(1).children([
        make_card("Energy", &energy_value, energy_color),
        make_card("State", &mood_value, primary),
        make_card("Memory", &memory_value, dim),
        make_card("Backend", &backend_value, dim),
    ])
}

/// A bordered card with a coloured title and a centred value.
fn make_card(label: &str, value: &str, color: Color) -> Box<Pane> {
    let mut content = StyledString::new();
    content.push_span(StyledStr::new(value).bold().fg(color));

    Pane::new()
        .vertical()
        .bordered()
        .border_style(Style::new().fg(color).dim())
        .flex(1)
        .min_width(14)
        .children([
            Text::new().content(format!(" {label} ").fg(color).bold()) as Box<dyn Widget>,
            Pane::new()
                .flex(1)
                .min_height(0)
                .x_place(Place::Center)
                .y_place(Place::Center)
                .children([Text::new().content(content).center()]),
        ])
}

fn build_activity(palette: &ChatPalette, status: &AgentStatus) -> Box<dyn Widget> {
    let dim = theme::to_tuie_color(palette.agent_dim);
    let mut content = StyledString::new();
    content.push_str("\n");

    if status.recent_activity.is_empty() {
        content.push_span(
            StyledStr::new("  (no recent activity — open Chat to begin)\n")
                .fg(dim)
                .italic(),
        );
    } else {
        for line in &status.recent_activity {
            content.push_span(StyledStr::new(&format!("  {line}\n")).fg(dim));
        }
    }

    content.push_str("\n");
    Text::new().content(content)
}

fn build_footer(palette: &ChatPalette) -> Box<dyn Widget> {
    let dim = theme::to_tuie_color(palette.agent_dim);
    let mut content = StyledString::new();
    content.push_span(
        StyledStr::new("  \u{2191}\u{2193} Navigate \u{2022} Enter select \u{2022} a Add \u{2022} i Inspect \u{2022} p Presence \u{2022} q Quit").fg(dim),
    );
    Text::new().content(content)
}
