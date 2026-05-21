use std::time::{Duration, Instant};

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};

use crate::ui::atmosphere::lerp_color;
use crate::ui::markdown;

use super::{
    BtwState, ChatMessage, ChatMode, ChatPalette, ChatState, MarkdownCache,
    MsgLayout, ToolResultBlock, TurnPhase, SPINNER,
};
use super::wrap::{count_visual_lines, wrap_input_line, wrap_words};
use super::cockpit::draw_cockpit;
use super::footer::draw_footer;
use super::overlays::{draw_btw_pane, draw_esc_overlay, draw_overlay};

pub fn draw(f: &mut Frame, state: &ChatState) {
    let area = f.size();

    let input_inner_width = (area.width as usize).saturating_sub(5).max(1);
    let input_visual_lines = count_visual_lines(&state.input, input_inner_width);
    let max_input_lines = ((area.height as usize) * 40 / 100).max(1);
    let input_height = (input_visual_lines.min(max_input_lines) as u16) + 2;

    let phase_height: u16 = if state.busy
        || state.phase == TurnPhase::Interrupted
        || state.phase == TurnPhase::Subconscious
    { 1 } else { 0 };

    let itinerary_height: u16 = if state.itinerary_line.is_empty() { 0 } else { 1 };

    let header_section = 1 + itinerary_height;

    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_section),
            Constraint::Min(5),
            Constraint::Length(phase_height),
            Constraint::Length(input_height),
            Constraint::Length(1),
        ])
        .split(area);

    if itinerary_height > 0 {
        // Split the header area into two rows: main header + itinerary strip
        let header_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(vchunks[0]);
        draw_header(f, state, header_rows[0]);
        draw_itinerary(f, state, header_rows[1]);
    } else {
        draw_header(f, state, vchunks[0]);
    }

    if state.cockpit {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(36)])
            .split(vchunks[1]);
        draw_messages(f, state, body[0]);
        draw_cockpit(f, state, body[1]);
    } else {
        draw_messages(f, state, vchunks[1]);
    }

    if phase_height > 0 {
        draw_phase(f, state, vchunks[2]);
    }
    draw_input(f, state, vchunks[3]);
    draw_footer(f, state, vchunks[4]);

    draw_overlay(f, state, area, vchunks[3]);

    if !matches!(state.btw_state, BtwState::Idle) {
        draw_btw_pane(f, state, area);
    }

    if state.show_esc_overlay {
        draw_esc_overlay(f, state, area);
    }
}

fn draw_phase(f: &mut Frame, state: &ChatState, area: Rect) {
    let elapsed = state
        .turn_started
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);
    let spinner = SPINNER[(state.tick as usize / 2) % SPINNER.len()];

    let (glyph, label, color) = match state.phase {
        TurnPhase::Thinking | TurnPhase::Idle => (spinner, "Thinking".to_string(), state.palette.agent_primary),
        TurnPhase::Tool => {
            let label = if state.tool_calls_this_turn == 1 {
                "Running tool · 1 tool used".to_string()
            } else {
                format!("Running tool · {} tools used", state.tool_calls_this_turn)
            };
            (spinner, label, state.palette.tool_accent)
        }
        TurnPhase::Streaming => (spinner, "Streaming".to_string(), state.palette.agent_primary),
        TurnPhase::Interrupted => ("×", "Interrupted".to_string(), state.palette.compaction),
        TurnPhase::Subconscious => (spinner, "Subconscious".to_string(), state.palette.surfacing),
    };
    let queued = state
        .pending_interjections
        .lock()
        .ok()
        .map(|q| q.len())
        .unwrap_or(0);

    let quiet_secs = state.last_event_at.elapsed().as_secs();
    let liveness = if quiet_secs >= 120 {
        Some(format!("still waiting {}s...", quiet_secs))
    } else if quiet_secs >= 5 {
        Some(format!("waiting {}s...", quiet_secs))
    } else {
        None
    };

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(format!(" {} ", glyph), Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{}", label), Style::default().fg(color)),
        Span::styled(format!("  ·  {}s", elapsed), Style::default().fg(state.palette.agent_dim)),
    ];
    if let Some(liveness_label) = liveness {
        spans.push(Span::styled(
            format!("  ·  {}", liveness_label),
            Style::default().fg(state.palette.surfacing).add_modifier(Modifier::ITALIC),
        ));
    }
    if queued > 0 {
        spans.push(Span::styled(
            format!("  ·  {} queued", queued),
            Style::default().fg(state.palette.surfacing).add_modifier(Modifier::ITALIC),
        ));
    }
    let line = Line::from(spans);
    f.render_widget(Paragraph::new(line).alignment(Alignment::Left), area);
}

fn draw_header(f: &mut Frame, state: &ChatState, area: Rect) {
    let mode_color = match state.mode.as_str() {
        "local" => state.palette.tool_accent,
        "remote" => state.palette.agent_primary,
        _ => state.palette.agent_dim,
    };
    let posture_label = if state.tool_cards_expanded {
        "tools shown"
    } else {
        "tools folded"
    };
    let posture_color = if state.tool_cards_expanded {
        state.palette.tool_accent
    } else {
        state.palette.agent_dim
    };
    let mut spans = vec![
        Span::styled("✦ Souveraine ", Style::default().fg(state.palette.agent_primary).add_modifier(Modifier::BOLD)),
        Span::styled(format!("· {} ", state.agent_name), Style::default().fg(Color::White)),
    ];
    // Code mode pill — distinct visual indicator when in code rendering mode
    if state.render_mode == super::ChatMode::Code {
        spans.push(Span::styled(
            " CODE MODE ",
            Style::default()
                .fg(Color::Black)
                .bg(state.palette.tool_accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(format!("[{}] ", state.mode), Style::default().fg(mode_color)));
    spans.push(Span::styled(format!("[{}]", posture_label), Style::default().fg(posture_color)));
    let title = Line::from(spans);
    f.render_widget(Paragraph::new(title).alignment(Alignment::Center), area);
}

/// Draw the itinerary strip — shows the current route with stop indicators.
/// Only rendered when there is an active itinerary.
fn draw_itinerary(f: &mut Frame, state: &ChatState, area: Rect) {
    let glyph_color = state.palette.agent_dim;
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  ⟡ ",
                Style::default().fg(state.palette.surfacing),
            ),
            Span::styled(
                &state.itinerary_line,
                Style::default().fg(glyph_color),
            ),
        ])).alignment(Alignment::Left),
        area,
    );
}

fn entry_intensity(ts: Instant) -> f32 {
    const ENTRY_MS: f32 = 450.0;
    const SHIMMER_MAX: f32 = 0.5;
    let age = ts.elapsed().as_millis() as f32;
    if age >= ENTRY_MS {
        return 0.0;
    }
    let t = age / ENTRY_MS;
    (t * std::f32::consts::PI).sin().max(0.0) * SHIMMER_MAX
}

fn msg_entry_ts(msg: &ChatMessage) -> Option<Instant> {
    match msg {
        ChatMessage::User { ts, .. }
        | ChatMessage::Assistant { ts, .. }
        | ChatMessage::Surfacing { ts, .. }
        | ChatMessage::System { ts, .. }
        | ChatMessage::Interjection { ts, .. }
        | ChatMessage::Image { ts, .. } => Some(*ts),
        _ => None,
    }
}

fn shimmer_lines(lines: &mut [Line<'static>], t: f32) {
    let blend = |x: u8| (x as f32 + (255.0 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    for line in lines.iter_mut() {
        for span in line.spans.iter_mut() {
            if let Some(Color::Rgb(r, g, b)) = span.style.fg {
                span.style.fg = Some(Color::Rgb(blend(r), blend(g), blend(b)));
            }
        }
    }
}

fn fade_streaming_tail(mut lines: Vec<Line<'static>>, palette: &ChatPalette) -> Vec<Line<'static>> {
    const FADE: [f32; 3] = [0.62, 0.34, 0.13];
    let (tr, tg, tb) = match palette.bg {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (12u8, 14u8, 18u8),
    };
    let n = lines.len();
    for (offset, &amount) in FADE.iter().enumerate() {
        if offset >= n {
            break;
        }
        let blend = |x: u8, t: u8| {
            (x as f32 + (t as f32 - x as f32) * amount).round().clamp(0.0, 255.0) as u8
        };
        for span in lines[n - 1 - offset].spans.iter_mut() {
            if let Some(Color::Rgb(r, g, b)) = span.style.fg {
                span.style.fg = Some(Color::Rgb(blend(r, tr), blend(g, tg), blend(b, tb)));
            }
        }
    }
    lines
}

fn with_stream_cursor(mut lines: Vec<Line<'static>>, accent: Color) -> Vec<Line<'static>> {
    if let Some(last) = lines.last_mut() {
        last.spans.push(Span::styled(
            "▌",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    }
    lines
}

fn tool_name_pulse(elapsed: Duration) -> Color {
    let t = (elapsed.as_secs_f32() * 2.0).sin() * 0.5 + 0.5;
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color::Rgb(lerp(80, 186), lerp(139, 200), lerp(220, 255))
}

fn tool_footer_label(indices: &[usize], msgs: &[ChatMessage]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut run: Option<(String, usize)> = None;
    let flush = |run: &mut Option<(String, usize)>, parts: &mut Vec<String>| {
        if let Some((n, c)) = run.take() {
            parts.push(if c > 1 { format!("{} ×{}", n, c) } else { n });
        }
    };
    for &i in indices {
        if let Some(ChatMessage::Tool { name, .. }) = msgs.get(i) {
            match &mut run {
                Some((n, c)) if n == name => *c += 1,
                _ => {
                    flush(&mut run, &mut parts);
                    run = Some((name.clone(), 1));
                }
            }
        }
    }
    flush(&mut run, &mut parts);
    format!("⚙ {}", parts.join(" · "))
}

fn draw_messages(f: &mut Frame, state: &ChatState, area: Rect) {
    let mdpal = crate::ui::markdown::MarkdownPalette::from_chat_palette(&state.palette);
    let max_bubble = ((area.width as usize).saturating_sub(8) * 70 / 100).max(20);
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Fold pass only runs in Chat mode when tools are hidden.
    // Code mode always shows tools (compact when closed, full when open).
    let do_fold = !state.tool_cards_expanded && state.render_mode == ChatMode::Conversation;
    let mut fold_footer: std::collections::HashMap<usize, String> =
        std::collections::HashMap::new();
    let mut consumed: std::collections::HashSet<usize> = std::collections::HashSet::new();
    if do_fold {
        let msgs = &state.messages;
        let mut leading: Vec<usize> = Vec::new();
        for (idx, m) in msgs.iter().enumerate() {
            match m {
                ChatMessage::Tool { .. } => leading.push(idx),
                ChatMessage::Assistant { .. } => {
                    // Scan forward through the tool block, skipping Interstitial/Interjection
                    // entries that sit between the bubble and the tool block (narration
                    // from turn.rs). Only tools get consumed into the fold footer; the
                    // narration renders independently in the second pass.
                    let mut j = idx + 1;
                    while j < msgs.len() {
                        match &msgs[j] {
                            ChatMessage::Tool { .. } => j += 1,
                            ChatMessage::Interstitial { .. } | ChatMessage::Interjection { .. } => {
                                j += 1;
                            }
                            _ => break,
                        }
                    }
                    // Gather leading tools that arrived before any Assistant (e.g. first
                    // turn's tool calls fired before narration). Filter out any that were
                    // already consumed by a prior Assistant's gap scan.
                    let owned: Vec<usize> = std::mem::take(&mut leading)
                        .into_iter()
                        .filter(|i| !consumed.contains(i))
                        .collect();
                    // Only tools in the gap get consumed — interstitials/interjections
                    // render independently in the second pass below.
                    let gap_tools: Vec<usize> = ((idx + 1)..j)
                        .filter(|&i| matches!(msgs[i], ChatMessage::Tool { .. }))
                        .collect();
                    let all_indices: Vec<usize> = owned.into_iter().chain(gap_tools).collect();
                    if !all_indices.is_empty() {
                        for &i in &all_indices {
                            consumed.insert(i);
                        }
                        fold_footer.insert(idx, tool_footer_label(&all_indices, msgs));
                    }
                }
                // Interstitials/interjections sit between the bubble and tool block;
                // don't clear leading — they don't break the tool-to-assistant flow.
                ChatMessage::Interstitial { .. } | ChatMessage::Interjection { .. } => {}
                _ => leading.clear(),
            }
        }
    }

    let mut span_spans: Vec<(usize, usize, usize)> = Vec::new();
    let mut prev_was_tool = false;
    for (idx, msg) in state.messages.iter().enumerate() {
        if consumed.contains(&idx) {
            continue;
        }
        let this_is_tool = matches!(msg, ChatMessage::Tool { .. });
        if this_is_tool != prev_was_tool && !lines.is_empty() {
            lines.push(Line::from(""));
        }
        prev_was_tool = this_is_tool;
        let line_start = lines.len();
        match msg {
            ChatMessage::User { text, .. } => {
                lines.extend(bubble(
                    &format!("⧉ {}", state.human_name),
                    text,
                    max_bubble,
                    Style::default().fg(state.palette.user_accent),
                    BubbleAlign::Right,
                    area.width,
                ));
                lines.push(Line::from(""));
            }
            ChatMessage::Assistant { text, streaming, rendered_cache, .. } => {
                let label = if *streaming {
                    format!("⧉ {} ◦", state.agent_name)
                } else {
                    format!("⧉ {}", state.agent_name)
                };
                let inner_width = max_bubble.saturating_sub(4).max(8);
                let body_lines = if text.is_empty() && *streaming {
                    vec![Line::from("…")]
                } else {
                    let key_len = text.len();
                    let palette_hash = state.palette.hash();
                    let agent_color = state.palette.agent_primary;
                    let cached = rendered_cache.borrow();
                    if let Some(c) = &*cached {
                        if c.text_len == key_len && c.inner_width == inner_width && c.palette_hash == palette_hash {
                            c.lines.clone()
                        } else {
                            drop(cached);
                            let lines = markdown::render_with_width(text, agent_color, Some(inner_width), &mdpal);
                            *rendered_cache.borrow_mut() = Some(MarkdownCache {
                                text_len: key_len,
                                inner_width,
                                palette_hash,
                                lines: lines.clone(),
                            });
                            lines
                        }
                    } else {
                        drop(cached);
                        let lines = markdown::render_with_width(text, agent_color, Some(inner_width), &mdpal);
                        *rendered_cache.borrow_mut() = Some(MarkdownCache {
                            text_len: key_len,
                            inner_width,
                            palette_hash,
                            lines: lines.clone(),
                        });
                        lines
                    }
                };
                let body_lines = if *streaming && !text.is_empty() {
                    with_stream_cursor(
                        fade_streaming_tail(body_lines, &state.palette),
                        state.palette.agent_primary,
                    )
                } else {
                    body_lines
                };
                lines.extend(bubble_rendered(
                    &label,
                    &body_lines,
                    max_bubble,
                    Style::default().fg(state.palette.agent_primary),
                    BubbleAlign::Left,
                    area.width,
                    fold_footer.get(&idx).map(|s| s.as_str()),
                ));
                lines.push(Line::from(""));
            }
            ChatMessage::Surfacing { source, content, priority, .. } => {
                if priority == "low" {
                    let brief = if content.len() > 90 {
                        format!("{}…", &content[..content.floor_char_boundary(87)])
                    } else {
                        content.clone()
                    };
                    lines.push(Line::from(Span::styled(
                        format!("  · [{}] {}", source, brief),
                        Style::default().fg(state.palette.surfacing).add_modifier(Modifier::ITALIC),
                    )));
                    lines.push(Line::from(""));
                } else {
                    let label = format!("surfacing · {} · {}", source, priority);
                    lines.extend(bubble(
                        &label,
                        content,
                        max_bubble.min(60),
                        Style::default().fg(state.palette.surfacing),
                        BubbleAlign::Center,
                        area.width,
                    ));
                    lines.push(Line::from(""));
                }
            }
            ChatMessage::System { text, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("  · {}", text),
                    Style::default().fg(state.palette.agent_dim).add_modifier(Modifier::ITALIC),
                )));
                lines.push(Line::from(""));
            }
            ChatMessage::Tool { name, arguments, round, result, ts, .. } => {
                let expand = state.tool_cards_expanded;
                let name_pulse = if result.is_none() {
                    Some(tool_name_pulse(ts.elapsed()))
                } else {
                    None
                };
                if expand {
                    lines.extend(render_tool_card(
                        name,
                        arguments,
                        *round,
                        result.as_ref(),
                        max_bubble,
                        area.width,
                        &state.palette,
                        name_pulse,
                    ));
                    lines.push(Line::from(""));
                } else {
                    lines.extend(render_tool_card_compact(
                        name,
                        arguments,
                        *round,
                        result.as_ref(),
                        area.width,
                        &state.palette,
                        name_pulse,
                    ));
                }
            }
            ChatMessage::Interjection { text, delivered, .. } => {
                let (glyph, label, color) = if *delivered {
                    ("✋", "noticed", state.palette.agent_dim)
                } else {
                    ("✋", "hand raised", state.palette.surfacing)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("  {} {}  ", glyph, label),
                        Style::default().fg(color).add_modifier(Modifier::BOLD)),
                    Span::styled(text.clone(), Style::default().fg(color).add_modifier(Modifier::ITALIC)),
                ]));
                lines.push(Line::from(""));
            }
            ChatMessage::Image { media_type, label, dimensions, .. } => {
                let dim_str = dimensions.map(|(w,h)| format!("{}x{}", w, h)).unwrap_or_default();
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("🖼", Style::default().fg(state.palette.user_accent)),
                    Span::raw(" "),
                    Span::styled(label.clone(), Style::default().fg(state.palette.user_accent).add_modifier(Modifier::BOLD)),
                    Span::styled(format!(" · {}", media_type), Style::default().fg(state.palette.agent_dim)),
                ]));
                lines.push(Line::from(""));
            }
            ChatMessage::Interstitial { text, register } => {
                match register {
                    crate::backend::Register::Cenno => {
                        lines.push(Line::from(Span::styled(
                            format!("  ⟡ {} ", text),
                            Style::default()
                                .fg(state.palette.agent_dim)
                                .add_modifier(Modifier::ITALIC),
                        )));
                    }
                    crate::backend::Register::HerVoice => {
                        let bar = Style::default()
                            .fg(state.palette.agent_primary)
                            .add_modifier(Modifier::DIM);
                        let body = Style::default().fg(state.palette.agent_primary);
                        let wrap_w = (area.width as usize).saturating_sub(6).max(20);
                        for seg in wrap_words(text, wrap_w) {
                            lines.push(Line::from(vec![
                                Span::styled("  ▌ ", bar),
                                Span::styled(seg, body),
                            ]));
                        }
                    }
                }
                lines.push(Line::from(""));
            }
        }
        if let Some(ts) = msg_entry_ts(msg) {
            let intensity = entry_intensity(ts);
            if intensity > 0.0 {
                shimmer_lines(&mut lines[line_start..], intensity);
            }
        }
        span_spans.push((idx, line_start, lines.len()));
    }

    let visible_width = area.width.saturating_sub(0) as usize;
    let mut wrap_remap: Vec<usize> = Vec::with_capacity(lines.len() + 1);
    let mut wrapped_lines: Vec<Line<'static>> = Vec::with_capacity(lines.len());
    for line in lines {
        wrap_remap.push(wrapped_lines.len());
        wrapped_lines.extend(markdown::wrap_line(line, visible_width));
    }
    wrap_remap.push(wrapped_lines.len());
    let lines = wrapped_lines;
    for (_, s, e) in span_spans.iter_mut() {
        *s = wrap_remap.get(*s).copied().unwrap_or(*s);
        *e = wrap_remap.get(*e).copied().unwrap_or(*e);
    }

    let trailing_empty = lines
        .iter()
        .rev()
        .take_while(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
        .count();
    let effective_total = lines.len().saturating_sub(trailing_empty);

    let view = area.height.saturating_sub(2) as usize;
    let max_scroll = effective_total.saturating_sub(view);
    let user_scroll = (state.scroll as usize).min(max_scroll);
    let offset = max_scroll.saturating_sub(user_scroll) as u16;

    *state.msg_layout.borrow_mut() = MsgLayout {
        area,
        offset,
        spans: std::mem::take(&mut span_spans),
    };

    let para = Paragraph::new(lines)
        .scroll((offset, 0))
        .block(
            Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::default().fg(state.palette.agent_dim))
                .border_type(BorderType::Plain),
        );
    f.render_widget(para, area);
}

#[derive(Clone, Copy)]
enum BubbleAlign {
    Left,
    Right,
    Center,
}

fn bubble(
    title: &str,
    body: &str,
    max_width: usize,
    border: Style,
    align: BubbleAlign,
    container_width: u16,
) -> Vec<Line<'static>> {
    let max_inner = max_width.saturating_sub(4).max(8);
    let wrapped = wrap_words(body, max_inner);
    let widest = wrapped
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(0)
        .max(title.chars().count() + 2);
    let inner = widest.min(max_inner);
    let outer = inner + 4;

    let title_text = format!(" {} ", title);
    let dashes = outer.saturating_sub(2 + title_text.chars().count());
    let left_dash = "─".repeat(dashes / 2);
    let right_dash = "─".repeat(dashes - dashes / 2);

    let pad = match align {
        BubbleAlign::Left => 2,
        BubbleAlign::Right => (container_width as usize).saturating_sub(outer + 2),
        BubbleAlign::Center => (container_width as usize).saturating_sub(outer) / 2,
    };
    let pad_str = " ".repeat(pad);

    let mut lines = Vec::new();

    let top = format!("{}╭{}{}{}╮", pad_str, left_dash, title_text, right_dash);
    lines.push(Line::from(Span::styled(top, border)));

    for chunk in &wrapped {
        let chunk_width = chunk.chars().count();
        let inner_pad = inner.saturating_sub(chunk_width);
        let line_str = format!("{}│ {}{} │", pad_str, chunk, " ".repeat(inner_pad));
        let mut spans = Vec::new();
        spans.push(Span::raw(pad_str.clone()));
        spans.push(Span::styled("│ ", border));
        spans.push(Span::raw(chunk.clone()));
        if inner_pad > 0 {
            spans.push(Span::raw(" ".repeat(inner_pad)));
        }
        spans.push(Span::styled(" │", border));
        let _ = line_str;
        lines.push(Line::from(spans));
    }

    let bottom = format!("{}╰{}╯", pad_str, "─".repeat(outer - 2));
    lines.push(Line::from(Span::styled(bottom, border)));

    lines
}

fn bubble_rendered(
    title: &str,
    body_lines: &[Line<'static>],
    max_width: usize,
    border: Style,
    align: BubbleAlign,
    container_width: u16,
    footer: Option<&str>,
) -> Vec<Line<'static>> {
    let max_inner = max_width.saturating_sub(4).max(8);
    let footer_w = footer
        .filter(|f| !f.is_empty())
        .map(|f| f.chars().count() + 2)
        .unwrap_or(0);
    let widest = body_lines
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(0)
        .max(title.chars().count() + 2)
        .max(footer_w);
    let inner = widest.min(max_inner);
    let outer = inner + 4;

    let title_text = format!(" {} ", title);
    let dashes = outer.saturating_sub(2 + title_text.chars().count());
    let left_dash = "─".repeat(dashes / 2);
    let right_dash = "─".repeat(dashes - dashes / 2);

    let pad = match align {
        BubbleAlign::Left => 2,
        BubbleAlign::Right => (container_width as usize).saturating_sub(outer + 2),
        BubbleAlign::Center => (container_width as usize).saturating_sub(outer) / 2,
    };
    let pad_str = " ".repeat(pad);

    let mut lines = Vec::new();

    let top = format!("{}╭{}{}{}╮", pad_str, left_dash, title_text, right_dash);
    lines.push(Line::from(Span::styled(top, border)));

    for line in body_lines {
        let chunk_width = line.width();
        let inner_pad = inner.saturating_sub(chunk_width);
        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::raw(pad_str.clone()));
        spans.push(Span::styled("│ ", border));
        spans.extend(line.spans.iter().cloned());
        if inner_pad > 0 {
            spans.push(Span::raw(" ".repeat(inner_pad)));
        }
        spans.push(Span::styled(" │", border));
        lines.push(Line::from(spans));
    }

    let bottom = match footer {
        Some(f) if !f.is_empty() => {
            let ftext = format!(" {} ", f);
            let fdashes = outer.saturating_sub(2 + ftext.chars().count());
            let fl = "─".repeat(fdashes / 2);
            let fr = "─".repeat(fdashes - fdashes / 2);
            format!("{}╰{}{}{}╯", pad_str, fl, ftext, fr)
        }
        _ => format!("{}╰{}╯", pad_str, "─".repeat(outer - 2)),
    };
    lines.push(Line::from(Span::styled(bottom, border)));

    lines
}

fn render_tool_card(
    name: &str,
    arguments: &str,
    round: u32,
    result: Option<&ToolResultBlock>,
    max_width: usize,
    container_width: u16,
    palette: &ChatPalette,
    name_pulse: Option<Color>,
) -> Vec<Line<'static>> {
    let is_err = result.map(|r| r.is_error).unwrap_or(false);
    let border_color = if is_err {
        palette.compaction
    } else if let Some(p) = name_pulse {
        p
    } else {
        palette.tool_accent
    };
    let dim_color = if is_err { palette.compaction } else { palette.tool_dim };
    let border = Style::default().fg(border_color);

    let glyph = match result {
        None => '⟳',
        Some(r) if r.is_error => '✗',
        Some(_) => '✓',
    };
    let title = format!("{} {}  ·  round {}", glyph, name, round);

    // Delegate body rendering to per-tool renderers (Tier 3)
    let inner_width = max_width.saturating_sub(4).max(8);
    let (args_lines, rendered_body) =
        super::tool_renderers::render_card_body(
            name, arguments,
            result.map(|r| r.output.as_str()),
            result.map(|r| r.is_error).unwrap_or(false),
            inner_width, palette,
        );

    let mut body_lines: Vec<Line<'static>> = Vec::new();
    body_lines.extend(args_lines);
    let has_body = !rendered_body.is_empty();

    if has_body {
        body_lines.push(Line::from(""));
        body_lines.extend(rendered_body);
    }

    // Truncation notice — only for fallback cards that didn't get per-tool
    // rendering (per-tool renderers add their own footers).
    if let Some(r) = result {
        let total = r.output.lines().count();
        if total > 30 && !has_body {
            body_lines.push(Line::from(Span::styled(
                format!("  … ({} total lines)", total),
                Style::default().fg(dim_color).add_modifier(Modifier::ITALIC),
            )));
        }
    }

    bubble_rendered(&title, &body_lines, max_width, border, BubbleAlign::Left, container_width, None)
}

fn render_tool_card_compact(
    name: &str,
    arguments: &str,
    round: u32,
    result: Option<&ToolResultBlock>,
    container_width: u16,
    palette: &ChatPalette,
    name_pulse: Option<Color>,
) -> Vec<Line<'static>> {
    let is_err = result.map(|r| r.is_error).unwrap_or(false);
    let pending = result.is_none();
    let pulse = name_pulse.unwrap_or(palette.tool_accent);
    let (glyph, glyph_color) = match (pending, is_err) {
        (true, _) => ("⟳", pulse),
        (false, true) => ("✗", palette.compaction),
        (false, false) => ("✓", palette.tool_accent),
    };

    let name_color = if is_err {
        palette.compaction
    } else if pending {
        pulse
    } else {
        palette.tool_accent
    };
    let dim = if is_err { palette.compaction } else { palette.tool_dim };

    let reserved = name.chars().count() + 14;
    let arg_budget = (container_width as usize)
        .saturating_sub(reserved + 6)
        .max(20)
        .min(120);
    let args_summary = clip(&super::tool_renderers::summarize_tool_args(name, arguments), arg_budget);

    let mut spans: Vec<Span<'static>> = vec![
        Span::raw("  "),
        Span::styled(glyph.to_string(), Style::default().fg(glyph_color)),
        Span::raw(" "),
        Span::styled(
            name.to_string(),
            Style::default().fg(name_color).add_modifier(Modifier::BOLD),
        ),
    ];
    if !args_summary.is_empty() {
        spans.push(Span::styled("  ·  ", Style::default().fg(dim)));
        spans.push(Span::styled(args_summary, Style::default().fg(dim)));
    }
    if round > 1 {
        spans.push(Span::styled(
            format!("  ·  r{}", round),
            Style::default().fg(dim).add_modifier(Modifier::DIM),
        ));
    }

    let mut out = vec![Line::from(spans)];

    // Per-tool result detail line (Tier 2)
    if let Some(r) = result {
        if let Some(detail) = super::tool_renderers::compact_result_details(name, &r.output, r.is_error) {
            let inner = (container_width as usize).saturating_sub(8).max(20);
            let preview = clip(&detail, inner);
            let detail_color = if r.is_error { palette.compaction } else { palette.tool_dim };
            out.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    preview,
                    Style::default().fg(detail_color).add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }

    out
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn draw_input(f: &mut Frame, state: &ChatState, area: Rect) {
    let border_color = if state.busy {
        let phase = (state.tick as f32 / 8.0).sin().abs();
        lerp_color(state.palette.agent_dim, state.palette.agent_primary, phase)
    } else {
        state.palette.agent_primary
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color));

    let cursor_visible = (state.tick / 5) % 2 == 0;
    let cursor_ch: &str = if cursor_visible { "▏" } else { " " };
    let inner_width = (area.width as usize).saturating_sub(5).max(1);

    let (prefix_str, prefix_color) = match state.render_mode {
        ChatMode::Conversation => (" › ", state.palette.agent_primary),
        ChatMode::Code => (" ≡ ", state.palette.tool_accent),
    };
    let prefix_style = Style::default().fg(prefix_color).add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let logical: Vec<&str> = state.input.split('\n').collect();

    // Find the visual line index (li, chunk pos) that contains the cursor.
    let mut cursor_byte_remaining = state.input_cursor;
    let mut cursor_visual_line: Option<(usize, usize)> = None; // (logical_line, char_offset)
    for (li, ll) in logical.iter().enumerate() {
        let line_len = ll.len();
        if cursor_byte_remaining <= line_len {
            // Cursor is on this logical line — find which visual chunk.
            let chars: Vec<char> = ll.chars().collect();
            let char_offset = ll[..cursor_byte_remaining].chars().count();
            let chunks = wrap_input_line(&chars, inner_width);
            let mut consumed = 0;
            for (pos, &(cs, ce)) in chunks.iter().enumerate() {
                if char_offset < consumed + (ce - cs) || char_offset == consumed && (ce - cs) == 0 {
                    cursor_visual_line = Some((li, consumed + cs + (char_offset - consumed)));
                    break;
                }
                consumed += ce - cs;
            }
            // If cursor is at the very end of the logical line, it's on the last chunk.
            if cursor_visual_line.is_none() && !chunks.is_empty() {
                let last = chunks.len() - 1;
                cursor_visual_line = Some((li, ll.len()));
            }
            break;
        }
        cursor_byte_remaining = cursor_byte_remaining.saturating_sub(line_len + 1); // +1 for \n
    }

    for (li, logical_line) in logical.iter().enumerate() {
        let chars: Vec<char> = logical_line.chars().collect();
        let chunks = wrap_input_line(&chars, inner_width);
        for (pos, &(chunk_start, chunk_end)) in chunks.iter().enumerate() {
            let chunk: String = chars[chunk_start..chunk_end].iter().collect();
            let is_first = li == 0 && pos == 0;
            let prefix: Span<'static> = if is_first {
                Span::styled(prefix_str.to_string(), prefix_style)
            } else {
                Span::raw("   ")
            };
            let mut spans = vec![prefix];
            // Check if cursor is on this visual line.
            let is_cursor_line = cursor_visual_line == Some((li, pos));
            if is_cursor_line {
                // Compute char offset within this chunk.
                let (_, char_offset) = cursor_visual_line.unwrap();
                let in_chunk_offset = char_offset.saturating_sub(chunk_start).min(chunk.len());
                let before: String = chunk.chars().take(in_chunk_offset).collect();
                let after: String = chunk.chars().skip(in_chunk_offset).collect();
                spans.push(Span::styled(before, Style::default().fg(Color::White)));
                spans.push(Span::styled(cursor_ch.to_string(), Style::default().fg(prefix_color)));
                spans.push(Span::styled(after, Style::default().fg(Color::White)));
            } else {
                spans.push(Span::styled(chunk, Style::default().fg(Color::White)));
            }
            lines.push(Line::from(spans));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(prefix_str.to_string(), prefix_style),
            Span::styled(cursor_ch.to_string(), Style::default().fg(prefix_color)),
        ]));
    }

    let visible_height = area.height.saturating_sub(2) as usize;
    let scroll = if lines.len() > visible_height {
        (lines.len() - visible_height) as u16
    } else {
        0
    };

    let para = Paragraph::new(lines).scroll((scroll, 0)).block(block);
    f.render_widget(para, area);
}
