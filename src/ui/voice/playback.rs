//! mp3 playback via rodio — the voice channel's output path.
//!
//! `VoicePlayer` wraps a rodio `Sink` and `OutputStream`. Calling `play_mp3`
//! decodes the bytes and appends them to the sink; playback is asynchronous.
//! The UI polls `is_speaking()` to know when the Posture should drop back to
//! Idle. `stop()` is the Esc interrupt.

use anyhow::{Context, Result};
use std::io::Cursor;

pub struct VoicePlayer {
    sink: rodio::Sink,
    /// Must be held alive for the duration of playback; dropping it cuts the
    /// audio output stream. Named with underscore prefix per convention.
    _stream: rodio::OutputStream,
}

impl VoicePlayer {
    /// Open the default audio output device and create a paused sink.
    pub fn new() -> Result<Self> {
        let (stream, stream_handle) = rodio::OutputStream::try_default()
            .context("failed to open audio output device")?;
        let sink = rodio::Sink::try_new(&stream_handle)
            .context("failed to create rodio sink")?;
        Ok(Self {
            sink,
            _stream: stream,
        })
    }

    /// Decode `bytes` as mp3 and append to the playback queue.
    /// Returns immediately; playback is non-blocking.
    pub fn play_mp3(&self, bytes: Vec<u8>) -> Result<()> {
        let cursor = Cursor::new(bytes);
        let decoder = rodio::Decoder::new(cursor)
            .context("mp3 decode failed — bytes may not be valid mp3")?;
        self.sink.append(decoder);
        Ok(())
    }

    /// `true` while audio is still playing through the sink.
    pub fn is_speaking(&self) -> bool {
        !self.sink.empty()
    }

    /// Stop playback immediately (Esc interrupt).
    pub fn stop(&self) {
        self.sink.stop();
    }
}
