//! Bootstrap — declarative startup pipeline.
//!
//! Composes three patterns from reference projects:
//!
//! 1. **Claw-open's `BootstrapPlan`** — ordered phases, each self-contained,
//!    composable, independently testable.
//! 2. **Letta-code's pure-function resolver** — zero-I/O decision tree that
//!    maps a `BootstrapProbe` → `Resolution`. No side effects, no async,
//!    fully testable by feeding probe fixtures.
//! 3. **J code's progressive hints** — non-blocking advisory nudges that
//!    escalate with launch count. The wizard is the heavy option; hints are
//!    the light touch.
//!
//! ## Startup pipeline
//!
//! ```text
//! Phase 0: Splash (bloom)         ← always, non-blocking
//! Phase 1: Probe                   ← gather disk state → BootstrapProbe
//! Phase 2: Resolve                 ← pure fn: probe → Resolution
//!   ├── Ready { agent }            → skip phases 3-4, go to 5
//!   ├── NeedsSetup { flow }        → phase 3 (wizard)
//!   └── NeedsHint { hint }         → phase 4 (nudge)
//! Phase 3: Setup Wizard            ← blocking, first-run only
//! Phase 4: Hint Display            ← non-blocking, advisory
//! Phase 5: Background Tasks        ← model fetch, health, git sync
//! Phase 6: Enter TUI               ← dashboard / presence / chat
//! ```

use std::path::Path;

use crate::ui::setup::SetupFlow;

// ── Probe — data snapshot (gathered once, no I/O in resolve) ────────

/// Snapshot of machine/install state at startup. Gathered by probing the
/// filesystem and environment once, then fed to `resolve()` as input.
/// No I/O inside `resolve()` — it's a pure function over this struct.
#[derive(Debug, Clone, Default)]
pub struct BootstrapProbe {
    /// Whether a souveraine config file exists (any format/location).
    pub has_config: bool,
    /// Number of active agents on disk (memory/ dirs under ~/.souveraine/agents/).
    pub agent_count: u32,
    /// How many times the TUI has been launched (read from ~/.souveraine/.launch_count).
    pub launch_count: u32,
    /// Whether the user explicitly set SOUVERAINE_SETUP=federation.
    pub force_federation: bool,
}

// ── Resolution — pure decision output ───────────────────────────────

/// What the resolver decided. No I/O inside the resolver — all data
/// comes from `BootstrapProbe`.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// Everything is already set up. Go directly to the TUI dashboard,
    /// optionally showing advisory hints.
    Ready {
        /// Non-blocking hint to show (empty string = no hint).
        hint: String,
    },
    /// User needs a blocking setup wizard.
    NeedsSetup {
        /// Which wizard flow to run.
        flow: SetupFlow,
    },
}

/// Non-blocking advisory hint shown on the Welcome screen.
/// These escalate with `launch_count` and are never required.
pub fn hint_for_launch(count: u32, agent_count: u32) -> String {
    match (count, agent_count) {
        // First ever launch with agents: welcome hint
        (1, _) if agent_count > 0 => {
            "Welcome to Souveraine. Press Enter to start chatting, or 'i' to browse your agents."
                .to_string()
        }
        // First few launches: brief orientation
        (1..=3, _) if agent_count == 0 => {
            "No agents yet. Press 'a' to create one, or check the Settings screen."
                .to_string()
        }
        // Seasoned user: no hint
        _ => String::new(),
    }
}

// ── Pure resolver ───────────────────────────────────────────────────

/// Pure decision function: given a probe, return a `Resolution`.
///
/// No I/O, no async, no side effects. Feed it a `BootstrapProbe` and get
/// a deterministic result. Test by constructing probes and asserting
/// the expected resolution.
///
/// This is the **brain** — the `BootstrapPlan` skeleton calls it once
/// after the probe phase and dispatches accordingly.
pub fn resolve(probe: &BootstrapProbe) -> Resolution {
    if probe.force_federation {
        return Resolution::Ready {
            hint: String::new(),
        };
    }

    match (probe.agent_count, probe.has_config) {
        // No agents AND no config → new user, full wizard
        (0, false) => Resolution::NeedsSetup {
            flow: SetupFlow::FreshInstall,
        },
        // No agents (but has config) → just needs an agent
        (0, true) => Resolution::NeedsSetup {
            flow: SetupFlow::ImportAgent,
        },
        // Has agents — ready to go, maybe show a hint
        (_, _) => Resolution::Ready {
            hint: hint_for_launch(probe.launch_count, probe.agent_count),
        },
    }
}

// ── BootstrapPlan — ordered phases ──────────────────────────────────

/// One phase in the startup pipeline. Each is self-contained and can
/// succeed, skip, or fail independently.
#[derive(Debug, Clone, PartialEq)]
pub enum BootstrapPhase {
    /// The bloom animation. Always plays. Non-blocking.
    Splash,
    /// Gather disk state into a `BootstrapProbe`. Runs once at startup.
    Probe,
    /// Pure-function resolution from probe to decision.
    Resolve,
    /// Blocking setup wizard (first-run only).
    SetupWizard(SetupFlow),
    /// Non-blocking advisory hint on the Welcome screen.
    ShowHint(String),
    /// Background tasks that fire after the critical path (model fetch,
    /// git sync, health check).
    BackgroundTasks,
    /// Enter the main TUI (Welcome / Presence / Chat).
    EnterTui,
}

impl BootstrapPhase {
    pub fn label(&self) -> &'static str {
        match self {
            BootstrapPhase::Splash => "splash",
            BootstrapPhase::Probe => "probe",
            BootstrapPhase::Resolve => "resolve",
            BootstrapPhase::SetupWizard(_) => "setup-wizard",
            BootstrapPhase::ShowHint(_) => "show-hint",
            BootstrapPhase::BackgroundTasks => "background-tasks",
            BootstrapPhase::EnterTui => "enter-tui",
        }
    }
}

/// The startup pipeline: an ordered list of phases.
///
/// Constructed once per session by `plan()` which calls `resolve()` to
/// insert the right phases between Probe and EnterTui.
pub struct BootstrapPlan {
    /// Ordered list of phases to execute.
    pub phases: Vec<BootstrapPhase>,
}

impl BootstrapPlan {
    /// Build a plan from a probe. The probe is gathered once, then
    /// `resolve()` decides which phases to insert.
    pub fn plan(probe: &BootstrapProbe) -> Self {
        let mut phases = Vec::new();

        // Phase 0: always splash
        phases.push(BootstrapPhase::Splash);
        // Phase 1: always probe (already done, this is a marker)
        phases.push(BootstrapPhase::Probe);
        // Phase 2: always resolve
        phases.push(BootstrapPhase::Resolve);

        // Phase 3-6: determined by resolution
        let resolution = resolve(probe);
        match resolution {
            Resolution::Ready { hint } => {
                if !hint.is_empty() {
                    phases.push(BootstrapPhase::ShowHint(hint));
                }
            }
            Resolution::NeedsSetup { flow } => {
                phases.push(BootstrapPhase::SetupWizard(flow));
            }
        }

        // Phase 6: always enter TUI
        phases.push(BootstrapPhase::BackgroundTasks);
        phases.push(BootstrapPhase::EnterTui);

        Self { phases }
    }
}

// ── Probe gathering (the one place I/O lives) ───────────────────────

/// Gather a `BootstrapProbe` from disk. This is the **only** place I/O
/// happens in the bootstrap pipeline — everything downstream of `resolve()`
/// is pure.
pub fn gather_probe(home: &Path) -> BootstrapProbe {
    let souveraine_dir = home.join(".souveraine");

    // Config check
    let has_config = crate::core::config::ConsciousnessConfig::discover_path().is_some();

    // Agent count
    let agent_count = count_agents(&souveraine_dir.join("agents"));

    // Launch count
    let launch_count = read_launch_count(&souveraine_dir);

    // Federation env override
    let force_federation =
        std::env::var("SOUVERAINE_SETUP").as_deref() == Ok("federation");

    // Increment launch count for next time
    let _ = increment_launch_count(&souveraine_dir);

    BootstrapProbe {
        has_config,
        agent_count,
        launch_count,
        force_federation,
    }
}

fn count_agents(agents_dir: &Path) -> u32 {
    if !agents_dir.is_dir() {
        return 0;
    }
    let mut count = 0u32;
    if let Ok(entries) = std::fs::read_dir(agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            if name == "system" || name == "schedules" {
                continue;
            }
            if path.join("memory").is_dir() {
                count += 1;
            }
        }
    }
    count
}

fn launch_count_path(souveraine_dir: &Path) -> std::path::PathBuf {
    souveraine_dir.join(".launch_count")
}

fn read_launch_count(souveraine_dir: &Path) -> u32 {
    let path = launch_count_path(souveraine_dir);
    if !path.exists() {
        return 0;
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

fn increment_launch_count(souveraine_dir: &Path) -> std::io::Result<()> {
    let current = read_launch_count(souveraine_dir);
    std::fs::write(launch_count_path(souveraine_dir), (current + 1).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_install_no_config_no_agents() {
        let probe = BootstrapProbe {
            has_config: false,
            agent_count: 0,
            ..Default::default()
        };
        assert_eq!(
            resolve(&probe),
            Resolution::NeedsSetup {
                flow: SetupFlow::FreshInstall
            }
        );
    }

    #[test]
    fn test_has_config_no_agents() {
        let probe = BootstrapProbe {
            has_config: true,
            agent_count: 0,
            ..Default::default()
        };
        assert_eq!(
            resolve(&probe),
            Resolution::NeedsSetup {
                flow: SetupFlow::ImportAgent
            }
        );
    }

    #[test]
    fn test_has_config_and_agents_ready() {
        let probe = BootstrapProbe {
            has_config: true,
            agent_count: 1,
            launch_count: 5,
            ..Default::default()
        };
        assert_eq!(
            resolve(&probe),
            Resolution::Ready {
                hint: String::new()
            }
        );
    }

    #[test]
    fn test_gives_hint_on_first_launch_with_agents() {
        let probe = BootstrapProbe {
            has_config: true,
            agent_count: 1,
            launch_count: 1,
            ..Default::default()
        };
        let resolution = resolve(&probe);
        match resolution {
            Resolution::Ready { hint } => {
                assert!(!hint.is_empty());
                assert!(hint.contains("Welcome"));
            }
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_gives_hint_on_first_launch_no_agents() {
        let probe = BootstrapProbe {
            has_config: true,
            agent_count: 0,
            launch_count: 1,
            ..Default::default()
        };
        let resolution = resolve(&probe);
        match resolution {
            Resolution::NeedsSetup { .. } => {} // wizard handles it
            _ => panic!("expected NeedsSetup"),
        }
    }

    #[test]
    fn test_hint_on_early_launches_no_agents() {
        let hint = hint_for_launch(2, 0);
        assert!(!hint.is_empty());
        assert!(hint.contains("No agents"));
    }

    #[test]
    fn test_no_hint_for_seasoned_user() {
        let hint = hint_for_launch(10, 3);
        assert!(hint.is_empty());
    }

    #[test]
    fn test_force_federation_returns_ready() {
        let probe = BootstrapProbe {
            has_config: false,
            agent_count: 0,
            force_federation: true,
            ..Default::default()
        };
        assert_eq!(
            resolve(&probe),
            Resolution::Ready {
                hint: String::new()
            }
        );
    }

    #[test]
    fn test_plan_contains_correct_phases() {
        let probe = BootstrapProbe {
            has_config: false,
            agent_count: 0,
            ..Default::default()
        };
        let plan = BootstrapPlan::plan(&probe);
        let labels: Vec<&str> = plan.phases.iter().map(|p| p.label()).collect();
        assert_eq!(
            labels,
            vec!["splash", "probe", "resolve", "setup-wizard", "background-tasks", "enter-tui"]
        );
    }

    #[test]
    fn test_plan_for_ready_user() {
        let probe = BootstrapProbe {
            has_config: true,
            agent_count: 2,
            launch_count: 10,
            ..Default::default()
        };
        let plan = BootstrapPlan::plan(&probe);
        let labels: Vec<&str> = plan.phases.iter().map(|p| p.label()).collect();
        // No setup-wizard, no hint (seasoned user)
        assert_eq!(
            labels,
            vec!["splash", "probe", "resolve", "background-tasks", "enter-tui"]
        );
    }
}
