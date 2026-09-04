//! Central keyboard map for the tuie app.
//!
//! Every binding lives in [`KEYMAP`] as a `(KeyContext, KeyCode, mods) ->
//! Action` row. Dispatch computes the current [`KeyContext`] from app state,
//! looks up the [`Action`] by *exact* `(code, mods)` match, and the action
//! handler in `app/mod.rs` runs the semantics — which may still branch on app
//! state (Presence `Esc` interrupts or leaves depending on posture; Chat `Esc`
//! depends on `busy`/overlay).
//!
//! Why a table instead of inline `match` arms:
//!   * Lookup is exact on `(code, mods)`, so a plain key and its modified
//!     sibling are *distinct rows*. The old `KeyCode::Left` arm shadowing a
//!     later `KeyCode::Left if CONTROL` arm — silently killing Ctrl+Left word
//!     jump — is now structurally impossible.
//!   * [`tests::no_duplicate_bindings`] fails the build if any context binds the
//!     same `(code, mods)` twice. Accidental overlap can't ship.
//!
//! Scope: covers Welcome, Presence, Chat (+ its modal sub-contexts), and the
//! agent manager. Setup/Splash/Cron/Settings keep their own handlers for now
//! (form-style text editing); folding them in is a follow-up.

use crossterm::event::{KeyCode, KeyModifiers};

/// The modal context a key is interpreted in. Finer-grained than `Screen`
/// because Chat has several modal overlays that capture keys differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyContext {
    /// Home screen menu.
    Welcome,
    /// Fullscreen "be with her" voice mode.
    Presence,
    /// Agent manager card grid.
    AgentsManager,
    /// Plain back-to-welcome screens (Therapy, AgentTime).
    GenericBack,
    /// Chat is selected but no `ChatState` is connected yet.
    ChatDisconnected,
    /// Normal chat text input.
    Chat,
    /// `/btw` fork pane is showing.
    ChatBtw,
    /// Slash-command completion overlay (falls through to `Chat` on miss).
    ChatSlashComplete,
    /// Conversation picker overlay (fully modal — no fall-through).
    ChatConvPicker,
    /// Esc interrupt-or-leave overlay shown while a turn is running.
    ChatEscOverlay,
}

/// A semantic intent resolved from a keypress. The handler owns the behavior;
/// several actions stay state-dependent on purpose (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    // Global-ish
    Quit,
    Back,

    // Welcome
    MenuUp,
    MenuDown,
    MenuSelect,
    CycleAgent,
    OpenPresence,
    OpenManager,

    // Presence
    PresenceEsc,
    PresenceRecordToggle,
    PresenceExit,
    PresenceReplay,
    PresenceRegen,
    PresenceSave,

    // Agent manager
    ManagerLeft,
    ManagerRight,
    ManagerUp,
    ManagerDown,
    ManagerSelect,
    ManagerPin,

    // Chat — input editing
    Submit,
    InsertNewline,
    Backspace,
    CharLeft,
    CharRight,
    WordLeft,
    WordRight,
    LineHome,
    LineEnd,

    // Chat — global
    ToggleCockpit,
    ToggleToolCards,
    PasteImage,
    ChatEsc,

    // Chat — overlays
    OverlayUp,
    OverlayDown,
    OverlayAccept,
    OverlayCancel,
    PickerNewConversation,

    // Chat — /btw pane
    BtwDismiss,
    BtwJump,

    // Chat — esc overlay
    EscOverlayResume,
    EscOverlayInterject,
    EscOverlayLeave,
}

/// One row of the keymap. `mods` is stored in canonical form (see [`canon`]).
pub struct Binding {
    pub ctx: KeyContext,
    pub code: KeyCode,
    pub mods: KeyModifiers,
    pub action: Action,
}

const NONE: KeyModifiers = KeyModifiers::NONE;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;

use Action::*;
use KeyContext::*;

/// The single source of truth for every keyboard binding in the covered
/// contexts. Grouped by context; a multi-key action (e.g. `q`/`Esc`) is just
/// several rows. Order does not matter — lookup is exact, not first-match.
pub const KEYMAP: &[Binding] = &[
    // ── Welcome ──────────────────────────────────────────
    b(Welcome, KeyCode::Char('q'), NONE, Quit),
    b(Welcome, KeyCode::Esc, NONE, Quit),
    b(Welcome, KeyCode::Up, NONE, MenuUp),
    b(Welcome, KeyCode::Down, NONE, MenuDown),
    b(Welcome, KeyCode::Enter, NONE, MenuSelect),
    b(Welcome, KeyCode::Char('a'), NONE, CycleAgent),
    b(Welcome, KeyCode::Char('p'), NONE, OpenPresence),
    b(Welcome, KeyCode::Char('i'), NONE, OpenManager),
    // ── Presence ─────────────────────────────────────────
    b(Presence, KeyCode::Esc, NONE, PresenceEsc),
    b(Presence, KeyCode::Char(' '), NONE, PresenceRecordToggle),
    b(Presence, KeyCode::Char('q'), NONE, PresenceExit),
    b(Presence, KeyCode::Char('r'), NONE, PresenceReplay),
    b(Presence, KeyCode::Char('g'), NONE, PresenceRegen),
    b(Presence, KeyCode::Char('s'), NONE, PresenceSave),
    // ── Agent manager ────────────────────────────────────
    b(AgentsManager, KeyCode::Char('q'), NONE, Back),
    b(AgentsManager, KeyCode::Esc, NONE, Back),
    b(AgentsManager, KeyCode::Char('i'), NONE, Back),
    b(AgentsManager, KeyCode::Left, NONE, ManagerLeft),
    b(AgentsManager, KeyCode::Right, NONE, ManagerRight),
    b(AgentsManager, KeyCode::Up, NONE, ManagerUp),
    b(AgentsManager, KeyCode::Down, NONE, ManagerDown),
    b(AgentsManager, KeyCode::Enter, NONE, ManagerSelect),
    b(AgentsManager, KeyCode::Char('f'), NONE, ManagerPin),
    // ── Generic back-to-welcome screens ──────────────────
    b(GenericBack, KeyCode::Char('q'), NONE, Back),
    b(GenericBack, KeyCode::Esc, NONE, Back),
    b(GenericBack, KeyCode::Char('m'), NONE, Back),
    // ── Chat, not yet connected ──────────────────────────
    b(ChatDisconnected, KeyCode::Esc, NONE, Back),
    b(ChatDisconnected, KeyCode::Char('q'), NONE, Back),
    // ── Chat /btw fork pane ──────────────────────────────
    b(ChatBtw, KeyCode::Esc, NONE, BtwDismiss),
    b(ChatBtw, KeyCode::Char('q'), NONE, BtwDismiss),
    b(ChatBtw, KeyCode::Char('j'), NONE, BtwJump),
    // ── Chat slash-completion overlay (misses fall through to Chat) ──
    b(ChatSlashComplete, KeyCode::Up, NONE, OverlayUp),
    b(ChatSlashComplete, KeyCode::Down, NONE, OverlayDown),
    b(ChatSlashComplete, KeyCode::Tab, NONE, OverlayAccept),
    b(ChatSlashComplete, KeyCode::Enter, NONE, OverlayAccept),
    b(ChatSlashComplete, KeyCode::Esc, NONE, OverlayCancel),
    // ── Chat conversation picker (fully modal) ───────────
    b(ChatConvPicker, KeyCode::Up, NONE, OverlayUp),
    b(ChatConvPicker, KeyCode::Char('k'), NONE, OverlayUp),
    b(ChatConvPicker, KeyCode::Down, NONE, OverlayDown),
    b(ChatConvPicker, KeyCode::Char('j'), NONE, OverlayDown),
    b(ChatConvPicker, KeyCode::Enter, NONE, OverlayAccept),
    b(
        ChatConvPicker,
        KeyCode::Char('n'),
        NONE,
        PickerNewConversation,
    ),
    b(ChatConvPicker, KeyCode::Esc, NONE, OverlayCancel),
    // ── Chat esc overlay (interrupt-or-leave, shown while busy) ──
    b(ChatEscOverlay, KeyCode::Esc, NONE, EscOverlayResume),
    b(ChatEscOverlay, KeyCode::Char('c'), NONE, EscOverlayResume),
    b(
        ChatEscOverlay,
        KeyCode::Char('i'),
        NONE,
        EscOverlayInterject,
    ),
    b(ChatEscOverlay, KeyCode::Char('m'), NONE, EscOverlayLeave),
    // ── Chat, normal text input ──────────────────────────
    // Unmatched keys here fall back to inserting the character (handled by the
    // dispatcher), so plain printable chars deliberately have no rows.
    b(Chat, KeyCode::Esc, NONE, ChatEsc),
    b(Chat, KeyCode::Enter, SHIFT, InsertNewline),
    b(Chat, KeyCode::Char('j'), CTRL, InsertNewline),
    b(Chat, KeyCode::Enter, NONE, Submit),
    b(Chat, KeyCode::Backspace, NONE, Backspace),
    b(Chat, KeyCode::Left, NONE, CharLeft),
    b(Chat, KeyCode::Right, NONE, CharRight),
    b(Chat, KeyCode::Left, CTRL, WordLeft),
    b(Chat, KeyCode::Right, CTRL, WordRight),
    b(Chat, KeyCode::Home, NONE, LineHome),
    b(Chat, KeyCode::End, NONE, LineEnd),
    b(Chat, KeyCode::Tab, NONE, ToggleCockpit),
    b(Chat, KeyCode::Char('c'), CTRL, Quit),
    b(Chat, KeyCode::Char('t'), CTRL, ToggleToolCards),
    // Ctrl+Shift+V — `Char('V')` is the shifted glyph; canon keeps CTRL only.
    b(Chat, KeyCode::Char('V'), CTRL, PasteImage),
];

/// Const-fn row constructor so [`KEYMAP`] stays a `const`.
const fn b(ctx: KeyContext, code: KeyCode, mods: KeyModifiers, action: Action) -> Binding {
    Binding {
        ctx,
        code,
        mods,
        action,
    }
}

/// Reduce raw modifiers to the bits the keymap compares on.
///
/// For `Char` codes the SHIFT bit is dropped — case is already encoded in the
/// glyph (`v` vs `V`), so a shifted letter must not look different from a typed
/// capital. For every other code SHIFT is meaningful (e.g. Shift+Enter) and
/// kept. Modifiers we never bind on (SUPER, HYPER, META) are discarded.
pub fn canon(code: KeyCode, mods: KeyModifiers) -> KeyModifiers {
    let mut m = mods & (CTRL | KeyModifiers::ALT | SHIFT);
    if matches!(code, KeyCode::Char(_)) {
        m.remove(SHIFT);
    }
    m
}

/// Resolve a keypress to an [`Action`] in the given context, or `None` if the
/// context doesn't bind it. A `None` is the caller's cue to apply the context's
/// fallback (insert the char in Chat, exit-if-idle in Presence, etc.).
pub fn resolve(ctx: KeyContext, code: KeyCode, mods: KeyModifiers) -> Option<Action> {
    let want = canon(code, mods);
    KEYMAP
        .iter()
        .find(|bnd| bnd.ctx == ctx && bnd.code == code && bnd.mods == want)
        .map(|bnd| bnd.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No context may bind the same `(code, canon mods)` twice — that's the
    /// accidental-overlap class of bug, caught at build time.
    #[test]
    fn no_duplicate_bindings() {
        let mut seen: Vec<(KeyContext, KeyCode, KeyModifiers)> = Vec::new();
        for bnd in KEYMAP {
            let key = (bnd.ctx, bnd.code, canon(bnd.code, bnd.mods));
            assert!(
                !seen.contains(&key),
                "duplicate binding: {:?} {:?} {:?}",
                bnd.ctx,
                bnd.code,
                bnd.mods
            );
            seen.push(key);
        }
    }

    /// Table rows must already be canonical — otherwise `resolve` (which
    /// canonicalizes the *incoming* key) could never match them.
    #[test]
    fn rows_are_canonical() {
        for bnd in KEYMAP {
            assert_eq!(
                bnd.mods,
                canon(bnd.code, bnd.mods),
                "non-canonical mods on {:?} {:?}",
                bnd.ctx,
                bnd.code
            );
        }
    }

    /// The regression that motivated the table: a plain arrow and its Ctrl
    /// sibling must resolve to different actions.
    #[test]
    fn ctrl_arrows_are_distinct() {
        assert_eq!(resolve(Chat, KeyCode::Left, NONE), Some(CharLeft));
        assert_eq!(resolve(Chat, KeyCode::Left, CTRL), Some(WordLeft));
        assert_eq!(resolve(Chat, KeyCode::Right, NONE), Some(CharRight));
        assert_eq!(resolve(Chat, KeyCode::Right, CTRL), Some(WordRight));
    }

    /// A shifted capital `V` inserts; only Ctrl+(Shift+)V pastes.
    #[test]
    fn shift_v_inserts_ctrl_v_pastes() {
        assert_eq!(resolve(Chat, KeyCode::Char('V'), SHIFT), None);
        assert_eq!(resolve(Chat, KeyCode::Char('V'), CTRL), Some(PasteImage));
        assert_eq!(
            resolve(Chat, KeyCode::Char('V'), CTRL | SHIFT),
            Some(PasteImage)
        );
    }

    /// Shift+Enter and plain Enter are different actions.
    #[test]
    fn shift_enter_is_newline() {
        assert_eq!(resolve(Chat, KeyCode::Enter, NONE), Some(Submit));
        assert_eq!(resolve(Chat, KeyCode::Enter, SHIFT), Some(InsertNewline));
    }
}
