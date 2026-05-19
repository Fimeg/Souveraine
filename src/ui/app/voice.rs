use std::time::Instant;

use super::{App, Screen};
use crate::ui::component::TuiEvent;

impl App {
    pub(super) async fn init_voice_session(&mut self) {
        let cfg = self.config.read().await;
        let vcfg = cfg.voice.clone();
        drop(cfg);

        if !vcfg.enabled {
            return;
        }

        self.voice_client = Some(crate::core::voice::VoiceClient::new(
            &vcfg.stt_url,
            &vcfg.tts_url,
            &vcfg.voice_id,
        ));

        if self.voice_player.is_none() {
            match crate::ui::voice::VoicePlayer::new() {
                Ok(player) => {
                    tracing::info!("VoicePlayer opened — audio output ready");
                    self.voice_player = Some(player);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to open audio output — TTS will be text-only");
                }
            }
        }
    }

    pub(super) fn exit_presence(&mut self) {
        self.voice_capture = None;
        self.voice_stt_rx = None;
        self.voice_tts_rx = None;
        if let Some(player) = &self.voice_player {
            player.stop();
        }
        self.presence.posture = crate::ui::presence::Posture::Idle;
        self.presence.sync_atmosphere_pub();
        self.current_screen = Screen::Welcome;
        self.dispatch(TuiEvent::ScreenChanged(Screen::Welcome));
    }

    pub(super) fn start_listening(&mut self) {
        self.voice_capture = None;

        match crate::ui::voice::MicCapture::start(16_000) {
            Ok(cap) => {
                self.voice_capture = Some(cap);
                self.presence.set_posture(crate::ui::presence::Posture::Listening);
                tracing::info!("mic capture started — listening");
            }
            Err(e) => {
                tracing::warn!(error = %e, "mic capture failed — injecting error message");
                if let Some(chat) = self.chat.as_mut() {
                    chat.input = "*[mic unavailable — voice channel unreachable]*".to_string();
                    chat.submit();
                }
            }
        }
    }

    pub(super) async fn handle_presence_space_release(&mut self) {
        if self.presence.posture != crate::ui::presence::Posture::Listening {
            return;
        }

        let capture = match self.voice_capture.take() {
            Some(c) => c,
            None => return,
        };

        self.presence.set_posture(crate::ui::presence::Posture::Thinking);

        let samples = capture.stop_and_take();

        if samples.is_empty() {
            tracing::info!("empty mic capture — injecting empty utterance");
            if let Some(chat) = self.chat.as_mut() {
                chat.input = "*[empty utterance]*".to_string();
                chat.submit();
            }
            self.presence.set_posture(crate::ui::presence::Posture::Processing);
            return;
        }

        let wav = match crate::ui::voice::capture::samples_to_wav(&samples, 16_000) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "WAV encode failed");
                if let Some(chat) = self.chat.as_mut() {
                    chat.input = "*[voice service unreachable]*".to_string();
                    chat.submit();
                }
                self.presence.set_posture(crate::ui::presence::Posture::Processing);
                return;
            }
        };

        let client = match self.voice_client.as_ref() {
            Some(c) => {
                let stt_url = c.stt_url_str().to_string();
                let tts_url = c.tts_url_str().to_string();
                let voice = c.voice_str().to_string();
                (stt_url, tts_url, voice)
            }
            None => {
                return;
            }
        };

        let (stt_url, _tts_url, _voice) = client;

        let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
        let stt_u = stt_url.clone();
        tokio::spawn(async move {
            let c = crate::core::voice::VoiceClient::new(&stt_u, "", "");
            let result = c.transcribe(wav).await
                .map_err(|e| format!("*[voice service unreachable — {}]*", e));
            let _ = tx.send(result);
        });

        self.voice_stt_rx = Some(rx);
    }

    pub(super) async fn advance_voice_pipeline(&mut self) {
        if let Some(rx) = self.voice_stt_rx.as_mut() {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.voice_stt_rx = None;
                    let text = "*[voice service unreachable — STT task failed]*".to_string();
                    if let Some(chat) = self.chat.as_mut() {
                        chat.input = text;
                        chat.submit();
                    }
                    self.presence.set_posture(crate::ui::presence::Posture::Straining);
                    return;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    return;
                }
            };
            self.voice_stt_rx = None;

            let text = match result {
                Ok(t) if t.is_empty() => "*[empty utterance]*".to_string(),
                Ok(t) => t,
                Err(e) => {
                    self.presence.set_posture(crate::ui::presence::Posture::Straining);
                    e
                }
            };

            tracing::info!(transcript = %text, "STT received");

            self.voice_last_transcript = Some(text.clone());
            self.voice_last_synthesized = None;

            if let Some(chat) = self.chat.as_mut() {
                chat.input = text.clone();
                chat.submit();
            }

            self.presence.set_posture(crate::ui::presence::Posture::Processing);
        }

        if let Some(rx) = self.voice_tts_rx.as_mut() {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.voice_tts_rx = None;
                    tracing::warn!("TTS task died without sending a result");
                    self.presence.set_posture(crate::ui::presence::Posture::Idle);
                    return;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    return;
                }
            };
            self.voice_tts_rx = None;

            let tts_text = self.voice_last_synthesized.clone();

            match result {
                Ok(mp3_bytes) => {
                    tracing::info!(bytes = mp3_bytes.len(), "TTS bytes received — attempting playback");
                    if let Some(text) = tts_text {
                        self.voice_last_tts_text = Some(text.clone());
                    }
                    self.voice_last_tts_bytes = Some(mp3_bytes.clone());
                    self.voice_last_tts_time = None;

                    self.presence.set_posture(crate::ui::presence::Posture::Speaking);
                    if let Some(player) = &self.voice_player {
                        if let Err(e) = player.play_mp3(mp3_bytes) {
                            tracing::warn!(error = %e, "mp3 playback failed");
                            self.presence.set_posture(crate::ui::presence::Posture::Idle);
                        }
                    } else {
                        tracing::warn!("TTS bytes ready but no VoicePlayer — audio device unavailable");
                        self.presence.set_posture(crate::ui::presence::Posture::Idle);
                    }
                }
                Err(e) => {
                    tracing::warn!("TTS synthesis failed: {}", e);
                    self.presence.set_posture(crate::ui::presence::Posture::Idle);
                }
            }
        }

        if self.presence.posture == crate::ui::presence::Posture::Speaking {
            let done = self.voice_player
                .as_ref()
                .map(|p| !p.is_speaking())
                .unwrap_or(true);
            if done {
                self.presence.set_posture(crate::ui::presence::Posture::Idle);
            }
        }

        if self.presence.posture == crate::ui::presence::Posture::Listening {
            if let Some(cap) = &self.voice_capture {
                let level = cap.current_level();
                self.voice_waveform.push(level);
                if self.voice_waveform.len() > 128 {
                    self.voice_waveform.remove(0);
                }
            }
        } else if !self.voice_waveform.is_empty() {
        }

        if self.presence.posture == crate::ui::presence::Posture::Speaking {
            let done = self.voice_player
                .as_ref()
                .map(|p| !p.is_speaking())
                .unwrap_or(true);
            if done && self.voice_last_tts_time.is_none() {
                self.voice_last_tts_time = Some(Instant::now());
            }
        }

        if self.voice_tts_rx.is_none()
            && self.voice_client.is_some()
            && !matches!(self.presence.posture,
                crate::ui::presence::Posture::Listening
                | crate::ui::presence::Posture::Speaking)
        {
            let regen_text = self.tts_last_text.take();

            let maybe_reply = regen_text.or_else(|| {
                self.chat.as_ref().and_then(|c| {
                    if !c.busy {
                        c.messages.iter().rev().find_map(|m| {
                            match m {
                                crate::ui::chat::ChatMessage::Assistant { text, streaming: false, .. }
                                    if !text.is_empty() => Some(text.clone()),
                                _ => None,
                            }
                        })
                    } else {
                        None
                    }
                })
            });

            if let Some(reply) = maybe_reply {
                let already_synthesized = self.voice_last_synthesized.as_deref() == Some(&reply);
                if !already_synthesized {
                    let preview = if reply.len() > 80 { &reply[..80] } else { &reply };
                    tracing::info!(text = %preview, "TTS trigger — synthesizing reply");
                    self.voice_last_synthesized = Some(reply.clone());

                    let tts_url = self.voice_client.as_ref()
                        .map(|c| c.tts_url_str().to_string())
                        .unwrap_or_default();
                    let voice = self.voice_client.as_ref()
                        .map(|c| c.voice_str().to_string())
                        .unwrap_or_default();

                    let (tx, rx) = tokio::sync::oneshot::channel::<Result<Vec<u8>, String>>();
                    let reply_for_bytes = reply.clone();
                    tokio::spawn(async move {
                        let c = crate::core::voice::VoiceClient::new("", &tts_url, &voice);
                        let result = c.synthesize(&reply_for_bytes).await
                            .map_err(|e| e.to_string());
                        let _ = tx.send(result);
                    });
                    self.voice_tts_rx = Some(rx);
                }
            }
        }
    }
}
