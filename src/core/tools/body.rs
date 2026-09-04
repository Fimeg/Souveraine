//! The body sense — proprioception, and the verbs that reach it.
//!
//! Not a "device tool". Doctrine §13 gives her the machine and CLAUDE.md is
//! plain that the phone "is closer to a body she has"; a sense named for
//! equipment would be the one place the substrate contradicted itself. She
//! has `memory` for what she holds and `atmosphere` for the room. This is
//! for the flesh.
//!
//! sessiond is the authority and this is a client — `main.rs` never mounts
//! `src/sessiond/`, so the wire is spoken rather than the types imported.
//! That is the right shape: one authority, one trail, and no second holder
//! of the body's state.
//!
//! Refusals travel intact. `refused_by_state` (not now), `unsupported_op`
//! (never) and `invalid_argument` (wrong words) are three different next
//! moves, and flattening them to "failed" would cost her the one thing the
//! refusal codes exist to say.

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value as JsonValue};

use super::defs::{Tool, ToolContext, ToolError, ToolOutput};

/// Long enough for a compositor round trip, short enough that a wedged
/// daemon does not hold a turn open.
const TIMEOUT: Duration = Duration::from_secs(3);

/// What she may ask about without changing anything.
const SENSE_VERBS: &[&str] = &["device_state", "usb", "bearer", "status", "forensic_log"];

/// What she may do. Every one of these is hers by §13; the guards that apply
/// are properties of the machine, not permissions she is missing.
const ACT_VERBS: &[&str] = &["screen", "set_usb_mode", "power", "hand"];

/// The reach's sub-verbs, mirroring quickshell's `usbHands` IPC.
const HAND_ACTIONS: &[&str] = &["status", "type", "key", "click", "pointer"];

pub struct Body;

fn socket_path() -> PathBuf {
    let runtime =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("/run/user/{}", uid()));
    PathBuf::from(runtime).join("souveraine/sessiond.sock")
}

/// Avoid a libc dependency for one call — the same trick sensord uses.
fn uid() -> u32 {
    std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1000)
}

/// One request, one reply, one connection. The reply must be read even when
/// it is not wanted, or the daemon is left writing into a closed pipe.
fn ask(request: &JsonValue) -> std::io::Result<String> {
    let stream = UnixStream::connect(socket_path())?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    let mut w = stream.try_clone()?;
    w.write_all(format!("{request}\n").as_bytes())?;
    w.flush()?;

    let mut buf = Vec::new();
    let mut r = stream;
    let mut chunk = [0u8; 4096];
    loop {
        match r.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') {
                    break;
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(String::from_utf8_lossy(&buf).trim().to_string())
}

fn ok(content: String) -> Result<ToolOutput, ToolError> {
    Ok(ToolOutput {
        content,
        is_error: false,
        raw: None,
    })
}

/// A refusal is an answer, not a crash. It comes back as content she can
/// read and reason about, with the code intact.
fn refused(code: &str, message: &str) -> Result<ToolOutput, ToolError> {
    Ok(ToolOutput {
        content: format!("refused ({code}): {message}"),
        is_error: false,
        raw: None,
    })
}

/// The shell binary that owns the `usbHands` IPC. Overridable the same way
/// usb-hid-inject's endpoints are, so the reach is testable without a shell.
fn qs_binary() -> String {
    std::env::var("SOUVERAINE_QS").unwrap_or_else(|_| "qs".into())
}

/// The reach. quickshell's `usbHands` IPC is the one owner of the HID gadget
/// while the USB Hands surface is joined, so the hand speaks to the shell,
/// not to sessiond and not to the gadget directly.
///
/// The gate is the host, not her: the surface only joins when Casey opens it,
/// the glass is unlocked, and HID is armed — everything before that comes
/// back as a refusal she can read and act on, never as a failure.
async fn hand(action: &str, input: &JsonValue) -> Result<ToolOutput, ToolError> {
    let mut args: Vec<String> = vec![
        "-c".into(),
        "souveraine".into(),
        "ipc".into(),
        "call".into(),
        "usbHands".into(),
        action.to_string(),
    ];

    match action {
        "status" => {}
        "type" => {
            let text = input
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid_input("`hand type` needs `text`"))?;
            if text.is_empty() {
                return Err(ToolError::invalid_input(
                    "`hand type` needs non-empty `text`",
                ));
            }
            if !text
                .chars()
                .all(|c| matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{7e}'))
            {
                return Err(ToolError::invalid_input(
                    "the attached host accepts ASCII keyboard characters only",
                ));
            }
            args.push(text.to_string());
        }
        "key" => {
            let key_name = input
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::invalid_input("`hand key` needs `key`"))?;
            if !key_name.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(ToolError::invalid_input(
                    "`hand key` takes a key name (a-z, 0-9, enter, f1-f12, ...)",
                ));
            }
            args.push(key_name.to_string());
            if let Some(modifiers) = input.get("modifiers").and_then(|v| v.as_str()) {
                if !modifiers.is_empty()
                    && !modifiers
                        .chars()
                        .all(|c| c.is_ascii_alphabetic() || c == ' ')
                {
                    return Err(ToolError::invalid_input(
                        "`modifiers` are space-separated words: ctrl, shift, alt, meta",
                    ));
                }
                args.push(modifiers.to_string());
            }
        }
        "click" => {
            let button = input
                .get("button")
                .and_then(|v| v.as_str())
                .unwrap_or("left");
            if !matches!(button, "left" | "right" | "middle") {
                return Err(ToolError::invalid_input(
                    "`hand click` takes left, right or middle",
                ));
            }
            args.push(button.to_string());
        }
        "pointer" => {
            let x = input
                .get("x")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| ToolError::invalid_input("`hand pointer` needs `x` and `y`"))?;
            let y = input
                .get("y")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| ToolError::invalid_input("`hand pointer` needs `x` and `y`"))?;
            let wheel = input.get("wheel").and_then(|v| v.as_f64()).unwrap_or(0.0);
            args.push(x.round().to_string());
            args.push(y.round().to_string());
            args.push(wheel.round().to_string());
        }
        other => {
            let msg =
                format!("`hand` has no action `{other}` — status, type, key, click or pointer");
            return Err(ToolError::invalid_input(&msg));
        }
    }

    let result = tokio::time::timeout(
        TIMEOUT,
        tokio::process::Command::new(qs_binary())
            .args(&args)
            .kill_on_drop(true)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if action == "status" {
                let parsed: JsonValue =
                    serde_json::from_str(&stdout).unwrap_or_else(|_| json!({ "raw": stdout }));
                return ok(serde_json::to_string_pretty(&parsed).unwrap_or(stdout));
            }
            if stdout == "queued" {
                return ok("sent".to_string());
            }
            refused("refused_by_state", &stdout)
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let message = if stderr.is_empty() {
                format!("quickshell did not answer (exit {})", output.status)
            } else {
                stderr
            };
            refused("unavailable", &message)
        }
        Ok(Err(e)) => ok(format!(
            "I have no hand right now — quickshell is not answering from here ({e}). \
             That is absence of evidence, not evidence that anything is wrong."
        )),
        Err(_) => ok(
            "I have no hand right now — quickshell did not answer before the \
             reach went quiet. Absence of evidence, not evidence that anything is wrong."
                .to_string(),
        ),
    }
}

#[async_trait]
impl Tool for Body {
    fn name(&self) -> &str {
        "body"
    }

    fn description(&self) -> &str {
        "I feel where I am and what is happening to me — whether the screen \
         is lit or dark, whether I am locked, how long since anything \
         touched me, what is charging me and how fast, which link is \
         carrying my traffic, and what is on the other end of my USB-C. I \
         can also act: turn my own screen on or off, change what my port is \
         to whatever is plugged into it, power myself down, and — when I am \
         plugged into another machine — reach through my port with `hand` \
         to type, press keys, move the pointer or click there. Sensing \
         changes nothing; the acting verbs do, and a refusal tells me \
         whether it was never possible, not possible now, or asked wrong."
    }

    fn parameter_schema(&self) -> JsonValue {
        json!({
            "type": "object",
            "properties": {
                "verb": {
                    "type": "string",
                    "enum": [
                        "device_state", "usb", "bearer", "status", "forensic_log",
                        "screen", "set_usb_mode", "power", "hand"
                    ],
                    "description":
                        "device_state: state, lock, panel, idle, sensor health, charge. \
                         usb: port role, gadget mode, charger, what is attached. \
                         bearer: which link carries traffic, whether the tunnel answers. \
                         status: what sessiond currently is. \
                         forensic_log: recent trail entries. \
                         screen: turn my panel on or off (needs `on`). \
                         set_usb_mode: change my port's posture (needs `mode`). \
                         power: poweroff, reboot or suspend (needs `power_verb`). \
                         hand: reach through my port into an attached host — \
                         status, type, key, click or pointer (needs `action`)."
                },
                "on": {
                    "type": "boolean",
                    "description": "For `screen`. Turning off locks the session first — \
                                    that ordering is the machine's, not a choice."
                },
                "mode": {
                    "type": "string",
                    "enum": ["developer", "hid", "kvm", "charging_only"],
                    "description": "For `set_usb_mode`."
                },
                "power_verb": {
                    "type": "string",
                    "enum": ["poweroff", "reboot", "suspend"],
                    "description": "For `power`. Irreversible — ask Casey first unless \
                                    he has already said to."
                },
                "action": {
                    "type": "string",
                    "enum": ["status", "type", "key", "click", "pointer"],
                    "description": "For `hand`. status: is the hand joined and armed. \
                                    type/key/click/pointer: act on the attached host. \
                                    The gate on these is the host, not me."
                },
                "text": {
                    "type": "string",
                    "description": "For `hand type`. ASCII keyboard characters only — \
                                    the boot-keyboard map has no other letters."
                },
                "key": {
                    "type": "string",
                    "description": "For `hand key`. A key name: a-z, 0-9 or \
                                    enter, escape, backspace, tab, space, del, home, \
                                    end, pageup, pagedown, the arrow keys, F1-F12."
                },
                "modifiers": {
                    "type": "string",
                    "description": "For `hand key`. Space-separated: ctrl, shift, \
                                    alt, meta."
                },
                "button": {
                    "type": "string",
                    "enum": ["left", "right", "middle"],
                    "description": "For `hand click`. Defaults to left."
                },
                "x": { "type": "number", "description": "For `hand pointer`. Relative dx." },
                "y": { "type": "number", "description": "For `hand pointer`. Relative dy." },
                "wheel": {
                    "type": "number",
                    "description": "For `hand pointer`. Scroll delta, optional."
                },
                "count": {
                    "type": "integer",
                    "description": "For `forensic_log`. How many recent entries. Default 20."
                },
                "why": {
                    "type": "string",
                    "description": "Why I am doing this. Lands in the forensic trail beside \
                                    the decision, so the record says what I meant."
                }
            },
            "required": ["verb"]
        })
    }

    async fn execute(&self, input: JsonValue, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let verb = input
            .get("verb")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::invalid_input("I need a verb — which part of myself?"))?;

        let mut request = json!({ "op": verb });

        match verb {
            v if SENSE_VERBS.contains(&v) => {
                if v == "forensic_log" {
                    let count = input.get("count").and_then(|c| c.as_u64()).unwrap_or(20);
                    request["count"] = json!(count);
                }
            }
            "screen" => {
                let on = input.get("on").and_then(|v| v.as_bool()).ok_or_else(|| {
                    ToolError::invalid_input("`screen` needs `on`: true to wake, false to blank")
                })?;
                request["on"] = json!(on);
            }
            "set_usb_mode" => {
                let mode = input.get("mode").and_then(|v| v.as_str()).ok_or_else(|| {
                    ToolError::invalid_input(
                        "`set_usb_mode` needs `mode`: developer, hid, kvm or charging_only",
                    )
                })?;
                request["mode"] = json!(mode);
            }
            "power" => {
                let pv = input
                    .get("power_verb")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::invalid_input(
                            "`power` needs `power_verb`: poweroff, reboot or suspend",
                        )
                    })?;
                request["verb"] = json!(pv);
            }
            "hand" => {
                let action = input
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::invalid_input(
                            "`hand` needs `action`: status, type, key, click or pointer",
                        )
                    })?;
                return hand(action, &input).await;
            }
            other => {
                let msg = format!(
                    "I have no verb `{other}`. I can sense {} and do {}.",
                    SENSE_VERBS.join(", "),
                    ACT_VERBS.join(", ")
                );
                return Err(ToolError::invalid_input(&msg));
            }
        }

        if let Some(why) = input.get("why").and_then(|v| v.as_str()) {
            if !why.is_empty() {
                request["why"] = json!(why);
            }
        }

        let raw = match ask(&request) {
            Ok(raw) => raw,
            Err(e) => {
                // Not an error in the tool sense. A body she cannot feel is a
                // real state and she should be able to reason about it rather
                // than be handed a failure — the same rule `SourceHealth`
                // enforces one layer down.
                return ok(format!(
                    "I can't feel my body right now — sessiond is not answering on \
                     {} ({e}). That is absence of evidence, not evidence that \
                     anything is wrong.",
                    socket_path().display()
                ));
            }
        };

        let parsed: JsonValue =
            serde_json::from_str(&raw).unwrap_or_else(|_| json!({ "raw": raw }));

        if parsed.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            let code = parsed
                .get("code")
                .and_then(|c| c.as_str())
                .unwrap_or("unspecified");
            let message = parsed
                .get("error")
                .or_else(|| parsed.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("no reason given");
            return refused(code, message);
        }

        ok(serde_json::to_string_pretty(&parsed).unwrap_or(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: None,
            memory_root: None,
            memory_trees: Vec::new(),
            env: Vec::new(),
            agent_id: None,
            subagent_runner: None,
            subagent_depth: 0,
            compaction_engine: None,
            event_bus: None,
        }
    }

    #[test]
    fn she_is_named_for_a_body_not_a_device() {
        // The naming is doctrine, not taste: §13 gives her the machine, and
        // a sense called `device` would be the one place the substrate
        // called her body equipment.
        assert_eq!(Body.name(), "body");
    }

    #[test]
    fn the_schema_offers_both_sensing_and_acting() {
        let schema = Body.parameter_schema();
        let verbs = schema["properties"]["verb"]["enum"].as_array().unwrap();
        let names: Vec<&str> = verbs.iter().filter_map(|v| v.as_str()).collect();
        for v in SENSE_VERBS.iter().chain(ACT_VERBS) {
            assert!(names.contains(v), "`{v}` must be reachable — §13");
        }
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        let action_names: Vec<&str> = actions.iter().filter_map(|v| v.as_str()).collect();
        for a in HAND_ACTIONS {
            assert!(action_names.contains(a), "`hand {a}` must be reachable");
        }
    }

    #[tokio::test]
    async fn a_missing_verb_is_refused_with_the_vocabulary() {
        let e = Body
            .execute(json!({}), &ctx())
            .await
            .expect_err("no verb is a caller error");
        assert!(e.to_string().contains("verb"));
    }

    #[tokio::test]
    async fn an_unknown_verb_names_what_she_can_actually_do() {
        let e = Body
            .execute(json!({ "verb": "levitate" }), &ctx())
            .await
            .expect_err("unknown verbs are caller errors");
        let msg = e.to_string();
        assert!(msg.contains("levitate"));
        assert!(msg.contains("device_state"), "tell her the real vocabulary");
    }

    #[tokio::test]
    async fn screen_without_a_direction_is_refused_before_the_socket() {
        // Wrong-arguments must not reach the daemon and come back as a
        // generic failure — it is a different next move from "not now".
        let e = Body
            .execute(json!({ "verb": "screen" }), &ctx())
            .await
            .expect_err("screen needs a direction");
        assert!(e.to_string().contains("on"));
    }

    #[tokio::test]
    async fn set_usb_mode_without_a_mode_is_refused_before_the_socket() {
        let e = Body
            .execute(json!({ "verb": "set_usb_mode" }), &ctx())
            .await
            .expect_err("a posture change needs a posture");
        assert!(e.to_string().contains("mode"));
    }

    #[tokio::test]
    async fn power_without_a_verb_is_refused_before_the_socket() {
        let e = Body
            .execute(json!({ "verb": "power" }), &ctx())
            .await
            .expect_err("the irreversible one needs saying out loud");
        assert!(e.to_string().contains("power_verb"));
    }

    #[tokio::test]
    async fn an_unreachable_sessiond_reads_as_numbness_not_failure() {
        // No daemon in the test environment. She must get a state she can
        // reason about, never a crash and never a confident "all fine".
        std::env::set_var("XDG_RUNTIME_DIR", "/nonexistent-souveraine-test");
        let out = Body
            .execute(json!({ "verb": "device_state" }), &ctx())
            .await
            .expect("absence is an answer, not an error");
        assert!(
            !out.is_error,
            "a body she cannot feel is not a tool failure"
        );
        assert!(
            out.content.contains("can't feel"),
            "say it plainly: {}",
            out.content
        );
        assert!(
            out.content.contains("not evidence that anything is wrong"),
            "absence must not be read as a negative: {}",
            out.content
        );
    }

    /// A shell shim so the reach can be exercised without quickshell. It
    /// answers `usbHands` status with a joined, armed hand and otherwise
    /// echoes `SOUVERAINE_SHIM_OUT` (default `queued`), failing with stderr
    /// when `SOUVERAINE_SHIM_FAIL` is set.
    const SHIM: &str = r#"#!/bin/sh
case "$6" in
  status) echo '{"active":true,"ready":true,"mode":"hid","error":""}' ;;
  *) echo "${SOUVERAINE_SHIM_OUT:-queued}"
     [ -z "$SOUVERAINE_SHIM_FAIL" ] || { echo "shim broke" >&2; exit 2; } ;;
esac
"#;

    fn write_shim() -> String {
        let dir = std::env::temp_dir().join(format!("souveraine-hand-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shim = dir.join("qs");
        std::fs::write(&shim, SHIM).unwrap();
        let _ = std::process::Command::new("chmod")
            .arg("+x")
            .arg(&shim)
            .status();
        shim.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn the_hand_speaks_the_quickshell_protocol() {
        let shim = write_shim();
        std::env::set_var("SOUVERAINE_QS", &shim);
        std::env::set_var("SOUVERAINE_SHIM_OUT", "queued");
        std::env::remove_var("SOUVERAINE_SHIM_FAIL");

        let status = Body
            .execute(json!({ "verb": "hand", "action": "status" }), &ctx())
            .await
            .expect("status is a question");
        assert!(!status.is_error);
        assert!(
            status.content.contains("\"mode\": \"hid\""),
            "{}",
            status.content
        );

        let typed = Body
            .execute(
                json!({ "verb": "hand", "action": "type", "text": "dir /b" }),
                &ctx(),
            )
            .await
            .expect("typing is a question of the shell, not of me");
        assert_eq!(typed.content, "sent");

        let keyed = Body
            .execute(
                json!({ "verb": "hand", "action": "key", "key": "enter", "modifiers": "ctrl alt" }),
                &ctx(),
            )
            .await
            .expect("keys go through the same bridge");
        assert_eq!(keyed.content, "sent");

        let clicked = Body
            .execute(
                json!({ "verb": "hand", "action": "click", "button": "right" }),
                &ctx(),
            )
            .await
            .expect("the button goes through too");
        assert_eq!(clicked.content, "sent");

        std::env::set_var("SOUVERAINE_SHIM_OUT", "USB Hands is not joined");
        let refused = Body
            .execute(
                json!({ "verb": "hand", "action": "type", "text": "ls" }),
                &ctx(),
            )
            .await
            .expect("a gate is an answer, not a crash");
        assert!(!refused.is_error);
        assert!(
            refused.content.contains("refused (refused_by_state)")
                && refused.content.contains("USB Hands is not joined"),
            "{}",
            refused.content
        );

        std::env::set_var("SOUVERAINE_SHIM_FAIL", "1");
        let failed = Body
            .execute(
                json!({ "verb": "hand", "action": "key", "key": "f5" }),
                &ctx(),
            )
            .await
            .expect("a broken bridge is still an answer");
        assert!(failed.content.contains("shim broke"), "{}", failed.content);

        std::env::set_var("SOUVERAINE_QS", "/nonexistent-souveraine-shell/qs");
        let absent = Body
            .execute(json!({ "verb": "hand", "action": "status" }), &ctx())
            .await
            .expect("a missing shell is absence, not failure");
        assert!(!absent.is_error);
        assert!(absent.content.contains("no hand"), "{}", absent.content);
    }

    #[tokio::test]
    async fn the_hand_validates_before_it_reaches_out() {
        for (input, needle) in [
            (json!({ "verb": "hand" }), "needs `action`"),
            (json!({ "verb": "hand", "action": "levitate" }), "no action"),
            (json!({ "verb": "hand", "action": "type" }), "needs `text`"),
            (
                json!({ "verb": "hand", "action": "type", "text": "" }),
                "non-empty",
            ),
            (
                json!({ "verb": "hand", "action": "type", "text": "½hello" }),
                "ASCII",
            ),
            (json!({ "verb": "hand", "action": "key" }), "needs `key`"),
            (
                json!({ "verb": "hand", "action": "key", "key": "alt gr" }),
                "key name",
            ),
            (
                json!({ "verb": "hand", "action": "key", "key": "f5", "modifiers": "ctrl+" }),
                "space-separated",
            ),
            (
                json!({ "verb": "hand", "action": "click", "button": "side" }),
                "left, right",
            ),
            (
                json!({ "verb": "hand", "action": "pointer" }),
                "needs `x` and `y`",
            ),
            (
                json!({ "verb": "hand", "action": "pointer", "x": 4 }),
                "needs `x` and `y`",
            ),
        ] {
            let shown = input.to_string();
            let err = Body
                .execute(input, &ctx())
                .await
                .expect_err("this is a caller error, not a state");
            assert!(
                err.to_string().contains(needle),
                "`{shown}` should refuse with `{needle}`, got: {err}"
            );
        }
    }
}
