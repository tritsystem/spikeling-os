# Milestone 48 verification -- independent review

Reviewer note: this is an independent check, not a re-implementation. I did
not use Edit/Write; every claim below was checked by reading the current
working-tree source and by running cargo build / cargo run -- bios
myself, fresh, with my own log files (m48_review_boot1.log,
m48_review_boot2.log, both in the repo root, not reused from the
implementer).

## 1. What ternary.rs actually was before this milestone -- claim check

Claim: before this milestone, every call site of ternary.rs
(pack_ternary/unpack_ternary/encode_weight/decode_weight) was on the
disk save/load boundary only; nothing used a trit-valued decision at
runtime.

REPRODUCED. Read kernel/src/ternary.rs in full. The module header
(lines 1-73) documents exactly this scope ("Applies ONLY at the
save-to-disk / load-from-disk boundary", line 62), and compare_trit
(lines 178-187) is clearly marked as the new, first live decision
primitive added by this milestone. The pre-existing functions
(pack_ternary, unpack_ternary, encode_weight, decode_weight) are
unchanged persistence-boundary code; git diff --stat shows only additive
changes to this file (+31 lines).

## 2. compare_trit -- claim check

Claim: ternary::compare_trit(a: f32, b: f32, epsilon: f32) -> i8
returns +1/-1/0 (clearly-greater / clearly-less / within-epsilon-tied).

REPRODUCED. kernel/src/ternary.rs:178-187:
```rust
pub fn compare_trit(a: f32, b: f32, epsilon: f32) -> i8 {
    let diff = a - b;
    if diff > epsilon { 1 } else if diff < -epsilon { -1 } else { 0 }
}
```
Matches the claim exactly.

## 3. Wiring into the real scheduler path -- claim check

Claim: scheduler.rs::TopologicalScheduler::step() -- the function
tasks.rs's real timer-interrupt-driven scheduler calls every tick via
SCHED.step() -- now uses select_winner_ternary() (built on
compare_trit), not the old plain max_by binary comparison.
select_winner_binary()/step_binary() are kept only for the A/B trial
and are not called from the real scheduling path.

REPRODUCED.
- kernel/src/interrupts.rs:106 -> crate::tasks::timer_tick_switch();
- kernel/src/tasks.rs:373 -> let next = SCHED.lock().as_mut().and_then(|s| s.step());
- kernel/src/scheduler.rs:229-240 -- step() calls accumulate() then
  select_winner_ternary() (lines 199-223), which walks candidates
  pairwise via crate::ternary::compare_trit(...) and, on trit 0, picks
  the slot with fewer fire_count (line 213: if self.slots[i].fire_count
  < self.slots[b].fire_count).
- Grepped the whole kernel/src/ tree for step_binary/
  select_winner_binary: the only call sites are main.rs:503
  (sched.step_binary() inside the boot-time A/B trial) and internal to
  scheduler.rs itself. tasks.rs calls step() only. Confirmed
  select_winner_binary/step_binary are genuinely dead from the real
  scheduling path's perspective, exactly as claimed.

## 4. Build claim

Claim: cargo build succeeds clean, same two pre-existing unrelated
warnings (ALTENTRY_TEST_ELF_BYTES unused, extra_frames unread), no new
warnings, ~4.6s incremental.

REPRODUCED, with one wrinkle worth flagging: running cargo build
inside kernel/ directly fails (error: unwinding panics are not
supported without std -- the kernel target needs the workspace-root
.cargo/config.toml's build-std setup). Running it from the repo
root (C:\Users\gbran\OneDrive\Documents\spikeling-os-wt-m48, where
.cargo/config.toml and the top-level Cargo.toml live) succeeds:
```
$ cargo build
warning: static `ALTENTRY_TEST_ELF_BYTES` is never used --> kernel\src\loader.rs:402:8
warning: field `extra_frames` is never read --> kernel\src\process.rs:573:5
warning: `kernel` (bin "kernel") generated 2 warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.62s
```
Exactly the two warnings claimed, nothing new, from the correct
(root) working directory. Not a discrepancy in the implementer's account
(they didn't specify a directory and their own log shows the same root
invocation via cargo run -- bios), just noting it here in case a future
checker hits the same trap.

## 5. Boot / serial log claims

Claim: booting via cargo run -- bios, piped to a real log file,
produces the exact serial lines quoted in the implementer's account,
reproducibly across two independent boots, at TIE_EPSILON = 0.01:
```
milestone 48: 4000 ticks over 8 slots (g=0.6, real tasks.rs coupling) -- ternary tiebreak (compare_trit within epsilon=0.01) actually fired on 2 of 4000 ticks
milestone 48: ternary selection  -- fire counts min=399 max=533 total=3995 fairness=0.7486
milestone 48: binary   selection -- fire counts min=399 max=533 total=3995 fairness=0.7486
milestone 48: result -- ternary tiebreak engaged but made no measurable fairness difference here -- a real neutral result, not an assumed win
```

REPRODUCED, twice, byte-for-byte, with my own fresh logs (not the
implementer's file):

Boot 1 (m48_review_boot1.log):
```
$ cd spikeling-os-wt-m48 && cargo run -- bios > m48_review_boot1.log 2>&1
[time-bounded to 90s -- kernel blocks at the interactive shell prompt
 after boot completes, same documented reason as the implementer's run
 and the README's own Milestone 39/40 sendkey-unreliability notes]
```
grep of that log:
```
138:milestone 4: kernel survived defect injection at slot 3 -- 7 slots still alive and scheduled, no panic
139:milestone 4: topological (g=+0.6) post-defect fire counts: min=289 max=366 fairness=0.790
140:milestone 4: trivial     (g=-0.6) post-defect fire counts: min=299 max=341 fairness=0.877
141:milestone 4: result -- topological coupling did NOT stay fairer under the defect here (reporting honestly, not the assumed outcome)
142:milestone 48: 4000 ticks over 8 slots (g=0.6, real tasks.rs coupling) -- ternary tiebreak (compare_trit within epsilon=0.01) actually fired on 2 of 4000 ticks
143:milestone 48: ternary selection  -- fire counts min=399 max=533 total=3995 fairness=0.7486
144:milestone 48: binary   selection -- fire counts min=399 max=533 total=3995 fairness=0.7486
145:milestone 48: result -- ternary tiebreak engaged but made no measurable fairness difference here -- a real neutral result, not an assumed win
```
Identical to the implementer's quoted output, including the M4 lines
immediately above it.

Boot 2 (m48_review_boot2.log), independently re-run:
```
142:milestone 48: 4000 ticks over 8 slots (g=0.6, real tasks.rs coupling) -- ternary tiebreak (compare_trit within epsilon=0.01) actually fired on 2 of 4000 ticks
143:milestone 48: ternary selection  -- fire counts min=399 max=533 total=3995 fairness=0.7486
144:milestone 48: binary   selection -- fire counts min=399 max=533 total=3995 fairness=0.7486
145:milestone 48: result -- ternary tiebreak engaged but made no measurable fairness difference here -- a real neutral result, not an assumed win
```
diff -a m48_review_boot1.log m48_review_boot2.log shows exactly one
line differing between the two runs: the cargo "Finished ... in 3.43s"
vs "in 6.31s" build-timing line. Every line of actual kernel/serial
output -- including the Milestone 48 numbers -- is byte-for-byte identical
across both independent boots. This confirms the implementer's claim that
the dynamics are fully deterministic and the reported numbers are not
cherry-picked.

## 6. No-regression claim

Claim: milestone 42: self-test -- OVERALL: PASS and milestone 43:
self-test -- OVERALL: PASS still appear; no panic/double-fault/bare FAIL
anywhere in the log; boot proceeds to the interactive shell prompt.

REPRODUCED.
```
$ grep -a -n "milestone 42: self-test -- OVERALL\|milestone 43: self-test -- OVERALL" m48_review_boot1.log
135:milestone 42: self-test -- OVERALL: PASS
247:milestone 43: self-test -- OVERALL: PASS

$ grep -a -in "panic\|double fault\|kernel panic" m48_review_boot1.log
138:milestone 4: kernel survived defect injection at slot 3 -- 7 slots still alive and scheduled, no panic
179:milestone 41: self-test -- running SIGSEGV_TEST_PROCESS, expecting a real page fault to terminate it without panicking the kernel...
189:milestone 41: self-test -- run() returned normally after the fault (the kernel did not panic/hlt_loop) -- ACTIVE_PROCESS reset to 0 (expected 0)
```
(only expected mentions of the word "panic" -- describing what did not
happen -- no actual panic occurred.)
```
$ grep -a -n "FAIL" m48_review_boot1.log
[no output]
```
No bare FAIL anywhere. Log runs 248 lines and ends mid-shell-prompt
(process time-bounded at 90s, matching the documented reason), not on a
crash.

## 7. Diagnostic epsilon-widening claim (0.2 -> mechanism engages 2996/4000 ties, fairness 0.749 -> 0.998)

NOT INDEPENDENTLY RE-RUN. I was given Bash only, not Edit/Write, and
this check requires temporarily editing TIE_EPSILON in source -- outside
my tool scope for this review, and I judged it out of scope for "verify,
don't modify." I did not reproduce this specific number. However, I did
verify the logic path it depends on by inspection (section 3 above):
select_winner_ternary unconditionally routes every within-epsilon
comparison through the fire_count-based tiebreak, so a larger epsilon
mechanically produces more trit-0 outcomes and more tiebreak decisions --
consistent with the claimed direction of the diagnostic result, though I
did not verify the exact reported numbers (2996/4000, fairness 0.9980)
myself. Flagging this explicitly as unverified rather than silently
treating it as confirmed.

## 8. Observation not raised as a claim by the implementer (informational only)

README.md's Status checklist (the project's own running milestone
log, entries through at least Milestone 42 in the current tree) has no
Milestone 48 entry, and git diff --stat confirms README.md was not
touched by this milestone's changes. The implementer's account never
claimed a README update, so this is not a failed claim -- but it's a
divergence from this project's own established pattern (every prior
milestone in the Status list gets a "- [x] **Milestone N**" entry) worth
surfacing for whoever merges this.

## Overall verdict: VERIFIED

Every concrete, checkable claim in the implementer's account (build
success + exact warnings, the specific wiring of compare_trit into the
real step() path used by tasks::timer_tick_switch(), step_binary/
select_winner_binary being genuinely dead code from the real scheduling
path, the exact serial-log text for Milestones 4/42/43/48, determinism
across two independent boots, and no panics/double-faults/bare FAILs) was
independently REPRODUCED with fresh commands and fresh log files this
session. The one item not independently re-run is the implementer's own
internal diagnostic sanity check (temporarily widening TIE_EPSILON to
0.2) -- that required a source edit outside this review's tool scope, so
it is reported as unverified-by-me rather than silently accepted (section
7). The reported "neutral result" (ternary tiebreak engaged on 2/4000
ticks, identical fairness to the binary path at the real, honestly-derived
epsilon) is real and reproducible, not assumed -- consistent with this
project's own Milestone 4 discipline of reporting negative/neutral results
plainly.
