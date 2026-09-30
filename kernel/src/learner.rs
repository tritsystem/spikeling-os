//! MILESTONE 162: an online, self-repairing disk-read learner -- a
//! prefetcher built from the coupling of OBSERVATION, VARIATION, SELECTION,
//! MEMORY and DIVERSITY, not from one clever predictor.
//!
//! * **Diversity.** A population of `NSLOTS` predictors, each a different
//!   belief about what the next demand read will be: `SEQ` (last+1),
//!   `STRIDE` (last + the last delta), `MARKOV` (whatever followed this
//!   sector last time), `LAG(L)` (the read L steps ago repeats: loops), and
//!   `LAGSTRIDE(L)` (the L-steps-ago read plus that lane's own stride:
//!   interleaved sequential streams). Different workloads are won by
//!   different members; no member is trusted a priori.
//! * **Observation.** Every logical demand read (`ata::read_sector`) is
//!   fed to `observe()`, which scores EVERY predictor against what really
//!   happened (an EMA of exact hits) whether or not it was used.
//! * **Selection.** Only predictors whose score has reached `MIN_CONF`
//!   are allowed to trigger a prefetch, best first; the mutable slots
//!   compete and the worst mediocre one is replaced.
//! * **Variation (mutation).** Every `MUT_PERIOD` reads the worst
//!   mutable slot with enough trials is replaced by a mutant: half the time
//!   a small step (`L +/- 1`) around the best mutable slot (exploitation of
//!   what works), half the time a random `(kind, L)` (exploration).
//!   Mutation alone is noise and adaptation alone is brittle -- it is the
//!   coupling with scoring (selection) that keeps only what predicts.
//! * **Memory.** The learned state (parameters, scores, the Markov table)
//!   is checksummed and saved to two disk sectors and validated on load, so
//!   a warm start keeps what was learned and a corrupt record is refused.
//! * **Resilience (retaining core function under disturbance).** Prefetch
//!   is purely ADVISORY -- it can waste work but can never change what a
//!   read returns. A "do no harm" `evidence_gate` subscriber
//!   (`GateKind::PrefetchWaste`) throttles prefetching when prefetched
//!   sectors keep being evicted unused. A predictor that is corrupted --
//!   even one lying with high confidence (`inject_fault`) -- is exposed by
//!   its own scoring and is self-healed (re-initialised) once its score
//!   collapses, while the remaining diverse members carry the load.
//!
//! Prefetching only helps if the disk work can happen off the critical
//! path; with synchronous PIO reads that is only true in idle time, so the
//! benchmark (`self_test_learner`) counts CRITICAL-PATH cycles (time inside
//! demand reads) separately from the total prefetch work it added. Off by
//! default (`ENABLED`): nothing in the shipped kernel prefetches yet.

use crate::ata;
use crate::evidence_gate::{self, GateConfig, GateKind};
use crate::serial;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use spin::Mutex;

pub const NSLOTS: usize = 6;
/// Slots at index >= this may be replaced by mutation; the first three
/// (SEQ, STRIDE, MARKOV) are the fixed, always-present base members.
const FIRST_MUTABLE: usize = 3;
const MARKOV_SLOTS: usize = 128;
/// 4-way set-associative (32 sets x 4 ways). A first version was direct-mapped:
/// on the 40-sector loop benchmark 9 keys collided and colliding keys evicted
/// EACH OTHER on every pass, so both lost their prediction (160 of 320 reads
/// covered; a 2-way table covers 237, 4-way 279 -- simulated, then measured).
const MARKOV_SETS: usize = 32;
const MARKOV_WAYS: usize = 4;
const HIST: usize = 64;
const DISK_SECTORS: u32 = 2048;
const NONE: u32 = u32::MAX;
/// EMA weight kept by the old score (`score = 0.85*score + 0.15*hit`).
const SCORE_KEEP: f32 = 0.85;
/// A predictor must have earned this score before it may trigger a prefetch.
const MIN_CONF: f32 = 0.4;
/// Per-read confidence decay for a predictor that made no prediction.
const ABSTAIN_DECAY: f32 = 0.95;
const MUT_PERIOD: u64 = 8;
const MIN_TRIALS_BEFORE_REPLACE: u32 = 8;
/// A mutable slot scoring above this is doing useful work and is kept.
const KEEP_SCORE: f32 = 0.3;
/// A corrupted slot whose score has fallen under this, after at least
/// `HEAL_TRIALS` scored predictions, is re-initialised.
const HEAL_SCORE: f32 = 0.05;
const HEAL_TRIALS: u32 = 24;
/// While the waste gate is conservative, still allow one probe prefetch
/// call in this many, so recovery is possible.
const PROBE_EVERY: u32 = 8;
/// A predictor whose own running score is at least this is trusted even while
/// the waste gate is throttling: its accuracy is independently demonstrated
/// (scoring costs no disk reads), so the throttle only holds back the unproven.
/// (Measured before adding: after a phase change the gate tripped on the OLD
/// workload's stale predictors and throttled 70 calls, holding back the correct
/// new predictor -- loop re-learning took 116 reads.)
const SHADOW_TRUST: f32 = 0.8;

pub const LEARNER_LBA: u32 = 1980; // three sectors: 1980, 1981, 1982
const RECORD_BYTES: usize = 1536;
const MAGIC: u32 = 0x4C52_4E31; // "LRN1"

const KIND_SEQ: u8 = 0;
const KIND_STRIDE: u8 = 1;
const KIND_MARKOV: u8 = 2;
const KIND_LAG: u8 = 3;
const KIND_LAGSTRIDE: u8 = 4;

const WASTE_GATE_CONFIG: GateConfig = GateConfig {
    gate_on_threshold: 0.5,
    gate_off_threshold: 0.2,
    fast_tau: 1,
};

#[derive(Clone, Copy)]
struct Slot {
    kind: u8,
    param: u8,
    score: f32,
    pred: u32,
    trials: u32,
    faulty: bool,
}

impl Slot {
    const fn new(kind: u8, param: u8) -> Self {
        Slot { kind, param, score: 0.0, pred: NONE, trials: 0, faulty: false }
    }
}

struct State {
    slots: [Slot; NSLOTS],
    hist: [u32; HIST],
    n: u64,
    markov_tag: [u32; MARKOV_SLOTS],
    markov_next: [u32; MARKOV_SLOTS],
    markov_age: [u32; MARKOV_SLOTS],
    markov_tick: u32,
    rng: u64,
    mutate_on: bool,
    observed: u64,
    mutations: u64,
    heals: u64,
    faults: u64,
}

impl State {
    const fn new() -> Self {
        State {
            slots: [
                Slot::new(KIND_SEQ, 0),
                Slot::new(KIND_STRIDE, 0),
                Slot::new(KIND_MARKOV, 0),
                Slot::new(KIND_LAG, 4),
                Slot::new(KIND_LAGSTRIDE, 3),
                Slot::new(KIND_LAG, 8),
            ],
            hist: [0; HIST],
            n: 0,
            markov_tag: [NONE; MARKOV_SLOTS],
            markov_next: [NONE; MARKOV_SLOTS],
            markov_age: [0; MARKOV_SLOTS],
            markov_tick: 0,
            rng: 0x2545_F491_4F6C_DD1D,
            mutate_on: true,
            observed: 0,
            mutations: 0,
            heals: 0,
            faults: 0,
        }
    }

    fn rnd(&mut self, n: u32) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (((x >> 32) as u32 as u64 * n as u64) >> 32) as u32
    }

    /// The read with absolute index `j` (0-based); the caller guarantees
    /// `j < n` and `n - j <= HIST`.
    fn at(&self, j: u64) -> u32 {
        self.hist[(j % HIST as u64) as usize]
    }

    fn in_range(t: i64) -> u32 {
        if t >= 0 && t < DISK_SECTORS as i64 {
            t as u32
        } else {
            NONE
        }
    }

    /// What slot `s` believes the NEXT demand read (index `n`) will be.
    fn predict(&mut self, s: Slot) -> u32 {
        if s.faulty {
            // a corrupted predictor: emits arbitrary sectors
            return self.rnd(DISK_SECTORS);
        }
        let n = self.n;
        match s.kind {
            KIND_SEQ => {
                if n >= 1 {
                    Self::in_range(self.at(n - 1) as i64 + 1)
                } else {
                    NONE
                }
            }
            KIND_STRIDE => {
                if n >= 2 {
                    let last = self.at(n - 1) as i64;
                    let d = last - self.at(n - 2) as i64;
                    if d != 0 && d >= -128 && d <= 128 {
                        Self::in_range(last + d)
                    } else {
                        NONE
                    }
                } else {
                    NONE
                }
            }
            KIND_MARKOV => {
                if n >= 1 {
                    let last = self.at(n - 1);
                    let base = ((last as usize) % MARKOV_SETS) * MARKOV_WAYS;
                    let mut out = NONE;
                    for w in 0..MARKOV_WAYS {
                        if self.markov_tag[base + w] == last {
                            out = self.markov_next[base + w];
                        }
                    }
                    out
                } else {
                    NONE
                }
            }
            KIND_LAG => {
                let l = s.param as u64;
                if l >= 1 && l <= 63 && n >= l {
                    self.at(n - l)
                } else {
                    NONE
                }
            }
            KIND_LAGSTRIDE => {
                let l = s.param as u64;
                if l >= 1 && 2 * l <= 63 && n >= 2 * l {
                    let a = self.at(n - l) as i64;
                    let b = self.at(n - 2 * l) as i64;
                    Self::in_range(a + (a - b))
                } else {
                    NONE
                }
            }
            _ => NONE,
        }
    }

    /// Re-initialise a slot; a mutable one is re-rolled, a base one keeps
    /// its (fixed) kind.
    fn reinit(&mut self, i: usize) {
        if i >= FIRST_MUTABLE {
            let kind = if self.rnd(2) == 0 { KIND_LAG } else { KIND_LAGSTRIDE };
            let param = 2 + self.rnd(11) as u8;
            self.slots[i] = Slot::new(kind, param);
        } else {
            let k = self.slots[i].kind;
            self.slots[i] = Slot::new(k, 0);
        }
    }

    fn mutate(&mut self) {
        // victim: the lowest-scoring mutable slot that has had enough trials
        let mut victim = NSLOTS;
        let mut vscore = 2.0f32;
        let mut best = NSLOTS;
        let mut bscore = -1.0f32;
        for i in FIRST_MUTABLE..NSLOTS {
            let s = self.slots[i];
            if s.trials >= MIN_TRIALS_BEFORE_REPLACE && s.score < vscore {
                vscore = s.score;
                victim = i;
            }
            if s.score > bscore {
                bscore = s.score;
                best = i;
            }
        }
        if victim == NSLOTS || vscore >= KEEP_SCORE {
            return; // everything eligible is earning its keep
        }
        let (kind, param) = if self.rnd(2) == 0 && best < NSLOTS && bscore > 0.0 {
            // exploit: a small step around the best mutable slot
            let b = self.slots[best];
            let step = if self.rnd(2) == 0 { 1i32 } else { -1i32 };
            let mut p = b.param as i32 + step;
            if p < 1 {
                p = 2;
            }
            if p > 12 {
                p = 12;
            }
            (b.kind, p as u8)
        } else {
            // explore: a random (kind, L)
            (if self.rnd(2) == 0 { KIND_LAG } else { KIND_LAGSTRIDE }, 2 + self.rnd(11) as u8)
        };
        self.slots[victim] = Slot::new(kind, param);
        self.mutations += 1;
    }
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static GATE_ON: AtomicBool = AtomicBool::new(true);
static PROBE: AtomicU32 = AtomicU32::new(0);
// DIAGNOSTIC counters (reset with the learner): idle calls, calls with a trusted
// candidate, candidates already cached, calls throttled by the gate
static DBG_IDLE: AtomicU32 = AtomicU32::new(0);
static DBG_CAND: AtomicU32 = AtomicU32::new(0);
static DBG_CACHED: AtomicU32 = AtomicU32::new(0);
static DBG_THROTTLED: AtomicU32 = AtomicU32::new(0);
static STATE: Mutex<State> = Mutex::new(State::new());

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

/// Benchmark switches: the waste gate and mutation can be ablated.
pub fn configure(gate: bool, mutate: bool) {
    GATE_ON.store(gate, Ordering::SeqCst);
    STATE.lock().mutate_on = mutate;
}

/// Fresh, cold learner (defaults) and a fresh waste gate.
pub fn reset() {
    let mut st = STATE.lock();
    let m = st.mutate_on;
    *st = State::new();
    st.mutate_on = m;
    drop(st);
    evidence_gate::reset(GateKind::PrefetchWaste);
    PROBE.store(0, Ordering::SeqCst);
    DBG_IDLE.store(0, Ordering::SeqCst);
    DBG_CAND.store(0, Ordering::SeqCst);
    DBG_CACHED.store(0, Ordering::SeqCst);
    DBG_THROTTLED.store(0, Ordering::SeqCst);
}

/// Called by `ata::read_sector` for every logical demand read.
pub fn observe(lba: u32) {
    if !ENABLED.load(Ordering::Relaxed) || SUSPEND.load(Ordering::Relaxed) {
        return;
    }
    let mut st = STATE.lock();
    // 1. score every predictor against what actually happened
    for i in 0..NSLOTS {
        let s = st.slots[i];
        if s.pred != NONE {
            let hit = if s.pred == lba { 1.0f32 } else { 0.0f32 };
            st.slots[i].score = s.score * SCORE_KEEP + hit * (1.0 - SCORE_KEEP);
            st.slots[i].trials = s.trials.saturating_add(1);
        } else {
            // Abstaining is not evidence of being right: without this a predictor
            // that stopped predicting when the workload changed kept its old high
            // score forever and could outrank the right one the moment it spoke
            // again (measured: loop re-learning took 113 reads after a phase change).
            st.slots[i].score = s.score * ABSTAIN_DECAY;
        }
    }
    // 2. remember: the Markov successor of the previous read, then the history
    if st.n >= 1 {
        let last = st.at(st.n - 1);
        let base = ((last as usize) % MARKOV_SETS) * MARKOV_WAYS;
        st.markov_tick = st.markov_tick.wrapping_add(1);
        let tick = st.markov_tick;
        // update the way already holding `last`, else replace the least recently used way
        let mut way = MARKOV_WAYS;
        for w in 0..MARKOV_WAYS {
            if st.markov_tag[base + w] == last {
                way = w;
            }
        }
        if way == MARKOV_WAYS {
            way = 0;
            for w in 1..MARKOV_WAYS {
                if st.markov_age[base + w] < st.markov_age[base + way] {
                    way = w;
                }
            }
        }
        st.markov_tag[base + way] = last;
        st.markov_next[base + way] = lba;
        st.markov_age[base + way] = tick;
    }
    let idx = (st.n % HIST as u64) as usize;
    st.hist[idx] = lba;
    st.n += 1;
    st.observed += 1;
    // 3. self-heal a corrupted predictor whose (initially lying) score collapsed
    for i in 0..NSLOTS {
        let s = st.slots[i];
        if s.faulty && s.score < HEAL_SCORE && s.trials >= HEAL_TRIALS {
            st.reinit(i);
            st.heals += 1;
        }
    }
    // 4. variation + selection
    if st.mutate_on && st.observed % MUT_PERIOD == 0 {
        st.mutate();
    }
    // 5. new predictions for the read that comes next
    for i in 0..NSLOTS {
        let s = st.slots[i];
        let p = st.predict(s);
        st.slots[i].pred = p;
    }
}

/// A prefetched sector was either used by a demand read (`wasted=false`)
/// or evicted never having been used (`wasted=true`). Feeds the "do no
/// harm" gate. Takes only atomics and the gate's own lock (never the
/// learner state), so `ata` can call it right after dropping its cache lock.
pub fn on_prefetch_resolved(wasted: bool) {
    if GATE_ON.load(Ordering::Relaxed) {
        let _ = evidence_gate::record_sample(GateKind::PrefetchWaste, &WASTE_GATE_CONFIG, wasted);
    }
}

/// Use idle time to prefetch up to `max` sectors predicted by the
/// currently-trusted predictors. Returns how many disk reads were issued.
pub fn idle_prefetch(max: usize) -> u32 {
    if !ENABLED.load(Ordering::Relaxed) {
        return 0;
    }
    DBG_IDLE.fetch_add(1, Ordering::Relaxed);
    // collect (score, lba) for trusted predictors, best first, distinct
    let mut cand: [(f32, u32); NSLOTS] = [(-1.0, NONE); NSLOTS];
    let mut nc = 0usize;
    {
        let st = STATE.lock();
        for i in 0..NSLOTS {
            let s = st.slots[i];
            if s.pred != NONE && s.score >= MIN_CONF {
                cand[nc] = (s.score, s.pred);
                nc += 1;
            }
        }
    }
    if nc > 0 {
        DBG_CAND.fetch_add(1, Ordering::Relaxed);
    }
    // Throttle: while the waste gate is conservative only occasional probes are
    // allowed -- unless the best candidate has proven itself (SHADOW_TRUST).
    if nc > 0 && GATE_ON.load(Ordering::Relaxed) && evidence_gate::is_conservative(GateKind::PrefetchWaste) {
        let mut best = 0.0f32;
        for i in 0..nc {
            if cand[i].0 > best {
                best = cand[i].0;
            }
        }
        if best < SHADOW_TRUST {
            let p = PROBE.fetch_add(1, Ordering::Relaxed) + 1;
            if p % PROBE_EVERY != 0 {
                DBG_THROTTLED.fetch_add(1, Ordering::Relaxed);
                return 0;
            }
        }
    }
    // insertion sort by score, descending
    for i in 1..nc {
        let mut j = i;
        while j > 0 && cand[j].0 > cand[j - 1].0 {
            cand.swap(j, j - 1);
            j -= 1;
        }
    }
    let mut issued = 0u32;
    let mut done = [NONE; NSLOTS];
    let mut nd = 0usize;
    for i in 0..nc {
        if nd >= max {
            break;
        }
        let lba = cand[i].1;
        if done[..nd].contains(&lba) {
            continue;
        }
        done[nd] = lba;
        nd += 1;
        if ata::prefetch_sector(lba) {
            issued += 1;
        } else {
            DBG_CACHED.fetch_add(1, Ordering::Relaxed);
        }
    }
    issued
}

/// Adversarial test hook: corrupt slot `i` so it emits arbitrary sectors
/// while advertising a high confidence score.
pub fn inject_fault(i: usize) {
    let mut st = STATE.lock();
    if i < NSLOTS {
        st.slots[i].faulty = true;
        st.slots[i].score = 1.0; // maximally adversarial: perfectly confident
        st.slots[i].trials = 0;
        st.faults += 1;
    }
}

pub fn debug_counters() -> (u32, u32, u32, u32) {
    (DBG_IDLE.load(Ordering::Relaxed), DBG_CAND.load(Ordering::Relaxed), DBG_CACHED.load(Ordering::Relaxed), DBG_THROTTLED.load(Ordering::Relaxed))
}

#[allow(dead_code)]
pub struct Stats {
    pub observed: u64,
    pub mutations: u64,
    pub heals: u64,
    pub faults: u64,
    pub best_kind: u8,
    pub best_param: u8,
    pub best_score: f32,
    pub scores: [f32; NSLOTS],
    pub kinds: [u8; NSLOTS],
    pub params: [u8; NSLOTS],
}

pub fn stats() -> Stats {
    let st = STATE.lock();
    let mut best = 0usize;
    for i in 1..NSLOTS {
        if st.slots[i].score > st.slots[best].score {
            best = i;
        }
    }
    let mut scores = [0.0f32; NSLOTS];
    let mut kinds = [0u8; NSLOTS];
    let mut params = [0u8; NSLOTS];
    for i in 0..NSLOTS {
        scores[i] = st.slots[i].score;
        kinds[i] = st.slots[i].kind;
        params[i] = st.slots[i].param;
    }
    Stats {
        observed: st.observed,
        mutations: st.mutations,
        heals: st.heals,
        faults: st.faults,
        best_kind: st.slots[best].kind,
        best_param: st.slots[best].param,
        best_score: st.slots[best].score,
        scores,
        kinds,
        params,
    }
}

// ---------------------------------------------------------------- memory

fn checksum(buf: &[u8]) -> u32 {
    // position-dependent rotating sum: detects a flipped byte and a swap
    let mut h: u32 = 0x811C_9DC5;
    for (i, &b) in buf.iter().enumerate() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193).rotate_left((i as u32 % 7) + 1);
    }
    h
}

fn put_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn get_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// The learned state as the 1536-byte record `save()` writes.
pub fn serialize() -> [u8; RECORD_BYTES] {
    let st = STATE.lock();
    let mut b = [0u8; RECORD_BYTES];
    put_u32(&mut b, 0, MAGIC);
    put_u32(&mut b, 4, NSLOTS as u32);
    for i in 0..NSLOTS {
        let o = 8 + i * 12;
        put_u32(&mut b, o, st.slots[i].kind as u32);
        put_u32(&mut b, o + 4, st.slots[i].param as u32);
        put_u32(&mut b, o + 8, st.slots[i].score.to_bits());
    }
    let mo = 8 + NSLOTS * 12; // 80
    for i in 0..MARKOV_SLOTS {
        put_u32(&mut b, mo + i * 8, st.markov_tag[i]);
        put_u32(&mut b, mo + i * 8 + 4, st.markov_next[i]);
    }
    let end = mo + MARKOV_SLOTS * 8; // 1104
    let sum = checksum(&b[..end]);
    put_u32(&mut b, end, sum);
    b
}

pub fn save() -> bool {
    save_record(&serialize())
}

/// Reads/writes of the record itself must not be counted as workload reads.
static SUSPEND: AtomicBool = AtomicBool::new(false);

/// Validate and apply a saved record. Returns false (state untouched) for
/// anything that is not a well-formed record of exactly this format.
pub fn load() -> bool {
    let was = SUSPEND.swap(true, Ordering::SeqCst);
    let mut b = [0u8; RECORD_BYTES];
    let mut ok = true;
    for k in 0..3usize {
        let mut sec = [0u8; 512];
        if ata::read_sector(LEARNER_LBA + k as u32, &mut sec).is_err() {
            ok = false;
        }
        b[k * 512..(k + 1) * 512].copy_from_slice(&sec);
    }
    SUSPEND.store(was, Ordering::SeqCst);
    if !ok {
        return false;
    }
    let mo = 8 + NSLOTS * 12;
    let end = mo + MARKOV_SLOTS * 8;
    if get_u32(&b, 0) != MAGIC || get_u32(&b, 4) != NSLOTS as u32 || get_u32(&b, end) != checksum(&b[..end]) {
        return false;
    }
    // sanity-check every field before touching live state
    let mut slots = [Slot::new(KIND_SEQ, 0); NSLOTS];
    for i in 0..NSLOTS {
        let o = 8 + i * 12;
        let kind = get_u32(&b, o);
        let param = get_u32(&b, o + 4);
        let score = f32::from_bits(get_u32(&b, o + 8));
        if kind > KIND_LAGSTRIDE as u32 || param > 63 || !(score >= 0.0 && score <= 1.0) {
            return false;
        }
        slots[i] = Slot { kind: kind as u8, param: param as u8, score, pred: NONE, trials: MIN_TRIALS_BEFORE_REPLACE, faulty: false };
    }
    let mut tag = [NONE; MARKOV_SLOTS];
    let mut next = [NONE; MARKOV_SLOTS];
    for i in 0..MARKOV_SLOTS {
        let t = get_u32(&b, mo + i * 8);
        let n = get_u32(&b, mo + i * 8 + 4);
        if (t != NONE && t >= DISK_SECTORS) || (n != NONE && n >= DISK_SECTORS) {
            return false;
        }
        tag[i] = t;
        next[i] = n;
    }
    let mut st = STATE.lock();
    st.slots = slots;
    st.markov_tag = tag;
    st.markov_next = next;
    st.markov_age = [0; MARKOV_SLOTS];
    st.markov_tick = 0;
    true
}

// ------------------------------------------------------------- benchmark

fn tsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

fn lcg(x: &mut u64) -> u32 {
    *x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (*x >> 33) as u32
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cond {
    A,  // no learner, no prefetch
    B,  // conventional readahead: always prefetch lba+1
    C,  // learner, no waste gate, with mutation
    D,  // learner, waste gate, with mutation
    E,  // learner, waste gate, NO mutation (ablation)
}

const MAXW: usize = 400;

struct Workload {
    lbas: [u32; MAXW],
    n: usize,
}

impl Workload {
    fn new() -> Self {
        Workload { lbas: [0; MAXW], n: 0 }
    }
    fn push(&mut self, l: u32) {
        if self.n < MAXW {
            self.lbas[self.n] = l;
            self.n += 1;
        }
    }
}

struct RunResult {
    crit: u64,   // cycles inside demand reads (the critical path)
    hits: u64,
    reads: u64,
    pf_issued: u64,
    pf_useful: u64,
    pf_wasted: u64,
    bad: u32,    // demand reads that returned the wrong bytes
}

fn fill_pattern(lba: u32) {
    let mut s = [0u8; 512];
    s[0] = (lba & 0xFF) as u8;
    s[1] = (lba >> 8) as u8;
    s[2] = 0xA5;
    let _ = ata::write_sector(lba, &s);
}

fn setup_cond(cond: Cond) {
    ata::cache_reset();
    ata::cache_set_mode(ata::CacheMode::AlwaysOn);
    reset();
    match cond {
        Cond::A | Cond::B => set_enabled(false),
        Cond::C => {
            configure(false, true);
            set_enabled(true);
        }
        Cond::D => {
            configure(true, true);
            set_enabled(true);
        }
        Cond::E => {
            configure(true, false);
            set_enabled(true);
        }
    }
}

/// Run one read: returns (critical cycles, was_hit, data_ok).
fn demand_read(lba: u32) -> (u64, bool, bool) {
    let before = ata::cache_stats().1;
    let mut buf = [0u8; 512];
    let t0 = tsc();
    let ok = ata::read_sector(lba, &mut buf).is_ok();
    let dt = tsc() - t0;
    let hit = ata::cache_stats().1 > before;
    let good = ok && buf[0] == (lba & 0xFF) as u8 && buf[1] == (lba >> 8) as u8 && buf[2] == 0xA5;
    (dt, hit, good)
}

fn idle_hook(cond: Cond, lba: u32) {
    match cond {
        Cond::A => {}
        Cond::B => {
            if lba + 1 < DISK_SECTORS {
                let _ = ata::prefetch_sector(lba + 1);
            }
        }
        _ => {
            let _ = idle_prefetch(1);
        }
    }
}

fn run(w: &Workload, cond: Cond) -> RunResult {
    setup_cond(cond);
    let mut r = RunResult { crit: 0, hits: 0, reads: 0, pf_issued: 0, pf_useful: 0, pf_wasted: 0, bad: 0 };
    for i in 0..w.n {
        let lba = w.lbas[i];
        let (dt, hit, good) = demand_read(lba);
        r.crit += dt;
        r.reads += 1;
        if hit {
            r.hits += 1;
        }
        if !good {
            r.bad += 1;
        }
        idle_hook(cond, lba);
    }
    let (iss, useful, wasted) = ata::prefetch_stats();
    r.pf_issued = iss;
    r.pf_useful = useful;
    r.pf_wasted = wasted;
    r
}

fn pct(part: u64, whole: u64) -> u64 {
    if whole == 0 { 0 } else { part * 100 / whole }
}

pub fn self_test_learner() -> bool {
    // ---- workloads (sectors are pre-filled so every read can be verified)
    let mut w1 = Workload::new(); // sequential scan
    for i in 0..120u32 {
        w1.push(1000 + i);
    }
    let mut w2 = Workload::new(); // stride 7
    for i in 0..80u32 {
        w2.push(200 + 7 * i);
    }
    let mut w3 = Workload::new(); // 40-sector loop, 8 passes: thrashes the 32-line LRU
    let mut seed = 0xC0FFEEu64;
    let mut list = [0u32; 40];
    let mut k = 0usize;
    while k < 40 {
        let c = 300 + lcg(&mut seed) % 1400;
        if !list[..k].contains(&c) {
            list[k] = c;
            k += 1;
        }
    }
    for _pass in 0..8 {
        for i in 0..40 {
            w3.push(list[i]);
        }
    }
    let mut w4 = Workload::new(); // unpredictable
    let mut seed4 = 0xBEEF01u64;
    for _ in 0..100 {
        w4.push(300 + lcg(&mut seed4) % 1600);
    }
    let mut w5 = Workload::new(); // two interleaved sequential streams
    for i in 0..150u32 {
        w5.push(1200 + i);
        w5.push(1500 + i);
    }
    let all = [&w1, &w2, &w3, &w4, &w5];
    for w in all.iter() {
        for i in 0..w.n {
            fill_pattern(w.lbas[i]);
        }
    }

    // ---- conditions x workloads
    let conds = [Cond::A, Cond::B, Cond::C, Cond::D, Cond::E];
    let mut res: [[Option<RunResult>; 5]; 5] = [[None, None, None, None, None], [None, None, None, None, None], [None, None, None, None, None], [None, None, None, None, None], [None, None, None, None, None]];
    let mut bad_total = 0u32;
    let mut dbg3 = (0u32, 0u32, 0u32, 0u32);
    let mut sc3 = [0.0f32; NSLOTS];
    let mut kd3 = [0u8; NSLOTS];
    let mut pm3 = [0u8; NSLOTS];
    for (wi, w) in all.iter().enumerate() {
        for (ci, &c) in conds.iter().enumerate() {
            // E (no mutation) is only needed where mutation matters
            if c == Cond::E && wi != 4 {
                continue;
            }
            let r = run(w, c);
            if wi == 2 && c == Cond::D {
                dbg3 = debug_counters();
                let st = stats();
                sc3 = st.scores;
                kd3 = st.kinds;
                pm3 = st.params;
            }
            bad_total += r.bad;
            res[wi][ci] = Some(r);
        }
    }
    let g = |wi: usize, ci: usize| -> &RunResult { res[wi][ci].as_ref().unwrap() };

    // ---- pre-registered performance predictions
    let a = |wi: usize| g(wi, 0).crit;
    let p1 = g(0, 1).crit * 5 <= a(0) && g(0, 2).crit * 5 <= a(0) && g(0, 3).crit * 5 <= a(0);
    let p2 = g(1, 1).crit * 10 >= a(1) * 8 && g(1, 2).crit * 4 <= a(1) && g(1, 3).crit * 4 <= a(1);
    let p3 = g(2, 1).crit * 10 >= a(2) * 9 && g(2, 2).crit * 20 <= a(2) * 7 && g(2, 3).crit * 20 <= a(2) * 7;
    let p4 = g(3, 3).pf_wasted <= 15 && g(3, 3).crit * 100 <= a(3) * 105 && g(3, 1).pf_wasted >= 90;
    let p5 = g(4, 3).crit * 2 <= a(4) && g(4, 4).crit * 10 >= a(4) * 9;

    // ---- R1: adaptation across phase changes (D, no reset between phases)
    setup_cond(Cond::D);
    let mut adapt = [0u32; 3];
    let mut gate_probe = [(0u32, 0u64, false); 3];
    let phases = [&w1, &w2, &w3];
    for (pi, w) in phases.iter().enumerate() {
        let mut win = [false; 8];
        let mut found = 0u32;
        let limit = if pi == 2 { 120 } else { 60 };
        for i in 0..(w.n.min(limit)) {
            let (_, hit, _) = demand_read(w.lbas[i]);
            win[i % 8] = hit;
            if found == 0 && i >= 7 && win.iter().filter(|&&x| x).count() >= 4 {
                found = i as u32 + 1;
            }
            idle_hook(Cond::D, w.lbas[i]);
        }
        adapt[pi] = if found == 0 { 9999 } else { found };
        gate_probe[pi] = (debug_counters().3, evidence_gate::stats(GateKind::PrefetchWaste).1, evidence_gate::is_conservative(GateKind::PrefetchWaste));
    }
    let r1 = adapt[0] <= 16 && adapt[1] <= 24 && adapt[2] <= 64;

    // ---- R2: fault injection mid-run. Corrupt BOTH predictors that were
    // carrying a sequential scan (SEQ and STRIDE), one of them lying with
    // a high score, and require the run to keep returning correct data and
    // to recover on its own.
    setup_cond(Cond::D);
    let mut hits_after = [false; 120];
    let mut bad2 = 0u32;
    for i in 0..w1.n {
        if i == 30 {
            inject_fault(0);
            inject_fault(1);
        }
        let (_, hit, good) = demand_read(w1.lbas[i]);
        if !good {
            bad2 += 1;
        }
        hits_after[i] = hit;
        idle_hook(Cond::D, w1.lbas[i]);
    }
    let heals = stats().heals;
    let mut recovered_at = 9999u32;
    for i in 38..w1.n {
        let mut c = 0;
        for j in (i - 7)..=i {
            if hits_after[j] {
                c += 1;
            }
        }
        if c >= 6 && i > 30 {
            recovered_at = (i - 30) as u32;
            break;
        }
    }
    let r2 = recovered_at <= 45 && heals >= 2 && bad2 == 0;

    // ---- R3: memory. cold vs warm first pass of the loop, corrupt-record
    // refusal, and an exact save/load/save round trip.
    setup_cond(Cond::D);
    for i in 0..w3.n {
        let _ = demand_read(w3.lbas[i]);
        idle_hook(Cond::D, w3.lbas[i]);
    }
    let saved = save();
    let rec1 = serialize();
    // cold: fresh learner, first pass of 40
    setup_cond(Cond::D);
    let mut cold_hits = 0u32;
    for i in 0..40 {
        let (_, h, _) = demand_read(w3.lbas[i]);
        if h {
            cold_hits += 1;
        }
        idle_hook(Cond::D, w3.lbas[i]);
    }
    // warm: fresh learner + load, first pass of 40
    setup_cond(Cond::D);
    let loaded = load();
    let mut warm_hits = 0u32;
    for i in 0..40 {
        let (_, h, _) = demand_read(w3.lbas[i]);
        if h {
            warm_hits += 1;
        }
        idle_hook(Cond::D, w3.lbas[i]);
    }
    // round trip: load the saved record into a fresh learner and re-serialize
    setup_cond(Cond::D);
    let _ = load();
    let rec2 = serialize();
    let round_trip = rec1[..] == rec2[..];
    // corruption: flip one byte of the stored record; load must refuse and
    // must leave a fresh learner untouched
    let mut s0 = [0u8; 512];
    let _ = ata::read_sector(LEARNER_LBA, &mut s0);
    s0[100] ^= 0x5A;
    let _ = ata::write_sector(LEARNER_LBA, &s0);
    setup_cond(Cond::D);
    let before = serialize();
    let refused = !load();
    let untouched = before[..] == serialize()[..];
    // restore the good record so a corrupt one is not left on disk
    let _ = save_record(&rec1);
    let r3 = saved && loaded && warm_hits >= 25 && cold_hits <= 4 && round_trip && refused && untouched;

    // leave the shipped state: learner off, cache defaults
    set_enabled(false);
    configure(true, true);
    reset();
    ata::cache_reset();
    ata::cache_restore_default();

    let correct = bad_total == 0 && bad2 == 0;
    // PASS gates on the GUARANTEES (correct data, self-repair, exact memory, corrupt
    // record refused). The performance predictions are pre-registered measurements:
    // reported met/missed in the ledger, not tuned until they pass.
    let met = [p1, p2, p3, p4, p5, r1].iter().filter(|&&x| x).count();
    let ok = correct && r2 && r3;
    let mut out = serial();
    let _ = writeln!(
        out,
        "milestone 162: disk-read learner -- data correctness: bad_reads={bad_total} (fault run {bad2}) -- CRITICAL-PATH cycles (M), A=none/B=readahead+1/C=learner-nogate/D=learner+gate(+E=no-mutation):"
    );
    let names = ["W1 sequential", "W2 stride-7", "W3 loop-40 x8", "W4 random", "W5 interleaved"];
    for wi in 0..5 {
        let _ = writeln!(
            out,
            "milestone 162:   {} -- A={} B={} C={} D={} | hit% A/B/C/D={}/{}/{}/{} | prefetch issued/useful/wasted B={}/{}/{} C={}/{}/{} D={}/{}/{}",
            names[wi],
            g(wi, 0).crit / 1_000_000,
            g(wi, 1).crit / 1_000_000,
            g(wi, 2).crit / 1_000_000,
            g(wi, 3).crit / 1_000_000,
            pct(g(wi, 0).hits, g(wi, 0).reads),
            pct(g(wi, 1).hits, g(wi, 1).reads),
            pct(g(wi, 2).hits, g(wi, 2).reads),
            pct(g(wi, 3).hits, g(wi, 3).reads),
            g(wi, 1).pf_issued, g(wi, 1).pf_useful, g(wi, 1).pf_wasted,
            g(wi, 2).pf_issued, g(wi, 2).pf_useful, g(wi, 2).pf_wasted,
            g(wi, 3).pf_issued, g(wi, 3).pf_useful, g(wi, 3).pf_wasted,
        );
        if wi == 4 {
            let _ = writeln!(out, "milestone 162:   W5 no-mutation ablation E={}M", g(4, 4).crit / 1_000_000);
        }
    }
    let _ = writeln!(
        out,
        "milestone 162: DIAG W3/D idle_calls={} with_trusted_candidate={} candidate_already_cached={} gate_throttled={} | slots kind:param:score = {}:{}:{:.2} {}:{}:{:.2} {}:{}:{:.2} {}:{}:{:.2} {}:{}:{:.2} {}:{}:{:.2}",
        dbg3.0, dbg3.1, dbg3.2, dbg3.3,
        kd3[0], pm3[0], sc3[0], kd3[1], pm3[1], sc3[1], kd3[2], pm3[2], sc3[2], kd3[3], pm3[3], sc3[3], kd3[4], pm3[4], sc3[4], kd3[5], pm3[5], sc3[5]
    );
    let _ = writeln!(
        out,
        "milestone 162: DIAG R1 end-of-phase (gate_throttled_calls, waste_gate_trip_episodes, conservative_now) W1={:?} W2={:?} W3={:?}",
        gate_probe[0], gate_probe[1], gate_probe[2]
    );
    let _ = writeln!(
        out,
        "milestone 162: adaptation (reads until 4 of 8 recent are cache hits) W1={} W2={} W3={} -- fault injection: recovered after {} reads, heals={heals} -- memory: cold first-pass hits={cold_hits}/40 warm={warm_hits}/40 round_trip={round_trip} corrupt_refused={refused} state_untouched={untouched}",
        adapt[0], adapt[1], adapt[2], recovered_at
    );
    let _ = writeln!(
        out,
        "milestone 162: performance predictions met {met}/6: P1(seq)={p1} P2(stride)={p2} P3(loop)={p3} P4(random,do-no-harm)={p4} P5(interleave+mutation)={p5} R1(adapt)={r1} -- guarantees: correctness={correct} R2(fault-repair)={r2} R3(memory)={r3} -- {}",
        if ok { "PASS" } else { "FAIL" }
    );
    ok
}

/// Write a serialized record to its three sectors.
fn save_record(b: &[u8; RECORD_BYTES]) -> bool {
    let mut ok = true;
    for k in 0..3usize {
        let mut sec = [0u8; 512];
        sec.copy_from_slice(&b[k * 512..(k + 1) * 512]);
        ok &= ata::write_sector(LEARNER_LBA + k as u32, &sec).is_ok();
    }
    ok
}
