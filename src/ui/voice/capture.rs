//! Microphone capture via cpal — push-to-talk input for the voice channel.
//!
//! `MicCapture` opens the default input device at 16 kHz mono i16 (Whisper's
//! native rate). The audio callback writes samples into a shared buffer and
//! updates a peak-level atomic for the waveform meter widget to read
//! lock-free.
//!
//! Usage:
//! ```no_run
//! let cap = MicCapture::start(16000)?;
//! // ... user holds Space ...
//! let samples = cap.stop_and_take();
//! // encode to WAV with hound, POST to STT
//! ```
//!
//! Reference: `~/Projects/hyprwhspr/lib/mic_osd/audio.py` — the same
//! pattern (shared state, callback writes, owner reads) in Python.

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc, Mutex,
};

pub struct MicCapture {
    stream: cpal::Stream,
    buffer: Arc<Mutex<Vec<i16>>>,
    /// Peak as fixed-point 0..65535. Lock-free read for the meter widget.
    level: Arc<AtomicU32>,
}

impl MicCapture {
    /// Open the default input device and start capturing.
    ///
    /// Requests 16 kHz mono i16. If the device can't deliver exactly that,
    /// this fails loudly — resampling is out of scope per the task spec.
    pub fn start(sample_rate: u32) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no default input device found")?;

        tracing::info!(device = %device.name().unwrap_or_default(), "opening mic for PTT capture");

        // Build the desired config: 16 kHz, mono, i16.
        let desired = cpal::StreamConfig {
            channels: 1,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let buffer: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
        let level = Arc::new(AtomicU32::new(0));

        let buf_cb = Arc::clone(&buffer);
        let level_cb = Arc::clone(&level);

        // Error handler: log, don't panic.
        let err_fn = |e: cpal::StreamError| {
            tracing::warn!(error = %e, "mic stream error");
        };

        let stream = device
            .build_input_stream(
                &desired,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    // Peak: abs-max of this block, stored as 0..65535.
                    let peak = data
                        .iter()
                        .map(|s| s.unsigned_abs())
                        .max()
                        .unwrap_or(0);
                    level_cb.store(peak as u32, Ordering::Relaxed);

                    if let Ok(mut buf) = buf_cb.lock() {
                        buf.extend_from_slice(data);
                    }
                },
                err_fn,
                None, // no timeout
            )
            .context("failed to build mic input stream")?;

        stream.play().context("failed to start mic stream")?;

        Ok(Self {
            stream,
            buffer,
            level,
        })
    }

    /// Current peak level as 0.0..1.0 for the waveform meter widget.
    /// Lock-free — safe to call every frame.
    pub fn current_level(&self) -> f32 {
        let raw = self.level.load(Ordering::Relaxed);
        (raw as f32) / (u16::MAX as f32)
    }

    /// Stop capturing and return all accumulated i16 samples.
    /// The caller encodes these to WAV via hound and sends to STT.
    pub fn stop_and_take(self) -> Vec<i16> {
        // Drop the stream to stop capturing. The stream must be dropped
        // before we take the buffer to avoid a race with the callback.
        drop(self.stream);

        // Try to take the buffer. If the mutex is somehow poisoned, return
        // whatever we have rather than panicking.
        match Arc::try_unwrap(self.buffer) {
            Ok(mutex) => mutex.into_inner().unwrap_or_default(),
            Err(arc) => arc.lock().map(|g| g.clone()).unwrap_or_default(),
        }
    }
}

/// Encode a slice of i16 mono PCM samples to WAV bytes using `hound`.
/// Sample rate should match what was requested in `MicCapture::start`.
pub fn samples_to_wav(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut buf = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut writer = hound::WavWriter::new(cursor, spec)
            .context("WAV writer init failed")?;
        for &s in samples {
            writer.write_sample(s).context("WAV sample write failed")?;
        }
        writer.finalize().context("WAV finalize failed")?;
    }
    Ok(buf)
}
