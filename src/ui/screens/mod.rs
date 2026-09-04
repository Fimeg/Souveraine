//! tuie screen widgets — one per app screen.
//!
//! Each screen is a tuie Widget that composes custom widgets and tuie
//! primitives. The root App widget switches between screens via Stack
//! layer push/pop.

pub mod agents;
pub mod chat;
pub mod cron;
pub mod presence;
pub mod settings;
pub mod splash;
pub mod welcome;
