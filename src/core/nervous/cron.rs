use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Result;
use chrono::{DateTime, Utc};
use cron::Schedule as CronSchedule;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::{EventBus, SensorEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleEntry {
    pub name: String,
    pub kind: ScheduleKind,
    pub schedule: String,
    pub source: String,
    pub enabled: bool,
    pub urgency: f32,
    pub created_at: DateTime<Utc>,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKind {
    Once,
    Interval,
    Cron,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScheduleState {
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_fired: Option<DateTime<Utc>>,
    pub fire_count: u64,
    pub last_status: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CronState {
    pub entries: Vec<ScheduleEntry>,
    pub state: HashMap<String, ScheduleState>,
    #[serde(skip)]
    last_mtime: Option<SystemTime>,
}

impl CronState {
    pub fn next_due_time(&self) -> tokio::time::Instant {
        let now = Utc::now();
        let mut soonest: Option<DateTime<Utc>> = None;

        for entry in &self.entries {
            if !entry.enabled {
                continue;
            }
            if let Some(s) = self.state.get(&entry.name) {
                if let Some(next) = s.next_run_at {
                    if soonest.is_none() || next < soonest.unwrap() {
                        soonest = Some(next);
                    }
                }
            }
        }

        match soonest {
            Some(t) if t > now => {
                let dur = (t - now).to_std().unwrap_or(std::time::Duration::from_secs(60));
                tokio::time::Instant::now() + dur
            }
            Some(_) => tokio::time::Instant::now(),
            None => tokio::time::Instant::now() + std::time::Duration::from_secs(60),
        }
    }

    pub fn due_entries(&self) -> Vec<ScheduleEntry> {
        let now = Utc::now();
        self.entries
            .iter()
            .filter(|e| {
                e.enabled
                    && self
                        .state
                        .get(&e.name)
                        .and_then(|s| s.next_run_at)
                        .map(|t| t <= now)
                        .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    pub fn advance(&mut self, name: &str, entry: &ScheduleEntry) {
        let s = self.state.entry(name.to_string()).or_default();
        s.last_fired = Some(Utc::now());
        s.fire_count += 1;
        s.next_run_at = compute_next_run(entry);
    }

    pub fn persist(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.state)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_state(&mut self, path: &Path) -> Result<()> {
        if path.exists() {
            let data = std::fs::read_to_string(path)?;
            self.state = serde_json::from_str(&data)?;
        }
        Ok(())
    }
}

fn compute_next_run(entry: &ScheduleEntry) -> Option<DateTime<Utc>> {
    match entry.kind {
        ScheduleKind::Once => None,
        ScheduleKind::Interval => {
            let secs: u64 = entry.schedule.parse().unwrap_or(3600);
            Some(Utc::now() + chrono::Duration::seconds(secs as i64))
        }
        ScheduleKind::Cron => {
            if let Ok(sched) = entry.schedule.parse::<CronSchedule>() {
                sched.upcoming(Utc).next()
            } else {
                warn!(schedule = %entry.schedule, "invalid cron expression");
                None
            }
        }
    }
}

pub fn parse_schedule_file(path: &Path) -> Result<ScheduleEntry> {
    let content = std::fs::read_to_string(path)?;
    let (frontmatter, body) = split_frontmatter(&content);
    let mut entry: ScheduleEntry = serde_yaml::from_str(&frontmatter)?;
    entry.prompt = body.trim().to_string();
    Ok(entry)
}

fn split_frontmatter(content: &str) -> (String, String) {
    if content.starts_with("---") {
        if let Some(end) = content[3..].find("---") {
            let fm = content[3..3 + end].to_string();
            let body = content[3 + end + 3..].to_string();
            return (fm, body);
        }
    }
    (String::new(), content.to_string())
}

fn scan_schedule_files(dir: &Path) -> Vec<ScheduleEntry> {
    let mut entries = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                match parse_schedule_file(&path) {
                    Ok(e) => entries.push(e),
                    Err(err) => warn!(path = %path.display(), %err, "skipping schedule file"),
                }
            }
        }
    }
    entries
}

pub struct CronSensor {
    pub agent_id: String,
    pub schedules_dir: PathBuf,
    pub event_bus: EventBus,
    pub wake_notify: Arc<Notify>,
    pub cancel: CancellationToken,
    pub state: Arc<RwLock<CronState>>,
    pub active_sessions: Arc<AtomicU32>,
}

impl CronSensor {
    pub fn new(
        agent_id: String,
        schedules_dir: PathBuf,
        event_bus: EventBus,
        active_sessions: Arc<AtomicU32>,
    ) -> Self {
        Self {
            agent_id,
            schedules_dir,
            event_bus,
            wake_notify: Arc::new(Notify::new()),
            cancel: CancellationToken::new(),
            state: Arc::new(RwLock::new(CronState::default())),
            active_sessions,
        }
    }

    pub fn nudge(&self) {
        self.wake_notify.notify_one();
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub async fn run(&self) {
        let state_file = self.schedules_dir.join(".state.json");

        {
            let mut state = self.state.write().await;
            let _ = state.load_state(&state_file);
        }

        self.rescan().await;

        loop {
            let next_wake = self.state.read().await.next_due_time();

            tokio::select! {
                _ = self.cancel.cancelled() => {
                    debug!("CronSensor shutting down");
                    break;
                }
                _ = self.wake_notify.notified() => {
                    debug!("CronSensor nudged — rescanning");
                }
                _ = tokio::time::sleep_until(next_wake) => {}
            }

            self.rescan().await;

            if self.active_sessions.load(Ordering::Relaxed) > 0 {
                continue;
            }

            let due = self.state.read().await.due_entries();
            for entry in &due {
                {
                    let mut state = self.state.write().await;
                    state.advance(&entry.name, entry);
                    let _ = state.persist(&state_file);
                }

                self.event_bus.send(SensorEvent {
                    sensor_name: "cron".into(),
                    timestamp: Utc::now(),
                    event_type: "schedule_due".into(),
                    target: Some(entry.name.clone()),
                    urgency: entry.urgency,
                    payload: Some(serde_json::json!({
                        "kind": entry.kind,
                        "prompt": entry.prompt,
                        "source": entry.source,
                    })),
                    seed_id: None,
                });

                debug!(schedule = %entry.name, "fired schedule event");
            }
        }
    }

    async fn rescan(&self) {
        let dir_mtime = std::fs::metadata(&self.schedules_dir)
            .and_then(|m| m.modified())
            .ok();

        let mut state = self.state.write().await;
        if state.last_mtime == dir_mtime && dir_mtime.is_some() {
            return;
        }
        state.last_mtime = dir_mtime;

        let entries = scan_schedule_files(&self.schedules_dir);
        for entry in &entries {
            if !state.state.contains_key(&entry.name) {
                let next = compute_next_run(entry);
                state.state.insert(
                    entry.name.clone(),
                    ScheduleState {
                        next_run_at: next,
                        ..Default::default()
                    },
                );
            }
        }
        state.entries = entries;
    }
}
