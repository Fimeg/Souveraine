//! Voice service HTTP clients — STT (Faster-Whisper) and TTS (VibeVoice).
//!
//! Pure HTTP, no audio I/O. Both services run on 127.0.0.1 by default.
//!
//! - STT: `POST /transcribe` — multipart form, `audio` field (WAV), returns
//!   `{ "text": string, "language"?: string }`.
//! - TTS: `POST /audio/speech` — JSON `{ input, voice, model }`, returns mp3
//!   bytes.
//!
//! Text is cleaned before synthesis via [`clean_text_for_tts`].

use anyhow::{Context, Result};

// ── Pronunciation map ────────────────────────────────────────────────────
//
// Word-boundary replacements applied before synthesis.
// Ported from PRONUNCIATION_MAP in tts.ts lines 18–30.

const PRONUNCIATION_MAP: &[(&str, &str)] = &[
    ("Xzaviar",   "X-zay-V-ar"),
    ("xzaviar",   "X-zay-V-ar"),
    ("Jean Luc",  "Zhan-Look"),
    ("jean luc",  "Zhan-Look"),
    ("Sebastian", "Se-BASS-chen"),
    ("sebastian", "Se-BASS-chen"),
    ("API",       "A P I"),
    ("SDK",       "S D K"),
    ("E2EE",      "end-to-end encrypted"),
    ("TTS",       "text to speech"),
    ("STT",       "speech to text"),
];

/// Clean text for TTS synthesis.
///
/// Strips control tags, HTML, markdown, code blocks, most emoji.
/// Preserves ✨ and 🎤. Applies pronunciation fixes. Collapses whitespace.
///
/// Matches the TypeScript `cleanTextForTTS` in tts.ts lines 52–103.
pub fn clean_text_for_tts(text: &str) -> String {
    let mut s = text.to_string();

    // Strip agent control tags
    s = s.replace("[silent]", "");
    s = s.replace("[chromatophore]", "");
    s = s.replace("[!c]", "");
    s = s.replace("[!s]", "");
    // [react:...] tags
    {
        let re = regex::Regex::new(r"(?i)\[react:[^\]]*\]").unwrap();
        s = re.replace_all(&s, "").into_owned();
    }

    // Strip color syntax {color|text} → keep text
    {
        let re = regex::Regex::new(r"\{[^}|]+\|([^}]+)\}").unwrap();
        s = re.replace_all(&s, "$1").into_owned();
    }

    // Strip spoilers ||text|| → keep text
    {
        let re = regex::Regex::new(r"(?s)\|\|(.+?)\|\|").unwrap();
        s = re.replace_all(&s, "$1").into_owned();
    }

    // Strip HTML tags (keep content)
    {
        let re = regex::Regex::new(r"<[^>]+>").unwrap();
        s = re.replace_all(&s, "").into_owned();
    }

    // Remove code blocks entirely
    {
        let re = regex::Regex::new(r"(?s)```[\s\S]*?```").unwrap();
        s = re.replace_all(&s, "").into_owned();
    }

    // Strip bold and italic markers, keeping content
    {
        let re = regex::Regex::new(r"\*\*(.+?)\*\*").unwrap();
        s = re.replace_all(&s, "$1").into_owned();
    }
    {
        let re = regex::Regex::new(r"\*(.+?)\*").unwrap();
        s = re.replace_all(&s, "$1").into_owned();
    }

    // Remove inline code markers
    {
        let re = regex::Regex::new(r"`([^`]+)`").unwrap();
        s = re.replace_all(&s, "$1").into_owned();
    }

    // Convert markdown links: [text](url) → text
    {
        let re = regex::Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap();
        s = re.replace_all(&s, "$1").into_owned();
    }

    // Emoji handling: preserve ✨ and 🎤, strip the rest.
    // Use placeholder markers to survive the strip pass.
    const SPARKLE: &str = "__SPARKLE__";
    const MIC: &str = "__MIC__";
    s = s.replace('✨', SPARKLE);
    s = s.replace('🎤', MIC);
    {
        // Broad emoji ranges — same as the TypeScript version
        let re = regex::Regex::new(
            r"[\u{1F600}-\u{1F64F}\u{1F300}-\u{1F5FF}\u{1F680}-\u{1F6FF}\u{1F1E0}-\u{1F1FF}\u{2702}-\u{27B0}\u{24C2}-\u{1F251}\u{1F900}-\u{1FAFF}]"
        ).unwrap();
        s = re.replace_all(&s, "").into_owned();
    }
    s = s.replace(SPARKLE, "✨");
    s = s.replace(MIC, "🎤");

    // Pronunciation fixes — word-boundary replacements
    for &(wrong, right) in PRONUNCIATION_MAP {
        // Build a word-boundary regex. Escape special chars.
        let escaped = regex::escape(wrong);
        if let Ok(re) = regex::Regex::new(&format!(r"\b{}\b", escaped)) {
            s = re.replace_all(&s, right).into_owned();
        }
    }

    // Collapse whitespace
    {
        let re = regex::Regex::new(r"\s+").unwrap();
        s = re.replace_all(&s, " ").into_owned();
    }

    s.trim().to_string()
}

// ── VoiceClient ──────────────────────────────────────────────────────────

/// HTTP client for the STT (Faster-Whisper) and TTS (VibeVoice) services.
///
/// No retry logic — if a request fails, the caller treats the error as a
/// sensorium event (the voice is hoarse; the body has bad days).
pub struct VoiceClient {
    /// Faster-Whisper base URL, e.g. `http://127.0.0.1:7862`
    stt_url: String,
    /// VibeVoice base URL, e.g. `http://127.0.0.1:7861`
    tts_url: String,
    /// Voice ID for synthesis, e.g. `en-Soother_woman`
    voice: String,
    /// Shared reqwest client (connection pool)
    http: reqwest::Client,
}

impl VoiceClient {
    /// Build a client from config values. Panics if reqwest can't build
    /// (this only happens if TLS config is broken, which won't happen here).
    pub fn new(stt_url: &str, tts_url: &str, voice: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest client build failed");
        Self {
            stt_url: stt_url.trim_end_matches('/').to_string(),
            tts_url: tts_url.trim_end_matches('/').to_string(),
            voice: voice.to_string(),
            http,
        }
    }

    /// STT base URL (for cloning into spawn tasks).
    pub fn stt_url_str(&self) -> &str { &self.stt_url }
    /// TTS base URL (for cloning into spawn tasks).
    pub fn tts_url_str(&self) -> &str { &self.tts_url }
    /// Voice ID (for cloning into spawn tasks).
    pub fn voice_str(&self) -> &str { &self.voice }

    /// Transcribe a WAV buffer via Faster-Whisper.
    ///
    /// Sends `audio` as `audio/wav` multipart, plus `model=small`.
    /// Returns the trimmed transcription string, or an error.
    pub async fn transcribe(&self, wav_bytes: Vec<u8>) -> Result<String> {
        let url = format!("{}/transcribe", self.stt_url);

        let part = reqwest::multipart::Part::bytes(wav_bytes)
            .file_name("audio.wav")
            .mime_str("audio/wav")?;

        let form = reqwest::multipart::Form::new()
            .part("audio", part)
            .text("model", "small");

        let resp = self.http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .context("STT request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("STT {}: {}", status, body));
        }

        #[derive(serde::Deserialize)]
        struct SttResult {
            text: String,
        }

        let result: SttResult = resp.json().await.context("STT response decode failed")?;
        Ok(result.text.trim().to_string())
    }

    /// Synthesize speech via VibeVoice.
    ///
    /// Cleans the text with [`clean_text_for_tts`] before sending.
    /// Returns raw mp3 bytes.
    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        let url = format!("{}/audio/speech", self.tts_url);
        let cleaned = clean_text_for_tts(text);

        #[derive(serde::Serialize)]
        struct TtsRequest<'a> {
            input: &'a str,
            voice: &'a str,
            model: &'static str,
        }

        let body = TtsRequest {
            input: &cleaned,
            voice: &self.voice,
            model: "vibevoice-v1",
        };

        let resp = self.http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("TTS request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("TTS {}: {}", status, body_text));
        }

        let bytes = resp.bytes().await.context("TTS bytes read failed")?;
        Ok(bytes.to_vec())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_markdown() {
        let raw = "**hello** `world` [link](http://example.com)";
        let cleaned = clean_text_for_tts(raw);
        assert_eq!(cleaned, "hello world link");
    }

    #[test]
    fn clean_strips_code_blocks() {
        let raw = "here is code:\n```\nfn main() {}\n```\nend";
        let cleaned = clean_text_for_tts(raw);
        assert!(!cleaned.contains("fn main"));
        assert!(cleaned.contains("here is code"));
        assert!(cleaned.contains("end"));
    }

    #[test]
    fn clean_preserves_sparkle_and_mic() {
        let raw = "✨ hello 🎤";
        let cleaned = clean_text_for_tts(raw);
        assert!(cleaned.contains("✨"));
        assert!(cleaned.contains("🎤"));
    }

    #[test]
    fn clean_strips_control_tags() {
        let raw = "[silent] hello [!c] world [react:smile] end";
        let cleaned = clean_text_for_tts(raw);
        assert!(!cleaned.contains("[silent]"));
        assert!(!cleaned.contains("[!c]"));
        assert!(!cleaned.contains("[react"));
        assert!(cleaned.contains("hello"));
        assert!(cleaned.contains("world"));
        assert!(cleaned.contains("end"));
    }

    #[test]
    fn clean_pronunciation_fixes_api() {
        let raw = "the API call failed";
        let cleaned = clean_text_for_tts(raw);
        assert!(cleaned.contains("A P I"));
        assert!(!cleaned.contains("API"));
    }

    #[test]
    fn clean_collapses_whitespace() {
        let raw = "  hello   world  ";
        let cleaned = clean_text_for_tts(raw);
        assert_eq!(cleaned, "hello world");
    }
}
