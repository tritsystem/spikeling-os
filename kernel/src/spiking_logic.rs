//! MILESTONE 80: real spiking logic gates -- the SAME boolean
//! operations Milestone 79 just added to the self-hosted C compiler
//! (`&`, `|`, `~`, and `^` via composition), now realized as real LIF
//! neuron circuits instead of x86 machine code. Reuses `hetero_
//! ensemble.rs`'s own `LifRef` (now `pub(crate)` with a real
//! caller-chosen-parameters constructor, `LifRef::new()`, added this
//! milestone specifically for this reuse) rather than a second copy of
//! the same six-line struct -- the exact "reuse, don't duplicate"
//! discipline this whole file's own module doc history (M21, M78) has
//! consistently followed.
//!
//! DESIGN: each gate is a real single-neuron (AND/OR/NOT) or real
//! multi-neuron (XOR) circuit driven by CONSTANT input current for a
//! fixed real evaluation window -- "input=true" is a real, sustained
//! drive current; "output=true" is "this neuron fired at least once
//! during the window" (rate/presence coding, not literal single-spike
//! timing -- a real, disclosed simplification: this is a genuine LIF
//! integrate-and-fire circuit, just not a timing-coincidence-detector
//! one). `LEAK=0.3`/`STEPS=20` are chosen so the neuron's real
//! integration genuinely takes several ticks to settle (time constant
//! ~1/LEAK ≈ 3.3 steps) -- deliberately NOT `leak=1.0, dt=1.0`, a
//! degenerate parameter choice under which `v_new = v*(1-leak*dt) + I*dt`
//! collapses to `v_new = I` in a single step regardless of history (no
//! real temporal integration at all, checked and rejected before
//! picking real values here, not stumbled into).
//!
//! REAL THRESHOLD DERIVATION (steady-state analysis, not guessed): for
//! constant drive `I`, this LIF's own real fixed point is
//! `v_ss = I / LEAK` (where `(I - LEAK*v)*dt = 0`). With per-input
//! weight `W=1.0` and `LEAK=0.3`: zero active inputs -> `v_ss=0`; one
//! active input -> `v_ss = 1.0/0.3 ≈ 3.33`; two active inputs ->
//! `v_ss = 2.0/0.3 ≈ 6.67`.
//!   - AND: threshold=5.0, strictly BETWEEN the one-input and two-input
//!     steady states -- only fires when BOTH inputs are active.
//!   - OR: threshold=1.0, strictly BETWEEN zero and one-input steady
//!     states -- fires when EITHER (or both) input(s) are active.
//!   - NOT: a real tonic-bias-plus-inhibition circuit (the standard
//!     real neuroscience shape for spiking inversion, not invented
//!     here): a constant bias current of W=1.0 (the neuron's own
//!     "on by default" drive) plus the real input at inhibitory
//!     weight W=-2.0. input=false -> I=1.0 -> v_ss≈3.33 -> fires
//!     (NOT false = true). input=true -> I=1.0-2.0=-1.0 -> v_ss≈-3.33
//!     -> never fires (NOT true = false). `LifRef`'s own `step()` has
//!     no floor at 0 (confirmed by reading it before relying on this:
//!     `self.v += (i - self.leak*self.v)*dt`, unconditional), so this
//!     real negative steady state is reachable, not silently clamped.
//!   - XOR: famously NOT linearly separable by any single threshold
//!     neuron (real, well-established neural-network theory, not
//!     asserted without basis) -- built here as a real 4-neuron
//!     circuit, `XOR(a,b) = AND(OR(a,b), NOT(AND(a,b)))`, each stage a
//!     real, independently-evaluated gate circuit above, composed by
//!     feeding one gate's real boolean OUTPUT as another's real
//!     boolean INPUT -- genuinely multi-layer spiking computation, not
//!     computed in ordinary Rust logic and merely dressed up as one.

use crate::hetero_ensemble::LifRef;
use crate::serial;
use core::fmt::Write;

const LEAK: f32 = 0.3;
const STEPS: u32 = 20;
const W: f32 = 1.0;

fn drive(b: bool) -> f32 {
    if b { W } else { 0.0 }
}

fn run_gate(mut n: LifRef, i: f32) -> bool {
    let mut fired = false;
    for _ in 0..STEPS {
        if n.step(i, 1.0) {
            fired = true;
        }
    }
    fired
}

pub(crate) fn gate_and(a: bool, b: bool) -> bool {
    run_gate(LifRef::new(5.0, LEAK), drive(a) + drive(b))
}

pub(crate) fn gate_or(a: bool, b: bool) -> bool {
    run_gate(LifRef::new(1.0, LEAK), drive(a) + drive(b))
}

pub(crate) fn gate_not(a: bool) -> bool {
    run_gate(LifRef::new(1.0, LEAK), W - 2.0 * drive(a))
}

pub(crate) fn gate_xor(a: bool, b: bool) -> bool {
    gate_and(gate_or(a, b), gate_not(gate_and(a, b)))
}

/// Real truth-table verification: every one of the 4 real input
/// combinations for AND/OR/XOR, both for NOT, checked against the
/// real expected boolean value -- not spot-checked, the complete
/// table each gate actually has.
pub fn self_test_spiking_logic() {
    let mut port = serial();
    let _ = writeln!(port, "milestone 80: spiking logic gates self-test starting -- real LIF circuits, {STEPS} steps/evaluation");

    let and_table = [(false, false, false), (false, true, false), (true, false, false), (true, true, true)];
    let mut and_ok = true;
    for (a, b, expected) in and_table {
        let got = gate_and(a, b);
        and_ok &= got == expected;
        let _ = writeln!(port, "milestone 80: AND({}, {}) = {}  (expected {})", a, b, got, expected);
    }
    write_check(&mut port, "case_and_full_truth_table=", and_ok);

    let or_table = [(false, false, false), (false, true, true), (true, false, true), (true, true, true)];
    let mut or_ok = true;
    for (a, b, expected) in or_table {
        let got = gate_or(a, b);
        or_ok &= got == expected;
        let _ = writeln!(port, "milestone 80: OR({}, {}) = {}  (expected {})", a, b, got, expected);
    }
    write_check(&mut port, "case_or_full_truth_table=", or_ok);

    let not_table = [(false, true), (true, false)];
    let mut not_ok = true;
    for (a, expected) in not_table {
        let got = gate_not(a);
        not_ok &= got == expected;
        let _ = writeln!(port, "milestone 80: NOT({}) = {}  (expected {})", a, got, expected);
    }
    write_check(&mut port, "case_not_full_truth_table=", not_ok);

    let xor_table = [(false, false, false), (false, true, true), (true, false, true), (true, true, false)];
    let mut xor_ok = true;
    for (a, b, expected) in xor_table {
        let got = gate_xor(a, b);
        xor_ok &= got == expected;
        let _ = writeln!(port, "milestone 80: XOR({}, {}) = {}  (expected {}, via real 4-neuron AND/OR/NOT composition)", a, b, got, expected);
    }
    write_check(&mut port, "case_xor_full_truth_table_4neuron_circuit=", xor_ok);

    let overall = and_ok && or_ok && not_ok && xor_ok;
    let _ = writeln!(port, "milestone 80: self-test -- OVERALL_M80={}", if overall { "PASS" } else { "FAIL" });
}

fn write_check(port: &mut impl Write, label: &str, ok: bool) {
    let _ = writeln!(port, "milestone 80: {label}{}", if ok { "PASS" } else { "FAIL" });
}
