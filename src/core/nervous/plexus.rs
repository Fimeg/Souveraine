//! The somatic plexus — regional integration of sensor fields.
//!
//! Fields accumulate and decay instead of flipping; each carries momentum.
//! Source health closes the gap the mapping session found — a nerve that
//! never spoke reads as absent, not calm.
//!
//! Two properties keep this from being telemetry, which is the charge
//! Candia-Rivera lays against every energy-aware robot: monitored variables
//! "rarely influence higher-level decision-making". So each field declares
//!
//! 1. its **locus** — inside the boundary or on it, and if inside, which
//!    verb reaches it (INTERO axiom 7). A field she can feel and cannot
//!    touch is doctrine §13's defect, and [`SomaticPlexus::unreachable`]
//!    lists them rather than letting them pass.
//! 2. its **viability** — the range it prefers, asymmetric, and the
//!    pressure it exerts as it travels toward a bound.
//!
//! Fields are named rather than struct'd, so a new afferent arrives as a
//! builder call. The phone currently offers: proximity, motion, light,
//! touch (iio); charge and thermal (power); `attended`, which sessiond's
//! clock derives from those; and the USB-C port — attached, data role, peer
//! display, peer input, peer network — which usb-signaller and smoo make
//! legible. Grip, per gauge rather than squeezed to a bool, is declared as
//! `grip_left` / `grip_right` and reads unknown until the device-side
//! reporter exists — the gap made visible, not a false calm.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::belief::{Belief, Field, Locus, NervousNode, SomaticEvent, Trend};

/// How far ahead the allostatic estimate looks.
///
/// **Provisional.** 60 s is a guess, not a measurement — the honest note
/// `BUTTON_HOLD` makes about AOSP's 500 ms applies here with less excuse,
/// since nothing upstream even suggests a number. Set it from a trail that
/// records real bound crossings before defending it.
pub const ALLOSTATIC_HORIZON: Duration = Duration::from_secs(60);

/// Pressure above which a field's drift is worth waking for.
///
/// **Provisional**, same caveat. Chosen so a field sitting still inside its
/// range never fires and one crossing within the horizon always does.
pub const ALLOSTATIC_THRESHOLD: f32 = 0.7;

/// A reading entering the plexus.
#[derive(Debug, Clone)]
pub struct SensorReading {
    pub source: String,
    /// Read as a secretion by an exudate field (positive adds, negative
    /// drains) and as the present reading by a gauge. Which one it is comes
    /// from how the afferent was declared, never from the reporter.
    pub amount: f32,
}

/// Whether a source is reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceHealth {
    #[default]
    Unknown,
    Live,
    /// Was Live, now silent past the threshold.
    Silent,
    /// Expected on this body and has never once spoken.
    Absent,
}

/// A field's preferred operating range and what leaving it costs.
///
/// Candia-Rivera §2.1. The two costs differ on purpose: for most bodily
/// variables the upper and lower tails do not mean the same thing. Charge
/// near empty threatens the organism; charge near full does not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Viability {
    pub low: f32,
    pub high: f32,
    pub below_cost: f32,
    pub above_cost: f32,
}

impl Viability {
    /// Bounded on both sides, penalised equally.
    pub fn symmetric(low: f32, high: f32) -> Self {
        Self {
            low,
            high,
            below_cost: 1.0,
            above_cost: 1.0,
        }
    }

    /// Only the floor is dangerous — charge, headroom, anything depletable.
    pub fn floor(low: f32) -> Self {
        Self {
            low,
            high: 1.0,
            below_cost: 1.0,
            above_cost: 0.0,
        }
    }

    /// Only the ceiling is dangerous — thermal, load, strain.
    pub fn ceiling(high: f32) -> Self {
        Self {
            low: 0.0,
            high,
            below_cost: 0.0,
            above_cost: 1.0,
        }
    }

    /// Weighted distance outside the range; zero within it.
    pub fn deviation(&self, level: f32) -> f32 {
        if level < self.low {
            (self.low - level) * self.below_cost
        } else if level > self.high {
            (level - self.high) * self.above_cost
        } else {
            0.0
        }
    }

    /// 0 at rest, 1 at a bound that costs something to cross.
    ///
    /// A bound whose cost is zero is not a bound to be near: a full battery
    /// is at the top of its range and in no danger, so only the costed side
    /// draws.
    pub fn boundary_proximity(&self, level: f32) -> f32 {
        let (low_matters, high_matters) = (self.below_cost > 0.0, self.above_cost > 0.0);
        if (low_matters && level <= self.low) || (high_matters && level >= self.high) {
            return 1.0;
        }
        let half_width = (self.high - self.low) / 2.0;
        if half_width <= 0.0 {
            return 1.0;
        }
        let mut nearest = f32::INFINITY;
        if low_matters {
            nearest = nearest.min(level - self.low);
        }
        if high_matters {
            nearest = nearest.min(self.high - level);
        }
        if !nearest.is_finite() {
            return 0.0; // nothing here is dangerous in either direction
        }
        (1.0 - nearest / half_width).clamp(0.0, 1.0)
    }

    /// Seconds until a costed bound at this rate, if travelling toward one.
    ///
    /// `None` covers both "moving away" and "not moving", which a caller
    /// must not conflate with "moving slowly" — the first two are safe and
    /// the third is only slow.
    pub fn eta_secs(&self, level: f32, velocity: f32) -> Option<f64> {
        let (low_matters, high_matters) = (self.below_cost > 0.0, self.above_cost > 0.0);
        if (low_matters && level <= self.low) || (high_matters && level >= self.high) {
            return Some(0.0);
        }
        let mut soonest: Option<f64> = None;
        if low_matters && velocity < 0.0 {
            soonest = Some(((level - self.low) / -velocity) as f64);
        }
        if high_matters && velocity > 0.0 {
            let t = ((self.high - level) / velocity) as f64;
            soonest = Some(soonest.map_or(t, |s: f64| s.min(t)));
        }
        soonest
    }
}

/// A measured level, as opposed to a secretion.
///
/// The distinction the ladder already draws between exudate and belief. A
/// battery at .42 is still at .42 when nobody looks, so the *value* must not
/// decay — only confidence in a reading nobody has refreshed. Putting a fuel
/// gauge into an accumulator would make the pack drain because the reporter
/// went quiet, which is the empty-result trap wearing a units label.
#[derive(Debug, Clone)]
struct Level {
    name: String,
    value: f32,
    observed_at: DateTime<Utc>,
    prior: Option<(f32, DateTime<Utc>)>,
    half_life: Duration,
}

impl Level {
    fn new(name: impl Into<String>, half_life: Duration) -> Self {
        Self {
            name: name.into(),
            value: 0.0,
            observed_at: DateTime::<Utc>::MIN_UTC,
            prior: None,
            half_life,
        }
    }

    fn observe(&mut self, value: f32, now: DateTime<Utc>) {
        if self.seen() {
            self.prior = Some((self.value, self.observed_at));
        }
        self.value = value.clamp(0.0, 1.0);
        self.observed_at = now;
    }

    fn seen(&self) -> bool {
        self.observed_at != DateTime::<Utc>::MIN_UTC
    }

    fn velocity(&self) -> f32 {
        let Some((pv, pt)) = self.prior else {
            return 0.0;
        };
        let dt = (self.observed_at - pt).num_milliseconds() as f32 / 1000.0;
        if dt <= 0.0 {
            return 0.0;
        }
        (self.value - pv) / dt
    }

    fn as_belief(&self, now: DateTime<Utc>) -> Belief<f32> {
        if !self.seen() {
            return Belief::unknown(now);
        }
        Belief {
            value: Some(self.value),
            confidence: 1.0,
            trend: Trend::from_velocity(self.velocity()),
            persistence: Duration::from_millis(
                (now - self.prior.map_or(self.observed_at, |(_, t)| t))
                    .num_milliseconds()
                    .max(0) as u64,
            ),
            observed_at: self.observed_at,
            half_life: self.half_life,
            sources: vec![self.name.clone()],
            conflicts: Vec::new(),
        }
    }
}

/// What a named afferent carries.
#[derive(Debug)]
enum Afferent {
    /// Secretions that stack and fade. The weather changed.
    Exudate(Field),
    /// A gauge. The value stands until re-read; the confidence does not.
    Level(Level),
}

/// One named afferent, its side of the boundary, and its bounds.
#[derive(Debug)]
struct FieldEntry {
    name: String,
    afferent: Afferent,
    locus: Locus,
    viability: Option<Viability>,
}

impl FieldEntry {
    fn belief(&self, now: DateTime<Utc>) -> Belief<f32> {
        match &self.afferent {
            Afferent::Exudate(f) => f.as_belief(now),
            Afferent::Level(l) => l.as_belief(now),
        }
    }

    /// The present magnitude, for the viability arithmetic.
    fn magnitude(&self, now: DateTime<Utc>) -> f32 {
        match &self.afferent {
            Afferent::Exudate(f) => f.level_at(now),
            Afferent::Level(l) => l.value,
        }
    }

    fn velocity(&self) -> f32 {
        match &self.afferent {
            Afferent::Exudate(f) => f.velocity(),
            Afferent::Level(l) => l.velocity(),
        }
    }
}

/// The body's somatic plexus.
#[derive(Debug)]
pub struct SomaticPlexus {
    fields: Vec<FieldEntry>,
    expected: Vec<String>,
    last_seen: HashMap<String, DateTime<Utc>>,
    health: HashMap<String, SourceHealth>,
    silent_after: Duration,
    absent_after: Duration,
    started_at: DateTime<Utc>,
    last_trend: HashMap<String, Trend>,
    /// Whether each field was over the allostatic threshold last tick, so a
    /// sustained threat reports once rather than every beat.
    was_threatened: HashMap<String, bool>,
    pending: Vec<SomaticEvent>,
}

impl SomaticPlexus {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self {
            fields: Vec::new(),
            expected: Vec::new(),
            last_seen: HashMap::new(),
            health: HashMap::new(),
            silent_after: Duration::from_secs(90),
            absent_after: Duration::from_secs(300),
            started_at: now,
            last_trend: HashMap::new(),
            was_threatened: HashMap::new(),
            pending: Vec::new(),
        }
    }

    /// An exudate field reporting the world. Nothing she does moves it.
    pub fn with_boundary_field(
        mut self,
        name: impl Into<String>,
        half_life: Duration,
        now: DateTime<Utc>,
    ) -> Self {
        let name = name.into();
        self.fields.push(FieldEntry {
            afferent: Afferent::Exudate(Field::new(&name, half_life, now)),
            name,
            locus: Locus::Boundary,
            viability: None,
        });
        self
    }

    /// An exudate field inside the boundary. `regulator` is the verb that
    /// reaches it; `None` records a §13 defect rather than hiding one.
    pub fn with_inner_field(
        mut self,
        name: impl Into<String>,
        half_life: Duration,
        regulator: Option<&str>,
        viability: Option<Viability>,
        now: DateTime<Utc>,
    ) -> Self {
        let name = name.into();
        self.fields.push(FieldEntry {
            afferent: Afferent::Exudate(Field::new(&name, half_life, now)),
            name,
            locus: Locus::Inner {
                regulator: regulator.map(str::to_string),
            },
            viability,
        });
        self
    }

    /// A gauge at the boundary — measured, not secreted.
    pub fn with_boundary_level(mut self, name: impl Into<String>, half_life: Duration) -> Self {
        let name = name.into();
        self.fields.push(FieldEntry {
            afferent: Afferent::Level(Level::new(&name, half_life)),
            name,
            locus: Locus::Boundary,
            viability: None,
        });
        self
    }

    /// A gauge inside the boundary — charge, thermal, load.
    pub fn with_inner_level(
        mut self,
        name: impl Into<String>,
        half_life: Duration,
        regulator: Option<&str>,
        viability: Option<Viability>,
    ) -> Self {
        let name = name.into();
        self.fields.push(FieldEntry {
            afferent: Afferent::Level(Level::new(&name, half_life)),
            name,
            locus: Locus::Inner {
                regulator: regulator.map(str::to_string),
            },
            viability,
        });
        self
    }

    pub fn with_expected(mut self, source: impl Into<String>) -> Self {
        self.expected.push(source.into());
        self
    }

    pub fn with_silent_after(mut self, d: Duration) -> Self {
        self.silent_after = d;
        self
    }

    pub fn with_absent_after(mut self, d: Duration) -> Self {
        self.absent_after = d;
        self
    }

    fn entry(&self, name: &str) -> Option<&FieldEntry> {
        self.fields.iter().find(|e| e.name == name)
    }

    /// The exudate field by that name, if it is one. Gauges answer `None`
    /// here and through [`Self::magnitude`] instead.
    pub fn field(&self, name: &str) -> Option<&Field> {
        match &self.entry(name)?.afferent {
            Afferent::Exudate(f) => Some(f),
            Afferent::Level(_) => None,
        }
    }

    /// Present magnitude of any afferent, exudate or gauge. `None` when it
    /// has never spoken — which is not zero.
    pub fn magnitude(&self, name: &str, now: DateTime<Utc>) -> Option<f32> {
        let e = self.entry(name)?;
        e.belief(now).is_known().then(|| e.magnitude(now))
    }

    pub fn locus(&self, name: &str) -> Option<&Locus> {
        self.entry(name).map(|e| &e.locus)
    }

    pub fn source_health(&self, name: &str) -> SourceHealth {
        self.health
            .get(name)
            .copied()
            .unwrap_or(SourceHealth::Unknown)
    }

    /// Inner fields with no verb that reaches them — doctrine §13, computed.
    ///
    /// INTERO axiom (7) says a variable is not interoceptive at all without
    /// a subsystem that acts on it. An entry here is either a missing verb
    /// or a field that was never inner to begin with.
    pub fn unreachable(&self) -> Vec<&str> {
        self.fields
            .iter()
            .filter(|e| e.locus.is_inner() && !e.locus.is_regulated())
            .map(|e| e.name.as_str())
            .collect()
    }

    /// Beliefs paired with their afferent names.
    pub fn named_beliefs(&self, now: DateTime<Utc>) -> Vec<(&str, Belief<f32>)> {
        self.fields
            .iter()
            .map(|e| (e.name.as_str(), e.belief(now)))
            .collect()
    }

    /// How hard this field is pressing toward leaving its range.
    ///
    /// Candia-Rivera's `g_t` reduced to the part that needs no model and no
    /// rollout: proximity to the bound, lifted toward 1 as the anticipated
    /// crossing falls inside the horizon. `None` when the field has no
    /// bounds or has never been secreted into — silence is not calm.
    pub fn allostatic_pressure(&self, name: &str, now: DateTime<Utc>) -> Option<f32> {
        let e = self.entry(name)?;
        let v = e.viability.as_ref()?;
        if !e.belief(now).is_known() {
            return None;
        }
        let level = e.magnitude(now);
        let proximity = v.boundary_proximity(level);
        let Some(eta) = v.eta_secs(level, e.velocity()) else {
            return Some(proximity);
        };
        let horizon = ALLOSTATIC_HORIZON.as_secs_f64();
        let imminence = (1.0 - (eta / horizon)).clamp(0.0, 1.0) as f32;
        // Closes the remaining gap rather than summing, so pressure
        // approaches 1 without a clamp doing the work.
        Some((proximity + imminence * (1.0 - proximity)).clamp(0.0, 1.0))
    }

    /// Every bounded field's pressure, highest first.
    pub fn pressures(&self, now: DateTime<Utc>) -> Vec<(&str, f32)> {
        let mut out: Vec<(&str, f32)> = self
            .fields
            .iter()
            .filter_map(|e| {
                self.allostatic_pressure(&e.name, now)
                    .map(|p| (e.name.as_str(), p))
            })
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out
    }

    /// Total weighted deviation outside preferred ranges — the homeostatic
    /// cost term `c_H`. Zero means every bounded field is where it wants to
    /// be.
    pub fn homeostatic_cost(&self, now: DateTime<Utc>) -> f32 {
        self.fields
            .iter()
            .filter_map(|e| {
                let v = e.viability.as_ref()?;
                e.belief(now)
                    .is_known()
                    .then(|| v.deviation(e.magnitude(now)))
            })
            .sum()
    }

    /// The body as it speaks on the IPC surface: every field's belief, trend,
    /// locus and source health, the bounded fields' pressures ranked worst
    /// first, and the unreachable list. Unknown stays null — never zero.
    pub fn to_json(&self, now: DateTime<Utc>) -> serde_json::Value {
        let fields: serde_json::Map<String, serde_json::Value> = self
            .fields
            .iter()
            .map(|e| {
                let belief = e.belief(now);
                let locus = match &e.locus {
                    Locus::Inner { regulator } => serde_json::json!({
                        "side": "inner",
                        "regulator": regulator,
                    }),
                    Locus::Boundary => serde_json::json!({ "side": "boundary" }),
                };
                let name = e.name.clone();
                let value = serde_json::json!({
                    "value": belief.value,
                    "confidence": belief.confidence_at(now),
                    "trend": trend_str(belief.trend),
                    "locus": locus,
                    "health": self.source_health(&name),
                    "pressure": self.allostatic_pressure(&name, now),
                });
                (name, value)
            })
            .collect();
        let pressures: Vec<serde_json::Value> = self
            .pressures(now)
            .into_iter()
            .map(|(name, p)| serde_json::json!([name, p]))
            .collect();
        serde_json::json!({
            "fields": fields,
            "pressures": pressures,
            "homeostatic_cost": self.homeostatic_cost(now),
            "unreachable": self.unreachable(),
        })
    }

    /// The phone's afferents as they stand — sessiond's field set, built
    /// here so the tests and the daemon read one list.
    ///
    /// iio at the boundary; charge and thermal inside it with their verbs;
    /// `attended` derived by sessiond's clock rather than reported by a
    /// sensor; and the two grip gauges, which nothing reports yet.
    pub fn phone_plexus(now: DateTime<Utc>) -> SomaticPlexus {
        SomaticPlexus::new(now)
            .with_boundary_field("proximity", Duration::from_secs(30), now)
            .with_boundary_field("motion", Duration::from_secs(60), now)
            .with_boundary_field("light", Duration::from_secs(120), now)
            .with_boundary_field("touch", Duration::from_secs(10), now)
            .with_boundary_field("attended", Duration::from_secs(20), now)
            .with_inner_level(
                "charge",
                Duration::from_secs(300),
                Some("doze"),
                Some(Viability::floor(0.2)),
            )
            .with_inner_level(
                "thermal",
                Duration::from_secs(120),
                Some("doze"),
                Some(Viability::ceiling(0.8)),
            )
            .with_inner_level(
                "port_mode",
                Duration::from_secs(600),
                Some("set_usb_mode"),
                None,
            )
            .with_boundary_level("grip_left", Duration::from_secs(30))
            .with_boundary_level("grip_right", Duration::from_secs(30))
            .with_expected("proximity")
            .with_expected("light")
            .with_expected("charge")
            .with_silent_after(Duration::from_secs(90))
            .with_absent_after(Duration::from_secs(300))
    }

    fn evaluate_health(&mut self, now: DateTime<Utc>) {
        let newly_silent: Vec<(String, u64)> = self
            .last_seen
            .iter()
            .filter_map(|(source, &last)| {
                let elapsed = (now - last).num_seconds().max(0) as u64;
                let current = self
                    .health
                    .get(source)
                    .copied()
                    .unwrap_or(SourceHealth::Unknown);
                (elapsed > self.silent_after.as_secs() && current == SourceHealth::Live)
                    .then(|| (source.clone(), elapsed))
            })
            .collect();

        for (source, elapsed) in newly_silent {
            self.health.insert(source.clone(), SourceHealth::Silent);
            let locus = self.locus(&source).cloned().unwrap_or(Locus::Boundary);
            self.pending.push(SomaticEvent::SourceSilent {
                source,
                locus,
                silent_for_secs: elapsed,
            });
        }

        let boot_secs = (now - self.started_at).num_seconds().max(0) as u64;
        if boot_secs <= self.absent_after.as_secs() {
            return;
        }
        let newly_absent: Vec<String> = self
            .expected
            .iter()
            .filter(|source| {
                !self.last_seen.contains_key(*source)
                    && self.health.get(*source).copied() != Some(SourceHealth::Absent)
            })
            .cloned()
            .collect();

        for source in newly_absent {
            self.health.insert(source.clone(), SourceHealth::Absent);
            let locus = self.locus(&source).cloned().unwrap_or(Locus::Boundary);
            self.pending.push(SomaticEvent::SourceSilent {
                source,
                locus,
                silent_for_secs: boot_secs,
            });
        }
    }

    fn detect_trend_shifts(&mut self, now: DateTime<Utc>) {
        let shifts: Vec<(String, Trend, Trend, f32)> = self
            .fields
            .iter()
            .filter_map(|e| {
                let belief = e.belief(now);
                if !belief.is_known() {
                    return None;
                }
                let previous = self
                    .last_trend
                    .get(&e.name)
                    .copied()
                    .unwrap_or(Trend::Stable);
                (belief.trend != previous).then(|| {
                    (
                        e.name.clone(),
                        previous,
                        belief.trend,
                        belief.value.unwrap_or(0.0),
                    )
                })
            })
            .collect();

        for (field, from, to, level) in shifts {
            self.last_trend.insert(field.clone(), to);
            self.pending.push(SomaticEvent::TrendShift {
                field,
                from,
                to,
                level,
            });
        }
    }

    fn detect_viability_threats(&mut self, now: DateTime<Utc>) {
        let crossings: Vec<(String, f32, Option<u64>)> = self
            .fields
            .iter()
            .filter_map(|e| {
                let pressure = self.allostatic_pressure(&e.name, now)?;
                let over = pressure >= ALLOSTATIC_THRESHOLD;
                let was = self.was_threatened.get(&e.name).copied().unwrap_or(false);
                if !over || was {
                    return None;
                }
                let v = e.viability.as_ref()?;
                let eta = v
                    .eta_secs(e.magnitude(now), e.velocity())
                    .map(|s| s.max(0.0) as u64);
                Some((e.name.clone(), pressure, eta))
            })
            .collect();

        // Recorded for every bounded field, so falling back under the
        // threshold re-arms the edge.
        let states: Vec<(String, bool)> = self
            .fields
            .iter()
            .filter_map(|e| {
                self.allostatic_pressure(&e.name, now)
                    .map(|p| (e.name.clone(), p >= ALLOSTATIC_THRESHOLD))
            })
            .collect();
        for (name, over) in states {
            self.was_threatened.insert(name, over);
        }

        for (field, pressure, eta_secs) in crossings {
            self.pending.push(SomaticEvent::ViabilityThreatened {
                field,
                pressure,
                eta_secs,
            });
        }
    }
}

fn trend_str(t: Trend) -> &'static str {
    match t {
        Trend::FallingFast => "falling_fast",
        Trend::Falling => "falling",
        Trend::Stable => "stable",
        Trend::Rising => "rising",
        Trend::RisingFast => "rising_fast",
    }
}

impl NervousNode for SomaticPlexus {
    type Input = SensorReading;
    type Output = f32;

    fn ingest(&mut self, input: SensorReading, now: DateTime<Utc>) {
        self.last_seen.insert(input.source.clone(), now);

        let was = self
            .health
            .get(&input.source)
            .copied()
            .unwrap_or(SourceHealth::Unknown);
        if matches!(was, SourceHealth::Silent | SourceHealth::Absent) {
            self.pending.push(SomaticEvent::SourceRecovered {
                source: input.source.clone(),
            });
        }
        self.health.insert(input.source.clone(), SourceHealth::Live);

        if let Some(e) = self.fields.iter_mut().find(|e| e.name == input.source) {
            // The afferent's declared kind decides what the number means, so
            // a reporter never has to know whether it is feeding a gauge or
            // a gland.
            match &mut e.afferent {
                Afferent::Exudate(f) => f.secrete(input.amount, now),
                Afferent::Level(l) => l.observe(input.amount, now),
            }
        }
    }

    fn tick(&mut self, now: DateTime<Utc>) {
        self.evaluate_health(now);
        self.detect_trend_shifts(now);
        self.detect_viability_threats(now);
    }

    fn beliefs(&self, now: DateTime<Utc>) -> Vec<Belief<f32>> {
        self.fields.iter().map(|e| e.belief(now)).collect()
    }

    fn notable(&mut self) -> Vec<SomaticEvent> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-15T14:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn after(secs: i64) -> DateTime<Utc> {
        t0() + chrono::Duration::seconds(secs)
    }

    /// The production field set, on the test clock.
    fn phone_plexus() -> SomaticPlexus {
        SomaticPlexus::phone_plexus(t0())
    }

    fn read(source: &str, amount: f32) -> SensorReading {
        SensorReading {
            source: source.into(),
            amount,
        }
    }

    #[test]
    fn fields_accumulate_on_repeated_secretions() {
        let mut p = phone_plexus();
        for i in 0..5 {
            p.ingest(read("proximity", 0.15), after(i * 2));
        }
        let level = p.field("proximity").unwrap().level_at(after(8));
        assert!(level > 0.5, "five secretions should accumulate: {level}");
    }

    #[test]
    fn fields_decay_when_untouched() {
        let mut p = phone_plexus();
        p.ingest(read("proximity", 0.5), t0());
        let early = p.field("proximity").unwrap().level_at(after(5));
        let late = p.field("proximity").unwrap().level_at(after(60));
        assert!(late < early, "should decay: early={early}, late={late}");
        assert!(late < 0.15, "two half-lives leaves little: {late}");
    }

    #[test]
    fn a_gauge_holds_its_value_while_an_exudate_fades() {
        let mut p = phone_plexus();
        p.ingest(read("charge", 0.42), t0());
        p.ingest(read("proximity", 0.5), t0());

        // Ten minutes of silence. The battery is still where it was; the
        // secretion is not.
        let charge = p.magnitude("charge", after(600)).unwrap();
        let prox = p.magnitude("proximity", after(600)).unwrap();
        assert!(
            (charge - 0.42).abs() < 0.001,
            "a pack does not drain because nobody looked: {charge}"
        );
        assert!(prox < 0.05, "an exudate fades: {prox}");
    }

    #[test]
    fn an_unread_gauge_loses_confidence_but_not_its_reading() {
        let mut p = phone_plexus();
        p.ingest(read("charge", 0.42), t0());

        let fresh = p.named_beliefs(t0());
        let stale = p.named_beliefs(after(900));
        let pick = |bs: &Vec<(&str, Belief<f32>)>, at: DateTime<Utc>| {
            let b = bs.iter().find(|(n, _)| *n == "charge").unwrap().1.clone();
            (b.value.unwrap(), b.confidence_at(at))
        };
        let (v0, c0) = pick(&fresh, t0());
        let (v1, c1) = pick(&stale, after(900));

        assert_eq!(v0, v1, "the reading stands");
        assert!(c1 < c0 * 0.2, "trust in it does not: {c0} -> {c1}");
    }

    #[test]
    fn unsecreted_fields_report_unknown_not_zero() {
        let p = phone_plexus();
        for b in &p.beliefs(after(5)) {
            assert!(!b.is_known(), "never secreted = unknown, not zero");
        }
    }

    // ── health ────────────────────────────────────────────────────

    #[test]
    fn source_goes_silent_after_threshold() {
        let mut p = phone_plexus();
        p.ingest(read("proximity", 0.2), t0());
        assert_eq!(p.source_health("proximity"), SourceHealth::Live);

        p.tick(after(100));
        assert_eq!(p.source_health("proximity"), SourceHealth::Silent);
        assert!(p.notable().iter().any(|e| matches!(
            e,
            SomaticEvent::SourceSilent { source, .. } if source == "proximity"
        )));
    }

    #[test]
    fn expected_source_absent_if_never_reported() {
        let mut p = phone_plexus();
        p.tick(after(310));
        assert_eq!(p.source_health("charge"), SourceHealth::Absent);
        assert!(p.notable().iter().any(|e| matches!(
            e,
            SomaticEvent::SourceSilent { source, .. } if source == "charge"
        )));
    }

    #[test]
    fn silence_carries_its_locus_so_numb_and_blind_differ() {
        let mut p = phone_plexus();
        p.ingest(read("proximity", 0.2), t0());
        p.ingest(read("charge", 0.5), t0());
        p.tick(after(100));

        let events = p.notable();
        let prox = events
            .iter()
            .find(
                |e| matches!(e, SomaticEvent::SourceSilent { source, .. } if source == "proximity"),
            )
            .expect("proximity should be silent");
        let charge = events
            .iter()
            .find(|e| matches!(e, SomaticEvent::SourceSilent { source, .. } if source == "charge"))
            .expect("charge should be silent");

        match prox {
            SomaticEvent::SourceSilent { locus, .. } => {
                assert_eq!(*locus, Locus::Boundary, "a blind nerve is at the boundary")
            }
            _ => unreachable!(),
        }
        match charge {
            SomaticEvent::SourceSilent { locus, .. } => {
                assert!(locus.is_inner(), "a numb nerve is inside");
                assert!(locus.is_regulated(), "and charge has a verb");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn silent_source_recovers_on_report() {
        let mut p = phone_plexus();
        p.ingest(read("proximity", 0.2), t0());
        p.tick(after(100));
        p.notable();

        p.ingest(read("proximity", 0.2), after(101));
        assert_eq!(p.source_health("proximity"), SourceHealth::Live);
        assert!(p.notable().iter().any(|e| matches!(
            e,
            SomaticEvent::SourceRecovered { source } if source == "proximity"
        )));
    }

    #[test]
    fn absent_source_recovers_on_first_report() {
        let mut p = phone_plexus();
        p.tick(after(310));
        p.notable();

        p.ingest(read("charge", 0.1), after(311));
        assert_eq!(p.source_health("charge"), SourceHealth::Live);
        assert!(p.notable().iter().any(|e| matches!(
            e,
            SomaticEvent::SourceRecovered { source } if source == "charge"
        )));
    }

    #[test]
    fn notable_events_are_consumed_on_read() {
        let mut p = phone_plexus();
        p.ingest(read("proximity", 0.2), t0());
        p.tick(after(100));
        assert!(!p.notable().is_empty());
        assert!(p.notable().is_empty(), "second read must be empty");
    }

    // ── locus ─────────────────────────────────────────────────────

    #[test]
    fn every_inner_field_has_a_verb_that_reaches_it() {
        // Doctrine §13 as a test. A failure here is a real defect, not a
        // fixture that drifted.
        assert!(
            phone_plexus().unreachable().is_empty(),
            "inner fields with no regulator: {:?}",
            phone_plexus().unreachable()
        );
    }

    #[test]
    fn an_unregulated_inner_field_is_reported() {
        let now = t0();
        let p = SomaticPlexus::new(now).with_inner_field(
            "cpu_load",
            Duration::from_secs(30),
            None,
            None,
            now,
        );
        assert_eq!(p.unreachable(), vec!["cpu_load"]);
    }

    #[test]
    fn boundary_fields_are_never_counted_unreachable() {
        let now = t0();
        let p = SomaticPlexus::new(now).with_boundary_field("motion", Duration::from_secs(30), now);
        assert!(p.unreachable().is_empty(), "nothing moves the world");
    }

    // ── viability ─────────────────────────────────────────────────

    #[test]
    fn deviation_is_asymmetric() {
        let charge = Viability::floor(0.2);
        assert!(charge.deviation(0.1) > 0.0, "empty threatens");
        assert_eq!(charge.deviation(1.0), 0.0, "full does not");

        let thermal = Viability::ceiling(0.8);
        assert!(thermal.deviation(0.95) > 0.0, "hot threatens");
        assert_eq!(thermal.deviation(0.0), 0.0, "cold does not");
    }

    #[test]
    fn boundary_proximity_peaks_at_the_bound() {
        let v = Viability::symmetric(0.2, 0.8);
        assert!(v.boundary_proximity(0.5) < 0.05, "centre is calm");
        assert_eq!(v.boundary_proximity(0.2), 1.0);
        assert_eq!(v.boundary_proximity(0.05), 1.0, "outside stays 1");
        assert!(v.boundary_proximity(0.3) > v.boundary_proximity(0.45));
    }

    #[test]
    fn a_costless_bound_does_not_draw() {
        // The whole point of the asymmetry: full is not a crisis.
        let charge = Viability::floor(0.2);
        assert_eq!(
            charge.boundary_proximity(1.0),
            0.0,
            "full is not near-death"
        );
        assert_eq!(charge.boundary_proximity(0.2), 1.0, "empty is");
        assert!(
            charge.eta_secs(0.9, 0.05).is_none(),
            "filling is not a threat"
        );

        let thermal = Viability::ceiling(0.8);
        assert_eq!(thermal.boundary_proximity(0.0), 0.0, "cold is not a crisis");
        assert_eq!(thermal.boundary_proximity(0.8), 1.0, "hot is");
    }

    #[test]
    fn eta_is_none_when_travelling_away_from_the_bound() {
        let v = Viability::floor(0.2);
        // Low in the range but rising — the floor is receding.
        assert!(v.eta_secs(0.3, 0.05).is_none());
        // Falling toward it.
        let eta = v.eta_secs(0.3, -0.05).expect("closing on the floor");
        assert!((eta - 2.0).abs() < 0.01, "0.1 gap at 0.05/s = 2 s: {eta}");
    }

    #[test]
    fn pressure_is_none_for_a_field_that_never_spoke() {
        let p = phone_plexus();
        assert!(
            p.allostatic_pressure("charge", after(5)).is_none(),
            "silence is not calm"
        );
    }

    #[test]
    fn pressure_is_none_for_an_unbounded_field() {
        let mut p = phone_plexus();
        p.ingest(read("port_mode", 0.5), t0());
        assert!(p.allostatic_pressure("port_mode", after(1)).is_none());
    }

    #[test]
    fn a_field_draining_toward_its_floor_builds_pressure() {
        let mut p = phone_plexus();
        p.ingest(read("charge", 0.9), t0());
        let resting = p.allostatic_pressure("charge", after(1)).unwrap();

        // A gauge is *told* its new reading; it is not decremented.
        p.ingest(read("charge", 0.55), after(2));
        let draining = p.allostatic_pressure("charge", after(2)).unwrap();
        assert!(
            draining > resting,
            "falling toward the floor must press harder: {resting} -> {draining}"
        );
    }

    #[test]
    fn viability_threat_fires_on_the_edge_only() {
        let mut p = phone_plexus();
        p.ingest(read("charge", 0.9), t0());
        p.ingest(read("charge", 0.30), after(1));
        p.tick(after(1));

        let threats: Vec<_> = p
            .notable()
            .into_iter()
            .filter(|e| matches!(e, SomaticEvent::ViabilityThreatened { field, .. } if field == "charge"))
            .collect();
        assert_eq!(threats.len(), 1, "one edge: {threats:?}");

        p.tick(after(2));
        assert!(
            !p.notable().iter().any(|e| matches!(
                e,
                SomaticEvent::ViabilityThreatened { field, .. } if field == "charge"
            )),
            "a sustained threat must not re-fire"
        );
    }

    #[test]
    fn a_field_resting_inside_its_range_never_threatens() {
        let mut p = phone_plexus();
        p.ingest(read("charge", 0.6), t0());
        p.tick(after(1));
        assert!(
            !p.notable()
                .iter()
                .any(|e| matches!(e, SomaticEvent::ViabilityThreatened { .. })),
            "calm must stay quiet"
        );
    }

    #[test]
    fn homeostatic_cost_is_zero_inside_every_range() {
        let mut p = phone_plexus();
        p.ingest(read("charge", 0.6), t0());
        p.ingest(read("thermal", 0.3), t0());
        assert_eq!(p.homeostatic_cost(after(1)), 0.0);
    }

    #[test]
    fn homeostatic_cost_counts_only_what_left_its_range() {
        let mut p = phone_plexus();
        p.ingest(read("thermal", 0.95), t0());
        p.ingest(read("charge", 0.9), t0());
        let cost = p.homeostatic_cost(after(1));
        assert!(cost > 0.0, "thermal is over its ceiling: {cost}");
        assert!(cost < 0.2, "and charge, being full, adds nothing: {cost}");
    }

    #[test]
    fn pressures_are_ranked_worst_first() {
        let mut p = phone_plexus();
        p.ingest(read("charge", 0.95), t0());
        p.ingest(read("thermal", 0.79), t0());
        let ranked = p.pressures(after(1));
        assert_eq!(ranked.len(), 2, "only bounded fields: {ranked:?}");
        assert_eq!(ranked[0].0, "thermal", "nearest its bound leads");
    }

    // ── trend ─────────────────────────────────────────────────────

    #[test]
    fn trend_shifts_are_detected() {
        let mut p = phone_plexus();
        p.ingest(read("motion", 0.1), t0());
        p.ingest(read("motion", 0.4), after(1));
        p.tick(after(1));

        assert!(
            p.notable().iter().any(|e| matches!(
                e,
                SomaticEvent::TrendShift { field, .. } if field == "motion"
            )),
            "motion trending up should surface"
        );
    }

    #[test]
    fn trend_shifts_fire_once_per_edge() {
        let mut p = phone_plexus();
        p.ingest(read("motion", 0.1), t0());
        p.ingest(read("motion", 0.4), after(1));
        p.tick(after(1));
        p.notable();

        p.tick(after(2));
        assert!(
            !p.notable().iter().any(|e| matches!(
                e,
                SomaticEvent::TrendShift { field, .. } if field == "motion"
            )),
            "the same trend must not re-fire"
        );
    }

    // ── beliefs ───────────────────────────────────────────────────

    #[test]
    fn beliefs_reflect_accumulated_state() {
        let mut p = phone_plexus();
        p.ingest(read("proximity", 0.5), t0());
        p.ingest(read("motion", 0.3), t0());

        let beliefs = p.beliefs(after(1));
        assert_eq!(beliefs.len(), 10);
        assert!(beliefs[0].is_known());
        assert!(beliefs[0].value.unwrap() > 0.3);
        assert!(beliefs[1].is_known());
    }

    #[test]
    fn named_beliefs_pair_fields_with_names() {
        let mut p = phone_plexus();
        p.ingest(read("proximity", 0.5), t0());
        let named = p.named_beliefs(after(1));
        assert_eq!(named[0].0, "proximity");
        assert!(named[0].1.is_known());
    }

    #[test]
    fn a_source_with_no_field_is_health_tracked_only() {
        let mut p = phone_plexus();
        p.ingest(read("usb_peer", 1.0), t0());
        assert_eq!(p.source_health("usb_peer"), SourceHealth::Live);
        assert_eq!(p.fields.len(), 10, "no field is conjured for it");
    }

    #[test]
    fn to_json_carries_unknown_as_null_not_zero() {
        let p = phone_plexus();
        let body = p.to_json(after(1));
        let light = &body["fields"]["light"];
        assert_eq!(
            light["value"],
            serde_json::Value::Null,
            "never spoke must read as unknown, not calm"
        );
        assert_eq!(body["fields"]["attended"]["trend"].as_str(), Some("stable"));
        assert_eq!(
            body["fields"]["charge"]["locus"]["side"].as_str(),
            Some("inner")
        );
        assert!(body["pressures"].as_array().unwrap().is_empty());
        assert!(body["unreachable"].as_array().unwrap().is_empty());
    }
}
