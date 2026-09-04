#![allow(dead_code)] // WIP scaffolding not yet wired
//! Settings screen — interactive two-column browse over [`SettingsView`].
//!
//! Layout (wide, ≥80 cols):
//!   root (Pane, vertical, flex=1)
//!     header (Text) — "⚙ Settings"
//!     body (Pane, horizontal, flex=1)
//!       categories (Pane, vertical, width=26, bordered)
//!         Category items as FocusPane-wrapped Text rows
//!       page_layout (PageLayout, flex=1, bordered)
//!         main_page: field grid (Checkbox, Counter, SegmentedControl per field)
//!         sub-pages: model picker, etc.
//!     footer (Text) — contextual hints
//!
//! Layout (narrow, <80 cols):
//!   root (Pane, vertical, flex=1)
//!     header (Text)
//!     tabs (SegmentedControl) — category tabs
//!     page_layout (PageLayout, flex=1)
//!       main_page: field grid
//!       sub-pages
//!     footer (Text)
//!
//! Navigation:
//!   - Up/Down       move selection within the focused column
//!   - Tab / Right   move focus categories → fields
//!   - Shift+Tab / Left  move focus fields → categories
//!   - Enter         activate the selected field (toggle bool, cycle enum, open picker)
//!   - Left/Right     on an enum field, cycle variants
//!   - Esc           pop sub-page, or fields → categories, or bubble up

pub mod actions;
pub mod field_grid;
pub mod model_picker;
pub mod text_editor;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use tuie::input::key::Key;
use tuie::input::modifiers::Modifier;
use tuie::input::trigger::Trigger;
use tuie::prelude::*;

use crate::core::config::ConsciousnessConfig;
use crate::ui::chat::{ChatPalette, SPINNER};
use crate::ui::settings::SettingsView;
use crate::ui::settings::{Category, EditableValue, FieldLoc};
use crate::ui::theme;
use crate::ui::widgets::accordion::Accordion;
use crate::ui::widgets::button::Button;
use crate::ui::widgets::page_layout::PageLayout;
use crate::ui::widgets::segmented_control::SegmentedControl;

use actions::SettingsAction;
use field_grid::FieldGrid;
use model_picker::ModelPicker;
use text_editor::TextEditor;

const WIDE_BREAKPOINT: u16 = 80;
const FOOTER_HINT: &str = " Tab: fields \u{2502} Enter: toggle \u{2502} \u{2190}\u{2192}: cycle \u{2502} Ctrl+S: save \u{2502} Esc: back ";

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusCol {
    Categories,
    Fields,
}

/// The settings screen widget.
pub struct SettingsScreen {
    root: Box<Pane>,
    view: SettingsView,
    palette: ChatPalette,
    focus_col: FocusCol,
    /// Category selection mirrors between wide/narrow arrangements.
    cat_idx: usize,
    /// Field selection within the current category.
    field_idx: usize,

    // Signals for TuieApp to check after input processing.
    /// Set to true when the user wants to save and go back.
    go_back_signal: Rc<Cell<bool>>,
    /// Set to true when the user wants to save (stay on screen).
    save_signal: Rc<Cell<bool>>,
    /// Set to true when models should be fetched.
    fetch_models_signal: Rc<Cell<bool>>,
    /// Set when the atmosphere field is edited. Carries the new atmosphere name.
    atmosphere_changed_signal: Rc<Cell<Option<String>>>,
    /// Set when the outfit field is edited. Carries the new outfit name.
    outfit_changed_signal: Rc<Cell<Option<String>>>,
    /// Shared buffer for fetched models. Written by TuieApp's async task.
    models_buffer: Rc<RefCell<Option<Vec<String>>>>,
    /// Path to the config file for saving.
    config_path: PathBuf,

    // Wide layout IDs
    wide_cat_text_id: WidgetId<Text>,
    wide_accordion_id: WidgetId<Accordion>,
    wide_field_grid_id: WidgetId<Pane>,
    wide_page_layout_id: WidgetId<PageLayout>,

    // Narrow layout IDs
    narrow_tab_id: WidgetId<SegmentedControl>,
    narrow_field_grid_id: WidgetId<Pane>,
    narrow_page_layout_id: WidgetId<PageLayout>,

    // Maps each field's value-widget id (wide + narrow) to its FieldLoc, so a
    // widget event can be routed back to the field that produced it. Rebuilt
    // alongside the field grid.
    field_map: Vec<(WidgetId, FieldLoc)>,

    // Action queue drained each frame.
    action_queue: Vec<SettingsAction>,

    // Sub-page state
    active_model_picker_field: Option<FieldLoc>,
    model_picker_id: WidgetId<ModelPicker>,

    // Text editor sub-page state
    active_text_editor_field: Option<FieldLoc>,
    text_editor_input_id: Option<WidgetId<Input>>,
    text_editor_save_id: Option<WidgetId<Button>>,
    text_editor_cancel_id: Option<WidgetId<Button>>,
    text_editor_multiline: bool,

    // Footer widget id — used to show spinner during model fetch.
    footer_id: WidgetId<Text>,
    /// Current spinner animation frame index.
    spinner_frame: usize,
    /// Scheduled task that advances the spinner.
    spinner_task: TaskHandle,
}

impl DelegateWidget for SettingsScreen {
    tuie::delegate_widget!(root);
    fn override_is_focusable(&self) -> bool {
        true
    }

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        // Poll for fetched models.
        self.poll_models();

        let Some(event) = queue.peek() else {
            return InputResult::Rejected;
        };
        let Trigger::Key(key) = &event.chord.trigger else {
            return self.get_delegate_mut().on_input(queue);
        };
        let key = *key;
        let shift = event.chord.modifiers.has(Modifier::Shift);

        // When a text editor sub-page is active, route most input to the
        // Input widget so the user can type, move the cursor, etc.  Only
        // intercept Esc (cancel), Ctrl+S (save), and Enter on single-line
        // fields (save).
        if self.active_text_editor_field.is_some() {
            return match key {
                Key::Esc => {
                    queue.next();
                    self.clear_text_editor();
                    self.pop_sub_page();
                    InputResult::Handled
                }
                Key::Char('s') if event.chord.modifiers.has(Modifier::Ctrl) => {
                    queue.next();
                    self.commit_text_editor();
                    InputResult::Handled
                }
                Key::Enter if !self.text_editor_multiline => {
                    queue.next();
                    self.commit_text_editor();
                    InputResult::Handled
                }
                _ => self.get_delegate_mut().on_input(queue),
            };
        }

        // When the model picker sub-page is active, route input to it. The
        // picker handles its own navigation, fuzzy filtering, Enter-to-select,
        // and 'r' to refresh. Esc with an empty query bubbles back as Rejected
        // — close the sub-page here.
        if self.active_model_picker_field.is_some() {
            let is_esc = matches!(key, Key::Esc);
            let result = self.get_delegate_mut().on_input(queue);
            if is_esc && matches!(result, InputResult::Rejected) {
                queue.next();
                self.process_action(SettingsAction::PopPage);
                return InputResult::Handled;
            }
            return result;
        }

        match key {
            Key::Arrow(Direction2D::Up) => {
                queue.next();
                self.process_action(SettingsAction::MoveSelection(-1));
                InputResult::Handled
            }
            Key::Arrow(Direction2D::Down) => {
                queue.next();
                self.process_action(SettingsAction::MoveSelection(1));
                InputResult::Handled
            }
            Key::Tab if shift => {
                queue.next();
                self.process_action(SettingsAction::FocusCategories);
                InputResult::Handled
            }
            Key::Tab => {
                queue.next();
                self.process_action(SettingsAction::FocusFields);
                InputResult::Handled
            }
            Key::Arrow(Direction2D::Right) if self.focus_col == FocusCol::Categories => {
                queue.next();
                self.process_action(SettingsAction::FocusFields);
                InputResult::Handled
            }
            Key::Arrow(Direction2D::Right) => {
                queue.next();
                self.process_action(SettingsAction::CycleEnum(
                    self.selected_field().unwrap_or(FieldLoc::PvName),
                    1,
                ));
                InputResult::Handled
            }
            Key::Arrow(Direction2D::Left) if self.focus_col == FocusCol::Fields => {
                queue.next();
                let loc = self.selected_field().unwrap_or(FieldLoc::PvName);
                if !self.try_cycle_enum(loc, -1) {
                    self.process_action(SettingsAction::FocusCategories);
                }
                InputResult::Handled
            }
            Key::Enter if self.focus_col == FocusCol::Categories => {
                queue.next();
                self.process_action(SettingsAction::FocusFields);
                InputResult::Handled
            }
            Key::Enter => {
                queue.next();
                self.activate_selected_field();
                InputResult::Handled
            }
            Key::Esc if self.page_depth() > 1 => {
                queue.next();
                self.process_action(SettingsAction::PopPage);
                InputResult::Handled
            }
            Key::Esc if self.focus_col == FocusCol::Fields => {
                queue.next();
                self.process_action(SettingsAction::FocusCategories);
                InputResult::Handled
            }
            Key::Char('s') if event.chord.modifiers.has(Modifier::Ctrl) => {
                queue.next();
                self.process_action(SettingsAction::Save);
                InputResult::Handled
            }
            _ => self.get_delegate_mut().on_input(queue),
        }
    }

    fn after_on_event(&mut self, event: &mut WidgetEvent) {
        // Drain the action queue.
        let actions: Vec<_> = self.action_queue.drain(..).collect();
        for action in actions {
            self.process_action(action);
        }

        // Check for widget events from field widgets.
        self.handle_widget_events(event);
    }
}

impl SettingsScreen {
    /// Creates the settings screen from a config snapshot and config file path.
    /// The `models_buffer` is a shared cell that the async fetch writes into.
    pub fn new(
        config: &ConsciousnessConfig,
        config_path: PathBuf,
        models_buffer: Rc<RefCell<Option<Vec<String>>>>,
    ) -> Box<Self> {
        let view = SettingsView::new(config);
        let palette = ChatPalette::default();
        let accent = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);

        // ── Wide layout ───────────────────────────────────────────────────
        let mut wide_cat_text_id = WidgetId::EMPTY;
        let mut wide_accordion_id = WidgetId::EMPTY;
        let mut wide_field_grid_id = WidgetId::EMPTY;
        let mut wide_page_layout_id = WidgetId::EMPTY;

        let cat_content = build_category_content(&view, FocusCol::Categories, &palette);
        let cat_text = Text::new().content(cat_content).id(&mut wide_cat_text_id);

        let categories_pane = Pane::new()
            .vertical()
            .width(26)
            .min_width(20)
            .bordered()
            .border_style(Style::new().fg(dim).dim())
            .padding(Spacing::new().horizontal(1))
            .children([
                Text::new().content(StyledStr::new(" Categories ").bold()) as Box<dyn Widget>,
                cat_text,
            ]);

        let fields = view.fields_for_category(view.selected_category());
        let field_grid = FieldGrid::new(&fields, &palette);
        let field_grid_pane = field_grid.widget().id(&mut wide_field_grid_id);

        let cat_title = view.selected_category().label().to_string();
        let accordion = Accordion::new_expanded(&cat_title, field_grid_pane as Box<dyn Widget>)
            .id(&mut wide_accordion_id);

        let main_page = Pane::new()
            .vertical()
            .flex(1)
            .min_height(0)
            .children([accordion as Box<dyn Widget>]);

        let page_layout = PageLayout::new(main_page).id(&mut wide_page_layout_id);

        let wide = Pane::new()
            .horizontal()
            .flex(1)
            .min_height(0)
            .children([categories_pane, page_layout as Box<dyn Widget>]);

        // ── Narrow layout ─────────────────────────────────────────────────
        let mut narrow_tab_id = WidgetId::EMPTY;
        let mut narrow_field_grid_id = WidgetId::EMPTY;
        let mut narrow_page_layout_id = WidgetId::EMPTY;

        let cat_labels: Vec<&str> = Category::all().iter().map(|c| c.label()).collect();
        let tabs = SegmentedControl::new(&cat_labels).id(&mut narrow_tab_id);

        let narrow_fields = view.fields_for_category(view.selected_category());
        let narrow_field_grid = FieldGrid::new(&narrow_fields, &palette);
        let narrow_field_grid_pane = narrow_field_grid.widget().id(&mut narrow_field_grid_id);

        let narrow_main_page = Pane::new()
            .vertical()
            .flex(1)
            .min_height(0)
            .children([narrow_field_grid_pane]);

        let narrow_page_layout = PageLayout::new(narrow_main_page).id(&mut narrow_page_layout_id);

        let narrow = Pane::new()
            .vertical()
            .flex(1)
            .min_height(0)
            .gap(1)
            .children([
                tabs as Box<dyn Widget>,
                narrow_page_layout as Box<dyn Widget>,
            ]);

        // ── Responsive body ───────────────────────────────────────────────
        let body = crate::ui::widgets::responsive::Responsive::new(WIDE_BREAKPOINT, wide, narrow)
            .flex(1)
            .min_height(0);

        // ── Header and footer ─────────────────────────────────────────────
        let header = Text::new().content(StyledStr::new(" \u{2699} Settings ").fg(accent).bold());

        let mut footer_id = WidgetId::EMPTY;
        let footer = Text::new()
            .content(StyledStr::new(FOOTER_HINT).fg(dim))
            .id(&mut footer_id);

        let root = Pane::new().vertical().flex(1).children([
            header as Box<dyn Widget>,
            body as Box<dyn Widget>,
            footer,
        ]);

        let mut this = Box::new(Self {
            root,
            view,
            palette,
            focus_col: FocusCol::Categories,
            cat_idx: 0,
            field_idx: 0,
            wide_cat_text_id,
            wide_accordion_id,
            wide_field_grid_id,
            wide_page_layout_id,
            narrow_tab_id,
            narrow_field_grid_id,
            narrow_page_layout_id,
            field_map: Vec::new(),
            action_queue: Vec::new(),
            active_model_picker_field: None,
            model_picker_id: WidgetId::EMPTY,
            active_text_editor_field: None,
            text_editor_input_id: None,
            text_editor_save_id: None,
            text_editor_cancel_id: None,
            text_editor_multiline: false,
            footer_id,
            spinner_frame: 0,
            spinner_task: TaskHandle::EMPTY,
            go_back_signal: Rc::new(Cell::new(false)),
            save_signal: Rc::new(Cell::new(false)),
            fetch_models_signal: Rc::new(Cell::new(false)),
            atmosphere_changed_signal: Rc::new(Cell::new(None)),
            outfit_changed_signal: Rc::new(Cell::new(None)),
            models_buffer,
            config_path,
        });
        // Populate the field map (and normalize the grids) for the initial
        // category so widget events route correctly from the first frame.
        this.rebuild();
        this
    }

    pub fn set_palette(&mut self, palette: ChatPalette) {
        self.palette = palette;
        self.rebuild();
    }

    pub fn get_view(&self) -> &SettingsView {
        &self.view
    }
    pub fn get_view_mut(&mut self) -> &mut SettingsView {
        &mut self.view
    }

    /// Load the active agent's per-agent settings into the view so the Agent
    /// category shows per-agent fields. `None` clears it (per-agent fields are
    /// hidden).
    pub fn set_active_agent(&mut self, agent: Option<crate::ui::settings::ActiveAgentSettings>) {
        self.view.set_active_agent(agent);
    }

    /// Returns a signal that fires when the user wants to save and go back.
    pub fn go_back_signal(&self) -> Rc<Cell<bool>> {
        self.go_back_signal.clone()
    }

    /// Returns a signal that fires when the user wants to save (stay on screen).
    pub fn save_signal(&self) -> Rc<Cell<bool>> {
        self.save_signal.clone()
    }

    /// Returns a signal that fires when models should be fetched.
    pub fn fetch_models_signal(&self) -> Rc<Cell<bool>> {
        self.fetch_models_signal.clone()
    }

    /// Returns a signal that fires when the atmosphere field is edited.
    pub fn atmosphere_changed_signal(&self) -> Rc<Cell<Option<String>>> {
        self.atmosphere_changed_signal.clone()
    }

    /// Returns a signal that fires when the outfit field is edited.
    pub fn outfit_changed_signal(&self) -> Rc<Cell<Option<String>>> {
        self.outfit_changed_signal.clone()
    }

    /// Returns the current config (with edits applied).
    pub fn config(&self) -> &ConsciousnessConfig {
        &self.view.config
    }

    /// Saves the current config to disk and clears the save signal.
    pub fn save_config(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.view.save(path)?;
        self.save_signal.set(false);
        Ok(())
    }

    /// Saves the config and signals go-back.
    pub fn save_and_go_back(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.view.save(path)?;
        self.go_back_signal.set(false);
        Ok(())
    }

    // ── Action queue ──────────────────────────────────────────────────────

    fn enqueue(&mut self, action: SettingsAction) {
        self.action_queue.push(action);
    }

    // ── Navigation ────────────────────────────────────────────────────────

    fn selected_field(&self) -> Option<FieldLoc> {
        let fields = self.view.fields_for_category(self.view.selected_category());
        fields.get(self.field_idx).map(|(loc, _)| *loc)
    }

    fn page_depth(&self) -> usize {
        // Check both layouts — the active one will have the real depth.
        // We track this via the sub-page state instead.
        if self.active_model_picker_field.is_some() || self.active_text_editor_field.is_some() {
            2
        } else {
            1
        }
    }

    fn try_cycle_enum(&self, loc: FieldLoc, _delta: i32) -> bool {
        let fields = self.view.fields_for_category(self.view.selected_category());
        matches!(
            fields.iter().find(|(l, _)| *l == loc),
            Some((_, EditableValue::EnumVariant { .. }))
        )
    }

    fn activate_selected_field(&mut self) {
        let Some(loc) = self.selected_field() else {
            return;
        };
        let fields = self.view.fields_for_category(self.view.selected_category());
        let Some((_, value)) = fields.iter().find(|(l, _)| *l == loc) else {
            return;
        };

        match value {
            EditableValue::Bool(_b) => {
                self.enqueue(SettingsAction::ToggleBool(loc));
            }
            EditableValue::EnumVariant { index: _, variants } => {
                if variants.len() > 6 {
                    // Open model picker sub-page for large variant lists.
                    self.enqueue(SettingsAction::OpenModelPicker(loc));
                } else {
                    self.enqueue(SettingsAction::CycleEnum(loc, 1));
                }
            }
            EditableValue::Text(_) | EditableValue::Secret(_) | EditableValue::OptionalText(_) => {
                // Model fields open the model picker; all others open the text editor.
                if matches!(
                    loc,
                    FieldLoc::PvPrimaryModel
                        | FieldLoc::AgModel
                        | FieldLoc::ScModel
                        | FieldLoc::RfModel
                        | FieldLoc::ArCompressionModel
                        | FieldLoc::CpModel
                ) {
                    self.enqueue(SettingsAction::OpenModelPicker(loc));
                } else {
                    self.enqueue(SettingsAction::OpenTextEditor(loc));
                }
            }
            _ => {}
        }
    }

    // ── Action processing ─────────────────────────────────────────────────

    fn process_action(&mut self, action: SettingsAction) {
        match action {
            SettingsAction::MoveSelection(delta) => match self.focus_col {
                FocusCol::Categories => {
                    let n = Category::all().len() as i32;
                    let next = (self.cat_idx as i32 + delta).clamp(0, n - 1);
                    if next != self.cat_idx as i32 {
                        self.cat_idx = next as usize;
                        self.field_idx = 0;
                        if Category::all().get(self.cat_idx) == Some(&Category::Providers) {
                            self.view.ensure_provider_selected();
                        }
                        self.rebuild();
                    }
                }
                FocusCol::Fields => {
                    let fields = self.view.fields_for_category(self.view.selected_category());
                    if fields.is_empty() {
                        return;
                    }
                    let n = fields.len() as i32;
                    let next = (self.field_idx as i32 + delta).clamp(0, n - 1);
                    self.field_idx = next as usize;
                    self.rebuild();
                }
            },
            SettingsAction::FocusFields => {
                let has_fields = !self
                    .view
                    .fields_for_category(self.view.selected_category())
                    .is_empty();
                if has_fields {
                    self.focus_col = FocusCol::Fields;
                    self.field_idx = 0;
                    self.rebuild();
                }
            }
            SettingsAction::FocusCategories => {
                self.focus_col = FocusCol::Categories;
                self.rebuild();
            }
            SettingsAction::ToggleBool(loc) => {
                let fields = self.view.fields_for_category(self.view.selected_category());
                if let Some((_, EditableValue::Bool(b))) = fields.iter().find(|(l, _)| *l == loc) {
                    self.view.apply_field(loc, EditableValue::Bool(!b));
                    self.rebuild();
                }
            }
            SettingsAction::CycleEnum(loc, delta) => {
                let fields = self.view.fields_for_category(self.view.selected_category());
                if let Some((_, EditableValue::EnumVariant { index, variants })) =
                    fields.iter().find(|(l, _)| *l == loc)
                {
                    if variants.is_empty() {
                        return;
                    }
                    let n = variants.len() as i32;
                    let next = (*index as i32 + delta).rem_euclid(n) as usize;
                    let name = variants.get(next).cloned().unwrap_or_default();
                    self.view.apply_field(
                        loc,
                        EditableValue::EnumVariant {
                            index: next,
                            variants: variants.clone(),
                        },
                    );
                    // Fire live-apply signals for atmosphere and outfit.
                    match loc {
                        FieldLoc::PrAtmosphere => {
                            self.atmosphere_changed_signal.set(Some(name));
                        }
                        FieldLoc::PrOutfit => {
                            self.outfit_changed_signal.set(Some(name));
                        }
                        _ => {}
                    }
                    self.rebuild();
                }
            }
            SettingsAction::AdjustNumber(loc, delta) => {
                let fields = self.view.fields_for_category(self.view.selected_category());
                if let Some((_, value)) = fields.iter().find(|(l, _)| *l == loc) {
                    match value {
                        EditableValue::Uint(v) => {
                            let new = (*v as i64 + delta as i64).max(0) as u64;
                            self.view.apply_field(loc, EditableValue::Uint(new));
                        }
                        EditableValue::Int(v) => {
                            self.view
                                .apply_field(loc, EditableValue::Int(v + delta as i64));
                        }
                        EditableValue::Float(v) => {
                            let new = v + delta as f32 / 100.0;
                            self.view
                                .apply_field(loc, EditableValue::Float(new.clamp(0.0, 1.0)));
                        }
                        _ => {}
                    }
                    self.rebuild();
                }
            }
            SettingsAction::OpenModelPicker(loc) => {
                if self.view.available_models.is_empty() {
                    self.enqueue(SettingsAction::FetchModels);
                    return;
                }
                let models = self.view.available_models.clone();
                let current = self.get_current_model_value(loc);
                let picker = ModelPicker::new(&models, &current, &self.palette)
                    .id(&mut self.model_picker_id);
                self.active_model_picker_field = Some(loc);
                self.push_sub_page(picker as Box<dyn Widget>);
            }
            SettingsAction::OpenTextEditor(loc) => {
                let current = self.get_current_text_value(loc);
                let editor = TextEditor::new(loc, &current, &self.palette);
                self.text_editor_input_id = Some(editor.input_id());
                self.text_editor_save_id = Some(editor.save_button_id());
                self.text_editor_cancel_id = Some(editor.cancel_button_id());
                self.text_editor_multiline = editor.is_multiline();
                self.active_text_editor_field = Some(loc);
                self.push_sub_page(editor as Box<dyn Widget>);
            }
            SettingsAction::SelectModel(loc, model) => {
                self.view.apply_field(loc, EditableValue::Text(model));
                self.active_model_picker_field = None;
                self.pop_sub_page();
                self.rebuild();
            }
            SettingsAction::PopPage => {
                self.active_model_picker_field = None;
                self.clear_text_editor();
                self.pop_sub_page();
            }
            SettingsAction::FetchModels => {
                // Signal TuieApp to spawn the async fetch.
                self.view.models_fetching = true;
                self.fetch_models_signal.set(true);
                self.start_spinner();
            }
            SettingsAction::ModelsFetched(models) => {
                self.view.available_models = models;
                self.view.models_fetching = false;
                self.stop_spinner();
                self.rebuild();
            }
            SettingsAction::Save => {
                // Save config to disk directly.
                match self.view.save(&self.config_path) {
                    Ok(()) => {
                        self.save_signal.set(true);
                    }
                    Err(e) => {
                        self.show_footer_error(&format!("save failed: {e}"));
                        tracing::warn!("settings save failed: {e}");
                    }
                }
            }
            SettingsAction::SaveAndGoBack => {
                // Save config to disk and signal go-back.
                match self.view.save(&self.config_path) {
                    Ok(()) => {
                        self.go_back_signal.set(true);
                    }
                    Err(e) => {
                        self.show_footer_error(&format!("save failed: {e}"));
                        tracing::warn!("settings save failed: {e}");
                    }
                }
            }
            SettingsAction::Discard => {
                // Signal TuieApp to return to Welcome without saving.
                self.go_back_signal.set(true);
            }
            SettingsAction::CommitText(loc, text) => {
                // Apply the correct EditableValue variant based on the field's type.
                let fields = self.view.fields_for_category(self.view.selected_category());
                let current = fields.iter().find(|(l, _)| *l == loc).map(|(_, v)| v);
                match current {
                    Some(EditableValue::Secret(_)) => {
                        self.view.apply_field(loc, EditableValue::Secret(text));
                    }
                    Some(EditableValue::OptionalText(_)) => {
                        if text.is_empty() {
                            self.view
                                .apply_field(loc, EditableValue::OptionalText(None));
                        } else {
                            self.view
                                .apply_field(loc, EditableValue::OptionalText(Some(text)));
                        }
                    }
                    _ => {
                        self.view.apply_field(loc, EditableValue::Text(text));
                    }
                }
                self.clear_text_editor();
                self.pop_sub_page();
                self.rebuild();
            }
            SettingsAction::None => {}
        }
    }

    /// Poll the models buffer. If models arrived, update the view and rebuild.
    /// Public so `TuieApp` can drain cached models right after construction.
    pub fn poll_models_from_buffer(&mut self) {
        self.poll_models();
    }

    fn poll_models(&mut self) {
        let models = self.models_buffer.borrow_mut().take();
        if let Some(models) = models {
            self.view.available_models = models.clone();
            self.view.models_fetching = false;
            self.stop_spinner();
            if self.active_model_picker_field.is_some() {
                if let Some(picker) = self.root.get_widget_mut(self.model_picker_id) {
                    picker.set_models(models);
                }
            }
            self.rebuild();
        }
    }

    /// Start the loading spinner in the footer.
    fn start_spinner(&mut self) {
        self.spinner_task.cancel();
        self.spinner_frame = 0;
        self.advance_spinner();
    }

    /// Advance the spinner by one frame, update the footer, and reschedule.
    fn advance_spinner(&mut self) {
        let ch = SPINNER[self.spinner_frame % SPINNER.len()];
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
        let dim = theme::to_tuie_color(self.palette.agent_dim);
        if let Some(footer) = self.root.get_widget_mut(self.footer_id) {
            footer
                .set_content(StyledStr::new(&format!(" {} Fetching models\u{2026} ", ch)).fg(dim));
        }
        tuie::dirty_paint();
        let id = self.get_id();
        self.spinner_task =
            tuie::schedule(id, Duration::from_millis(80), |w: &mut SettingsScreen| {
                w.advance_spinner();
            });
    }

    /// Stop the spinner and restore the default footer hint text.
    fn stop_spinner(&mut self) {
        self.spinner_task.cancel();
        self.spinner_task = TaskHandle::EMPTY;
        self.spinner_frame = 0;
        let dim = theme::to_tuie_color(self.palette.agent_dim);
        if let Some(footer) = self.root.get_widget_mut(self.footer_id) {
            footer.set_content(StyledStr::new(FOOTER_HINT).fg(dim));
        }
    }

    /// Show an error message in the footer for a few seconds.
    fn show_footer_error(&mut self, msg: &str) {
        let color = theme::to_tuie_color(self.palette.compaction);
        if let Some(footer) = self.root.get_widget_mut(self.footer_id) {
            footer.set_content(StyledStr::new(&format!(" ✗ {msg}")).fg(color));
        }
    }

    fn get_current_model_value(&self, loc: FieldLoc) -> String {
        let fields = self.view.fields_for_category(self.view.selected_category());
        fields
            .iter()
            .find(|(l, _)| *l == loc)
            .and_then(|(_, v)| match v {
                EditableValue::Text(s) | EditableValue::OptionalText(Some(s)) => Some(s.clone()),
                EditableValue::EnumVariant { index, variants } => variants.get(*index).cloned(),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Returns the current string value for a Text/Secret/OptionalText field.
    fn get_current_text_value(&self, loc: FieldLoc) -> String {
        let fields = self.view.fields_for_category(self.view.selected_category());
        fields
            .iter()
            .find(|(l, _)| *l == loc)
            .and_then(|(_, v)| match v {
                EditableValue::Text(s)
                | EditableValue::Secret(s)
                | EditableValue::OptionalText(Some(s)) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Reads the text editor's Input content and commits it as a [`CommitText`] action.
    fn commit_text_editor(&mut self) {
        let Some(loc) = self.active_text_editor_field else {
            return;
        };
        let Some(input_id) = self.text_editor_input_id else {
            return;
        };
        let text = self
            .root
            .get_widget(input_id)
            .map(|input| input.get_string())
            .unwrap_or_default();
        // Re-apply through CommitText which handles Secret/OptionalText correctly.
        self.process_action(SettingsAction::CommitText(loc, text));
    }

    /// Clears the text editor sub-page tracking state.
    fn clear_text_editor(&mut self) {
        self.active_text_editor_field = None;
        self.text_editor_input_id = None;
        self.text_editor_save_id = None;
        self.text_editor_cancel_id = None;
        self.text_editor_multiline = false;
    }

    fn push_sub_page(&mut self, page: Box<dyn Widget>) {
        // Try wide layout first, then narrow.
        if let Some(pl) = self.root.get_widget_mut(self.wide_page_layout_id) {
            pl.push(page);
        }
        // The narrow layout's PageLayout gets a dummy push to keep depth in sync.
        // In practice, only one layout is active.
    }

    fn pop_sub_page(&mut self) {
        if let Some(pl) = self.root.get_widget_mut(self.wide_page_layout_id) {
            pl.pop();
        }
    }

    // ── Widget event handling ─────────────────────────────────────────────

    fn handle_widget_events(&mut self, event: &mut WidgetEvent) {
        // Check for ChangeEvent from the narrow layout's category tabs.
        if let Some(&ChangeEvent(idx)) = event.get_by::<ChangeEvent<usize>>(self.narrow_tab_id) {
            if idx != self.cat_idx {
                self.cat_idx = idx;
                self.field_idx = 0;
                self.rebuild();
            }
            return;
        }

        // Field widget events, routed by the value-widget id captured at build
        // time. Interactive controls already updated their own visual state and
        // carry the new absolute value, so we apply it straight to the view.
        if let Some(loc) = self
            .field_map
            .iter()
            .find(|(id, _)| *id == event.source)
            .map(|(_, l)| *l)
        {
            let fields = self.view.fields_for_category(self.view.selected_category());
            let Some(value) = fields
                .iter()
                .find(|(l, _)| *l == loc)
                .map(|(_, v)| v.clone())
            else {
                return;
            };

            // Trigger buttons (text / secret / model) open a sub-page.
            if event.of::<ClickEvent>() {
                match &value {
                    EditableValue::EnumVariant { variants, .. } if variants.len() > 6 => {
                        self.enqueue(SettingsAction::OpenModelPicker(loc));
                    }
                    EditableValue::Text(_)
                    | EditableValue::Secret(_)
                    | EditableValue::OptionalText(_) => {
                        if field_grid::is_model_loc(loc) {
                            self.enqueue(SettingsAction::OpenModelPicker(loc));
                        } else {
                            self.enqueue(SettingsAction::OpenTextEditor(loc));
                        }
                    }
                    _ => {}
                }
                return;
            }

            // Checkbox — bool toggle (no rebuild; the box already reflects it).
            if let Some(&ChangeEvent(b)) = event.get::<ChangeEvent<bool>>() {
                if let EditableValue::Bool(_) = value {
                    self.view.apply_field(loc, EditableValue::Bool(b));
                }
                return;
            }

            // Segmented / radio / dropdown — absolute enum index. Rebuild so the
            // dropdown trigger's displayed label refreshes.
            if let Some(&ChangeEvent(idx)) = event.get::<ChangeEvent<usize>>() {
                if let EditableValue::EnumVariant { variants, .. } = &value {
                    self.view.apply_field(
                        loc,
                        EditableValue::EnumVariant {
                            index: idx,
                            variants: variants.clone(),
                        },
                    );
                    self.rebuild();
                }
                return;
            }

            // Counter / slider — absolute scalar (no rebuild, keeps drag smooth).
            if let Some(&ChangeEvent(v)) = event.get::<ChangeEvent<i32>>() {
                match &value {
                    EditableValue::Uint(_) => {
                        self.view
                            .apply_field(loc, EditableValue::Uint(v.max(0) as u64));
                    }
                    EditableValue::Int(_) => {
                        self.view.apply_field(loc, EditableValue::Int(v as i64));
                    }
                    EditableValue::Float(_) => {
                        self.view.apply_field(
                            loc,
                            EditableValue::Float((v as f32 / 100.0).clamp(0.0, 1.0)),
                        );
                    }
                    _ => {}
                }
                return;
            }

            // Point picker — 2D selection.
            if let Some(&ChangeEvent(point)) = event.get::<ChangeEvent<Vec2<u16>>>() {
                if let EditableValue::Point2D { .. } = value {
                    self.view.apply_field(
                        loc,
                        EditableValue::Point2D {
                            x: point.x as u64,
                            y: point.y as u64,
                        },
                    );
                }
                return;
            }
            return;
        }

        // Check for ChangeEvent<String> from model picker list (Enter key
        // selects a model).
        if let Some(ChangeEvent(name)) = event.get::<ChangeEvent<String>>() {
            if let Some(loc) = self.active_model_picker_field {
                self.enqueue(SettingsAction::SelectModel(loc, name.clone()));
                return;
            }
        }

        // Check for ClickEvent from the refresh button in model picker
        // or Save/Cancel buttons in text editor.
        if event.of::<ClickEvent>() {
            // Text editor Save button.
            if let Some(save_id) = self.text_editor_save_id {
                if event.source == save_id.untyped() {
                    self.commit_text_editor();
                    return;
                }
            }
            // Text editor Cancel button.
            if let Some(cancel_id) = self.text_editor_cancel_id {
                if event.source == cancel_id.untyped() {
                    self.clear_text_editor();
                    self.pop_sub_page();
                    return;
                }
            }
            // Model picker refresh button (click or 'r' keyboard shortcut).
            if self.active_model_picker_field.is_some() {
                self.enqueue(SettingsAction::FetchModels);
            }
        }
    }

    // ── Rendering ─────────────────────────────────────────────────────────

    fn rebuild(&mut self) {
        // Sync view index BEFORE building content so the category list
        // renders the correct selection marker on the same frame.
        self.view.category_idx = self.cat_idx;

        // Update category list (wide layout).
        let cat_content = build_category_content(&self.view, self.focus_col, &self.palette);
        if let Some(t) = self.root.get_widget_mut(self.wide_cat_text_id) {
            t.set_content(cat_content);
        }

        // Sync the narrow layout's tab selection with the current category.
        if let Some(tab) = self.root.get_widget_mut(self.narrow_tab_id) {
            tab.set_selected(self.cat_idx);
        }

        // Update the wide layout's accordion title with the current category name.
        if let Some(acc) = self.root.get_widget_mut(self.wide_accordion_id) {
            let cat_name = Category::all()[self.cat_idx].label();
            acc.set_title(cat_name);
        }

        // Rebuild the field grid for the selected category (both layouts).
        // The grid panes (wide_field_grid_id / narrow_field_grid_id) keep their
        // identity; only their children are swapped, so the new category's
        // fields — and any updated values — actually render.
        let fields = self.view.fields_for_category(self.view.selected_category());

        // Capture each grid's (widget-id → loc) map before consuming the grid
        // into the tree; both layouts contribute (their widget ids are unique).
        self.field_map.clear();

        // Highlight the selected row when the field column has focus. Rebuilt
        // grids start unhighlighted, so without this Down/Up moves the cursor
        // invisibly and field navigation looks dead.
        let field_selected = matches!(self.focus_col, FocusCol::Fields);

        let mut wide_grid = FieldGrid::new(&fields, &self.palette);
        self.field_map.extend(wide_grid.row_map());
        if field_selected {
            wide_grid.set_selected(self.field_idx);
        }
        if let Some(pane) = self.root.get_widget_mut(self.wide_field_grid_id) {
            pane.clear();
            pane.add_child(wide_grid.widget() as Box<dyn Widget>);
        }

        let mut narrow_grid = FieldGrid::new(&fields, &self.palette);
        self.field_map.extend(narrow_grid.row_map());
        if field_selected {
            narrow_grid.set_selected(self.field_idx);
        }
        if let Some(pane) = self.root.get_widget_mut(self.narrow_field_grid_id) {
            pane.clear();
            pane.add_child(narrow_grid.widget() as Box<dyn Widget>);
        }

        tuie::dirty_paint();
    }
}

// ── Category list rendering ──────────────────────────────────────────────

use crate::ui::settings::CategoryGroup;

fn build_category_content(
    view: &SettingsView,
    focus: FocusCol,
    palette: &ChatPalette,
) -> StyledString {
    let primary = theme::to_tuie_color(palette.agent_primary);
    let dim = Color::grey256(8); // subtle grey for inactive items, like tuie-demo's track color
    let group_dim = Color::grey256(6); // group headers — dimmer than items
    let mut content = StyledString::new();

    for group in CategoryGroup::all() {
        // Group header — dim separator
        let header = format!(" {} {}\n", group.icon(), group.label());
        let h_start = content.as_ref().len();
        content.push_str(&header);
        content.style_range(h_start..content.as_ref().len(), |s| {
            s.set_fg(Some(group_dim));
            *s = s.bold();
        });

        // Categories in this group
        for cat in group.categories() {
            let i = cat.flat_index();
            let selected = i == view.category_idx;
            let marker = if selected { "›" } else { " " };
            let line = format!("   {marker} {}\n", cat.label());
            let start = content.as_ref().len();
            content.push_str(&line);
            let color = if selected {
                if focus == FocusCol::Categories {
                    primary
                } else {
                    dim
                }
            } else {
                dim
            };
            content.style_range(start..content.as_ref().len(), |s| {
                s.set_fg(Some(color));
                if selected {
                    *s = s.bold();
                }
            });
        }
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuie::emulator::Emulator;
    use tuie::input::chord::Chord;
    use tuie::input::modifiers::Modifiers;

    fn screen() -> Box<SettingsScreen> {
        let config = crate::core::config::ConsciousnessConfig::default();
        let buffer = Rc::new(RefCell::new(None));
        SettingsScreen::new(&config, std::path::PathBuf::from("souveraine.toml"), buffer)
    }

    fn down() -> RuntimeEvent {
        Chord::new(
            Trigger::Key(Key::Arrow(Direction2D::Down)),
            Modifiers::new(),
        )
        .into()
    }
    fn enter() -> RuntimeEvent {
        Chord::new(Trigger::Key(Key::Enter), Modifiers::new()).into()
    }

    /// Renders without panicking and is non-empty.
    #[test]
    fn settings_renders_without_panic() {
        let mut s = screen();
        let term = Emulator::new(&mut *s, Vec2::new(120, 40));
        assert!(
            !term.get_snapshot_text().trim().is_empty(),
            "settings rendered nothing"
        );
    }

    /// Switching categories must swap the right-hand field grid. "virtual key"
    /// is a Providers-only field label that never appears in the Agent category
    /// or the left category list, so it's a clean signal the grid rebuilt.
    #[test]
    fn category_change_swaps_field_grid() {
        let buffer = Rc::new(RefCell::new(None));
        let mut config = crate::core::config::ConsciousnessConfig::default();
        config.providers.insert(
            "testbf".into(),
            crate::core::config::ProviderConfig {
                credential_files: None,
                provider_type: "openai-compatible".into(),
                base_url: "http://localhost:8080/v1".into(),
                api_key: String::new(),
                virtual_key: "vk-test".into(),
                primary_model: String::new(),
                timeout_secs: 30,
                credential_file: None,
                cc_version: None,
                account_uuid: None,
                device_id: None,
                extra_metadata: None,
            },
        );
        let mut s =
            SettingsScreen::new(&config, std::path::PathBuf::from("souveraine.toml"), buffer);
        let mut term = Emulator::new(&mut *s, Vec2::new(60, 40));
        assert!(
            !term.get_snapshot_text().contains("virtual key"),
            "Agent category should not show Providers virtual key field"
        );
        // Agent(0) -> Inference(1) -> Providers(2).
        term.update(&mut *s, &[down(), down()]);
        assert_eq!(s.cat_idx, 2, "expected to land on Providers");
        assert!(
            term.get_snapshot_text().contains("virtual key"),
            "Providers category should render its fields after navigating"
        );
    }

    /// A 4-variant enum (Subconscious `n1_trigger`) should render as a
    /// [`RadioGroup`], whose unselected options draw the "( )" marker. This
    /// pins the field-grid control mapping to the richer widgets.
    ///
    /// [`RadioGroup`]: crate::ui::widgets::radio_group::RadioGroup
    #[test]
    fn enum_renders_as_radio_group() {
        let mut s = screen();
        let mut term = Emulator::new(&mut *s, Vec2::new(60, 40));
        // Agent(0) -> Inference(1) -> Bifrost(2) -> Subconscious(3).
        term.update(&mut *s, &[down(), down(), down()]);
        assert_eq!(s.cat_idx, 3, "expected to land on Subconscious");
        assert!(
            term.get_snapshot_text().contains("( )"),
            "n1_trigger (4 variants) should render as a radio group"
        );
    }

    /// Down arrow on the category column advances the selected category and
    /// resets the field cursor.
    #[test]
    fn down_advances_category_selection() {
        let mut s = screen();
        let mut term = Emulator::new(&mut *s, Vec2::new(120, 40));
        assert_eq!(s.cat_idx, 0);
        term.update(&mut *s, &[down()]);
        assert_eq!(s.cat_idx, 1, "Down should move to the next category");
        assert_eq!(s.field_idx, 0, "changing category resets the field cursor");
    }
}
