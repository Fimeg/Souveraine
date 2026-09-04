#![allow(dead_code)] // WIP scaffolding not yet wired
//! Agent portrait widget — loads and displays an agent's portrait image.
//!
//! Tries to find a portrait file at the standard location
//! (`~/.souveraine/agents/{id}/memory/assets/portrait.{png,jpg,jpeg}`).
//! Falls back to ASCII diamond art when no image is found or image
//! support isn't available.
//!
//! Supports expression switching via [`ExpressionCache`] — when an
//! agent has expression frames under `memory/assets/expressions/`,
//! the portrait swaps images based on the agent's current presence state.

use tuie::prelude::*;

use super::expression_cache::{ExpressionCache, ExpressionKey};
use crate::ui::chat::ChatPalette;
use crate::ui::presence::{Eye, Posture};
use crate::ui::theme;

pub struct HudStats {
    pub age: String,
    pub commits: u32,
    pub uptime: String,
    pub instances: u32,
    pub mem_count: u32,
    pub mood: String,
}

pub struct Portrait {
    inner: Box<dyn Widget>,
    agent_id: Option<String>,
    current_key: Option<ExpressionKey>,
    name: String,
    color: Color,
    has_hud: bool,
}

impl DelegateWidget for Portrait {
    tuie::delegate_widget!(inner);
    fn override_is_focusable(&self) -> bool {
        false
    }
}

impl Portrait {
    pub fn new(agent_id: Option<&str>, name: &str, color: Color) -> Box<Self> {
        let inner: Box<dyn Widget> = match agent_id.and_then(load_portrait_image) {
            Some(source) => {
                let mut img = Image::new(source);
                img.set_fill(true);
                img.flex(1).x_align(FlexAlign::Center)
            }
            None => ascii_portrait(name, color),
        };

        Box::new(Self {
            inner,
            agent_id: agent_id.map(|s| s.to_string()),
            current_key: None,
            name: name.to_string(),
            color,
            has_hud: false,
        })
    }

    pub fn update_expression(
        &mut self,
        cache: &mut ExpressionCache,
        posture: Posture,
        eye: Eye,
        breath: bool,
    ) {
        let agent_id = match &self.agent_id {
            Some(id) => id.clone(),
            None => return,
        };

        let assets_dir = match dirs::home_dir() {
            Some(home) => home
                .join(".souveraine")
                .join("agents")
                .join(&agent_id)
                .join("memory")
                .join("assets"),
            None => return,
        };

        let key = ExpressionKey {
            posture,
            eye,
            breath,
            outfit: None,
        };

        if self.current_key.as_ref() == Some(&key) {
            return;
        }

        if let Some(source) = cache.resolve(&agent_id, key.clone(), &assets_dir) {
            let mut img = Image::new(source);
            img.set_fill(true);
            let img = img.flex(1).x_align(FlexAlign::Center);
            self.inner = img;
            self.current_key = Some(key);
        }
    }

    pub fn set_hud(&mut self, stats: &HudStats, palette: &ChatPalette) {
        let primary = theme::to_tuie_color(palette.agent_primary);
        let dim = theme::to_tuie_color(palette.agent_dim);

        let mut content = StyledString::new();
        content.push_span(StyledStr::new(" AGE  ").fg(dim));
        content.push_span(StyledStr::new(&stats.age).fg(primary));
        content.push_str("\n");
        content.push_span(StyledStr::new(" STATS").fg(dim));
        content.push_str("  ");
        content.push_span(StyledStr::new(&format!("C {}", stats.commits)).fg(Color::GREEN));
        content.push_str("  ");
        content.push_span(StyledStr::new(&format!("U {}", stats.uptime)).fg(Color::GREEN));
        content.push_str("  ");
        content.push_span(StyledStr::new(&format!("I {}", stats.instances)).fg(Color::BLUE));
        content.push_str("  ");
        content.push_span(StyledStr::new(&format!("M {}", stats.mem_count)).fg(Color::GREEN));
        content.push_str("\n");
        content.push_span(StyledStr::new(" MOOD ").fg(dim));
        content.push_span(StyledStr::new(&stats.mood).fg(primary));

        self.has_hud = true;
    }
}

fn load_portrait_image(agent_id: &str) -> Option<ImageSource> {
    let assets_dir = dirs::home_dir()?
        .join(".souveraine")
        .join("agents")
        .join(agent_id)
        .join("memory")
        .join("assets");

    let path = ["portrait.png", "portrait.jpg", "portrait.jpeg"]
        .iter()
        .map(|s| assets_dir.join(s))
        .find(|p| p.exists())?;

    let bytes = std::fs::read(&path).ok()?;
    ImageSource::from_encoded(bytes).ok()
}

fn ascii_portrait(name: &str, color: Color) -> Box<dyn Widget> {
    let mut content = StyledString::new();
    content.push_str("\n");
    content.push_span(StyledStr::new("       ◈\n").fg(color));
    content.push_span(StyledStr::new("    ◈     ◈\n").fg(color));
    content.push_span(StyledStr::new("   ◈  ◈  ◈\n").fg(color));
    content.push_span(StyledStr::new("     ◈◈\n").fg(color));
    content.push_span(StyledStr::new("   ◈    ◈\n").fg(color));
    content.push_str("\n");
    content.push_span(StyledStr::new(&format!("    {name}\n")).bold().fg(color));
    content.push_str("\n");

    Text::new().content(content).center()
}
