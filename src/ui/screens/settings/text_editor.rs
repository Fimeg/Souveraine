#![allow(dead_code)] // WIP scaffolding not yet wired
//! Text editor sub-page for the settings screen.
//!
//! Pushed onto the PageLayout when the user activates a Text, Secret, or
//! OptionalText field. Shows the field label, an [`Input`] widget for editing,
//! and Save / Cancel buttons.
//!
//! Keyboard handling is done by the parent [`SettingsScreen`] — when the text
//! editor sub-page is active, the parent delegates most keystrokes to the
//! Input widget and intercepts only Esc (cancel), Enter on single-line
//! fields (save), and Ctrl+S (save).

use tuie::prelude::*;

use crate::ui::chat::ChatPalette;
use crate::ui::settings::FieldLoc;
use crate::ui::theme;
use crate::ui::widgets::button::Button;

/// Text editor sub-page widget.
pub struct TextEditor {
    root: Box<Pane>,
    input_id: WidgetId<Input>,
    save_button_id: WidgetId<Button>,
    cancel_button_id: WidgetId<Button>,
    loc: FieldLoc,
    multiline: bool,
}

impl TextEditor {
    /// Creates a new text editor sub-page for the given field.
    ///
    /// `current_value` is pre-filled into the input. For `OptionalText(None)`
    /// pass an empty string.
    pub fn new(loc: FieldLoc, current_value: &str, palette: &ChatPalette) -> Box<Self> {
        let accent = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);

        let mut input_id = WidgetId::EMPTY;
        let mut save_button_id = WidgetId::EMPTY;
        let mut cancel_button_id = WidgetId::EMPTY;

        let header = Text::new().content(
            StyledStr::new(&format!(" Edit: {} ", loc.label()))
                .fg(accent)
                .bold(),
        );

        // System prompt fields get multiline editing; everything else is single-line.
        let is_multiline = matches!(loc, FieldLoc::AgSystemPrompt | FieldLoc::ScSystemPrompt);

        let mut input = Input::new()
            .content(current_value)
            .flex(1)
            .placeholder(Text::new().content(StyledStr::new("(empty)").fg(dim)));
        if is_multiline {
            input = input.multiline().word_wrap();
        }
        let input_widget = input.id(&mut input_id);

        let input_pane = Pane::new()
            .vertical()
            .flex(1)
            .min_height(1)
            .bordered()
            .border_style(Style::new().fg(dim).dim())
            .children([input_widget as Box<dyn Widget>]);

        let save_button = Button::new()
            .children([Text::new().content(" Save ")])
            .id(&mut save_button_id);

        let cancel_button = Button::new()
            .children([Text::new().content(" Cancel ")])
            .id(&mut cancel_button_id);

        let footer = Pane::new().horizontal().children([
            save_button as Box<dyn Widget>,
            Text::new().content("  ") as Box<dyn Widget>,
            cancel_button as Box<dyn Widget>,
            Pane::new().flex(1),
        ]);

        let root = Pane::new().vertical().flex(1).gap(1).children([
            header as Box<dyn Widget>,
            input_pane as Box<dyn Widget>,
            footer as Box<dyn Widget>,
        ]);

        Box::new(Self {
            root,
            input_id,
            save_button_id,
            cancel_button_id,
            loc,
            multiline: is_multiline,
        })
    }

    /// Returns the WidgetId of the [`Input`] widget so the parent can read its
    /// content when committing.
    pub fn input_id(&self) -> WidgetId<Input> {
        self.input_id
    }

    /// Returns the WidgetId of the Save button (for click detection by parent).
    pub fn save_button_id(&self) -> WidgetId<Button> {
        self.save_button_id
    }

    /// Returns the WidgetId of the Cancel button (for click detection by parent).
    pub fn cancel_button_id(&self) -> WidgetId<Button> {
        self.cancel_button_id
    }

    /// Returns the [`FieldLoc`] being edited.
    pub fn loc(&self) -> FieldLoc {
        self.loc
    }

    /// Returns whether the input is multiline.
    pub fn is_multiline(&self) -> bool {
        self.multiline
    }
}

impl DelegateWidget for TextEditor {
    tuie::delegate_widget!(root);

    fn override_is_focusable(&self) -> bool {
        true
    }
}
