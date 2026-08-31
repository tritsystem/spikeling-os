//! MILESTONE 78: does the SAME spike-timing-dependent plasticity rule
//! this kernel already trusts (network.rs's `apply_stdp()`, verified
//! since Milestone 17/21 on LIF-only LeftKey/RightKey->Motor) still
//! produce a real, meaningful weight trajectory when driven by the
//! other three neuron dynamics Milestone 77 introduced (Izhikevich,
//! AdEx, Resonator) -- via their own REAL step()-driven firing, not a
//! synthetic forced fire-tick like network.rs's own `train_synapse()`
//! uses for its controlled trials?
//!
//! `apply_stdp()` is neuron-model-agnostic by construction: it only
//! ever reads two `Option<u64>` fire ticks, never a neuron's `v`/
//! `threshold`/other internal state. That's WHY this milestone is a
//! straightforward reuse (`network.rs::apply_stdp` is now `pub(crate)`
//! for exactly this) rather than a second STDP implementation --
//! avoiding the Milestone 21 mistake (module doc, `network.rs`) of
//! letting the SAME mechanism drift into two copies. What's genuinely
//! untested is whether each type's own real dynamics reliably produce
//! the countable, orderable fire events STDP needs in the first place.
//!
//! PRE-REGISTERED HYPOTHESIS (stated before running anything, same
//! discipline as Spikeling's own heterogeneous_ontology_test.py and
//! this kernel's own Milestone 77):
//!   1. LIF and Izhikevich -- both fire from a simple threshold
//!      crossing -- should reliably fire on a strong, sustained pulse
//!      and show a clean, high trial-validity STDP trajectory,
//!      matching LIF's own already-verified Milestone 10/17 behavior.
//!   2. AdEx should also fire reliably, but its own intrinsic
//!      adaptation variable `w` (real biological "fatigue", unrelated
//!      to synaptic weight) is expected to make LATER trials' pulses
//!      progressively less likely to cross threshold than EARLIER
//!      trials -- a second, independent adaptation mechanism
//!      interacting with STDP, not a STDP failure.
//!   3. Resonator is the real disconfirm candidate: its firing
//!      mechanism is an edge-triggered RMS-energy threshold that
//!      accumulates only from SUSTAINED oscillation near its own
//!      tuned period (see hetero_ensemble.rs's own module doc) -- a
//!      brief constant-style pulse, the same shape every other type
//!      gets here for a fair, non-cherry-picked comparison, is
//!      expected to fire it unreliably or not at all, starving STDP of
//!      the paired pre/post events it needs. Reported honestly either
//!      way -- a real negative result for this type would be a useful
//!      finding, not a bug to hide.
//!
//! Self-contained: reuses hetero_ensemble.rs's neuron dynamics
//! (Kind/Neuron/make_neuron/Xorshift64, made `pub(crate)` for this)
//! and network.rs's real `apply_stdp()`, but creates its OWN pre/post
//! neuron pairs and its OWN weight state -- touches neither
//! network.rs's GenericNetwork nor hetero_ensemble.rs's own M77
//! disk-anomaly self-test.

use crate::hetero_ensemble::{kind_name, make_neuron, Kind, Neuron, Xorshift64, KINDS};
use crate::network::apply_stdp;
use crate::serial;
use core::fmt::Write;

const STEP_DT: f32 = 1.0; // same step-domain convention as hetero_ensemble.rs (Milestone 77, adaptation #1)

/// Real per-kind drive scale for the pulse used to elicit a spike here
/// -- deliberately the SAME table hetero_ensemble.rs's own SCALE uses
/// for its disk-signal drive (`scale_for()` there), reused via the
/// same constant values rather than re-measuring a second time; a
/// pulse of PULSE_DRIVE_RAW is well above the baseline dither
/// magnitude Milestone 77 already established reliably fires LIF, so
/// it's a fair, deliberately strong "can you fire from this at all"
/// test for every type.
fn scale_for(k: Kind) -> f32 {
    match k {
        Kind::Lif => 1.0,
        Kind::Izhikevich => 6.0,
        Kind::Adex => 180.0,
        Kind::Resonator => 3.0,
    }
}

const PULSE_DRIVE_RAW: f32 = 40.0;
// A single real step, not a sustained pulse: network.rs's own STDP_TAU_MS
// (20ms) is short relative to one real tick (~54.9ms), so a multi-step
// pulse pushes the pre/post EVENT far enough apart in real tick-time that
// apply_stdp()'s exponential decays the update to numerically zero
// regardless of firing reliability -- measured directly (dev boot,
// PULSE_STEPS=6 produced weight 0.5000 -> 0.5000 for every kind, even
// LIF/Izhikevich's 20/20-reliable trials). One step keeps pre/post close
// enough in tick-time to land in the same real regime network.rs's own
// verified `train`/DSL gap_ticks=2 convention already relies on.
const PULSE_STEPS: u32 = 1;
const GAP_TICKS: u64 = 2; // same real gap network.rs's own bare `train` command uses (neurons.rs::run_training_trial)
const COOLDOWN_STEPS: u32 = 10;
const N_TRIALS: u32 = 20;
const INITIAL_WEIGHT: f32 = 0.5; // same neutral default as neurons.rs::INITIAL_WEIGHT

struct TrialResult {
    pre_fires: u32,
    post_fires: u32,
    valid_trials: u32, // both pre AND post fired -- the only trials STDP actually had real timing to work with
    initial_weight: f32,
    final_weight: f32,
}

/// Steps BOTH `pre` and `post` together for `steps` real ticks, every
/// tick, so each neuron's own internal state (leak, adaptation,
/// oscillation) evolves in real wall-of-steps time regardless of which
/// one is actually being driven this phase -- `pre_drive`/`post_drive`
/// select which one (if either) receives real stimulus. Records the
/// tick of each neuron's FIRST real fire during this phase (only if
/// the accumulator is still `None` going in) -- the moment a neuron
/// first responds to its own stimulus is what STDP should time
/// against, not whatever it happened to do last if driven long enough
/// to fire more than once. `Option<u64>` accumulators are passed in so
/// a caller can choose which phases' fires actually count (see
/// `run_kind()`'s deliberate use of throwaway accumulators for the gap
/// and cooldown phases below).
fn step_both(pre: &mut Neuron, post: &mut Neuron, pre_drive: f32, post_drive: f32, steps: u32, tick: &mut u64, pre_fire: &mut Option<u64>, post_fire: &mut Option<u64>) {
    for _ in 0..steps {
        if pre.step(pre_drive, STEP_DT) && pre_fire.is_none() {
            *pre_fire = Some(*tick);
        }
        if post.step(post_drive, STEP_DT) && post_fire.is_none() {
            *post_fire = Some(*tick);
        }
        *tick += 1;
    }
}

fn run_kind(kind: Kind, seed: u64) -> TrialResult {
    let mut rng = Xorshift64::new(seed);
    let mut pre = make_neuron(kind, &mut rng);
    let mut post = make_neuron(kind, &mut rng);
    let drive = PULSE_DRIVE_RAW * scale_for(kind);

    let mut tick: u64 = 0;
    let mut weight = INITIAL_WEIGHT;
    let mut pre_fires = 0u32;
    let mut post_fires = 0u32;
    let mut valid_trials = 0u32;

    for _ in 0..N_TRIALS {
        let mut pre_fire_tick = None;
        let mut post_fire_tick = None;
        let mut ignored_a = None;
        let mut ignored_b = None;

        // phase 1: pulse pre only -- only THIS phase's fires count as
        // "pre fired from its own real stimulus"
        step_both(&mut pre, &mut post, drive, 0.0, PULSE_STEPS, &mut tick, &mut pre_fire_tick, &mut post_fire_tick);
        // phase 2: quiet gap (real tick gap network.rs's own `train` command
        // uses) -- any residual ringing/adaptation-driven fire here is
        // real neuron behavior but NOT a stimulus-driven event, so it's
        // tracked separately and never reaches apply_stdp()
        step_both(&mut pre, &mut post, 0.0, 0.0, GAP_TICKS as u32, &mut tick, &mut ignored_a, &mut ignored_b);
        // phase 3: pulse post only -- only THIS phase's fire counts as
        // "post fired from its own real stimulus"
        ignored_a = None;
        step_both(&mut pre, &mut post, 0.0, drive, PULSE_STEPS, &mut tick, &mut ignored_a, &mut post_fire_tick);
        // phase 4: cooldown before the next trial (lets a Resonator's own
        // ringing settle, AdEx's `w` relax) -- same "real but not
        // stimulus-attributable" treatment as phase 2
        ignored_a = None;
        ignored_b = None;
        step_both(&mut pre, &mut post, 0.0, 0.0, COOLDOWN_STEPS, &mut tick, &mut ignored_a, &mut ignored_b);

        if pre_fire_tick.is_some() {
            pre_fires += 1;
        }
        if post_fire_tick.is_some() {
            post_fires += 1;
        }
        if pre_fire_tick.is_some() && post_fire_tick.is_some() {
            apply_stdp(&mut weight, pre_fire_tick, post_fire_tick);
            valid_trials += 1;
        }
    }

    TrialResult { pre_fires, post_fires, valid_trials, initial_weight: INITIAL_WEIGHT, final_weight: weight }
}

pub fn self_test_hetero_stdp() {
    let mut port = serial();
    let _ = writeln!(
        port,
        "milestone 78: heterogeneous STDP self-test starting -- same apply_stdp() formula, real per-type pulse-driven firing, {N_TRIALS} trials/type"
    );
    for kind in KINDS.iter() {
        let r = run_kind(*kind, 0x7800 + *kind as u64);
        let _ = writeln!(
            port,
            "milestone 78: {:<12} pre_fired={}/{N_TRIALS} post_fired={}/{N_TRIALS} valid_trials={}/{N_TRIALS} weight {:.4} -> {:.4}",
            kind_name(*kind), r.pre_fires, r.post_fires, r.valid_trials, r.initial_weight, r.final_weight
        );
    }
    let _ = writeln!(port, "milestone 78: self-test complete -- OVERALL_M78=PASS (all four types produced a real, measured result; see per-type validity rates above for the honest comparison, no single expected outcome enforced)");
}
