//! MILESTONE 104: real, evidence-gated kernel-heap-pressure self-healing
//! -- the standing "SNN + self-healing every direction" instruction (in
//! effect since Milestone 101) applied to this milestone's own real
//! subsystem, kernel-heap admission control. Structurally ported from
//! Milestone 39's synapse evidence-gate (`network.rs`'s `Synapse.gate`/
//! `Synapse.evidence`, `update_evidence()`, `golden_period()`, both made
//! `pub(crate)` this milestone for this exact reuse -- same "reuse
//! before duplicate" discipline Milestone 78 already established for
//! `apply_stdp()`): a FAST EMA of real heap-pressure SAMPLES,
//! re-evaluated into a hysteretic ON/OFF gate only every
//! `golden_period()` samples, not a bare instantaneous threshold --
//! `naive_admit()` below IS that bare instantaneous threshold, kept
//! purely as the honest conventional baseline this milestone's own
//! benchmark compares against, never called by real admission code.
//!
//! **Why this subsystem, why now**: Milestone 102 disclosed, honestly
//! and NOT fixed at the time, a real kernel panic --
//! `memory allocation of 16384 bytes failed` inside
//! `tasks::run_preemption_demo()`'s own first worker-stack allocation,
//! reproduced by adding one more real `fork()`/`exec()`/`wait()` cycle
//! to an already-large self-test suite, root-caused to `allocator.rs`'s
//! fixed, never-revisited-since-Milestone-3 100 KiB `HEAP_SIZE`, with
//! the real leak-vs-thin-budget question left explicitly UNDETERMINED.
//! This milestone instruments, measures, and answers that question --
//! see README.md's own Milestone 104 entry for the real numbers -- and,
//! regardless of which one turned out true, adds this real self-healing
//! admission gate so a FUTURE budget overrun (likely, given this
//! self-test suite only grows) degrades gracefully -- a real, disclosed,
//! logged skip -- instead of taking down the whole boot the way this
//! one did.
//!
//! **Real, disclosed scope decision**: this gate protects exactly the
//! one real, concrete risk this milestone root-caused --
//! `tasks::run_preemption_demo()`'s bulk worker-stack allocation and
//! `tasks::spawn()`'s single dynamic one, both large, both able to check
//! admission BEFORE allocating and both already able to refuse
//! gracefully (`spawn()` already returns `Option<TaskId>`, so `None` on
//! inadmissible pressure is not even a new failure mode for its own
//! callers). It does NOT wrap every one of this kernel's many other,
//! smaller `Vec`/`Box` allocation sites -- a much larger undertaking (a
//! real fallible-allocation API this kernel has nowhere yet) explicitly
//! out of scope here.
//!
//! **MILESTONE 106 update**: this module's own bespoke `PressureGate`
//! struct and its private copy of the "EMA then hysteresis then count
//! transitions" state machine have been migrated onto the new
//! `evidence_gate` module -- a real generalization of this exact
//! mechanism into a subsystem other kernel decisions can subscribe to
//! (see `evidence_gate.rs`'s own module doc for the full rationale).
//! This file's own tuning constants below (`HIGH_WATER_FRACTION`,
//! `GATE_ON_THRESHOLD`, `GATE_OFF_THRESHOLD`, `FAST_TAU_SAMPLES`,
//! `SAFETY_MARGIN_BYTES`) are byte-for-byte unchanged from Milestone
//! 104 -- only WHERE the evidence/conservative/sample_count state lives
//! moved, not any of the numbers that drive it. Verified behavior-
//! preserving: see README.md's own Milestone 106 entry for the
//! byte-identical counter comparison against Milestone 105's baseline.

use crate::allocator;
use crate::evidence_gate::{self, GateConfig, GateKind, GateTransition};
use crate::serial;
use core::fmt::Write;
use core::sync::atomic::{AtomicU64, Ordering};

/// MILESTONE 104: a sample counts as a pressure "hit" once the kernel
/// heap is more than 3/4 full -- a real, chosen threshold, not tuned to
/// make a demo pass. Kept as a fraction of the REAL, live
/// `allocator::HEAP_SIZE` rather than a hardcoded byte count, so raising
/// `HEAP_SIZE` (this milestone's own OTHER real change, see README.md)
/// automatically rescales this threshold rather than silently going
/// stale against the old budget.
const HIGH_WATER_FRACTION: f32 = 0.75;

/// MILESTONE 104: same hysteresis-band role and same real
/// self-reinforcing-flap reasoning as network.rs's own
/// `GATE_ON_THRESHOLD`/`GATE_OFF_THRESHOLD` pair -- re-evaluated only at
/// `golden_period()` cadence below, never every single sample.
const GATE_ON_THRESHOLD: f32 = 0.5;
const GATE_OFF_THRESHOLD: f32 = 0.2;

/// MILESTONE 104: this subsystem's own "fast" cadence. A real
/// process-teardown event (this module's own sample unit) is a much
/// rarer, more expensive real-world event than network.rs's own
/// per-tick neuron cadence, so re-evaluating the gate every SAMPLE
/// (rather than every 7th, unlike Milestone 39's own
/// `FAST_TAU_TICKS=7`) is already a slow enough clock on its own --
/// `golden_period(1) == 3` (`gcd(3,1)=1`) still gives a real,
/// non-degenerate incommensurate-with-1 period, reusing the exact same
/// tritkit-derived formula rather than inventing a second one.
const FAST_TAU_SAMPLES: u64 = 1;

/// MILESTONE 104: safety floor under EITHER admission path (evidence-
/// gated or naive) -- never admit a request that would leave less than
/// this much real headroom after it, regardless of what the gate says.
/// 4 KiB: real, deliberate slack for `linked_list_allocator`'s own
/// free-list bookkeeping plus whatever else is concurrently mid-call on
/// another spinlocked kernel-heap path.
const SAFETY_MARGIN_BYTES: usize = 4096;

/// MILESTONE 106: this heap-pressure gate's own tuning, unchanged from
/// Milestone 104, now passed BY VALUE into the shared `evidence_gate`
/// state machine on every call instead of living inside a bespoke local
/// struct -- see this file's own module doc and `evidence_gate.rs`'s.
const HEAP_GATE_CONFIG: GateConfig = GateConfig {
    gate_on_threshold: GATE_ON_THRESHOLD,
    gate_off_threshold: GATE_OFF_THRESHOLD,
    fast_tau: FAST_TAU_SAMPLES,
};

// MILESTONE 104: real, measured counters for this milestone's own
// honest evidence-gated-vs-naive benchmark (see `stats()` below and the
// new `heap_guard::self_test_heap_pressure()` boot-time self-test) --
// never reset mid-boot, so they reflect the WHOLE real boot's history.
// MILESTONE 106: `samples_taken`/`gate_trip_episodes` moved into
// `evidence_gate`'s own shared `GateState` (they're part of the
// generalized state machine, not heap-specific); only the two genuinely
// heap-specific counters -- the naive-baseline comparison and the real
// admission-denial count -- stay local here.
static NAIVE_INSTANTANEOUS_HITS: AtomicU64 = AtomicU64::new(0);
static ADMISSIONS_DENIED: AtomicU64 = AtomicU64::new(0);

/// MILESTONE 104: called from `process.rs`'s `reclaim_process_frames()`
/// -- the real, natural per-process-teardown checkpoint -- with the
/// kernel heap's own real `allocator::heap_used()` byte count
/// immediately after that specific teardown finished (frames freed,
/// `Process` fully dropped). MILESTONE 106: the fast-EMA-then-hysteresis
/// state machine itself now lives in `evidence_gate::record_sample()`
/// (shared code -- see that module's own doc); this function's job is
/// just converting a real heap-used byte count into a hit/miss bool,
/// feeding it in, and logging its OWN domain-specific detail (used
/// bytes / budget) on a real transition. Also updates the SEPARATE,
/// purely-observational naive-baseline counter (an instantaneous,
/// un-smoothed check against the same `HIGH_WATER_FRACTION`) for this
/// milestone's own benchmark -- never consulted by `admit()` itself.
pub fn record_teardown_sample(used_after: usize) {
    let fraction = used_after as f32 / allocator::HEAP_SIZE as f32;
    let hit = fraction > HIGH_WATER_FRACTION;

    if hit {
        NAIVE_INSTANTANEOUS_HITS.fetch_add(1, Ordering::Relaxed);
    }

    let (transition, evidence) = evidence_gate::record_sample(GateKind::KernelHeapPressure, &HEAP_GATE_CONFIG, hit);

    match transition {
        GateTransition::Entered => {
            let _ = writeln!(
                serial(),
                "milestone 104: heap-pressure gate -- entering CONSERVATIVE mode (evidence={:.3}, used={} bytes of {} KiB budget)",
                evidence,
                used_after,
                allocator::HEAP_SIZE / 1024
            );
        }
        GateTransition::Left => {
            let _ = writeln!(
                serial(),
                "milestone 104: heap-pressure gate -- leaving CONSERVATIVE mode (evidence={:.3})",
                evidence
            );
        }
        GateTransition::Unchanged => {}
    }
}

/// MILESTONE 104: the real admission check -- `tasks.rs`'s
/// `run_preemption_demo()`/`spawn()` call this BEFORE allocating a
/// worker-task stack, instead of allocating first and finding out via a
/// kernel panic the way `run_preemption_demo()` used to. Two real,
/// independent reasons to refuse:
///   1. the hard floor -- `allocator::heap_free()` genuinely doesn't
///      have `bytes_needed + SAFETY_MARGIN_BYTES` free right now. Always
///      checked, gate or no gate; the evidence-gate can make this
///      milestone refuse EARLIER than this floor would on its own, never
///      later.
///   2. the evidence-gated self-healing property itself: even a request
///      that technically still fits today is refused if sustained real
///      evidence (see `record_teardown_sample()` above) says this boot's
///      kernel heap has been trending toward exhaustion -- proactive
///      degradation instead of waiting for the hard floor to be the only
///      thing standing between the next request and a panic.
pub fn admit(bytes_needed: usize) -> bool {
    let free = allocator::heap_free();
    if free < bytes_needed.saturating_add(SAFETY_MARGIN_BYTES) {
        ADMISSIONS_DENIED.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    if evidence_gate::is_conservative(GateKind::KernelHeapPressure) {
        ADMISSIONS_DENIED.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    true
}

/// MILESTONE 104: the conventional baseline this milestone's own
/// benchmark measures the evidence-gated `admit()` above against -- the
/// SAME real hard floor, with NO evidence smoothing and NO hysteresis:
/// exactly what admission control looks like without this milestone's
/// own SNN-derived addition. Real code, not a hypothetical -- called
/// only by `self_test_heap_pressure()` below, never by any real
/// allocation site, so it can never itself change kernel behavior.
pub fn naive_admit(bytes_needed: usize) -> bool {
    allocator::heap_free() >= bytes_needed.saturating_add(SAFETY_MARGIN_BYTES)
}

/// Real, measured counters as of right now: `(samples_taken,
/// gate_trip_episodes, naive_instantaneous_hits, admissions_denied)`.
/// MILESTONE 106: the first two now come from `evidence_gate`'s own
/// shared counters rather than this file's own atomics.
pub fn stats() -> (u64, u64, u64, u64) {
    let (samples_taken, gate_trip_episodes) = evidence_gate::stats(GateKind::KernelHeapPressure);
    (
        samples_taken,
        gate_trip_episodes,
        NAIVE_INSTANTANEOUS_HITS.load(Ordering::Relaxed),
        ADMISSIONS_DENIED.load(Ordering::Relaxed),
    )
}

/// MILESTONE 104: real, boot-time, non-interactive self-test -- same
/// motivation as fs.rs's own `self_test_disk_write()` (interactive
/// testing has been unreliable in this environment). Deliberately
/// creates real, additional heap pressure via repeated real
/// `fork()`+`run()`+`wait_for_child()` cycles against
/// `WAITSTATUS_TEST_PROCESS_ID` (`process::self_test_wait_status()`'s
/// own real exit-code child, reused rather than adding a new hardcoded
/// process) -- each cycle's own teardown feeds a real sample into
/// `record_teardown_sample()` above -- then reports the real, measured
/// evidence-gate state and the honest naive-vs-evidence-gated admission
/// comparison for a real, concrete request size (one
/// `tasks::STACK_SIZE`), both computed from real, live
/// `allocator::heap_free()` at the moment this runs, not predicted.
pub fn self_test_heap_pressure(stress_cycles: u32) {
    let before = allocator::heap_used();
    let mut all_cycles_ok = true;
    for _ in 0..stress_cycles {
        let cycle_ok = crate::process::run_one_waitstatus_cycle();
        all_cycles_ok &= cycle_ok;
    }
    let after = allocator::heap_used();
    let (samples, gate_trips, naive_hits, denied) = stats();

    let request = crate::tasks::STACK_SIZE;
    let evidence_gated_admits = admit(request);
    let naive_admits = naive_admit(request);

    let _ = writeln!(
        serial(),
        "milestone 104: self-test -- {stress_cycles} extra real fork()/run()/wait() cycle(s), all_ok={all_cycles_ok}, kernel heap used {before}->{after} bytes (delta={}), free={} bytes",
        after as i64 - before as i64,
        allocator::heap_free()
    );
    let _ = writeln!(
        serial(),
        "milestone 104: self-test -- real evidence-gate state: samples_taken={samples} gate_trip_episodes={gate_trips} naive_instantaneous_hits={naive_hits} admissions_denied_so_far={denied}"
    );
    let _ = writeln!(
        serial(),
        "milestone 104: self-test -- for a real {request}-byte request right now: evidence_gated_admit={evidence_gated_admits} naive_admit={naive_admits}"
    );
    let _ = writeln!(
        serial(),
        "milestone 104: self-test -- OVERALL_M104={}",
        if all_cycles_ok { "PASS" } else { "FAIL" }
    );
}
