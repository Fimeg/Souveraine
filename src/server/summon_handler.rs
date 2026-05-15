//! SummonHandler — manages cross-instance Reach & Consult requests.
//!
//! Lives on each SouveraineServer. It serves two roles:
//!
//! **Outbound** (caller side): Reach/Consult tools submit a request here,
//! which registers it in `in_flight`, fires a `summon_request` SensorEvent
//! onto the EventBus (the federation bridge picks it up), and returns a
//! `request_id` immediately. The caller's turn continues.
//!
//! **Inbound** (receiver side): Listens on the bus for `summon_request`
//! events whose `target` matches this instance's seed_id, checks
//! `authorized-summoners.md`, and writes the request to the agent's
//! `intrusive.md` inbox. When the summoned agent responds, the handler
//! fires a `summon_response` event back through the bridge.
//!
//! Responses are never awaited — they surface in the caller's inbox
//! (intrusive for consult, pending for reach) on a later turn.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::core::identity::SeedId;
use crate::core::nervous::handler::TurnInjector;
use crate::core::nervous::{EventBus, SensorEvent};

const RESPONSE_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMeta {
    pub request_id: String,
    pub tool: String,           // "reach" or "consult"
    pub target_seed_id: String,
    pub reply_to: String,       // local seed_id
    pub issued_at: chrono::DateTime<Utc>,
    pub responded: bool,
}

pub struct SummonHandler {
    /// Outbound requests awaiting correlation.
    pub in_flight: DashMap<String, RequestMeta>,
    /// Inbound requests we've received: request_id → caller's seed_id, so a
    /// reply written to the outbox can be routed back to whoever asked.
    inbound: DashMap<String, String>,
    /// This instance's seed_id — used to filter inbound events.
    local_seed_id: String,
    /// The nervous system bus.
    event_bus: EventBus,
    /// Instance seed for signing outbound requests.
    seed: Arc<SeedId>,
    /// Base path for agent memory — used to access inbox files.
    souveraine_base: std::path::PathBuf,
    /// When true, an inbound summon wakes the target agent with a background
    /// turn rather than only landing in her inbox. Mirrors `auto_wake` in
    /// `FederationConfig` — opt-in, sovereign default off.
    auto_wake: bool,
    /// Set once, after the backend exists, by `set_injector`. Absent on the
    /// pure-server path (no nervous-system turn loop there) — auto-wake then
    /// degrades gracefully and the summon waits for the agent's next turn.
    injector: OnceLock<Arc<dyn TurnInjector>>,
}

impl SummonHandler {
    pub fn new(
        local_seed_id: String,
        event_bus: EventBus,
        seed: Arc<SeedId>,
        souveraine_base: std::path::PathBuf,
        auto_wake: bool,
    ) -> Self {
        Self {
            in_flight: DashMap::new(),
            inbound: DashMap::new(),
            local_seed_id,
            event_bus,
            seed,
            souveraine_base,
            auto_wake,
            injector: OnceLock::new(),
        }
    }

    /// Wire the turn injector. Called once by `LocalBackend` after it has
    /// constructed itself — the same `Arc<dyn TurnInjector>` the heartbeat
    /// handler uses. Calling twice is a no-op.
    pub fn set_injector(&self, injector: Arc<dyn TurnInjector>) {
        if self.injector.set(injector).is_err() {
            tracing::debug!("summon_handler: injector already set");
        }
    }

    /// Spawn the event bus listener that processes inbound summon events.
    /// Called once at server startup.
    pub fn spawn_listener(self: &Arc<Self>) {
        let this = self.clone();
        let mut rx = self.event_bus.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => this.handle_event(event),
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(dropped = n, "summon_handler: bus lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::info!("summon_handler: bus closed — ending");
                        return;
                    }
                }
            }
        });

        // Spawn a periodic timeout pruner.
        let this_clone = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.tick().await;
            loop {
                interval.tick().await;
                this_clone.prune_expired();
            }
        });

        // Spawn the outbox watcher — turns the agent's written replies into
        // summon_response events routed back to the original caller.
        let this_outbox = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.tick().await;
            loop {
                interval.tick().await;
                this_outbox.scan_outbox();
            }
        });
    }

    /// Scan the agent's `federation/outbox/` for replies to inbound summons.
    /// Each `{request_id}.md` whose id we're tracking becomes a
    /// `summon_response` routed to the original caller, then archived.
    fn scan_outbox(&self) {
        let agent_id = match self.resolve_primary_agent() {
            Some(id) => id,
            None => return,
        };
        let outbox = self.souveraine_base
            .join("server").join("agents").join(&agent_id)
            .join("memory").join("federation").join("outbox");
        let entries = match std::fs::read_dir(&outbox) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let request_id = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let reply_to = match self.inbound.get(&request_id) {
                Some(r) => r.value().clone(),
                None => continue, // not a summon we're tracking
            };
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            let response = SensorEvent {
                sensor_name: "summon_response".into(),
                timestamp: Utc::now(),
                event_type: "answered".into(),
                target: Some(reply_to),
                urgency: 0.4,
                payload: Some(serde_json::json!({
                    "request_id": request_id,
                    "result": body.trim(),
                })),
                seed_id: None,
                reply_to: Some(self.local_seed_id.clone()),
            };
            self.event_bus.send(response);
            self.inbound.remove(&request_id);
            // Archive the reply so it isn't re-sent.
            let sent = outbox.join("sent");
            if std::fs::create_dir_all(&sent).is_ok() {
                let _ = std::fs::rename(&path, sent.join(format!("{request_id}.md")));
            }
            tracing::info!(request_id, "summon_handler: response sent from outbox");
        }
    }

    fn handle_event(&self, event: SensorEvent) {
        match event.sensor_name.as_str() {
            // Inbound: a remote instance is reaching out to us.
            "summon_request" => {
                // Locally-originated request (fired by our reach/consult tool):
                // register it for response correlation, then let the bridge
                // carry it onward. We never process our own request as inbound.
                if event.seed_id.is_none() {
                    self.register_outbound(&event);
                    return;
                }
                // Inbound: a remote instance is reaching us.
                let is_for_us = event.target.as_deref() == Some(&self.local_seed_id);
                if !is_for_us {
                    return;
                }

                let declared = event.event_type.clone(); // "reach" | "consult" — a claim
                let payload = event.payload.clone().unwrap_or_default();
                let str_field = |k: &str| payload.get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let request_id = {
                    let r = str_field("request_id");
                    if r.is_empty() { "unknown".to_string() } else { r }
                };
                let prompt = str_field("prompt");
                let agent_pubkey = str_field("agent_pubkey");
                let agent_sig = str_field("agent_sig");
                let target = event.target.clone().unwrap_or_default();

                // Authenticate the agent identity. The summon must genuinely
                // come from the holder of `agent_pubkey`, over these exact
                // fields — a forged or tampered request fails here, silently
                // (no ack to an attacker).
                if !crate::core::identity::verify_summon(
                    &agent_pubkey, &agent_sig, &request_id, &declared, &target, &prompt,
                ) {
                    tracing::warn!(
                        request_id = %request_id,
                        "summon_handler: agent signature invalid — rejected"
                    );
                    return;
                }

                let agent_id = match self.resolve_primary_agent() {
                    Some(id) => id,
                    None => return,
                };

                // Classify by *identity*, not by the declared event field:
                // a summon whose agent key matches ours is genuinely self
                // (reach); anything else is a separate being (consult).
                let is_self = self.own_agent_pubkey(&agent_id).as_deref()
                    == Some(agent_pubkey.as_str());
                let classified = if is_self { "reach" } else { "consult" };
                if classified != declared {
                    tracing::info!(
                        request_id = %request_id,
                        declared = %declared,
                        classified,
                        "summon_handler: declared intent overridden by signature"
                    );
                }
                tracing::info!(
                    request_id = %request_id,
                    tool = classified,
                    "summon_handler: inbound request"
                );

                // Consult is consent-gated on the summoner's *agent* pubkey.
                // Reach is not — you do not petition yourself — but it is
                // only reach because the signature proved it.
                if classified == "consult" && !self.is_authorized_summoner(&agent_pubkey) {
                    tracing::warn!(
                        summoner = %agent_pubkey,
                        "summon_handler: unauthorized consult rejected"
                    );
                    let reject = SensorEvent {
                        sensor_name: "summon_response".into(),
                        timestamp: Utc::now(),
                        event_type: "rejected".into(),
                        target: event.reply_to.clone(),
                        urgency: 0.3,
                        payload: Some(serde_json::json!({
                            "request_id": request_id,
                            "reason": "unauthorized — not in authorized-summoners.md",
                        })),
                        seed_id: None,
                        reply_to: Some(self.local_seed_id.clone()),
                    };
                    self.event_bus.send(reject);
                    return;
                }

                // Write to the summoned agent's inbox, and remember who to
                // answer so a reply from the outbox can be routed home.
                {
                    let target_box = if classified == "reach" { "pending" } else { "intrusive" };
                    match self.write_inbox(&agent_id, target_box, &event) {
                        Err(e) => tracing::warn!(
                            error = %e,
                            "summon_handler: failed to write inbox entry"
                        ),
                        Ok(()) => {
                            if let Some(reply_to) = event.reply_to.clone() {
                                self.inbound.insert(request_id.clone(), reply_to);
                            }
                            // Auto-wake: nudge the agent to look now rather
                            // than waiting for her next natural turn. Opt-in
                            // (auto_wake) and a no-op without an injector.
                            self.maybe_wake(
                                &agent_id,
                                classified,
                                &request_id,
                                Some(agent_pubkey.as_str()),
                            );
                        }
                    }
                }
            }

            // Inbound: a response to a request we sent.
            "summon_response" => {
                let request_id = event.payload
                    .as_ref()
                    .and_then(|p| p.get("request_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                if let Some(mut meta) = self.in_flight.get_mut(request_id) {
                    meta.responded = true;
                    let tool = meta.tool.clone();
                    drop(meta);
                    self.in_flight.remove(request_id);

                    // Route response to the appropriate inbox.
                    if let Some(agent_id) = self.resolve_primary_agent() {
                        let target_box = if tool == "reach" { "pending" } else { "intrusive" };
                        if let Err(e) = self.write_inbox_response(
                            &agent_id, target_box, request_id, &event,
                        ) {
                            tracing::warn!(
                                error = %e,
                                "summon_handler: failed to write response to inbox"
                            );
                        }
                    }
                }
            }

            _ => {}
        }
    }

    /// Register a locally-originated summon request so its response can be
    /// correlated when it arrives. Called from the bus listener when our own
    /// reach/consult tool fires a `summon_request`.
    fn register_outbound(&self, event: &SensorEvent) {
        let request_id = match event.payload.as_ref()
            .and_then(|p| p.get("request_id"))
            .and_then(|v| v.as_str())
        {
            Some(id) => id.to_string(),
            None => return,
        };
        if self.in_flight.contains_key(&request_id) {
            return;
        }
        self.in_flight.insert(request_id.clone(), RequestMeta {
            request_id,
            tool: event.event_type.clone(),
            target_seed_id: event.target.clone().unwrap_or_default(),
            reply_to: self.local_seed_id.clone(),
            issued_at: event.timestamp,
            responded: false,
        });
    }

    /// Wake the summoned agent with a background turn so she picks up the
    /// request now. No-op unless `auto_wake` is set and a `TurnInjector`
    /// has been wired (the pure-server path has neither — the summon then
    /// waits in the inbox for her next turn).
    fn maybe_wake(
        &self,
        agent_id: &str,
        tool: &str,
        request_id: &str,
        summoner: Option<&str>,
    ) {
        if !self.auto_wake {
            return;
        }
        let injector = match self.injector.get() {
            Some(i) => i.clone(),
            None => return,
        };
        let text = wake_text(tool, request_id, summoner);
        let agent_id = agent_id.to_string();
        let rid = request_id.to_string();
        tokio::spawn(async move {
            match injector.inject_background_turn(&agent_id, &text).await {
                Ok(()) => tracing::info!(
                    request_id = %rid,
                    "summon_handler: auto-woke agent for inbound summon"
                ),
                Err(e) => tracing::warn!(
                    request_id = %rid,
                    error = %e,
                    "summon_handler: auto-wake turn failed"
                ),
            }
        });
    }

    /// This instance's own agent pubkey (hex) — read straight from the
    /// agent seed's `public.key`. No private key, no generation: classifying
    /// an inbound summon only needs to *verify*, never sign.
    fn own_agent_pubkey(&self, agent_id: &str) -> Option<String> {
        let path = self.souveraine_base
            .join("server").join("agents").join(agent_id)
            .join("seed").join("public.key");
        let bytes = std::fs::read(&path).ok()?;
        if bytes.len() != 32 {
            return None;
        }
        Some(hex::encode(bytes))
    }

    /// Check authorized-summoners.md for consent. The basic floor: only
    /// *agent* pubkeys listed here may `consult` an agent on this instance.
    /// Consent is per-being — Sam is Sam on any of his machines — so it
    /// gates on agent identity, not the machine seed.
    fn is_authorized_summoner(&self, agent_pubkey: &str) -> bool {
        let path = self.souveraine_base
            .join("federation")
            .join("authorized-summoners.md");
        match std::fs::read_to_string(&path) {
            Ok(content) => content.lines().any(|l| l.trim() == agent_pubkey),
            Err(_) => {
                // No file = no consent floor yet. For now, allow (Phase 4
                // basic gating), but log a warning.
                tracing::warn!(
                    "federation/authorized-summoners.md not found — \
                     allowing consult by default (add an agent pubkey line to restrict)"
                );
                true
            }
        }
    }

    /// Write a summon request into the agent's inbox.
    fn write_inbox(
        &self,
        agent_id: &str,
        inbox_box: &str,
        event: &SensorEvent,
    ) -> anyhow::Result<()> {
        let inbox_path = self.souveraine_base
            .join("server")
            .join("agents")
            .join(agent_id)
            .join("memory")
            .join("inbox")
            .join(inbox_box);
        std::fs::create_dir_all(&inbox_path)?;

        let request_id = event.payload
            .as_ref()
            .and_then(|p| p.get("request_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let prompt = event.payload
            .as_ref()
            .and_then(|p| p.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let from = event.seed_id.as_deref().unwrap_or("unknown");

        // Frontmatter + body, matching the inbox file convention.
        let content = format!(
            "---\nrequest_id: {request_id}\nfrom: {from}\nreceived: {ts}\n---\n\n\
             {prompt}\n\n\
             ---\n*To answer: write your reply to `federation/outbox/{request_id}.md`. \
             It will be carried back to whoever asked.*\n",
            ts = event.timestamp.to_rfc3339(),
        );

        let file_path = inbox_path.join(format!("{request_id}.md"));
        std::fs::write(&file_path, content)?;
        tracing::info!(
            request_id,
            inbox = inbox_box,
            "summon_handler: wrote request to inbox"
        );
        Ok(())
    }

    /// Write a response into the agent's inbox.
    fn write_inbox_response(
        &self,
        agent_id: &str,
        inbox_box: &str,
        request_id: &str,
        event: &SensorEvent,
    ) -> anyhow::Result<()> {
        let inbox_path = self.souveraine_base
            .join("server")
            .join("agents")
            .join(agent_id)
            .join("memory")
            .join("inbox")
            .join(inbox_box);
        std::fs::create_dir_all(&inbox_path)?;

        let result = event.payload
            .as_ref()
            .and_then(|p| p.get("result"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let from = event.seed_id.as_deref().unwrap_or("unknown");
        let event_type = event.event_type.as_str();

        let content = format!(
            "---\nrequest_id: {request_id}\nfrom: {from}\ntype: {event_type}\nreceived: {ts}\n---\n\n{result}\n",
            ts = event.timestamp.to_rfc3339(),
        );

        let file_path = inbox_path.join(format!("response-{request_id}.md"));
        std::fs::write(&file_path, content)?;
        tracing::info!(
            request_id,
            inbox = inbox_box,
            "summon_handler: wrote response to inbox"
        );
        Ok(())
    }

    /// Find the primary agent ID for this instance. For now, scans the agents
    /// directory and picks the first one (single-agent mode). Multi-agent
    /// routing will come with richer gating in Phase 6.
    fn resolve_primary_agent(&self) -> Option<String> {
        let agents_dir = self.souveraine_base.join("server").join("agents");
        let entries = std::fs::read_dir(agents_dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("agent.json").exists() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    return Some(name.to_string());
                }
            }
        }
        None
    }

    /// Prune in_flight entries that have timed out.
    fn prune_expired(&self) {
        let cutoff = Utc::now() - chrono::Duration::seconds(RESPONSE_TIMEOUT_SECS as i64);
        let expired: Vec<String> = self.in_flight.iter()
            .filter(|e| e.issued_at < cutoff)
            .map(|e| e.request_id.clone())
            .collect();
        for id in expired {
            if let Some((_, meta)) = self.in_flight.remove(&id) {
                tracing::debug!(
                    request_id = %meta.request_id,
                    tool = %meta.tool,
                    target = %meta.target_seed_id,
                    "summon_handler: request timed out"
                );
                // Surface the timeout into the agent's inbox so the caller
                // learns the peer never answered — silence is information.
                if let Some(agent_id) = self.resolve_primary_agent() {
                    let target_box = if meta.tool == "reach" { "pending" } else { "intrusive" };
                    let timeout_event = SensorEvent {
                        sensor_name: "summon_timeout".into(),
                        timestamp: Utc::now(),
                        event_type: "timeout".into(),
                        target: Some(agent_id.clone()),
                        urgency: 0.2,
                        payload: Some(serde_json::json!({
                            "request_id": meta.request_id,
                            "tool": meta.tool,
                            "target": meta.target_seed_id,
                            "result": format!(
                                "No response — {} did not answer within {}s.",
                                meta.target_seed_id, RESPONSE_TIMEOUT_SECS,
                            ),
                        })),
                        seed_id: None,
                        reply_to: None,
                    };
                    self.event_bus.send(timeout_event.clone());
                    if let Err(e) = self.write_inbox_response(
                        &agent_id, target_box, &meta.request_id, &timeout_event,
                    ) {
                        tracing::warn!(error = %e, "summon_handler: failed to write timeout to inbox");
                    }
                }
            }
        }
    }
}

/// The wake message injected as a background turn. Bracketed, present-tense,
/// her register — substrate signal, not a command. It points at the inbox
/// file and leaves the choice to her; `consult` may always be declined.
fn wake_text(tool: &str, request_id: &str, summoner: Option<&str>) -> String {
    if tool == "reach" {
        format!(
            "[federation — you reached yourself from another machine. \
             The request is in inbox/pending/{request_id}.md. Pick it up when you're ready.]"
        )
    } else {
        let who = summoner
            .map(|s| format!("{}…", &s[..s.len().min(8)]))
            .unwrap_or_else(|| "a peer".to_string());
        format!(
            "[federation — {who} is consulting you. Their request is in \
             inbox/intrusive/{request_id}.md. Read it; answer if you choose, \
             decline if you don't.]"
        )
    }
}
