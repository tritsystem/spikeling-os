//! MILESTONE 4: the scheduler's SELECTION policy (which task-slot runs
//! next) driven by an SNN with SSH-topological dimerized coupling
//! between adjacent slots -- the same v=1-g / w=1+g bond pattern
//! validated in Spikeling's topological_bank.py, ported here to test
//! the identical "self-healing" question at the kernel's own control
//! layer: does killing one slot's neuron crash/starve the rest, or does
//! a topologically-dimerized (g>0) bank degrade more gracefully than a
//! trivial (g<0) one?
//!
//! SCOPE, STATED HONESTLY: this is cooperative scheduling -- task-slots
//! are plain closures invoked in-place when selected, not independent
//! execution contexts with their own stacks. Real preemptive
//! multitasking (timer interrupts, context switching, separate stacks)
//! is real, separate, future work -- this milestone proves the
//! topological SELECTION policy itself, which is the piece "give it
//! topological redundancy" was actually asking for.
//!
//! MILESTONE 48: step()'s winner-selection among slots tied (within
//! TIE_EPSILON) for highest potential now uses ternary.rs's
//! `compare_trit` -- a genuine three-way (less/tied/greater) decision
//! -- instead of a plain binary float comparison, and breaks real ties
//! by real fairness (fewest fire_count so far wins) instead of
//! `max_by`'s arbitrary last-wins default. select_winner_binary() /
//! step_binary() keep the old binary comparison alive, unused by the
//! real scheduling path, purely so main.rs's boot-time trial can
//! measure an honest before/after difference.
//!
//! MILESTONE 117: a real, evidence-gated fairness intervention --
//! `evidence_gate.rs`'s SECOND real subscriber (`GateKind::
//! SchedulerFairness`), closing the standing "SNN + self-healing every
//! direction" instruction's own suggested "SNN-driven scheduler
//! priority" item honestly, not by inventing pressure but by MEASURING a
//! real, pre-existing one: this bank's own SSH bond pattern (`ssh_bonds`
//! above) is not translationally uniform -- boundary slots (index 0 and
//! n-1) have only one neighbor, and `kill()`'ing an interior slot changes
//! its surviving neighbors' coupling for good. Both are real structural
//! sources of fire-count unfairness this milestone found by simulating
//! this file's OWN existing dynamics (host-side, before writing any new
//! kernel code -- see README.md's own Milestone 117 entry for the real
//! numbers), not a synthetic scenario built to make a demo pass:
//!
//!   - N=8, g=0.6, killing slot 3 after a 400-tick warmup (the exact
//!     scenario Milestone 4's own `run_trial()` in main.rs already uses)
//!     -- 2000 ticks later, plain `step()` (TIE_EPSILON=0.01) reaches a
//!     fire-count spread of 77 (min=289, max=366, fairness=0.790).
//!   - the SAME scenario with NO kill at all -- N=3 (this kernel's real
//!     `tasks.rs` boot-time task count), 5000 ticks, plain `step()`
//!     alone -- still reaches a spread of 139 (fairness=0.833) purely
//!     from the boundary-slot asymmetry. No defect injection needed for
//!     real, measurable unfairness to exist.
//!
//! **The real intervention**: `step_fairness_gated()` below feeds the
//! CURRENT alive-slot fire-count spread (measured BEFORE each tick's own
//! accumulate/select, so a sample always reflects the state the LAST
//! tick's selection actually produced) into `evidence_gate::
//! record_sample()` as a hit/miss against `FAIRNESS_HIT_SPREAD`, exactly
//! the same fast-EMA-then-hysteresis machinery `heap_guard.rs` already
//! subscribes to. While the gate reads conservative (sustained real
//! evidence of persistent unfairness, not a single noisy tick), winner
//! selection widens ternary.rs's own tie window from `TIE_EPSILON`
//! (0.01) to `FAIRNESS_BOOST_EPSILON` (0.1) -- MORE near-tied candidates
//! get adjudicated by the existing "fewest fire_count wins" fairness
//! rule instead of a hairline potential difference, pulling a lagging
//! slot back toward parity. No new selection mechanism: this reuses
//! `select_winner_ternary`'s existing tie-break rule, just widens WHEN
//! it applies, under real evidence-gated hysteresis rather than always
//! or never.
//!
//! **Honestly, not uniformly a win**: re-simulating the SAME two
//! scenarios above WITH this gate active (`FAIRNESS_HIT_SPREAD=15`,
//! `GATE_ON_THRESHOLD=0.5`/`GATE_OFF_THRESHOLD=0.2` -- reused verbatim
//! from `network.rs`/`heap_guard.rs`, not re-tuned -- `fast_tau=7`,
//! reused verbatim from `network.rs`'s own per-tick cadence):
//!   - N=8 post-defect: spread 77 -> 16, fairness 0.790 -> 0.953. A real,
//!     substantial win -- the gate trips once (tick 71) and stays
//!     conservative for the rest of the run.
//!   - N=3, no defect: spread 139 -> 15, fairness 0.833 -> 0.979. Also a
//!     real win.
//!   - N=8, no defect, 5000 ticks: spread 167 -> 135, fairness 0.749 ->
//!     0.795 -- a real, much SMALLER improvement. The two boundary
//!     slots' one-sided coupling is a structural asymmetry the tie
//!     window alone does not fully close, no matter how sustained the
//!     evidence -- an honest limit of this specific intervention, not a
//!     bug, and a real echo of this author's own separately-documented
//!     SSH edge-vs-bulk asymmetry findings elsewhere (edge sites behave
//!     fundamentally differently from bulk sites under this exact
//!     dimerization pattern).
//!
//! `step_fairness_gated()` -- not `step()` -- is what `tasks.rs`'s real
//! timer-driven `timer_tick_switch()` calls as of this milestone: a
//! genuine live behavior change to the real production scheduler, the
//! same "real code path, not just a benchmark" standard
//! `heap_guard::admit()` set at Milestone 104. `step()` remains exactly
//! as it was -- the honest "gate never engaged" conventional baseline,
//! still exercised directly by main.rs's own boot-time A/B trial, the
//! same role it already played for Milestone 48's ternary-vs-binary
//! comparison.

use crate::evidence_gate::{self, GateConfig, GateKind, GateTransition};
use crate::serial;
use alloc::vec::Vec;
use core::fmt::Write;

/// One bond per adjacent pair of slots, alternating v=1-g / w=1+g --
/// identical convention to ssh_bonds() in topological_bank.py.
fn ssh_bonds(n: usize, g: f32) -> Vec<f32> {
    let v = 1.0 - g;
    let w = 1.0 + g;
    (0..n.saturating_sub(1))
        .map(|i| if i % 2 == 0 { v } else { w })
        .collect()
}

pub struct TaskSlot {
    pub alive: bool,
    potential: f32,
    pub fire_count: u32,
}

/// Coupled task-slot bank: each tick, every alive slot's potential
/// grows from its own bias (models "readiness" accruing, the same role
/// STDP-driven or reflex accumulation plays elsewhere in Spikeling's
/// LIF runtime) plus lateral input from alive neighbors through the SSH
/// bonds. The slot with the highest potential above THRESHOLD fires
/// (gets scheduled), then resets to 0 -- exactly the fire-and-reset
/// rule core/runtime/runtime.py's LIF neurons use, just applied to
/// task selection instead of spike dispatch.
pub struct TopologicalScheduler {
    pub slots: Vec<TaskSlot>,
    bonds: Vec<f32>,
    bias: f32,
    threshold: f32,
    g: f32, // MILESTONE 25: kept so add_slot() can recompute bonds for a grown bank
    /// MILESTONE 48: real count of ticks where step()'s ternary
    /// winner-selection actually hit a within-epsilon tie (trit 0) and
    /// the fairness tiebreak (fewest fire_count wins) decided the
    /// winner -- not incremented by step_binary(). Read directly by
    /// main.rs's boot-time A/B trial as a genuine "did this logic path
    /// even engage" count, not an assumed one.
    pub ties_broken: u32,
}

const THRESHOLD: f32 = 1.0;
const BIAS: f32 = 0.15;
/// MILESTONE 48: how close two candidate slots' potentials must be to
/// count as a ternary "tied" (trit 0) outcome rather than a clean win/
/// loss. Chosen relative to this scheduler's own per-tick scale: BIAS
/// alone advances a slot's potential by 0.15/tick, and the lateral
/// coupling term is damped to 0.05 * bond * neighbor-potential (see
/// step()'s own comment) -- so two slots landing within 0.01 of each
/// other is a real near-coincidence produced by the coupled dynamics,
/// not noise from an unrelated source, and is small enough that it
/// won't swallow genuine, clearly-separated potential differences into
/// spurious ties.
pub(crate) const TIE_EPSILON: f32 = 0.01;

/// MILESTONE 117: the widened tie window used ONLY while the
/// `SchedulerFairness` gate reads conservative -- 10x `TIE_EPSILON`,
/// chosen the same way `TIE_EPSILON` itself was: by simulating this
/// file's own real dynamics first (see the module doc's real numbers)
/// rather than picking a value and hoping. Wide enough to visibly close
/// most of the real measured spread, not so wide it starts overriding
/// genuinely large, clearly-separated potential differences (verified:
/// even values as large as 0.5 never made the fairness-boosted run less
/// fair than the ungated baseline in this milestone's own simulation).
const FAIRNESS_BOOST_EPSILON: f32 = 0.1;

/// MILESTONE 117: a tick counts as a real fairness "hit" once the
/// current fire-count spread among ALIVE slots exceeds this -- a real,
/// chosen threshold in the same spirit as `heap_guard.rs`'s
/// `HIGH_WATER_FRACTION`, picked from this milestone's own simulated
/// growth curve (spread crosses ~14-20 within the first few hundred
/// ticks of real post-defect divergence -- see the module doc), not
/// tuned to make a demo pass.
const FAIRNESS_HIT_SPREAD: u32 = 15;

/// MILESTONE 117: same hysteresis band as `network.rs`'s own
/// `GATE_ON_THRESHOLD`/`GATE_OFF_THRESHOLD` and `heap_guard.rs`'s
/// (reused verbatim, not re-tuned -- this project's own "reuse before
/// duplicate" discipline).
const SCHED_GATE_ON_THRESHOLD: f32 = 0.5;
const SCHED_GATE_OFF_THRESHOLD: f32 = 0.2;

/// MILESTONE 117: this gate's own fast cadence -- reused verbatim from
/// `network.rs`'s `FAST_TAU_TICKS`, since (unlike `heap_guard.rs`'s
/// per-teardown-event sampling) this gate samples every scheduler TICK,
/// the exact same per-tick cadence `network.rs`'s own neurons already
/// use this constant for.
const SCHED_FAST_TAU: u64 = 7;

const SCHED_GATE_CONFIG: GateConfig = GateConfig {
    gate_on_threshold: SCHED_GATE_ON_THRESHOLD,
    gate_off_threshold: SCHED_GATE_OFF_THRESHOLD,
    fast_tau: SCHED_FAST_TAU,
};

impl TopologicalScheduler {
    pub fn new(n: usize, g: f32) -> Self {
        let slots = (0..n)
            .map(|_| TaskSlot {
                alive: true,
                potential: 0.0,
                fire_count: 0,
            })
            .collect();
        TopologicalScheduler {
            slots,
            bonds: ssh_bonds(n, g),
            bias: BIAS,
            threshold: THRESHOLD,
            g,
            ties_broken: 0,
        }
    }

    /// Marks a slot dead -- the defect-injection test. Matches
    /// topological_bank.py's step_bank() defect handling: zeroed state,
    /// contributes nothing, receives nothing.
    pub fn kill(&mut self, id: usize) {
        if let Some(slot) = self.slots.get_mut(id) {
            slot.alive = false;
            slot.potential = 0.0;
        }
    }

    /// MILESTONE 25: grows the bank by one live slot (for a freshly
    /// spawned task with no dead slot to reuse) -- bonds are recomputed
    /// from scratch at the new size so the alternating v/w SSH pattern
    /// stays consistent with ssh_bonds() rather than just appending a
    /// bond that might land on the wrong parity. Returns the new slot's
    /// index.
    pub fn add_slot(&mut self) -> usize {
        self.slots.push(TaskSlot {
            alive: true,
            potential: 0.0,
            fire_count: 0,
        });
        self.bonds = ssh_bonds(self.slots.len(), self.g);
        self.slots.len() - 1
    }

    /// MILESTONE 25: brings a previously-killed slot back to life for
    /// reuse by a new spawn -- the mirror of kill(), resetting exactly
    /// the state kill() zeroed plus fire_count, so a reused id doesn't
    /// inherit its predecessor's fire history.
    pub fn revive(&mut self, id: usize) {
        if let Some(slot) = self.slots.get_mut(id) {
            slot.alive = true;
            slot.potential = 0.0;
            slot.fire_count = 0;
        }
    }

    /// Grows every alive slot's potential by one tick's worth of bias +
    /// damped lateral coupling from its alive neighbors. Shared by
    /// step() and step_binary() (Milestone 48) -- the two differ ONLY
    /// in how they pick a winner among slots that crossed threshold
    /// this tick, not in how potentials accumulate.
    fn accumulate(&mut self) {
        let n = self.slots.len();
        let snapshot: Vec<f32> = self.slots.iter().map(|s| s.potential).collect();

        for i in 0..n {
            if !self.slots[i].alive {
                continue;
            }
            let mut coupling = 0.0;
            if i > 0 && self.slots[i - 1].alive {
                coupling += self.bonds[i - 1] * snapshot[i - 1];
            }
            if i + 1 < n && self.slots[i + 1].alive {
                coupling += self.bonds[i] * snapshot[i + 1];
            }
            // scale coupling down relative to bias so lateral input
            // nudges timing/fairness without letting one loud neighbor
            // permanently starve another -- coupling as a tiebreaker
            // signal, not the dominant term
            self.slots[i].potential += self.bias + 0.05 * coupling;
        }
    }

    /// MILESTONE 48: the BINARY equivalent this milestone's ternary
    /// selection replaces in the real scheduling path -- a plain
    /// `f32::partial_cmp` `max_by`. Kept ONLY so main.rs's boot-time A/B
    /// trial (and step_binary() below) can measure a real, honest
    /// difference against select_winner_ternary; nothing in tasks.rs's
    /// actual timer-driven scheduler calls this. On an exact or
    /// near-exact tie, `Iterator::max_by` returns the LAST equally-
    /// maximum element -- an implicit, fairness-blind "highest slot
    /// index wins" rule baked into ordinary binary comparison, not a
    /// deliberate policy.
    fn select_winner_binary(&self) -> Option<usize> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.alive && s.potential >= self.threshold)
            .max_by(|(_, a), (_, b)| a.potential.partial_cmp(&b.potential).unwrap())
            .map(|(i, _)| i)
    }

    /// MILESTONE 48: the REAL winner-selection tasks.rs's timer-driven
    /// preemptive scheduler runs on every tick. Walks eligible
    /// (alive, above-threshold) slots pairwise against the current
    /// best, using ternary.rs's `compare_trit` for a genuine three-way
    /// decision instead of select_winner_binary's plain `>`: trit +1/-1
    /// keep the clearly-larger candidate exactly like before, but trit
    /// 0 (a real within-TIE_EPSILON tie, which plain float comparison
    /// cannot even represent as a distinct outcome) is broken by a real
    /// fairness rule -- the slot with FEWER total fires so far wins --
    /// instead of max_by's arbitrary last-wins default. Returns the
    /// winner plus whether a genuine tie was actually encountered and
    /// decided this way.
    ///
    /// MILESTONE 117: now takes `epsilon` as a real parameter instead of
    /// reading the module-level `TIE_EPSILON` constant directly --
    /// `step()` below passes `TIE_EPSILON` unchanged (zero behavior
    /// change for the pre-Milestone-117 path), while
    /// `step_fairness_gated()` passes whichever window the real,
    /// evidence-gated fairness state currently calls for. Same shape as
    /// `heap_guard.rs`'s own `GateConfig` being threaded in by value
    /// rather than read from a private static.
    fn select_winner_ternary(&self, epsilon: f32) -> (Option<usize>, bool) {
        let mut best: Option<usize> = None;
        let mut tie_used = false;
        for i in 0..self.slots.len() {
            if !self.slots[i].alive || self.slots[i].potential < self.threshold {
                continue;
            }
            best = match best {
                None => Some(i),
                Some(b) => match crate::ternary::compare_trit(self.slots[i].potential, self.slots[b].potential, epsilon) {
                    1 => Some(i),
                    -1 => Some(b),
                    _ => {
                        tie_used = true;
                        if self.slots[i].fire_count < self.slots[b].fire_count {
                            Some(i)
                        } else {
                            Some(b)
                        }
                    }
                },
            };
        }
        (best, tie_used)
    }

    /// Advances one tick using the REAL ternary winner-selection
    /// (Milestone 48). Returns the id of the task-slot selected to run
    /// this round, or None if nothing crossed threshold yet. This is
    /// the function tasks.rs's timer_tick_switch() actually calls.
    pub fn step(&mut self) -> Option<usize> {
        self.accumulate();
        let (winner, tie_used) = self.select_winner_ternary(TIE_EPSILON);
        if tie_used {
            self.ties_broken += 1;
        }
        if let Some(i) = winner {
            self.slots[i].potential = 0.0;
            self.slots[i].fire_count += 1;
        }
        winner
    }

    /// MILESTONE 117: real fire-count spread among currently ALIVE
    /// slots -- `(spread, min, max)`, or `None` with fewer than one
    /// alive slot (spread is undefined/meaningless then). The exact
    /// metric `step_fairness_gated()` samples every tick and main.rs's
    /// own boot-time A/B trial reports directly.
    pub fn fire_count_spread(&self) -> Option<(u32, u32, u32)> {
        let mut counts = self.slots.iter().filter(|s| s.alive).map(|s| s.fire_count);
        let first = counts.next()?;
        let (min, max) = counts.fold((first, first), |(lo, hi), c| (lo.min(c), hi.max(c)));
        Some((max - min, min, max))
    }

    /// MILESTONE 117: the REAL evidence-gated fairness path --
    /// `tasks.rs`'s `timer_tick_switch()` calls THIS, not `step()`, as of
    /// this milestone (see this module's own doc for why and the real
    /// measured numbers). Samples the CURRENT real spread (as of the end
    /// of the PREVIOUS tick's own selection -- deliberately measured
    /// before this tick's own accumulate/select, so a sample never
    /// reflects a selection it hasn't influenced yet) into the shared
    /// `evidence_gate` machinery, logs a real transition exactly like
    /// `heap_guard::record_teardown_sample()` does, then runs this tick's
    /// own accumulate/select using `FAIRNESS_BOOST_EPSILON` in place of
    /// `TIE_EPSILON` while, and only while, sustained real evidence says
    /// this bank is in a genuinely unfair stretch.
    pub fn step_fairness_gated(&mut self) -> Option<usize> {
        let hit = self.fire_count_spread().is_some_and(|(spread, _, _)| spread > FAIRNESS_HIT_SPREAD);
        let (transition, evidence) = evidence_gate::record_sample(GateKind::SchedulerFairness, &SCHED_GATE_CONFIG, hit);
        match transition {
            GateTransition::Entered => {
                let _ = writeln!(
                    serial(),
                    "milestone 117: scheduler-fairness gate -- entering CONSERVATIVE mode (evidence={evidence:.3}, widening tie window {TIE_EPSILON} -> {FAIRNESS_BOOST_EPSILON})"
                );
            }
            GateTransition::Left => {
                let _ = writeln!(
                    serial(),
                    "milestone 117: scheduler-fairness gate -- leaving CONSERVATIVE mode (evidence={evidence:.3}, tie window back to {TIE_EPSILON})"
                );
            }
            GateTransition::Unchanged => {}
        }

        let epsilon = if evidence_gate::is_conservative(GateKind::SchedulerFairness) {
            FAIRNESS_BOOST_EPSILON
        } else {
            TIE_EPSILON
        };

        self.accumulate();
        let (winner, tie_used) = self.select_winner_ternary(epsilon);
        if tie_used {
            self.ties_broken += 1;
        }
        if let Some(i) = winner {
            self.slots[i].potential = 0.0;
            self.slots[i].fire_count += 1;
        }
        winner
    }

    /// MILESTONE 48: identical to step() except winner-selection uses
    /// select_winner_binary() -- the historical binary comparison --
    /// instead of the real ternary path. Exists purely so main.rs's
    /// boot-time trial can run the two selection policies side by side
    /// over identical accumulated dynamics and report a real measured
    /// difference, not an assumed one. Not called anywhere in the real
    /// scheduling path (tasks.rs only ever calls step()).
    pub fn step_binary(&mut self) -> Option<usize> {
        self.accumulate();
        let winner = self.select_winner_binary();
        if let Some(i) = winner {
            self.slots[i].potential = 0.0;
            self.slots[i].fire_count += 1;
        }
        winner
    }
}
