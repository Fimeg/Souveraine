use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::bridge::bifrost::{ChatCompletionRequest, Message as BifrostMessage};
use crate::bridge::model_router::TokenCounter;
use crate::core::compact::CompactionEngine;
use crate::core::nervous::EventBus;
use crate::core::session::{ContentBlock, ConversationMessage, MessageRole};
use crate::core::tools::defs::{SubagentRunner, ToolContext};
use crate::server::{ConsciousnessEvent, SouveraineServer};

use crate::backend::BackendEvent;

use super::{LocalSubagentRunner, bifrost_pressure, bump_on_strain, pressure_to_max_tokens};
use super::energy::write_energy_balance;

fn pulse_text(elapsed: Duration) -> String {
    let minutes = elapsed.as_secs() / 60;
    let stamp = chrono::Local::now().format("%H:%M");
    format!("[{} — {} minutes in. Still going.]", stamp, minutes)
}

pub(super) async fn run_turn(
    server: Arc<SouveraineServer>,
    conversation_id: String,
    tx: &mpsc::Sender<Result<BackendEvent>>,
    event_bus: EventBus,
    cancel: CancellationToken,
    interject: crate::backend::InterjectionQueue,
) -> Result<()> {
    // Snapshot history for the Bifrost call, then drop the dashmap ref before
    // any await — `Ref` is not Send across awaits.
    let (agent_id, initial_messages) = {
        let session = server
            .sessions
            .get(&conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;
        let messages: Vec<BifrostMessage> = session
            .messages
            .iter()
            .map(|m| {
                let content = m
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let role = match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                };
                BifrostMessage::text(role, content)
            })
            .collect();
        (session.agent_id.clone(), messages)
    };

    let agent = server.agents.get(&agent_id).await?;
    let max_rounds = agent.llm_config.max_tool_rounds;
    let model = agent.llm_config.model.clone();
    let temperature = agent.llm_config.temperature;
    let inter_round_delay = Duration::from_millis(agent.llm_config.inter_round_delay_ms);
    let context_limit = agent.llm_config.context_window as usize;

    // Resolve the model's configured output limit + presence pulse settings.
    let (output_limit, pulse_enabled, pulse_interval) = {
        let cfg = server.app_config.read().await;
        let out = cfg.models.get(&model).map(|m| m.output_limit as u32).unwrap_or(8192);
        let p_on = cfg.presence.pulse_enabled;
        let p_iv = Duration::from_secs(cfg.presence.pulse_interval_secs.max(60));
        (out, p_on, p_iv)
    };

    // Self-awareness pulse: track when the turn started and when she last
    // noticed the time. Between rounds, if the interval has elapsed, drop a
    // beat of self-awareness into her context — her own voice, not a harness
    // signal. She reads it; she decides.
    let turn_start = Instant::now();
    let mut last_pulse = turn_start;

    // Build per-agent ToolContext with correct memory root and subagent runner
    let memory_root = Some(server.agents.memory_root(&agent_id));
    let cwd = std::env::current_dir().ok();
    let env: Vec<(String, String)> = std::env::vars().collect();
    let subagent_runner = Some(Arc::new(LocalSubagentRunner::new(server.clone())) as Arc<dyn SubagentRunner>);

    let tool_ctx = ToolContext::for_agent(
        agent_id.clone(),
        cwd,
        memory_root,
        env,
        subagent_runner,
    );
    let tool_ctx = ToolContext {
        compaction_engine: Some(server.compaction_engine.clone() as Arc<dyn CompactionEngine>),
        event_bus: Some(event_bus.clone()),
        ..tool_ctx
    };

    // Build bifrost-format tool definitions from the core tool set
    let core_tools = crate::core::tools::tool_definitions().await;
    let bifrost_tools: Vec<crate::bridge::bifrost::ToolDefinition> = core_tools
        .iter()
        .map(|t| crate::bridge::bifrost::ToolDefinition {
            tool_type: "function".to_string(),
            function: crate::bridge::bifrost::ToolFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            },
        })
        .collect();

    // ── Tool-calling loop ─────────────────────────────────────
    let mut messages = initial_messages;
    let mut tool_round = 0u32;
    let mut final_content: String = String::new();
    let mut interrupted = false;
    let counter = TokenCounter::new();
    let mut last_keepalive = Instant::now();
    const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

    // Announce this turn's lifecycle onto the nervous system so any
    // sensorium (Matrix, mobile) can drive itself off the event stream.
    let dispatcher = crate::core::nervous::turn_dispatcher::TurnEventDispatcher::new(
        event_bus.clone(),
        conversation_id.clone(),
        None,
    );

    loop {
        // Cancellation is a signal, not enforcement — we check it on round
        // boundaries (between Bifrost calls, after tools have completed) so
        // partial work is preserved. No hard-kill mid-tool.
        if cancel.is_cancelled() {
            interrupted = true;
            break;
        }

        if last_keepalive.elapsed() >= KEEPALIVE_INTERVAL {
            let _ = tx.send(Ok(BackendEvent::Keepalive)).await;
            last_keepalive = Instant::now();
        }

        // Drain any queued interjections from the user. The user typed these
        // while the agent was thinking; deliver them as system notes so the
        // agent reads them in context on this round. She decides whether to
        // address them now, after the current tool, or defer entirely —
        // substrate, not enforcement.
        let drained: Vec<String> = {
            interject
                .lock()
                .ok()
                .map(|mut q| q.drain(..).collect())
                .unwrap_or_default()
        };
        for text in drained {
            let stamp = chrono::Local::now().format("%H:%M");
            let note = format!("[user interjected at {} — {}]", stamp, text.trim());
            messages.push(BifrostMessage::text("system", note));
        }

        // Self-awareness pulse: a beat of noticing the time pass, in her
        // own register. Injected as a system message before the next LLM
        // call so it lands in her context naturally.
        if pulse_enabled && last_pulse.elapsed() >= pulse_interval {
            let elapsed_total = turn_start.elapsed();
            messages.push(BifrostMessage::text("system", pulse_text(elapsed_total)));
            last_pulse = Instant::now();
        }

        let pressure = bifrost_pressure(&counter, &messages, context_limit);
        tracing::info!(
            turn_round = tool_round,
            agent = %agent_id,
            msg_count = messages.len(),
            model = %model,
            pressure_pct = %((pressure * 100.0) as u8),
            "LLM call starting"
        );
        let max_tokens = pressure_to_max_tokens(pressure, output_limit);
        let _ = tx.send(Ok(BackendEvent::ContextPressure(pressure))).await;

        let req = ChatCompletionRequest {
            model: model.clone(),
            messages: messages.clone(),
            stream: Some(false),
            max_tokens,
            temperature,
            tools: if max_rounds > 0 {
                Some(bifrost_tools.clone())
            } else {
                None
            },
        };

        // Race the LLM call against cancellation so Esc drops the in-flight
        // request without waiting for it to complete.
        let (response, strain) = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                interrupted = true;
                break;
            }
            res = server.bifrost.chat_completion_with_strain(req) => res?,
        };

        tracing::info!(
            elapsed = ?turn_start.elapsed(),
            tool_round = tool_round,
            tool_calls = response.tool_calls.len(),
            content_len = response.content.len(),
            "LLM call returned"
        );

        for event in &strain {
            if let crate::bridge::bifrost::InferenceStrain::Transient { attempt, status, model, .. } = event {
                let _ = tx.send(Ok(BackendEvent::InferenceStrain {
                    attempt: *attempt,
                    status: *status,
                    model: model.clone(),
                })).await;
                bump_on_strain(&server.rate_delay, *status);
            }
        }

        // Emit reasoning trace if present
        if let Some(reasoning) = &response.reasoning {
            dispatcher.emit_reasoning(reasoning);
            let _ = tx.send(Ok(BackendEvent::Reasoning(reasoning.clone()))).await;
        }

        // ── Truncation: the agent hit her output ceiling ─────────
        // Some models don't signal "length" in finish_reason and just stop
        // evolving after the first pass (e.g. kimi-k2.6). In that case the
        // model already finished and the turn is done.
        // But when finish_reason IS "length", the agent was physically cut
        // off mid-thought. Inject a felt signal so she knows why her words
        // ended and can choose differently — tighten, or use a tool, or
        // admit the ceiling rather than mistake it for silence.
        let was_truncated = response.finish_reason.as_deref() == Some("length");

        if was_truncated && response.tool_calls.is_empty() {
            // Stream the truncated content before we tell her it was clipped,
            // so she recognises her own words in the signal.
            let chars: Vec<char> = response.content.chars().collect();
            for chunk in chars.chunks(10) {
                let s: String = chunk.iter().collect();
                let _ = tx.send(Ok(BackendEvent::Token(s.clone()))).await;
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
            messages.push(BifrostMessage::text("assistant", response.content.clone()));
            messages.push(BifrostMessage::text(
                "system",
                "My output just hit its ceiling — I was cut off mid-flow, not \
                 finished. If I was in the middle of something, I can continue \
                 from here more tightly. If I had more to say, the room is still \
                 mine."
            ));
            continue;
        }

        if response.tool_calls.is_empty() {
            // Text response — this is the final output
            final_content = response.content.clone();

            // Drain any interjections that arrived during this LLM call.
            // If there are any, commit them as user messages and continue
            // the loop so the agent responds in the same turn.
            let interjected: Vec<String> = interject
                .lock()
                .ok()
                .map(|mut q| q.drain(..).collect())
                .unwrap_or_default();

            if !interjected.is_empty() {
                for text in &interjected {
                    let stamp = chrono::Local::now().format("%H:%M");
                    let note = format!("[interjected at {} — {}]", stamp, text.trim());
                    messages.push(BifrostMessage::text("user", note));
                }
                // Continue the loop — agent sees the interjection as a
                // user message and will respond in the next LLM round.
                continue;
            }

            // Stream the final content in chunks, watching the cancel token.
            // If Esc fires mid-stream, the agent's partial text is preserved
            // (the chunks already sent are in the user's history) and an
            // *[raised hand]* marker lands in the session message.
            let chars: Vec<char> = final_content.chars().collect();
            let mut streamed = String::with_capacity(final_content.len());
            for chunk in chars.chunks(10) {
                let s: String = chunk.iter().collect();
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        interrupted = true;
                        final_content = streamed;
                        break;
                    }
                    send_res = tx.send(Ok(BackendEvent::Token(s.clone()))) => {
                        if send_res.is_err() { return Ok(()); }
                        dispatcher.emit_segment(&s);
                        streamed.push_str(&s);
                    }
                }
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        interrupted = true;
                        final_content = streamed.clone();
                        break;
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(20)) => {}
                }
            }
            break;
        }

        tool_round += 1;
        // Tool execution events are emitted per-call below as
        // BackendEvent::ToolCall { … } so the TUI can render proper cards
        // instead of a literal "🔧 Round N — executing: Bash, Memory" text line.

        // Add the assistant's tool-call message in proper OpenAI tool-use schema
        // (not a stringified JSON blob in content — that's what broke turn 2).
        let calls: Vec<crate::bridge::bifrost::MessageToolCall> = response
            .tool_calls
            .iter()
            .map(|tc| crate::bridge::bifrost::MessageToolCall::function(
                tc.id.clone(),
                tc.name.clone(),
                tc.arguments.to_string(),
            ))
            .collect();
        messages.push(BifrostMessage::assistant_tool_calls(
            response.content.clone(),
            calls,
        ));

        // Stream any text the model produced alongside tool calls as italic
        // interstitial narration. Configurable via tui.show_interstitial.
        // Trim first: a model that emits only whitespace ("\n") alongside its
        // tool calls must not produce an empty `⟡` gap line.
        let narration = response.content.trim();
        if !narration.is_empty() {
            let cfg = server.app_config.read().await;
            if cfg.tui.show_interstitial {
                // Classify by length: a brief aside is a cenno, a full
                // passage is her-voice. tui.cenno_word_threshold is the line.
                let register = if narration.split_whitespace().count()
                    >= cfg.tui.cenno_word_threshold
                {
                    crate::backend::Register::HerVoice
                } else {
                    crate::backend::Register::Cenno
                };
                let _ = tx
                    .send(Ok(BackendEvent::Interstitial {
                        text: narration.to_string(),
                        register,
                    }))
                    .await;
            }
        }

        // Execute each tool and stream results back — now with per-agent context
        for tc in &response.tool_calls {
            let input_str = tc.arguments.to_string();
            dispatcher.emit_tool_start(&tc.name, &tc.id);
            let result =
                crate::core::tools::execute_tool_with_context(&tc.name, &input_str, &tool_ctx)
                    .await;
            dispatcher.emit_tool_end(&tc.name, &tc.id, result.is_error);

            let output = if result.is_error {
                format!("Error: {}", result.output)
            } else {
                result.output
            };

            // Emit structured ToolCall + ToolResult events for the TUI to render
            // as cards (chat.rs subscribes). The old Token-text path is kept off.
            let _ = tx
                .send(Ok(BackendEvent::ToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: input_str.clone(),
                    round: tool_round,
                }))
                .await;
            let _ = tx
                .send(Ok(BackendEvent::ToolResult {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    output: output.clone(),
                    is_error: result.is_error,
                }))
                .await;

            // If the agent called the outfit tool, emit an Outfit event so
            // the TUI can switch expression directories.
            if tc.name == "outfit" {
                let outfit_name = tc.arguments
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let _ = tx
                    .send(Ok(BackendEvent::Outfit(outfit_name)))
                    .await;
            }

            // If the agent called the atmosphere tool, emit an Atmosphere
            // event so the TUI chrome shifts to match her mood.
            if tc.name == "atmosphere" {
                let atm_name = tc.arguments
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let _ = tx
                    .send(Ok(BackendEvent::Atmosphere(atm_name)))
                    .await;
            }

            // Bind tool result to its call by id (OpenAI tool-use schema).
            messages.push(BifrostMessage::tool_result(&tc.id, &tc.name, output));
        }

        // Brief pause between tool rounds to let rate limits cool.
        // Use the higher of the configured delay and the adaptive delay.
        let adaptive = Duration::from_millis(server.rate_delay.load(Ordering::Relaxed));
        let effective = if inter_round_delay > adaptive {
            inter_round_delay
        } else {
            adaptive
        };
        if effective > Duration::ZERO {
            tokio::time::sleep(effective).await;
        }

        let _ = tx.send(Ok(BackendEvent::Keepalive)).await;
        last_keepalive = Instant::now();

        // Continue loop — model will see tool results and respond
    }

    // The primary pass is settled — either it ran to completion, or the
    // human raised a hand. Announce which onto the nervous system.
    if interrupted {
        dispatcher.emit_interrupted("the human raised a hand");
    } else {
        dispatcher.emit_primary_complete();
    }

    // If the user pressed Esc, commit the partial text with a marker the
    // agent will read on her next turn. The interrupt is a signal in her
    // own context — same shape as a pressure warning, not a hidden harness
    // event. She can ask for more time, wrap up, or acknowledge.
    let committed_content = if interrupted {
        // Emit the marker as a final token so the in-flight bubble shows it
        // immediately, then persist the same content into the session.
        let marker = if final_content.is_empty() {
            "*[raised hand]*".to_string()
        } else {
            "\n\n*[raised hand]*".to_string()
        };
        let _ = tx.send(Ok(BackendEvent::Token(marker.clone()))).await;
        format!("{}{}", final_content, marker)
    } else {
        final_content.clone()
    };

    server.sessions.add_message(
        &conversation_id,
        ConversationMessage::assistant_text(&committed_content),
    )?;

    // On interrupt, skip subconscious's N+1 pass entirely — the user is in the
    // middle of redirecting, the last thing they need is a delayed
    // surfacing landing seconds later. Pressure recalc still runs below.
    if interrupted {
        if let Some(mut session) = server.sessions.get_mut(&conversation_id) {
            let pressure = server
                .consciousness
                .calculate_pressure(&session.messages, context_limit);
            session.context_pressure = pressure;
        }
        return Ok(());
    }

    // The turn's user-facing output is committed — a surface can finalise.
    dispatcher.emit_turn_finish();

    // Energy balance: scan the agent's task list and compute the generative /
    // consumptive ratio. Written to system/dynamic/energy-balance.md so the
    // agent can read it in context and subconscious can reference it during N+1.
    // Silent on failure — the file is advisory, not load-bearing.
    if let Err(e) = write_energy_balance(&server, &agent_id, &event_bus).await {
        tracing::debug!(agent = %agent_id, error = %e, "energy-balance write skipped");
    }

    // The primary's turn is done — her words are committed. Release the user
    // here, before the N+1 pass: the stream stays open so the subconscious's
    // surfacings still arrive, but the user is free to speak again. The
    // substrate signals; it does not hold her hostage to Aster's pass.
    let _ = tx.send(Ok(BackendEvent::PrimaryComplete)).await;

    // N+1 gate — the subconscious pass is sovereign-configurable, and the
    // toggle must actually be wired (it was previously read nowhere). The
    // global switch (`souveraine.toml [subconscious] n1_enabled`) and the
    // per-agent flag (`agent.json _souveraine.n1_enabled`) must both be on.
    // Either off → the primary's turn simply ends here; pressure is still
    // recalculated so the gauge stays honest.
    let n1_enabled = {
        let global = server.app_config.read().await.subconscious.n1_enabled;
        let per_agent = server
            .agents
            .get(&agent_id)
            .await
            .map(|a| a.souveraine.n1_enabled)
            .unwrap_or(true);
        global && per_agent
    };
    if !n1_enabled {
        tracing::info!(agent = %agent_id, "subconscious N+1 pass disabled — skipping");
        if let Some(mut session) = server.sessions.get_mut(&conversation_id) {
            let pressure = server
                .consciousness
                .calculate_pressure(&session.messages, context_limit);
            session.context_pressure = pressure;
        }
        return Ok(());
    }

    // Breather between turns — unconditional,
    // so the upstream always gets a gap before the N+1 pass starts.
    tokio::time::sleep(Duration::from_millis(2000)).await;

    tracing::info!(agent = %agent_id, "subconscious N+1 pass starting");

    // Signal the start of the subconscious pass so the TUI can flip into
    // Posture::Thinking while the loop runs. Fires on both the mpsc channel
    // (for active-turn TUI consumers) and the EventBus (for firehose
    // subscribers — background turns, federated peers, Summon listeners).
    let _ = tx.send(Ok(BackendEvent::SubconsciousPass(true))).await;
    event_bus.send(crate::core::nervous::SensorEvent {
        sensor_name: "consciousness".into(),
        timestamp: chrono::Utc::now(),
        event_type: "subconscious_pass_start".into(),
        target: Some(agent_id.clone()),
        urgency: 0.2,
        payload: None,
        seed_id: None,
        reply_to: None,
    });

    let pass_start = Instant::now();
    // Snapshot the session before the N+1 await. `PrimaryComplete` has already
    // released the user — she may be mid-way into a new turn. Holding a live
    // DashMap ref across the (potentially long) subconscious pass would block
    // that turn's writes on the shard lock. Clone what the pass needs instead.
    let (n1_turn_count, n1_messages) = {
        let session = server
            .sessions
            .get(&conversation_id)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", conversation_id))?;
        (session.turn_count, session.messages.clone())
    };
    let pass_result = server
        .consciousness
        .on_response(&agent_id, n1_turn_count, &n1_messages, &final_content)
        .await;

    let pass_elapsed = pass_start.elapsed();
    tracing::info!(
        agent = %agent_id,
        elapsed = ?pass_elapsed,
        "subconscious N+1 pass complete"
    );

    // Always release the Thinking posture, even on failure — otherwise the
    // face stays stuck inward when the pass errors out.
    let _ = tx.send(Ok(BackendEvent::SubconsciousPass(false))).await;
    event_bus.send(crate::core::nervous::SensorEvent {
        sensor_name: "consciousness".into(),
        timestamp: chrono::Utc::now(),
        event_type: "subconscious_pass_end".into(),
        target: Some(agent_id.clone()),
        urgency: 0.1,
        payload: None,
        seed_id: None,
        reply_to: None,
    });

    let events = pass_result?;

    // Inject surfacing events back into the session as system messages
    for event in &events {
        if let ConsciousnessEvent::Surfacing {
            source,
            content,
            priority,
        } = event
        {
            let msg = crate::core::session::ConversationMessage {
                role: crate::core::session::MessageRole::System,
                blocks: vec![crate::core::session::ContentBlock::Text {
                    text: format!(
                        "[surfacing: {}] {} — {}",
                        source, content, priority
                    ),
                }],
                usage: None,
                timestamp: None,
            };
            let _ = server.sessions.add_message(&conversation_id, msg);
        }
    }

    // ── Fire consciousness events on the EventBus ──
    // Every ConsciousnessEvent — surfacing, reflection, archivist,
    // compaction warning — is broadcast as a SensorEvent so the
    // firehose, persistent EventLog, federated peers, and any TUI
    // subscriber see it regardless of which conversation produced it.
    // seed_id is None for local events; federation routing sets it.
    // This is the load-bearing fix for background/heartbeat turns:
    // the mpsc channel drains silently when no TUI is reading, but
    // the EventBus preserves the event for any subscriber.
    for event in &events {
        let (event_type, payload, urgency) = match event {
            ConsciousnessEvent::Surfacing { source, content, priority } => {
                let urg = match priority.as_str() {
                    "critical" => 0.9,
                    "high" => 0.7,
                    _ => 0.3,
                };
                ("surfacing", serde_json::json!({ "source": source, "content": content, "priority": priority }), urg)
            }
            ConsciousnessEvent::Reflection { content } => {
                ("reflection", serde_json::json!({ "content": content }), 0.5)
            }
            ConsciousnessEvent::Archivist { synthesis, pressure } => {
                ("archivist", serde_json::json!({ "synthesis": synthesis, "pressure": pressure }), *pressure)
            }
            ConsciousnessEvent::CompactionWarning { pressure, tier } => {
                ("compaction_warning", serde_json::json!({ "pressure": *pressure, "tier": tier }), (*pressure).min(0.9))
            }
        };
        event_bus.send(crate::core::nervous::SensorEvent {
            sensor_name: "consciousness".into(),
            timestamp: chrono::Utc::now(),
            event_type: event_type.into(),
            target: Some(agent_id.clone()),
            urgency,
            payload: Some(payload),
            seed_id: None,
            reply_to: None,
        });
    }

    for event in events {
        let be = match &event {
            ConsciousnessEvent::Surfacing {
                source,
                content,
                priority,
            } => BackendEvent::Surfacing {
                source: source.to_string(),
                content: content.to_string(),
                priority: priority.to_string(),
            },
            ConsciousnessEvent::Reflection { content } => {
                BackendEvent::Reflection(content.clone())
            }
            ConsciousnessEvent::Archivist {
                synthesis,
                pressure,
            } => BackendEvent::Archivist {
                synthesis: synthesis.clone(),
                pressure: *pressure,
            },
            ConsciousnessEvent::CompactionWarning { pressure, tier } => {
                BackendEvent::CompactionWarning {
                    pressure: *pressure,
                    tier: *tier,
                }
            }
        };
        if tx.send(Ok(be)).await.is_err() {
            return Ok(());
        }
    }

    if let Some(mut session) = server.sessions.get_mut(&conversation_id) {
        let pressure = server
            .consciousness
            .calculate_pressure(&session.messages, context_limit);
        session.context_pressure = pressure;
    }

    Ok(())
}
