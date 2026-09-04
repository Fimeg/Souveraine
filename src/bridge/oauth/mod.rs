//! Riding the Codex CLI's ChatGPT OAuth login.
//!
//! Souveraine does not run its own ChatGPT login flow — it reads the token the
//! Codex CLI already minted (`~/.codex/auth.json`), refreshes it against
//! OpenAI's token endpoint when it ages out, and writes the rotated token back
//! so the two stay in sync. The same "ride the CLI" move OpenClaw makes.

pub mod catalog;
pub mod codex_creds;
pub mod refresh;
