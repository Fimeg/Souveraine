//! Expression-driven portrait system — per-agent animated expression frames.
//!
//! Each agent can have a set of expression images under
//! `memory/assets/expressions/` following the naming convention:
//!
//! ```text
//! expressions/
//!   idle.png                  ← Posture::Idle, Eye::Open
//!   idle-blink.png            ← Posture::Idle, Eye::Blinking
//!   idle-interim.png          ← Posture::Idle, breath frame
//!   processing.png            ← Posture::Processing, Eye::Open
//!   processing-blink.png
//!   processing-interim.png
//!   affectionate.png          ← Posture::Affectionate
//!   affectionate-blink.png
//!   affectionate-interim.png
//!   straining.png             ← Posture::Straining
//!   straining-blink.png
//!   straining-interim.png
//!   yawning.png               ← Posture::Yawning
//!   yawning-blink.png
//!   yawning-interim.png
//! ```
//!
//! Frame types:
//! - `{posture}.png` — base expression (steady state, open eyes).
//! - `{posture}-blink.png` — blink frame (briefly replaces base).
//! - `{posture}-interim.png` — breath frame (2-second periodic swap).
//!
//! Fallback chain per frame: exact expression → base posture (open eyes,
//! no breath) → `portrait.png` in assets root → half-block silhouette.
//!
//! The fallback-to-base-posture means an agent can ship just `idle.png`,
//! `idle-blink.png`, and `affectionate.png` — every other state degrades
//! gracefully without missing-file errors or broken renders.

use std::collections::HashMap;

use ratatui_image::{picker::Picker, protocol::StatefulProtocol};

use crate::ui::presence::{Eye, Posture, Presence};

/// Composite key that selects which expression image to render.
///
/// Every combination of posture × eye × breath is representable, but only
/// a subset will have actual files on disk — the [`ExpressionCache::resolve`]
/// method walks a fallback chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExpressionKey {
    pub posture: Posture,
    pub eye: Eye,
    /// `true` when the presence breath timer is in its interim frame.
    pub breath: bool,
}

impl ExpressionKey {
    /// Build the key from the current Presence state.
    pub fn from_presence(p: &Presence) -> Self {
        Self {
            posture: p.posture,
            eye: p.eye,
            breath: p.is_breathing,
        }
    }

    /// Filename in the `expressions/` directory, e.g. `"idle-blink.png"`.
    pub fn filename(&self) -> String {
        let posture = match self.posture {
            Posture::Idle => "idle",
            Posture::Processing => "processing",
            Posture::Affectionate => "affectionate",
            Posture::Straining => "straining",
            Posture::Yawning => "yawning",
        };
        match (self.eye, self.breath) {
            (Eye::Blinking, _) => format!("{}-blink.png", posture),
            (_, true) => format!("{}-interim.png", posture),
            _ => format!("{}.png", posture),
        }
    }
}

/// Per-agent expression image cache.
///
/// Protocol lifetimes mirror the Godot approach: each expression frame is
/// loaded once and held in memory for the session. The `preload_all` method
/// warms the cache when entering the Presence screen; `resolve` lazily loads
/// on first access for any combo requested at render time.
pub struct ExpressionCache {
    /// Keyed by agent_id → expression key → loaded protocol.
    inner: HashMap<String, HashMap<ExpressionKey, StatefulProtocol>>,
}

impl ExpressionCache {
    pub fn new() -> Self {
        Self { inner: HashMap::new() }
    }

    /// Ensure an expression key is loaded into the cache.
    /// Returns `true` if the file existed and was decoded, `false` otherwise.
    fn ensure_loaded(
        &mut self,
        agent_id: &str,
        key: ExpressionKey,
        picker: &Picker,
        expressions_dir: &std::path::Path,
    ) -> bool {
        let agent_map = self.inner.entry(agent_id.to_string()).or_default();
        if agent_map.contains_key(&key) {
            return true;
        }

        let path = expressions_dir.join(key.filename());
        if !path.exists() {
            return false;
        }

        let dyn_img = match image::ImageReader::open(&path) {
            Ok(reader) => match reader.decode() {
                Ok(img) => img,
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "expression decode failed");
                    return false;
                }
            },
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "expression open failed");
                return false;
            }
        };

        let proto = picker.new_resize_protocol(dyn_img);
        tracing::info!(agent = %agent_id, key = ?key, "expression loaded");
        agent_map.insert(key, proto);
        true
    }

    /// Resolve an expression for the given state, walking the fallback chain:
    ///
    /// 1. Exact match (e.g. `idle-blink.png`)
    /// 2. Base posture, open eyes, no breath (e.g. `idle.png`)
    /// 3. `None` — caller falls through to `portrait.png` → silhouette
    pub fn resolve(
        &mut self,
        agent_id: &str,
        key: ExpressionKey,
        picker: &Picker,
        assets_dir: &std::path::Path,
    ) -> Option<&mut StatefulProtocol> {
        let expressions_dir = assets_dir.join("expressions");
        if !expressions_dir.is_dir() {
            return None;
        }

        // 1. Try exact match.
        self.ensure_loaded(agent_id, key, picker, &expressions_dir);

        // 2. Try base posture (open eyes, no breath) if different.
        let base = ExpressionKey {
            eye: Eye::Open,
            breath: false,
            ..key
        };
        if base != key {
            self.ensure_loaded(agent_id, base, picker, &expressions_dir);
        }

        // Now pull from cache — try exact first, then base posture.
        let inner = self.inner.get_mut(agent_id)?;
        if inner.contains_key(&key) {
            inner.get_mut(&key)
        } else {
            inner.get_mut(&base)
        }
    }

    /// Preload every expression file that exists for this agent.
    /// Call once when entering the Presence screen so transitions are
    /// instant rather than hitting disk on every blink.
    pub fn preload_all(
        &mut self,
        agent_id: &str,
        picker: &Picker,
        assets_dir: &std::path::Path,
    ) {
        let expressions_dir = assets_dir.join("expressions");
        if !expressions_dir.is_dir() {
            return;
        }

        for posture in &[
            Posture::Idle,
            Posture::Processing,
            Posture::Affectionate,
            Posture::Straining,
            Posture::Yawning,
        ] {
            for eye in &[Eye::Open, Eye::Blinking] {
                for breath in &[false, true] {
                    let key = ExpressionKey {
                        posture: *posture,
                        eye: *eye,
                        breath: *breath,
                    };
                    if expressions_dir.join(key.filename()).exists() {
                        let _ = self.ensure_loaded(agent_id, key, picker, &expressions_dir);
                    }
                }
            }
        }
        tracing::info!(
            agent = %agent_id,
            loaded = self.inner.get(agent_id).map(|m| m.len()).unwrap_or(0),
            "expressions preloaded",
        );
    }

    /// Remove an agent from the cache (e.g. when switching agents or
    /// refreshing from disk).
    pub fn remove_agent(&mut self, agent_id: &str) {
        self.inner.remove(agent_id);
    }

    /// Count loaded expression frames for a given agent.
    /// Returns `None` if the agent has no entries at all.
    pub fn count_for(&self, agent_id: &str) -> Option<usize> {
        self.inner.get(agent_id).map(|m| m.len())
    }

    /// Total cached protocol count across all agents.
    pub fn total_count(&self) -> usize {
        self.inner.values().map(|m| m.len()).sum()
    }
}

impl Default for ExpressionCache {
    fn default() -> Self {
        Self::new()
    }
}
