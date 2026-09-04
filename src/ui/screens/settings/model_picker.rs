#![allow(dead_code)] // WIP scaffolding not yet wired
//! Model picker sub-page for the settings screen.
//!
//! A collapsible tree (Source → Org → Model) with a fuzzy search bar on top.
//! Sources and orgs collapse so a catalog of hundreds of models stays
//! navigable; typing fuzzy-filters and auto-expands every group that matches.
//!
//! Layout:
//!   ┌─ Select Model ──────────────────────────┐
//!   │ 🔍 hermes▏                          [↻]  │
//!   │─────────────────────────────────────────│
//!   │ ▾ huggingface  (412)                     │
//!   │   ▾ featherless-ai/NousResearch  (2)     │
//!   │     ▶ Hermes-3-Llama-3.1-8B             │
//!   │       Hermes-3-Llama-3.1-70B           │
//!   │ ▸ openai  (9)                            │
//!   │─────────────────────────────────────────│
//!   │ huggingface/featherless-ai/.../Hermes-3 │
//!   └─────────────────────────────────────────┘
//!
//! Keyboard:
//!   Type      → fuzzy filter (auto-expands matches)
//!   ↑/↓       → move cursor (over headers and models alike)
//!   →         → expand the highlighted group
//!   ←         → collapse the highlighted group (or a model's parent)
//!   Enter     → select model, or toggle a group header
//!   Esc       → clear query (first), close (second)
//!   r         → refresh models from server

use std::collections::{BTreeMap, HashSet};

use tuie::input::key::Key;
use tuie::input::trigger::Trigger;
use tuie::prelude::*;

use crate::ui::chat::ChatPalette;
use crate::ui::theme;
use crate::ui::widgets::button::Button;

/// One visible row in the rendered tree.
#[derive(Clone)]
enum Row {
    /// Source header (path segment 0). Collapsible.
    Source {
        key: String,
        label: String,
        count: usize,
        collapsed: bool,
    },
    /// Org header (middle path segments). Collapsible.
    Org {
        key: String,
        label: String,
        count: usize,
        collapsed: bool,
    },
    /// A selectable model. `idx` indexes into the full `models` vec.
    Model {
        idx: usize,
        name: String,
        indent: u16,
    },
}

impl Row {
    /// The collapse key for a header row, if this is a header.
    fn header_key(&self) -> Option<&str> {
        match self {
            Row::Source { key, .. } | Row::Org { key, .. } => Some(key),
            Row::Model { .. } => None,
        }
    }
}

/// Model picker sub-page widget.
pub struct ModelPicker {
    root: Box<Pane>,
    input_id: WidgetId<Input>,
    list_id: WidgetId<List>,
    refresh_button_id: WidgetId<Button>,
    footer_id: WidgetId<Text>,

    models: Vec<String>,
    rows: Vec<Row>,
    /// Group keys that are currently collapsed.
    collapsed: HashSet<String>,
    selected: usize, // index into `rows`
    query: String,

    accent: Color,
    dim: Color,
    label: Color,
}

/// Renderer context carried by the List widget.
struct PickerCtx {
    rows: Vec<Row>,
    selected: usize,
    accent: Color,
    dim: Color,
    label: Color,
}

// ── Fuzzy matching ──────────────────────────────────────────────────────────

fn fuzzy_score(query: &str, path: &str) -> Option<i32> {
    let q = query.to_lowercase();
    let p = path.to_lowercase();

    // Exact substring match — best signal.
    if p.contains(&q) {
        return Some(1000 - path.len() as i32);
    }

    // All tokens must appear somewhere in the path.
    let tokens: Vec<&str> = q.split_whitespace().collect();
    if tokens.is_empty() {
        return Some(0);
    }

    let mut score = 0i32;
    for tok in &tokens {
        let pos = p.find(tok)?;
        score += 100 - pos as i32;
    }
    score -= path.len() as i32 / 4;
    Some(score)
}

/// Split a model path into (source, org, model_name).
/// "huggingface/fal-ai/renderartist/coloringbookflux"
///   → ("huggingface", "fal-ai/renderartist", "coloringbookflux")
/// "openai/gpt-4" → ("openai", "", "gpt-4")
/// "gpt-4"        → ("gpt-4", "", "gpt-4")
fn split_model_path(path: &str) -> (String, String, String) {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.len() {
        0 | 1 => (path.to_string(), String::new(), path.to_string()),
        2 => (parts[0].to_string(), String::new(), parts[1].to_string()),
        _ => (
            parts[0].to_string(),
            parts[1..parts.len() - 1].join("/"),
            parts[parts.len() - 1].to_string(),
        ),
    }
}

/// Org key for collapse tracking.
fn org_key(source: &str, org: &str) -> String {
    format!("{source}/{org}")
}

/// Every collapsible group key present in the catalog.
fn all_group_keys(models: &[String]) -> HashSet<String> {
    let mut keys = HashSet::new();
    for path in models {
        let (source, org, _) = split_model_path(path);
        if !org.is_empty() {
            keys.insert(org_key(&source, &org));
        }
        keys.insert(source);
    }
    keys
}

/// Build the visible rows from the catalog, query, and collapsed set.
/// A non-empty query force-expands every group that has a match.
fn build_rows(models: &[String], query: &str, collapsed: &HashSet<String>) -> Vec<Row> {
    let force_expand = !query.is_empty();

    // Which models survive the filter.
    let included: Vec<usize> = if query.is_empty() {
        (0..models.len()).collect()
    } else {
        models
            .iter()
            .enumerate()
            .filter(|(_, p)| fuzzy_score(query, p).is_some())
            .map(|(i, _)| i)
            .collect()
    };

    // source -> org ("" = directly under source) -> [(idx, model_name)]
    let mut tree: BTreeMap<String, BTreeMap<String, Vec<(usize, String)>>> = BTreeMap::new();
    for &idx in &included {
        let (source, org, name) = split_model_path(&models[idx]);
        tree.entry(source)
            .or_default()
            .entry(org)
            .or_default()
            .push((idx, name));
    }

    let mut rows = Vec::new();
    for (source, orgs) in &tree {
        let count: usize = orgs.values().map(Vec::len).sum();
        let src_collapsed = !force_expand && collapsed.contains(source);
        rows.push(Row::Source {
            key: source.clone(),
            label: source.clone(),
            count,
            collapsed: src_collapsed,
        });
        if src_collapsed {
            continue;
        }
        for (org, entries) in orgs {
            let mut entries = entries.clone();
            entries.sort_by(|a, b| a.1.cmp(&b.1));
            if org.is_empty() {
                for (idx, name) in entries {
                    rows.push(Row::Model {
                        idx,
                        name,
                        indent: 2,
                    });
                }
            } else {
                let key = org_key(source, org);
                let org_collapsed = !force_expand && collapsed.contains(&key);
                rows.push(Row::Org {
                    key,
                    label: org.clone(),
                    count: entries.len(),
                    collapsed: org_collapsed,
                });
                if !org_collapsed {
                    for (idx, name) in entries {
                        rows.push(Row::Model {
                            idx,
                            name,
                            indent: 5,
                        });
                    }
                }
            }
        }
    }
    rows
}

// ── ModelPicker implementation ──────────────────────────────────────────────

impl ModelPicker {
    /// Creates a new model picker with the given models and initially selected model.
    pub fn new(models: &[String], selected_model: &str, palette: &ChatPalette) -> Box<Self> {
        let accent = theme::to_tuie_color(palette.agent_primary);
        let dim = Color::grey256(6);
        let label = Color::grey256(18);

        // Start with everything collapsed except the path to the selected model.
        let mut collapsed = all_group_keys(models);
        if let Some(sel) = models.iter().find(|m| m.as_str() == selected_model) {
            let (source, org, _) = split_model_path(sel);
            collapsed.remove(&source);
            if !org.is_empty() {
                collapsed.remove(&org_key(&source, &org));
            }
        }

        let mut input_id = WidgetId::EMPTY;
        let mut list_id = WidgetId::EMPTY;
        let mut refresh_button_id = WidgetId::EMPTY;
        let mut footer_id = WidgetId::EMPTY;

        // ── Search bar ──────────────────────────────────────────────────
        let search_input = Input::new()
            .placeholder(Text::new().content(StyledStr::new(" fuzzy search\u{2026} ").fg(dim)))
            .id(&mut input_id);

        let refresh_button = Button::new()
            .children([Text::new().content(StyledStr::new(" \u{21bb} ").fg(Color::BLACK).bold())])
            .id(&mut refresh_button_id);

        let search_bar = Pane::new().horizontal().gap(1).children([
            Text::new().content(StyledStr::new(" \u{1f50d} ").fg(dim)) as Box<dyn Widget>,
            search_input as Box<dyn Widget>,
            refresh_button as Box<dyn Widget>,
        ]);

        // ── Results tree ──────────────────────────────────────────────────
        let rows = build_rows(models, "", &collapsed);
        let selected = rows
            .iter()
            .position(|r| {
                matches!(r, Row::Model { name, .. } if {
                    let (_, _, n) = split_model_path(selected_model);
                    *name == n
                })
            })
            .unwrap_or(0);

        let mut list = List::new()
            .vertical()
            .flex(1)
            .min_height(3)
            .gap(0)
            .scroll(Scrollbar::AutoHide)
            .bordered()
            .border_style(Style::new().fg(Color::grey256(4)).dim());

        let ctx = PickerCtx {
            rows: rows.clone(),
            selected,
            accent,
            dim,
            label,
        };
        list.set_renderer(ctx, render_row);
        list.set_item_count(rows.len());
        let list_widget = list.id(&mut list_id);

        // ── Footer (preview) ────────────────────────────────────────────
        let preview = match rows.get(selected) {
            Some(Row::Model { idx, .. }) => models.get(*idx).cloned().unwrap_or_default(),
            _ => String::new(),
        };
        let footer = Text::new()
            .content(StyledStr::new(&format!(" {preview}")).fg(dim))
            .id(&mut footer_id);

        // ── Root ────────────────────────────────────────────────────────
        let root = Pane::new().vertical().flex(1).children([
            search_bar as Box<dyn Widget>,
            list_widget as Box<dyn Widget>,
            footer as Box<dyn Widget>,
        ]);

        Box::new(Self {
            root,
            input_id,
            list_id,
            refresh_button_id,
            footer_id,
            models: models.to_vec(),
            rows,
            collapsed,
            selected,
            query: String::new(),
            accent,
            dim,
            label,
        })
    }

    /// Returns the ID of the refresh button (for event matching).
    pub fn refresh_button_id(&self) -> WidgetId<Button> {
        self.refresh_button_id
    }

    /// Returns the ID of the list (for event matching).
    pub fn list_id(&self) -> WidgetId<List> {
        self.list_id
    }

    /// Returns the currently selected model name.
    pub fn selected_model(&self) -> Option<&str> {
        match self.rows.get(self.selected)? {
            Row::Model { idx, .. } => self.models.get(*idx).map(String::as_str),
            _ => None,
        }
    }

    /// Returns the currently selected model name as an owned String.
    pub fn selected_model_name(&self) -> Option<String> {
        self.selected_model().map(str::to_string)
    }

    /// Updates the model list (e.g., after a refresh).
    pub fn set_models(&mut self, models: Vec<String>) {
        // Newly-seen groups default to collapsed (unless filtering).
        self.collapsed = all_group_keys(&models)
            .into_iter()
            .filter(|k| self.collapsed.contains(k) || !self.models_contains_group(k))
            .collect();
        self.models = models;
        self.rebuild();
    }

    fn models_contains_group(&self, key: &str) -> bool {
        all_group_keys(&self.models).contains(key)
    }

    /// Rebuild rows from current state, keeping the cursor stable.
    fn rebuild(&mut self) {
        let anchor = self.selection_anchor();
        self.rows = build_rows(&self.models, &self.query, &self.collapsed);
        self.selected = self
            .restore_anchor(anchor)
            .min(self.rows.len().saturating_sub(1));
        self.sync_renderer();
        self.sync_footer();
    }

    /// Capture what the cursor points at, so it survives a rebuild.
    fn selection_anchor(&self) -> Option<Anchor> {
        match self.rows.get(self.selected)? {
            Row::Model { idx, .. } => Some(Anchor::Model(*idx)),
            Row::Source { key, .. } | Row::Org { key, .. } => Some(Anchor::Header(key.clone())),
        }
    }

    fn restore_anchor(&self, anchor: Option<Anchor>) -> usize {
        let Some(anchor) = anchor else { return 0 };
        self.rows
            .iter()
            .position(|r| match (&anchor, r) {
                (Anchor::Model(i), Row::Model { idx, .. }) => i == idx,
                (Anchor::Header(k), _) => r.header_key() == Some(k.as_str()),
                _ => false,
            })
            .unwrap_or(0)
    }

    fn row_index_of_key(&self, key: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.header_key() == Some(key))
    }

    fn sync_renderer(&mut self) {
        if let Some(list) = self.root.get_widget_mut(self.list_id) {
            if let Some(ctx) = list.get_context_mut::<PickerCtx>() {
                ctx.rows.clone_from(&self.rows);
                ctx.selected = self.selected;
            }
            list.set_item_count(self.rows.len());
        }
        tuie::dirty_paint();
    }

    fn sync_footer(&mut self) {
        let preview = self.selected_model().unwrap_or("").to_string();
        if let Some(footer) = self.root.get_widget_mut(self.footer_id) {
            footer.set_content(StyledStr::new(&format!(" {preview}")).fg(self.dim));
        }
    }

    /// Move the cursor by `delta`, clamped.
    fn move_selection(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let max = self.rows.len() as i32 - 1;
        self.selected = (self.selected as i32 + delta).clamp(0, max) as usize;
        self.sync_renderer();
        self.sync_footer();
    }

    /// Set a group's collapsed state, then rebuild keeping the header in view.
    fn set_collapsed(&mut self, key: &str, collapsed: bool) {
        if collapsed {
            self.collapsed.insert(key.to_string());
        } else {
            self.collapsed.remove(key);
        }
        // Collapsing only matters when there is no active query.
        self.rows = build_rows(&self.models, &self.query, &self.collapsed);
        self.selected = self
            .row_index_of_key(key)
            .unwrap_or(self.selected)
            .min(self.rows.len().saturating_sub(1));
        self.sync_renderer();
        self.sync_footer();
    }

    /// Enter / right / left act on the row under the cursor.
    fn activate(&mut self) {
        match self.rows.get(self.selected).cloned() {
            Some(Row::Model { name, .. }) => {
                tuie::emit(self.list_id, ChangeEvent(name));
            }
            Some(Row::Source { key, collapsed, .. }) | Some(Row::Org { key, collapsed, .. }) => {
                self.set_collapsed(&key, !collapsed);
            }
            None => {}
        }
    }

    fn expand(&mut self) {
        if let Some(
            Row::Source {
                key,
                collapsed: true,
                ..
            }
            | Row::Org {
                key,
                collapsed: true,
                ..
            },
        ) = self.rows.get(self.selected).cloned().as_ref()
        {
            self.set_collapsed(key, false);
        }
    }

    fn collapse(&mut self) {
        match self.rows.get(self.selected).cloned() {
            Some(Row::Source {
                key,
                collapsed: false,
                ..
            })
            | Some(Row::Org {
                key,
                collapsed: false,
                ..
            }) => {
                self.set_collapsed(&key, true);
            }
            Some(Row::Model { idx, .. }) => {
                // Collapse the model's nearest parent group and land on it.
                let (source, org, _) = split_model_path(&self.models[idx]);
                let key = if org.is_empty() {
                    source
                } else {
                    org_key(&source, &org)
                };
                self.set_collapsed(&key, true);
            }
            _ => {}
        }
    }

    /// Handle a query change from the Input widget.
    fn on_query_change(&mut self, new_query: String) {
        self.query = new_query;
        self.rows = build_rows(&self.models, &self.query, &self.collapsed);
        // Land on the first selectable model.
        self.selected = self
            .rows
            .iter()
            .position(|r| matches!(r, Row::Model { .. }))
            .unwrap_or(0);
        self.sync_renderer();
        self.sync_footer();
    }
}

/// What the cursor is pinned to across a rebuild.
enum Anchor {
    Model(usize),
    Header(String),
}

/// List row renderer.
fn render_row(ctx: &mut PickerCtx, idx: usize) -> Option<Box<dyn Widget>> {
    let row = ctx.rows.get(idx)?;
    let is_sel = idx == ctx.selected;
    let text = match row {
        Row::Source {
            label,
            count,
            collapsed,
            ..
        } => {
            let chev = if *collapsed { "\u{25b8}" } else { "\u{25be}" };
            format!(" {chev} {label}  ({count})")
        }
        Row::Org {
            label,
            count,
            collapsed,
            ..
        } => {
            let chev = if *collapsed { "\u{25b8}" } else { "\u{25be}" };
            format!("   {chev} {label}  ({count})")
        }
        Row::Model { name, indent, .. } => {
            let pad = " ".repeat(*indent as usize);
            let marker = if is_sel { "\u{25b6}" } else { " " };
            format!("{pad}{marker} {name}")
        }
    };

    let base = match row {
        Row::Source { .. } => ctx.label,
        Row::Org { .. } => ctx.dim,
        Row::Model { .. } => ctx.dim,
    };
    let color = if is_sel { ctx.accent } else { base };
    let mut s = StyledStr::new(&text).fg(color);
    if is_sel || matches!(row, Row::Source { .. }) {
        s = s.bold();
    }
    Some(Text::new().content(s) as Box<dyn Widget>)
}

impl DelegateWidget for ModelPicker {
    tuie::delegate_widget!(root);

    fn override_is_focusable(&self) -> bool {
        true
    }

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.peek() else {
            return InputResult::Rejected;
        };

        // Navigation and action keys — intercept before the Input widget.
        match &event.chord.trigger {
            Trigger::Key(Key::Arrow(Direction2D::Up)) => {
                queue.next();
                self.move_selection(-1);
                return InputResult::Handled;
            }
            Trigger::Key(Key::Arrow(Direction2D::Down)) => {
                queue.next();
                self.move_selection(1);
                return InputResult::Handled;
            }
            Trigger::Key(Key::Arrow(Direction2D::Right)) => {
                queue.next();
                self.expand();
                return InputResult::Handled;
            }
            Trigger::Key(Key::Arrow(Direction2D::Left)) => {
                queue.next();
                self.collapse();
                return InputResult::Handled;
            }
            Trigger::Key(Key::Char('r')) if self.query.is_empty() => {
                queue.next();
                tuie::emit(self.refresh_button_id, ClickEvent);
                return InputResult::Handled;
            }
            Trigger::Key(Key::Enter) => {
                queue.next();
                self.activate();
                return InputResult::Handled;
            }
            Trigger::Key(Key::Esc) => {
                queue.next();
                if self.query.is_empty() {
                    // No query — let Esc bubble up to close the sub-page.
                    return InputResult::Rejected;
                }
                // Clear the query first.
                self.on_query_change(String::new());
                if let Some(input) = self.root.get_widget_mut(self.input_id) {
                    input.set_content("");
                }
                return InputResult::Handled;
            }
            _ => {}
        }

        // All other keys (typing) → delegate to the Input widget, which emits
        // ChangeEvent<String> that we catch in after_on_event.
        self.get_delegate_mut().on_input(queue)
    }

    fn after_on_event(&mut self, event: &mut WidgetEvent) {
        if let Some(ChangeEvent(text)) = event.get::<ChangeEvent<String>>() {
            if event.source == self.input_id.untyped() {
                let q = text.clone();
                self.on_query_change(q);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models() -> Vec<String> {
        vec![
            "openai/gpt-4".to_string(),
            "openai/gpt-4o-mini".to_string(),
            "anthropic/claude-3".to_string(),
            "huggingface/fal-ai/renderartist/coloringbookflux".to_string(),
            "huggingface/NousResearch/Hermes-3-Llama-3.1-8B".to_string(),
        ]
    }

    #[test]
    fn split_three_levels() {
        assert_eq!(
            split_model_path("huggingface/fal-ai/renderartist/x"),
            (
                "huggingface".into(),
                "fal-ai/renderartist".into(),
                "x".into()
            )
        );
        assert_eq!(
            split_model_path("openai/gpt-4"),
            ("openai".into(), String::new(), "gpt-4".into())
        );
        assert_eq!(
            split_model_path("gpt-4"),
            ("gpt-4".into(), String::new(), "gpt-4".into())
        );
    }

    #[test]
    fn collapsed_source_hides_its_models() {
        let m = models();
        let mut collapsed = HashSet::new();
        collapsed.insert("openai".to_string());
        let rows = build_rows(&m, "", &collapsed);
        // openai header present, but no openai models rendered.
        assert!(rows
            .iter()
            .any(|r| matches!(r, Row::Source { label, .. } if label == "openai")));
        assert!(!rows
            .iter()
            .any(|r| matches!(r, Row::Model { name, .. } if name.starts_with("gpt"))));
    }

    #[test]
    fn query_force_expands_matches() {
        let m = models();
        let mut collapsed = all_group_keys(&m); // everything collapsed
        collapsed.insert("openai".to_string());
        let rows = build_rows(&m, "hermes", &collapsed);
        // Despite collapse, the matching model is visible.
        assert!(rows
            .iter()
            .any(|r| matches!(r, Row::Model { name, .. } if name.contains("Hermes"))));
        // Non-matching gpt models are filtered out.
        assert!(!rows
            .iter()
            .any(|r| matches!(r, Row::Model { name, .. } if name.starts_with("gpt"))));
    }

    #[test]
    fn org_grouping_under_source() {
        let m = models();
        let rows = build_rows(&m, "", &HashSet::new());
        let has_org = rows
            .iter()
            .any(|r| matches!(r, Row::Org { label, .. } if label == "fal-ai/renderartist"));
        assert!(has_org);
    }
}
