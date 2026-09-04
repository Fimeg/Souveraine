//! Message conversion — `ChatMessage` (state-machine model) → `MsgKind`
//! (display model the `MessageList` widget renders).
//!
//! Pure functions, no `ChatScreen` coupling. The system-context heuristic
//! collapses a large multi-heading agent-context dump into a one-line summary
//! so it doesn't drown the transcript.

use crate::ui::chat::ChatMessage;
use crate::ui::widgets::message_list::MsgKind;

fn is_system_context_dump(text: &str) -> bool {
    let heading_count = text
        .lines()
        .filter(|l| l.starts_with("## ") || l.starts_with("# "))
        .count();
    heading_count >= 3 && text.len() > 500
}

fn system_context_summary(text: &str) -> String {
    let headings: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("## ") || l.starts_with("# "))
        .take(4)
        .map(|l| l.trim_start_matches('#').trim())
        .collect();
    let line_count = text.lines().count();
    if headings.is_empty() {
        format!("Agent context ({line_count} lines)")
    } else {
        format!(
            "Agent context: {} ({line_count} lines)",
            headings.join(", ")
        )
    }
}

pub(super) fn chat_message_to_msgkind(msg: &ChatMessage) -> MsgKind {
    match msg {
        ChatMessage::User { text, .. } => MsgKind::User {
            name: "you".into(),
            text: text.clone(),
        },
        ChatMessage::Assistant {
            text, streaming, ..
        } => {
            if !*streaming && is_system_context_dump(text) {
                return MsgKind::SystemContext {
                    summary: system_context_summary(text),
                    _full_text: text.clone(),
                };
            }
            MsgKind::Assistant {
                name: "agent".into(),
                text: text.clone(),
                streaming: *streaming,
            }
        }
        ChatMessage::Surfacing {
            source,
            content,
            priority,
            ..
        } => MsgKind::Surfacing {
            source: source.clone(),
            content: content.clone(),
            priority: priority.clone(),
        },
        ChatMessage::System { text, .. } => MsgKind::System { text: text.clone() },
        ChatMessage::Interjection {
            text, delivered, ..
        } => MsgKind::Interjection {
            text: text.clone(),
            delivered: *delivered,
        },
        ChatMessage::Interstitial { text, register } => MsgKind::Interstitial {
            text: text.clone(),
            is_voice: matches!(register, crate::backend::Register::HerVoice),
        },
        ChatMessage::Tool {
            name,
            arguments,
            round,
            result,
            ..
        } => {
            let args = crate::ui::chat::tool_renderers::summarize_tool_args(name, arguments);
            MsgKind::Tool {
                name: name.clone(),
                args_summary: args,
                round: *round,
                is_error: result.as_ref().map(|r| r.is_error).unwrap_or(false),
                is_pending: result.is_none(),
                result_output: result.as_ref().map(|r| r.output.clone()),
            }
        }
        ChatMessage::Image { label, .. } => MsgKind::System {
            text: format!("[image: {label}]"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::chat::{ChatMessage, ToolResultBlock};
    use crate::ui::widgets::message_list::MsgKind;
    use std::cell::RefCell;
    use std::time::Instant;

    #[test]
    fn user_message_converts_to_user_msgkind() {
        let msg = ChatMessage::User {
            text: "hello".into(),
            ts: Instant::now(),
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::User { ref name, ref text }
                if name == "you" && text == "hello"),
            "expected User name=you text=hello, got {result:?}"
        );
    }

    #[test]
    fn assistant_message_converts_streaming() {
        let msg = ChatMessage::Assistant {
            text: "hi".into(),
            streaming: true,
            ts: Instant::now(),
            rendered_cache: RefCell::new(None),
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::Assistant { ref name, ref text, streaming }
                if name == "agent" && text == "hi" && streaming),
            "expected Assistant name=agent text=hi streaming=true, got {result:?}"
        );
    }

    #[test]
    fn assistant_message_converts_not_streaming() {
        let msg = ChatMessage::Assistant {
            text: "hi".into(),
            streaming: false,
            ts: Instant::now(),
            rendered_cache: RefCell::new(None),
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::Assistant { streaming, .. } if !streaming),
            "expected Assistant streaming=false, got {result:?}"
        );
    }

    #[test]
    fn system_message_converts() {
        let msg = ChatMessage::System {
            text: "status".into(),
            ts: Instant::now(),
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::System { ref text } if text == "status"),
            "expected System text=status, got {result:?}"
        );
    }

    #[test]
    fn surfacing_message_converts() {
        let msg = ChatMessage::Surfacing {
            source: "sub".into(),
            content: "msg".into(),
            priority: "high".into(),
            ts: Instant::now(),
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::Surfacing { ref source, ref content, ref priority }
                if source == "sub" && content == "msg" && priority == "high"),
            "expected Surfacing with matching fields, got {result:?}"
        );
    }

    #[test]
    fn interjection_message_converts() {
        let msg = ChatMessage::Interjection {
            text: "hey".into(),
            delivered: false,
            ts: Instant::now(),
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::Interjection { ref text, delivered }
                if text == "hey" && !delivered),
            "expected Interjection delivered=false, got {result:?}"
        );
    }

    #[test]
    fn interjection_message_converts_delivered() {
        let msg = ChatMessage::Interjection {
            text: "hey".into(),
            delivered: true,
            ts: Instant::now(),
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::Interjection { delivered, .. } if delivered),
            "expected Interjection delivered=true, got {result:?}"
        );
    }

    #[test]
    fn tool_message_pending() {
        let msg = ChatMessage::Tool {
            id: "t1".into(),
            name: "read".into(),
            arguments: "{}".into(),
            round: 1,
            result: None,
            ts: Instant::now(),
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::Tool { ref name, ref args_summary, round, is_error, is_pending, result_output: None }
                if name == "read" && round == 1 && !is_error && is_pending),
            "expected Tool pending with name=read, got {result:?}"
        );
    }

    #[test]
    fn tool_message_with_result() {
        let msg = ChatMessage::Tool {
            id: "t2".into(),
            name: "write".into(),
            arguments: r#"{"path":"/tmp/x"}"#.into(),
            round: 2,
            result: Some(ToolResultBlock {
                output: "ok".into(),
                is_error: false,
            }),
            ts: Instant::now(),
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::Tool { ref name, is_pending: false, is_error: false, .. }
                if name == "write"),
            "expected Tool with result (not pending, not error), got {result:?}"
        );
    }

    #[test]
    fn tool_message_with_error() {
        let msg = ChatMessage::Tool {
            id: "t3".into(),
            name: "exec".into(),
            arguments: r#"{"cmd":"ls"}"#.into(),
            round: 3,
            result: Some(ToolResultBlock {
                output: "permission denied".into(),
                is_error: true,
            }),
            ts: Instant::now(),
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::Tool { ref name, is_pending: false, is_error: true, .. }
                if name == "exec"),
            "expected Tool with error (not pending, is_error), got {result:?}"
        );
    }

    #[test]
    fn tool_message_threads_result_output() {
        let msg = ChatMessage::Tool {
            id: "t_output".into(),
            name: "exec".into(),
            arguments: "{}".into(),
            round: 1,
            result: Some(ToolResultBlock {
                output: "hello world".into(),
                is_error: false,
            }),
            ts: Instant::now(),
        };
        let result = chat_message_to_msgkind(&msg);
        match &result {
            MsgKind::Tool { result_output, .. } => {
                assert_eq!(result_output.as_deref(), Some("hello world"));
            }
            other => panic!("expected Tool, got {other:?}"),
        }
    }

    #[test]
    fn tool_message_truncates_long_arguments() {
        let long_args = "a".repeat(80);
        let msg = ChatMessage::Tool {
            id: "t4".into(),
            name: "long".into(),
            arguments: long_args.clone(),
            round: 1,
            result: None,
            ts: Instant::now(),
        };
        let result = chat_message_to_msgkind(&msg);
        let (got_len, got_text) = match &result {
            MsgKind::Tool { args_summary, .. } => (args_summary.len(), args_summary.clone()),
            other => (0, format!("{other:?}")),
        };
        assert!(
            matches!(result, MsgKind::Tool { ref args_summary, .. }
                if got_len > 0 && got_len <= 120),
            "expected summarized args (<=120 chars), got length={got_len} args={got_text:?}"
        );
    }

    #[test]
    fn image_message_converts_to_system() {
        let msg = ChatMessage::Image {
            media_type: "image/png".into(),
            label: "photo.png".into(),
            data: "base64blob".into(),
            dimensions: None,
            ts: Instant::now(),
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::System { ref text }
                if text.contains("image") && text.contains("photo.png")),
            "expected System text containing 'image' and 'photo.png', got {result:?}"
        );
    }

    #[test]
    fn interstitial_message_converts_voice() {
        let msg = ChatMessage::Interstitial {
            text: "waiting".into(),
            register: crate::backend::Register::HerVoice,
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::Interstitial { ref text, is_voice }
                if text == "waiting" && is_voice),
            "expected Interstitial is_voice=true, got {result:?}"
        );
    }

    #[test]
    fn interstitial_message_converts_cenno() {
        let msg = ChatMessage::Interstitial {
            text: "checking".into(),
            register: crate::backend::Register::Cenno,
        };
        let result = chat_message_to_msgkind(&msg);
        assert!(
            matches!(result, MsgKind::Interstitial { ref text, is_voice }
                if text == "checking" && !is_voice),
            "expected Interstitial is_voice=false, got {result:?}"
        );
    }
}
