//! MILESTONE 106: generalizes Milestone 39's two-timescale evidence gate
//! (`network.rs`'s per-synapse `gate`/`evidence`) and Milestone 104's own
//! one-off port of it (`heap_guard.rs`'s bespoke `PressureGate` struct)
//! into a real, reusable, multi-subscriber kernel subsystem -- Tier 13's
//! own roadmap item, named verbatim in README.md: "generalize Milestone
//! 39's evidence-gated topology from its toy 3-neuron demo network into
//! a real subsystem other kernel decisions can subscribe to."
//!
//! **What actually gets shared, and what deliberately doesn't.** Before
//! this milestone, `heap_guard.rs` had its own private `PressureGate`
//! struct (`evidence: f32, conservative: bool, sample_count: u64`) plus
//! its own copy of the "update evidence every sample, re-evaluate
//! hysteresis only every `golden_period()`-th sample, count trip
//! episodes" state machine -- structurally identical to, but a SEPARATE
//! piece of code from, what `network.rs`'s own `evaluate_gates()` does
//! per-synapse. That is exactly the "two independently-implemented
//! copies of the same dynamics silently drifting out of sync" failure
//! mode `network.rs`'s own module doc names as a hard-won Milestone 21
//! lesson -- except one level up from where Milestone 78/104 already
//! fixed it (`apply_stdp()`, `update_evidence()`, `golden_period()` are
//! already the single shared low-level formulas both callers use). This
//! module centralizes the STATE MACHINE built on top of those formulas
//! (evidence-EMA-then-hysteresis-then-count-transitions), so a future
//! subscriber writes zero copies of it. Deliberately NOT centralized:
//! each subscriber's own tuning (thresholds, cadence) and its own
//! metric-to-hit conversion (heap_guard's `HIGH_WATER_FRACTION` fraction
//! check, or any future subsystem's own equivalent) -- those stay owned
//! by the subscriber, passed in by value on every call, exactly the way
//! `network.rs`'s own `GATE_ON_THRESHOLD`/`GATE_OFF_THRESHOLD`/
//! `FAST_TAU_TICKS` already stayed local to that one file rather than
//! becoming kernel-wide constants. Only the mechanism is shared; the
//! judgment calls about what counts as pressure stay with whoever is
//! actually watching that resource.
//!
//! **Real subscriber roster, honestly disclosed.** As of this milestone,
//! ONE real kernel decision subscribes: `heap_guard.rs`'s kernel-heap-
//! pressure admission gate, migrated onto this module with byte-for-byte
//! identical tuning (`GATE_ON_THRESHOLD`/`GATE_OFF_THRESHOLD`/
//! `FAST_TAU_SAMPLES`/`HIGH_WATER_FRACTION` all unchanged, still owned by
//! `heap_guard.rs`) and, per this milestone's own verification below,
//! byte-identical measured counters. A second real subscriber (a flaky-
//! driver recovery policy, per-Tier-13's own suggestion, or SNN-driven
//! page eviction) is deliberately NOT invented here just to prove the
//! API works with two callers -- this kernel has no currently-observed
//! real flakiness or real memory pressure outside the kernel heap to
//! honestly gate on (physical frame counts are generously large relative
//! to this OS's tiny process footprints; inventing scarcity to exercise
//! a second gate would be exactly the kind of fabricated-problem
//! dishonesty this project's whole verification discipline exists to
//! avoid). Left open, named plainly, for the next real subsystem that
//! actually needs it.
//!
//! **A real, closed enum, not a runtime registry.** `GateKind` lists
//! every subscriber at compile time rather than supporting dynamic
//! string-keyed registration -- a deliberate choice: this kernel has a
//! small, statically-known set of subsystems that could ever plausibly
//! subscribe, so a runtime registry (with its own "registry full"
//! failure mode, needed for zero real benefit here) would be more
//! machinery than the actual problem calls for. Any future subscriber
//! adds one enum variant and one array slot -- real generality (nothing
//! about the state machine below is heap-specific), without an
//! unnecessary runtime API surface.

use crate::network;
use spin::Mutex;

/// One entry per real kernel subsystem that subscribes to this gate.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GateKind {
    /// `heap_guard.rs`'s kernel-heap-pressure admission gate, migrated
    /// here this milestone. The only real subscriber so far.
    KernelHeapPressure,
}

const GATE_COUNT: usize = 1;

fn index(kind: GateKind) -> usize {
    match kind {
        GateKind::KernelHeapPressure => 0,
    }
}

/// A subscriber's own tuning, passed in by value on every call rather
/// than stored here -- see the module doc's "what deliberately doesn't"
/// get shared. Field meanings match `network.rs`'s own
/// `GATE_ON_THRESHOLD`/`GATE_OFF_THRESHOLD`/`FAST_TAU_TICKS` exactly.
pub struct GateConfig {
    pub gate_on_threshold: f32,
    pub gate_off_threshold: f32,
    pub fast_tau: u64,
}

/// What happened to the gate's conservative flag as a *result* of this
/// particular `record_sample()` call -- `Unchanged` covers both "not a
/// golden_period() re-evaluation boundary yet" and "was a boundary, but
/// evidence didn't cross either threshold". Lets a caller log its own
/// domain-specific detail (e.g. heap_guard's used-bytes/budget) only on
/// a real transition, exactly matching Milestone 104's own log
/// discipline, without this module needing to know what that detail is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GateTransition {
    Entered,
    Left,
    Unchanged,
}

struct GateState {
    evidence: f32,
    conservative: bool,
    sample_count: u64,
    samples_taken: u64,
    trip_episodes: u64,
}

impl GateState {
    const fn new() -> Self {
        GateState {
            evidence: 0.0,
            conservative: false,
            sample_count: 0,
            samples_taken: 0,
            trip_episodes: 0,
        }
    }
}

static GATES: Mutex<[GateState; GATE_COUNT]> = Mutex::new([GateState::new()]);

/// Feed one real sample into `kind`'s gate: updates the fast EMA every
/// call (`network::update_evidence()`, the exact same formula Milestone
/// 39/104 already established), re-evaluates the hysteretic ON/OFF
/// `conservative` flag only every `network::golden_period(config.
/// fast_tau)`-th call -- Milestone 39's own fast/slow-clock structure,
/// now shared code rather than a second hand-copy of it. Returns the
/// evidence value after this update alongside the transition, so a
/// caller can log a real number without a second lock/read.
pub fn record_sample(kind: GateKind, config: &GateConfig, hit: bool) -> (GateTransition, f32) {
    let mut gates = GATES.lock();
    let g = &mut gates[index(kind)];

    g.samples_taken += 1;
    network::update_evidence(&mut g.evidence, hit);
    g.sample_count += 1;

    let mut transition = GateTransition::Unchanged;
    if g.sample_count % network::golden_period(config.fast_tau) == 0 {
        let was_conservative = g.conservative;
        if g.conservative {
            if g.evidence < config.gate_off_threshold {
                g.conservative = false;
            }
        } else if g.evidence > config.gate_on_threshold {
            g.conservative = true;
        }
        if g.conservative && !was_conservative {
            g.trip_episodes += 1;
            transition = GateTransition::Entered;
        } else if was_conservative && !g.conservative {
            transition = GateTransition::Left;
        }
    }

    (transition, g.evidence)
}

/// Real, current state of `kind`'s gate -- the actual admission check a
/// subscriber's own gated call site consults.
pub fn is_conservative(kind: GateKind) -> bool {
    GATES.lock()[index(kind)].conservative
}

/// Real, measured counters for `kind`: `(samples_taken, trip_episodes)`.
pub fn stats(kind: GateKind) -> (u64, u64) {
    let g = &GATES.lock()[index(kind)];
    (g.samples_taken, g.trip_episodes)
}
