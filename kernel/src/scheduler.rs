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

use alloc::vec::Vec;

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
    fn select_winner_ternary(&self) -> (Option<usize>, bool) {
        let mut best: Option<usize> = None;
        let mut tie_used = false;
        for i in 0..self.slots.len() {
            if !self.slots[i].alive || self.slots[i].potential < self.threshold {
                continue;
            }
            best = match best {
                None => Some(i),
                Some(b) => match crate::ternary::compare_trit(self.slots[i].potential, self.slots[b].potential, TIE_EPSILON) {
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
        let (winner, tie_used) = self.select_winner_ternary();
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
