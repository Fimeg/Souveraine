//! Settings field grid — builds real tuie widgets for each settings field.
//!
//! For each `(FieldLoc, EditableValue)` from `fields_for_category()`, creates
//! the control best suited to the value:
//! - `Bool`                       → Checkbox
//! - `EnumVariant`, ≤3 variants   → SegmentedControl (horizontal chips)
//! - `EnumVariant`, 4 variants    → RadioGroup (vertical)
//! - `EnumVariant`, 5–6 variants  → Dropdown (popup menu)
//! - `EnumVariant`, >6 variants   → FlatButton → ModelPicker sub-page (model lists)
//! - `Uint` / `Int`              → Counter
//! - `Float`                     → Slider (0.00–1.00)
//! - `Text` / `OptionalText`     → FlatButton → text editor / model picker
//! - `Secret`                    → FlatButton (masked) → text editor
//!
//! The value widget's id is recorded per row ([`FieldGrid::row_map`]) so the
//! screen can map a widget event back to the `FieldLoc` that produced it.

use tuie::prelude::*;

use crate::ui::chat::ChatPalette;
use crate::ui::settings::{EditableValue, FieldLoc};
use crate::ui::theme;
use crate::ui::widgets::checkbox::Checkbox;
use crate::ui::widgets::counter::Counter;
use crate::ui::widgets::dropdown::Dropdown;
use crate::ui::widgets::flat_button::FlatButton;
use crate::ui::widgets::point_picker::PointPicker;
use crate::ui::widgets::radio_group::RadioGroup;
use crate::ui::widgets::segmented_control::SegmentedControl;
use crate::ui::widgets::slider::Slider;

/// True for fields whose string value is chosen from the model catalog (a long
/// list best handled by the dedicated `ModelPicker` sub-page rather than an
/// inline control).
pub fn is_model_loc(loc: FieldLoc) -> bool {
    matches!(
        loc,
        FieldLoc::PvPrimaryModel
            | FieldLoc::AgModel
            | FieldLoc::ScModel
            | FieldLoc::RfModel
            | FieldLoc::ArCompressionModel
            | FieldLoc::CpModel
    )
}

/// One row in the settings field grid: a label + an interactive widget.
struct FieldRow {
    loc: FieldLoc,
    widget_id: WidgetId, // untyped ID of the value widget
    label_id: WidgetId<Text>,
}

/// Builds and manages the field grid for a single settings category.
pub struct FieldGrid {
    root: Box<Pane>,
    rows: Vec<FieldRow>,
    accent: Color,
    dim: Color,
}

impl FieldGrid {
    /// Builds a new field grid for the given fields.
    pub fn new(fields: &[(FieldLoc, EditableValue)], palette: &ChatPalette) -> Box<Self> {
        let accent = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);

        let mut rows = Vec::new();
        let mut grid = Grid::new();
        grid.set_cols(vec![
            Track::auto().min(16), // label column, auto-sized with min 16
            Track::flex(1),        // control column, fills remaining space
        ]);
        grid.set_col_gap(1);

        for (i, (loc, value)) in fields.iter().enumerate() {
            let row_idx = i as u16;

            let mut label_id = WidgetId::EMPTY;
            let label = Text::new()
                .content(format!(" {}", loc.label()))
                .id(&mut label_id);
            grid.add_cell(Cell::new(row_idx, 0, label as Box<dyn Widget>));

            let widget_id: WidgetId;
            let value_widget: Box<dyn Widget> = match value {
                EditableValue::Bool(b) => {
                    let mut id = WidgetId::EMPTY;
                    let mut c = Checkbox::new(Text::new().content(""));
                    c.set_checked(*b);
                    let w = c.id(&mut id);
                    widget_id = id.untyped();
                    w as Box<dyn Widget>
                }
                EditableValue::EnumVariant { index, variants } => {
                    let labels: Vec<&str> = variants.iter().map(|s| s.as_str()).collect();
                    match labels.len() {
                        // Long lists (model catalogs): a colored trigger that
                        // opens the dedicated, scrollable picker sub-page.
                        n if n > 6 => {
                            let current = labels.get(*index).copied().unwrap_or("?");
                            let mut id = WidgetId::EMPTY;
                            let w = FlatButton::new()
                                .child(Text::new().content(format!(" {current} \u{25be}")))
                                .id(&mut id);
                            widget_id = id.untyped();
                            w as Box<dyn Widget>
                        }
                        5 | 6 => {
                            let mut id = WidgetId::EMPTY;
                            let w = Dropdown::new(&labels, *index).id(&mut id);
                            widget_id = id.untyped();
                            w as Box<dyn Widget>
                        }
                        4 => {
                            let mut id = WidgetId::EMPTY;
                            let w = RadioGroup::new(&labels).selected(*index).id(&mut id);
                            widget_id = id.untyped();
                            w as Box<dyn Widget>
                        }
                        _ => {
                            let mut id = WidgetId::EMPTY;
                            let w = SegmentedControl::new(&labels).selected(*index).id(&mut id);
                            widget_id = id.untyped();
                            w as Box<dyn Widget>
                        }
                    }
                }
                EditableValue::Uint(v) => {
                    let mut id = WidgetId::EMPTY;
                    let w = Counter::new("")
                        .value(*v as i32)
                        .min(0)
                        .max(i32::MAX)
                        .id(&mut id);
                    widget_id = id.untyped();
                    w as Box<dyn Widget>
                }
                EditableValue::Int(v) => {
                    let mut id = WidgetId::EMPTY;
                    let w = Counter::new("").value(*v as i32).id(&mut id);
                    widget_id = id.untyped();
                    w as Box<dyn Widget>
                }
                EditableValue::Float(v) => {
                    let mut id = WidgetId::EMPTY;
                    let scaled = (*v * 100.0).round() as i32;
                    let w = Slider::new(0, 100, accent).value(scaled).id(&mut id);
                    widget_id = id.untyped();
                    w as Box<dyn Widget>
                }
                EditableValue::Point2D { x, y } => {
                    let mut id = WidgetId::EMPTY;
                    let w = PointPicker::new()
                        .point(Vec2::new(*x as u16, *y as u16))
                        .id(&mut id);
                    widget_id = id.untyped();
                    w as Box<dyn Widget>
                }
                EditableValue::Text(s) | EditableValue::OptionalText(Some(s)) => {
                    let display = if s.len() > 20 {
                        format!(" {}\u{2026} ", &s[..17])
                    } else {
                        format!(" {s} ")
                    };
                    let mut id = WidgetId::EMPTY;
                    let w = FlatButton::new()
                        .child(Text::new().content(display))
                        .id(&mut id);
                    widget_id = id.untyped();
                    w as Box<dyn Widget>
                }
                EditableValue::Secret(_) => {
                    let mut id = WidgetId::EMPTY;
                    let w = FlatButton::new()
                        .child(Text::new().content(
                            " \u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022} ",
                        ))
                        .id(&mut id);
                    widget_id = id.untyped();
                    w as Box<dyn Widget>
                }
                EditableValue::OptionalText(None) => {
                    let mut id = WidgetId::EMPTY;
                    let w = FlatButton::new()
                        .child(Text::new().content(StyledStr::new(" (none) ").fg(dim)))
                        .id(&mut id);
                    widget_id = id.untyped();
                    w as Box<dyn Widget>
                }
            };

            grid.add_cell(Cell::new(row_idx, 1, value_widget));
            rows.push(FieldRow {
                loc: *loc,
                widget_id,
                label_id,
            });
        }

        // Set row tracks after all cells are added.
        if fields.is_empty() {
            grid.set_rows(vec![Track::auto()]);
            grid.add_cell(Cell::new(
                0,
                0,
                Text::new().content(StyledStr::new("  (no fields)").fg(dim)) as Box<dyn Widget>,
            ));
        } else {
            grid.set_rows(vec![Track::auto(); fields.len()]);
        }

        let root = Pane::new().vertical().children([grid as Box<dyn Widget>]);

        Box::new(Self {
            root,
            rows,
            accent,
            dim,
        })
    }

    /// Returns `(value-widget-id, field-loc)` pairs so the screen can route a
    /// widget event back to the field that emitted it.
    pub fn row_map(&self) -> Vec<(WidgetId, FieldLoc)> {
        self.rows.iter().map(|r| (r.widget_id, r.loc)).collect()
    }

    /// Highlights the selected row's label and dims the rest.
    pub fn set_selected(&mut self, selected: usize) {
        for (i, row) in self.rows.iter().enumerate() {
            if let Some(label) = self.root.get_widget_mut(row.label_id) {
                let color = if i == selected { self.accent } else { self.dim };
                let style = if i == selected {
                    Style::new().fg(color).bold()
                } else {
                    Style::new().fg(color)
                };
                label.set_style(style);
            }
        }
    }

    /// Returns the root widget.
    #[allow(clippy::boxed_local)] // consumes the boxed builder, returns its root
    pub fn widget(self: Box<Self>) -> Box<Pane> {
        self.root
    }
}

impl DelegateWidget for FieldGrid {
    tuie::delegate_widget!(root);
}
