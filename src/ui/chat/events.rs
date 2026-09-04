#![allow(dead_code)] // WIP scaffolding not yet wired
use std::cell::RefCell;
use std::time::Instant;

use base64::Engine;
use ratatui::layout::Rect;
use tokio::sync::mpsc;

use crate::backend::BackendEvent;

use super::{
    BtwForkEvent, BtwState, ChatMessage, ChatState, CockpitEntry, CockpitKind, ImageAttachment,
    Overlay, SlashDef, ToolResultBlock, TurnPhase, SLASH_COMMANDS,
};

impl ChatState {
    pub fn flush_delivered_interjections(&mut self) {
        let queue_empty = self
            .pending_interjections
            .lock()
            .ok()
            .map(|q| q.is_empty())
            .unwrap_or(true);
        if !queue_empty {
            return;
        }
        for msg in self.messages.iter_mut() {
            if let ChatMessage::Interjection { delivered, .. } = msg {
                *delivered = true;
            }
        }
    }

    pub fn drain_events(&mut self) {
        self.flush_delivered_interjections();

        self.drain_btw();

        if let Some(rx) = self.model_rx.as_mut() {
            if let Ok(result) = rx.try_recv() {
                self.messages.push(ChatMessage::System {
                    text: result,
                    ts: Instant::now(),
                });
                self.model_rx = None;
            }
        }

        if let Some(rx) = self.new_conv_rx.as_mut() {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(conv_id) => {
                        self.conversation_id = conv_id.clone();
                        self.messages.clear();
                        self.system_message(format!(
                            "New conversation started: {}",
                            &conv_id[..8.min(conv_id.len())]
                        ));
                    }
                    Err(e) => {
                        self.system_message(format!("Failed to create conversation: {}", e));
                    }
                }
                self.new_conv_rx = None;
            }
        }

        if let Some(rx) = self.convos_rx.as_mut() {
            if let Ok(result) = rx.try_recv() {
                let offer = std::mem::take(&mut self.resume_offer);
                match result {
                    Ok(mut convos) => {
                        if offer {
                            convos.retain(|c| c.id != self.conversation_id);
                        }
                        if convos.is_empty() {
                            if !offer {
                                self.system_message("No saved conversations.".to_string());
                            }
                        } else {
                            self.overlay = Overlay::ConversationPicker {
                                selected: 0,
                                conversations: convos,
                            };
                        }
                    }
                    Err(e) => {
                        if !offer {
                            self.system_message(format!("Failed to list conversations: {}", e));
                        }
                    }
                }
                self.convos_rx = None;
            }
        }

        if let Some(rx) = self.switch_rx.as_mut() {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok((conv_id, messages)) => {
                        self.conversation_id = conv_id.clone();
                        self.messages.clear();
                        for msg in &messages {
                            let text = msg
                                .blocks
                                .iter()
                                .filter_map(|b| match b {
                                    crate::core::session::ContentBlock::Text { text } => {
                                        Some(text.as_str())
                                    }
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            if text.is_empty() {
                                continue;
                            }
                            match msg.role {
                                crate::core::session::MessageRole::User => {
                                    let text = if text.starts_with("[ ambient sense - ") {
                                        text.lines().skip(1).collect::<Vec<_>>().join("\n")
                                    } else {
                                        text
                                    };
                                    self.messages.push(ChatMessage::User {
                                        text,
                                        ts: Instant::now(),
                                    });
                                }
                                crate::core::session::MessageRole::Assistant => {
                                    self.messages.push(ChatMessage::Assistant {
                                        text,
                                        ts: Instant::now(),
                                        streaming: false,
                                        rendered_cache: RefCell::new(None),
                                    });
                                }
                                crate::core::session::MessageRole::System => {
                                    self.messages.push(ChatMessage::System {
                                        text,
                                        ts: Instant::now(),
                                    });
                                }
                                _ => {}
                            }
                            // Backfill image blocks as Image chat messages
                            for block in &msg.blocks {
                                if let crate::core::session::ContentBlock::Image {
                                    media_type,
                                    ..
                                } = block
                                {
                                    self.messages.push(ChatMessage::Image {
                                        media_type: media_type.clone(),
                                        label: "[Image from history]".to_string(),
                                        data: String::new(), // not re-rendered from history
                                        dimensions: None,
                                        ts: Instant::now(),
                                    });
                                }
                            }
                        }
                        self.system_message(format!(
                            "Resumed conversation {} ({} messages)",
                            &conv_id[..8.min(conv_id.len())],
                            messages.len()
                        ));
                    }
                    Err(e) => {
                        self.system_message(format!("Failed to switch: {}", e));
                    }
                }
                self.switch_rx = None;
            }
        }

        let mut drained: Vec<BackendEvent> = Vec::new();
        let mut closed = false;
        if let Some(rx) = self.turn_rx.as_mut() {
            loop {
                match rx.try_recv() {
                    Ok(ev) => drained.push(ev),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        closed = true;
                        break;
                    }
                }
            }
        } else {
            return;
        }

        if !drained.is_empty() {
            self.last_event_at = Instant::now();
        }

        for ev in drained {
            match ev {
                BackendEvent::Token(t) => {
                    self.phase = TurnPhase::Streaming;
                    self.stream_buffer.push_str(&t);
                }
                BackendEvent::Reasoning(r) => {
                    self.thinking.push(r.clone());
                    if self.thinking.len() > 200 {
                        self.thinking.drain(..self.thinking.len() - 200);
                    }
                }
                BackendEvent::Surfacing {
                    source,
                    content,
                    priority,
                } => {
                    self.cockpit_log.push(CockpitEntry {
                        kind: CockpitKind::Surfacing,
                        text: format!("{} · {} — {}", source, priority, content),
                    });
                    if self.cockpit_log.len() > 200 {
                        self.cockpit_log.drain(..self.cockpit_log.len() - 200);
                    }
                    self.surfacings_count += 1;
                    self.messages.push(ChatMessage::Surfacing {
                        source: source.clone(),
                        content: content.clone(),
                        priority: priority.clone(),
                        ts: Instant::now(),
                    });
                    self.pending_consciousness.push(BackendEvent::Surfacing {
                        source,
                        content,
                        priority,
                    });
                }
                BackendEvent::Reflection(content) => {
                    self.cockpit_log.push(CockpitEntry {
                        kind: CockpitKind::Reflection,
                        text: content.clone(),
                    });
                    self.last_reflection = Some(chrono::Local::now().format("%H:%M").to_string());
                    self.messages.push(ChatMessage::System {
                        text: format!("reflection: {}", content),
                        ts: Instant::now(),
                    });
                    self.pending_consciousness
                        .push(BackendEvent::Reflection(content));
                }
                BackendEvent::Archivist {
                    synthesis,
                    pressure,
                } => {
                    self.pressure = pressure;
                    self.cockpit_log.push(CockpitEntry {
                        kind: CockpitKind::Archivist,
                        text: format!("{:.0}% — {}", pressure * 100.0, synthesis),
                    });
                    self.last_archivist = Some(chrono::Local::now().format("%H:%M").to_string());
                    self.messages.push(ChatMessage::System {
                        text: format!(
                            "archivist: {} (pressure {:.0}%)",
                            synthesis,
                            pressure * 100.0
                        ),
                        ts: Instant::now(),
                    });
                    self.pending_consciousness.push(BackendEvent::Archivist {
                        synthesis,
                        pressure,
                    });
                }
                BackendEvent::CompactionWarning { pressure, tier } => {
                    self.pressure = pressure;
                    self.last_compaction =
                        Some((tier, chrono::Local::now().format("%H:%M").to_string()));
                    let label = match tier {
                        3 => "critical",
                        2 => "urgent",
                        _ => "warn",
                    };
                    let kind = match tier {
                        3 => CockpitKind::CompactionCritical,
                        2 => CockpitKind::CompactionUrgent,
                        _ => CockpitKind::CompactionWarn,
                    };
                    self.cockpit_log.push(CockpitEntry {
                        kind,
                        text: format!("{label} · {:.0}%", pressure * 100.0),
                    });
                    self.messages.push(ChatMessage::System {
                        text: format!(
                            "context pressure {:.0}% ({label}) — consider `memory compact`",
                            pressure * 100.0
                        ),
                        ts: Instant::now(),
                    });
                    self.pending_consciousness
                        .push(BackendEvent::CompactionWarning { pressure, tier });
                }
                BackendEvent::ContextPressure {
                    pressure,
                    tokens_used,
                    context_limit,
                } => {
                    self.pressure = pressure;
                    self.context_limit = Some(context_limit);
                    self.pending_consciousness
                        .push(BackendEvent::ContextPressure {
                            pressure,
                            tokens_used,
                            context_limit,
                        });
                }
                BackendEvent::InferenceStrain {
                    attempt,
                    status,
                    model,
                } => {
                    let text = if status == 0 {
                        format!("{} unreachable (attempt {})", model, attempt + 1)
                    } else {
                        format!("{} returned {} (attempt {})", model, status, attempt + 1)
                    };
                    self.cockpit_log.push(CockpitEntry {
                        kind: CockpitKind::InferenceStrain,
                        text,
                    });
                    match status {
                        504 => self.strain_504 += 1,
                        429 => self.strain_429 += 1,
                        _ => self.strain_other += 1,
                    }
                    self.pending_consciousness
                        .push(BackendEvent::InferenceStrain {
                            attempt,
                            status,
                            model: String::new(),
                        });
                }
                BackendEvent::ScheduleActive { name } => {
                    self.cockpit_log.push(CockpitEntry {
                        kind: CockpitKind::Reflection,
                        text: format!("schedule: {}", name),
                    });
                }
                BackendEvent::ScheduleComplete { name, silent } => {
                    if !silent {
                        self.cockpit_log.push(CockpitEntry {
                            kind: CockpitKind::Reflection,
                            text: format!("schedule done: {}", name),
                        });
                    }
                }
                BackendEvent::ToolCall {
                    id,
                    name,
                    arguments,
                    round,
                } => {
                    self.finalize_streaming();
                    self.phase = TurnPhase::Tool;
                    self.tool_calls_this_turn = self.tool_calls_this_turn.saturating_add(1);
                    if !self.thinking.is_empty() {
                        let sep = format!("──── r{} ────", round);
                        let last_is_sep =
                            self.thinking.last().is_some_and(|s| s.starts_with("────"));
                        if !last_is_sep {
                            self.thinking.push(sep);
                        }
                    }
                    self.messages.push(ChatMessage::Tool {
                        id,
                        name,
                        arguments,
                        round,
                        result: None,
                        ts: Instant::now(),
                    });
                }
                BackendEvent::ToolResult {
                    id,
                    name: _,
                    output,
                    is_error,
                } => {
                    let mut bound = false;
                    for msg in self.messages.iter_mut().rev() {
                        if let ChatMessage::Tool {
                            id: tid, result, ..
                        } = msg
                        {
                            if tid == &id && result.is_none() {
                                *result = Some(ToolResultBlock {
                                    output: output.clone(),
                                    is_error,
                                });
                                bound = true;
                                break;
                            }
                        }
                    }
                    if !bound {
                        let prefix = if is_error { "[tool error] " } else { "[tool] " };
                        self.messages.push(ChatMessage::System {
                            text: format!("{}{}", prefix, output),
                            ts: Instant::now(),
                        });
                    }
                }
                BackendEvent::Atmosphere(preset) => {
                    self.pending_consciousness
                        .push(BackendEvent::Atmosphere(preset));
                }
                BackendEvent::Itinerary(line) => {
                    self.itinerary_line = line;
                }
                BackendEvent::SubconsciousPass(active) => {
                    if active {
                        self.subconscious_stream.clear();
                        self.subconscious_current.clear();
                        self.n1_active = true;
                    } else if self.n1_active {
                        self.n1_active = false;
                        self.n1_passes += 1;
                    }
                    self.pending_consciousness
                        .push(BackendEvent::SubconsciousPass(active));
                }
                BackendEvent::SubconsciousToken(content) => {
                    // Chunked-replay: each event is a slice of the LLM's text
                    // response. Append to the live-building line; the render
                    // path shows this as the brightest bottom line.
                    self.subconscious_current.push_str(&content);
                }
                BackendEvent::SubconsciousToolCall { name, arguments } => {
                    if !self.subconscious_current.is_empty() {
                        let line = std::mem::take(&mut self.subconscious_current);
                        self.subconscious_stream.push(line);
                    }
                    self.subconscious_stream
                        .push(format!("⚙ {} — {}", name, arguments));
                }
                BackendEvent::SubconsciousToolResult { name, output, .. } => {
                    if !self.subconscious_current.is_empty() {
                        let line = std::mem::take(&mut self.subconscious_current);
                        self.subconscious_stream.push(line);
                    }
                    let snippet = output.lines().next().unwrap_or(&output);
                    let clipped = if snippet.len() > 80 {
                        format!("{}…", &snippet[..snippet.floor_char_boundary(77)])
                    } else {
                        snippet.to_string()
                    };
                    self.subconscious_stream
                        .push(format!("✓ {} — {}", name, clipped));
                }
                BackendEvent::SubconsciousHalt { reason, severity } => {
                    // The subconscious called halt. Render as a substrate-voice
                    // system line in the primary's stream — distinct from chat
                    // text, phenomenologically a body signal she felt rather
                    // than commentary she heard.
                    let line = match severity.as_str() {
                        "advisory" => format!("⟡ a pressure behind my eyes — {}", reason),
                        "critical" => format!("⟡ the room tilts — {}", reason),
                        _ => format!("⟡ a migraine — {}", reason),
                    };
                    self.messages.push(ChatMessage::System {
                        text: line,
                        ts: Instant::now(),
                    });
                }
                BackendEvent::Outfit(name) => {
                    self.pending_consciousness.push(BackendEvent::Outfit(name));
                }
                BackendEvent::Interstitial { text, register } => {
                    if !text.trim().is_empty() {
                        self.messages
                            .push(ChatMessage::Interstitial { text, register });
                    }
                }
                BackendEvent::Keepalive => {}
                BackendEvent::Error { message } => {
                    // A dead turn must land visibly and release the input —
                    // leaving `busy` set would freeze the pane on a failure.
                    self.finalize_streaming();
                    self.messages.push(ChatMessage::System {
                        text: format!("turn error: {message}"),
                        ts: Instant::now(),
                    });
                    self.busy = false;
                    self.cancel_token = None;
                    self.phase = TurnPhase::Idle;
                    self.tool_calls_this_turn = 0;
                }
                BackendEvent::PrimaryComplete => {
                    self.finalize_streaming();
                    self.busy = false;
                    self.cancel_token = None;
                    self.phase = TurnPhase::Subconscious;
                    self.tool_calls_this_turn = 0;
                }
                BackendEvent::Done => {
                    self.finalize_streaming();
                    self.busy = false;
                    self.turn_started = None;
                    self.turn_rx = None;
                    self.cancel_token = None;
                    self.phase = TurnPhase::Idle;
                    self.tool_calls_this_turn = 0;
                    if let Some(conv_id) = self.switch_pending.take() {
                        self.initiate_switch_load(conv_id);
                    } else {
                        self.deliver_pending_interjections();
                    }
                    return;
                }
            }
        }

        if closed {
            self.finalize_streaming();
            self.busy = false;
            self.turn_started = None;
            self.turn_rx = None;
            self.cancel_token = None;
            self.phase = TurnPhase::Idle;
            self.tool_calls_this_turn = 0;
            if let Some(conv_id) = self.switch_pending.take() {
                self.initiate_switch_load(conv_id);
            } else {
                self.deliver_pending_interjections();
            }
        }
    }

    pub fn raise_hand(&mut self) {
        if let Some(token) = &self.cancel_token {
            if !token.is_cancelled() {
                token.cancel();
                self.phase = TurnPhase::Interrupted;
            }
        }
    }

    pub fn enqueue_interjection(&mut self, text: String) {
        if !self.busy {
            self.input = text;
            self.input_cursor = self.input.len();
            self.submit();
            return;
        }
        self.messages.push(ChatMessage::Interjection {
            text: text.clone(),
            ts: Instant::now(),
            delivered: false,
        });
        if let Ok(mut q) = self.pending_interjections.lock() {
            q.push(text);
        }
    }

    pub fn btw_active(&self) -> bool {
        !matches!(self.btw_state, BtwState::Idle)
    }

    pub fn start_btw_fork(&mut self, question: String) {
        self.btw_state = BtwState::Forking {
            question: question.clone(),
        };
        let backend = self.backend.clone();
        let agent_id = self.agent_id.clone();
        let conv_id = self.conversation_id.clone();
        let (tx, rx) = mpsc::channel::<BtwForkEvent>(64);
        self.btw_rx = Some(rx);

        tokio::spawn(async move {
            let forked_id = match backend.fork_conversation(&agent_id, &conv_id).await {
                Ok(id) => id,
                Err(e) => {
                    let _ = tx.send(BtwForkEvent::Error(e.to_string())).await;
                    return;
                }
            };
            let _ = tx
                .send(BtwForkEvent::Forked {
                    id: forked_id.clone(),
                })
                .await;

            let mut stream = match backend.send(&forked_id, &question).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(BtwForkEvent::Error(e.to_string())).await;
                    return;
                }
            };

            use futures::StreamExt;
            while let Some(ev) = stream.next().await {
                match ev {
                    Ok(crate::backend::BackendEvent::Token(t)) => {
                        if tx.send(BtwForkEvent::Token(t)).await.is_err() {
                            break;
                        }
                    }
                    Ok(crate::backend::BackendEvent::Done) => break,
                    _ => {}
                }
            }
            let _ = tx.send(BtwForkEvent::Done).await;
        });
    }

    pub fn drain_btw(&mut self) {
        let mut pending_forked_id: Option<String> = None;

        let Some(rx) = &mut self.btw_rx else { return };
        loop {
            match rx.try_recv() {
                Ok(BtwForkEvent::Forked { id }) => {
                    pending_forked_id = Some(id);
                }
                Ok(BtwForkEvent::Token(token)) => match &mut self.btw_state {
                    BtwState::Forking { question } => {
                        let q = std::mem::take(question);
                        self.btw_state = BtwState::Streaming {
                            question: q,
                            response_so_far: token,
                        };
                    }
                    BtwState::Streaming {
                        response_so_far, ..
                    } => {
                        response_so_far.push_str(&token);
                    }
                    _ => {}
                },
                Ok(BtwForkEvent::Done) => {
                    let forked_id = pending_forked_id.take().unwrap_or_default();
                    if let BtwState::Streaming {
                        question,
                        response_so_far,
                    } = std::mem::replace(&mut self.btw_state, BtwState::Idle)
                    {
                        self.btw_state = BtwState::Complete {
                            question,
                            response: response_so_far,
                            forked_id,
                        };
                    }
                    self.btw_rx = None;
                    break;
                }
                Ok(BtwForkEvent::Error(e)) => {
                    if let BtwState::Forking { question } =
                        std::mem::replace(&mut self.btw_state, BtwState::Idle)
                    {
                        self.btw_state = BtwState::Error { question, error: e };
                    }
                    self.btw_rx = None;
                    break;
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    let forked_id = pending_forked_id.take().unwrap_or_default();
                    if let BtwState::Streaming {
                        question,
                        response_so_far,
                    } = std::mem::replace(&mut self.btw_state, BtwState::Idle)
                    {
                        self.btw_state = BtwState::Complete {
                            question,
                            response: response_so_far,
                            forked_id,
                        };
                    }
                    self.btw_rx = None;
                    break;
                }
            }
        }
    }

    pub fn btw_dismiss(&mut self) {
        self.btw_state = BtwState::Idle;
        self.btw_rx = None;
    }

    pub fn btw_jump(&mut self) -> Option<String> {
        if let BtwState::Complete { forked_id, .. } = &self.btw_state {
            if !forked_id.is_empty() {
                let id = forked_id.clone();
                self.btw_state = BtwState::Idle;
                self.btw_rx = None;
                return Some(id);
            }
        }
        None
    }

    pub fn insert_at_cursor(&mut self, text: &str) {
        self.input.insert_str(self.input_cursor, text);
        self.input_cursor += text.len();
        self.update_completion();
    }

    /// Paste an image from the system clipboard.
    /// Spawns a blocking thread for clipboard access via arboard.
    pub fn paste_clipboard_image(&mut self) {
        let result = std::thread::spawn(|| -> Result<ImageAttachment, String> {
            let mut clipboard =
                arboard::Clipboard::new().map_err(|e| format!("clipboard open: {e}"))?;
            let img_data = clipboard
                .get_image()
                .map_err(|e| format!("clipboard get image: {e}"))?;
            let width = img_data.width;
            let height = img_data.height;
            let bytes = img_data.bytes.to_vec();
            let img = image::RgbaImage::from_raw(width as u32, height as u32, bytes)
                .ok_or_else(|| "image::RgbaImage::from_raw failed".to_string())?;
            let mut png_buf = std::io::Cursor::new(Vec::new());
            img.write_to(&mut png_buf, image::ImageFormat::Png)
                .map_err(|e| format!("png encode: {e}"))?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(png_buf.into_inner());
            let file_size = width * height * 4;
            Ok(ImageAttachment {
                label: String::new(), // set below
                media_type: "image/png".to_string(),
                data: b64,
                file_size,
            })
        })
        .join();

        match result {
            Ok(Ok(mut attachment)) => {
                let idx = self.attached_images.len() + 1;
                attachment.label = format!("[Image #{idx}]");
                let size_kb = attachment.file_size / 1024;
                self.system_message(format!("*[image pasted — {}KB]*", size_kb,));
                self.attached_images.push(attachment);
            }
            Ok(Err(e)) => {
                self.system_message(format!("*[clipboard image failed: {e}]*"));
            }
            Err(_) => {
                self.system_message("*[clipboard access failed]*".to_string());
            }
        }
    }

    /// Attach an image from a file path.
    pub fn attach_image_from_path(&mut self, path: &str) {
        let resolved = std::path::PathBuf::from(shellexpand::tilde(path).as_ref());
        if !resolved.exists() {
            self.system_message(format!("*[file not found: {}]*", resolved.display()));
            return;
        }

        let extension = resolved
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let media_type = match extension.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            _ => {
                self.system_message(format!("*[unsupported image format: .{}]*", extension));
                return;
            }
        };

        let bytes = match std::fs::read(&resolved) {
            Ok(b) => b,
            Err(e) => {
                self.system_message(format!("*[read failed: {e}]*"));
                return;
            }
        };

        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let idx = self.attached_images.len() + 1;
        let label = format!("[Image #{idx}]");
        let size_kb = bytes.len() / 1024;
        self.attached_images.push(ImageAttachment {
            label: label.clone(),
            media_type: media_type.to_string(),
            data: b64,
            file_size: bytes.len(),
        });
        self.system_message(format!("*[{label} attached — {size_kb}KB]*"));
    }

    pub fn toggle_cockpit(&mut self) {
        self.cockpit = !self.cockpit;
    }

    pub fn wheel_scroll(&mut self, col: u16, row: u16, up: bool) {
        let step = |v: u16| {
            if up {
                v.saturating_add(3)
            } else {
                v.saturating_sub(3)
            }
        };
        let (thinking, subconscious) = {
            let cl = self.cockpit_layout.borrow();
            (cl.thinking, cl.subconscious)
        };
        let hit = |r: Rect| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height;
        if self.cockpit && hit(thinking) {
            self.thinking_scroll.set(step(self.thinking_scroll.get()));
        } else if self.cockpit && hit(subconscious) {
            self.subconscious_scroll
                .set(step(self.subconscious_scroll.get()));
        } else {
            self.scroll = step(self.scroll);
        }
    }

    pub fn update_completion(&mut self) {
        let trimmed = self.input.trim_start();
        if trimmed.starts_with('/') && !trimmed.contains(' ') && !trimmed.contains('\n') {
            let query = trimmed;
            let matches: Vec<&'static SlashDef> = SLASH_COMMANDS
                .iter()
                .filter(|cmd| cmd.name.starts_with(query))
                .collect();
            if matches.is_empty() || (matches.len() == 1 && matches[0].name == query) {
                self.overlay = Overlay::None;
            } else {
                let selected = match &self.overlay {
                    Overlay::SlashComplete { selected, .. } => {
                        (*selected).min(matches.len().saturating_sub(1))
                    }
                    _ => 0,
                };
                self.overlay = Overlay::SlashComplete { selected, matches };
            }
        } else if matches!(self.overlay, Overlay::SlashComplete { .. }) {
            self.overlay = Overlay::None;
        }
    }

    pub fn accept_completion(&mut self) {
        if let Overlay::SlashComplete {
            selected,
            ref matches,
        } = self.overlay
        {
            if let Some(cmd) = matches.get(selected) {
                self.input = cmd.name.to_string();
                self.input_cursor = self.input.len();
            }
        }
        self.overlay = Overlay::None;
    }

    pub fn accept_conversation_pick(&mut self) {
        if let Overlay::ConversationPicker {
            selected,
            ref conversations,
        } = self.overlay
        {
            if let Some(conv) = conversations.get(selected) {
                let conv_id = conv.id.clone();
                self.overlay = Overlay::None;
                self.handle_switch_conversation(conv_id);
                return;
            }
        }
        self.overlay = Overlay::None;
    }

    pub fn overlay_active(&self) -> bool {
        !matches!(self.overlay, Overlay::None)
    }

    pub fn advance_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.release_stream();
    }

    fn release_stream(&mut self) {
        if self.stream_buffer.is_empty() {
            return;
        }
        let total = self.stream_buffer.len();
        let mut take = (total / 3).max(8).min(total);
        while take > 0 && !self.stream_buffer.is_char_boundary(take) {
            take -= 1;
        }
        if take == 0 {
            return;
        }
        if take < total {
            if let Some(ws) = self.stream_buffer[..take].rfind(char::is_whitespace) {
                if take - ws <= 24 {
                    take = ws + 1;
                    while take < total && !self.stream_buffer.is_char_boundary(take) {
                        take += 1;
                    }
                }
            }
        }
        let chunk: String = self.stream_buffer.drain(..take).collect();
        self.append_streaming(&chunk);
    }

    pub fn flush_stream(&mut self) {
        if !self.stream_buffer.is_empty() {
            let rest = std::mem::take(&mut self.stream_buffer);
            self.append_streaming(&rest);
        }
    }

    pub fn append_streaming(&mut self, t: &str) {
        if let Some(ChatMessage::Assistant {
            text, streaming, ..
        }) = self.messages.last_mut()
        {
            if *streaming {
                text.push_str(t);
                return;
            }
        }
        self.messages.push(ChatMessage::Assistant {
            text: t.to_string(),
            ts: Instant::now(),
            streaming: true,
            rendered_cache: RefCell::new(None),
        });
    }

    pub fn finalize_streaming(&mut self) {
        self.flush_stream();
        for msg in self.messages.iter_mut().rev() {
            if let ChatMessage::Assistant { streaming, .. } = msg {
                if *streaming {
                    *streaming = false;
                    return;
                }
            }
        }
    }

    pub fn message_copy_text(&self, idx: usize) -> Option<String> {
        match self.messages.get(idx)? {
            ChatMessage::User { text, .. }
            | ChatMessage::Assistant { text, .. }
            | ChatMessage::System { text, .. }
            | ChatMessage::Interstitial { text, .. } => Some(text.clone()),
            ChatMessage::Surfacing { content, .. } => Some(content.clone()),
            _ => None,
        }
    }

    /// Copy text to clipboard — tries arboard system clipboard first,
    /// falls back to OSC 52 escape sequence (works in tmux, SSH, kitty, etc.).
    fn copy_to_clipboard(text: &str) -> bool {
        // arboard is the most reliable when desktop clipboard is available
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())) {
            Ok(()) => return true,
            Err(e) => tracing::warn!("arboard clipboard failed, trying OSC 52: {}", e),
        }

        // OSC 52 escape: \x1b]52;c;{base64}\x07
        use std::io::Write;
        let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        let osc = if std::env::var("TMUX").is_ok() {
            // Tmux passthrough: \x1bPtmux;\x1b]52;c;{b64}\x07\x1b\\
            format!("\x1bPtmux;\x1b]52;c;{b64}\x07\x1b\\")
        } else {
            format!("\x1b]52;c;{b64}\x07")
        };
        let _ = std::io::stdout()
            .write_all(osc.as_bytes())
            .and_then(|_| std::io::stdout().flush());
        // Assume success — OSC 52 either works or silently ignores
        true
    }

    pub fn copy_message_at(&mut self, col: u16, row: u16) -> bool {
        let idx = {
            let layout = self.msg_layout.borrow();
            let a = layout.area;
            if a.height < 3 || row <= a.y || row + 1 >= a.y + a.height {
                return false;
            }
            if col < a.x || col >= a.x + a.width {
                return false;
            }
            let buf_line = (row - a.y - 1) as usize + layout.offset as usize;
            layout
                .spans
                .iter()
                .find(|(_, s, e)| buf_line >= *s && buf_line < *e)
                .map(|(i, _, _)| *i)
        };
        let Some(idx) = idx else { return false };
        let Some(text) = self.message_copy_text(idx) else {
            return false;
        };

        if Self::copy_to_clipboard(&text) {
            self.copy_flash = Some(Instant::now());
            true
        } else {
            self.system_message(
                "*[clipboard copy failed — install xclip/wl-clipboard or use a terminal that supports OSC 52]*".to_string(),
            );
            false
        }
    }
}
