use std::cell::RefCell;
use std::time::Instant;

use anyhow::Result;
use futures::StreamExt;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tokio::sync::mpsc;

use crate::backend::{Backend, BackendEvent};
use crate::bridge::bifrost::BifrostClient;
use crate::core::config::ConsciousnessConfig;

use super::{
    BtwForkEvent, BtwState, ChatMessage, ChatMode, ChatState, TurnPhase,
};

impl ChatState {
    pub const HELP_TEXT: &'static str = "Available commands:
  /help              Show this help
  /clear             Clear chat history
  /new               Start a new conversation
  /resume            List and switch conversations
  /convos            Alias for /resume
  /model             List available models
  /model <name>      Set the active model
  /btw <text>        Interject — delivered to her next LLM round
  /code              Shift to code posture (tools expanded, ≡ prompt)
  /chat              Shift to conversation posture (tools collapsed)
  /outfit <name>     Change agent's outfit (empty to reset)
  !<command>         Run a shell command (Linux/macOS)

Esc during a turn shows the raise-hand dialog (signal, not kill — she sees *[raised hand]*).
You can also type while she works — Enter raises your hand (she sees it next round).
Tab toggles the cockpit pane. `t` (on empty input) toggles tool expansion.";

    pub fn submit(&mut self) -> bool {
        if self.input.trim().is_empty() {
            return false;
        }

        let trimmed = self.input.trim().to_string();
        self.input.clear();

        if let Some(rest) = trimmed.strip_prefix("/btw ") {
            let question = rest.trim().to_string();
            if !question.is_empty() && !self.btw_active() {
                self.start_btw_fork(question);
            }
            return true;
        }

        if trimmed.starts_with('/') {
            return self.handle_slash_command(&trimmed);
        }

        if trimmed.starts_with('!') {
            let cmd = trimmed[1..].trim();
            if !cmd.is_empty() {
                self.handle_bang_command(cmd);
            }
            return true;
        }

        if self.busy {
            self.enqueue_interjection(trimmed);
            return true;
        }

        let text = trimmed;
        self.messages.push(ChatMessage::User { text: text.clone(), ts: Instant::now() });
        self.spawn_turn(text);
        true
    }

    pub fn spawn_turn(&mut self, text: String) {
        self.busy = true;
        self.tool_calls_this_turn = 0;
        self.phase = TurnPhase::Thinking;
        self.turn_started = Some(Instant::now());
        self.last_event_at = Instant::now();

        let (tx, rx) = mpsc::channel::<BackendEvent>(64);
        self.turn_rx = Some(rx);

        let cancel = CancellationToken::new();
        self.cancel_token = Some(cancel.clone());

        let backend = self.backend.clone();
        let conv_id = self.conversation_id.clone();
        let interject_queue = self.pending_interjections.clone();
        tokio::spawn(async move {
            match backend.send_with_signals(&conv_id, &text, cancel, interject_queue).await {
                Ok(mut stream) => {
                    while let Some(ev) = stream.next().await {
                        match ev {
                            Ok(e) => {
                                if tx.send(e).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                let _ = tx
                                    .send(BackendEvent::Token(format!("\n[error] {}\n", err)))
                                    .await;
                                break;
                            }
                        }
                    }
                }
                Err(err) => {
                    let _ = tx
                        .send(BackendEvent::Token(format!("\n[connect error] {}\n", err)))
                        .await;
                }
            }
            let _ = tx.send(BackendEvent::Done).await;
        });
    }

    pub fn cancel_active_turn(&self) {
        if let Some(token) = &self.cancel_token {
            if !token.is_cancelled() {
                token.cancel();
            }
        }
    }

    pub fn deliver_pending_interjections(&mut self) {
        if self.busy {
            return;
        }
        let pending: Vec<String> = self
            .pending_interjections
            .lock()
            .ok()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default();
        if pending.is_empty() {
            return;
        }
        for msg in &mut self.messages {
            if let ChatMessage::Interjection { delivered, .. } = msg {
                *delivered = true;
            }
        }
        self.spawn_turn(pending.join("\n"));
    }

    fn handle_slash_command(&mut self, input: &str) -> bool {
        let trimmed = input.trim();

        if trimmed == "/help" {
            self.system_message(Self::HELP_TEXT.to_string());
            return true;
        }

        if trimmed == "/clear" {
            self.messages.clear();
            self.system_message("Chat cleared.".to_string());
            return true;
        }

        if trimmed == "/new" {
            self.handle_new_conversation();
            return true;
        }

        if trimmed == "/resume" || trimmed == "/convos" {
            self.handle_list_conversations();
            return true;
        }

        if trimmed.starts_with("/resume ") {
            let conv_id = trimmed.strip_prefix("/resume ").unwrap().trim();
            if !conv_id.is_empty() {
                self.handle_switch_conversation(conv_id.to_string());
            }
            return true;
        }

        if trimmed.starts_with("/model") {
            return self.handle_model_command(trimmed);
        }

        if trimmed == "/code" {
            self.render_mode = ChatMode::Code;
            self.system_message(
                "Code posture. Tool gestures expand; the prompt becomes ≡. Same conversation."
                    .to_string(),
            );
            return true;
        }

        if trimmed == "/chat" {
            self.render_mode = ChatMode::Conversation;
            self.system_message(
                "Conversation posture. Tool gestures collapse; the prompt returns to ›.".to_string(),
            );
            return true;
        }

        if trimmed == "/outfit" || trimmed.starts_with("/outfit ") {
            let name = trimmed.strip_prefix("/outfit")
                .map(|s| s.trim())
                .unwrap_or("")
                .to_string();
            self.pending_consciousness.push(BackendEvent::Outfit(name.clone()));
            if name.is_empty() {
                self.system_message("Returned to default appearance.".to_string());
            } else {
                self.system_message(format!("Changed to **{name}** outfit."));
            }
            return true;
        }

        if trimmed == "/btw" {
            self.system_message(
                "Usage: /btw <text> — fork a side-quest conversation.\nThe main chat carries on; the fork streams into a floating pane. Press j to jump to the fork, Esc to dismiss."
                    .to_string(),
            );
            return true;
        }

        let cmd = trimmed.split_whitespace().next().unwrap_or(trimmed);
        self.system_message(format!(
            "Unknown command: {}\nType /help for available commands.",
            cmd
        ));
        true
    }

    fn handle_new_conversation(&mut self) {
        let backend = self.backend.clone();
        let agent_id = self.agent_id.clone();
        let (tx, rx) = oneshot::channel();

        tokio::spawn(async move {
            let result = backend.new_conversation(&agent_id).await;
            let _ = tx.send(result);
        });

        self.system_message("Creating new conversation...".to_string());
        self.new_conv_rx = Some(rx);
    }

    pub fn offer_resume_or_new(&mut self) {
        let backend = self.backend.clone();
        let agent_id = self.agent_id.clone();
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let result = backend.list_conversations(&agent_id).await;
            let _ = tx.send(result);
        });
        self.resume_offer = true;
        self.convos_rx = Some(rx);
    }

    fn handle_list_conversations(&mut self) {
        let backend = self.backend.clone();
        let agent_id = self.agent_id.clone();
        let (tx, rx) = oneshot::channel();

        tokio::spawn(async move {
            let result = backend.list_conversations(&agent_id).await;
            let _ = tx.send(result);
        });

        self.system_message("Loading conversations...".to_string());
        self.convos_rx = Some(rx);
    }

    pub fn handle_switch_conversation(&mut self, conversation_id: String) {
        if self.busy {
            self.raise_hand();
            self.system_message("Signalled active turn — switching when it finalizes.".to_string());
            self.switch_pending = Some(conversation_id);
            return;
        }
        self.initiate_switch_load(conversation_id);
    }

    pub fn initiate_switch_load(&mut self, conversation_id: String) {
        let backend = self.backend.clone();
        let conv_id = conversation_id.clone();
        let (tx, rx) = oneshot::channel();

        tokio::spawn(async move {
            let result = backend.load_conversation(&conv_id).await;
            let _ = tx.send(result.map(|msgs| (conv_id, msgs)));
        });

        self.system_message(format!("Switching to {}...", conversation_id));
        self.switch_rx = Some(rx);
    }

    fn handle_model_command(&mut self, input: &str) -> bool {
        let rest = input.strip_prefix("/model").unwrap_or("").trim();

        if !rest.is_empty() && !rest.starts_with('-') {
            let model_name = rest.to_string();
            let cfg_path = std::env::current_dir()
                .map(|d| d.join("souveraine.toml"))
                .unwrap_or_else(|_| std::path::PathBuf::from("souveraine.toml"));

            match ConsciousnessConfig::load(&cfg_path) {
                Ok(mut cfg) => {
                    cfg.bifrost.primary_model = model_name.clone();
                    match cfg.save(&cfg_path) {
                        Ok(()) => self.system_message(format!("Set model to: {}", model_name)),
                        Err(e) => self.error_message(format!("Failed to save config: {}", e)),
                    }
                }
                Err(e) => self.error_message(format!("Failed to load config: {}", e)),
            }
            return true;
        }

        self.system_message("Fetching models from Bifrost…".to_string());

        let cfg_path = std::env::current_dir()
            .map(|d| d.join("souveraine.toml"))
            .unwrap_or_else(|_| std::path::PathBuf::from("souveraine.toml"));

        let (tx, rx) = oneshot::channel();
        self.model_rx = Some(rx);

        tokio::spawn(async move {
            let result = match ConsciousnessConfig::load(&cfg_path) {
                Ok(cfg) => {
                    let bifrost = BifrostClient::new(
                        &cfg.bifrost.base_url,
                        &cfg.bifrost.api_key,
                        &cfg.bifrost.virtual_key,
                        &cfg.bifrost.primary_model,
                        cfg.bifrost.timeout_secs,
                    );
                    let bifrost_models = bifrost.list_models().await.unwrap_or_default();
                    let mut all_models = bifrost_models.clone();
                    for name in cfg.models.keys() {
                        if !all_models.contains(name) {
                            all_models.push(name.clone());
                        }
                    }
                    let mut text = format!(
                        "Selected: {}\nAvailable ({}):\n",
                        cfg.bifrost.primary_model,
                        all_models.len()
                    );
                    for m in &all_models {
                        let marker = if bifrost_models.contains(&m) { "⚡" } else { "⚙" };
                        text.push_str(&format!("  {} {}\n", marker, m));
                    }
                    text
                }
                Err(e) => format!("✕ Failed to load config: {}", e),
            };
            let _ = tx.send(result);
        });

        true
    }

    fn handle_bang_command(&mut self, cmd: &str) {
        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(cmd)
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let mut result = String::new();
                if !stdout.is_empty() {
                    result.push_str(stdout.trim());
                }
                if !stderr.is_empty() {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(stderr.trim());
                }
                if result.is_empty() {
                    result = format!("[exit code {}]", out.status.code().unwrap_or(-1));
                }
                self.system_message(format!("$ {}\n{}", cmd, result));
            }
            Err(e) => {
                self.error_message(format!("Shell command failed: {}", e));
            }
        }
    }

    pub fn system_message(&mut self, text: String) {
        self.messages.push(ChatMessage::System {
            text,
            ts: Instant::now(),
        });
    }

    pub fn error_message(&mut self, text: String) {
        self.messages.push(ChatMessage::System {
            text: format!("✕ {}", text),
            ts: Instant::now(),
        });
    }
}
