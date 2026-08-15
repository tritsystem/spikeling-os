# Milestone 46: a real trait/dispatch layer generalizing fs.rs

## What I built

Until this milestone, `fs.rs`'s `write_file`/`read_file`/`delete_file`/
`make_dir`/`remove_dir`/`list` were free functions hardcoded to exactly
one backing store: the ATA-disk directory-table format built up across
Milestones 18/22/28/32. Every caller in the kernel (shell.rs's `write`/
`read`/`rm`/`mkdir`/`rmdir`/`ls`/`cd`, loader.rs's ELF seeding/running,
process.rs's fd-backed `open()`/`fdwrite()`/`close()` syscalls) called
these functions directly, with no notion that "a file" could live
anywhere other than that one disk.

`fs.rs` now has:

- **A real `FileSystem` trait** (`write_file`, `read_file`,
  `delete_file`, `make_dir`, `remove_dir`, `list` -- the exact
  pre-M46 free-function signatures, turned into trait methods).
- **`DiskFs`**, a zero-behavior-change wrapper: the pre-M46 disk logic
  was moved byte-for-byte unchanged into `_disk`-suffixed private
  functions (`write_file` -> `write_file_disk`, etc.), and `DiskFs`'s
  trait impl does nothing but call straight through to them. No disk
  logic was rewritten, only renamed and re-homed behind the trait.
- **`RamFs`**, a genuinely second, concrete backing store: a flat,
  in-memory filesystem backed by `Mutex<BTreeMap<String, Vec<u8>>>`,
  with zero ATA I/O anywhere in it.
- **`resolve_backend(path)`**, the actual dispatch: inspects only the
  path's first component. A path that IS `"ram"` or starts with
  `"ram/"` routes to `RamFs` (with the `"ram/"` prefix stripped off);
  every other path routes to `DiskFs`, completely unchanged from
  before this milestone. The six `pub fn` module-level functions
  (`write_file`, `read_file`, `delete_file`, `make_dir`, `remove_dir`,
  `list`) are now this one-line dispatch followed by a call into
  whichever backend it picked.

**No changes were needed to shell.rs, process.rs, or loader.rs.** Every
caller already goes through these same six module-level functions with
a plain path string -- that was true before this milestone and remains
true now. Concretely this means, with zero code changes outside
`fs.rs`:
- `write ram/foo hello` / `read ram/foo` / `ls ram` / `rm ram/foo` all
  work at the shell today, because shell.rs's handlers call
  `crate::fs::write_file(&target, ...)` etc. with whatever path the
  user typed (after `resolve_arg`'s CWD-prefixing, which is a no-op at
  root).
- `cd ram` works too: `resolve_arg` builds `"ram/whatever"` once CWD is
  `["ram"]`, and `crate::fs::list(Some("ram"))` (used by `cd`'s own
  validation) already routes to `RamFs` via the same dispatcher.
- The fd-backed `open()`/`fdwrite()`/`close()` syscalls in process.rs
  call `fs::read_file`/`fs::write_file` directly with a caller-supplied
  path, so a user program that opens `"ram/x"` would also transparently
  reach `RamFs`. I did **not** independently verify this specific path
  with its own self-test this milestone (see Scope cuts below) -- it's
  a logical consequence of not having touched process.rs at all, not a
  separately tested claim.

## What's deliberately NOT supported (honest scope cuts)

- **`RamFs` is flat -- no subdirectories.** `make_dir`/`remove_dir`
  against it are refused outright ("ramfs: subdirectories not
  supported (flat namespace only)"), and any path with a second `/`
  after the `ram/` mount is refused the same way. The disk side's
  arbitrary-depth nesting (Milestone 32) is not replicated here. A
  real subset, disclosed rather than silently partial.
- **The `ram` mount does not appear in the disk root's own `ls`
  listing.** Unlike a full VFS, there is no synthetic mount-point entry
  injected into `list(None)`/`list(Some(""))` -- you have to already
  know to address `ram/...` directly. This was a deliberate choice to
  keep the diff's blast radius to zero for every existing self-test/
  shell flow that depends on the disk root's exact contents (see
  Verification below for the self-test that specifically checks this).
- **`RamFs` reuses `MAX_FILE_BYTES` (4096 bytes) as its own per-file
  cap**, not because it shares the disk's 512-byte-sector physical
  constraint (it has none), but so behavior stays predictable across
  both backends and a single runaway write can't eat a large fraction
  of the kernel's 100 KiB heap (`allocator.rs`).
- **No fixed entry-count cap** on `RamFs`'s directory, unlike the
  disk's fixed 8 slots -- a `BTreeMap` just grows, bounded only by the
  heap.
- **Genuinely volatile.** `RamFs` never touches `ata::*`; its contents
  do not survive a reboot. This is the actual point of having a second,
  different kind of backing store, not an oversight.
- A pre-existing disk directory/file literally named `ram` at the true
  root (unlikely, since no shell command or self-test in this kernel's
  history has ever created one) would now be shadowed -- unreachable
  via `fs::*`, since any path starting with `ram` at the top level
  always routes to the ramfs mount instead. Disclosed, not tested
  against directly (would require deliberately corrupting/pre-seeding a
  disk image, out of scope for this milestone).
- The fd-backed syscall path (process.rs) inherits `fs::MAX_FILE_BYTES`
  as its own per-fd write-buffer cap regardless of which backend a path
  resolves to (see process.rs's own `write_fd` doc comment) -- since
  `RamFs` uses that same constant as its cap too, this isn't a new
  mismatch, just worth naming.

## What I verified, and how

Real evidence only -- every claim below is backed by an actual QEMU
boot's serial output, not code inspection alone.

**Build**: `cargo run -- bios` (the project's own runner, which builds
`kernel/` as an `x86_64-unknown-none` artifact dependency and boots it
in QEMU with serial piped to the terminal) compiled clean. The only two
warnings in the build log (`ALTENTRY_TEST_ELF_BYTES` unused,
`extra_frames` field unread) are pre-existing and unrelated to `fs.rs` --
confirmed by grepping the full build log for `warning` and finding
nothing referencing `fs.rs` or this milestone's new code.

**Boot-time self-tests, real serial output** (`main.rs` now calls
`fs::self_test_disk_write()` immediately followed by the new
`fs::self_test_ramfs()`, in the same position in the boot sequence as
before):

```
fs self-test: disk write+read roundtrip OK -- real bytes matched
fs self-test: ramfs write+read roundtrip OK -- real bytes matched, routed through the same fs::write_file/read_file surface as the disk path
fs self-test: ramfs list() OK -- 'ramfsselftest' present with the correct length
fs self-test: ramfs isolation OK -- disk root listing does NOT contain 'ramfsselftest' (two real, separate backing stores)
```

- Line 1 is the pre-existing Milestone 18-era disk self-test,
  unmodified in behavior -- it now flows through `resolve_backend`
  (since `write_file`/`read_file` are the dispatcher, not the disk
  logic directly), and still passes, proving the OLD path is intact
  through the new dispatch layer.
- Line 2 is the NEW self-test (`fs::self_test_ramfs`, added at the
  bottom of `fs.rs`): writes `"ram/ramfsselftest"` then reads it back
  through the identical `fs::write_file`/`fs::read_file` functions the
  disk test uses, and confirms the bytes match exactly.
- Line 3 confirms `fs::list(Some("ram"))` (the same dispatcher used by
  the shell's `ls ram`) reports the new file with the correct byte
  length.
- Line 4 is an isolation check: confirms `fs::list(None)` (the disk's
  own root listing, completely unchanged code path) does NOT contain
  the ramfs file -- proving `RamFs` is a genuinely separate store, not
  the disk under a different-looking path string.

**Every other existing self-test in the boot sequence still passes,
confirmed from the same real boot log** (searched the full log for
`self-test`, `FAIL`, `MISMATCH`, `panic` and reviewed every match):
Milestone 34 program-size check, Milestone 35 FDTEST layout check,
Milestone 36 ELF parse self-test, Milestone 37 FORK_TEST_PROGRAM layout
check, Milestone 40 pipe-mechanics self-test (`write_ok=true
roundtrip_ok=true dup_ok=true dup2_ok=true`), Milestone 42 process-groups
self-test (`OVERALL: PASS`), Milestone 41 SIGSEGV/SIGKILL self-tests
(kernel recovered from a real page fault without panicking, confirmed
process-table slot reuse), Milestone 43 wait-status self-test
(`OVERALL: PASS`). No panics, no `hlt_loop`, boot reached the
interactive shell prompt normally both times I ran it.

**A real bug found and fixed along the way, not hidden**: my first
version of `self_test_ramfs()` reused the literal filename
`"selftestwrite"` -- the exact same name `self_test_disk_write()`
already writes to the real disk root, earlier in the same boot
sequence. Its isolation check (line 4 above) then failed for real:

```
fs self-test: ramfs isolation FAILED -- disk root listing unexpectedly contains 'selftestwrite': [("selftestwrite", false, 38)]
```

Diagnosed correctly before touching the dispatcher itself: this was
never a routing bug (the roundtrip and `list(Some("ram"))` checks
immediately above it in the same run both passed cleanly), it was a
self-test naming collision -- the disk root legitimately contains
`"selftestwrite"` because `self_test_disk_write()` put it there two
self-tests earlier. Fixed by giving `self_test_ramfs()` a disjoint name
(`"ramfsselftest"`), not by weakening the isolation check. Rebuilt and
re-ran; the isolation check now reports `OK` as shown above. This is
exactly the kind of test-authoring mistake this project's own README
discipline calls for disclosing rather than quietly correcting without
a trace.

## What I did NOT verify

- **No live interactive shell session** (`write ram/foo hello` typed
  through QEMU's `sendkey`, etc.) was run this milestone. This project's
  own README already documents `sendkey`-based interactive testing as
  repeatedly unreliable (Milestones 39/40's own notes), which is why
  boot-time self-tests are the preferred verification method here, and
  why I leaned on those instead. shell.rs itself was not modified at
  all -- its handlers call the exact same `fs::write_file`/`read_file`/
  `list`/`make_dir`/`remove_dir`/`delete_file` functions the self-tests
  above already exercise for a `ram/...` path, so this is the same
  proven code path, not an untested one, but I want to be precise that
  "the shell will work" is a code-path inference from the self-test
  above, not something I separately watched happen in a live session.
- **The fd-backed syscall path** (process.rs's `open`/`read`/`fdwrite`/
  `close`, syscalls 3-6) reaching `RamFs` for a `ram/...` path was not
  independently exercised with its own self-test. process.rs was not
  touched this milestone; it already calls `fs::read_file`/
  `fs::write_file` generically, so it inherits the new dispatch for
  free, but I did not add a kernel-side self-test (in the style of
  `process::self_test_pipe_mechanics`/`self_test_process_groups`) that
  actually drives `open_file()`/`read_fd()`/`write_fd()`/`close_fd()`
  against a `"ram/..."` path and checks the real return values. Left
  out to keep this milestone's diff scoped to `fs.rs` (plus the one
  `main.rs` self-test wire-up) rather than also touching process.rs's
  process-table machinery, which has its own hard-won ordering
  constraints (see Milestone 41/42's own doc comments) I didn't want to
  risk disturbing for a claim the fs.rs-level self-test already
  substantially supports.
