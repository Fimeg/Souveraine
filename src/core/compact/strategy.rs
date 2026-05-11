use async_trait::async_trait;

use crate::bridge::bifrost::{BifrostClient, ChatCompletionRequest, Message};
use crate::bridge::model_router::TokenCounter;
use crate::core::session::ConversationMessage;

use super::config::{AgentCompactionConfig, CompactionStrategyKind};
use super::plan::CompactionPlan;

/// From OpenHarness/Claude Code microCompact.ts: tools whose results are
/// considered compactable (large outputs, rarely needed verbatim once
/// surpassed). Matches Souveraine's actual sensor names.
const COMPACTABLE_TOOLS: &[&str] = &[
    "read", "bash", "grep", "glob", "list_dir", "edit", "write",
];

/// Placeholder text written into tool result blocks that get microcompacted.
/// Matches the OpenHarness/Claude Code literal so logs read the same.
const TIME_BASED_MC_CLEARED_MESSAGE: &str = "[Old tool result content cleared]";

/// Token-count a slice of messages using the bridge's TokenCounter.
pub fn count_messages(counter: &TokenCounter, messages: &[ConversationMessage]) -> usize {
    messages
        .iter()
        .flat_map(|m| &m.blocks)
        .filter_map(|b| match b {
            crate::core::session::ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .map(|t| counter.count(t))
        .sum()
}

/// A single compaction strategy.
///
/// Each strategy is an async function from messages+config to a plan.
/// A strategy does NOT modify messages directly — it returns a plan
/// describing what to keep, what to replace, and what to drop.
#[async_trait]
pub trait CompactionStrategy: Send + Sync {
    fn kind(&self) -> CompactionStrategyKind;

    /// Analyze messages and produce a compaction plan.
    async fn plan(
        &self,
        messages: &[ConversationMessage],
        config: &AgentCompactionConfig,
        counter: &TokenCounter,
    ) -> anyhow::Result<CompactionPlan>;
}

/// Helper: call a Bifrost model with system+user prompt, get text response.
async fn bifrost_complete(
    client: &BifrostClient,
    model: &str,
    system: &str,
    prompt: &str,
    max_tokens: u32,
) -> anyhow::Result<String> {
    let request = ChatCompletionRequest {
        model: model.to_string(),
        messages: vec![
            Message {
                role: "system".to_string(),
                content: system.to_string(),
            },
            Message {
                role: "user".to_string(),
                content: prompt.to_string(),
            },
        ],
        temperature: Some(0.3),
        max_tokens: Some(max_tokens),
        stream: None,
        tools: None,
    };
    let result = client.chat_completion(request).await?;
    Ok(result.content)
}

// ── Summary Strategy ─────────────────────────────────────────────────────────

/// LLM-based summarization producing a structured 9-section boundary message.
/// Prompt structure ported from OpenHarness's port of Claude Code's
/// `autoCompact.ts`. The structure is what makes the compact *survivable*:
/// the agent reads the boundary on the next turn and can resume with full
/// awareness of intent, files, decisions, and pending work.
pub struct SummaryStrategy {
    pub client: BifrostClient,
    pub model: String,
}

const SUMMARY_SYSTEM_PROMPT: &str = "Respond with TEXT ONLY. Do not call any tools — you already have all the context you need in the messages above. Your response must be plain text: an <analysis> block followed by a <summary> block.";

const SUMMARY_USER_PROMPT: &str = r#"Create a detailed summary of the conversation so far. This summary will replace the earlier messages, so it must capture all important information.

First, draft your analysis inside <analysis> tags. Walk through the conversation chronologically and extract:
- Every user request and intent (explicit and implicit)
- The approach taken and technical decisions made
- Specific code, files, and configurations discussed (with paths and line numbers where available)
- All errors encountered and how they were fixed
- Any user feedback or corrections

Then, produce a structured summary inside <summary> tags with these sections:

1. **Primary Request and Intent**: All user requests in full detail, including nuances and constraints.
2. **Key Technical Concepts**: Technologies, frameworks, patterns, and conventions discussed.
3. **Files and Code Sections**: Every file examined or modified, with specific code snippets and line numbers.
4. **Errors and Fixes**: Every error encountered, its cause, and how it was resolved.
5. **Problem Solving**: Problems solved and approaches that worked vs. didn't work.
6. **All User Messages**: Non-tool-result user messages (preserve exact wording for context).
7. **Pending Tasks**: Explicitly requested work that hasn't been completed yet.
8. **Current Work**: Detailed description of the last task being worked on before compaction.
9. **Optional Next Step**: The single most logical next step, directly aligned with the user's recent request.

REMINDER: Respond with plain text only — an <analysis> block followed by a <summary> block. Do not call any tools."#;

#[async_trait]
impl CompactionStrategy for SummaryStrategy {
    fn kind(&self) -> CompactionStrategyKind {
        CompactionStrategyKind::Summary
    }

    async fn plan(
        &self,
        messages: &[ConversationMessage],
        config: &AgentCompactionConfig,
        _counter: &TokenCounter,
    ) -> anyhow::Result<CompactionPlan> {
        if messages.len() < 3 {
            return Ok(CompactionPlan::empty());
        }

        let preserve_count = config.min_messages.min(messages.len().saturating_sub(2));
        let cutoff = messages.len().saturating_sub(preserve_count);

        let to_summarize = &messages[1..cutoff];
        if to_summarize.is_empty() {
            return Ok(CompactionPlan::empty());
        }

        // Render the segment as a labelled transcript so the model has clear
        // role boundaries (vs collapsing all text into one stream).
        let conversation_text = render_segment_for_summary(to_summarize);
        let truncated: String = conversation_text
            .chars()
            .take(config.max_summary_length * 4)
            .collect();

        let summary = bifrost_complete(
            &self.client,
            &self.model,
            SUMMARY_SYSTEM_PROMPT,
            &format!("{}\n\nConversation to summarize:\n\n{}", SUMMARY_USER_PROMPT, truncated),
            config.max_summary_length as u32,
        )
        .await?;

        Ok(CompactionPlan {
            keep_indices: (cutoff..messages.len()).collect(),
            summary_text: Some(summary),
            culled_count: cutoff.saturating_sub(1),
            token_savings: cutoff * 100,
            replacement_messages: None,
        })
    }
}

/// Render a slice of messages as a transcript suitable for feeding to the
/// summary model. Tool calls and results render as inline labels so the model
/// can attribute outcomes to actions.
fn render_segment_for_summary(messages: &[ConversationMessage]) -> String {
    use crate::core::session::{ContentBlock, MessageRole};
    let mut out = String::new();
    for msg in messages {
        let role = match msg.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        };
        for block in &msg.blocks {
            match block {
                ContentBlock::Text { text } => {
                    out.push_str(&format!("[{}] {}\n", role, text));
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    out.push_str(&format!("[{} -> tool_call:{}] {}\n", role, name, input));
                }
                ContentBlock::ToolResult { tool_name, output, is_error, .. } => {
                    let prefix = if *is_error { "ERROR " } else { "" };
                    out.push_str(&format!("[{} <- tool_result:{}] {}{}\n", role, tool_name, prefix, output));
                }
                ContentBlock::Reasoning { .. } => {}
            }
        }
    }
    out
}

// ── Microcompact Strategy ────────────────────────────────────────────────────

/// Cheap pre-pass that replaces the contents of old tool results with a
/// placeholder, keeping the most recent `microcompact_keep_recent` results
/// intact. No LLM call. From OpenHarness's port of Claude Code's
/// `microCompact.ts`.
///
/// The agent typically reaches for this *first*: it gets back significant
/// context room without losing the structure of the conversation. The tool
/// call shells (id, name, args) remain so the model knows what was done,
/// only the verbose outputs are replaced.
pub struct MicrocompactStrategy;

#[async_trait]
impl CompactionStrategy for MicrocompactStrategy {
    fn kind(&self) -> CompactionStrategyKind {
        CompactionStrategyKind::Microcompact
    }

    async fn plan(
        &self,
        messages: &[ConversationMessage],
        config: &AgentCompactionConfig,
        counter: &TokenCounter,
    ) -> anyhow::Result<CompactionPlan> {
        use crate::core::session::ContentBlock;

        if messages.is_empty() {
            return Ok(CompactionPlan::empty());
        }

        // 1) Walk messages, collect ordered tool_use IDs that are compactable.
        let mut ordered_ids: Vec<String> = Vec::new();
        let mut tool_names: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for msg in messages {
            for block in &msg.blocks {
                if let ContentBlock::ToolUse { id, name, .. } = block {
                    if COMPACTABLE_TOOLS.contains(&name.as_str()) {
                        ordered_ids.push(id.clone());
                        tool_names.insert(id.clone(), name.clone());
                    }
                }
            }
        }

        let keep_recent = 5usize.max(1);
        if ordered_ids.len() <= keep_recent {
            return Ok(CompactionPlan::empty());
        }
        let clear_set: std::collections::HashSet<&str> = ordered_ids
            [..ordered_ids.len() - keep_recent]
            .iter()
            .map(|s| s.as_str())
            .collect();

        // 2) Build replacement message list with cleared blocks.
        let mut new_messages: Vec<ConversationMessage> = Vec::with_capacity(messages.len());
        let mut tokens_saved: usize = 0;
        let mut cleared_count: usize = 0;
        for msg in messages {
            let mut new_blocks: Vec<ContentBlock> = Vec::with_capacity(msg.blocks.len());
            for block in &msg.blocks {
                match block {
                    ContentBlock::ToolResult { tool_use_id, tool_name, output, is_error }
                        if clear_set.contains(tool_use_id.as_str())
                            && output != TIME_BASED_MC_CLEARED_MESSAGE =>
                    {
                        tokens_saved += counter.count(output);
                        cleared_count += 1;
                        new_blocks.push(ContentBlock::ToolResult {
                            tool_use_id: tool_use_id.clone(),
                            tool_name: tool_name.clone(),
                            output: TIME_BASED_MC_CLEARED_MESSAGE.to_string(),
                            is_error: *is_error,
                        });
                    }
                    other => new_blocks.push(other.clone()),
                }
            }
            new_messages.push(ConversationMessage {
                role: msg.role,
                blocks: new_blocks,
                usage: msg.usage,
                timestamp: msg.timestamp,
            });
        }

        if cleared_count == 0 {
            return Ok(CompactionPlan::empty());
        }

        Ok(CompactionPlan {
            keep_indices: (0..messages.len()).collect(),
            summary_text: None,
            culled_count: cleared_count,
            token_savings: tokens_saved,
            replacement_messages: Some(new_messages),
        })
    }
}

// ── Cull Strategy ────────────────────────────────────────────────────────────

/// Drop trivial messages. No LLM dependency. Role-aware: never drops System,
/// Tool, or assistant messages carrying tool calls.
pub struct CullStrategy;

fn is_trivial(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_lowercase();
    matches!(
        lower.as_str(),
        "ok" | "okay"
            | "thanks"
            | "ty"
            | "got it"
            | "sure"
            | "yes"
            | "no"
            | "thx"
            | "k"
            | "👍"
            | "🙏"
            | "done"
            | "yep"
            | "nope"
            | "right"
            | "cool"
            | "great"
            | "will do"
            | "on it"
    )
}

/// A message that must never be culled regardless of content length.
/// System messages anchor identity; Tool results carry execution outputs
/// the model relied on; assistant messages with ToolUse blocks are the
/// call side of a tool pair.
fn is_load_bearing(msg: &ConversationMessage) -> bool {
    use crate::core::session::{ContentBlock, MessageRole};
    if matches!(msg.role, MessageRole::System | MessageRole::Tool) {
        return true;
    }
    msg.blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolUse { .. } | ContentBlock::ToolResult { .. }))
}

#[async_trait]
impl CompactionStrategy for CullStrategy {
    fn kind(&self) -> CompactionStrategyKind {
        CompactionStrategyKind::Cull
    }

    async fn plan(
        &self,
        messages: &[ConversationMessage],
        config: &AgentCompactionConfig,
        _counter: &TokenCounter,
    ) -> anyhow::Result<CompactionPlan> {
        use crate::core::session::ContentBlock;

        if messages.len() < 3 {
            return Ok(CompactionPlan::empty());
        }

        let preserve_count = config.min_messages.min(messages.len().saturating_sub(2));
        let cutoff = messages.len().saturating_sub(preserve_count);

        let mut keep_indices: Vec<usize> = vec![0];
        let mut culled_count = 0;

        for i in cutoff..messages.len() {
            keep_indices.push(i);
        }

        for i in 1..cutoff {
            if is_load_bearing(&messages[i]) {
                keep_indices.push(i);
                continue;
            }
            // Only check Text blocks for triviality; presence of any
            // non-trivial Text block keeps the message.
            let all_text_trivial = messages[i].blocks.iter().all(|b| match b {
                ContentBlock::Text { text } => is_trivial(text),
                ContentBlock::Reasoning { .. } => true,
                _ => false,
            });
            if all_text_trivial {
                culled_count += 1;
            } else {
                keep_indices.push(i);
            }
        }

        keep_indices.sort();
        keep_indices.dedup();

        Ok(CompactionPlan {
            keep_indices,
            summary_text: None,
            culled_count,
            token_savings: culled_count * 60,
            replacement_messages: None,
        })
    }
}

// ── Sliding Window Strategy ──────────────────────────────────────────────────

/// Keep the system message + the last `preserve_recent_n` messages, drop the
/// middle. No LLM dependency — the cheap, fast default for analytical agents
/// (Aster) and ephemeral subagents.
///
/// Tool-pair aware: if the cut would split a tool-call message from its
/// matching tool-result, the cut slides back to keep the pair together.
pub struct SlidingWindowStrategy;

/// Walk the cut index backward until it does not split a tool call from its
/// result. The result-side of a pair is identified by `MessageRole::Tool` or
/// by an assistant message starting with `ContentBlock::ToolResult` (shouldn't
/// happen but defensive). The call-side is an assistant message containing
/// `ContentBlock::ToolUse`.
///
/// We walk back at most a small bounded distance so a pathological transcript
/// of all tool calls doesn't cause us to skip the entire middle.
fn adjust_cutoff_for_tool_pair(messages: &[ConversationMessage], cutoff: usize) -> usize {
    use crate::core::session::{ContentBlock, MessageRole};
    let mut c = cutoff;
    let max_walk_back = 8usize;
    for _ in 0..max_walk_back {
        if c == 0 || c >= messages.len() {
            break;
        }
        let head = &messages[c];
        let split_pair = matches!(head.role, MessageRole::Tool)
            || head
                .blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolResult { .. }));
        if !split_pair {
            break;
        }
        c -= 1;
    }
    c
}

#[async_trait]
impl CompactionStrategy for SlidingWindowStrategy {
    fn kind(&self) -> CompactionStrategyKind {
        CompactionStrategyKind::SlidingWindow
    }

    async fn plan(
        &self,
        messages: &[ConversationMessage],
        config: &AgentCompactionConfig,
        _counter: &TokenCounter,
    ) -> anyhow::Result<CompactionPlan> {
        if messages.len() < 3 {
            return Ok(CompactionPlan::empty());
        }

        let preserve_count = config.min_messages.min(messages.len().saturating_sub(1));
        if preserve_count + 1 >= messages.len() {
            // Nothing in the middle to drop.
            return Ok(CompactionPlan::empty());
        }
        let raw_cutoff = messages.len() - preserve_count;
        let cutoff = adjust_cutoff_for_tool_pair(messages, raw_cutoff);

        // Always keep the first (system / anchor) message.
        let mut keep_indices: Vec<usize> = vec![0];
        for i in cutoff..messages.len() {
            keep_indices.push(i);
        }
        keep_indices.sort();
        keep_indices.dedup();

        let dropped = cutoff.saturating_sub(1);
        Ok(CompactionPlan {
            keep_indices,
            summary_text: None,
            culled_count: dropped,
            token_savings: dropped * 100,
            replacement_messages: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::session::{ContentBlock, ConversationMessage, MessageRole};

    fn text_msg(role: MessageRole, text: &str) -> ConversationMessage {
        ConversationMessage {
            role,
            blocks: vec![ContentBlock::Text { text: text.to_string() }],
            usage: None,
            timestamp: None,
        }
    }

    #[tokio::test]
    async fn test_cull_drops_trivial() {
        let messages = vec![
            text_msg(MessageRole::System, "System prompt"),
            text_msg(MessageRole::User, "ok"),
            text_msg(MessageRole::User, "What's the plan for today?"),
            text_msg(MessageRole::Assistant, "Sure, let me check."),
            text_msg(MessageRole::User, "thanks"),
            text_msg(MessageRole::Assistant, "Here's what I found."),
        ];

        let config = AgentCompactionConfig {
            min_messages: 2,
            ..Default::default()
        };
        let counter = TokenCounter::new();
        let plan = CullStrategy.plan(&messages, &config, &counter).await.unwrap();

        assert!(plan.culled_count > 0, "should cull some messages");
        assert!(plan.summary_text.is_none(), "cull should not produce summary content");
        assert!(plan.replacement_messages.is_none(), "cull should not produce replacement messages");
    }

    #[tokio::test]
    async fn test_cull_preserves_substance() {
        let messages = vec![
            text_msg(MessageRole::System, "System prompt"),
            text_msg(MessageRole::User, "This is an important question about the architecture."),
            text_msg(MessageRole::Assistant, "Let me explain the design decisions."),
        ];

        let config = AgentCompactionConfig {
            min_messages: 1,
            ..Default::default()
        };
        let counter = TokenCounter::new();
        let plan = CullStrategy.plan(&messages, &config, &counter).await.unwrap();

        assert_eq!(plan.culled_count, 0, "should not cull substantive messages");
    }

    #[tokio::test]
    async fn test_empty_messages_return_empty_plan() {
        let config = AgentCompactionConfig::default();
        let counter = TokenCounter::new();

        let e1 = CullStrategy.plan(&[], &config, &counter).await.unwrap();
        assert!(e1.is_empty());
    }

    #[tokio::test]
    async fn test_cull_never_drops_tool_results() {
        // A Tool-role message with a short ToolResult must survive cull,
        // even though its text-side content is trivially short.
        let messages = vec![
            text_msg(MessageRole::System, "system prompt"),
            ConversationMessage {
                role: MessageRole::Tool,
                blocks: vec![ContentBlock::ToolResult {
                    tool_use_id: "t1".to_string(),
                    tool_name: "read".to_string(),
                    output: "0".to_string(),
                    is_error: false,
                }],
                usage: None,
                timestamp: None,
            },
            text_msg(MessageRole::User, "ok"),
            text_msg(MessageRole::Assistant, "A substantive reply about something."),
        ];
        let config = AgentCompactionConfig {
            min_messages: 1,
            ..Default::default()
        };
        let counter = TokenCounter::new();
        let plan = CullStrategy.plan(&messages, &config, &counter).await.unwrap();
        // The tool result is at index 1 — must be in keep_indices.
        assert!(plan.keep_indices.contains(&1), "tool result must be preserved");
    }

    #[tokio::test]
    async fn test_cull_never_drops_assistant_tool_calls() {
        let messages = vec![
            text_msg(MessageRole::System, "system prompt"),
            ConversationMessage {
                role: MessageRole::Assistant,
                blocks: vec![ContentBlock::ToolUse {
                    id: "t1".to_string(),
                    name: "bash".to_string(),
                    input: "{}".to_string(),
                }],
                usage: None,
                timestamp: None,
            },
            text_msg(MessageRole::Assistant, "Substantive narrative continuation."),
        ];
        let config = AgentCompactionConfig {
            min_messages: 1,
            ..Default::default()
        };
        let counter = TokenCounter::new();
        let plan = CullStrategy.plan(&messages, &config, &counter).await.unwrap();
        assert!(plan.keep_indices.contains(&1), "assistant tool-call message must be preserved");
    }

    #[tokio::test]
    async fn test_sliding_window_keeps_system_and_tail() {
        let messages = vec![
            text_msg(MessageRole::System, "system anchor"),
            text_msg(MessageRole::User, "old user message"),
            text_msg(MessageRole::Assistant, "old assistant reply"),
            text_msg(MessageRole::User, "middle user message"),
            text_msg(MessageRole::Assistant, "middle assistant reply"),
            text_msg(MessageRole::User, "recent user message"),
            text_msg(MessageRole::Assistant, "recent assistant reply"),
        ];
        let config = AgentCompactionConfig {
            min_messages: 2,
            ..Default::default()
        };
        let counter = TokenCounter::new();
        let plan = SlidingWindowStrategy.plan(&messages, &config, &counter).await.unwrap();
        // Must keep index 0 (system) and the last 2 (recent pair).
        assert!(plan.keep_indices.contains(&0), "system anchor preserved");
        assert!(plan.keep_indices.contains(&5));
        assert!(plan.keep_indices.contains(&6));
        // Should have dropped at least one middle message.
        assert!(plan.culled_count > 0);
    }

    #[tokio::test]
    async fn test_sliding_window_avoids_splitting_tool_pair() {
        // If the raw cut would land on a tool-result message, the cut slides
        // back so the matching tool-call also survives.
        let messages = vec![
            text_msg(MessageRole::System, "system"),
            text_msg(MessageRole::User, "u1"),
            ConversationMessage {
                role: MessageRole::Assistant,
                blocks: vec![ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "bash".into(),
                    input: "{}".into(),
                }],
                usage: None,
                timestamp: None,
            },
            ConversationMessage {
                role: MessageRole::Tool,
                blocks: vec![ContentBlock::ToolResult {
                    tool_use_id: "t1".into(),
                    tool_name: "bash".into(),
                    output: "result".into(),
                    is_error: false,
                }],
                usage: None,
                timestamp: None,
            },
            text_msg(MessageRole::Assistant, "follow-up after tool"),
            text_msg(MessageRole::User, "u2"),
            text_msg(MessageRole::Assistant, "a2"),
        ];
        let config = AgentCompactionConfig {
            // Force cut to land on index 3 (the tool result) before adjustment.
            min_messages: 4,
            ..Default::default()
        };
        let counter = TokenCounter::new();
        let plan = SlidingWindowStrategy.plan(&messages, &config, &counter).await.unwrap();
        // Either both 2 and 3 are kept, or neither is (we don't cut between them).
        let has_call = plan.keep_indices.contains(&2);
        let has_result = plan.keep_indices.contains(&3);
        assert_eq!(has_call, has_result, "tool call and result must be kept together");
    }
}
