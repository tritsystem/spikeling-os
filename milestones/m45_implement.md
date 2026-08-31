# Milestone 45: real `exec()` -- a genuine ELF teardown-and-rebuild

## What this milestone builds

Milestone 37's `exec()` was always a disclosed placeholder: it overwrote
the SAME code frame's bytes in place (zero-fill + copy), reusing the
calling process's existing PML4, stack, and heap unchanged -- a flat
byte-blob replacement, not a real loader, and explicitly flagged as such
in its own doc comment.

This milestone replaces that placeholder with a REAL implementation:
`exec()` now reads the target file, parses it as a genuine ELF64 image
(`elf::parse()`, Milestone 36's own parser -- no new parsing code), and
tears down and rebuilds the calling process's ENTIRE address space --
a fresh PML4, fresh physical code/stack/heap frames, one physical page
per real `PT_LOAD` segment page -- via `create_process_from_elf()`, the
EXACT SAME function `runelf`/`load_and_run_elf()` already use. Nothing
new was written for the actual loading mechanism; this milestone's own
code (`process::exec_elf()`) is the orchestration around it: snapshot
what must survive real exec() (open fds, `parent_pid`, `pgid`), build
the new process, splice those three fields back in, replace the
process's table slot outright, switch CR3, and hand the real, parsed
`e_entry` back to `usertest::exec_replace_and_enter()` -- which now
jumps to that real address instead of a hardcoded `USER_CODE_ADDR`
(Milestone 44's `Process::entry` generalization, exercised by a real
`exec()` call for the first time here).

Real POSIX `exec()` contract, checked field by field in the new code and
in the self-test below: same pid, same open fds, same `parent_pid`, same
`pgid`, `heap_used` reset to 0 -- but a genuinely NEW address space and a
genuinely NEW entry point.

### Files changed
- `kernel/src/process.rs` -- new `exec_elf()` (replaces `exec_process()`),
  new `replace_process()` slot-swap helper, a ninth hardcoded process
  slot (`EXEC_TEST_PROCESS_ID = 9`) + `EXEC_TEST_PROGRAM` (hand-
  assembled, keystone+capstone verified), `self_test_exec_test_program()`
  (byte-layout check) and `self_test_real_exec()` (the real, non-
  interactive ring-3 proof).
- `kernel/src/usertest.rs` -- syscall-9 arm now parses ELF and calls
  `process::exec_elf()`; updated doc comments on `exec_replace_and_enter`/
  `exec_into_ring3` to reflect the real address-space swap.
- `kernel/src/loader.rs` -- `seed_test_elf_altentry()` (writes Milestone
  44's ALTENTRY_TEST_ELF_BYTES to disk at `EXEC_TEST_PATH`).
- `kernel/src/shell.rs` -- new `seedaltentry` / `runexectest` commands,
  help text updated.
- `kernel/src/main.rs` -- wires `init_exec_test_process()` +
  `self_test_exec_test_program()` + `self_test_real_exec()` into boot;
  **one real, pre-existing bug fix** (see below).
- `kernel/assets/testelf_altentry.elf` -- rebuilt for real (was a
  16-byte placeholder stub, not a real ELF -- see below).
- `tools/testelf_altentry_src/testelf_altentry.rs` + its `README.md` --
  a real, independent compile-error fix and a corrected build recipe
  (see below).

## What was verified, and how

Booted the real kernel in QEMU (`qemu-system-x86_64`, `-serial
file:...`, no display, headless) and read the real serial log -- not
inferred from source, not assumed from a successful `cargo build`. Two
separate real boots are kept in `milestones/`:

- `m45_boot.log` -- the FIRST real boot, run against the ORIGINAL
  (unbuilt, 16-byte placeholder) `testelf_altentry.elf` asset, showing
  the self-test's own honest FAILURE path working correctly.
- `m45_final.log` -- the final, fully-fixed boot: every self-test
  through Milestone 45 passes, and the kernel proceeds cleanly all the
  way to the interactive shell.

### The self-test itself (`process::self_test_real_exec()`)

Same "boot-time, non-interactive" discipline this project has used
since Milestone 40 (interactive shell-command testing via QEMU sendkey
has been repeatedly unreliable here). A ninth hardcoded process slot
(`EXEC_TEST_PROCESS`) runs a tiny hand-assembled program
(`EXEC_TEST_PROGRAM`, assembled + verified via a standalone Python
script using keystone, with a capstone disassembly round-trip
confirming the intended control flow byte for byte -- same discipline
as every other hand-assembled program in this codebase) that calls the
real `exec()` syscall into `"altentry"` (Milestone 44's own staged,
never-actually-wired-in `testelf_altentry.elf` test payload, now
finally exercised for real).

Real evidence gathered by the self-test, from the actual final boot log
(`m45_final.log`):

```
milestone 45: seedaltentry -- wrote 856 real bytes (a genuine externally-built ELF64 executable whose e_entry does NOT equal USER_CODE_ADDR) to 'altentry' on the real on-disk filesystem
milestone 45: self-test -- seeded 'altentry' (testelf_altentry.elf) to the real on-disk filesystem: true
milestone 45: self-test -- fd opened before exec()=true, before-state: pml4=0x76000 entry=0x555550000000 pgid=9
milestone 30: CR3 switched -- entering ring 3 for process 9 at 0x555550000000
milestone 36: process exec'd -- ELF validated: e_entry=0x555550003000 falls inside a mapped segment, 1 PT_LOAD segment(s), 1 total page(s), all within private p4 index 170
milestone 36: process exec'd -- new pml4 0x11e000 populated: 511 kernel-space entries shared, index 170 left private
milestone 45: syscall EXEC (process 9) -- REAL teardown-and-rebuild complete: new pml4=0x11e000, new entry=0x555550003000 (real parsed e_entry), fd table/parent_pid/pgid preserved, CR3 switched
milestone 31: syscall WRITE (process 9) -- hardware-recorded CS=0x1b (CPL=3) -- ptr=0x55555000301c len=70 -- raw bytes >>>
hello from a non-USER_CODE_ADDR entry point -- milestone 44 confirmed!
<<< end of write (70 bytes)
milestone 45: self-test -- run_ok=true, after-state: pml4=0x11e000 entry=0x555550003000 pgid=9 fds[0]_present=true
milestone 45: self-test -- pml4_changed=true (before=0x76000 after=0x11e000), entry_changed=true (before=0x555550000000 after=0x555550003000, expected real e_entry=0x555550003000), fd_survived_exec=true, pgid_preserved=true
milestone 45: self-test -- OVERALL: PASS
```

This is real, not assumed: `pml4_changed=true` proves a genuinely new
PML4 physical frame was allocated and switched to (0x76000 ->
0x11e000), `entry_changed=true` with `entry_after` equal to
`0x555550003000` proves execution resumed at the TARGET ELF's own real,
parsed `e_entry` (three pages past `USER_CODE_ADDR`, Milestone 44's
whole point) rather than the old process's entry or a hardcoded
constant, `fd_survived_exec=true` proves a file descriptor opened
BEFORE `exec()` (kernel-side, via `open_file()`, independent of ring 3)
is still present in the NEW process's fd table afterward, and
`pgid_preserved=true` proves process-group identity survived. The
`write` syscall's own raw byte dump shows the REAL message compiled
into `testelf_altentry.elf`, printed from CPL=3 code running at the new
process's own new code page -- direct, hardware-recorded proof this
was genuinely new code executing, not the old program still running.

The self-test's own honest failure path is ALSO real, not assumed --
captured in `m45_boot.log` from the FIRST real boot (before the ELF
asset itself was actually built, see below):

```
milestone 45: seedaltentry -- wrote 16 real bytes ... to 'altentry' ...
milestone 45: syscall EXEC (process 9) -- FAILED, 'altentry' is not a valid ELF64 image (elf: file smaller than a 64-byte ELF64 header) -- returning u64::MAX, old program continues running
milestone 45: EXEC_TEST fallback -- real exec() failed
milestone 45: self-test -- OVERALL: FAIL
```

A real ELF-format failure was reported honestly (not silently ignored),
the calling process's OWN fallback branch ran correctly (old program
kept executing, wrote its own distinguishing failure message, exited
cleanly -- no crash), and the self-test's own OVERALL result correctly
reported FAIL rather than a false PASS. This is exactly the "degrades
cleanly and honestly either way" contract FORK_TEST_PROGRAM's own child
fallback path already established, now proven for real exec() too.

## Real bugs found and fixed

This milestone required chasing down three separate, independently
real bugs before a genuinely clean, fully-verified boot was achievable
-- each one found with real evidence, not guessed:

### 1. `testelf_altentry.elf` (Milestone 44's staged test payload) was never actually built

Discovered by trying to seed it and getting `elf: file smaller than a
64-byte ELF64 header` from the REAL parser (not assumed -- see the
`m45_boot.log` excerpt above). `git show --stat` on Milestone 44's own
commit confirmed it: `kernel/assets/testelf_altentry.elf | Bin 0 -> 16
bytes`. The file that shipped was a 16-byte all-zero placeholder, not a
compiled ELF -- Milestone 44's own commit message says as much ("staged
but not yet wired into a self-test"), but the asset itself had never
even been built once with the documented recipe.

Running that documented recipe for the first time surfaced TWO further,
separate real problems, both confirmed directly with `readelf`:
- A genuine compile error in `testelf_altentry.rs` itself: `MESSAGE`'s
  declared array size (71) didn't match its actual string literal
  length (70) -- `rustc` correctly refused to build (`E0308`) until
  fixed. Fixed in the source file directly.
- Even after that fix, the documented recipe (`rustc ... -C
  code-model=large -C panic=abort ...`, no other flags) produced a
  `Type: DYN` (position-independent) ELF, which `elf.rs`'s parser
  correctly rejects (it requires `ET_EXEC`) -- and a ~4.8 KB file,
  comfortably over `fs.rs`'s own 4096-byte `MAX_FILE_BYTES` cap, so it
  could never even be written to this kernel's own filesystem. Fixed by
  adding `-C relocation-model=static -C link-arg=-z -C
  link-arg=max-page-size=16` to the build command (the same
  `max-page-size` trick `testelf.elf`'s own README already documents
  for the identical file-offset-alignment reason) -- the corrected
  recipe produces a clean 856-byte, `Type: EXEC`, `Entry point address:
  0x555550003000` file. `tools/testelf_altentry_src/README.md` updated
  with the corrected recipe and this real diagnosis.

### 2. A real, pre-existing, boot-hanging interrupt bug (not introduced by this milestone)

While chasing why `self_test_real_exec()`'s own boot log stopped dead
right after its `OVERALL: PASS` line -- no panic, no further output,
QEMU still running -- root-caused this with a real QEMU `-d int`
hardware trace (`SPIKELING_QEMU_TRACE`-style direct invocation, not
`cargo run`) plus a controlled A/B check: rebuilt and booted the
UNMODIFIED pre-Milestone-45 kernel (commit `a604426`, via an isolated
`git worktree` so my own working tree was untouched) and observed the
IDENTICAL hang, at the IDENTICAL point (right after
`self_test_wait_status()`'s own `OVERALL: PASS`) -- zero Milestone 45
code involved. **This is a real, previously-invisible bug that predates
this milestone**, silently present in every non-interactive boot since
Milestone 41/43 shipped, simply never noticed because nothing had ever
checked whether the serial log kept going PAST a self-test's own PASS
line.

Real mechanism, traced through the actual code (not guessed):
`int 0x80`'s IDT gate is a genuine Interrupt Gate (the `x86_64` crate's
own default -- confirmed in its source; `interrupts.rs` never opts out
via `.disable_interrupts(false)`), so hardware clears `RFLAGS.IF` to 0
the instant ANY `int 0x80` fires. Ordinary syscalls restore `IF=1` for
free on their normal return (`syscall_entry`'s own `iretq`, using the
CPU's original hardware-pushed frame). But the exit syscall and the
page-fault/SIGSEGV path both resume back into kernel context via
`usertest::resume_kernel()`, which deliberately does a plain `ret` --
**no** `iretq`, **no** `sti` -- by design: its own Milestone 27 doc
comment explains an unconditional `sti` there caused a real,
previously-diagnosed deadlock when `run()` is called nested inside the
keyboard ISR's own call chain, so `sti` was removed, relying instead on
whichever OUTER interrupt handler's own eventual `iretq` to naturally
restore `IF` -- which only exists if there IS one. Every subsequent
`enter_ring3`-family call masks this gap for free (its own iretq frame
hardcodes `RFLAGS=0x202`, unconditionally), so it's invisible as long
as ANOTHER ring-3 excursion follows. Called from the interactive shell
(nested inside the keyboard ISR), this is exactly correct. Called
directly from `kernel_main()` -- true of every non-interactive
self-test -- there is no outer handler to restore it, so `IF` stays 0
forever after the LAST such excursion's exit unwinds, silently
disabling every interrupt (the PIT included) for the rest of the boot;
`hlt()` with `IF=0` then blocks forever (only NMI could wake it, and
none occurs), matching the observed near-0% CPU usage during the "hang"
(a real deadlock/busy-spin would instead peg a core at 100%).

**Fix**: one explicit `x86_64::instructions::interrupts::enable()` call
in `kernel_main`, immediately after the three non-interactive ring-3
self-tests and before the code that first depends on interrupts being
enabled (the timer-tick count) -- `resume_kernel()`'s own doc comment
explicitly names "ordinary kernel_main code" as a SAFE case for `sti`
(as opposed to the ISR-nested case that's actually dangerous), so this
restores interrupts in exactly the one place that's both correct and
necessary, without reintroducing the Milestone 27 keyboard-ISR
deadlock. Verified for real: after the fix, the same boot's serial log
continues cleanly past `self_test_real_exec()`'s `OVERALL: PASS` --

```
milestone 45: self-test -- OVERALL: PASS
milestone 5b: 84 real timer interrupts observed after 80 hlt cycles -- PIT firing confirmed
milestone 5c: returned from preemption demo -- worker counters: task0=1056 task1=821 task2=1062
milestone 5c: all three tasks genuinely preempted and ran -- real preemptive multitasking confirmed
milestone 6: waiting for keyboard input (external test harness will type)...
milestone 6: received 0 chars: "" / FAILED -- no keystrokes received
milestone 8: interactive shell active -- see framebuffer for real typed session
milestone 25: background task scheduling enabled -- spawn/kill available via the shell
```

(Milestone 6's own "no keystrokes received" is the expected, honest
result for a run with no synthetic keyboard input sent -- not a
regression; this project's own README already documents that specific
test as needing an external keystroke source.)

Every `OVERALL:` self-test line in the final boot reports PASS (grepped
directly, not selectively quoted): Milestone 42 (process groups),
Milestone 43 (wait status), Milestone 45 (real exec). No panics, no
double faults, no other FAILED lines anywhere in the full log.

## Honest scope-cuts

- **`exec()` now requires a real ELF64 image** -- the same requirement
  `runelf` has always had. A flat, headerless binary (Milestone 34's
  `testprog`, built by `seedtestprog`) is no longer a valid exec()
  target; this is a disclosed, real behavior change from Milestone 37's
  own placeholder (which never checked the target's format at all).
  `FORK_TEST_PROGRAM`'s own child branch still targets `"testprog"`
  (Milestone 37's own demo), so that specific interactive `runfork` path
  now always takes its own honest fallback branch instead of a
  successful exec() -- verified BY CODE INSPECTION (the flat binary's
  first bytes don't match the ELF magic `elf.rs::parse()` checks for),
  not by a live interactive run (this project's own established,
  disclosed limitation: QEMU-sendkey-driven interactive testing has
  been repeatedly unreliable here). Real exec() itself is verified
  separately and non-interactively by this milestone's own
  `EXEC_TEST_PROCESS` / `self_test_real_exec()` against a genuine ELF64
  target instead.
- **Physical frames from the OLD address space are not reclaimed** --
  matches `kill()`/`wait_for_child()`'s own already-shipped, disclosed
  precedent exactly: this kernel's frame allocator has never freed a
  physical frame on process exit, reap, or kill either (`BootInfo
  FrameAllocator` is bump-only, by its own doc comment). `exec()`
  doesn't introduce a new gap here, it inherits the existing one. What
  IS genuinely new: the OLD frames are truly abandoned (no Process
  struct in this kernel references them anymore once the slot is
  replaced), not reused -- the real "teardown" half of "teardown-and-
  rebuild", even though "teardown" in this kernel has only ever meant
  "stop referencing," never "return to a free list" (there isn't one).
- Every other `create_process_from_elf()` limitation already disclosed
  by Milestone 36/44 (page-aligned segments only, a fixed per-segment/
  total page cap, `p_flags` parsed but not enforced as real page
  permissions) is unchanged and inherited as-is -- `exec()` is a new
  CALLER of that existing loader, not a new loader.

## Build

`cargo build` is clean (one pre-existing, unrelated warning --
`extra_frames` field never read, present before this milestone too,
disclosed in its own doc comment as intentionally-kept-but-unread
bookkeeping).
