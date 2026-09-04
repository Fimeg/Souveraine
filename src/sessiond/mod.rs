//! souveraine-sessiond — the session authority daemon.
//!
//! The lock authority below the shell: takes ext-session-lock before the
//! shell exists at boot, hands off to the shell's rich lock surface without
//! ever passing through an unlocked instant, and takes the lock back the
//! moment the shell heartbeat drops. See
//! `SouveraineOS/docs/session-authority-boot-order.md` (Phase C) and
//! `SESSION-AUTHORITY-DOCTRINE.md` §10–§11.
//!
//! Compiled into the `souveraine-sessiond` bin via `#[path]` includes, the
//! same pattern as machined and secrets.

pub mod bearer;
pub mod device_state;
pub mod draw;
pub mod idle;
pub mod lock;
pub mod lockhint;
pub mod protocol;
pub mod server;
