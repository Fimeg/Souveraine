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
/// Every combination of posture × eye × breath × outfit is representable,
/// but only a subset will have actual files on disk — the
/// [`ExpressionCache::resolve`] method walks a fallback chain.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExpressionKey {
    pub posture: Posture,
    pub eye: Eye,
    /// `true` when the presence breath timer is in its interim frame.
    pub breath: bool,
    /// Outfit subdirectory under `expressions/`, or `None` for root-level
    /// (default) expressions. Added in the outfits feature.
    pub outfit: Option<String>,
}

impl ExpressionKey {
    /// Build the key from the current Presence state.
    pub fn from_presence(p: &Presence) -> Self {
        Self {
            posture: p.posture,
            eye: p.eye,
            breath: p.is_breathing,
            outfit: p.outfit.clone(),
        }
    }

    /// Filename in the `expressions/` directory, e.g. `"idle-blink.png"`.
    pub fn filename(&self) -> String {
        let posture = match self.posture {
            Posture::Idle => "idle",
            Posture::Alert => "alert",
            Posture::Thinking => "thinking",
            Posture::Processing => "processing",
            Posture::Affectionate => "affectionate",
            Posture::Straining => "straining",
            Posture::Yawning => "yawning",
            Posture::Listening => "listening",
            Posture::Speaking => "speaking",
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

    /// Ensure an expression key is loaded into the cache, trying the outfit
    /// subdirectory first then the root expressions directory.
    /// Returns `true` if the file existed and was decoded, `false` otherwise.
    fn ensure_loaded(
        &mut self,
        agent_id: &str,
        key: &ExpressionKey,
        picker: &Picker,
        expressions_dir: &std::path::Path,
    ) -> bool {
        let agent_map = self.inner.entry(agent_id.to_string()).or_default();
        if agent_map.contains_key(key) {
            return true;
        }

        // Try outfit subdirectory first.
        if let Some(outfit) = &key.outfit {
            let outfit_dir = expressions_dir.join(outfit);
            if outfit_dir.is_dir() {
                let path = outfit_dir.join(key.filename());
                if path.exists() {
                    return Self::load_path(agent_id, agent_map, key, picker, &path);
                }
            }
        }

        // Fall back to root expressions directory.
        let path = expressions_dir.join(key.filename());
        if path.exists() {
            return Self::load_path(agent_id, agent_map, key, picker, &path);
        }

        false
    }

    /// Decode an image file and insert it into the cache.
    fn load_path(
        agent_id: &str,
        agent_map: &mut HashMap<ExpressionKey, StatefulProtocol>,
        key: &ExpressionKey,
        picker: &Picker,
        path: &std::path::Path,
    ) -> bool {
        let dyn_img = match image::ImageReader::open(path) {
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
        tracing::info!(agent = %agent_id, key = ?key, path = %path.display(), "expression loaded");
        agent_map.insert(key.clone(), proto);
        true
    }

    /// Resolve an expression for the given state, walking the fallback chain:
    ///
    /// 1. Outfit + exact match (e.g. `casual/idle-blink.png`)
    /// 2. Outfit + base posture (e.g. `casual/idle.png`)
    /// 3. Root + exact match (e.g. `idle-blink.png`)
    /// 4. Root + base posture (e.g. `idle.png`)
    /// 5. `None` — caller falls through to `portrait.png` → silhouette
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

        // Build the base posture key (open eyes, no breath, same outfit).
        let base = ExpressionKey {
            eye: Eye::Open,
            breath: false,
            outfit: key.outfit.clone(),
            ..key.clone()
        };
        // Build the no-outfit variants for fallback tiers 3-4.
        let root_key = ExpressionKey { outfit: None, ..key.clone() };
        let root_base = ExpressionKey { outfit: None, eye: Eye::Open, breath: false, ..key.clone() };

        // 1. Outfit + exact.
        if key.outfit.is_some() {
            self.ensure_loaded(agent_id, &key, picker, &expressions_dir);
        }
        // 2. Outfit + base posture (if different).
        if key.outfit.is_some() && base != key {
            self.ensure_loaded(agent_id, &base, picker, &expressions_dir);
        }
        // 3. Root + exact.
        self.ensure_loaded(agent_id, &root_key, picker, &expressions_dir);
        // 4. Root + base posture (if different from exact).
        if root_base != root_key {
            self.ensure_loaded(agent_id, &root_base, picker, &expressions_dir);
        }

        // Pull from cache — try best match first.
        let inner = self.inner.get_mut(agent_id)?;
        for candidate in [&key, &base, &root_key, &root_base] {
            if inner.contains_key(candidate) {
                return inner.get_mut(candidate);
            }
        }
        None
    }

    /// Preload every expression file that exists for this agent, including
    /// all outfit subdirectories. Call once when entering the Presence screen.
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

        // Preload root-level expressions (default / no-outfit).
        self.preload_for_dir(agent_id, picker, &expressions_dir, None);

        // Scan for outfit subdirectories and preload those too.
        if let Ok(entries) = std::fs::read_dir(&expressions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        self.preload_for_dir(agent_id, picker, &path, Some(name.to_string()));
                    }
                }
            }
        }

        tracing::info!(
            agent = %agent_id,
            loaded = self.inner.get(agent_id).map(|m| m.len()).unwrap_or(0),
            "expressions preloaded (all outfits)",
        );
    }

    /// Preload expression frames from a single directory (root or outfit subdir).
    fn preload_for_dir(
        &mut self,
        agent_id: &str,
        picker: &Picker,
        dir: &std::path::Path,
        outfit: Option<String>,
    ) {
        for posture in &[
            Posture::Idle,
            Posture::Alert,
            Posture::Thinking,
            Posture::Processing,
            Posture::Affectionate,
            Posture::Straining,
            Posture::Yawning,
            Posture::Listening,
            Posture::Speaking,
        ] {
            for eye in &[Eye::Open, Eye::Blinking] {
                for breath in &[false, true] {
                    let key = ExpressionKey {
                        posture: *posture,
                        eye: *eye,
                        breath: *breath,
                        outfit: outfit.clone(),
                    };
                    if !dir.join(key.filename()).exists() {
                        continue;
                    }
                    let agent_map = self.inner.entry(agent_id.to_string()).or_default();
                    if !agent_map.contains_key(&key) {
                        let path = dir.join(key.filename());
                        Self::load_path(agent_id, agent_map, &key, picker, &path);
                    }
                }
            }
        }
    }

    /// Preload a specific outfit's expression frames. Called lazily when an
    /// outfit changes so the user doesn't wait for all outfits at startup.
    pub fn preload_outfit(
        &mut self,
        agent_id: &str,
        outfit_name: &str,
        picker: &Picker,
        assets_dir: &std::path::Path,
    ) {
        let expressions_dir = assets_dir.join("expressions");
        let outfit_dir = expressions_dir.join(outfit_name);
        if outfit_dir.is_dir() {
            self.preload_for_dir(agent_id, picker, &outfit_dir, Some(outfit_name.to_string()));
        }
    }

    /// Remove all cached frames for a specific outfit from the agent's cache.
    /// Keeps the default (no-outfit) frames and other outfits intact.
    pub fn remove_outfit(&mut self, agent_id: &str, outfit_name: &str) {
        if let Some(agent_map) = self.inner.get_mut(agent_id) {
            agent_map.retain(|key, _| key.outfit.as_deref() != Some(outfit_name));
        }
    }

    /// Remove an agent from the cache entirely.
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
