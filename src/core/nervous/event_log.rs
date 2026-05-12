use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use tokio::sync::broadcast;
use tracing::{debug, warn};

use super::SensorEvent;

/// Persistent event log — the firehose written to disk.
///
/// Subscribes to the EventBus and appends every event as a JSONL line
/// to a date-partitioned file: `events-YYYY-MM-DD.jsonl`.
///
/// The morning pass reads this log to triage what happened while the
/// agent was absent. Federation reads it to know what to broadcast.
pub struct EventLog {
    events_dir: PathBuf,
    rx: broadcast::Receiver<SensorEvent>,
}

impl EventLog {
    pub fn new(events_dir: PathBuf, rx: broadcast::Receiver<SensorEvent>) -> Self {
        Self { events_dir, rx }
    }

    pub async fn run(&mut self) {
        if let Err(e) = std::fs::create_dir_all(&self.events_dir) {
            warn!(path = %self.events_dir.display(), %e, "cannot create events dir");
            return;
        }

        loop {
            match self.rx.recv().await {
                Ok(event) => {
                    if let Err(e) = self.append(&event) {
                        warn!(sensor = %event.sensor_name, %e, "failed to log event");
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(skipped = n, "event log lagged");
                }
                Err(broadcast::error::RecvError::Closed) => {
                    debug!("event bus closed, event log exiting");
                    break;
                }
            }
        }
    }

    fn append(&self, event: &SensorEvent) -> Result<()> {
        use std::io::Write;

        let date = event.timestamp.format("%Y-%m-%d");
        let path = self.events_dir.join(format!("events-{date}.jsonl"));
        let line = serde_json::to_string(event)?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(file, "{line}")?;
        Ok(())
    }
}

// ── Reading the log ─────────────────────────────────────────────

/// Load events from the log since a given timestamp.
/// Used by the morning pass to triage what accumulated overnight.
pub fn events_since(
    events_dir: &Path,
    since: DateTime<Utc>,
) -> Result<Vec<SensorEvent>> {
    let mut results = Vec::new();
    let since_date = since.date_naive();

    let mut dates_to_scan = Vec::new();
    let today = Utc::now().date_naive();
    let mut d = since_date;
    while d <= today {
        dates_to_scan.push(d);
        d += chrono::Duration::days(1);
    }

    for date in dates_to_scan {
        let path = events_dir.join(format!("events-{date}.jsonl"));
        if !path.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<SensorEvent>(line) {
                Ok(event) if event.timestamp >= since => {
                    results.push(event);
                }
                Ok(_) => {} // before our cutoff
                Err(e) => {
                    warn!(path = %path.display(), %e, "skipping malformed event line");
                }
            }
        }
    }

    Ok(results)
}

/// Load all events from a specific date.
pub fn events_for_date(
    events_dir: &Path,
    date: NaiveDate,
) -> Result<Vec<SensorEvent>> {
    let path = events_dir.join(format!("events-{date}.jsonl"));
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&path)?;
    let mut results = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<SensorEvent>(line) {
            Ok(event) => results.push(event),
            Err(e) => {
                warn!(path = %path.display(), %e, "skipping malformed event line");
            }
        }
    }
    Ok(results)
}

/// Purge event logs older than `retain_days`.
pub fn purge_old_events(events_dir: &Path, retain_days: i64) -> Result<u32> {
    let cutoff = Utc::now().date_naive() - chrono::Duration::days(retain_days);
    let mut removed = 0u32;

    if let Ok(entries) = std::fs::read_dir(events_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(date_str) = name_str
                .strip_prefix("events-")
                .and_then(|s| s.strip_suffix(".jsonl"))
            {
                if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                    if date < cutoff {
                        if std::fs::remove_file(entry.path()).is_ok() {
                            removed += 1;
                            debug!(file = %name_str, "purged old event log");
                        }
                    }
                }
            }
        }
    }

    Ok(removed)
}
