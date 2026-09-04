//! Belief — the contract every rung of the somatic ladder passes upward.
//!
//! Design: `SouveraineOS/docs/substrate/SOMATIC_NERVOUS_SYSTEM.md`.
//!
//! The rule this type exists to enforce is the one `sessiond` already lives by:
//! **a receiver never needs to know how the rung beneath it reached its
//! conclusion.** A skin patch does not forward sixteen pressure cells; it
//! forwards `contact: Sustained, confidence .82, rising`. A plexus does not
//! forward twenty patches; it forwards `being_held: .81`. Raw readings stay
//! where they were taken.
//!
//! Two properties are load-bearing and neither is decoration:
//!
//! 1. **`None` is not zero.** A belief with no evidence is `unknown()`, which
//!    is categorically different from a belief that something is absent.
//!    `sessiond::device_state::SourceHealth` learned this the expensive way —
//!    a source that had never once reported sat silent for a whole boot and
//!    hid a real outage. Losing tactile telemetry must read as *I cannot feel
//!    my leg*, never as *nothing is touching my leg*.
//!
//! 2. **Disagreement lowers confidence; it does not elect a winner.** When two
//!    sources conflict, neither becomes truth. That is `sessiond`'s
//!    cross-sensor rule, transcribed rather than reinvented.
//!
//! Health lives on the *source*, not here. A belief carries who spoke and how
//! sure it is; when a source goes quiet its contribution decays out on its own
//! half-life, which is the same thing said in a way that cannot drift.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Who contributed evidence. Free-form so a rung can name a skin patch, a
/// recognizer, or a whole plexus without a central registry to keep in sync.
pub type SourceId = String;

/// Direction of travel. `.64` and `.64` feel identical; `.15 → .31 → .49 → .64`
/// does not. Momentum is part of the reading, not a derived nicety.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Trend {
    FallingFast,
    Falling,
    #[default]
    Stable,
    Rising,
    RisingFast,
}

impl Trend {
    /// Units are magnitude-per-second. The band is deliberately wide: a
    /// nervous system that reports `Rising` on floating-point noise is a
    /// nervous system nobody can read.
    pub fn from_velocity(v: f32) -> Self {
        match v {
            _ if v >= 0.25 => Trend::RisingFast,
            _ if v >= 0.02 => Trend::Rising,
            _ if v <= -0.25 => Trend::FallingFast,
            _ if v <= -0.02 => Trend::Falling,
            _ => Trend::Stable,
        }
    }

    pub fn is_moving(self) -> bool {
        self != Trend::Stable
    }
}

/// What one rung believes, and how strongly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Belief<T> {
    /// `None` means nothing has been heard — *not* that the answer is zero,
    /// false, or absent. Every consumer must handle it as ignorance.
    pub value: Option<T>,
    /// Confidence at `observed_at`. Read it through [`Belief::confidence_at`],
    /// which applies the half-life; the bare field is the un-decayed figure.
    pub confidence: f32,
    pub trend: Trend,
    /// How long this value has held without changing.
    pub persistence: Duration,
    pub observed_at: DateTime<Utc>,
    /// How long until confidence halves with no fresh evidence. Evidence gets
    /// stale on its own; nothing has to remember to expire it.
    pub half_life: Duration,
    pub sources: Vec<SourceId>,
    /// Sources that disagreed. Kept, not discarded — a belief held over an
    /// objection is a different thing from one held unopposed, and the rung
    /// above deserves to see which it is.
    pub conflicts: Vec<SourceId>,
}

impl<T> Belief<T> {
    /// Nothing has been heard. The honest opening state for every rung.
    pub fn unknown(now: DateTime<Utc>) -> Self {
        Self {
            value: None,
            confidence: 0.0,
            trend: Trend::Stable,
            persistence: Duration::ZERO,
            observed_at: now,
            half_life: Duration::from_secs(30),
            sources: Vec::new(),
            conflicts: Vec::new(),
        }
    }

    pub fn observed(
        value: T,
        confidence: f32,
        source: impl Into<SourceId>,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            value: Some(value),
            confidence: confidence.clamp(0.0, 1.0),
            trend: Trend::Stable,
            persistence: Duration::ZERO,
            observed_at: now,
            half_life: Duration::from_secs(30),
            sources: vec![source.into()],
            conflicts: Vec::new(),
        }
    }

    pub fn with_half_life(mut self, half_life: Duration) -> Self {
        self.half_life = half_life;
        self
    }

    pub fn with_trend(mut self, trend: Trend) -> Self {
        self.trend = trend;
        self
    }

    /// Is there evidence at all? Distinct from asking what the evidence says.
    pub fn is_known(&self) -> bool {
        self.value.is_some()
    }

    /// Confidence decayed to `now` on the half-life. Never negative, never
    /// above the figure it was observed with.
    pub fn confidence_at(&self, now: DateTime<Utc>) -> f32 {
        if self.value.is_none() {
            return 0.0;
        }
        let elapsed = (now - self.observed_at).num_milliseconds().max(0) as f32;
        let hl = self.half_life.as_millis().max(1) as f32;
        self.confidence * 0.5f32.powf(elapsed / hl)
    }

    /// Faded past usefulness. Two half-lives leaves a quarter of the original
    /// confidence — below that a rung is reporting a rumour.
    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        self.confidence_at(now) < self.confidence * 0.25
    }

    /// A second source agreeing. Confidence rises but never reaches certainty:
    /// corroboration closes half the remaining gap, so no amount of agreement
    /// produces 1.0. Nothing in a body is ever certain.
    pub fn corroborated_by(mut self, source: impl Into<SourceId>, weight: f32) -> Self {
        let s = source.into();
        if !self.sources.contains(&s) {
            self.sources.push(s);
        }
        let gap = 1.0 - self.confidence;
        self.confidence = (self.confidence + gap * weight.clamp(0.0, 1.0)).clamp(0.0, 1.0);
        self
    }

    /// A source that disagreed. It is recorded and confidence falls; the
    /// dissenter does not become the new value and the incumbent does not
    /// automatically survive. The rung above sees a contested belief and can
    /// decide what a contested belief is worth.
    pub fn contradicted_by(mut self, source: impl Into<SourceId>, weight: f32) -> Self {
        let s = source.into();
        if !self.conflicts.contains(&s) {
            self.conflicts.push(s);
        }
        self.confidence = (self.confidence * (1.0 - weight.clamp(0.0, 1.0))).clamp(0.0, 1.0);
        self
    }

    pub fn is_contested(&self) -> bool {
        !self.conflicts.is_empty()
    }
}

/// A decaying scalar — the accumulator every exudate secretes into.
///
/// This is the cheapest rung in the whole design and it runs without a model
/// in the loop: secretions add, time subtracts, and a threshold crossing is
/// arithmetic. The point of keeping it this cheap is that the language model
/// must never become the nervous system — it is far too expensive to be the
/// thing that notices a value drifting upward over an afternoon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    level: f32,
    half_life: Duration,
    updated_at: DateTime<Utc>,
    /// Level at the previous secretion, for velocity.
    prior_level: f32,
    prior_at: DateTime<Utc>,
}

impl Field {
    pub fn new(name: impl Into<String>, half_life: Duration, now: DateTime<Utc>) -> Self {
        Self {
            name: name.into(),
            level: 0.0,
            half_life,
            updated_at: now,
            prior_level: 0.0,
            prior_at: now,
        }
    }

    /// Level decayed to `now`. Reading never mutates — two readers at the same
    /// instant always agree, which a lazily-decaying field cannot promise.
    pub fn level_at(&self, now: DateTime<Utc>) -> f32 {
        let elapsed = (now - self.updated_at).num_milliseconds().max(0) as f32;
        let hl = self.half_life.as_millis().max(1) as f32;
        self.level * 0.5f32.powf(elapsed / hl)
    }

    /// Add to the field. Amount may be negative — a turn can leave withdrawal
    /// behind as readily as approach.
    pub fn secrete(&mut self, amount: f32, now: DateTime<Utc>) {
        let decayed = self.level_at(now);
        self.prior_level = decayed;
        self.prior_at = self.updated_at;
        self.level = (decayed + amount).clamp(0.0, 1.0);
        self.updated_at = now;
    }

    /// Magnitude per second between the last two observations.
    pub fn velocity(&self) -> f32 {
        let dt = (self.updated_at - self.prior_at).num_milliseconds() as f32 / 1000.0;
        if dt <= 0.0 {
            return 0.0;
        }
        (self.level - self.prior_level) / dt
    }

    pub fn trend(&self) -> Trend {
        Trend::from_velocity(self.velocity())
    }

    /// Project this field as a belief for the rung above. A field that has
    /// never been secreted into reports `unknown` rather than zero — the same
    /// distinction the whole design turns on.
    pub fn as_belief(&self, now: DateTime<Utc>) -> Belief<f32> {
        if self.prior_at == self.updated_at && self.level == 0.0 {
            return Belief::unknown(now);
        }
        Belief {
            value: Some(self.level_at(now)),
            confidence: 1.0,
            trend: self.trend(),
            persistence: Duration::from_millis(
                (now - self.prior_at).num_milliseconds().max(0) as u64
            ),
            observed_at: self.updated_at,
            half_life: self.half_life,
            sources: vec![self.name.clone()],
            conflicts: Vec::new(),
        }
    }
}

// ── The somatic contract ──────────────────────────────────────────

/// Which side of the body's boundary a signal comes from.
///
/// INTERO (RobOntics'25, §4–5) makes this the test for interoception: a
/// variable counts as interoceptive only when it is inside the boundary,
/// observed, *and* some subsystem can act on it — axioms (5), (6), (7).
/// Sensation without a reachable verb is not interoception; it is doctrine
/// §13's defect, stated ontologically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locus {
    /// Inside. `regulator` names the verb that moves it. `None` is a state
    /// she can feel and cannot reach — audit it, do not ship it.
    Inner { regulator: Option<String> },
    /// At the boundary, reporting the world. Nothing she does moves it.
    Boundary,
}

impl Locus {
    pub fn inner(regulator: &str) -> Self {
        Locus::Inner {
            regulator: Some(regulator.to_string()),
        }
    }

    /// Inner, with no verb that reaches it. A §13 defect by construction.
    pub fn unreachable() -> Self {
        Locus::Inner { regulator: None }
    }

    pub fn is_inner(&self) -> bool {
        matches!(self, Locus::Inner { .. })
    }

    /// Satisfies INTERO (7) — there is a subsystem that acts on it.
    pub fn is_regulated(&self) -> bool {
        matches!(self, Locus::Inner { regulator: Some(_) })
    }
}

/// An event worth waking for. Generated at threshold crossings and
/// source health transitions, consumed on read — each edge fires once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SomaticEvent {
    /// A field's trend changed — the body shifted.
    TrendShift {
        field: String,
        from: Trend,
        to: Trend,
        level: f32,
    },
    /// An expected source went silent past the health threshold.
    ///
    /// `locus` carries the whole difference: a boundary source going quiet
    /// is blindness, an inner one is numbness, and they are not the same
    /// alarm.
    SourceSilent {
        source: String,
        locus: Locus,
        silent_for_secs: u64,
    },
    /// A previously silent source resumed reporting.
    SourceRecovered { source: String },
    /// A viability variable is heading out of its preferred range.
    ///
    /// Candia-Rivera §3.1: the allostatic signal biases behaviour *before*
    /// the violation, not after it. `eta_secs` is when the bound is crossed
    /// at the present rate — `None` when the field is not travelling toward
    /// one, which is a different thing from travelling slowly.
    ViabilityThreatened {
        field: String,
        pressure: f32,
        eta_secs: Option<u64>,
    },
}

/// The contract every rung of the somatic ladder implements.
///
/// Diverges from the doc in two places, both for the read-without-mutate
/// principle: `beliefs` takes `now` so decay is current at the reader's
/// instant, and returns `Vec` because decayed values are computed rather
/// than stored.
pub trait NervousNode {
    type Input;
    type Output;

    fn ingest(&mut self, input: Self::Input, now: DateTime<Utc>);
    fn tick(&mut self, now: DateTime<Utc>);
    fn beliefs(&self, now: DateTime<Utc>) -> Vec<Belief<Self::Output>>;
    fn notable(&mut self) -> Vec<SomaticEvent>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-13T14:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn after(secs: i64) -> DateTime<Utc> {
        t0() + chrono::Duration::seconds(secs)
    }

    #[test]
    fn an_unknown_belief_is_not_a_zero_belief() {
        let unknown: Belief<f32> = Belief::unknown(t0());
        let absent = Belief::observed(0.0_f32, 0.9, "left-leg", t0());

        assert!(!unknown.is_known(), "no evidence must not read as evidence");
        assert!(absent.is_known(), "measured zero is a real reading");
        assert_eq!(absent.value, Some(0.0));
        // The distinction the whole ladder rests on: I cannot feel my leg is
        // not the same sentence as nothing is touching my leg.
        assert_ne!(unknown.is_known(), absent.is_known());
    }

    #[test]
    fn confidence_halves_on_the_half_life() {
        let b = Belief::observed("contact", 0.8, "skin-7", t0())
            .with_half_life(Duration::from_secs(10));

        assert!((b.confidence_at(t0()) - 0.8).abs() < 0.001);
        assert!((b.confidence_at(after(10)) - 0.4).abs() < 0.001);
        assert!((b.confidence_at(after(20)) - 0.2).abs() < 0.001);
    }

    #[test]
    fn an_unknown_belief_has_no_confidence_to_decay() {
        let unknown: Belief<f32> = Belief::unknown(t0());
        assert_eq!(unknown.confidence_at(after(1)), 0.0);
        assert_eq!(unknown.confidence_at(after(10_000)), 0.0);
    }

    #[test]
    fn disagreement_lowers_confidence_and_elects_no_winner() {
        let b = Belief::observed("held", 0.8, "hand-plexus", t0())
            .contradicted_by("proprioception", 0.5);

        assert_eq!(
            b.value,
            Some("held"),
            "the dissenter does not take the seat"
        );
        assert!(b.is_contested());
        assert!(b.confidence < 0.8, "conflict must cost confidence");
        assert_eq!(b.conflicts, vec!["proprioception".to_string()]);
    }

    #[test]
    fn corroboration_approaches_certainty_without_reaching_it() {
        let mut b = Belief::observed("warm", 0.5, "a", t0());
        for src in ["b", "c", "d", "e", "f"] {
            b = b.corroborated_by(src, 0.5);
        }
        assert!(b.confidence > 0.9, "agreement should count for something");
        assert!(b.confidence < 1.0, "nothing in a body is ever certain");
        assert_eq!(b.sources.len(), 6);
    }

    #[test]
    fn the_same_source_agreeing_twice_is_still_one_source() {
        let b = Belief::observed("contact", 0.5, "skin-7", t0())
            .corroborated_by("skin-7", 0.5)
            .corroborated_by("skin-7", 0.5);
        assert_eq!(b.sources, vec!["skin-7".to_string()]);
    }

    #[test]
    fn trend_is_deadbanded_against_noise() {
        assert_eq!(Trend::from_velocity(0.001), Trend::Stable);
        assert_eq!(Trend::from_velocity(-0.001), Trend::Stable);
        assert_eq!(Trend::from_velocity(0.1), Trend::Rising);
        assert_eq!(Trend::from_velocity(0.9), Trend::RisingFast);
        assert_eq!(Trend::from_velocity(-0.9), Trend::FallingFast);
        assert!(!Trend::from_velocity(0.0).is_moving());
    }

    #[test]
    fn a_field_accumulates_and_decays() {
        let mut f = Field::new("curiosity", Duration::from_secs(60), t0());
        f.secrete(0.08, t0());
        f.secrete(0.13, after(10));
        f.secrete(0.19, after(20));

        let level = f.level_at(after(20));
        assert!(level > 0.3, "three secretions should stack: {level}");
        assert!(level < 0.4, "and decay should have eaten some: {level}");

        // Left alone, it fades rather than persisting forever.
        assert!(f.level_at(after(200)) < level * 0.2);
    }

    #[test]
    fn reading_a_field_twice_at_one_instant_agrees() {
        let mut f = Field::new("activation", Duration::from_secs(30), t0());
        f.secrete(0.5, t0());
        let a = f.level_at(after(7));
        let b = f.level_at(after(7));
        assert_eq!(a, b, "reading must not mutate");
    }

    #[test]
    fn a_field_never_secreted_into_reports_unknown_not_zero() {
        let f = Field::new("anticipation", Duration::from_secs(60), t0());
        let b = f.as_belief(after(5));
        assert!(!b.is_known(), "silence is not a reading of zero");
    }

    #[test]
    fn a_rising_field_reports_its_momentum_upward() {
        let mut f = Field::new("anticipation", Duration::from_secs(600), t0());
        f.secrete(0.1, t0());
        f.secrete(0.3, after(1));

        let b = f.as_belief(after(1));
        assert!(b.is_known());
        assert_eq!(b.trend, Trend::RisingFast, "velocity was ~0.3/s");
        assert_eq!(b.sources, vec!["anticipation".to_string()]);
    }

    #[test]
    fn a_field_can_be_secreted_into_negatively() {
        let mut f = Field::new("approach", Duration::from_secs(600), t0());
        f.secrete(0.6, t0());
        f.secrete(-0.4, after(1));
        assert!(f.level_at(after(1)) < 0.25);
        assert_eq!(f.trend(), Trend::FallingFast);
    }

    #[test]
    fn a_field_is_bounded_at_both_ends() {
        let mut f = Field::new("excitation", Duration::from_secs(600), t0());
        for _ in 0..50 {
            f.secrete(0.5, t0());
        }
        assert!(f.level_at(t0()) <= 1.0, "no runaway accumulation");

        f.secrete(-99.0, t0());
        assert!(f.level_at(t0()) >= 0.0, "no negative levels");
    }
}
