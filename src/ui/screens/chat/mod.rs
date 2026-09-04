//! Chat screen — the primary conversation interface.
//!
//! Holds a `ChatState` (the existing state machine from `src/ui/chat`), polls
//! it for events via `tuie::schedule()`, and renders messages through the
//! MessageList widget. Input submission triggers `ChatState::submit()`.
//!
//! Layout:
//! ```text
//! root (Pane, vertical)
//!   header (Text) — "✦ Souveraine · agent_name"
//!   itinerary_strip (ItineraryStrip) — hidden when empty
//!   body (Pane, horizontal, flex=1)
//!     chat_column (Pane, vertical, flex=1/3)
//!       messages_pane (Pane, flex=1, scrollable, bordered)
//!       btw_pane (BtwPane) — hidden when idle
//!     right_column (Pane, vertical, flex=0/1) — togglable
//!       cockpit_pane (Pane, flex=1)
//!       sidebar_pane (Pane, flex=0) — ChatSidebar
//!   overlay (ChatOverlay) — hidden when Overlay::None
//!   phase_bar (PhaseBar) — between overlay and input; expands with subconscious stream lines
//!   input_row (Pane, horizontal, bordered)
//!   footer (Footer)
//!     prompt "›"  |  input (Input, flex=1)  |  SendButton (clickable ▸)
//! ```

use std::sync::Arc;

use tokio::sync::RwLock;
use tuie::prelude::*;

use crate::core::config::ConsciousnessConfig;
use crate::ui::chat::{BtwState, ChatPalette, ChatState, Overlay, TurnPhase, SPINNER};
use crate::ui::theme;
use crate::ui::widgets::btw_pane::BtwPane;
use crate::ui::widgets::chat_input;
use crate::ui::widgets::chat_overlay::ChatOverlay;
use crate::ui::widgets::chat_sidebar::ChatSidebar;
use crate::ui::widgets::cockpit::{ActivePane, Cockpit};
use crate::ui::widgets::footer::Footer;
use crate::ui::widgets::itinerary_strip::ItineraryStrip;
use crate::ui::widgets::message_list::{MessageList, MsgKind};
use crate::ui::widgets::phase_bar::{PhaseBar, PhaseKind};
use crate::ui::widgets::send_button::SendButton;

mod convert;
use convert::chat_message_to_msgkind;

pub struct ChatScreen {
    root: Box<Pane>,

    // Widget IDs for leaf / container widgets.
    message_list_id: WidgetId<MessageList>,
    /// Stable id of the chat `Input`. Stable because the input has no tuie
    /// placeholder — see `chat_input::new_chat_input`.
    input_id: WidgetId<Input>,
    header_id: WidgetId<Text>,
    prompt_id: WidgetId<Text>,
    cockpit_id: WidgetId<Cockpit>,
    footer_id: WidgetId<Footer>,
    sidebar_id: WidgetId<ChatSidebar>,
    overlay_id: WidgetId<ChatOverlay>,
    btw_pane_id: WidgetId<BtwPane>,
    itinerary_id: WidgetId<ItineraryStrip>,

    // Buffer for messages typed before connection completes.
    pending_messages: Vec<String>,
    phase_bar_id: WidgetId<PhaseBar>,
    send_button_id: WidgetId<SendButton>,

    // Container IDs for layout manipulation.
    sidebar_pane_id: WidgetId<Pane>,
    right_column_id: WidgetId<Pane>,

    // Visual state
    palette: ChatPalette,
    _agent_name: String,

    // Chat engine
    chat: Option<ChatState>,
    _config: Arc<RwLock<ConsciousnessConfig>>,
    connecting: bool,

    // Toggles
    cockpit_visible: bool,
    sidebar_visible: bool,

    // Slash input tracking
    slash_input: String,
    slash_selected: usize,

    // Live pressure for footer
    last_pressure: f32,
    last_model: String,

    // Last rendered message count — drives auto-scroll when new messages land.
    last_msg_count: usize,
}

impl DelegateWidget for ChatScreen {
    tuie::delegate_widget!(root);

    fn override_is_focusable(&self) -> bool {
        true
    }

    fn after_on_event(&mut self, event: &mut WidgetEvent) {
        // SendButton click — submit current input text.
        if event.of_by::<ClickEvent>(self.send_button_id) {
            let text = self.get_input_text();
            if !text.trim().is_empty() {
                self.submit_text(&text);
            }
            self.slash_input.clear();
            self.dismiss_overlay();
        }
    }

    fn after_layout_flow(&mut self, allocated: Vec2<u16>, _result: Vec2<u16>) {
        let w = allocated.x;
        if let Some(ml) = self.root.get_widget_mut(self.message_list_id) {
            ml.set_container_width(w);
        }
    }

    fn override_on_input(&mut self, queue: &mut InputQueue) -> InputResult {
        use tuie::input::key::Key;
        use tuie::input::modifiers::Modifier;
        use tuie::input::trigger::Trigger;

        if let Some(event) = queue.peek() {
            if let Trigger::Key(key) = &event.chord.trigger {
                // ── Global / overlay dismissal ────────────────────────────
                let has_overlay = self
                    .chat
                    .as_ref()
                    .map(|c| !matches!(c.overlay, Overlay::None))
                    .unwrap_or(false);

                if has_overlay {
                    match key {
                        Key::Esc => {
                            queue.next();
                            self.dismiss_overlay();
                            return InputResult::Handled;
                        }
                        Key::Arrow(Direction2D::Up) => {
                            queue.next();
                            self.overlay_move(-1);
                            return InputResult::Handled;
                        }
                        Key::Arrow(Direction2D::Down) => {
                            queue.next();
                            self.overlay_move(1);
                            return InputResult::Handled;
                        }
                        Key::Enter => {
                            // The resume-offer ConversationPicker auto-pops on
                            // connect. If the user has typed a message, a bare
                            // Enter means "send it", not "resume that other
                            // conversation" — fall through to the submit arm
                            // below (which also dismisses the overlay). Only
                            // confirm the picker when the input is empty.
                            let is_resume_picker = self
                                .chat
                                .as_ref()
                                .map(|c| matches!(c.overlay, Overlay::ConversationPicker { .. }))
                                .unwrap_or(false);
                            let has_text = !self.get_input_text().trim().is_empty();
                            if !(is_resume_picker && has_text) {
                                queue.next();
                                self.overlay_confirm();
                                return InputResult::Handled;
                            }
                        }
                        _ => {}
                    }
                }

                // ── Normal input ──────────────────────────────────────────
                match key {
                    // Alt+Enter / Ctrl+Enter insert a literal newline. The
                    // editor only newlines on a bare Enter, which we consume for
                    // submit below, so we insert the newline ourselves here.
                    Key::Enter
                        if event.chord.modifiers.has(Modifier::Alt)
                            || event.chord.modifiers.has(Modifier::Ctrl) =>
                    {
                        queue.next();
                        chat_input::insert_newline(&mut *self.root, self.input_id);
                        tuie::dirty_paint();
                        return InputResult::Handled;
                    }
                    // Bare Enter submits.
                    Key::Enter
                        if !event.chord.modifiers.has(Modifier::Alt)
                            && !event.chord.modifiers.has(Modifier::Ctrl) =>
                    {
                        queue.next();
                        // If btw complete is showing, don't submit — dismiss it.
                        if self
                            .chat
                            .as_ref()
                            .map(|c| {
                                matches!(
                                    c.btw_state,
                                    BtwState::Complete { .. } | BtwState::Error { .. }
                                )
                            })
                            .unwrap_or(false)
                        {
                            if let Some(chat) = self.chat.as_mut() {
                                chat.btw_state = BtwState::Idle;
                            }
                            self.update_btw();
                            return InputResult::Handled;
                        }
                        let text = self.get_input_text();
                        if !text.trim().is_empty() {
                            self.submit_text(&text);
                        }
                        self.slash_input.clear();
                        self.dismiss_overlay();
                        return InputResult::Handled;
                    }
                    Key::Char('l') if event.chord.modifiers.has(Modifier::Ctrl) => {
                        queue.next();
                        self.clear_input();
                        self.slash_input.clear();
                        self.dismiss_overlay();
                        return InputResult::Handled;
                    }
                    Key::Tab => {
                        queue.next();
                        self.toggle_cockpit();
                        return InputResult::Handled;
                    }
                    Key::Char('b') if event.chord.modifiers.has(Modifier::Ctrl) => {
                        queue.next();
                        self.toggle_sidebar();
                        return InputResult::Handled;
                    }
                    Key::Char('t') => {
                        // Toggle tool cards on empty input.
                        let text = self.get_input_text();
                        if text.trim().is_empty() {
                            queue.next();
                            if let Some(chat) = self.chat.as_mut() {
                                chat.tool_cards_expanded = !chat.tool_cards_expanded;
                            }
                            return InputResult::Handled;
                        }
                    }
                    Key::Esc => {
                        queue.next();
                        // If btw is streaming/forking, cancel it.
                        if let Some(chat) = self.chat.as_mut() {
                            if matches!(
                                chat.btw_state,
                                BtwState::Forking { .. } | BtwState::Streaming { .. }
                            ) {
                                chat.btw_state = BtwState::Idle;
                                self.update_btw();
                                return InputResult::Handled;
                            }
                            // Show esc overlay if busy.
                            if chat.busy {
                                chat.show_esc_overlay = true;
                                return InputResult::Handled;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Forward remaining input to the root pane (contains the Input widget)
        // FIRST, so the Input reflects the just-typed key before we recompute
        // completions. Computing the overlay beforehand read stale text — it
        // lagged one keystroke and never fired on the first `/`.
        let result = self.get_delegate_mut().on_input(queue);

        // Now refresh slash completions from the updated input text.
        if let Some(chat) = self.chat.as_ref() {
            if !matches!(chat.overlay, Overlay::ConversationPicker { .. }) {
                self.update_slash_overlay();
            }
        }

        result
    }
}

impl ChatScreen {
    pub fn new(
        config: Arc<RwLock<ConsciousnessConfig>>,
        palette: ChatPalette,
        agent_name: String,
    ) -> Box<Self> {
        let primary = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);

        // ── Header ──────────────────────────────────────────────────────────
        let mut header = Text::new();
        header.set_min_height(Some(1));
        header.set_content(
            StyledStr::new(&format!(" ✦ Souveraine · {agent_name}"))
                .bold()
                .fg(primary),
        );
        let header_id = header.get_id();

        // ── Itinerary strip ─────────────────────────────────────────────────
        let itinerary = ItineraryStrip::new(&palette);
        let itinerary_id = itinerary.get_id();

        // ── Phase bar ───────────────────────────────────────────────────────
        let phase_bar = PhaseBar::new();
        let phase_bar_id = phase_bar.get_id();

        // ── Message list ────────────────────────────────────────────────────
        let mut msg_list = MessageList::new();
        msg_list.attach_renderer();
        msg_list.set_messages(vec![MsgKind::Interstitial {
            text: "Connecting to the world…".into(),
            is_voice: false,
        }]);
        let message_list_id = msg_list.get_id();

        let messages_pane = Pane::new()
            .flex(1)
            .min_width(0)
            .min_height(0)
            .y_scroll(Scrollbar::AutoHide)
            .children([msg_list]);

        // ── Btw pane ────────────────────────────────────────────────────────
        let btw_pane = BtwPane::new(&palette);
        let btw_pane_id = btw_pane.get_id();

        // ── Chat column ─────────────────────────────────────────────────────
        let chat_column = Pane::new()
            .vertical()
            .flex(1)
            .min_width(0)
            .min_height(0)
            .children([messages_pane, btw_pane as Box<dyn Widget>]);

        // ── Cockpit ─────────────────────────────────────────────────────────
        let cockpit = Cockpit::new(&palette);
        let cockpit_id = cockpit.get_id();

        let cockpit_pane = Pane::new().flex(1).min_width(0).children([cockpit]);

        // ── Sidebar ─────────────────────────────────────────────────────────
        let sidebar = ChatSidebar::new(&palette);
        let sidebar_id = sidebar.get_id();

        let mut sidebar_pane_id = WidgetId::EMPTY;
        let sidebar_pane = Pane::new()
            .flex(0)
            .max_height(0)
            .min_height(0)
            .children([sidebar])
            .id(&mut sidebar_pane_id);

        // ── Right column ────────────────────────────────────────────────────
        let mut right_column_id = WidgetId::EMPTY;
        let right_column = Pane::new()
            .vertical()
            .width(0)
            .max_width(0)
            .children([cockpit_pane, sidebar_pane])
            .id(&mut right_column_id);

        // ── Body row ────────────────────────────────────────────────────────
        let body = Pane::new()
            .horizontal()
            .flex(1)
            .min_height(0)
            .children([chat_column, right_column]);

        // ── Overlay ─────────────────────────────────────────────────────────
        let overlay = ChatOverlay::new(&palette);
        let overlay_id = overlay.get_id();

        // ── Footer ──────────────────────────────────────────────────────────
        let footer = Footer::new(&palette);
        let footer_id = footer.get_id();

        // ── Input row ───────────────────────────────────────────────────────
        let (input, input_id) = chat_input::new_chat_input();

        let mut prompt_id = WidgetId::EMPTY;
        let prompt = Text::new()
            .content(StyledStr::new(" › ").fg(primary).bold())
            .id(&mut prompt_id);
        let send_btn = SendButton::new(primary);
        let send_button_id = send_btn.get_id();
        let input_row = Pane::new()
            .horizontal()
            .bordered()
            .border_style(Style::new().fg(dim).dim())
            .padding(Spacing::new().horizontal(0).vertical(0))
            .children([
                prompt as Box<dyn Widget>,
                input.flex(1),
                send_btn as Box<dyn Widget>,
            ]);

        // ── Root ────────────────────────────────────────────────────────────
        let root = Pane::new().vertical().min_height(0).children([
            header as Box<dyn Widget>,
            itinerary as Box<dyn Widget>,
            body,
            overlay as Box<dyn Widget>,
            phase_bar as Box<dyn Widget>,
            input_row,
            footer as Box<dyn Widget>,
        ]);

        Box::new(Self {
            root,
            message_list_id,
            input_id,
            header_id,
            prompt_id,
            cockpit_id,
            footer_id,
            sidebar_id,
            overlay_id,
            btw_pane_id,
            itinerary_id,
            phase_bar_id,
            sidebar_pane_id,
            send_button_id,
            right_column_id,
            palette,
            _agent_name: agent_name,
            chat: None,
            _config: config,
            connecting: false,
            pending_messages: Vec::new(),
            cockpit_visible: false,
            sidebar_visible: false,
            slash_input: String::new(),
            slash_selected: 0,
            last_pressure: 0.0,
            last_model: String::new(),
            last_msg_count: 0,
        })
    }

    // ── Toggles ────────────────────────────────────────────────────────────

    fn toggle_cockpit(&mut self) {
        self.cockpit_visible = !self.cockpit_visible;
        self.update_right_column();
        self.root.dirty_layout();
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;

        if let Some(p) = self.root.get_widget_mut(self.sidebar_pane_id) {
            if self.sidebar_visible {
                p.set_flex(1);
                p.set_max_height(None);
            } else {
                p.set_flex(0);
                p.set_max_height(Some(0));
            }
        }

        self.update_right_column();
        self.root.dirty_layout();
    }

    fn update_right_column(&mut self) {
        let right_visible = self.cockpit_visible || self.sidebar_visible;
        if let Some(rc) = self.root.get_widget_mut(self.right_column_id) {
            // Hard-pin the width: `set_width` sets min == max == preferred, so a
            // flex chat column with long content can never negotiate it down to
            // its border. Reopening max (the old behaviour) let the column
            // collapse under live layout pressure.
            rc.set_width(Some(if right_visible { 36 } else { 0 }));
        }
    }

    // ── Overlay ────────────────────────────────────────────────────────────

    fn update_slash_overlay(&mut self) {
        // Only update if we have a chat and no conversation picker is open.
        let Some(chat) = self.chat.as_ref() else {
            return;
        };
        if matches!(chat.overlay, Overlay::ConversationPicker { .. }) {
            return;
        }

        let text = self.get_input_text();
        if text.starts_with('/') {
            let prefix = text.trim();
            let match_refs: Vec<&'static crate::ui::chat::SlashDef> =
                crate::ui::chat::SLASH_COMMANDS
                    .iter()
                    .filter(|cmd| cmd.name.starts_with(prefix) || prefix.len() <= 1)
                    .collect();

            if !match_refs.is_empty() {
                let selected = self.slash_selected.min(match_refs.len().saturating_sub(1));
                if let Some(o) = self.root.get_widget_mut(self.overlay_id) {
                    o.show_slash_commands(selected, &match_refs, &self.palette);
                }
                self.slash_selected = selected;
                // Mirror into ChatState so the input-routing layer (has_overlay,
                // overlay_move, overlay_confirm) sees and drives the overlay.
                // Without this the overlay rendered but Up/Down/Enter/Esc never
                // routed to it.
                if let Some(chat) = self.chat.as_mut() {
                    chat.overlay = Overlay::SlashComplete {
                        selected,
                        matches: match_refs,
                    };
                }
                return;
            }
        }
        if let Some(o) = self.root.get_widget_mut(self.overlay_id) {
            o.hide();
        }
        // Clear a stale slash overlay from state. The conversation picker is
        // excluded by the early return above, so we never clobber it here.
        if let Some(chat) = self.chat.as_mut() {
            if matches!(chat.overlay, Overlay::SlashComplete { .. }) {
                chat.overlay = Overlay::None;
            }
        }
        self.slash_selected = 0;
    }

    fn dismiss_overlay(&mut self) {
        if let Some(chat) = self.chat.as_mut() {
            chat.overlay = Overlay::None;
            chat.show_esc_overlay = false;
        }
        if let Some(o) = self.root.get_widget_mut(self.overlay_id) {
            o.hide();
        }
        self.slash_selected = 0;
    }

    fn overlay_move(&mut self, delta: isize) {
        let Some(chat) = self.chat.as_ref() else {
            return;
        };
        match &chat.overlay {
            Overlay::SlashComplete { selected, matches } => {
                let new = if delta < 0 {
                    selected.saturating_sub(1)
                } else {
                    (*selected + 1).min(matches.len().saturating_sub(1))
                };
                // Update via ChatState's overlay.
                if let Some(chat) = self.chat.as_mut() {
                    if let Overlay::SlashComplete {
                        ref mut selected, ..
                    } = &mut chat.overlay
                    {
                        *selected = new;
                    }
                }
                self.slash_selected = new;
                self.update_overlay_from_state();
            }
            Overlay::ConversationPicker {
                selected,
                conversations,
            } => {
                let new = if delta < 0 {
                    selected.saturating_sub(1)
                } else {
                    (*selected + 1).min(conversations.len().saturating_sub(1))
                };
                if let Some(chat) = self.chat.as_mut() {
                    if let Overlay::ConversationPicker {
                        ref mut selected, ..
                    } = &mut chat.overlay
                    {
                        *selected = new;
                    }
                }
                self.update_overlay_from_state();
            }
            Overlay::None => {}
        }
    }

    fn overlay_confirm(&mut self) {
        let action = {
            let Some(chat) = self.chat.as_ref() else {
                return;
            };
            match &chat.overlay {
                Overlay::SlashComplete { selected, matches } => {
                    matches.get(*selected).map(|cmd| cmd.name.to_string())
                }
                Overlay::ConversationPicker {
                    selected,
                    conversations,
                } => conversations.get(*selected).map(|c| c.id.clone()),
                Overlay::None => None,
            }
        };

        match action {
            Some(cmd) if cmd.starts_with('/') => {
                // Fill input with the selected command.
                if let Some(i) = self.root.get_widget_mut(self.input_id) {
                    i.set_content(&cmd);
                }
                self.dismiss_overlay();
            }
            Some(conv_id) => {
                // Switch to the selected conversation.
                if let Some(chat) = self.chat.as_mut() {
                    chat.handle_switch_conversation(conv_id);
                }
                self.dismiss_overlay();
            }
            None => {}
        }
    }

    fn update_overlay_from_state(&mut self) {
        let Some(chat) = self.chat.as_ref() else {
            return;
        };
        match &chat.overlay {
            Overlay::SlashComplete { selected, matches } => {
                if let Some(o) = self.root.get_widget_mut(self.overlay_id) {
                    o.show_slash_commands(*selected, matches, &self.palette);
                }
            }
            Overlay::ConversationPicker {
                selected,
                conversations,
            } => {
                if let Some(o) = self.root.get_widget_mut(self.overlay_id) {
                    o.show_conversations(*selected, conversations, &self.palette);
                }
            }
            Overlay::None => {
                if let Some(o) = self.root.get_widget_mut(self.overlay_id) {
                    o.hide();
                }
            }
        }
    }

    // ── Activation (called by TuieApp after widget is in tree) ────────────

    /// Mark the chat screen as connecting so the interstitial message shows.
    /// Does NOT spawn the backend connect — that's owned by TuieApp because
    /// DelegateWidget tree traversal can't find this widget via get_widget_mut.
    pub fn start_connecting(&mut self) {
        self.connecting = true;
        self.focus_input();
    }

    /// Called by TuieApp once the async connect completes.
    pub fn finish_connect(&mut self, result: anyhow::Result<ChatState>) {
        self.connecting = false;
        match result {
            Ok(mut chat) => {
                chat.palette = self.palette;
                chat.offer_resume_or_new();

                let agent_name = chat.agent_name.clone();
                let mode = chat.mode.clone();
                let conv_id = chat.conversation_id.clone();
                self.last_model = mode.clone();
                if let Some(f) = self.root.get_widget_mut(self.footer_id) {
                    f.set_agent_name(&agent_name);
                    f.set_mode(&mode);
                    f.set_conversation_id(&conv_id);
                }

                // Drain any messages queued while connecting.
                let pending = std::mem::take(&mut self.pending_messages);
                for text in &pending {
                    chat.input = text.clone();
                    chat.submit();
                }

                self.chat = Some(chat);
                let msgs: Vec<MsgKind> = self
                    .chat
                    .as_ref()
                    .unwrap()
                    .messages
                    .iter()
                    .map(chat_message_to_msgkind)
                    .collect();
                self.set_messages(msgs);
                self.update_header(&agent_name);
                self.focus_input();
            }
            Err(e) => {
                self.set_messages(vec![MsgKind::System {
                    text: format!("Could not connect: {e}"),
                }]);
                tuie::dirty_paint();
            }
        }
    }

    // ── Polling ────────────────────────────────────────────────────────────

    /// Drain backend events and refresh widgets for one tick.
    ///
    /// Driven by `TuieApp` (see `TuieApp::schedule_chat_poll`), not self-scheduled:
    /// `ChatScreen` is the direct delegate of `TuieApp` and shares its widget id,
    /// so a `schedule`/`get_widget_mut::<ChatScreen>` keyed on that id resolves to
    /// the root `TuieApp` and fails to downcast — the callback would never fire.
    pub(crate) fn poll_tick(&mut self) {
        // Extract all state from chat in a scoped block, then apply to widgets.
        let snapshot = {
            let Some(chat) = self.chat.as_mut() else {
                return;
            };
            chat.drain_events();
            chat.advance_tick();

            let msgs: Vec<MsgKind> = chat.messages.iter().map(chat_message_to_msgkind).collect();

            // Scroll when the transcript grows (new user/agent/tool message) or
            // while the last message is a live stream still gaining tokens.
            let needs_scroll = msgs.len() > self.last_msg_count
                || msgs
                    .last()
                    .map(|m| {
                        matches!(
                            m,
                            MsgKind::Assistant {
                                streaming: true,
                                ..
                            }
                        )
                    })
                    .unwrap_or(false);

            let cockpit_data = if self.cockpit_visible {
                Some((chat.thinking.clone(), chat.cockpit_log.clone()))
            } else {
                None
            };

            // Phase info
            let phase = chat.phase;
            let tick = chat.tick;
            let turn_started = chat.turn_started;
            let tool_calls = chat.tool_calls_this_turn;
            let now = std::time::Instant::now();
            let queued = chat
                .pending_interjections
                .lock()
                .ok()
                .map(|q| q.len())
                .unwrap_or(0);
            let pressure = chat.pressure;
            let context_limit = chat.context_limit;
            let itinerary = chat.itinerary_line.clone();
            let btw_state = chat.btw_state.clone();
            let overlay = chat.overlay.clone();
            let show_esc = chat.show_esc_overlay;
            let model = chat.mode.clone();
            let render_mode = chat.render_mode;
            let conversation_id = chat.conversation_id.clone();
            let tools_expanded = chat.tool_cards_expanded;

            // Sidebar data
            let sidebar_data = Some((
                chat.agent_name.clone(),
                "Active".to_string(),          // mood — from ChatState phase
                (chat.pressure * 100.0) as u8, // energy as pressure proxy
                chat.pending_interjections
                    .lock()
                    .ok()
                    .map(|q| q.len())
                    .unwrap_or(0),
                0u32, // memory_commits placeholder
            ));

            // Health data for sidebar
            let health_data = crate::ui::widgets::chat_sidebar::HealthData {
                backend_mode: chat.backend_mode.clone(),
                backend_healthy: chat.backend_healthy,
                last_reflection: chat.last_reflection.clone(),
                last_archivist: chat.last_archivist.clone(),
                last_compaction: chat.last_compaction.clone(),
                strain_504: chat.strain_504,
                strain_429: chat.strain_429,
                strain_other: chat.strain_other,
            };

            // Subconscious stream data — live N+1 reasoning text for the phase bar.
            let subconscious_stream_lines = chat.subconscious_stream.clone();
            let subconscious_live_line = chat.subconscious_current.clone();

            PollSnapshot {
                msgs,
                needs_scroll,
                cockpit_data,
                phase,
                tick,
                turn_started,
                tool_calls,
                now,
                queued,
                pressure,
                context_limit,
                itinerary,
                btw_state,
                overlay,
                show_esc,
                model,
                render_mode,
                conversation_id,
                tools_expanded,
                sidebar_data,
                health_data,
                subconscious_stream_lines,
                subconscious_live_line,
            }
        };

        // ── Apply snapshot to widgets ──────────────────────────────────────

        self.last_msg_count = snapshot.msgs.len();
        self.set_messages(snapshot.msgs);

        if snapshot.needs_scroll {
            self.scroll_to_bottom();
        }

        // Cockpit
        if let Some((ref thinking, ref cockpit_log)) = snapshot.cockpit_data {
            let active = match snapshot.phase {
                TurnPhase::Subconscious => ActivePane::Subconscious,
                TurnPhase::Thinking | TurnPhase::Streaming | TurnPhase::Tool => {
                    ActivePane::Thinking
                }
                _ => ActivePane::None,
            };
            if let Some(c) = self.root.get_widget_mut(self.cockpit_id) {
                c.set_thinking(thinking, &self.palette);
                c.set_subconscious(cockpit_log, &self.palette);
                c.set_active(active);
                c.set_pressure(snapshot.pressure, &self.palette);
                c.refresh_inner_voice();
            }
        }

        // Phase bar
        if let Some(pb) = self.root.get_widget_mut(self.phase_bar_id) {
            if snapshot.phase == TurnPhase::Idle {
                pb.clear();
            } else {
                let kind = match snapshot.phase {
                    TurnPhase::Thinking => PhaseKind::Thinking,
                    TurnPhase::Tool => PhaseKind::RunningTool,
                    TurnPhase::Streaming => PhaseKind::Streaming,
                    TurnPhase::Interrupted => PhaseKind::Interrupted,
                    TurnPhase::Subconscious => PhaseKind::Subconscious,
                    TurnPhase::Idle => PhaseKind::Idle,
                };
                let elapsed = snapshot
                    .turn_started
                    .map(|t| snapshot.now.duration_since(t).as_secs())
                    .unwrap_or(0);
                let spinner_idx = (snapshot.tick as usize / 2) % SPINNER.len();
                // Elapsed since last event for "waiting" display.
                let quiet = 0u64; // simplified — could track last_event_at

                let style = Style::new().fg(theme::to_tuie_color(self.palette.agent_primary));
                let dim_style = Style::new().fg(theme::to_tuie_color(self.palette.agent_dim));
                pb.set_phase(
                    kind,
                    spinner_idx,
                    elapsed,
                    snapshot.tool_calls,
                    snapshot.queued,
                    quiet,
                    style,
                    dim_style,
                );
                if snapshot.phase == TurnPhase::Subconscious
                    && (!snapshot.subconscious_live_line.is_empty()
                        || !snapshot.subconscious_stream_lines.is_empty())
                {
                    pb.set_subconscious_stream(
                        &snapshot.subconscious_live_line,
                        &snapshot.subconscious_stream_lines,
                        style,
                        dim_style,
                    );
                }
            }
        }

        // Itinerary
        if let Some(s) = self.root.get_widget_mut(self.itinerary_id) {
            s.set_line(&snapshot.itinerary, &self.palette);
        }

        // Btw pane
        if let Some(b) = self.root.get_widget_mut(self.btw_pane_id) {
            b.set_state(&snapshot.btw_state, &self.palette);
        }

        // Overlay (sync from ChatState)
        match &snapshot.overlay {
            Overlay::None => {
                if snapshot.show_esc {
                    // Show esc overlay as a simplified popup.
                }
                // Don't auto-hide — let dismiss_overlay handle it.
            }
            _ => {
                self.update_overlay_from_state();
            }
        }

        // Input prompt glyph — reflects Conversation (›) vs Code (≡) posture.
        let (glyph, glyph_color) = match snapshot.render_mode {
            crate::ui::chat::ChatMode::Conversation => (" › ", self.palette.agent_primary),
            crate::ui::chat::ChatMode::Code => (" ≡ ", self.palette.tool_accent),
        };
        if let Some(p) = self.root.get_widget_mut(self.prompt_id) {
            p.set_content(
                StyledStr::new(glyph)
                    .fg(theme::to_tuie_color(glyph_color))
                    .bold(),
            );
        }

        // Footer
        self.last_pressure = snapshot.pressure;
        if let Some(f) = self.root.get_widget_mut(self.footer_id) {
            if snapshot.model != self.last_model {
                self.last_model = snapshot.model.clone();
                f.set_model(&snapshot.model);
            }
            f.set_pressure(snapshot.pressure);
            f.set_context_limit(snapshot.context_limit);
            f.set_conversation_id(&snapshot.conversation_id);
            f.set_cockpit_open(self.cockpit_visible);
            f.set_tools_expanded(snapshot.tools_expanded);
        }

        // Sidebar
        if self.sidebar_visible {
            if let Some((name, mood, energy, tasks, commits)) = snapshot.sidebar_data {
                let status = crate::ui::tuie_app::AgentStatus {
                    name,
                    mood,
                    energy,
                    memory_commits: commits,
                    pending_tasks: tasks,
                    ..Default::default()
                };
                if let Some(s) = self.root.get_widget_mut(self.sidebar_id) {
                    // N+1 count from thinking state
                    let n1 = if let Some((ref thinking, _)) = snapshot.cockpit_data {
                        thinking.len()
                    } else {
                        0
                    };
                    s.update(&status, snapshot.pressure, n1, &self.palette);
                    s.update_health(&snapshot.health_data, &self.palette);
                }
            }
        }

        tuie::dirty_paint();
    }

    // ── Input ──────────────────────────────────────────────────────────────

    fn submit_text(&mut self, text: &str) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }

        let submitted = if let Some(chat) = self.chat.as_mut() {
            // Drain any pending messages first, then submit the latest.
            for pending in self.pending_messages.drain(..) {
                chat.input = pending;
                chat.submit();
            }
            chat.input = text;
            chat.submit()
        } else {
            // Still connecting — queue the message for later.
            if !self.pending_messages.contains(&text) {
                self.pending_messages.push(text);
                self.set_messages(vec![MsgKind::System {
                    text: "\u{2192} waiting for connection...".to_string(),
                }]);
                tuie::dirty_paint();
            }
            return;
        };

        self.clear_input();

        if submitted {
            if let Some(chat) = self.chat.as_ref() {
                let msgs: Vec<MsgKind> =
                    chat.messages.iter().map(chat_message_to_msgkind).collect();
                self.set_messages(msgs);
            }
            self.scroll_to_bottom();
            tuie::dirty_paint();
        }
    }

    // ── Helpers ────────────────────────────────────────────────────────────

    fn set_messages(&mut self, messages: Vec<MsgKind>) {
        if let Some(ml) = self.root.get_widget_mut(self.message_list_id) {
            ml.set_messages(messages);
        }
    }

    fn scroll_to_bottom(&mut self) {
        if let Some(ml) = self.root.get_widget_mut(self.message_list_id) {
            ml.scroll_to_bottom();
        }
    }

    fn get_input_text(&self) -> String {
        chat_input::read_input_text(&*self.root, self.input_id)
    }

    fn clear_input(&mut self) {
        chat_input::clear_input(&mut *self.root, self.input_id);
    }

    fn update_btw(&mut self) {
        if let Some(chat) = self.chat.as_ref() {
            let state = chat.btw_state.clone();
            if let Some(b) = self.root.get_widget_mut(self.btw_pane_id) {
                b.set_state(&state, &self.palette);
            }
        }
    }

    pub fn focus_input(&self) {
        tuie::focus_widget(self.input_id.untyped());
    }

    fn update_header(&mut self, name: &str) {
        if let Some(h) = self.root.get_widget_mut(self.header_id) {
            let color = theme::to_tuie_color(self.palette.agent_primary);
            let mut content = StyledString::new();
            content.push_span(
                StyledStr::new(&format!(" ✦ Souveraine · {name}"))
                    .bold()
                    .fg(color),
            );
            h.set_content(content);
        }
    }

    // ── Setters (called by TuieApp) ────────────────────────────────────────

    #[allow(dead_code)]
    pub fn set_palette(&mut self, palette: ChatPalette) {
        self.palette = palette;
        if let Some(chat) = self.chat.as_mut() {
            chat.palette = palette;
        }
        if let Some(ml) = self.root.get_widget_mut(self.message_list_id) {
            ml.set_palette(palette);
        }
        let name = self
            .chat
            .as_ref()
            .map(|c| c.agent_name.clone())
            .unwrap_or_default();
        if !name.is_empty() {
            self.update_header(&name);
        }
    }
}

// ── Poll snapshot ────────────────────────────────────────────────────────────

/// Data extracted from `ChatState` during one poll tick.
/// Built in a scoped block to avoid borrow conflicts.
struct PollSnapshot {
    msgs: Vec<MsgKind>,
    needs_scroll: bool,
    cockpit_data: Option<(Vec<String>, Vec<crate::ui::chat::CockpitEntry>)>,
    phase: TurnPhase,
    tick: u64,
    turn_started: Option<std::time::Instant>,
    tool_calls: u32,
    now: std::time::Instant,
    queued: usize,
    pressure: f32,
    context_limit: Option<usize>,
    itinerary: String,
    btw_state: BtwState,
    overlay: Overlay,
    show_esc: bool,
    model: String,
    render_mode: crate::ui::chat::ChatMode,
    conversation_id: String,
    tools_expanded: bool,
    sidebar_data: Option<(String, String, u8, usize, u32)>,
    health_data: crate::ui::widgets::chat_sidebar::HealthData,
    subconscious_stream_lines: Vec<String>,
    subconscious_live_line: String,
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::chat::ChatPalette;
    use tuie::emulator::Emulator;
    use tuie::input::chord::Chord;
    use tuie::input::key::Key;
    use tuie::input::modifiers::{Modifier, Modifiers};
    use tuie::input::trigger::Trigger;

    // ── Input routing helpers ─────────────────────────────────────────────

    /// Build a keyboard [`RuntimeEvent`] for `key` with `mods` held.
    fn key_event(key: Key, mods: Modifiers) -> RuntimeEvent {
        Chord::new(Trigger::Key(key), mods).into()
    }

    /// A bare typed character event.
    fn typed(c: char) -> RuntimeEvent {
        key_event(Key::Char(c), Modifiers::new())
    }

    /// Create a chat screen, render it, and focus its input so typed keys are
    /// routed to the `Input` leaf (the focus-chain forward walk needs a target).
    fn focused_screen() -> (Box<ChatScreen>, Emulator) {
        let config = Arc::new(RwLock::new(ConsciousnessConfig::default()));
        let mut screen = ChatScreen::new(config, ChatPalette::default(), "test-agent".into());
        let mut term = Emulator::new(&mut *screen, Vec2::new(120, 40));
        screen.focus_input();
        // Force a dirty frame so `layout_and_render` runs `repair_selection`,
        // which is what actually applies a pending focus request. Without a
        // dirty frame the render is skipped and the request stays pending.
        tuie::dirty_paint();
        term.update(&mut *screen, &[RuntimeEvent::Resize(Vec2::new(120, 40))]);
        (screen, term)
    }

    #[test]
    fn typing_reaches_focused_input() {
        let (mut screen, mut term) = focused_screen();
        term.update(&mut *screen, &[typed('h')]);
        term.update(&mut *screen, &[typed('i')]);
        assert_eq!(screen.get_input_text(), "hi");
    }

    #[test]
    fn bare_enter_submits_input() {
        let (mut screen, mut term) = focused_screen();
        term.update(&mut *screen, &[typed('h'), typed('i')]);
        term.update(&mut *screen, &[key_event(Key::Enter, Modifiers::new())]);

        // Disconnected (chat == None), so submit queues into pending_messages.
        assert_eq!(
            screen.pending_messages,
            vec!["hi".to_string()],
            "bare Enter should submit the input text"
        );
        // Enter was consumed by ChatScreen — it must not have inserted a newline.
        assert!(
            !screen.get_input_text().contains('\n'),
            "bare Enter must not insert a newline into the Input"
        );
    }

    #[test]
    fn alt_enter_inserts_newline_without_submitting() {
        let (mut screen, mut term) = focused_screen();
        term.update(
            &mut *screen,
            &[
                typed('a'),
                key_event(Key::Enter, Modifiers::new().with(Modifier::Alt)),
                typed('b'),
            ],
        );

        assert_eq!(
            screen.get_input_text(),
            "a\nb",
            "Alt+Enter should insert a literal newline at the cursor"
        );
        assert!(
            screen.pending_messages.is_empty(),
            "Alt+Enter must not submit the input"
        );
    }

    #[test]
    fn ctrl_enter_inserts_newline_without_submitting() {
        let (mut screen, mut term) = focused_screen();
        term.update(
            &mut *screen,
            &[
                typed('x'),
                key_event(Key::Enter, Modifiers::new().with(Modifier::Ctrl)),
                typed('y'),
            ],
        );

        assert_eq!(screen.get_input_text(), "x\ny");
        assert!(screen.pending_messages.is_empty());
    }

    // ── Existing test ─────────────────────────────────────────────────────

    #[test]
    fn chat_screen_renders_border_and_interstitial() {
        let config = Arc::new(RwLock::new(ConsciousnessConfig::default()));
        let palette = ChatPalette::default();
        let mut screen = ChatScreen::new(config, palette, "test-agent".into());

        let term = Emulator::new(&mut *screen, Vec2::new(120, 40));
        let rendered = term.get_snapshot_text();

        assert!(!rendered.trim().is_empty(), "chat screen rendered nothing!");
        assert!(
            rendered.contains("Connecting") || rendered.contains("world"),
            "missing interstitial message\nraw: {rendered:?}"
        );
    }

    // ── Toggle tests ─────────────────────────────────────────────────────

    #[test]
    fn chat_screen_toggle_cockpit() {
        let config = Arc::new(RwLock::new(ConsciousnessConfig::default()));
        let palette = ChatPalette::default();
        let mut screen = ChatScreen::new(config, palette, "test-agent".into());

        assert!(!screen.cockpit_visible, "cockpit should start hidden");
        screen.toggle_cockpit();
        assert!(
            screen.cockpit_visible,
            "cockpit should be visible after toggle"
        );
        screen.toggle_cockpit();
        assert!(
            !screen.cockpit_visible,
            "cockpit should be hidden after second toggle"
        );
    }

    /// The cockpit panes must fill the column height, not collapse to their
    /// minimum content height. Regression for the missing `.flex(1)` on the
    /// cockpit `Split`, which left it content-sized and top-aligned with a
    /// large dead gap below.
    #[test]
    fn cockpit_open_fills_column_height() {
        let (mut screen, term) = focused_screen();
        screen.toggle_cockpit();
        tuie::dirty_paint();
        let mut term = term;
        term.update(&mut *screen, &[RuntimeEvent::Resize(Vec2::new(120, 40))]);
        let out = term.get_snapshot_text();

        assert!(
            out.contains("thinking") && out.contains("subconscious") && out.contains("inner voice"),
            "all three cockpit panes should render\n{out}"
        );
        // When the panes fill the ~36-row body, the right border glyph appears
        // on many rows. When collapsed (the bug) it was ~4. Use a conservative
        // threshold well above the collapsed count.
        let border_rows = out.lines().filter(|l| l.contains('│')).count();
        assert!(
            border_rows > 20,
            "cockpit panes look collapsed ({border_rows} bordered rows); expected fill\n{out}"
        );
    }

    #[test]
    fn chat_screen_toggle_sidebar() {
        let config = Arc::new(RwLock::new(ConsciousnessConfig::default()));
        let palette = ChatPalette::default();
        let mut screen = ChatScreen::new(config, palette, "test-agent".into());

        assert!(!screen.sidebar_visible, "sidebar should start hidden");
        screen.toggle_sidebar();
        assert!(
            screen.sidebar_visible,
            "sidebar should be visible after toggle"
        );
        screen.toggle_sidebar();
        assert!(
            !screen.sidebar_visible,
            "sidebar should be hidden after second toggle"
        );
    }

    #[test]
    fn chat_screen_toggle_cockpit_and_sidebar_independent() {
        let config = Arc::new(RwLock::new(ConsciousnessConfig::default()));
        let palette = ChatPalette::default();
        let mut screen = ChatScreen::new(config, palette, "test-agent".into());

        // Both start hidden
        assert!(!screen.cockpit_visible);
        assert!(!screen.sidebar_visible);

        // Open cockpit only
        screen.toggle_cockpit();
        assert!(screen.cockpit_visible);
        assert!(!screen.sidebar_visible);

        // Open sidebar (cockpit stays open)
        screen.toggle_sidebar();
        assert!(screen.cockpit_visible);
        assert!(screen.sidebar_visible);

        // Close cockpit only
        screen.toggle_cockpit();
        assert!(!screen.cockpit_visible);
        assert!(screen.sidebar_visible);

        // Close sidebar
        screen.toggle_sidebar();
        assert!(!screen.cockpit_visible);
        assert!(!screen.sidebar_visible);
    }

    #[test]
    fn cockpit_renders_on_screen_when_toggled() {
        let config = Arc::new(RwLock::new(ConsciousnessConfig::default()));
        let mut screen = ChatScreen::new(config, ChatPalette::default(), "test-agent".into());
        let mut term = Emulator::new(&mut *screen, Vec2::new(120, 40));
        tuie::dirty_paint();
        term.update(&mut *screen, &[RuntimeEvent::Resize(Vec2::new(120, 40))]);

        // Hidden: the cockpit pane titles must NOT be on screen.
        let before = term.get_snapshot_text();
        assert!(
            !before.contains("thinking")
                && !before.contains("subconscious")
                && !before.contains("inner voice"),
            "cockpit should be off-screen before toggle, got: {before:?}"
        );

        // Toggle open, push some data through the same path poll_tick uses.
        screen.toggle_cockpit();
        if let Some(c) = screen.root.get_widget_mut(screen.cockpit_id) {
            c.set_thinking(
                &["weighing the request".to_string()],
                &ChatPalette::default(),
            );
            c.set_subconscious(&[], &ChatPalette::default());
            c.refresh_inner_voice();
        }
        tuie::dirty_paint();
        term.update(&mut *screen, &[RuntimeEvent::Resize(Vec2::new(120, 40))]);

        let after = term.get_snapshot_text();
        assert!(
            after.contains("thinking"),
            "thinking pane title should be visible after toggle, got: {after:?}"
        );
        assert!(
            after.contains("subconscious"),
            "subconscious pane title should be visible after toggle, got: {after:?}"
        );
        assert!(
            after.contains("weighing the request"),
            "thinking content should render, got: {after:?}"
        );
    }
}
