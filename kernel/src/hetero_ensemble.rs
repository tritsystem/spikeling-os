//! MILESTONE 77: a real heterogeneous neuron-type ensemble applied to
//! this kernel's own disk I/O -- direct kernel-side application of
//! Spikeling's own research finding
//! (Spikeling/compute_ontology/heterogeneous_ontology_test.py,
//! 2026-08-29): a pre-registered test of the architectural claim that
//! resources differing in KIND need handling designed for their
//! differences, rather than being forced through one homogenized
//! abstraction. Applied there to four independently-verified Spikeling
//! neuron models (LIF -> magnitude, Izhikevich -> burst, AdEx ->
//! repetition/fatigue, Resonator -> frequency, per
//! Spikeling/tribe/NEURON_TYPES.md), it found a HETEROGENEOUS typed
//! ensemble detected 100% of a combined 4-anomaly-type synthetic test
//! set, vs 50% for a HOMOGENEOUS all-LIF ensemble of the same neuron
//! count. This milestone re-runs that SAME hypothesis against a
//! genuinely different real system: this kernel's own ATA PIO disk
//! driver (ata.rs), not a synthetic Python signal -- self-contained,
//! touching neither network.rs's GenericNetwork (Milestones 9-56's own
//! LIF-only, heavily-verified engine) nor ata.rs itself.
//!
//! DISCLOSED ADAPTATIONS from the Python original (same honesty
//! discipline as every other milestone in this README):
//!
//! 1. No fine-grained wall clock exists in this freestanding kernel --
//!    the only hardware timer wired up (interrupts.rs) is the PIT at
//!    ~18.2Hz, far too coarse to resolve an 8-cycle-per-window
//!    oscillation the way the Python file's real seconds/Hz could.
//!    "Time" here is STEPS, not seconds: one step is one iteration of
//!    this self-test's own fixed-rate loop. "Frequency" is therefore
//!    defined in steps-per-cycle, not Hz. The neuron equations
//!    themselves (below) are UNCHANGED from the literature/Python
//!    reference; only the definition of one simulated time unit
//!    changes -- the same kind of substitution the Python file's own
//!    DT_FOR_KIND table already discloses varies per neuron kind.
//!
//! 2. The anomaly signal is REAL, not synthetic: each step, the RDTSC
//!    cycle cost of zero or more real ata::read_sector() PIO operations
//!    (issued against a dedicated scratch LBA range, SCRATCH_LBA_BASE
//!    below -- 600..604, chosen clear of fs.rs's LBA 1..66 and
//!    ata.rs's own Milestone 66 self-test at LBA 500) IS the raw drive
//!    value fed to every neuron that step, rescaled per-type (same
//!    SCALE-table principle as the Python file). The four anomaly
//!    SHAPES (magnitude/burst/repetition/frequency) are produced by
//!    controlling WHEN real disk operations are issued, not by
//!    inventing the resulting numbers -- see the four workload
//!    functions below. Reads only; SCRATCH_LBA_BASE is never written,
//!    so this self-test cannot corrupt anything fs.rs or ata.rs's own
//!    persistence format depend on.
//!
//! 3. Smaller trial/population counts than the Python study (which
//!    spent real minutes per condition running off-kernel): this
//!    self-test must complete during ONE real QEMU boot in a bounded
//!    time, and every drive sample here costs a real PIO round trip,
//!    not a Python float multiply. N_PER_TYPE/N_TRIALS below are cut
//!    roughly 4-8x from the original. Real, independently-seeded
//!    trials either way -- just fewer of them; see this milestone's
//!    own Status entry for the honest cost of that in result
//!    granularity.
//!
//! 4. RDTSC gives real elapsed CPU cycles, not calibrated wall-clock
//!    seconds (TCG's virtual clock rate isn't a documented constant to
//!    rely on) -- used here strictly as a RELATIVE magnitude (elevated
//!    vs baseline), never converted to a time unit, which is all this
//!    test needs.

use crate::ata;
use crate::serial;
use alloc::vec::Vec;
use core::fmt::Write;

// ---------------------------------------------------------------------
// A tiny, self-contained xorshift64* PRNG -- this kernel has no `rand`
// dependency (see Cargo.toml: bootloader_api/uart_16550/x86_64/
// linked_list_allocator/lazy_static/pic8259/spin/pc-keyboard/
// noto-sans-mono-bitmap/libm only), and pulling one in for a single
// self-test isn't warranted. Deterministic and seedable, same
// reproducibility property the Python study's `random.Random(seed)`
// calls relied on.
// ---------------------------------------------------------------------
pub(crate) struct Xorshift64(u64);

impl Xorshift64 {
    pub(crate) fn new(seed: u64) -> Self {
        Xorshift64(if seed == 0 { 0xdead_beef_cafe_babe } else { seed })
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// Uniform float in [lo, hi).
    fn uniform(&mut self, lo: f32, hi: f32) -> f32 {
        let frac = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32; // 24 bits -> [0,1)
        lo + frac * (hi - lo)
    }
    fn pick3(&mut self) -> u8 {
        (self.next_u64() % 3) as u8
    }
}

// ---------------------------------------------------------------------
// Four neuron dynamics, faithfully ported from
// Spikeling/pyspike_neuron_models.py (Izhikevich/AdEx/LIFReference) and
// Spikeling/compute_ontology/heterogeneous_ontology_test.py's own
// ResonatorNeuron (itself a port of core/runtime/runtime.py's real
// ResonatorState.step()) -- same equations, same substep-integration
// discipline, only the surrounding "what is dt" convention changes (see
// adaptation #1 above).
// ---------------------------------------------------------------------

pub(crate) struct LifRef {
    v: f32,
    threshold: f32,
    leak: f32,
}

impl LifRef {
    /// MILESTONE 80: real constructor with CALLER-CHOSEN threshold/leak,
    /// not `make_neuron()`'s own fixed-range randomization -- needed for
    /// `spiking_logic.rs`'s real gate circuits, whose real per-gate
    /// steady-state math (see that module's own doc comment) requires
    /// specific, deliberately-calibrated values Milestone 77's own
    /// `rng.uniform(0.8, 1.3)` range was never meant to produce. Fields
    /// stay private; this is the one real, controlled way to set them
    /// from outside this file, same "add the real capability, don't
    /// duplicate the struct" discipline `network.rs::apply_stdp`'s own
    /// Milestone 78 `pub(crate)` widening already established.
    pub(crate) fn new(threshold: f32, leak: f32) -> Self {
        LifRef { v: 0.0, threshold, leak }
    }

    pub(crate) fn step(&mut self, i: f32, dt: f32) -> bool {
        self.v += (i - self.leak * self.v) * dt;
        if self.v >= self.threshold {
            self.v = 0.0;
            true
        } else {
            false
        }
    }
}

pub(crate) struct Izhikevich {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    v: f32,
    u: f32,
}

impl Izhikevich {
    /// Presets from Izhikevich (2003) Table 1, same as
    /// pyspike_neuron_models.py's own PRESETS dict.
    fn new(preset: u8) -> Self {
        let (a, b, c, d) = match preset {
            0 => (0.02, 0.2, -65.0, 8.0),  // regular_spiking
            1 => (0.02, 0.2, -50.0, 2.0),  // chattering
            _ => (0.02, 0.2, -55.0, 4.0),  // intrinsically_bursting
        };
        Izhikevich { a, b, c, d, v: -65.0, u: b * -65.0 }
    }
    fn step(&mut self, i: f32, dt: f32) -> bool {
        let substeps = ((dt / 0.5) as u32).max(1);
        let sub_dt = dt / substeps as f32;
        let mut fired = false;
        for _ in 0..substeps {
            let dv = 0.04 * self.v * self.v + 5.0 * self.v + 140.0 - self.u + i;
            let du = self.a * (self.b * self.v - self.u);
            self.v += dv * sub_dt;
            self.u += du * sub_dt;
            if self.v >= 30.0 {
                self.v = self.c;
                self.u += self.d;
                fired = true;
            }
        }
        fired
    }
}

pub(crate) struct AdEx {
    tau_w: f32,
    a: f32,
    b: f32,
    v: f32,
    w: f32,
}

const ADEX_C: f32 = 200.0;
const ADEX_GL: f32 = 10.0;
const ADEX_EL: f32 = -70.0;
const ADEX_VT: f32 = -50.0;
const ADEX_DELTA_T: f32 = 2.0;
const ADEX_VRESET: f32 = -58.0;

impl AdEx {
    fn new(tau_w: f32, b: f32) -> Self {
        AdEx { tau_w, a: 2.0, b, v: ADEX_EL, w: 0.0 }
    }
    fn step(&mut self, i: f32, dt: f32) -> bool {
        let substeps = ((dt / 0.1) as u32).max(1);
        let sub_dt = dt / substeps as f32;
        let mut fired = false;
        for _ in 0..substeps {
            let exp_arg = ((self.v - ADEX_VT) / ADEX_DELTA_T).min(50.0);
            let exp_term = ADEX_DELTA_T * libm::expf(exp_arg);
            let dv = (-ADEX_GL * (self.v - ADEX_EL) + ADEX_GL * exp_term - self.w + i) / ADEX_C;
            let dw = (self.a * (self.v - ADEX_EL) - self.w) / self.tau_w;
            self.v += dv * sub_dt;
            self.w += dw * sub_dt;
            if self.v >= 0.0 {
                self.v = ADEX_VRESET;
                self.w += self.b;
                fired = true;
            }
        }
        fired
    }
}

pub(crate) struct Resonator {
    period_steps: f32, // steps-per-cycle, this kernel's stand-in for freq_hz (see adaptation #1)
    damping: f32,
    coupling: f32,
    threshold: f32,
    gate_threshold: f32,
    energy_tau: f32,
    x: f32,
    v: f32,
    energy_ema: f32,
}

impl Resonator {
    fn step(&mut self, i: f32, dt: f32) -> bool {
        let omega = 2.0 * core::f32::consts::PI / self.period_steps;
        let mut accel = -(omega * omega) * self.x - 2.0 * self.damping * omega * self.v;
        accel += self.coupling * i;
        self.v += accel * dt;
        self.x += self.v * dt;

        let alpha = (dt / self.energy_tau).min(1.0);
        let was_above = libm::sqrtf(self.energy_ema) >= self.threshold;
        if libm::fabsf(self.x) >= self.gate_threshold {
            self.energy_ema += alpha * (self.x * self.x - self.energy_ema);
        } else {
            self.energy_ema -= alpha * self.energy_ema;
        }
        let now_above = libm::sqrtf(self.energy_ema) >= self.threshold;
        now_above && !was_above
    }
}

/// MILESTONE 78 reuses this enum (and everything below it up to
/// `make_neuron`) from `kernel/src/hetero_stdp.rs` rather than copying
/// the four neuron dynamics a second time -- the exact discipline
/// `network.rs`'s own module doc names as a hard-won lesson (Milestone
/// 21's history: two independently-implemented copies of the same LIF
/// dynamics silently drifted out of sync for several milestones before
/// being unified into one engine).
pub(crate) enum Neuron {
    Lif(LifRef),
    Izh(Izhikevich),
    Adex(AdEx),
    Res(Resonator),
}

impl Neuron {
    pub(crate) fn step(&mut self, drive: f32, dt: f32) -> bool {
        match self {
            Neuron::Lif(n) => n.step(drive, dt),
            Neuron::Izh(n) => n.step(drive, dt),
            Neuron::Adex(n) => n.step(drive, dt),
            Neuron::Res(n) => n.step(drive, dt),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Lif,
    Izhikevich,
    Adex,
    Resonator,
}

pub(crate) const KINDS: [Kind; 4] = [Kind::Lif, Kind::Izhikevich, Kind::Adex, Kind::Resonator];

pub(crate) fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Lif => "lif",
        Kind::Izhikevich => "izhikevich",
        Kind::Adex => "adex",
        Kind::Resonator => "resonator",
    }
}

/// Real per-kind drive rescaling -- each type has its own native
/// operating range (Izhikevich/AdEx: mV/pA-scale per their literature
/// presets; LIF/Resonator: small normalized scales), exactly the same
/// "physically meaningless to feed one raw shared signal" problem the
/// Python file's own SCALE table discloses. These four numbers were
/// measured directly against this file's own real RDTSC drive values
/// during development (boot log: hetero_calibrate_dev_boot.log) --
/// first guesses (izhikevich=1.0, adex=1.0) never crossed either
/// type's real spike threshold at all, matching the Python file's own
/// documented experience with the same class of mistake.
fn scale_for(k: Kind) -> f32 {
    match k {
        Kind::Lif => 1.0,
        Kind::Izhikevich => 6.0,
        Kind::Adex => 180.0,
        Kind::Resonator => 3.0,
    }
}

pub(crate) fn make_neuron(k: Kind, rng: &mut Xorshift64) -> Neuron {
    match k {
        Kind::Lif => Neuron::Lif(LifRef { v: 0.0, threshold: rng.uniform(0.8, 1.3), leak: 0.05 }),
        Kind::Izhikevich => Neuron::Izh(Izhikevich::new(rng.pick3())),
        Kind::Adex => Neuron::Adex(AdEx::new(rng.uniform(400.0, 900.0), rng.uniform(40.0, 90.0))),
        Kind::Resonator => Neuron::Res(Resonator {
            period_steps: TARGET_PERIOD_STEPS,
            damping: rng.uniform(0.03, 0.08),
            coupling: 1.0,
            threshold: 0.0008,
            gate_threshold: 0.00024,
            energy_tau: 0.0025,
            x: 0.0,
            v: 0.0,
            energy_ema: 0.0,
        }),
    }
}

// ---------------------------------------------------------------------
// Real drive source: RDTSC cycle cost of a real ata::read_sector() PIO
// round trip, sampled on demand. Read-only, dedicated scratch LBA
// range clear of every other user of this disk (see module doc #2).
// ---------------------------------------------------------------------
const SCRATCH_LBA_BASE: u32 = 600;
const STEP_DT: f32 = 1.0; // one simulated time unit per loop iteration (see adaptation #1)
const TARGET_PERIOD_STEPS: f32 = 20.0; // the Resonator's tuned cycle length, in steps
const DISTRACTOR_PERIOD_STEPS: f32 = 7.0; // present in baseline as a background "wrong" tone

/// One real disk read, timed via RDTSC. Returns the elapsed cycle
/// count (0 on read failure -- honestly treated as "no signal this
/// step", not injected as a fake spike).
fn timed_disk_read(lba_offset: u32) -> u64 {
    let mut buf = [0u8; 512];
    let start = unsafe { core::arch::x86_64::_rdtsc() };
    let ok = ata::read_sector(SCRATCH_LBA_BASE + (lba_offset % 4), &mut buf).is_ok();
    let end = unsafe { core::arch::x86_64::_rdtsc() };
    if ok { end.saturating_sub(start) } else { 0 }
}

/// A quiet baseline "tone": every DISTRACTOR_PERIOD_STEPS steps, one
/// real disk read (a genuine, low-level background access pattern, not
/// silence) plus small deterministic dither so repeated baseline
/// windows aren't bit-identical. No anomaly structure of any kind.
fn baseline_raw(step: u32, rng: &mut Xorshift64) -> f32 {
    let base = if step % (DISTRACTOR_PERIOD_STEPS as u32) == 0 {
        timed_disk_read(step) as f32
    } else {
        0.0
    };
    base + rng.uniform(-1.0, 1.0)
}

/// magnitude: one sustained run of real disk reads, every step, for a
/// fixed window -- pure elevated magnitude, no burst/repetition/
/// frequency structure. LIF's home turf.
fn magnitude_raw(step: u32, t_start: u32, rng: &mut Xorshift64) -> f32 {
    if step >= t_start && step < t_start + MAG_WINDOW_STEPS {
        timed_disk_read(step) as f32 + rng.uniform(-1.0, 1.0)
    } else {
        baseline_raw(step, rng)
    }
}
const MAG_WINDOW_STEPS: u32 = 40;

/// burst: the SAME total number of real reads as magnitude_raw's
/// window (4 groups of 3 reads each = 12... see BURST_* below for the
/// exact accounting), delivered as short rapid clusters separated by
/// quiet gaps instead of one sustained run -- same real operation
/// count, different temporal shape. Izhikevich's home turf.
const BURST_GROUPS: u32 = 4;
const BURST_GROUP_LEN: u32 = 3;
const BURST_GROUP_GAP: u32 = 10;
fn burst_raw(step: u32, t_start: u32, rng: &mut Xorshift64) -> f32 {
    for g in 0..BURST_GROUPS {
        let group_start = t_start + g * BURST_GROUP_GAP;
        if step >= group_start && step < group_start + BURST_GROUP_LEN {
            return timed_disk_read(step) as f32 + rng.uniform(-1.0, 1.0);
        }
    }
    baseline_raw(step, rng)
}

/// repetition: the SAME single real read repeated back-to-back, many
/// more times than baseline's one-per-DISTRACTOR_PERIOD_STEPS rate --
/// same per-read magnitude, only repeat COUNT changes. AdEx's home
/// turf (its own w-adaptation variable makes it progressively harder
/// to re-fire on repeated same-size drive -- "fatigue").
const REPETITION_COUNT: u32 = 10;
fn repetition_raw(step: u32, t_start: u32, rng: &mut Xorshift64) -> f32 {
    if step >= t_start && step < t_start + REPETITION_COUNT {
        timed_disk_read(step) as f32 + rng.uniform(-1.0, 1.0)
    } else {
        baseline_raw(step, rng)
    }
}

/// frequency: real disk reads issued on a fixed, different periodic
/// cadence (TARGET_PERIOD_STEPS, vs baseline's own
/// DISTRACTOR_PERIOD_STEPS "wrong" cadence) for a fixed window -- same
/// per-read magnitude as the baseline's own tone, only the PERIOD
/// differs. Resonator's home turf (tuned to TARGET_PERIOD_STEPS).
fn frequency_raw(step: u32, t_start: u32, rng: &mut Xorshift64) -> f32 {
    if step >= t_start && step < t_start + FREQ_WINDOW_STEPS {
        if step % (TARGET_PERIOD_STEPS as u32) == 0 {
            timed_disk_read(step) as f32 + rng.uniform(-1.0, 1.0)
        } else {
            rng.uniform(-1.0, 1.0)
        }
    } else {
        baseline_raw(step, rng)
    }
}
const FREQ_WINDOW_STEPS: u32 = 80;

type SignalFn = fn(u32, u32, &mut Xorshift64) -> f32;

const ANOMALIES: [(&str, SignalFn); 4] =
    [("magnitude", magnitude_raw), ("burst", burst_raw), ("repetition", repetition_raw), ("frequency", frequency_raw)];

const T_START: u32 = 15; // quiet lead-in steps before the anomaly window
const TOTAL_STEPS: u32 = 120;

fn run_population(kinds_and_neurons: &mut [(Kind, Neuron)], signal: SignalFn, seed: u64) -> Vec<u32> {
    let mut rng = Xorshift64::new(seed);
    let mut fires = alloc::vec![0u32; kinds_and_neurons.len()];
    for step in 0..TOTAL_STEPS {
        let raw = signal(step, T_START, &mut rng);
        for (idx, (kind, neuron)) in kinds_and_neurons.iter_mut().enumerate() {
            let drive = raw * scale_for(*kind);
            if neuron.step(drive, STEP_DT) {
                fires[idx] += 1;
            }
        }
    }
    fires
}

fn make_population(kind: Kind, n: usize, seed: u64) -> Vec<(Kind, Neuron)> {
    let mut rng = Xorshift64::new(seed);
    (0..n).map(|_| (kind, make_neuron(kind, &mut rng))).collect()
}

// MILESTONE 165: Milestone 77's calibration was unsound. It set each
// threshold at mean + 3 sigma of only 4 quiet windows and detected on
// `count >= threshold`. A 32-window measurement (m165_diag_run2_reused_boot.log)
// showed the resonator population fires exactly 3 times on EVERY signal --
// quiet baseline 3 (32/32), frequency anomaly 3 (32/32) -- so with sigma = 0
// its threshold was 3.00 and every window, quiet or not, counted as a
// "frequency detection". M77's 16/16-vs-12/16 PASS rested on that. When one
// calibration window happened to read 4, sigma became 0.5, the threshold
// 4.75, and M77 "flaked" to FAIL (M141, M157, M164's first fresh boot): that
// FAIL was the honest outcome. On this disk signal the frequency anomaly is
// a DROP in activity (Izhikevich 54 -> 33, AdEx 54 -> 33, LIF x3 ~78 -> ~55,
// LIF x12 ~300 -> ~210), so nothing here detects it as an excess.
//
// Fix: require a detection to exceed every calibration window (threshold
// >= max + 1, and >= mean + 3 sigma) and measure false positives on
// held-out quiet windows.
//
// First attempt (m165_first_attempt_{fresh,reused}_boot.log): 16 calibration
// windows, gate "0/8 held-out false positives". Old rule: heterogeneous 8/8
// false positives in both boots (the bug, confirmed). New rule:
// heterogeneous 0/8 but homogeneous 1/8 -- 12x LIF's quiet-window counts have
// a tail past mean + 3 sigma of 16 windows -- so the gate failed. A zero-
// false-positive gate is itself flaky for a tailed count: by exchangeability
// a new quiet window exceeds the max of n calibration windows with
// probability 1/(n+1).
//
// Second attempt (this code), re-pre-registered: 32 calibration windows
// (per-detector false-positive rate <= 1/33 by the argument above), 32
// held-out windows, gate: each ensemble <= MAX_HELDOUT_FP = 4/32. The
// heterogeneous decision ORs four detectors, so its worst case is ~4/33 per
// window. Predictions: (Q1) heterogeneous false positives <= 1/32 (its
// resonator and AdEx counts are constant on quiet windows, Izhikevich spans
// 54-56); (Q2) homogeneous <= 2/32; (Q3) old rule heterogeneous 32/32
// false positives whenever its resonator threshold is 3.00; (Q4) frequency
// detected <= 1/4 by either ensemble; (Q5) OVERALL_M77 is a tie (FAIL) or
// a margin of one trial, varying boot to boot.
/// Total spike count of an `n`-neuron population of `kind` on one quiet
/// baseline window per `w` in 0..n_windows (Milestone 77's seeding).
fn calibration_counts(kind: Kind, n: usize, seed: u64, n_windows: u32) -> Vec<f32> {
    let mut counts: Vec<f32> = Vec::new();
    for w in 0..n_windows {
        let mut pop = make_population(kind, n, seed.wrapping_add(w as u64).wrapping_mul(2654435761));
        let fires = run_population(&mut pop, |s, _t, r| baseline_raw(s, r), seed.wrapping_add(w as u64));
        counts.push(fires.iter().sum::<u32>() as f32);
    }
    counts
}

/// (mean, sample std, max) of a set of window counts.
fn count_stats(counts: &[f32]) -> (f32, f32, f32) {
    let mean = counts.iter().sum::<f32>() / counts.len() as f32;
    let var = counts.iter().map(|c| (c - mean) * (c - mean)).sum::<f32>() / (counts.len().max(2) - 1) as f32;
    let max = counts.iter().fold(0.0f32, |m, c| libm::fmaxf(m, *c));
    (mean, libm::sqrtf(var), max)
}

/// Milestone 77's original rule: mean + 3 sigma over the first 4 windows,
/// floored at 1.0, detection `count >= threshold`.
fn old_threshold(counts: &[f32]) -> f32 {
    let (mean, std, _) = count_stats(&counts[..OLD_CALIB_WINDOWS]);
    libm::fmaxf(mean + 3.0 * std, 1.0)
}

/// MILESTONE 165: the same mean + 3 sigma over all N_CALIB_WINDOWS, but
/// never at or below the largest calibration count -- a detection must
/// exceed every quiet window actually observed.
fn new_threshold(counts: &[f32]) -> f32 {
    let (mean, std, max) = count_stats(counts);
    libm::fmaxf(mean + 3.0 * std, max + 1.0)
}

const N_PER_TYPE: usize = 3;
const N_TOTAL_HOMOG: usize = 12; // same total neuron count as the 4 heterogeneous types combined
const N_TRIALS: u32 = 4;
const OLD_CALIB_WINDOWS: usize = 4; // Milestone 77's original calibration size
const N_CALIB_WINDOWS: u32 = 32; // MILESTONE 165
const N_HELDOUT: u32 = 32; // MILESTONE 165: held-out quiet windows for the false-positive control
const MAX_HELDOUT_FP: u32 = 4; // MILESTONE 165: gate -- at most 4/32 false positives per ensemble

/// Heterogeneous decision for one trial: which of the four typed
/// sub-populations reached its threshold (bit k = KINDS[k]), under the old
/// and the new thresholds, from the SAME spike counts.
fn het_trial(signal: SignalFn, seed: u64, old_thr: &[f32; 4], new_thr: &[f32; 4]) -> (u8, u8) {
    let mut old_bits = 0u8;
    let mut new_bits = 0u8;
    for (k_idx, kind) in KINDS.iter().enumerate() {
        let mut pop = make_population(*kind, N_PER_TYPE, seed);
        let total = run_population(&mut pop, signal, seed.wrapping_add(999)).iter().sum::<u32>() as f32;
        if total >= old_thr[k_idx] {
            old_bits |= 1 << k_idx;
        }
        if total >= new_thr[k_idx] {
            new_bits |= 1 << k_idx;
        }
    }
    (old_bits, new_bits)
}

/// Homogeneous decision for one trial under (old, new) thresholds.
fn homog_trial(signal: SignalFn, seed: u64, old_thr: f32, new_thr: f32) -> (bool, bool) {
    let mut pop = make_population(Kind::Lif, N_TOTAL_HOMOG, seed);
    let total = run_population(&mut pop, signal, seed.wrapping_add(999)).iter().sum::<u32>() as f32;
    (total >= old_thr, total >= new_thr)
}

/// MILESTONE 77: runs the full comparison and logs a result line per
/// anomaly type plus a final OVERALL_M77 verdict, same "PASS"/"FAIL"
/// convention every other milestone's self-test in this kernel uses.
/// Confirms the SAME pre-registered hypothesis the Python study tested
/// (heterogeneous typed ensemble beats homogeneous same-count LIF
/// ensemble, combined across all 4 anomaly types) against this
/// kernel's own real disk I/O -- reports honestly either way, per the
/// Python file's own DISCONFIRM clause.
pub fn self_test_hetero_ensemble() {
    let mut port = serial();
    let _ = writeln!(
        port,
        "milestone 77: heterogeneous neuron ensemble self-test starting -- real ata.rs disk I/O as the drive signal, real RDTSC timing, {N_PER_TYPE} neurons/type x 4 types vs {N_TOTAL_HOMOG}x homogeneous LIF"
    );

    // MILESTONE 165: calibrate on N_CALIB_WINDOWS; the old rule reuses the
    // first OLD_CALIB_WINDOWS of the same windows (M77's exact seeds).
    let mut old_thr = [0.0f32; 4];
    let mut new_thr = [0.0f32; 4];
    for (idx, kind) in KINDS.iter().enumerate() {
        let counts = calibration_counts(*kind, N_PER_TYPE, 0x1000 + idx as u64, N_CALIB_WINDOWS);
        let (mean, std, max) = count_stats(&counts);
        old_thr[idx] = old_threshold(&counts);
        new_thr[idx] = new_threshold(&counts);
        let _ = writeln!(
            port,
            "milestone 77: calibrated {} threshold={:.2} (baseline mean={mean:.2}, std={std:.2}, max={max:.0}, {N_CALIB_WINDOWS} windows; M77's old 4-window rule gave {:.2})",
            kind_name(*kind),
            new_thr[idx],
            old_thr[idx]
        );
    }
    let homog_counts = calibration_counts(Kind::Lif, N_TOTAL_HOMOG, 0x2000, N_CALIB_WINDOWS);
    let (h_mean, h_std, h_max) = count_stats(&homog_counts);
    let homog_old_thr = old_threshold(&homog_counts);
    let homog_new_thr = new_threshold(&homog_counts);
    let _ = writeln!(
        port,
        "milestone 77: calibrated homogeneous ({N_TOTAL_HOMOG}x LIF) threshold={homog_new_thr:.2} (baseline mean={h_mean:.2}, std={h_std:.2}, max={h_max:.0}; old rule {homog_old_thr:.2})"
    );

    let mut het_detected: u32 = 0;
    let mut het_old_detected: u32 = 0;
    let mut het_total: u32 = 0;
    let mut homog_detected: u32 = 0;
    let mut homog_old_detected: u32 = 0;
    let mut homog_total: u32 = 0;

    for (a_idx, (name, signal)) in ANOMALIES.iter().enumerate() {
        let mut het_hits = 0u32;
        let mut het_old_hits = 0u32;
        let mut per_type = [0u32; 4];
        for trial in 0..N_TRIALS {
            let seed = 0x3000 + (a_idx as u64) * 1000 + trial as u64 * 137;
            let (old_bits, new_bits) = het_trial(*signal, seed, &old_thr, &new_thr);
            if new_bits != 0 {
                het_hits += 1;
            }
            if old_bits != 0 {
                het_old_hits += 1;
            }
            for (k, c) in per_type.iter_mut().enumerate() {
                if new_bits & (1 << k) != 0 {
                    *c += 1;
                }
            }
        }
        het_detected += het_hits;
        het_old_detected += het_old_hits;
        het_total += N_TRIALS;

        let mut homog_det = 0u32;
        let mut homog_old_det = 0u32;
        for trial in 0..N_TRIALS {
            let seed = 0x4000 + a_idx as u64 * 1000 + trial as u64 * 137;
            let (o, n) = homog_trial(*signal, seed, homog_old_thr, homog_new_thr);
            homog_old_det += o as u32;
            homog_det += n as u32;
        }
        homog_detected += homog_det;
        homog_old_detected += homog_old_det;
        homog_total += N_TRIALS;

        let _ = writeln!(
            port,
            "milestone 77: {name:<12} heterogeneous={het_hits}/{N_TRIALS}  homogeneous({N_TOTAL_HOMOG}xLIF)={homog_det}/{N_TRIALS}  | by type: lif={} izhikevich={} adex={} resonator={}  | old rule: het={het_old_hits} homog={homog_old_det}",
            per_type[0], per_type[1], per_type[2], per_type[3]
        );
    }

    // MILESTONE 165: false-positive control -- the same decisions on quiet
    // baseline windows never used for calibration. A detector that fires
    // here is not detecting the anomaly.
    let quiet: SignalFn = |s, _t, r| baseline_raw(s, r);
    let mut het_fp = 0u32;
    let mut het_old_fp = 0u32;
    let mut homog_fp = 0u32;
    let mut homog_old_fp = 0u32;
    for trial in 0..N_HELDOUT {
        let (old_bits, new_bits) = het_trial(quiet, 0x5000 + trial as u64 * 137, &old_thr, &new_thr);
        het_fp += (new_bits != 0) as u32;
        het_old_fp += (old_bits != 0) as u32;
        let (o, n) = homog_trial(quiet, 0x6000 + trial as u64 * 137, homog_old_thr, homog_new_thr);
        homog_fp += n as u32;
        homog_old_fp += o as u32;
    }
    let _ = writeln!(
        port,
        "milestone 165: false-positive control ({N_HELDOUT} held-out quiet windows) -- heterogeneous={het_fp}/{N_HELDOUT} homogeneous={homog_fp}/{N_HELDOUT} | M77's old rule: heterogeneous={het_old_fp}/{N_HELDOUT} homogeneous={homog_old_fp}/{N_HELDOUT}"
    );
    let _ = writeln!(
        port,
        "milestone 165: old rule TOTAL heterogeneous={het_old_detected}/{het_total} homogeneous={homog_old_detected}/{homog_total} (what M77 reported before this fix)"
    );

    let het_pct = 100.0 * het_detected as f32 / het_total as f32;
    let homog_pct = 100.0 * homog_detected as f32 / homog_total as f32;
    let _ = writeln!(
        port,
        "milestone 77: TOTAL heterogeneous={het_detected}/{het_total} ({het_pct:.1}%)  homogeneous={homog_detected}/{homog_total} ({homog_pct:.1}%)"
    );

    // Honest verdict, per the Python file's own DISCONFIRM clause: PASS
    // means the heterogeneous ensemble detected strictly more than the
    // homogeneous one on this kernel's own real disk signal, not a
    // hardcoded expectation. Since Milestone 165 this uses the corrected
    // thresholds; before it, the resonator's threshold usually equalled its
    // constant quiet-window count, so every window -- quiet or not -- was a
    // "frequency detection".
    let pass = het_detected > homog_detected;
    let _ = writeln!(
        port,
        "milestone 77: self-test -- {}",
        if pass {
            "OVERALL_M77=PASS (heterogeneous typed ensemble detected more than homogeneous, confirming the Python study's finding transfers to this kernel's own real disk I/O)"
        } else {
            "OVERALL_M77=FAIL (heterogeneous ensemble did NOT beat homogeneous on this real signal -- reported honestly, a real negative result if so)"
        }
    );
    // MILESTONE 165's own check is that the evaluation is sound: under the
    // corrected rule neither ensemble fires on more than MAX_HELDOUT_FP of
    // the held-out quiet windows.
    let m165 = het_fp <= MAX_HELDOUT_FP && homog_fp <= MAX_HELDOUT_FP;
    let _ = writeln!(port, "milestone 165: OVERALL_M165={}", if m165 { "PASS" } else { "FAIL" });
}

// =====================================================================
// MILESTONE 173: does a Resonator configured per the vault lesson
// ("A Spikeling Resonator needs many real cycles to discriminate
// periodicity") tell a PERIODIC disk-read train from a RANDOM-interval
// train with the same mean rate, on this kernel's real disk signal?
//
// Why: M165 showed M77's resonator fires 3 times on every signal. Its
// setup violates every rule in that lesson -- 4 cycles of the target
// period (lesson: >= 30-50), energy_tau 0.0025 with dt = 1 step (alpha =
// 1, no averaging; lesson: several periods), damping 0.03-0.08 (lesson:
// ~0.01), 1-tick impulses of raw ~1e5-cycle counts (lesson: a short
// sustained burst). M77's frequency anomaly also changes the read RATE,
// so a rate detector could separate it; this study holds the rate fixed.
//
// Design: per seed, two 800-step drive sequences -- P: a real timed disk
// read every 20 steps (40 reads, 40 cycles of period 20); R: a real read
// after random gaps drawn uniformly from 10..30 steps (mean 20, same
// rate). Each read's RDTSC cost is normalised by the mean of 8 warm-up
// reads (~1.0) and applied for 2 steps (10% of a period). The SAME drive
// sequence feeds every detector (paired). Detectors: resonator tuned to
// 20 (damping 0.01, energy_tau 60 = 3 periods); the same resonator tuned
// to 13 (specificity control); LIF threshold 1.0 leak 0.05 (rate control).
// Metric: resonator = mean sqrt(energy_ema) over steps 400..800; LIF =
// spike count. 16 seeds.
//
// Pre-registered predictions, written before the first run:
//   Q1 resonator(20): mean metric on P >= 3.0 x mean on R
//      (the lesson measured 5-8x off-kernel)
//   Q2 resonator(20): P beats R in >= 90% of the 16 x 16 (P_i, R_j) pairs
//   Q3 LIF: |mean count P - mean count R| <= 10% of mean P (rate is equal,
//      so a rate detector cannot separate them)
//   Q4 resonator(20) on P >= 2.0 x resonator(13) on P (it is the TUNING,
//      not just "any oscillator", that responds)
// OVERALL_M173 only asserts the study ran on real data (every read
// succeeded, normaliser > 0); each prediction is printed MET or MISSED.
// =====================================================================
const M173_STEPS: usize = 800;
const M173_PERIOD: u32 = 20;
const M173_SEEDS: u32 = 16;

/// One drive sequence of real timed reads. Returns (drive, reads, failed reads).
fn m173_drive(periodic: bool, rng: &mut Xorshift64, norm: f32) -> (Vec<f32>, u32, u32) {
    let mut drive = alloc::vec![0.0f32; M173_STEPS];
    let mut reads = 0u32;
    let mut failed = 0u32;
    let mut t: usize = if periodic { 0 } else { (rng.next_u64() % 20) as usize };
    while t < M173_STEPS {
        let c = timed_disk_read(t as u32);
        if c == 0 {
            failed += 1;
        }
        let v = c as f32 / norm;
        drive[t] += v;
        if t + 1 < M173_STEPS {
            drive[t + 1] += v;
        }
        reads += 1;
        let gap = if periodic { M173_PERIOD as usize } else { 10 + (rng.next_u64() % 21) as usize };
        t += gap;
    }
    (drive, reads, failed)
}

fn m173_resonator(period: f32, drive: &[f32]) -> f32 {
    let mut r = Resonator {
        period_steps: period,
        damping: 0.01,
        coupling: 1.0,
        threshold: f32::MAX,
        gate_threshold: 0.0,
        energy_tau: 60.0,
        x: 0.0,
        v: 0.0,
        energy_ema: 0.0,
    };
    let mut acc = 0.0f32;
    for (step, d) in drive.iter().enumerate() {
        let _ = r.step(*d, STEP_DT);
        if step >= M173_STEPS / 2 {
            acc += libm::sqrtf(r.energy_ema);
        }
    }
    acc / (M173_STEPS / 2) as f32
}

fn m173_lif(drive: &[f32]) -> f32 {
    let mut n = LifRef::new(1.0, 0.05);
    let mut spikes = 0u32;
    for d in drive.iter() {
        if n.step(*d, STEP_DT) {
            spikes += 1;
        }
    }
    spikes as f32
}

pub fn self_test_m173_resonator_study() {
    let mut port = serial();
    // normaliser: mean cost of 8 warm-up reads
    let mut sum = 0u64;
    let mut warm_failed = 0u32;
    for i in 0..8u32 {
        let c = timed_disk_read(i);
        if c == 0 {
            warm_failed += 1;
        }
        sum += c;
    }
    let norm = (sum / 8) as f32;
    let _ = writeln!(port, "milestone 173: resonator periodicity study -- normaliser {norm:.0} cycles/read, {M173_SEEDS} seeds, {M173_STEPS} steps, period {M173_PERIOD}");

    let mut res_p = Vec::new();
    let mut res_r = Vec::new();
    let mut det_p = Vec::new();
    let mut lif_p = 0.0f32;
    let mut lif_r = 0.0f32;
    let mut reads_p = 0u32;
    let mut reads_r = 0u32;
    let mut failed = warm_failed;
    for seed in 0..M173_SEEDS {
        let mut rng = Xorshift64::new(0x7300 + seed as u64 * 7919);
        let (dp, np, fp) = m173_drive(true, &mut rng, norm);
        let (dr, nr, fr) = m173_drive(false, &mut rng, norm);
        reads_p += np;
        reads_r += nr;
        failed += fp + fr;
        res_p.push(m173_resonator(M173_PERIOD as f32, &dp));
        res_r.push(m173_resonator(M173_PERIOD as f32, &dr));
        det_p.push(m173_resonator(13.0, &dp));
        lif_p += m173_lif(&dp);
        lif_r += m173_lif(&dr);
    }
    let mean = |v: &Vec<f32>| v.iter().sum::<f32>() / v.len() as f32;
    let (mp, mr, md) = (mean(&res_p), mean(&res_r), mean(&det_p));
    let mut wins = 0u32;
    for p in res_p.iter() {
        for r in res_r.iter() {
            if p > r {
                wins += 1;
            }
        }
    }
    let pairs = M173_SEEDS * M173_SEEDS;
    let lp = lif_p / M173_SEEDS as f32;
    let lr = lif_r / M173_SEEDS as f32;
    let _ = writeln!(port, "milestone 173: reads periodic={reads_p} random={reads_r} failed={failed}");
    let _ = writeln!(port, "milestone 173: resonator(20) mean metric periodic={mp:.4} random={mr:.4} ratio={:.2} | P>R in {wins}/{pairs} pairs | resonator(13) on periodic={md:.4} (tuned/detuned {:.2}) | LIF spikes periodic={lp:.1} random={lr:.1}", mp / mr, mp / md);
    let met = |b: bool| if b { "MET" } else { "MISSED" };
    let q1 = mp >= 3.0 * mr;
    let q2 = wins * 10 >= pairs * 9;
    let q3 = libm::fabsf(lp - lr) <= 0.10 * lp;
    let q4 = mp >= 2.0 * md;
    let _ = writeln!(port, "milestone 173: Q1 ratio>=3.0 {} | Q2 pairs>=90% {} | Q3 LIF within 10% {} | Q4 tuned>=2x detuned {}", met(q1), met(q2), met(q3), met(q4));
    let ok = failed == 0 && norm > 0.0;
    let _ = writeln!(port, "milestone 173: OVERALL_M173={}", if ok { "PASS" } else { "FAIL" });
}

// =====================================================================
// MILESTONE 174: M77's hypothesis -- a heterogeneous typed ensemble
// detects more anomaly kinds than a same-size homogeneous LIF ensemble --
// re-tested fairly, now that M165 made the thresholds sound and M173
// showed how to configure a resonator that actually responds.
//
// What changes from M77, and why:
// - Baseline: real timed reads after RANDOM gaps uniform in 4..10 steps
//   (mean 7), 400-step trials. M77's baseline was a regular period-7 tone.
// - Frequency anomaly: from step 100 the reads become exactly PERIODIC
//   (every 7 steps) at the SAME mean rate -- only the timing structure
//   changes (M77's removed reads, a rate change). Magnitude (a read every
//   step 100..140), burst (4 groups of 3 reads, 10 apart) and repetition
//   (10 consecutive reads) are inserted at step 100 as in M77.
// - Resonator per M173 / the vault lesson: tuned to 7, damping 0.01,
//   energy_tau 21 (3 periods), drive = read cost / warm-up mean, 1 step
//   per read (~10% of a period); score = mean sqrt(energy) over steps
//   100..400 (~43 cycles). LIF, Izhikevich and AdEx keep M77's dynamics,
//   scales and spike-count scores (3 neurons per type; 12 LIF homogeneous).
// - Each trial's drive is generated ONCE (real reads) and fed to every
//   detector (paired).
// - Thresholds per M165: 32 quiet calibration windows; detect when
//   score > max(mean + 3 sigma, largest calibration score). False-positive
//   control on 32 held-out quiet windows; gate <= 4/32 per ensemble.
//
// Pre-registered predictions, written before the first run:
//   R1 frequency: heterogeneous >= 6/8 (through the resonator),
//      homogeneous <= 2/8 (same rate, so LIF counts cannot move)
//   R2 magnitude, burst, repetition: each ensemble >= 6/8 on each
//   R3 held-out false positives <= 4/32 for both ensembles (the gate)
//   R4 the hypothesis: heterogeneous total > homogeneous total
// OVERALL_M174 = R3 and every read succeeded (the evaluation is sound);
// R1, R2, R4 are printed MET or MISSED.
// =====================================================================
const M174_STEPS: usize = 400;
const M174_START: usize = 100;
const M174_PERIOD: usize = 7;
const M174_CALIB: u32 = 32;
const M174_HELDOUT: u32 = 32;
const M174_TRIALS: u32 = 8;

/// One trial's raw drive (M77 units: read cycles + uniform(-1,1) noise).
/// kind: 0 quiet, 1 magnitude, 2 burst, 3 repetition, 4 frequency.
fn m174_signal(kind: u8, rng: &mut Xorshift64, failed: &mut u32) -> Vec<f32> {
    let mut read_at = alloc::vec![false; M174_STEPS];
    let mut t = (rng.next_u64() % 7) as usize;
    while t < M174_STEPS {
        if kind == 4 && t >= M174_START {
            // periodic from the anomaly onset, same mean gap
            let mut p = M174_START;
            while p < M174_STEPS {
                read_at[p] = true;
                p += M174_PERIOD;
            }
            break;
        }
        read_at[t] = true;
        t += 4 + (rng.next_u64() % 7) as usize;
    }
    match kind {
        1 => {
            for s in M174_START..M174_START + 40 {
                read_at[s] = true;
            }
        }
        2 => {
            for g in 0..4 {
                for s in 0..3 {
                    read_at[M174_START + g * 10 + s] = true;
                }
            }
        }
        3 => {
            for s in M174_START..M174_START + 10 {
                read_at[s] = true;
            }
        }
        _ => {}
    }
    let mut raw = alloc::vec![0.0f32; M174_STEPS];
    for s in 0..M174_STEPS {
        let mut v = rng.uniform(-1.0, 1.0);
        if read_at[s] {
            let c = timed_disk_read(s as u32);
            if c == 0 {
                *failed += 1;
            }
            v += c as f32;
        }
        raw[s] = v;
    }
    raw
}

fn m174_spike_score(kind: Kind, n: usize, raw: &[f32], seed: u64) -> f32 {
    let mut pop = make_population(kind, n, seed);
    let mut total = 0u32;
    for x in raw.iter() {
        for (k, neuron) in pop.iter_mut() {
            if neuron.step(*x * scale_for(*k), STEP_DT) {
                total += 1;
            }
        }
    }
    total as f32
}

fn m174_resonator_score(raw: &[f32], norm: f32) -> f32 {
    let mut r = Resonator {
        period_steps: M174_PERIOD as f32,
        damping: 0.01,
        coupling: 1.0,
        threshold: f32::MAX,
        gate_threshold: 0.0,
        energy_tau: 3.0 * M174_PERIOD as f32,
        x: 0.0,
        v: 0.0,
        energy_ema: 0.0,
    };
    let mut acc = 0.0f32;
    for (s, x) in raw.iter().enumerate() {
        // reads only; the +-1 dither is far below one read's normalised cost
        let d = if *x > 2.0 { *x / norm } else { 0.0 };
        let _ = r.step(d, STEP_DT);
        if s >= M174_START {
            acc += libm::sqrtf(r.energy_ema);
        }
    }
    acc / (M174_STEPS - M174_START) as f32
}

/// Scores for one trial: [lif, izhikevich, adex, resonator, homogeneous].
fn m174_scores(raw: &[f32], norm: f32, seed: u64) -> [f32; 5] {
    [
        m174_spike_score(Kind::Lif, 3, raw, seed),
        m174_spike_score(Kind::Izhikevich, 3, raw, seed),
        m174_spike_score(Kind::Adex, 3, raw, seed),
        m174_resonator_score(raw, norm),
        m174_spike_score(Kind::Lif, 12, raw, seed ^ 0x55),
    ]
}

pub fn self_test_m174_ensemble_retest() {
    let mut port = serial();
    let mut failed = 0u32;
    let mut sum = 0u64;
    for i in 0..8u32 {
        let c = timed_disk_read(i);
        if c == 0 {
            failed += 1;
        }
        sum += c;
    }
    let norm = (sum / 8) as f32;
    let _ = writeln!(port, "milestone 174: ensemble re-test starting -- normaliser {norm:.0} cycles/read, {M174_STEPS}-step trials, anomaly at {M174_START}");

    // calibration on quiet windows
    let mut cal: [Vec<f32>; 5] = [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for w in 0..M174_CALIB {
        let seed = 0x7400 + w as u64 * 131;
        let mut rng = Xorshift64::new(seed);
        let raw = m174_signal(0, &mut rng, &mut failed);
        let sc = m174_scores(&raw, norm, seed);
        for d in 0..5 {
            cal[d].push(sc[d]);
        }
    }
    let mut thr = [0.0f32; 5];
    for d in 0..5 {
        let (mean, std, max) = count_stats(&cal[d]);
        thr[d] = libm::fmaxf(mean + 3.0 * std, max);
    }
    let _ = writeln!(
        port,
        "milestone 174: thresholds (score must EXCEED) lif={:.1} izhikevich={:.1} adex={:.1} resonator={:.3} homogeneous={:.1}",
        thr[0], thr[1], thr[2], thr[3], thr[4]
    );
    let het_hit = |sc: &[f32; 5]| -> u8 {
        let mut bits = 0u8;
        for d in 0..4 {
            if sc[d] > thr[d] {
                bits |= 1 << d;
            }
        }
        bits
    };

    // held-out false positives
    let mut het_fp = 0u32;
    let mut homog_fp = 0u32;
    for w in 0..M174_HELDOUT {
        let seed = 0x7500 + w as u64 * 137;
        let mut rng = Xorshift64::new(seed);
        let raw = m174_signal(0, &mut rng, &mut failed);
        let sc = m174_scores(&raw, norm, seed);
        het_fp += (het_hit(&sc) != 0) as u32;
        homog_fp += (sc[4] > thr[4]) as u32;
    }
    let _ = writeln!(port, "milestone 174: held-out false positives heterogeneous={het_fp}/{M174_HELDOUT} homogeneous={homog_fp}/{M174_HELDOUT}");

    let names = ["magnitude", "burst", "repetition", "frequency"];
    let mut het_tot = 0u32;
    let mut homog_tot = 0u32;
    let mut per = [[0u32; 2]; 4];
    for a in 0..4usize {
        let mut by_type = [0u32; 4];
        for tr in 0..M174_TRIALS {
            let seed = 0x7600 + a as u64 * 1000 + tr as u64 * 139;
            let mut rng = Xorshift64::new(seed);
            let raw = m174_signal(a as u8 + 1, &mut rng, &mut failed);
            let sc = m174_scores(&raw, norm, seed);
            let bits = het_hit(&sc);
            if bits != 0 {
                per[a][0] += 1;
            }
            if sc[4] > thr[4] {
                per[a][1] += 1;
            }
            for d in 0..4 {
                if bits & (1 << d) != 0 {
                    by_type[d] += 1;
                }
            }
        }
        het_tot += per[a][0];
        homog_tot += per[a][1];
        let _ = writeln!(
            port,
            "milestone 174: {:<10} heterogeneous={}/{M174_TRIALS} homogeneous={}/{M174_TRIALS} | by type: lif={} izhikevich={} adex={} resonator={}",
            names[a], per[a][0], per[a][1], by_type[0], by_type[1], by_type[2], by_type[3]
        );
    }
    let met = |b: bool| if b { "MET" } else { "MISSED" };
    let r1 = per[3][0] >= 6 && per[3][1] <= 2;
    let r2 = (0..3).all(|a| per[a][0] >= 6 && per[a][1] >= 6);
    let r3 = het_fp <= 4 && homog_fp <= 4;
    let r4 = het_tot > homog_tot;
    let _ = writeln!(port, "milestone 174: TOTAL heterogeneous={het_tot}/32 homogeneous={homog_tot}/32 | reads failed={failed}");
    let _ = writeln!(port, "milestone 174: R1 frequency {} | R2 other anomalies {} | R3 false-positive gate {} | R4 hypothesis het>homog {}", met(r1), met(r2), met(r3), met(r4));
    let ok = r3 && failed == 0 && norm > 0.0;
    let _ = writeln!(port, "milestone 174: OVERALL_M174={}", if ok { "PASS" } else { "FAIL" });
}
