//! Numeric stepper widget.

use axis2d::Axis2D;
use chord_macro::chord;
use std::any::Any;
use tuie::{field, prelude::*};

fn normalize_numeric(s: &str) -> String {
    if s.is_empty() {
        return "0".to_string();
    }
    let neg = s.starts_with('-');
    let digits = s.trim_start_matches('-').trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    if neg && digits != "0" {
        format!("-{}", digits)
    } else {
        digits.to_string()
    }
}

struct NumericBindings {
    inner: Box<dyn InputBindings<Text>>,
    min: i32,
    max: i32,
}

impl NumericBindings {
    #[allow(clippy::new_ret_no_self)] // trait-object factory, not a Self ctor
    fn new() -> Box<dyn InputBindings<Text>> {
        Box::new(Self {
            inner: DefaultBindings::new(),
            min: i32::MIN,
            max: i32::MAX,
        })
    }
}

impl InputBindings<Text> for NumericBindings {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn configure_state(&self, state: &mut EditorState<Text>) {
        self.inner.configure_state(state);
        state.inclusive_selection = false;
    }
    fn get_cursor_shape(&self, _state: &EditorState<Text>) -> CursorShape {
        CursorShape::Beam
    }
    fn on_focus(&mut self, state: &mut EditorState<Text>, text: &Text) {
        self.inner.on_focus(state, text);
    }
    fn on_blur(&mut self, state: &mut EditorState<Text>, text: &Text) {
        self.inner.on_blur(state, text);
    }

    fn on_input(
        &mut self,
        state: &mut EditorState<Text>,
        text: &mut Text,
        queue: &mut InputQueue,
    ) -> InputResult {
        let Some(event) = queue.peek() else {
            return InputResult::Rejected;
        };
        if let Trigger::Key(Key::Tab) = event.chord.trigger {
            return InputResult::Rejected;
        }
        if let Trigger::Key(Key::Char(c)) = event.chord.trigger {
            if event.chord.modifiers.modifiers == 0 {
                if c.is_ascii_digit() {
                    let (s, e) = state.get_selection();
                    let (start, end) = (s.get_index(), e.get_index());
                    let current = text.get_string();
                    let mut simulated = current[..start].to_string();
                    simulated.push(c);
                    simulated.push_str(&current[end..]);
                    if let Ok(v) = simulated.parse::<i32>() {
                        if v < self.min || v > self.max {
                            let clamped = v.clamp(self.min, self.max).to_string();
                            state.replace_range(text, 0..text.len(), &clamped);
                            queue.next();
                            return InputResult::Handled;
                        }
                    }
                } else if c == '-' {
                    let (s, e) = state.get_selection();
                    let (start, end) = (s.get_index(), e.get_index());
                    let has_selection = start != end;
                    let blocked = start != 0
                        || (!has_selection && text.get_string().starts_with('-'))
                        || self.min >= 0;
                    if blocked {
                        queue.next();
                        return InputResult::Handled;
                    }
                } else {
                    queue.next();
                    return InputResult::Handled;
                }
            }
        }
        self.inner.on_input(state, text, queue)
    }
}

/// Numeric stepper with `min`/`max` clamping.
pub struct Counter {
    root: Box<Pane>,
    input_id: WidgetId<Input>,
    minus_id: WidgetId<Text>,
    plus_id: WidgetId<Text>,
    label_id: Option<WidgetId<Text>>,
    value: i32,
    min: i32,
    max: i32,
    color: Color,
    selected: bool,
    pressed: Option<i32>,
}

impl Counter {
    fn sync_input(&mut self) {
        self.value = self.value.clamp(self.min, self.max);
        if let Some(input) = self.root.get_widget_mut(self.input_id) {
            input.set_content(self.value.to_string());
        }
    }

    fn sync_min(&mut self) {
        if let Some(input) = self.root.get_widget_mut(self.input_id) {
            if let Some((b, _)) = input.get_bindings_as_mut::<NumericBindings>() {
                b.min = self.min;
            }
        }
    }

    fn sync_max(&mut self) {
        if let Some(input) = self.root.get_widget_mut(self.input_id) {
            if let Some((b, _)) = input.get_bindings_as_mut::<NumericBindings>() {
                b.max = self.max;
            }
        }
    }

    fn sync_color(&mut self) {
        if let Some(input) = self.root.get_widget_mut(self.input_id) {
            input.set_selected_style(Some(Style::new().fg(self.color).reverse()));
        }
        self.apply_button_styles();
    }

    field!(value: i32; sync_input);
    field!(min: i32; sync_min);
    field!(max: i32; sync_max);
    field!(color: Color; sync_color);

    fn select_all_input(&mut self) {
        if let Some(input) = self.root.get_widget_mut(self.input_id) {
            input.select_all();
        }
    }

    fn adjust(&mut self, delta: i32) {
        let new_value = self.value.saturating_add(delta).clamp(self.min, self.max);
        if new_value != self.value {
            self.value = new_value;
            self.sync_input();
            tuie::emit(self.get_id(), ChangeEvent(new_value));
        }
    }

    fn commit_input(&mut self) {
        let text = match self.root.get_widget(self.input_id) {
            Some(input) => input.get_string(),
            None => return,
        };
        if let Ok(v) = text.parse::<i32>() {
            let clamped = v.clamp(self.min, self.max);
            if clamped != self.value {
                self.value = clamped;
                tuie::emit(self.get_id(), ChangeEvent(clamped));
            }
        }
        self.sync_input();
    }

    fn hit_child(&self, id: WidgetId<Text>, mouse_pos: Vec2<i32>) -> bool {
        let origin = self.root.get_pos();
        let Some(child) = self.root.get_widget(id) else {
            return false;
        };
        let pos = child.get_pos() - origin;
        let size = child.get_rect_size();
        Axis2D::all(|a| mouse_pos[a] >= pos[a] && mouse_pos[a] < pos[a] + size[a] as i32)
    }

    fn hit_test_buttons(&self, mouse_pos: Vec2<i32>) -> Option<i32> {
        if self.hit_child(self.minus_id, mouse_pos) {
            Some(-1)
        } else if self.hit_child(self.plus_id, mouse_pos) {
            Some(1)
        } else {
            None
        }
    }

    fn button_style(&self, which: i32) -> Style {
        let pressed = self.pressed == Some(which);
        if self.selected && !pressed {
            Style::new().fg(self.color).bold()
        } else if self.selected {
            Style::new().fg(self.color)
        } else {
            Style::new()
        }
    }

    fn apply_button_styles(&mut self) {
        let minus_style = self.button_style(-1);
        let plus_style = self.button_style(1);
        let label_style = if self.selected {
            Style::new().fg(self.color).bold()
        } else {
            Style::new()
        };
        if let Some(t) = self.root.get_widget_mut(self.minus_id) {
            t.set_style(minus_style);
        }
        if let Some(t) = self.root.get_widget_mut(self.plus_id) {
            t.set_style(plus_style);
        }
        if let Some(id) = self.label_id {
            if let Some(t) = self.root.get_widget_mut(id) {
                t.set_style(label_style);
            }
        }
    }

    fn set_pressed(&mut self, pressed: Option<i32>) {
        self.pressed = pressed;
        self.apply_button_styles();
    }
}

impl DelegateWidget for Counter {
    tuie::delegate_widget!(root);

    fn after_on_state_change(&mut self, state: WidgetState) {
        self.selected = matches!(
            state,
            WidgetState::Focused | WidgetState::FocusedHover | WidgetState::Active
        );
        self.apply_button_styles();
        if self.selected {
            self.select_all_input();
        }
    }

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        let Some(event) = queue.peek() else {
            return InputResult::Rejected;
        };

        match &event.chord {
            chord!(LeftClick) => {
                tuie::focus_widget(self.input_id);
                let mouse_pos = event.pos;
                if let Some(which) = self.hit_test_buttons(mouse_pos.map(|v| v as i32)) {
                    queue.next();
                    self.set_pressed(Some(which));
                    return InputResult::Handled;
                }
                InputResult::Rejected
            }
            chord!(LeftRelease) => {
                let mouse_pos = event.pos;
                queue.next();
                if let Some(which) = self.pressed {
                    if self.hit_test_buttons(mouse_pos.map(|v| v as i32)) == Some(which) {
                        self.adjust(which);
                        self.select_all_input();
                    }
                }
                self.set_pressed(None);
                InputResult::Handled
            }
            chord!(Enter) => {
                queue.next();
                self.commit_input();
                InputResult::Handled
            }
            chord!(Up) => {
                queue.next();
                self.adjust(1);
                self.select_all_input();
                InputResult::Handled
            }
            chord!(Down) => {
                queue.next();
                self.adjust(-1);
                self.select_all_input();
                InputResult::Handled
            }
            chord!(Tab) => {
                queue.next();
                tuie::focus_next_tab_order(Sign::Positive);
                InputResult::Handled
            }
            chord!(Shift + Tab) => {
                queue.next();
                tuie::focus_next_tab_order(Sign::Negative);
                InputResult::Handled
            }
            _ => InputResult::Rejected,
        }
    }

    fn after_on_event(&mut self, event: &mut WidgetEvent) {
        let Some(ChangeEvent(raw)) = event.get_by::<ChangeEvent<String>>(self.input_id) else {
            return;
        };
        let normalized = normalize_numeric(raw);
        if normalized != *raw {
            let display = if let Ok(v) = normalized.parse::<i32>() {
                v.clamp(self.min, self.max).to_string()
            } else {
                normalized
            };
            if let Some(input) = self.root.get_widget_mut(self.input_id) {
                input.set_content(display);
                if raw.is_empty() {
                    input.select_all();
                }
            }
            return;
        }
        if let Ok(v) = normalized.parse::<i32>() {
            let clamped = v.clamp(self.min, self.max);
            if clamped != v {
                if let Some(input) = self.root.get_widget_mut(self.input_id) {
                    input.set_content(clamped.to_string());
                }
            }
            if clamped != self.value {
                self.value = clamped;
                tuie::emit(self.get_id(), ChangeEvent(self.value));
            }
        }
    }
}

impl Counter {
    /// Creates a [`Counter`] with an optional trailing `label`.
    pub fn new(label: &str) -> Box<Self> {
        let mut minus_id = WidgetId::EMPTY;
        let mut input_id = WidgetId::EMPTY;
        let mut plus_id = WidgetId::EMPTY;
        let mut label_id = None;

        let mut root = Pane::new().horizontal().children([
            Text::new().content("< ").id(&mut minus_id) as Box<dyn Widget>,
            Input::new()
                .bindings(NumericBindings::new)
                .content("0")
                .overflow(TextOverflow::VISIBLE)
                .align(Align::Center)
                .horizontal_margin(0)
                .id(&mut input_id),
            Text::new().content(" >").id(&mut plus_id),
        ]);

        if !label.is_empty() {
            let mut id = WidgetId::EMPTY;
            root.add_child(Text::new().content(format!(" {}", label)).id(&mut id));
            label_id = Some(id);
        }

        Box::new(Self {
            root,
            input_id,
            minus_id,
            plus_id,
            label_id,
            value: 0,
            min: i32::MIN,
            max: i32::MAX,
            color: Color::BLUE,
            selected: false,
            pressed: None,
        })
    }
}
