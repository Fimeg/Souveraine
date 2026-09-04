#![allow(dead_code)] // WIP scaffolding not yet wired
//! Session — conversation message types and persistence
//!
//! Defines the message model shared across all Souveraine interfaces:
//! TUI, CLI, Web API, and subagent forks.
//!
//! Uses serde for serialization (unlike claw-code's custom JSON).
//! Persists sessions as JSON files in the agent's memory directory.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Role of a message participant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A single content block within a message
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
    ToolResult {
        tool_use_id: String,
        tool_name: String,
        output: String,
        is_error: bool,
    },
    Reasoning {
        reasoning: String,
    },
    Image {
        media_type: String,
        data: String,
    },
}

impl ContentBlock {
    /// Everything this block contributes to the context window, whatever its
    /// kind. This is the single authority for "how much room does this take" —
    /// every token counter in the system routes through it.
    ///
    /// The match is deliberately exhaustive with **no wildcard arm**. A new
    /// block kind must fail to compile here rather than quietly weigh nothing.
    /// That is not stylistic: `count_messages` in the compaction engine carried
    /// `_ => None`, so it measured Text only — 18% of a real conversation — and
    /// microcompact, whose entire job is clearing old *tool results*, concluded
    /// there was nothing to reclaim and declined to run. A counter blind to the
    /// blocks its caller exists to act on reports success while doing nothing.
    ///
    /// Known caveat, deliberately preserved rather than silently changed:
    /// `Image` counts its base64 payload as text, which vastly overstates a
    /// real image's token cost. Fixing that is a model-specific estimate and a
    /// separate decision — see the image-token follow-up. Unifying the counters
    /// and re-weighting images are two changes; doing both at once would make
    /// neither reviewable.
    pub fn countable_text(&self) -> std::borrow::Cow<'_, str> {
        use std::borrow::Cow;
        match self {
            ContentBlock::Text { text } => Cow::Borrowed(text),
            ContentBlock::ToolUse { id, name, input } => Cow::Owned(format!("{id} {name} {input}")),
            ContentBlock::ToolResult {
                tool_use_id,
                tool_name,
                output,
                ..
            } => Cow::Owned(format!("{tool_use_id} {tool_name} {output}")),
            ContentBlock::Reasoning { reasoning } => Cow::Borrowed(reasoning),
            ContentBlock::Image { media_type, data } => Cow::Owned(format!("{media_type} {data}")),
        }
    }

    /// The prose a block may take when replayed as cross-turn model history —
    /// the one authority for that lossy shape, as `countable_text` is for
    /// weight. This is not the transcript/API projection: surfaces receive the
    /// stored typed blocks verbatim.
    ///
    /// Complete tool rounds are projected natively by `replay_messages`, not
    /// through this method. This prose fallback exists for text and evidence
    /// that cannot be bound into a valid tool round. A `role=tool` message
    /// without a `tool_call_id` is rejected by OpenAI-shaped providers, and a
    /// persisted `ToolUse` whose result never landed — a turn killed
    /// mid-round, which has happened twice — must be repaired rather than sent
    /// as an orphaned half of a pair.
    ///
    /// Dropping these blocks instead of flattening them is what made a turn's
    /// own tool work invisible to the turn after it, and left microcompact —
    /// whose entire job is blurring old `ToolResult` output — with nothing in
    /// the payload to blur.
    ///
    /// `None` means the block has no honest prose form. Images are carried
    /// natively by callers that can send them and degraded by those that
    /// cannot. Reasoning remains a typed persisted block for audit and surface
    /// hydration, but is omitted from generic model replay: Anthropic thinking
    /// needs its original signature, and making `[Reasoning]: ...` assistant
    /// prose teaches the next model to emit internal thought as visible text.
    pub fn replay_text(&self) -> Option<String> {
        match self {
            ContentBlock::Text { text } => Some(text.clone()),
            ContentBlock::ToolUse { name, input, .. } => Some(format!("Tool use: {name}({input})")),
            ContentBlock::ToolResult {
                tool_name,
                output,
                is_error,
                ..
            } => Some(if *is_error {
                format!("Error ({tool_name}): {output}")
            } else {
                format!("Result ({tool_name}): {output}")
            }),
            ContentBlock::Reasoning { .. } => None,
            ContentBlock::Image { .. } => None,
        }
    }
}

/// An image attached to the current input, before submission.
/// Used by the TUI input pipeline and carried through to the Backend trait.
#[derive(Debug, Clone)]
pub struct ImageAttachment {
    /// Display label (e.g. "[Image #1]").
    pub label: String,
    /// MIME type (e.g. "image/png").
    pub media_type: String,
    /// Base64-encoded image data.
    pub data: String,
    /// File size in bytes before encoding.
    pub file_size: usize,
}

/// Token usage metadata
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

impl TokenUsage {
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }
}

/// A single message in a conversation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: MessageRole,
    pub blocks: Vec<ContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
}

impl ConversationMessage {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            blocks: vec![ContentBlock::Text { text: text.into() }],
            usage: None,
            timestamp: Some(Utc::now()),
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            blocks: vec![ContentBlock::Text { text: text.into() }],
            usage: None,
            timestamp: Some(Utc::now()),
        }
    }

    pub fn assistant_with_usage(blocks: Vec<ContentBlock>, usage: Option<TokenUsage>) -> Self {
        Self {
            role: MessageRole::Assistant,
            blocks,
            usage,
            timestamp: Some(Utc::now()),
        }
    }

    pub fn tool_result(
        tool_use_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            role: MessageRole::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                tool_name: tool_name.into(),
                output: output.into(),
                is_error,
            }],
            usage: None,
            timestamp: Some(Utc::now()),
        }
    }

    /// User message with text and image content blocks.
    pub fn user_with_images(text: impl Into<String>, images: Vec<ContentBlock>) -> Self {
        let mut blocks = vec![ContentBlock::Text { text: text.into() }];
        blocks.extend(images);
        Self {
            role: MessageRole::User,
            blocks,
            usage: None,
            timestamp: Some(Utc::now()),
        }
    }
}

/// A complete conversation session
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub messages: Vec<ConversationMessage>,
    pub agent_name: String,
    pub conversation_id: String,
}

impl Session {
    pub fn new(agent_name: &str) -> Self {
        let conversation_id = uuid::Uuid::new_v4().to_string();
        Self {
            version: 1,
            messages: Vec::new(),
            agent_name: agent_name.to_string(),
            conversation_id,
        }
    }

    pub fn with_id(agent_name: &str, conversation_id: &str) -> Self {
        Self {
            version: 1,
            messages: Vec::new(),
            agent_name: agent_name.to_string(),
            conversation_id: conversation_id.to_string(),
        }
    }

    pub fn add_message(&mut self, message: ConversationMessage) {
        self.messages.push(message);
    }

    /// Serialize to pretty JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON string
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Estimate token count (chars/4 heuristic, use for pre-tiktoken estimation)
    pub fn estimate_tokens(&self) -> usize {
        self.messages
            .iter()
            .map(|m| {
                m.blocks
                    .iter()
                    .map(|b| match b {
                        ContentBlock::Text { text } => text.len() / 4 + 1,
                        ContentBlock::ToolUse { name, input, .. } => {
                            (name.len() + input.len()) / 4 + 1
                        }
                        ContentBlock::ToolResult {
                            tool_name, output, ..
                        } => (tool_name.len() + output.len()) / 4 + 1,
                        ContentBlock::Reasoning { reasoning } => reasoning.len() / 4 + 1,
                        ContentBlock::Image { data, .. } => data.len() / 4 + 1,
                    })
                    .sum::<usize>()
            })
            .sum()
    }

    /// Save session to a directory
    pub async fn save_to_dir(&self, dir: &std::path::Path) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(dir).await?;
        let path = dir.join(format!("{}.json", self.conversation_id));
        let json = self.to_json()?;
        tokio::fs::write(&path, json).await?;
        debug!("Session saved: {}", path.display());
        Ok(())
    }

    /// Load session from a directory by conversation ID
    pub async fn load_from_dir(
        dir: &std::path::Path,
        conversation_id: &str,
    ) -> anyhow::Result<Option<Self>> {
        let path = dir.join(format!("{conversation_id}.json"));
        if !path.exists() {
            return Ok(None);
        }
        let json = tokio::fs::read_to_string(&path).await?;
        let session = Self::from_json(&json)?;
        Ok(Some(session))
    }

    /// Convert to the OpenAI wire message format.
    ///
    /// Delegates to [`replay_messages`] — the one projection of stored history
    /// onto the wire. This wrapper degrades images, because the callers that
    /// can see natively build their own policy from the agent's config.
    pub fn to_wire_messages(&self) -> Vec<crate::bridge::openai_compatible::Message> {
        replay_messages(&self.messages, ImagePolicy::Degrade)
    }
}

/// How a window carries an image.
///
/// The only thing that legitimately varies between callers replaying history.
/// Everything else about the record is the same record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImagePolicy {
    /// The model can see. Images ride as native multipart content.
    Native,
    /// The model cannot see. Images become a marker saying so plainly, rather
    /// than a stub shaped like success.
    Degrade,
}

/// What a tool call whose result never arrived replays as.
///
/// A turn killed mid-round leaves a call with no answer. Omitting it breaks
/// the wire's pairing rule; sending it with silence behind it shows the model
/// a call that produced nothing, which is the shape of a broken sensor. Saying
/// what actually happened is both valid and true.
const UNFINISHED_CALL: &str =
    "This call did not complete — the turn ended before its result arrived.";

/// The one projection of stored history onto the wire.
///
/// There is a single history. Both modes — her, and the subconscious a moment
/// later — replay the same record; a terminal, a voice and a web surface are
/// windows onto the same conversation. What varies is only what the glass can
/// carry, which is `images`.
///
/// **A tool round replays as a tool round.** An assistant message that called
/// tools comes back as `tool_calls` with `role: "tool"` results bound to it by
/// id — the same shape the live loop builds, because it is the same event.
/// Flattening a call into assistant prose falsifies the record: it tells the
/// model *she said* `Tool use: bash(…)` when in fact she *did* it. She reads
/// her own transcript, finds that sentence attributed to herself, and
/// reasonably concludes that writing it is how the thing is done — so she
/// writes it, and no tool runs. That happened on 2026-08-13, within an hour of
/// tool blocks first surviving replay at all.
///
/// **One wire message per stored message**, outside a tool round. Exploding a
/// message into one message per block splits a single assistant turn that
/// called several tools into adjacent assistant messages, which OpenAI-shaped
/// providers reject.
///
/// **Pairs are repaired, never half-sent.** A call with no result, or a result
/// whose call was compacted away, is rejected outright. Unanswered calls get
/// an explicit unfinished result; orphaned results, which can be bound to
/// nothing, degrade to prose rather than being dropped.
///
/// Reasoning stays typed in the stored record and in transcript APIs, but does
/// not enter this generic model projection. Anthropic requires a replayed
/// thinking block to carry its original signature and
/// `ContentBlock::Reasoning` does not persist one, so there is no honest native
/// form here. Flattening it as `[Reasoning]: ...` assistant prose is worse than
/// omission: the next model can imitate that label in visible output. Signed
/// thinking needed by the live Anthropic loop remains a separate typed field.
pub fn replay_messages(
    messages: &[ConversationMessage],
    images: ImagePolicy,
) -> Vec<crate::bridge::openai_compatible::Message> {
    use crate::bridge::openai_compatible::{ContentPart, ImageUrlSource, Message, MessageToolCall};
    use std::collections::HashSet;

    let ids = |f: fn(&ContentBlock) -> Option<&str>| -> HashSet<&str> {
        messages
            .iter()
            .flat_map(|m| m.blocks.iter())
            .filter_map(f)
            .collect()
    };
    let called = ids(|b| match b {
        ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
        _ => None,
    });
    let answered = ids(|b| match b {
        ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
        _ => None,
    });

    // Prose for the blocks around a tool round. A degraded image is named as
    // unseen rather than dropped — a model told it did not look can say so.
    fn prose_of<'a>(blocks: impl Iterator<Item = &'a ContentBlock>) -> String {
        blocks
            .filter_map(|b| match b {
                ContentBlock::Image { media_type, .. } => {
                    Some(format!("[Image: {media_type} — not visible to this model]"))
                }
                other => other.replay_text(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    let mut out: Vec<Message> = Vec::new();

    for m in messages {
        let role = match m.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            // A legacy text block stored under Tool has no call to bind to.
            MessageRole::Tool => "assistant",
        };

        // A whole round can live in one stored message. `turn.rs` commits the
        // entire turn as a single assistant message carrying
        // `[Reasoning, ToolUse, ToolResult, ToolUse, ToolResult, …]`, while the
        // subconscious stores the same round split across messages. Both
        // shapes must produce the same wire, so calls and results are handled
        // together rather than in two branches — handling them separately
        // emitted `tool_calls` with nothing answering them for every turn the
        // primary has ever taken, which every provider rejects.
        let calls: Vec<(&String, &String, &String)> = m
            .blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => Some((id, name, input)),
                _ => None,
            })
            .collect();

        let results: Vec<(&String, &String, &String, bool)> = m
            .blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    tool_name,
                    output,
                    is_error,
                } => Some((tool_use_id, tool_name, output, *is_error)),
                _ => None,
            })
            .collect();

        if !calls.is_empty() || !results.is_empty() {
            let local: std::collections::HashSet<&str> =
                calls.iter().map(|(id, _, _)| id.as_str()).collect();
            let mut stranded: Vec<String> = Vec::new();

            // A result answering a call made in an *earlier* message goes out
            // first, so it still directly follows the turn that made it.
            for (id, name, output, is_error) in &results {
                if local.contains(id.as_str()) {
                    continue;
                }
                if called.contains(id.as_str()) {
                    out.push(Message::tool_result(*id, *name, *output));
                } else if *is_error {
                    stranded.push(format!("Error ({name}): {output}"));
                } else {
                    stranded.push(format!("Result ({name}): {output}"));
                }
            }

            if !calls.is_empty() {
                let tool_calls: Vec<MessageToolCall> = calls
                    .iter()
                    .map(|(id, name, input)| {
                        MessageToolCall::function((*id).clone(), (*name).clone(), (*input).clone())
                    })
                    .collect();
                // Prose beside the calls rides as the assistant's own content.
                let content = prose_of(m.blocks.iter().filter(|b| {
                    !matches!(
                        b,
                        ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. }
                    )
                }));
                out.push(Message::assistant_tool_calls(content, tool_calls));

                for (id, name, _) in &calls {
                    match results.iter().find(|(rid, ..)| rid.as_str() == id.as_str()) {
                        Some((rid, rname, routput, _)) => {
                            out.push(Message::tool_result(*rid, *rname, *routput))
                        }
                        // Answered in a later message — that message emits it.
                        None if answered.contains(id.as_str()) => {}
                        None => out.push(Message::tool_result(*id, *name, UNFINISHED_CALL)),
                    }
                }
            }

            // Prose last, and only when it has not already ridden with the
            // calls: a text message between a call and its result breaks the
            // run. By here every pair this message opened is closed.
            let mut rest = if calls.is_empty() {
                prose_of(
                    m.blocks
                        .iter()
                        .filter(|b| !matches!(b, ContentBlock::ToolResult { .. })),
                )
            } else {
                String::new()
            };
            // A result whose call is gone — compacted away, most likely — can
            // be bound to nothing, so it carries as prose or not at all.
            for s in stranded {
                if !rest.is_empty() {
                    rest.push('\n');
                }
                rest.push_str(&s);
            }
            if !rest.is_empty() {
                out.push(Message::text(role, rest));
            }
            continue;
        }
        // ── an ordinary message ──────────────────────────────────────
        let has_images = m
            .blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Image { .. }));

        if has_images && images == ImagePolicy::Native {
            // Every non-image block still contributes its prose, so an image
            // in a message never costs the work beside it.
            let parts: Vec<ContentPart> = m
                .blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Image { media_type, data } => Some(ContentPart::ImageUrl {
                        image_url: ImageUrlSource {
                            url: format!("data:{media_type};base64,{data}"),
                        },
                    }),
                    other => other.replay_text().map(|text| ContentPart::Text { text }),
                })
                .collect();
            out.push(Message::multimodal(role, parts));
            continue;
        }

        let prose = prose_of(m.blocks.iter());
        if !prose.is_empty() {
            out.push(Message::text(role, prose));
        }
    }

    out
}

impl Default for Session {
    fn default() -> Self {
        Self::new("system")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this guards: cross-turn replay kept `Text` and dropped
    /// everything else, so a turn's own tool work was invisible to the turn
    /// after it — and microcompact, whose whole job is blurring old
    /// `ToolResult` output, had nothing in the payload left to blur.
    ///
    /// Images and reasoning have no generic cross-turn prose. Images are
    /// carried natively or degraded by the caller. Reasoning stays typed in
    /// the stored record and transcript API; without a provider-valid signed
    /// thinking block it must not masquerade as visible assistant speech.
    #[test]
    fn only_speech_and_tool_evidence_have_cross_turn_prose() {
        let blocks = vec![
            ContentBlock::Text {
                text: "plain".into(),
            },
            ContentBlock::ToolUse {
                id: "call-1".into(),
                name: "read".into(),
                input: r#"{"path":"a"}"#.into(),
            },
            ContentBlock::ToolResult {
                tool_use_id: "call-1".into(),
                tool_name: "read".into(),
                output: "file body".into(),
                is_error: false,
            },
        ];

        for b in &blocks {
            assert!(
                b.replay_text().is_some(),
                "{b:?} produced no replay prose — cross-turn history would \
                 drop it and the turn after would not know it happened"
            );
        }

        assert!(ContentBlock::Reasoning {
            reasoning: "weighing it".into(),
        }
        .replay_text()
        .is_none());

        assert!(ContentBlock::Image {
            media_type: "image/png".into(),
            data: "AAAA".into(),
        }
        .replay_text()
        .is_none());
    }

    /// The regression visible in the Panel in August 2026. Replay used to
    /// convert a typed reasoning block into `[Reasoning]: ...` assistant prose.
    /// The next model imitated that text, so the renderer quite correctly drew
    /// it as speech rather than a ThinkingCard. Keep the block on disk, but do
    /// not feed an unsigned thought back as something she said.
    #[test]
    fn reasoning_never_replays_as_visible_assistant_prose() {
        let stored = vec![ConversationMessage {
            role: MessageRole::Assistant,
            blocks: vec![
                ContentBlock::Reasoning {
                    reasoning: "private chain".into(),
                },
                ContentBlock::Text {
                    text: "the answer".into(),
                },
            ],
            usage: None,
            timestamp: None,
        }];

        for policy in [ImagePolicy::Native, ImagePolicy::Degrade] {
            let messages = replay_messages(&stored, policy);
            assert_eq!(messages.len(), 1, "{policy:?}: {messages:?}");
            let prose = messages[0].content.as_text();
            assert_eq!(prose, "the answer");
            assert!(!prose.contains("Reasoning"), "{policy:?}: {prose}");
            assert!(!prose.contains("private chain"), "{policy:?}: {prose}");
        }

        let reasoning_only = vec![ConversationMessage {
            role: MessageRole::Assistant,
            blocks: vec![ContentBlock::Reasoning {
                reasoning: "still private".into(),
            }],
            usage: None,
            timestamp: None,
        }];
        assert!(replay_messages(&reasoning_only, ImagePolicy::Degrade).is_empty());
    }

    #[test]
    fn replay_prose_carries_the_tool_name_and_its_output() {
        let call = ContentBlock::ToolUse {
            id: "call-1".into(),
            name: "grep".into(),
            input: r#"{"pattern":"halt"}"#.into(),
        }
        .replay_text()
        .expect("tool use must replay");
        assert!(call.contains("grep"), "{call}");
        assert!(call.contains("halt"), "{call}");

        let ok = ContentBlock::ToolResult {
            tool_use_id: "call-1".into(),
            tool_name: "grep".into(),
            output: "three matches".into(),
            is_error: false,
        }
        .replay_text()
        .expect("tool result must replay");
        assert!(ok.contains("three matches"), "{ok}");
        assert!(!ok.contains("Error"), "{ok}");

        // A failure must not replay as though it succeeded — the turn after
        // has to be able to tell that the sensor resisted.
        let bad = ContentBlock::ToolResult {
            tool_use_id: "call-2".into(),
            tool_name: "read".into(),
            output: "No such file".into(),
            is_error: true,
        }
        .replay_text()
        .expect("errored tool result must replay");
        assert!(bad.contains("Error"), "{bad}");
        assert!(bad.contains("No such file"), "{bad}");
    }

    #[test]
    fn test_session_create_and_serialize() {
        let mut session = Session::new("ani");
        session.add_message(ConversationMessage::user_text("hello"));
        session.add_message(ConversationMessage::assistant_text("hi there"));
        let json = session.to_json().unwrap();
        let restored: Session = Session::from_json(&json).unwrap();
        assert_eq!(restored.messages.len(), 2);
        assert_eq!(restored.agent_name, "ani");
    }

    #[test]
    fn test_wire_conversion() {
        let mut session = Session::new("ani");
        session.add_message(ConversationMessage::user_text("hello"));
        let msgs = session.to_wire_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content.as_text(), "hello");
    }

    #[test]
    fn cross_turn_tool_history_keeps_both_halves_of_every_pair() {
        let mut session = Session::new("subconscious");
        session.add_message(ConversationMessage {
            role: MessageRole::Assistant,
            blocks: vec![
                ContentBlock::ToolUse {
                    id: "call-a".into(),
                    name: "read".into(),
                    input: r#"{"path":"a"}"#.into(),
                },
                ContentBlock::ToolUse {
                    id: "call-b".into(),
                    name: "read".into(),
                    input: r#"{"path":"b"}"#.into(),
                },
            ],
            usage: None,
            timestamp: None,
        });
        session.add_message(ConversationMessage::tool_result(
            "call-a", "read", "A", false,
        ));
        // Simulate stale persisted damage: call-b never received a result.
        session.add_message(ConversationMessage {
            role: MessageRole::Tool,
            blocks: vec![ContentBlock::Text {
                text: "legacy tool text".into(),
            }],
            usage: None,
            timestamp: None,
        });

        let msgs = session.to_wire_messages();

        // Every call is declared, and every tool message answers a declared
        // call. A half-pair in either direction is rejected by the wire.
        let declared: std::collections::HashSet<&str> = msgs
            .iter()
            .filter_map(|m| m.tool_calls.as_ref())
            .flatten()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(declared.len(), 2, "both calls must be declared");
        for m in msgs.iter().filter(|m| m.role == "tool") {
            let id = m.tool_call_id.as_deref().expect("tool message needs an id");
            assert!(declared.contains(id), "unbound tool result: {id}");
        }

        // call-b never received one, so the record says so rather than
        // leaving a call standing in silence.
        let unfinished = msgs
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("call-b"))
            .expect("an unanswered call must still be answered");
        assert!(unfinished.content.as_text().contains("did not complete"));

        // A legacy text block stored under Tool has no call to bind to.
        assert!(msgs
            .iter()
            .any(|m| m.role == "assistant" && m.content.as_text().contains("legacy tool text")));
    }

    /// The regression of 2026-08-13: a tool call rendered as assistant prose
    /// is a sentence the model can read as instructions to itself. Annie began
    /// typing `Tool use: bash({...})` as text within an hour of tool blocks
    /// first surviving replay, and no tool ran.
    #[test]
    fn a_completed_round_never_replays_as_prose_she_can_imitate() {
        let stored = vec![
            ConversationMessage {
                role: MessageRole::Assistant,
                blocks: vec![ContentBlock::ToolUse {
                    id: "call-1".into(),
                    name: "bash".into(),
                    input: r#"{"command":"uptime"}"#.into(),
                }],
                usage: None,
                timestamp: None,
            },
            ConversationMessage::tool_result("call-1", "bash", "up 14h", false),
        ];

        for policy in [ImagePolicy::Native, ImagePolicy::Degrade] {
            let msgs = replay_messages(&stored, policy);
            for m in &msgs {
                let body = m.content.as_text();
                assert!(
                    !body.contains("Tool use:"),
                    "{policy:?} put a call in prose she will imitate: {body}"
                );
            }
            // The work itself is not lost — it is carried natively.
            assert!(msgs.iter().any(|m| m
                .tool_calls
                .as_ref()
                .is_some_and(|c| c[0].function.name == "bash")));
            assert!(msgs
                .iter()
                .any(|m| m.role == "tool" && m.content.as_text().contains("up 14h")));
        }
    }

    /// The shape `turn.rs` actually commits: one assistant message carrying
    /// the whole round, calls and results interleaved. Every earlier test
    /// built the *split* shape through `ConversationMessage::tool_result`,
    /// which nothing outside the test module calls — so a projection that
    /// emitted `tool_calls` with nothing answering them passed the whole
    /// suite and would have been rejected by every provider on the first
    /// real turn (2026-08-14).
    #[test]
    fn a_whole_round_stored_in_one_message_still_pairs_on_the_wire() {
        let stored = vec![ConversationMessage {
            role: MessageRole::Assistant,
            blocks: vec![
                ContentBlock::Reasoning {
                    reasoning: "checking two things".into(),
                },
                ContentBlock::ToolUse {
                    id: "call-a".into(),
                    name: "bash".into(),
                    input: r#"{"command":"uptime"}"#.into(),
                },
                ContentBlock::ToolResult {
                    tool_use_id: "call-a".into(),
                    tool_name: "bash".into(),
                    output: "up 14h".into(),
                    is_error: false,
                },
                ContentBlock::ToolUse {
                    id: "call-b".into(),
                    name: "read".into(),
                    input: r#"{"path":"x"}"#.into(),
                },
                ContentBlock::ToolResult {
                    tool_use_id: "call-b".into(),
                    tool_name: "read".into(),
                    output: "Error: No such file".into(),
                    is_error: true,
                },
                ContentBlock::Text {
                    text: "both done".into(),
                },
            ],
            usage: None,
            timestamp: None,
        }];

        for policy in [ImagePolicy::Native, ImagePolicy::Degrade] {
            let msgs = replay_messages(&stored, policy);

            let declared: Vec<String> = msgs
                .iter()
                .filter_map(|m| m.tool_calls.as_ref())
                .flatten()
                .map(|c| c.id.clone())
                .collect();
            assert_eq!(declared.len(), 2, "{policy:?}: {msgs:?}");

            let answered: Vec<String> = msgs
                .iter()
                .filter(|m| m.role == "tool")
                .filter_map(|m| m.tool_call_id.clone())
                .collect();
            for id in &declared {
                assert!(
                    answered.contains(id),
                    "{policy:?}: call {id} was declared and never answered — \
                     the provider rejects the whole request"
                );
            }

            // Each result directly follows the turn that declared it: no
            // ordinary message may sit between the calls and their answers.
            let call_at = msgs.iter().position(|m| m.tool_calls.is_some()).unwrap();
            for (i, m) in msgs.iter().enumerate().skip(call_at + 1).take(2) {
                assert_eq!(m.role, "tool", "{policy:?}: message {i} broke the run");
            }

            // The prose beside the round rides with it, and a failed sensor
            // still reads as a failure.
            assert!(msgs[call_at].content.as_text().contains("both done"));
            assert!(msgs
                .iter()
                .any(|m| m.content.as_text().contains("No such file")));
        }
    }
    /// A result whose call was compacted away can be bound to nothing, so it
    /// carries as prose rather than being dropped or sent unbound.
    #[test]
    fn a_result_whose_call_is_gone_carries_as_prose() {
        let stored = vec![ConversationMessage::tool_result(
            "long-gone",
            "grep",
            "three matches",
            false,
        )];
        let msgs = replay_messages(&stored, ImagePolicy::Degrade);
        assert!(msgs.iter().all(|m| m.role != "tool"), "would be unbound");
        assert!(msgs
            .iter()
            .any(|m| m.content.as_text().contains("three matches")));
    }

    /// One stored message becomes exactly one wire message.
    ///
    /// The old per-block projection split a single assistant turn that called
    /// several tools into adjacent assistant messages, which OpenAI-shaped
    /// providers reject outright.
    #[test]
    fn a_message_that_called_two_tools_stays_one_message() {
        let stored = vec![ConversationMessage {
            role: MessageRole::Assistant,
            blocks: vec![
                ContentBlock::Text {
                    text: "looking".into(),
                },
                ContentBlock::ToolUse {
                    id: "call-a".into(),
                    name: "read".into(),
                    input: r#"{"path":"a"}"#.into(),
                },
                ContentBlock::ToolUse {
                    id: "call-b".into(),
                    name: "grep".into(),
                    input: r#"{"pattern":"x"}"#.into(),
                },
            ],
            usage: None,
            timestamp: None,
        }];

        for policy in [ImagePolicy::Native, ImagePolicy::Degrade] {
            let msgs = replay_messages(&stored, policy);
            let assistant: Vec<_> = msgs.iter().filter(|m| m.role == "assistant").collect();
            assert_eq!(
                assistant.len(),
                1,
                "{policy:?} exploded one turn into several assistant messages"
            );
            let calls = assistant[0]
                .tool_calls
                .as_ref()
                .expect("both calls belong to the one turn that made them");
            assert_eq!(calls.len(), 2, "{calls:?}");
            assert!(assistant[0].content.as_text().contains("looking"));
            let names: Vec<&str> = calls.iter().map(|c| c.function.name.as_str()).collect();
            assert!(
                names.contains(&"read") && names.contains(&"grep"),
                "{names:?}"
            );
        }
    }

    /// Both modes replay the same record. Only the glass differs.
    #[test]
    fn the_only_difference_between_policies_is_the_image() {
        let stored = vec![
            ConversationMessage::user_text("hello"),
            ConversationMessage {
                role: MessageRole::Assistant,
                blocks: vec![
                    ContentBlock::Reasoning {
                        reasoning: "weighing".into(),
                    },
                    ContentBlock::Text {
                        text: "hi there".into(),
                    },
                ],
                usage: None,
                timestamp: None,
            },
        ];

        let native = replay_messages(&stored, ImagePolicy::Native);
        let degraded = replay_messages(&stored, ImagePolicy::Degrade);
        assert_eq!(native.len(), degraded.len());
        for (a, b) in native.iter().zip(degraded.iter()) {
            assert_eq!(a.role, b.role);
            assert_eq!(a.content.as_text(), b.content.as_text());
        }
    }

    /// A model that cannot see is told so, rather than handed a stub shaped
    /// like success — and the tool work beside the image is not lost with it.
    #[test]
    fn a_degraded_image_says_it_was_not_seen() {
        let stored = vec![ConversationMessage {
            role: MessageRole::User,
            blocks: vec![
                ContentBlock::Text {
                    text: "what is this".into(),
                },
                ContentBlock::Image {
                    media_type: "image/png".into(),
                    data: "AAAA".into(),
                },
            ],
            usage: None,
            timestamp: None,
        }];

        let body = replay_messages(&stored, ImagePolicy::Degrade)[0]
            .content
            .as_text();
        assert!(body.contains("what is this"), "{body}");
        assert!(body.contains("image/png"), "{body}");
        assert!(body.contains("not visible"), "{body}");
        assert!(
            !body.contains("AAAA"),
            "base64 must not ride in prose: {body}"
        );
    }

    /// An assistant message carrying an image must not replay as the human's.
    #[test]
    fn a_native_image_keeps_the_role_that_produced_it() {
        let stored = vec![ConversationMessage {
            role: MessageRole::Assistant,
            blocks: vec![
                ContentBlock::Text {
                    text: "here is what I found".into(),
                },
                ContentBlock::Image {
                    media_type: "image/png".into(),
                    data: "AAAA".into(),
                },
            ],
            usage: None,
            timestamp: None,
        }];

        let msgs = replay_messages(&stored, ImagePolicy::Native);
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].role, "assistant",
            "an image replayed under the wrong role reads as though the \
             human said it"
        );
        // The prose beside the image survives alongside it.
        assert!(msgs[0].content.as_text().contains("here is what I found"));
    }
}
