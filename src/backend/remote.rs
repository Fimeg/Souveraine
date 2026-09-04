//! HTTP+SSE client for `souveraine server`.

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{AgentInfo, Backend, BackendEvent, ConversationInfo};

#[derive(Clone)]
pub struct RemoteBackend {
    base_url: String,
    client: reqwest::Client,
    /// Optional bearer token. When set, sent as `Authorization: Bearer <token>`
    /// on all requests. Loaded from the per-agent token file at construction
    /// when the agent_id is known.
    token: Option<String>,
}

impl RemoteBackend {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let mut url = base_url.into();
        while url.ends_with('/') {
            url.pop();
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(2))
            .build()
            .context("building reqwest client")?;
        Ok(Self {
            base_url: url,
            client,
            token: None,
        })
    }

    /// Create a RemoteBackend that sends bearer tokens for the given agent_id.
    /// Loads the token from the standard on-disk location.
    pub fn with_agent(base_url: impl Into<String>, agent_id: &str) -> Result<Self> {
        let mut this = Self::new(base_url)?;
        this.load_token(agent_id);
        Ok(this)
    }

    /// Try to load the per-agent bearer token from disk. Silently leaves
    /// `token` as None if the token file doesn't exist or is unreadable —
    /// loopback bypass will handle the common case.
    fn load_token(&mut self, agent_id: &str) {
        let token_path = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".souveraine")
            .join("server")
            .join("agents")
            .join(agent_id)
            .join("api_token");
        if let Ok(content) = std::fs::read_to_string(&token_path) {
            self.token = Some(content.trim().to_string());
        }
    }

    /// Apply the Authorization header if a token is stored.
    fn auth_req(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref token) = self.token {
            req.bearer_auth(token)
        } else {
            req
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Watch a local cancel token and relay it as POST .../cancel.
    /// Esc in the TUI fires the same interrupt semantics as local mode.
    fn spawn_cancel_watch(
        &self,
        conversation_id: &str,
        cancel: tokio_util::sync::CancellationToken,
    ) {
        let client = self.client.clone();
        let url = self.url(&format!("/v1/conversations/{}/cancel", conversation_id));
        let token = self.token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = cancel.cancelled() => {
                    let mut req = client.post(url);
                    if let Some(t) = token {
                        req = req.bearer_auth(t);
                    }
                    let _ = req.send().await;
                }
                // ponytail: task self-reaps after an hour so uncancelled
                // turns don't accumulate watchers; scope to stream lifetime
                // if turns ever legitimately exceed this
                _ = tokio::time::sleep(Duration::from_secs(3600)) => {}
            }
        });
    }

    /// Relay locally-queued interjections to the server mid-turn.
    /// ponytail: 250ms polling pump bounded by cancel/1h; replace with a
    /// notify-driven channel if the queue ever grows a waker.
    fn spawn_interject_pump(
        &self,
        conversation_id: &str,
        interject: super::InterjectionQueue,
        cancel: tokio_util::sync::CancellationToken,
    ) {
        let client = self.client.clone();
        let url = self.url(&format!("/v1/conversations/{}/interject", conversation_id));
        let token = self.token.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3600);
            loop {
                let drained: Vec<String> = {
                    let mut q = interject.lock().unwrap();
                    q.drain(..).collect()
                };
                for text in drained {
                    let mut req = client.post(&url).json(&serde_json::json!({ "text": text }));
                    if let Some(ref t) = token {
                        req = req.bearer_auth(t);
                    }
                    let _ = req.send().await;
                }
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
            }
        });
    }
}

#[async_trait]
impl Backend for RemoteBackend {
    async fn health(&self) -> bool {
        match self
            .client
            .get(self.url("/health"))
            .timeout(Duration::from_millis(800))
            .send()
            .await
        {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }

    async fn server_status(&self) -> Option<crate::server::BootInfo> {
        self.client
            .get(self.url("/v1/server/status"))
            .timeout(Duration::from_millis(800))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .json()
            .await
            .ok()
    }

    async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        #[derive(Deserialize)]
        struct Wire {
            id: String,
            name: String,
            #[serde(default)]
            description: Option<String>,
        }
        let resp = self
            .client
            .get(self.url("/v1/agents"))
            .send()
            .await
            .context("GET /v1/agents")?
            .error_for_status()?;
        let agents: Vec<Wire> = resp.json().await?;
        Ok(agents
            .into_iter()
            .map(|a| AgentInfo {
                id: a.id,
                name: a.name,
                description: a.description,
            })
            .collect())
    }

    async fn update_agent_principal(
        &self,
        agent_id: &str,
        principal: crate::api::models::AgentPrincipalConfig,
    ) -> Result<()> {
        let req = self
            .client
            .patch(self.url(&format!("/v1/agents/{agent_id}")))
            .json(&serde_json::json!({ "principal": principal }));
        self.auth_req(req)
            .send()
            .await
            .context("PATCH /v1/agents/:id")?
            .error_for_status()?;
        Ok(())
    }

    async fn new_conversation(&self, agent_id: &str) -> Result<String> {
        // Remote: same as ensure_conversation for now — server always creates fresh
        self.ensure_conversation(agent_id).await
    }

    async fn list_conversations(&self, agent_id: &str) -> Result<Vec<ConversationInfo>> {
        #[derive(Deserialize)]
        struct Wire {
            id: String,
            #[serde(default)]
            agent_id: Option<String>,
            #[serde(default)]
            summary: Option<String>,
            #[serde(default)]
            message_count: Option<u32>,
            #[serde(default)]
            updated_at: Option<String>,
        }
        let url = format!("/v1/conversations?agent_id={}", agent_id);
        let resp = self
            .auth_req(self.client.get(self.url(&url)))
            .send()
            .await
            .context("GET /v1/conversations")?
            .error_for_status()?;
        let wires: Vec<Wire> = resp.json().await?;
        Ok(wires
            .into_iter()
            .map(|w| ConversationInfo {
                id: w.id,
                agent_id: w.agent_id.unwrap_or_default(),
                summary: w.summary,
                message_count: w.message_count.unwrap_or(0),
                updated_at: w.updated_at.unwrap_or_default(),
            })
            .collect())
    }

    async fn load_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<crate::core::session::ConversationMessage>> {
        let url = format!("/v1/conversations/{}/messages", conversation_id);
        let resp = self
            .auth_req(self.client.get(self.url(&url)))
            .send()
            .await
            .context("GET /v1/conversations/:id/messages")?
            .error_for_status()?;
        let messages: Vec<crate::core::session::ConversationMessage> = resp.json().await?;
        Ok(messages)
    }

    async fn fork_conversation(
        &self,
        _agent_id: &str,
        source_conversation_id: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct Wire {
            id: String,
        }
        let url = format!("/v1/conversations/{}/fork", source_conversation_id);
        let resp = self
            .auth_req(self.client.post(self.url(&url)))
            .send()
            .await
            .context("POST /v1/conversations/:id/fork")?
            .error_for_status()?;
        let conv: Wire = resp.json().await?;
        Ok(conv.id)
    }

    async fn ensure_conversation(&self, agent_id: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct Wire {
            id: String,
        }
        let body = serde_json::json!({ "agent_id": agent_id });
        let resp = self
            .client
            .post(self.url("/v1/conversations"))
            .json(&body)
            .send()
            .await
            .context("POST /v1/conversations")?
            .error_for_status()?;
        let conv: Wire = resp.json().await?;
        Ok(conv.id)
    }

    async fn send(
        &self,
        conversation_id: &str,
        text: &str,
    ) -> Result<BoxStream<'static, Result<BackendEvent>>> {
        let body = serde_json::json!({
            "messages": [{"role": "user", "content": text}],
            "stream": true,
        });
        let resp = self
            .auth_req(
                self.client
                    .post(self.url(&format!("/v1/conversations/{}/messages", conversation_id))),
            )
            // The client-wide 60s timeout covers the whole body — it would
            // sever any turn longer than a minute. Turns run long by design.
            .timeout(Duration::from_secs(3600))
            .json(&body)
            .send()
            .await
            .context("POST /v1/conversations/:id/messages")?
            .error_for_status()?;

        let (tx, rx) = mpsc::channel::<Result<BackendEvent>>(64);
        tokio::spawn(async move {
            let mut bytes_stream = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            while let Some(chunk) = bytes_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::Error::from(e))).await;
                        return;
                    }
                };
                buf.extend_from_slice(&chunk);
                while let Some(end) = find_frame_end(&buf) {
                    let frame: Vec<u8> = buf.drain(..end).collect();
                    // drop delimiter
                    if buf.starts_with(b"\r\n\r\n") {
                        buf.drain(..4);
                    } else if buf.starts_with(b"\n\n") {
                        buf.drain(..2);
                    } else {
                        // shouldn't happen given find_frame_end's contract
                        break;
                    }
                    if let Some(ev) = parse_frame(&frame) {
                        if tx.send(Ok(ev)).await.is_err() {
                            return;
                        }
                    }
                }
            }
            if !buf.is_empty() {
                if let Some(ev) = parse_frame(&buf) {
                    let _ = tx.send(Ok(ev)).await;
                }
            }
            let _ = tx.send(Ok(BackendEvent::Done)).await;
        });

        Ok(ReceiverStream::new(rx).boxed())
    }

    async fn send_with_cancel(
        &self,
        conversation_id: &str,
        text: &str,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<BoxStream<'static, Result<BackendEvent>>> {
        self.spawn_cancel_watch(conversation_id, cancel);
        self.send(conversation_id, text).await
    }

    async fn send_with_signals(
        &self,
        conversation_id: &str,
        text: &str,
        cancel: tokio_util::sync::CancellationToken,
        interject: super::InterjectionQueue,
    ) -> Result<BoxStream<'static, Result<BackendEvent>>> {
        self.spawn_cancel_watch(conversation_id, cancel.clone());
        self.spawn_interject_pump(conversation_id, interject, cancel);
        self.send(conversation_id, text).await
    }
}

/// Index of the start of the SSE frame delimiter (`\n\n` or `\r\n\r\n`).
fn find_frame_end(buf: &[u8]) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    let mut i = 0;
    while i + 1 < buf.len() {
        if i + 3 < buf.len()
            && buf[i] == b'\r'
            && buf[i + 1] == b'\n'
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some(i);
        }
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_frame(bytes: &[u8]) -> Option<BackendEvent> {
    let s = std::str::from_utf8(bytes).ok()?;
    let mut data = String::new();
    for line in s.split('\n') {
        let line = line.trim_end_matches('\r');
        if let Some(v) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(v.trim_start());
        }
    }
    if data.is_empty() {
        return None;
    }
    // The data payload is self-describing (message_type tag) — the SSE
    // event: line is redundant. Full round-trip through the wire mirror:
    // every event the server emits lands here as its BackendEvent self.
    let ev: crate::api::models::StreamEvent = serde_json::from_str(&data).ok()?;
    Some(ev.into())
}
