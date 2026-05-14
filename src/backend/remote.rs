//! HTTP+SSE client for `souveraine server`.

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use serde::Deserialize;
use serde_json::Value;
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
    pub fn new(base_url: impl Into<String>) -> Self {
        let mut url = base_url.into();
        while url.ends_with('/') {
            url.pop();
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(2))
            .build()
            .expect("reqwest client");
        Self { base_url: url, client, token: None }
    }

    /// Create a RemoteBackend that sends bearer tokens for the given agent_id.
    /// Loads the token from the standard on-disk location.
    pub fn with_agent(base_url: impl Into<String>, agent_id: &str) -> Self {
        let mut this = Self::new(base_url);
        this.load_token(agent_id);
        this
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

    async fn new_conversation(&self, agent_id: &str) -> Result<String> {
        // Remote: same as ensure_conversation for now — server always creates fresh
        self.ensure_conversation(agent_id).await
    }

    async fn list_conversations(&self, _agent_id: &str) -> Result<Vec<ConversationInfo>> {
        // TODO: implement remote conversation listing via GET /v1/agents/:id/conversations
        Ok(Vec::new())
    }

    async fn load_conversation(
        &self,
        _conversation_id: &str,
    ) -> Result<Vec<crate::core::session::ConversationMessage>> {
        // TODO: implement remote conversation loading
        anyhow::bail!("Remote conversation loading not yet implemented")
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
    let mut event_type: Option<String> = None;
    let mut data = String::new();
    for line in s.split('\n') {
        let line = line.trim_end_matches('\r');
        if let Some(v) = line.strip_prefix("event:") {
            event_type = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(v.trim_start());
        }
    }
    let event_type = event_type?;
    let json: Value = serde_json::from_str(&data).ok()?;
    map_event(&event_type, &json)
}

fn map_event(event_type: &str, v: &Value) -> Option<BackendEvent> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    Some(match event_type {
        "message" => BackendEvent::Token(s("content")?),
        "reasoning" => BackendEvent::Reasoning(s("content")?),
        "souveraine_surfacing" => BackendEvent::Surfacing {
            source: s("source").unwrap_or_default(),
            content: s("content").unwrap_or_default(),
            priority: s("priority").unwrap_or_default(),
        },
        "souveraine_reflection" => BackendEvent::Reflection(s("content").unwrap_or_default()),
        "souveraine_archivist" => BackendEvent::Archivist {
            synthesis: s("synthesis").unwrap_or_default(),
            pressure: v.get("pressure").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        },
        "ping" => BackendEvent::Keepalive,
        _ => return None,
    })
}
