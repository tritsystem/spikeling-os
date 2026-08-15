# Milestone 48: wiring `ternary.rs` into a real kernel decision path

## What `ternary.rs` actually was before this milestone

Read the file first, per the task's own instruction, rather than assuming.
`kernel/src/ternary.rs` (Milestone 38) is balanced-ternary weight
*compression*, not a general trit type used anywhere live: `pack_ternary`/
`unpack_ternary` pack trits `{-1,0,+1}` 5-to-a-byte, and
`encode_weight`/`decode_weight` use that to store one `f32` STDP synapse
weight as `TRITS_PER_WEIGHT` (10) base-3 fixed-point digits. Every call
site of this module before today (`ata.rs`, `main.rs`, `neurons.rs`) is on
the disk save/load boundary only -- the module's own header says so
explicitly ("Applies ONLY at the save-to-disk / load-from-disk boundary").
Nothing in the kernel used a trit-valued decision anywhere at runtime.

## What I built

Added `ternary::compare_trit(a: f32, b: f32, epsilon: f32) -> i8`
(`kernel/src/ternary.rs`) -- this module's first *live* (not
persistence-boundary) function. It returns a real trit: `+1` if `a` is
clearly greater than `b`, `-1` if clearly less, `0` if the two are within
`epsilon` -- an honest "tied" outcome that plain `f32::partial_cmp` (a
strictly binary less/greater-or-equal split) has no way to express as a
distinct case.

I wired this into `scheduler.rs`'s `TopologicalScheduler::step()` --
the exact function `tasks.rs`'s real, timer-interrupt-driven preemptive
scheduler calls every PIT tick to decide which task-slot runs next
(`tasks::timer_tick_switch()` -> `SCHED.step()`). This is genuinely live,
observable kernel scheduling code, not a demo copy: it is what actually
moves execution between the three-to-eight real worker tasks once
`tasks::enable_background_scheduling()` flips on after boot.

Previously, `step()`'s winner selection was:
```rust
self.slots.iter().enumerate()
    .filter(|(_, s)| s.alive && s.potential >= self.threshold)
    .max_by(|(_, a), (_, b)| a.potential.partial_cmp(&b.potential).unwrap())
    .map(|(i, _)| i)
```
a plain binary float comparison. On an exact or near-exact tie,
`Iterator::max_by` silently returns the *last* equally-maximum element --
an implicit, fairness-blind "highest slot index wins" rule baked into
ordinary binary comparison, not a deliberate policy; nobody chose it, it's
just what `max_by` does.

The new real path (`select_winner_ternary`, called by `step()`) walks
eligible candidates pairwise against the current best using
`ternary::compare_trit(candidate.potential, best.potential, TIE_EPSILON)`:
`+1`/`-1` keep the clearly-larger candidate exactly as before, but `0` (a
genuine within-epsilon tie) now falls through to a real fairness rule --
the slot with **fewer total fires so far** wins the tie -- instead of
`max_by`'s arbitrary last-wins default. `TIE_EPSILON = 0.01`, chosen
relative to the scheduler's own real per-tick scale (`BIAS = 0.15`/tick,
lateral coupling damped to `0.05 * bond * neighbor-potential`), so it
catches genuine near-coincidences produced by the coupled dynamics without
swallowing clearly-separated potentials into spurious ties.

I kept the old binary comparison too, as `select_winner_binary()` /
`step_binary()` -- explicitly documented as "not used anywhere in the real
scheduling path", existing only so the two policies can be measured
side by side over identical accumulated potential dynamics. `accumulate()`
(the per-tick bias+coupling update) was factored out so both `step()` and
`step_binary()` share it exactly; they differ *only* in winner selection.

A boot-time A/B trial in `kernel/src/main.rs`, modeled directly on
Milestone 4's own existing `run_trial` pattern right above it in the same
function, runs both policies for 4000 ticks over 8 slots at `g=0.6` --
the same slot cap (`tasks::MAX_TASKS`) and coupling
(`tasks::run_preemption_demo()`'s real `TopologicalScheduler::new(N_TASKS,
0.6)`) the real scheduler actually uses -- and logs real fire-count
fairness (`min/max`) for both, plus how many ticks the ternary tiebreak
actually engaged.

## What I verified, and how (real commands, real output)

Baseline build, before touching anything, to confirm a clean starting
point:
```
cargo build
...
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 47s
```
(two pre-existing, unrelated warnings only: `ALTENTRY_TEST_ELF_BYTES`
never used, `extra_frames` never read -- both leftover from Milestone 44's
disclosed in-progress state, not touched by this milestone.)

After the changes, rebuilt clean with the same two pre-existing warnings
and nothing new:
```
cargo build
...
Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.60s
```

Booted for real in QEMU (`cargo run -- bios`, serial piped to a real log
file, per this project's own established discipline -- sendkey/interactive
testing is unreliable here per the README's Milestone 39/40 notes, so this
reads a real captured serial log instead):
```
cargo run -- bios > m48_boot_final.log 2>&1
```
Actual captured serial output (verbatim, `TIE_EPSILON = 0.01`, the real,
justified value):
```
milestone 4: kernel survived defect injection at slot 3 -- 7 slots still alive and scheduled, no panic
milestone 4: topological (g=+0.6) post-defect fire counts: min=289 max=366 fairness=0.790
milestone 4: trivial     (g=-0.6) post-defect fire counts: min=299 max=341 fairness=0.877
milestone 4: result -- topological coupling did NOT stay fairer under the defect here (reporting honestly, not the assumed outcome)
milestone 48: 4000 ticks over 8 slots (g=0.6, real tasks.rs coupling) -- ternary tiebreak (compare_trit within epsilon=0.01) actually fired on 2 of 4000 ticks
milestone 48: ternary selection  -- fire counts min=399 max=533 total=3995 fairness=0.7486
milestone 48: binary   selection -- fire counts min=399 max=533 total=3995 fairness=0.7486
milestone 48: result -- ternary tiebreak engaged but made no measurable fairness difference here -- a real neutral result, not an assumed win
```
Reproduced identically on a second, independent boot (the dynamics are
fully deterministic -- same numbers both times, not cherry-picked).

Confirmed nothing regressed elsewhere in the same boot: every self-test
with an explicit pass/fail verdict still reports it correctly --
```
milestone 42: self-test -- OVERALL: PASS
milestone 43: self-test -- OVERALL: PASS
```
-- and a full-log scan for `panic!`, `kernel panic`, `double fault`, or
any bare `FAIL` found none; boot proceeded normally through to the
interactive shell prompt (the run then blocks waiting for input, which is
why the QEMU process was time-bounded to capture the log, per this
project's own established sendkey-unreliability workaround, not a hang).

**Real sanity check that the tie-break logic actually does something,
rather than silently being a no-op that happens to match `max_by` every
time**: I temporarily widened `TIE_EPSILON` to `0.2` (an intentionally
oversized value, purely diagnostic, not the reported result), rebuilt, and
rebooted:
```
milestone 48: 4000 ticks over 8 slots (g=0.6, real tasks.rs coupling) -- ternary tiebreak (compare_trit within epsilon=0.2) actually fired on 2996 of 4000 ticks
milestone 48: ternary selection  -- fire counts min=499 max=500 total=3995 fairness=0.9980
milestone 48: binary   selection -- fire counts min=399 max=533 total=3995 fairness=0.7486
milestone 48: result -- ternary tiebreak's fairness rule (fewest fire_count wins a tie) measurably improved fire-count fairness over the binary path's arbitrary last-wins tie rule
```
This confirms the ternary decision path is genuinely wired in and
functionally live -- given enough real ties to work with, it measurably
changes scheduling fairness (0.749 -> 0.998). I then reverted
`TIE_EPSILON` back to `0.01` (the value actually justified by the
scheduler's real per-tick dynamics, not the one that produces the nicest
headline number) and rebuilt/rebooted once more to confirm the reported
result above is the real, final, committed state -- identical output to
the first boot, confirmed byte-for-byte in the log.

## The honest result

At `TIE_EPSILON = 0.01` -- the value actually derived from this
scheduler's own real per-tick scale, not picked after the fact to produce
a favorable number -- the ternary tiebreak engaged on only 2 of 4000 real
ticks, and the two resulting fire-count distributions were **identical**
(`min=399 max=533 total=3995`, `fairness=0.7486`, both paths, both boot
runs). The diagnostic run at an artificially widened epsilon proves the
mechanism is real and can move fairness substantially (0.749 -> 0.998)
when ties are common enough to matter -- but under the real, honestly-
derived coupling dynamics of this scheduler, near-exact float ties between
two slots' potentials are simply rare (0.05%), so the ternary decision's
practical effect on this kernel's actual scheduling fairness, as actually
measured, is **negligible-to-none**. This is a fully reported, honest
neutral result -- same discipline as this repo's own Milestone 4 finding
immediately above it in the same boot log ("topological coupling did NOT
stay fairer under the defect here"), not an assumed advantage for
ternary decision-making dressed up as a win.

## Honest scope-cuts

- The ternary decision is scoped to `TopologicalScheduler::step()`'s
  winner-selection tiebreak only. I considered and rejected a syscall-
  dispatch-path ternary decision (e.g. a three-way allow/deny/defer
  authorization outcome for `setpgid`/`kill`) as a second candidate site,
  because those authorization checks are already correctly binary
  (permitted or refused -- there is no genuine third real outcome to
  model there, and inventing one just to use `ternary.rs` would not be a
  *genuine* ternary-valued decision, which the task explicitly asked to
  avoid). The scheduler tiebreak was the one site in the kernel where a
  real three-way (clearly-greater / clearly-less / genuinely-tied) outcome
  already existed conceptually but was being silently collapsed into a
  binary comparison by `max_by` -- a real fit, not a manufactured one.
- `select_winner_binary()`/`step_binary()` are dead code from the real
  scheduling path's point of view by design -- kept alive only for this
  milestone's own A/B measurement and documented as such at every call
  site, not left as an undisclosed duplicate implementation.
- `ternary::compare_trit`'s `epsilon` tie window is a single, scheduler-
  scale-derived constant (`TIE_EPSILON`), not adaptive/learned -- an
  honest, simple choice appropriate to this milestone's scope; a fancier
  adaptive epsilon was not attempted.
