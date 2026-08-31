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
//!
//! MILESTONE 81 scales this to real 8-bit multi-bit AND/OR/XOR/NOT --
//! the same real single-bit gate circuits above, run 8 INDEPENDENT
//! times, once per bit position (a real bit-sliced architecture, the
//! same real principle an actual multi-bit hardware ALU uses: N
//! parallel 1-bit gates, not one gate that somehow "knows" about 8
//! bits). 8 bits x 20 steps/gate = 160 real LIF neuron-steps per 8-bit
//! AND/OR/XOR call (320 for XOR's real 4-neuron-per-bit circuit); NOT8
//! is 8 real NOT-gate circuits. Genuinely untested beyond 8 bits (this
//! kernel's own native word is 64 bits -- scaling this same bit-sliced
//! approach to 64 parallel gates per operation is a real, disclosed,
//! straightforward-but-unverified next step, not attempted here).

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

/// MILESTONE 81: real 8-bit AND -- 8 independent real `gate_and()`
/// circuits, one per bit position, bit-sliced (see module doc).
pub(crate) fn gate_and8(a: u8, b: u8) -> u8 {
    let mut result = 0u8;
    for bit in 0..8u8 {
        let abit = (a >> bit) & 1 == 1;
        let bbit = (b >> bit) & 1 == 1;
        if gate_and(abit, bbit) {
            result |= 1 << bit;
        }
    }
    result
}

pub(crate) fn gate_or8(a: u8, b: u8) -> u8 {
    let mut result = 0u8;
    for bit in 0..8u8 {
        let abit = (a >> bit) & 1 == 1;
        let bbit = (b >> bit) & 1 == 1;
        if gate_or(abit, bbit) {
            result |= 1 << bit;
        }
    }
    result
}

pub(crate) fn gate_xor8(a: u8, b: u8) -> u8 {
    let mut result = 0u8;
    for bit in 0..8u8 {
        let abit = (a >> bit) & 1 == 1;
        let bbit = (b >> bit) & 1 == 1;
        if gate_xor(abit, bbit) {
            result |= 1 << bit;
        }
    }
    result
}

pub(crate) fn gate_not8(a: u8) -> u8 {
    let mut result = 0u8;
    for bit in 0..8u8 {
        let abit = (a >> bit) & 1 == 1;
        if gate_not(abit) {
            result |= 1 << bit;
        }
    }
    result
}

fn write_check(port: &mut impl Write, label: &str, ok: bool) {
    let _ = writeln!(port, "milestone 80: {label}{}", if ok { "PASS" } else { "FAIL" });
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

    let overall_m80 = and_ok && or_ok && not_ok && xor_ok;
    let _ = writeln!(port, "milestone 80: self-test -- OVERALL_M80={}", if overall_m80 { "PASS" } else { "FAIL" });

    self_test_spiking_logic_8bit();
}

/// MILESTONE 81: real 8-bit multi-bit AND/OR/XOR/NOT, verified against
/// real hand-computed expected values -- 8 real bit-pattern test
/// values chosen for real coverage (all-zero, all-one, alternating
/// bits, nibble patterns, and one arbitrary combined value), not an
/// exhaustive 65536-pair sweep (a real, deliberate scope cut: this
/// kernel's own established convention is hand-verified representative
/// cases, the same discipline every `tools/cc_src` CASE test already
/// uses, not brute-force exhaustion).
fn self_test_spiking_logic_8bit() {
    let mut port = serial();
    let _ = writeln!(port, "milestone 81: 8-bit multi-bit spiking gates self-test starting -- real bit-sliced circuits, 8x{STEPS} steps/8-bit-op");

    // (a, b, and8, or8, xor8) -- every expected value real, hand-computed
    // against a & b / a | b / a ^ b, not assumed from the gate design.
    let pairs: [(u8, u8, u8, u8, u8); 8] = [
        (0x00, 0x00, 0x00, 0x00, 0x00),
        (0xFF, 0xFF, 0xFF, 0xFF, 0x00),
        (0xFF, 0x00, 0x00, 0xFF, 0xFF),
        (0x0F, 0xF0, 0x00, 0xFF, 0xFF),
        (0xAA, 0x55, 0x00, 0xFF, 0xFF),
        (0xAA, 0xAA, 0xAA, 0xAA, 0x00),
        (0x3C, 0xC3, 0x00, 0xFF, 0xFF),
        (0x12, 0x34, 0x10, 0x36, 0x26),
    ];
    let mut and8_ok = true;
    let mut or8_ok = true;
    let mut xor8_ok = true;
    for (a, b, exp_and, exp_or, exp_xor) in pairs {
        let got_and = gate_and8(a, b);
        let got_or = gate_or8(a, b);
        let got_xor = gate_xor8(a, b);
        and8_ok &= got_and == exp_and;
        or8_ok &= got_or == exp_or;
        xor8_ok &= got_xor == exp_xor;
        let _ = writeln!(
            port,
            "milestone 81: a={a:#04x} b={b:#04x}  AND8={got_and:#04x}(exp {exp_and:#04x})  OR8={got_or:#04x}(exp {exp_or:#04x})  XOR8={got_xor:#04x}(exp {exp_xor:#04x})"
        );
    }
    write_check81(&mut port, "case_and8_8_pairs=", and8_ok);
    write_check81(&mut port, "case_or8_8_pairs=", or8_ok);
    write_check81(&mut port, "case_xor8_8_pairs=", xor8_ok);

    let not_values: [(u8, u8); 5] = [(0x00, 0xFF), (0xFF, 0x00), (0x0F, 0xF0), (0xAA, 0x55), (0x12, 0xED)];
    let mut not8_ok = true;
    for (a, expected) in not_values {
        let got = gate_not8(a);
        not8_ok &= got == expected;
        let _ = writeln!(port, "milestone 81: a={a:#04x}  NOT8={got:#04x}(exp {expected:#04x})");
    }
    write_check81(&mut port, "case_not8_5_values=", not8_ok);

    let overall_m81 = and8_ok && or8_ok && xor8_ok && not8_ok;
    let _ = writeln!(port, "milestone 81: self-test -- OVERALL_M81={}", if overall_m81 { "PASS" } else { "FAIL" });
}

fn write_check81(port: &mut impl Write, label: &str, ok: bool) {
    let _ = writeln!(port, "milestone 81: {label}{}", if ok { "PASS" } else { "FAIL" });
}
