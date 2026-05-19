use super::{App, portrait_cover_crop};

impl App {
    pub(super) fn load_image_protocol(&mut self, path: &std::path::Path) {
        let Some(picker) = self.image_picker.as_ref() else { return };
        let dyn_img = match image::ImageReader::open(path) {
            Ok(reader) => match reader.decode() {
                Ok(img) => img,
                Err(e) => { tracing::warn!(path = %path.display(), error = %e, "image protocol decode failed"); return; }
            },
            Err(e) => { tracing::warn!(path = %path.display(), error = %e, "image protocol open failed"); return; }
        };
        let dyn_img = portrait_cover_crop(dyn_img, 2, 3);
        let font_size = picker.font_size();
        let w = dyn_img.width().div_ceil(font_size.width as u32) as u16;
        let h = dyn_img.height().div_ceil(font_size.height as u32) as u16;
        match picker.new_protocol(dyn_img, ratatui::layout::Size::new(w, h), ratatui_image::Resize::Fit(None)) {
            Ok(proto) => {
                tracing::info!(path = %path.display(), "image protocol loaded");
                self.image_protocol = Some(proto);
            }
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "image protocol creation failed"),
        }
    }

    /// Convenience wrapper: find the first existing portrait in
    /// `<memfs_root>/assets/` and load it as a terminal image protocol.
    pub(super) fn load_image_protocol_from_memfs(&mut self, memfs_root: &std::path::Path) {
        let path = ["portrait.png", "portrait.jpg", "portrait.jpeg"]
            .iter().map(|s| memfs_root.join("assets").join(s))
            .find(|p| p.exists());
        if let Some(path) = path {
            self.load_image_protocol(&path);
        }
    }

    /// Resolve an agent's portrait file under `~/.souveraine/agents/{id}/memory/assets/`.
    /// Returns the first existing path among png/jpg/jpeg variants.
    pub(super) fn agent_portrait_path(agent_id: &str) -> Option<std::path::PathBuf> {
        let base = Self::agent_assets_dir(agent_id)?;
        ["portrait.png", "portrait.jpg", "portrait.jpeg"]
            .iter()
            .map(|s| base.join(s))
            .find(|p| p.exists())
    }

    /// Resolve an agent's assets directory.
    /// Returns None if homedir can't be determined.
    pub(super) fn agent_assets_dir(agent_id: &str) -> Option<std::path::PathBuf> {
        let base = dirs::home_dir()?
            .join(".souveraine")
            .join("agents")
            .join(agent_id)
            .join("memory")
            .join("assets");
        if base.is_dir() { Some(base) } else { None }
    }

    /// Build a `StatefulProtocol` for a given agent and insert it into
    /// `card_images`. Stateful protocols are used here (not the eager
    /// `Protocol` used on Welcome) because each card lives in a different
    /// rect — the protocol re-encodes itself for whatever area the
    /// `StatefulImage` widget is rendered into, so a single load works
    /// across resizes and grid reflows.
    pub(super) fn load_card_image(&mut self, agent_id: &str, path: &std::path::Path) {
        let Some(picker) = self.image_picker.as_ref() else { return };
        let dyn_img = match image::ImageReader::open(path) {
            Ok(reader) => match reader.decode() {
                Ok(img) => img,
                Err(e) => { tracing::warn!(path = %path.display(), error = %e, "card image decode failed"); return; }
            },
            Err(e) => { tracing::warn!(path = %path.display(), error = %e, "card image open failed"); return; }
        };
        // Pull the `image` crate into scope for resize_to_fill on the render path.
        
        let dyn_img = portrait_cover_crop(dyn_img, 2, 3);
        let proto = picker.new_resize_protocol(dyn_img.clone());
        let agent_id = agent_id.to_string();
        tracing::info!(agent = %agent_id, path = %path.display(), "card image loaded");
        self.card_images.insert(agent_id.clone(), proto);
        self.raw_card_images.insert(agent_id, dyn_img);
    }

    /// Refresh the card-image cache to match `agent_cards`. Loads any
    /// missing portraits and drops entries for agents no longer present.
    pub(super) fn refresh_card_images(&mut self) {
        let ids: Vec<(String, Option<std::path::PathBuf>)> = self.agent_cards
            .iter()
            .map(|c| (c.id.clone(), Self::agent_portrait_path(&c.id)))
            .collect();
        let valid: std::collections::HashSet<String> = ids.iter().map(|(id, _)| id.clone()).collect();
        self.card_images.retain(|k, _| valid.contains(k));
        self.raw_card_images.retain(|k, _| valid.contains(k));
        self.cover_protocols.retain(|k, _| valid.contains(k));
        for (id, path) in ids {
            if self.card_images.contains_key(&id) { continue; }
            if let Some(path) = path {
                self.load_card_image(&id, &path);
            }
        }
    }

    /// Preload all expression frames for the currently active agent.
    /// Called when entering Presence mode so blink/breath transitions
    /// are instant rather than loading from disk on every animation tick.
    pub(super) async fn preload_agent_expressions(&mut self) {
        let Some(picker) = self.image_picker.as_ref() else { return };
        let name = self.presence.name.clone();
        let id = self.agent_id_by_name(&name)
            .or_else(|| self.agent_id_by_name(&self.agent_pref));
        let Some(id) = id else { return };
        let Some(assets_dir) = Self::agent_assets_dir(&id) else { return };
        self.expression_cache.preload_all(&id, picker, &assets_dir);
    }

}
