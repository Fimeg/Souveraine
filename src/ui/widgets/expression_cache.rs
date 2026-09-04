#![allow(dead_code)] // WIP scaffolding not yet wired
use std::collections::HashMap;
use std::path::Path;

use tuie::prelude::*;

use crate::ui::presence::{Eye, Posture};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExpressionKey {
    pub posture: Posture,
    pub eye: Eye,
    pub breath: bool,
    pub outfit: Option<String>,
}

impl ExpressionKey {
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

pub struct ExpressionCache {
    inner: HashMap<String, HashMap<ExpressionKey, ImageSource>>,
}

impl ExpressionCache {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    fn ensure_loaded(
        &mut self,
        agent_id: &str,
        key: &ExpressionKey,
        expressions_dir: &Path,
    ) -> bool {
        let agent_map = self.inner.entry(agent_id.to_string()).or_default();
        if agent_map.contains_key(key) {
            return true;
        }

        if let Some(outfit) = &key.outfit {
            let outfit_dir = expressions_dir.join(outfit);
            if outfit_dir.is_dir() {
                let path = outfit_dir.join(key.filename());
                if path.exists() {
                    return Self::load_path(agent_map, key, &path);
                }
            }
        }

        let path = expressions_dir.join(key.filename());
        if path.exists() {
            return Self::load_path(agent_map, key, &path);
        }

        false
    }

    fn load_path(
        agent_map: &mut HashMap<ExpressionKey, ImageSource>,
        key: &ExpressionKey,
        path: &Path,
    ) -> bool {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "expression read failed");
                return false;
            }
        };
        match ImageSource::from_encoded(bytes) {
            Ok(source) => {
                tracing::info!(key = ?key, path = %path.display(), "expression loaded");
                agent_map.insert(key.clone(), source);
                true
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "expression decode failed");
                false
            }
        }
    }

    pub fn resolve(
        &mut self,
        agent_id: &str,
        key: ExpressionKey,
        assets_dir: &Path,
    ) -> Option<ImageSource> {
        let expressions_dir = assets_dir.join("expressions");
        if !expressions_dir.is_dir() {
            return None;
        }

        let base = ExpressionKey {
            eye: Eye::Open,
            breath: false,
            outfit: key.outfit.clone(),
            ..key.clone()
        };
        let root_key = ExpressionKey {
            outfit: None,
            ..key.clone()
        };
        let root_base = ExpressionKey {
            outfit: None,
            eye: Eye::Open,
            breath: false,
            ..key.clone()
        };

        if key.outfit.is_some() {
            self.ensure_loaded(agent_id, &key, &expressions_dir);
        }
        if key.outfit.is_some() && base != key {
            self.ensure_loaded(agent_id, &base, &expressions_dir);
        }
        self.ensure_loaded(agent_id, &root_key, &expressions_dir);
        if root_base != root_key {
            self.ensure_loaded(agent_id, &root_base, &expressions_dir);
        }

        let inner = self.inner.get(agent_id)?;
        for candidate in [&key, &base, &root_key, &root_base] {
            if let Some(source) = inner.get(candidate) {
                return Some(source.clone());
            }
        }
        None
    }

    pub fn preload_all(&mut self, agent_id: &str, assets_dir: &Path) {
        let expressions_dir = assets_dir.join("expressions");
        if !expressions_dir.is_dir() {
            return;
        }
        self.preload_for_dir(agent_id, &expressions_dir, None);

        if let Ok(entries) = std::fs::read_dir(&expressions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        self.preload_for_dir(agent_id, &path, Some(name.to_string()));
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

    fn preload_for_dir(&mut self, agent_id: &str, dir: &Path, outfit: Option<String>) {
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
                        Self::load_path(agent_map, &key, &path);
                    }
                }
            }
        }
    }

    pub fn remove_agent(&mut self, agent_id: &str) {
        self.inner.remove(agent_id);
    }
}

impl Default for ExpressionCache {
    fn default() -> Self {
        Self::new()
    }
}
