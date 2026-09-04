//! Intentional restarts carry their reason across the reboot. The API writes
//! a marker before asking systemd for a restart; boot reads it, announces
//! `resumed` on the event bus (firehose + federation peers), and clears it.
//! A boot with no marker reads as a cold `started` and stays quiet about why.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MARKER_FILE: &str = "restart.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartMarker {
    pub reason: String,
    pub at: DateTime<Utc>,
    pub by: String,
}

pub fn marker_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MARKER_FILE)
}

pub fn read(data_dir: &Path) -> Option<RestartMarker> {
    std::fs::read_to_string(marker_path(data_dir))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

pub fn write(data_dir: &Path, marker: &RestartMarker) -> anyhow::Result<()> {
    std::fs::write(marker_path(data_dir), serde_json::to_string_pretty(marker)?)?;
    Ok(())
}

pub fn clear(data_dir: &Path) {
    let _ = std::fs::remove_file(marker_path(data_dir));
}