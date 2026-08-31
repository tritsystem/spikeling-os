//! MILESTONE 34: a real general program loader -- until now, every
//! process (Milestone 30's PROCESS_A/PROCESS_B, and Milestone 27's
//! single usertest program before that) ran code from a BYTE ARRAY
//! BAKED DIRECTLY INTO THE KERNEL BINARY (usertest::USER_PROGRAM). This
//! module closes that gap for real: the `runfile PATH` shell command
//! reads a file's bytes off the actual on-disk filesystem (fs.rs,
//! Milestones 18/22/28/32) and hands them to
//! process::create_loaded_process()/run_loaded_process() (MILESTONE 35:
//! split from one original load_and_run_image() function -- see
//! run_file()'s own comment below for why) -- the EXACT SAME private-
//! PML4/P3/P2/P1 mechanism Milestone 30 established for PROCESS_A/
//! PROCESS_B, just sourced from a runtime file read instead of a
//! compile-time array. No
//! new page-table code was written for this: process.rs's
//! create_process() was refactored into a create_process_from_image()
//! core (taking an arbitrary `&[u8]`) plus a thin wrapper that rebuilds
//! the ORIGINAL fixed PROCESS_PROGRAM+message layout PROCESS_A/PROCESS_B
//! still use -- so both the hardcoded-array path and this file-loaded
//! path provably run through identical, unduplicated unsafe mapping
//! code.
//!
//! Honest limitations, disclosed rather than hidden (same spirit as
//! every prior milestone's own scoping notes, and this project's own
//! README, which already flags a real ELF loader as future work):
//!   - flat binary only -- no ELF (or any other object-file format)
//!     parsing, no relocations, no sections, no symbol table, no
//!     dynamic linking. A "program" here is just raw machine code
//!     copied verbatim to a fixed load address (usertest::USER_CODE_ADDR)
//!     at a fixed offset 0, exactly like usertest::USER_PROGRAM always
//!     was -- this milestone only changes WHERE those bytes come from,
//!     not what they're allowed to look like.
//!   - fixed max program size: one 4KiB page
//!     (process::MAX_CODE_IMAGE_BYTES), enforced BEFORE any copy
//!     happens -- an oversized file is rejected with a clear error,
//!     never silently truncated or allowed to overflow into whatever
//!     physical frame happens to follow the code frame in the new
//!     process's (private) address space.
//!   - only ONE loaded-from-file process can be resident at a time
//!     (process::LOADED_PROCESS is a single Mutex<Option<Process>>
//!     slot, replaced -- not accumulated -- by each `runfile` call),
//!     matching PROCESS_A/PROCESS_B's own one-shot-per-slot design
//!     rather than a general process table with PIDs.
//!   - no argv/envp, no file descriptors the loaded program can use to
//!     read further files itself -- it gets the same syscalls
//!     usertest.rs already implements (0 = write, 1 = exit). It DOES get
//!     a private heap mapped (Milestone 33's heap is now uniform across
//!     every process, loaded-from-file or not), but `sbrk` (syscall 2)
//!     currently only recognizes PROCESS_A/PROCESS_B's ids -- a
//!     loaded-from-file process's heap is real and mapped, but not
//!     currently reachable via sbrk. Disclosed, not hidden: nothing in
//!     this milestone's own test program needs it.
//!
//! MILESTONE 36: the real ELF loader flagged above as future work now
//! exists, ADDITIONALLY -- `runelf PATH` alongside `runfile PATH`, not
//! instead of it. `runfile`'s flat-binary behavior above is completely
//! unchanged: same functions, same limitations, same test program. The
//! new `runelf` path reads a file's bytes the same way, then hands them
//! to kernel/src/elf.rs's real ELF64 parser instead of assuming they're
//! already flat code at offset 0 -- see elf.rs and
//! process::create_process_from_elf() for the actual parsing/loading
//! logic and this milestone's honest scoping decision (real ELFs
//! specifically linked for this kernel's fixed entry address, not
//! arbitrary Linux binaries).

use crate::elf;
use crate::fs;
use crate::memory;
use crate::process;
use crate::serial;
use crate::usertest;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;
use core::sync::atomic::Ordering;

/// MILESTONE 34: this milestone's own test payload -- proof-of-concept
/// only, not something a real user would ever type. Reuses
/// usertest::USER_PROGRAM verbatim (Milestone 31's write(ptr,len)-ready
/// 31-byte program, already set up to read USER_CODE_ADDR+MESSAGE_OFFSET
/// for exactly MESSAGE_LEN bytes before calling syscall 0) with a
/// message that is clearly, deliberately different from every other
/// hardcoded string already present in this kernel binary ("hello from
/// ring 3...", "hello from process A/B") -- so a `runfile` serial-log
/// line printing THIS exact message is real, unambiguous proof the
/// executed bytes came from the file written to disk, not from any
/// array compiled into the kernel.
pub const TEST_PROGRAM_MESSAGE: &str = "hello from a REAL FILE on disk -- milestone 34 loader confirmed";

/// Builds the on-disk image for this milestone's test program: the
/// hand-assembled syscall bytes at offset 0 (usertest::USER_PROGRAM,
/// UNCHANGED -- it already sets up rdi/rsi for the write(ptr,len)
/// syscall), the distinguishing message at usertest::MESSAGE_OFFSET,
/// space-padded to EXACTLY usertest::MESSAGE_LEN bytes -- the write
/// syscall reads a fixed `len` immediate baked into USER_PROGRAM itself,
/// not a null-terminated string, so the message region must be exactly
/// that length, same convention usertest::write_fixed_message() and
/// process.rs's create_process() wrapper both already follow.
pub fn build_test_program_image() -> Vec<u8> {
    let offset = usertest::MESSAGE_OFFSET as usize;
    let mut image = vec![0u8; offset + usertest::MESSAGE_LEN];
    image[..usertest::USER_PROGRAM.len()].copy_from_slice(&usertest::USER_PROGRAM);

    let msg_bytes = TEST_PROGRAM_MESSAGE.as_bytes();
    let n = msg_bytes.len().min(usertest::MESSAGE_LEN);
    for b in image[offset..offset + usertest::MESSAGE_LEN].iter_mut() {
        *b = b' ';
    }
    image[offset..offset + n].copy_from_slice(&msg_bytes[..n]);
    image
}

/// MILESTONE 34: the `seedtestprog` shell command's entry point --
/// writes build_test_program_image()'s bytes to a REAL file
/// ("testprog") on the real on-disk filesystem (fs.rs) via the exact
/// same fs::write_file() the `write` shell command already uses. This
/// is the one piece of this milestone's own test setup that ISN'T
/// something a real user would type: the point is to get genuine,
/// non-typeable machine-code bytes (raw opcode 0x00 bytes included)
/// onto disk for real, which the keyboard-driven `write` command (typed
/// TEXT, not arbitrary bytes) has no way to do.
pub fn seed_test_program() -> Result<usize, String> {
    let image = build_test_program_image();
    let len = image.len();
    fs::write_file("testprog", &image).map_err(|e| format!("seedtestprog: {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 34: seedtestprog -- wrote {len} real bytes (hand-assembled syscalls + a distinguishing message) to 'testprog' on the real on-disk filesystem"
    );
    Ok(len)
}

/// MILESTONE 35: hand-assembled x86_64 machine code exercising the new
/// open(3)/read(4)/fdwrite(5)/close(6) syscalls end to end, loaded and
/// run exactly like build_test_program_image() above (via `runfile`,
/// through process::create_loaded_process()/run_loaded_process() --
/// LOADED_PROCESS, id 3, which gets a real fd table exactly like
/// PROCESS_A/PROCESS_B do; see
/// process.rs's with_process_mut() doc comment). Regenerated
/// deterministically by a standalone Python assembler script (each
/// instruction's encoding hand-derived and cross-checked, not
/// hand-counted hex digits -- same discipline USER_PROGRAM/
/// PROCESS_PROGRAM's own doc comments describe), not hand-typed hex.
///
/// Two phases, both exercising real on-disk files:
///
///   Phase 1 (proves open+read work against REAL, PRE-EXISTING content):
///   opens "fdtest" (a file the `write` shell command must create BEFORE
///   `runfile fdtestprog` runs -- see FDTEST_READ_PATH/its expected
///   content, verified against by the milestone report), reads up to 128
///   bytes into scratch space in the user stack page (never touched by
///   this program's own rsp, since it makes no call/push/pop -- safe to
///   use as a scratch buffer), and writes exactly what it read back out
///   via the EXISTING syscall 0 (write) to serial -- so the serial log
///   shows, byte for byte, what open+read actually produced, directly
///   comparable against what `read fdtest` reports from the shell.
///
///   Phase 2 (proves fdwrite+close persist FOR REAL): opens "fdout" (a
///   path that does NOT exist yet -- proving open()'s honest "start
///   empty" behavior for a missing path), fdwrite()s a fixed,
///   distinguishing message (see FDOUT_WRITE_CONTENT below) into it,
///   and closes it -- which is the one moment this milestone's design
///   actually writes to disk (process::close_fd). A `read fdout` shell
///   command run AFTER `runfile fdtestprog` should report exactly
///   FDOUT_WRITE_CONTENT back.
///
/// Layout (verified against the actual byte array below by
/// self_test_fdtest_program()):
///   offset   0..176   the syscall sequence itself (152 bytes of real
///                      instructions, zero-padded out to 176)
///   offset 176..182   "fdtest\0\0\0\0"  (PATH1, 6 real bytes, path for
///                      syscall 3 in phase 1)
///   offset 186..191   "fdout\0\0\0"     (PATH2, 5 real bytes, path for
///                      syscall 3 in phase 2)
///   offset 195..264   FDOUT_WRITE_CONTENT's 69 bytes (WDATA, the
///                      payload syscall 5 writes in phase 2)
pub const FDTEST_PROGRAM: [u8; 264] = [
    0x48, 0xBF, 0xB0, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x06,
    0x00, 0x00, 0x00, 0xB8, 0x03, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x49, 0x89,
    0xC2, 0x4C, 0x89, 0xD7, 0x48, 0xBE, 0x00, 0x02, 0x00, 0x60, 0x55, 0x55,
    0x00, 0x00, 0xBA, 0x80, 0x00, 0x00, 0x00, 0xB8, 0x04, 0x00, 0x00, 0x00,
    0xCD, 0x80, 0x49, 0x89, 0xC3, 0x48, 0xBF, 0x00, 0x02, 0x00, 0x60, 0x55,
    0x55, 0x00, 0x00, 0x4C, 0x89, 0xDE, 0xB8, 0x00, 0x00, 0x00, 0x00, 0xCD,
    0x80, 0x4C, 0x89, 0xD7, 0xB8, 0x06, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48,
    0xBF, 0xBA, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x05, 0x00,
    0x00, 0x00, 0xB8, 0x03, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x49, 0x89, 0xC2,
    0x4C, 0x89, 0xD7, 0x48, 0xBE, 0xC3, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00,
    0x00, 0xBA, 0x45, 0x00, 0x00, 0x00, 0xB8, 0x05, 0x00, 0x00, 0x00, 0xCD,
    0x80, 0x4C, 0x89, 0xD7, 0xB8, 0x06, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xB8,
    0x01, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xEB, 0xFE, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x66, 0x64, 0x74, 0x65,
    0x73, 0x74, 0x00, 0x00, 0x00, 0x00, 0x66, 0x64, 0x6F, 0x75, 0x74, 0x00,
    0x00, 0x00, 0x00, 0x77, 0x72, 0x69, 0x74, 0x74, 0x65, 0x6E, 0x20, 0x62,
    0x79, 0x20, 0x72, 0x69, 0x6E, 0x67, 0x2D, 0x33, 0x20, 0x66, 0x64, 0x77,
    0x72, 0x69, 0x74, 0x65, 0x20, 0x73, 0x79, 0x73, 0x63, 0x61, 0x6C, 0x6C,
    0x20, 0x2D, 0x2D, 0x20, 0x6D, 0x69, 0x6C, 0x65, 0x73, 0x74, 0x6F, 0x6E,
    0x65, 0x20, 0x33, 0x35, 0x20, 0x72, 0x65, 0x61, 0x6C, 0x20, 0x66, 0x64,
    0x20, 0x70, 0x65, 0x72, 0x73, 0x69, 0x73, 0x74, 0x65, 0x6E, 0x63, 0x65,
];

/// The exact content the `write` shell command must put into "fdtest"
/// BEFORE `runfile fdtestprog` is run, for phase 1's open+read to have
/// real, known content to prove against -- typed by a real (or
/// synthetic-but-real, QEMU `sendkey`-driven) keyboard session, the same
/// as any other shell `write` command; this constant exists purely so
/// this doc comment and the milestone report can both point at the
/// single source of truth for what that content must be, and so
/// self_test_fdtest_program() below can verify FDTEST_PROGRAM's own
/// hardcoded path length (6, for "fdtest") independent of this string's
/// length (they're unrelated by construction, but keeping both named
/// here makes that non-relationship explicit rather than implicit).
pub const FDTEST_READ_PATH: &str = "fdtest";

/// The exact content FDTEST_PROGRAM's phase 2 fdwrite()s into "fdout"
/// (baked into the WDATA region of FDTEST_PROGRAM above at compile time,
/// since a hand-assembled program can't compute a string's length at
/// runtime -- same convention usertest::MESSAGE_LEN/write_fixed_message
/// established). A `read fdout` shell command run after `runfile
/// fdtestprog` should report exactly this string back, byte for byte --
/// that's the real, honest proof fdwrite+close persisted to disk.
pub const FDOUT_WRITE_CONTENT: &str = "written by ring-3 fdwrite syscall -- milestone 35 real fd persistence";

/// MILESTONE 35: writes FDTEST_PROGRAM's bytes to a REAL file
/// ("fdtestprog") on the real on-disk filesystem, mirroring
/// seed_test_program()'s own reasoning exactly -- this is the one piece
/// of setup that isn't something a normal user would type (getting raw,
/// non-typeable opcode bytes onto disk), everything after this
/// (`runfile fdtestprog`, then `read fdout`) is ordinary shell usage.
pub fn seed_fdtest_program() -> Result<usize, String> {
    let len = FDTEST_PROGRAM.len();
    fs::write_file("fdtestprog", &FDTEST_PROGRAM).map_err(|e| format!("seedfdtest: {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 35: seedfdtest -- wrote {len} real bytes (hand-assembled open/read/fdwrite/close syscalls) to 'fdtestprog' on the real on-disk filesystem"
    );
    Ok(len)
}

/// MILESTONE 35: a direct, filesystem-independent proof that
/// FDTEST_PROGRAM's byte layout actually matches what its own doc
/// comment claims -- checks the PATH1 ("fdtest"), PATH2 ("fdout"), and
/// WDATA (FDOUT_WRITE_CONTENT) regions at their documented offsets
/// byte-for-byte, the same "verified with a standalone byte-for-byte
/// re-derivation, not hand-counted hex digits" discipline
/// USER_PROGRAM's own doc comment describes, applied here as a real
/// runtime check instead of just an author's claim. Called once at boot
/// from kernel_main, logging a real pass/fail.
pub fn self_test_fdtest_program() {
    let path1_ok = &FDTEST_PROGRAM[176..182] == FDTEST_READ_PATH.as_bytes();
    let path2_ok = &FDTEST_PROGRAM[186..191] == b"fdout";
    let wdata_ok = &FDTEST_PROGRAM[195..264] == FDOUT_WRITE_CONTENT.as_bytes();
    let _ = writeln!(
        serial(),
        "milestone 35: self-test -- FDTEST_PROGRAM layout check: path1('fdtest')={path1_ok} path2('fdout')={path2_ok} wdata({} bytes)={wdata_ok} -- {}",
        FDOUT_WRITE_CONTENT.len(),
        if path1_ok && path2_ok && wdata_ok { "all match, layout confirmed" } else { "FAILED -- byte layout drifted from doc comment" }
    );
}

/// The real, honest size check run_file() applies to every file it
/// reads, BEFORE any bytes are copied into a process's code frame --
/// factored out into its own function so self_test_size_check() (below)
/// can exercise this EXACT code path directly, rather than recomputing
/// the same comparison a second time and merely hoping the two stay in
/// sync.
fn check_program_size(len: usize) -> Result<(), String> {
    if len > process::MAX_CODE_IMAGE_BYTES {
        return Err(format!(
            "program is {len} bytes, exceeds the {}-byte code-page capacity -- refusing to load rather than silently overflow into adjacent memory",
            process::MAX_CODE_IMAGE_BYTES
        ));
    }
    Ok(())
}

/// MILESTONE 34: a direct, filesystem-independent proof that
/// check_program_size() above is real, working code. Disclosed
/// honestly: fs.rs's own MAX_FILE_BYTES cap (4096 bytes = 8 sectors)
/// happens to be EXACTLY process::MAX_CODE_IMAGE_BYTES (one page), so
/// no file that can actually exist on THIS filesystem could ever be
/// large enough to trip run_file()'s size check by itself --
/// fs::write_file() already refuses anything bigger before an
/// oversized file could even be created on disk. This calls the SAME
/// check_program_size() run_file() uses, directly, against both a
/// normal-sized and a synthetic oversized length that never touches
/// fs.rs at all -- logging a real pass/fail rather than asserting it
/// works. Called once at boot from kernel_main, so every serial log
/// from every test run of this milestone carries the proof.
pub fn self_test_size_check() {
    let small_len = usertest::USER_PROGRAM.len();
    let small_ok = check_program_size(small_len).is_ok();
    let big_len = process::MAX_CODE_IMAGE_BYTES + 1000;
    let big_rejected = check_program_size(big_len).is_err();
    let _ = writeln!(
        serial(),
        "milestone 34: self-test -- check_program_size({small_len}) accepted={small_ok} (expected true), check_program_size({big_len}) rejected={big_rejected} (expected true, {big_len} is {} bytes over the {}-byte cap)",
        big_len - process::MAX_CODE_IMAGE_BYTES,
        process::MAX_CODE_IMAGE_BYTES
    );
}

/// MILESTONE 34: the `runfile PATH` shell command's entry point. Reads
/// `path`'s bytes off the real on-disk filesystem, sanity-checks the
/// size against the code frame's real 4KiB capacity BEFORE any copy
/// happens (process::create_process_from_image enforces the same bound
/// again internally -- belt and suspenders, not relied on to catch what
/// this check should already have caught), then hands the bytes to
/// process::create_loaded_process()/run_loaded_process() -- the same
/// private-PML4 mechanism Milestone 30 built, just fed from this file
/// read instead of the compiled-in USER_PROGRAM array. MILESTONE 35:
/// these were originally one function (load_and_run_image()); split so
/// the ring-3 excursion below no longer runs from inside
/// memory::with_frame_allocator's closure -- see this function's own
/// implementation comment for the real bug that fixed.
pub fn run_file(path: &str) -> Result<(), String> {
    let bytes = fs::read_file(path).map_err(|e| format!("runfile: could not read '{path}': {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 34: runfile '{path}' -- read {} real bytes from the on-disk filesystem",
        bytes.len()
    );

    check_program_size(bytes.len()).map_err(|e| format!("runfile: '{path}' {e}"))?;

    // MILESTONE 35: split into create_loaded_process() (needs
    // frame_allocator, so it runs INSIDE with_frame_allocator's closure
    // -- and only for exactly this call, not a moment longer) then
    // run_loaded_process() (the ring-3 excursion itself, called AFTER
    // that closure -- and therefore the global FRAME_ALLOCATOR lock --
    // has already returned/dropped). This used to be one call
    // (`process::load_and_run_image`) that ran the WHOLE ring-3
    // excursion from inside with_frame_allocator's closure, holding that
    // lock the entire time for no reason (the excursion itself never
    // touches frame_allocator) -- a real, reproducible bug (see
    // process::run_loaded_process's own doc comment for the full
    // diagnosis), found while verifying Milestone 35's fd syscalls but
    // predating them entirely, fixed here.
    let phys_mem_offset = memory::phys_mem_offset();
    let create_result = memory::with_frame_allocator(|frame_allocator| {
        process::create_loaded_process(frame_allocator, phys_mem_offset, &bytes)
    });

    match create_result {
        Some(Ok(())) => process::run_loaded_process().map_err(|e| format!("runfile: {e}")),
        Some(Err(e)) => Err(format!("runfile: {e}")),
        None => Err("runfile: global frame allocator not installed yet (should never happen post-boot)".into()),
    }
}

/// MILESTONE 36: a REAL, externally-built ELF64 executable, embedded
/// into the kernel binary at compile time via `include_bytes!` --
/// genuinely produced by this machine's actual installed Rust toolchain
/// (`rustc --target x86_64-unknown-none`, linked by rust-lld with
/// `-C code-model=large` and a custom linker script forcing two
/// GENUINELY DISTINCT PT_LOAD segments onto two different pages via an
/// explicit `PHDRS` block -- GNU-style linkers otherwise merge same-
/// permission, contiguous-enough output sections into ONE PT_LOAD even
/// across an address gap, confirmed the hard way: an earlier attempt
/// using only address jumps in `SECTIONS` produced a single merged
/// segment with the gap as padding, not two segments, until the
/// explicit PHDRS block forced the split). The source
/// (kernel/tools_src reference copy is NOT part of this cargo
/// workspace -- see the milestone report for the exact build recipe)
/// defines two real, distinct PT_LOAD segments:
///   - segment 1 @ 0x0000_5555_5000_0000 (== usertest::USER_CODE_ADDR,
///     required exactly -- see process::create_process_from_elf()):
///     just `_start`, which does a real linker-resolved `call` across
///     the page boundary into segment 2.
///   - segment 2 @ 0x0000_5555_5000_1000 (one page later): the actual
///     write+exit syscalls AND the distinguishing message string,
///     genuinely on a separate page from segment 1 -- reached only by
///     that cross-segment call actually working, i.e. by the kernel's
///     ELF loader having mapped segment 2 at the right address with the
///     right permissions. This is real, load-bearing proof that
///     execution reached code from a NON-zero-offset PT_LOAD segment,
///     not just segment 0.
///
/// Total file size: 624 bytes (comfortably under fs.rs's own
/// MAX_FILE_BYTES/4096-byte cap -- getting there required overriding
/// the linker's default page-size-based segment file-offset alignment
/// (`-z max-page-size=16`), since page-aligned vaddrs 0x1000 apart would
/// otherwise force the SECOND segment's file offset out to 4096 too, to
/// stay congruent mod the linker's default max-page-size -- a
/// congruency this kernel's own byte-copying loader (process.rs's
/// create_process_from_elf(), which never mmaps the file) doesn't
/// actually need, so relaxing it costs nothing real).
static TEST_ELF_BYTES: &[u8] = include_bytes!("../assets/testelf.elf");

/// MILESTONE 40: real, externally-built ELF64 test payload for
/// pipe()/dup2() -- built the exact same way TEST_ELF_BYTES above is
/// (this project's own pinned Rust toolchain + rust-lld, single
/// PT_LOAD segment at USER_CODE_ADDR, source in
/// tools/pipetest_src/pipetest.rs), not hand-assembled.
static PIPETEST_ELF_BYTES: &[u8] = include_bytes!("../assets/pipetest.elf");

/// MILESTONE 44: real, externally-built ELF64 test payload whose
/// e_entry deliberately does NOT equal usertest::USER_CODE_ADDR -- see
/// tools/testelf_altentry_src/ for the build recipe and linker script.
/// Built the same way TEST_ELF_BYTES above is (this project's own
/// pinned Rust toolchain + rust-lld), not hand-assembled.
static ALTENTRY_TEST_ELF_BYTES: &[u8] = include_bytes!("../assets/testelf_altentry.elf");

/// MILESTONE 51: real, externally-built ELF64 test payload exercising
/// the new `malloc()`/`free()` added to libc.rs this milestone -- built
/// the same way TEST_ELF_BYTES/PIPETEST_ELF_BYTES above are (this
/// project's own pinned Rust toolchain + rust-lld, not hand-assembled),
/// source in tools/malloctest_src/. See tools/malloctest_src/README.md
/// for the exact build recipe and tools/malloctest_src/main.rs for the
/// hand-computed predictions this program checks its own allocator's
/// real behavior against.
static MALLOCTEST_ELF_BYTES: &[u8] = include_bytes!("../assets/malloctest.elf");

/// MILESTONE 58: the "receiver" half of the real argv/envp exec() test
/// -- real, externally-built ELF64 payload (same toolchain as every
/// other *_ELF_BYTES above, not hand-assembled) that reads argc/argv/
/// envp straight off its own real initial stack, per the actual x86_64
/// SysV process-entry contract, and checks it against the exact values
/// ARGVLAUNCHER_ELF_BYTES below is known to send. Source in
/// tools/argvtarget_src/, see that directory's own README.md for the
/// exact build recipe (and a real link failure hit and fixed while
/// building it).
static ARGVTARGET_ELF_BYTES: &[u8] = include_bytes!("../assets/argvtarget.elf");

/// MILESTONE 58: the "caller" half of the real argv/envp exec() test --
/// calls the kernel's new EXECARGV syscall (16) with a real 3-entry
/// argv array and a real 1-entry envp array, exec()ing into
/// ARGVTARGET_ELF_BYTES (seeded to disk at `process::EXECARGV_TARGET_
/// PATH` by seed_argvtarget_elf() below). Source in
/// tools/argvlauncher_src/.
static ARGVLAUNCHER_ELF_BYTES: &[u8] = include_bytes!("../assets/argvlauncher.elf");

/// MILESTONE 61: real, externally-built ELF64 test payload exercising
/// the new `string.h` (memcpy/memmove/memset/memcmp/strlen/strcmp/
/// strncmp/strcpy/strncpy/strcat/strchr) and buffered stdio (fopen/
/// fclose/fread/fwrite/fflush/fputc/fgetc/fputs/feof/fprintf-equivalent)
/// added to libc.rs this milestone -- built the same way every other
/// *_ELF_BYTES above is (this project's own pinned Rust toolchain +
/// rust-lld, not hand-assembled), source in tools/stdiotest_src/. See
/// tools/stdiotest_src/README.md for the exact build recipe and
/// tools/stdiotest_src/main.rs for the hand-computed predictions this
/// program checks its own string.h/stdio implementation against.
static STDIOTEST_ELF_BYTES: &[u8] = include_bytes!("../assets/stdiotest.elf");

/// MILESTONE 67: real, externally-built ELF64 test payload -- Tier 3's
/// first slice, a subset-C lexer + minimal recursive-descent parser (NO
/// codegen yet). Built the same way every other *_ELF_BYTES above is
/// (this project's own pinned Rust toolchain + rust-lld, not hand-
/// assembled), source in tools/cc_src/. See tools/cc_src/README.md for
/// the exact build recipe and tools/cc_src/main.rs for the hand-
/// computed token-stream/AST predictions (plus two deliberate error
/// cases) this program checks its own lex()/parse_function() against.
static CC_ELF_BYTES: &[u8] = include_bytes!("../assets/cc.elf");

/// MILESTONE 40: the `seedpipetest` shell command's entry point --
/// same reasoning as seed_test_elf() above, writes the real ELF bytes
/// to disk so `runelf pipetest` (reusing run_elf() below unchanged,
/// no pipe-specific loader code needed) can load and run it.
pub fn seed_pipetest_elf() -> Result<usize, String> {
    let len = PIPETEST_ELF_BYTES.len();
    fs::write_file("pipetest", PIPETEST_ELF_BYTES).map_err(|e| format!("seedpipetest: {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 40: seedpipetest -- wrote {len} real bytes (a genuine externally-built ELF64 executable exercising pipe()/dup2()) to 'pipetest' on the real on-disk filesystem"
    );
    Ok(len)
}

/// MILESTONE 45: the `seedaltentry` shell command's entry point --
/// same reasoning as seed_pipetest_elf() above, writes ALTENTRY_TEST_ELF_
/// BYTES (Milestone 44's staged, non-USER_CODE_ADDR-entry test payload,
/// left unwired to any actual test until now) to the real on-disk
/// filesystem at `process::EXEC_TEST_PATH` ("altentry"), the exact path
/// process::EXEC_TEST_PROGRAM's own hand-assembled exec() call targets.
/// Also called directly, non-interactively, from kernel_main (via
/// process::self_test_real_exec()) so this milestone's own real exec()
/// self-test needs no shell/keyboard interaction to have real content on
/// disk to exec() into.
pub fn seed_test_elf_altentry() -> Result<usize, String> {
    let len = ALTENTRY_TEST_ELF_BYTES.len();
    fs::write_file(crate::process::EXEC_TEST_PATH, ALTENTRY_TEST_ELF_BYTES).map_err(|e| format!("seedaltentry: {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 45: seedaltentry -- wrote {len} real bytes (a genuine externally-built ELF64 executable whose e_entry does NOT equal USER_CODE_ADDR) to '{}' on the real on-disk filesystem",
        crate::process::EXEC_TEST_PATH
    );
    Ok(len)
}

/// MILESTONE 36: writes TEST_ELF_BYTES to a real file ("testelf") on the
/// real on-disk filesystem -- the `seedtestelf` shell command's entry
/// point, mirroring seed_test_program()'s own reasoning above: this is
/// the one piece of test setup that needs genuine non-typeable bytes
/// (a real ELF header, binary program headers, raw machine code) on
/// disk, which the keyboard-driven `write` shell command has no way to
/// produce.
pub fn seed_test_elf() -> Result<usize, String> {
    let len = TEST_ELF_BYTES.len();
    fs::write_file("testelf", TEST_ELF_BYTES).map_err(|e| format!("seedtestelf: {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 36: seedtestelf -- wrote {len} real bytes (a genuine externally-built ELF64 executable, not hand-assembled) to 'testelf' on the real on-disk filesystem"
    );
    Ok(len)
}

/// MILESTONE 36: the `runelf PATH` shell command's entry point. Reads
/// `path`'s bytes off the real on-disk filesystem (same fs::read_file()
/// `runfile` uses above), parses them for real as ELF64 via elf::parse()
/// -- logging the real, parsed e_entry and every PT_LOAD segment's real
/// p_vaddr/p_offset/p_filesz/p_memsz/p_flags to serial BEFORE any
/// mapping happens, so a serial log from this call is independent proof
/// the parser actually read the file's structure -- then hands the
/// parsed elf::ElfImage (plus the original bytes, needed for the actual
/// segment-content copy) to process::load_and_run_elf().
pub fn run_elf(path: &str) -> Result<(), String> {
    let bytes = fs::read_file(path).map_err(|e| format!("runelf: could not read '{path}': {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 36: runelf '{path}' -- read {} real bytes from the on-disk filesystem",
        bytes.len()
    );

    let elf_image = elf::parse(&bytes).map_err(|e| format!("runelf: '{path}' failed to parse as ELF64 -- {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 36: runelf '{path}' -- parsed a REAL ELF64 header: e_entry={:#x}, {} PT_LOAD segment(s)",
        elf_image.entry,
        elf_image.segments.len()
    );
    for (i, seg) in elf_image.segments.iter().enumerate() {
        let _ = writeln!(
            serial(),
            "milestone 36: runelf '{path}' -- PT_LOAD[{i}]: p_vaddr={:#x} p_offset={:#x} p_filesz={:#x} p_memsz={:#x} p_flags={:#x}",
            seg.p_vaddr, seg.p_offset, seg.p_filesz, seg.p_memsz, seg.p_flags
        );
    }

    // MILESTONE 57: split into create_loaded_elf_process() (needs
    // frame_allocator, runs INSIDE with_frame_allocator's closure -- and
    // only for exactly this call) then run_loaded_elf_process() (the
    // ring-3 excursion itself, called AFTER that closure -- and
    // therefore the global FRAME_ALLOCATOR lock -- has already returned.
    // Same real fix, same reasoning, as run_file()'s own Milestone 35
    // split just above -- see create_loaded_elf_process()'s own doc
    // comment in process.rs for the freshly real (not just wasteful)
    // deadlock this closes.
    let phys_mem_offset = memory::phys_mem_offset();
    let create_result = memory::with_frame_allocator(|frame_allocator| {
        process::create_loaded_elf_process(frame_allocator, phys_mem_offset, &bytes, &elf_image)
    });

    match create_result {
        Some(Ok(())) => process::run_loaded_elf_process(elf_image.entry).map_err(|e| format!("runelf: {e}")),
        Some(Err(e)) => Err(format!("runelf: {e}")),
        None => Err("runelf: global frame allocator not installed yet (should never happen post-boot)".into()),
    }
}

/// MILESTONE 36: a direct, filesystem-independent proof that elf::parse()
/// actually parses TEST_ELF_BYTES correctly -- called once at boot from
/// kernel_main, mirroring self_test_size_check()'s own reasoning above,
/// so every serial log from every boot carries real, unconditional proof
/// the embedded ELF is well-formed and this parser reads it correctly,
/// independent of whether `seedtestelf`/`runelf` are ever typed at the
/// shell at all.
pub fn self_test_elf_parse() {
    match elf::parse(TEST_ELF_BYTES) {
        Ok(image) => {
            let _ = writeln!(
                serial(),
                "milestone 36: self-test -- parsed the embedded testelf.elf: e_entry={:#x} (expected {:#x}), {} PT_LOAD segment(s)",
                image.entry,
                crate::usertest::USER_CODE_ADDR,
                image.segments.len()
            );
            for (i, seg) in image.segments.iter().enumerate() {
                let _ = writeln!(
                    serial(),
                    "milestone 36: self-test -- PT_LOAD[{i}]: p_vaddr={:#x} p_offset={:#x} p_filesz={:#x} p_memsz={:#x} p_flags={:#x}",
                    seg.p_vaddr, seg.p_offset, seg.p_filesz, seg.p_memsz, seg.p_flags
                );
            }
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 36: self-test FAILED -- elf::parse(embedded testelf.elf) returned Err: {e}");
        }
    }
}

/// MILESTONE 44's own real, boot-time, non-interactive proof that the
/// generalized ELF loader genuinely ACCEPTS AND RUNS a non-
/// `usertest::USER_CODE_ADDR` entry point -- not just that
/// process::create_process_from_elf()'s old "entry MUST equal
/// USER_CODE_ADDR" rejection was removed (see that function's own doc
/// comment), but that a real ring-3 excursion actually reaches
/// ALTENTRY_TEST_ELF_BYTES's own `alt_entry()`, three pages past
/// USER_CODE_ADDR (see tools/testelf_altentry_src/), runs its write+exit
/// syscalls, and returns cleanly to kernel context. Same "sendkey is
/// unreliable, run unattended instead" reasoning as every other
/// self_test_* in this codebase; must run from kernel_main AFTER
/// interrupts::init_pics()/sti(), same real reason as
/// process::self_test_signals()/self_test_wait_status() (this enters
/// ring 3 for real, and Milestone 41's own hard-won PIC-ordering lesson
/// applies here too).
///
/// Two real checks, both boolean, neither relying on eyeballing the
/// serial log:
///   1. elf::parse() on the embedded bytes reports an e_entry that does
///      NOT equal usertest::USER_CODE_ADDR -- proves this test payload
///      genuinely exercises the new code path rather than accidentally
///      still using the old fixed address.
///   2. AFTER running it for real through process::load_and_run_elf()
///      (the EXACT function `runelf` itself calls, no test-only
///      shortcut), process::LAST_WRITE_SYSCALL_PID -- set unconditionally
///      inside the real syscall dispatcher; see that static's own doc
///      comment -- is checked against process::LOADED_PROCESS_ID. This
///      is deliberately NOT the same thing as "load_and_run_elf()
///      returned Ok(())": since Milestone 41, a faulting ring-3 process
///      is gracefully terminated and resumes kernel context the SAME way
///      a clean exit does (usertest::terminate_faulted_process_and_resume_kernel()
///      unwinds to the identical KERNEL_RSP anchor syscall 1/exit does),
///      so `Ok(())` alone can no longer distinguish "the new entry point
///      genuinely ran" from "the old bug regressed, IRETQ'd into the
///      UNMAPPED USER_CODE_ADDR page (this test ELF's only PT_LOAD
///      segment deliberately does NOT cover it), and immediately page-
///      faulted" -- both return `Ok(())` with no panic either way.
///      Checking LAST_WRITE_SYSCALL_PID closes that real gap: reaching
///      the write syscall AT ALL is only possible if RIP genuinely
///      started executing from inside the mapped page at the real,
///      parsed e_entry -- exactly this milestone's own point.
pub fn self_test_altentry_elf() {
    let elf_image = match elf::parse(ALTENTRY_TEST_ELF_BYTES) {
        Ok(img) => img,
        Err(e) => {
            let _ = writeln!(
                serial(),
                "milestone 44: self-test FAILED -- elf::parse(embedded testelf_altentry.elf) returned Err: {e}"
            );
            return;
        }
    };

    let entry_is_alt = elf_image.entry != usertest::USER_CODE_ADDR;
    let _ = writeln!(
        serial(),
        "milestone 44: self-test -- parsed embedded testelf_altentry.elf: e_entry={:#x} (USER_CODE_ADDR={:#x}) -- {}",
        elf_image.entry,
        usertest::USER_CODE_ADDR,
        if entry_is_alt {
            "genuinely a non-default entry point"
        } else {
            "MATCHES USER_CODE_ADDR -- this test payload isn't exercising the new code path"
        }
    );
    if !entry_is_alt {
        let _ = writeln!(serial(), "milestone 44: self-test -- OVERALL: FAIL (test payload setup is wrong)");
        return;
    }

    // Reset before running -- a stale value left over from an earlier
    // boot-time self-test (e.g. PROCESS_A's own write syscalls, or a
    // previous run of this very test) would otherwise make the
    // post-run check below pass trivially without this run having
    // reached the syscall at all.
    process::LAST_WRITE_SYSCALL_PID.store(0, Ordering::SeqCst);

    let _ = writeln!(
        serial(),
        "milestone 44: self-test -- running the alt-entry ELF via process::create_loaded_elf_process()/run_loaded_elf_process() (the SAME functions `runelf` itself calls)..."
    );
    // MILESTONE 57: split the same way run_elf()/self_test_malloc() are
    // split -- the ring-3 excursion (run_loaded_elf_process) must NOT
    // run from inside with_frame_allocator()'s closure. This particular
    // self-test never touches its heap, so the deadlock this closes was
    // never actually reachable through THIS specific call before -- but
    // it's the same shared load_and_run_elf() function self_test_malloc()
    // below DID reproducibly deadlock through, so it's fixed here too for
    // real consistency, not left as a live footgun for the next self-test
    // that happens to touch its heap.
    let phys_mem_offset = memory::phys_mem_offset();
    let create_result = memory::with_frame_allocator(|frame_allocator| {
        process::create_loaded_elf_process(frame_allocator, phys_mem_offset, ALTENTRY_TEST_ELF_BYTES, &elf_image)
    });
    let run_result = match create_result {
        Some(Ok(())) => process::run_loaded_elf_process(elf_image.entry),
        Some(Err(e)) => Err(e),
        None => Err("frame allocator not installed"),
    };

    let returned_ok = run_result.is_ok();
    let _ = writeln!(
        serial(),
        "milestone 44: self-test -- run_loaded_elf_process() returned {} (no panic either way -- see this function's own doc comment for why that alone isn't the real proof)",
        match &run_result {
            Ok(()) => String::from("Ok(())"),
            Err(e) => format!("Err({e})"),
        }
    );

    let last_write_pid = process::LAST_WRITE_SYSCALL_PID.load(Ordering::SeqCst);
    let reached_write = last_write_pid == process::LOADED_PROCESS_ID;
    let _ = writeln!(
        serial(),
        "milestone 44: self-test -- LAST_WRITE_SYSCALL_PID after running = {last_write_pid} (expected {}, LOADED_PROCESS_ID) -- {}",
        process::LOADED_PROCESS_ID,
        if reached_write {
            "confirmed: ring-3 execution genuinely reached the write syscall from inside the new entry point's own mapped page"
        } else {
            "FAILED -- the write syscall was never reached for this process"
        }
    );

    let _ = writeln!(
        serial(),
        "milestone 44: self-test -- OVERALL: {}",
        if entry_is_alt && returned_ok && reached_write { "PASS" } else { "FAIL" }
    );
}

/// MILESTONE 51: the `seedmalloctest` shell command's entry point --
/// same reasoning as seed_pipetest_elf()/seed_test_elf() above, writes
/// the real ELF bytes to disk so `runelf malloctest` can also load and
/// run it interactively (this milestone's own boot-time self-test below
/// runs it non-interactively too, straight from the embedded bytes, but
/// keeping the disk-seeding path lets a human re-run it by hand exactly
/// like every prior ELF test payload).
pub fn seed_malloctest_elf() -> Result<usize, String> {
    let len = MALLOCTEST_ELF_BYTES.len();
    fs::write_file("malloctest", MALLOCTEST_ELF_BYTES).map_err(|e| format!("seedmalloctest: {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 51: seedmalloctest -- wrote {len} real bytes (a genuine externally-built ELF64 executable exercising malloc()/free()) to 'malloctest' on the real on-disk filesystem"
    );
    Ok(len)
}

/// MILESTONE 51: a real, boot-time, non-interactive proof that
/// malloc()/free() actually work -- same "sendkey is unreliable, run
/// unattended instead" reasoning as every other self_test_* in this
/// project. Unlike self_test_elf_parse() above (which only proves
/// elf::parse() reads the file's structure correctly), this ACTUALLY
/// LOADS AND RUNS the embedded ELF via process::load_and_run_elf() --
/// the same real ring-3 execution path `runelf` uses interactively,
/// the first time this project's own automated self-test suite has
/// exercised that path non-interactively rather than only via a typed
/// shell command. Safe to do here: Milestone 36's own disclosed
/// "runelf/runfile intermittently page-faults if shell activity
/// follows within ~1s" bug was root-caused and fixed (see the README's
/// own "Fix the Milestone 36 page fault" entry) by making Milestone
/// 25's background task scheduling opt-in, enabled only once, as the
/// very last thing kernel_main does -- every self-test in this file
/// (including this one) runs well before that point, so the actual
/// racing mechanism that bug depended on cannot occur here.
///
/// The real pass/fail evidence for malloc()/free()'s own correctness
/// is written directly to serial BY THE RING-3 PROGRAM ITSELF, via its
/// own real write() syscalls (see tools/malloctest_src/main.rs's
/// hand-computed predictions and its final "OVERALL=PASS/FAIL" line) --
/// this kernel-side wrapper's own job is narrower and honest about it:
/// confirm the ELF parses, confirm load_and_run_elf() returns Ok (no
/// kernel panic, no double fault, no hang) rather than an Err or a
/// crash, and confirm the kernel is still alive and responsive
/// immediately afterward. Grep the serial log around this point for
/// the program's own "milestone 51: malloctest" lines for the actual
/// allocator-correctness evidence.
pub fn self_test_malloc() {
    let elf_image = match elf::parse(MALLOCTEST_ELF_BYTES) {
        Ok(image) => {
            let _ = writeln!(
                serial(),
                "milestone 51: self-test -- parsed the embedded malloctest.elf: e_entry={:#x} (expected {:#x}), {} PT_LOAD segment(s)",
                image.entry,
                usertest::USER_CODE_ADDR,
                image.segments.len()
            );
            image
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 51: self-test FAILED -- elf::parse(embedded malloctest.elf) returned Err: {e}");
            return;
        }
    };

    // MILESTONE 57: a REAL, reproducible deadlock was found and fixed
    // right here during this milestone's own verification -- the
    // original single `load_and_run_elf()` call ran malloctest.elf's
    // ENTIRE ring-3 excursion from inside this `with_frame_allocator()`
    // closure, and malloctest.elf is the first ELF-loaded program in
    // this project's history to actually touch its heap (a real
    // `sys_sbrk()` call followed by a real write to the returned
    // pointer). That first heap write's page fault called
    // `process::try_demand_page_heap()`, which itself needs
    // `memory::with_frame_allocator()` to get a fresh frame -- but
    // `spin::Mutex` is not reentrant, so it spun forever trying to
    // re-acquire the SAME lock this call site already held, confirmed
    // via a real, reproducible QEMU crash (exit code 2, no further
    // kernel output) across multiple fresh boots before being
    // root-caused and fixed by splitting create/run exactly like
    // run_file()'s own Milestone 35 precedent -- see
    // process::create_loaded_elf_process()'s own doc comment for the
    // full story.
    let phys_mem_offset = memory::phys_mem_offset();
    let create_result = memory::with_frame_allocator(|frame_allocator| {
        process::create_loaded_elf_process(frame_allocator, phys_mem_offset, MALLOCTEST_ELF_BYTES, &elf_image)
    });
    let result = match create_result {
        Some(Ok(())) => Some(process::run_loaded_elf_process(elf_image.entry)),
        Some(Err(e)) => Some(Err(e)),
        None => None,
    };

    match result {
        Some(Ok(())) => {
            let _ = writeln!(
                serial(),
                "milestone 51: self-test -- malloctest.elf ran to completion and returned to the kernel cleanly (no panic, no double fault) -- see the 'milestone 51: malloctest' lines above for the real allocator-correctness evidence written by the program itself"
            );
        }
        Some(Err(e)) => {
            let _ = writeln!(serial(), "milestone 51: self-test FAILED -- run_loaded_elf_process(malloctest.elf) returned Err: {e}");
        }
        None => {
            let _ = writeln!(serial(), "milestone 51: self-test FAILED -- global frame allocator not installed yet (should never happen post-boot)");
        }
    }
}

/// MILESTONE 58: the `seedargvtarget` shell command's entry point --
/// same reasoning as seed_malloctest_elf() above, writes
/// ARGVTARGET_ELF_BYTES to the real on-disk filesystem at
/// `process::EXECARGV_TARGET_PATH` ("argvtarget"), the exact path
/// ARGVLAUNCHER_ELF_BYTES's own real EXECARGV syscall call targets.
/// Also called directly, non-interactively, from self_test_execargv()
/// below so this milestone's own self-test needs no shell/keyboard
/// interaction to have real content on disk to exec() into.
pub fn seed_argvlauncher_elf() -> Result<usize, String> {
    let len = ARGVLAUNCHER_ELF_BYTES.len();
    fs::write_file("argvlauncher", ARGVLAUNCHER_ELF_BYTES).map_err(|e| format!("seedargvlauncher: {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 58: seedargvlauncher -- wrote {len} real bytes (a genuine externally-built ELF64 executable calling the real EXECARGV syscall) to 'argvlauncher' on the real on-disk filesystem"
    );
    Ok(len)
}

pub fn seed_argvtarget_elf() -> Result<usize, String> {
    let len = ARGVTARGET_ELF_BYTES.len();
    fs::write_file(crate::process::EXECARGV_TARGET_PATH, ARGVTARGET_ELF_BYTES).map_err(|e| format!("seedargvtarget: {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 58: seedargvtarget -- wrote {len} real bytes (a genuine externally-built ELF64 executable that reads real argv/envp off its own initial stack) to '{}' on the real on-disk filesystem",
        crate::process::EXECARGV_TARGET_PATH
    );
    Ok(len)
}

/// MILESTONE 58: a real, boot-time, non-interactive proof that argv/envp
/// genuinely thread through exec() -- same "sendkey is unreliable, run
/// unattended instead" reasoning as every other self_test_* in this
/// project. First seeds ARGVTARGET_ELF_BYTES to disk (same "self-
/// contained, no shell interaction needed" reasoning as
/// process::self_test_real_exec()'s own seed_test_elf_altentry() call),
/// THEN loads and runs ARGVLAUNCHER_ELF_BYTES via the same real
/// create_loaded_elf_process()/run_loaded_elf_process() split
/// self_test_malloc() above uses. The launcher itself calls the new
/// EXECARGV syscall, which tears down and rebuilds the SAME process
/// slot (LOADED_PROCESS_ID) mid-flight via
/// process::exec_elf_with_args() -- exactly like
/// process::self_test_real_exec()'s own EXEC_TEST_PROCESS does for the
/// plain syscall 9 path -- so run_loaded_elf_process() only returns
/// once the NEWLY exec()'d argvtarget.elf itself calls exit(), not when
/// the launcher's own code would have (a real EXECARGV success never
/// reaches the launcher's own exit() call at all).
///
/// The real pass/fail evidence is written directly to serial BY THE
/// RING-3 PROGRAMS THEMSELVES (argvlauncher.elf's own startup/fallback
/// messages, argvtarget.elf's own hand-computed argv/envp checks and
/// final "OVERALL=PASS/FAIL" line) -- this kernel-side wrapper's own
/// job is narrower and honest about it: confirm the target seeds to
/// disk, confirm the ELF loads and parses, confirm run_loaded_elf_
/// process() returns Ok (no kernel panic, no double fault, no hang)
/// rather than an Err or a crash. Grep the serial log around this point
/// for the programs' own "milestone 58:" lines for the actual
/// argv/envp-correctness evidence.
pub fn self_test_execargv() {
    if let Err(e) = seed_argvtarget_elf() {
        let _ = writeln!(serial(), "milestone 58: self-test FAILED -- could not seed argvtarget.elf to disk: {e}");
        return;
    }

    let elf_image = match elf::parse(ARGVLAUNCHER_ELF_BYTES) {
        Ok(image) => {
            let _ = writeln!(
                serial(),
                "milestone 58: self-test -- parsed the embedded argvlauncher.elf: e_entry={:#x} (expected {:#x}), {} PT_LOAD segment(s)",
                image.entry,
                usertest::USER_CODE_ADDR,
                image.segments.len()
            );
            image
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 58: self-test FAILED -- elf::parse(embedded argvlauncher.elf) returned Err: {e}");
            return;
        }
    };

    let phys_mem_offset = memory::phys_mem_offset();
    let create_result = memory::with_frame_allocator(|frame_allocator| {
        process::create_loaded_elf_process(frame_allocator, phys_mem_offset, ARGVLAUNCHER_ELF_BYTES, &elf_image)
    });
    let result = match create_result {
        Some(Ok(())) => Some(process::run_loaded_elf_process(elf_image.entry)),
        Some(Err(e)) => Some(Err(e)),
        None => None,
    };

    match result {
        Some(Ok(())) => {
            let _ = writeln!(
                serial(),
                "milestone 58: self-test -- argvlauncher.elf's real EXECARGV replaced it with argvtarget.elf, which ran to completion and returned to the kernel cleanly (no panic, no double fault) -- see the 'milestone 58:' lines above for the real argv/envp-correctness evidence written by the programs themselves"
            );
        }
        Some(Err(e)) => {
            let _ = writeln!(serial(), "milestone 58: self-test FAILED -- run_loaded_elf_process(argvlauncher.elf) returned Err: {e}");
        }
        None => {
            let _ = writeln!(serial(), "milestone 58: self-test FAILED -- global frame allocator not installed yet (should never happen post-boot)");
        }
    }
}

/// MILESTONE 61: the `seedstdiotest` shell command's entry point -- same
/// reasoning as seed_malloctest_elf() above, writes the real ELF bytes
/// to disk so `runelf stdiotest` can also load and run it interactively
/// (the boot-time self-test below runs the SAME embedded bytes
/// non-interactively; this is the interactive re-run path). NOTE:
/// stdiotest.elf's own self-test writes real files of its own
/// ("stdiotest_a"/"stdiotest_b") to the SAME on-disk filesystem this
/// seeds into -- running it more than once in the same boot (or after
/// `seedstdiotest` itself) is safe (fopen("w") on an existing path just
/// reuses/extends that path's existing on-disk entry, it doesn't fail),
/// but see libc.rs's own stdio doc comment for the real, disclosed
/// no-O_TRUNC caveat that applies if a re-run's total written length
/// ever ends up shorter than a previous run's.
pub fn seed_stdiotest_elf() -> Result<usize, String> {
    let len = STDIOTEST_ELF_BYTES.len();
    fs::write_file("stdiotest", STDIOTEST_ELF_BYTES).map_err(|e| format!("seedstdiotest: {e}"))?;
    let _ = writeln!(
        serial(),
        "milestone 61: seedstdiotest -- wrote {len} real bytes (a genuine externally-built ELF64 executable exercising string.h + buffered stdio) to 'stdiotest' on the real on-disk filesystem"
    );
    Ok(len)
}

/// MILESTONE 61: a real, boot-time, non-interactive proof that the new
/// string.h + buffered stdio actually work -- same "sendkey is
/// unreliable, run unattended instead" reasoning, and the same real
/// create/run split (self_test_malloc()'s own doc comment above tells
/// the full, already-fixed deadlock story this split closes) as every
/// other ELF-loading self-test in this file.
///
/// The real pass/fail evidence -- including every hand-computed
/// prediction's own PASS/FAIL (string.h correctness, the exact internal
/// `wlen`/`rlen`/`rpos` buffering-mechanism values, the append-mode
/// round trip, the fprintf-equivalent's byte-exact output) -- is written
/// directly to serial BY THE RING-3 PROGRAM ITSELF, via its own real
/// write() syscalls (see tools/stdiotest_src/main.rs's own inline
/// comments for exactly what each check predicts and why). This
/// kernel-side wrapper's own job stays the same narrow, honest scope
/// self_test_malloc() above already established: confirm the ELF
/// parses, confirm run_loaded_elf_process() returns Ok (no panic, no
/// double fault, no hang), confirm the kernel is still alive
/// immediately afterward. Grep the serial log around this point for the
/// program's own "milestone 61: stdiotest" lines for the actual
/// string.h/stdio-correctness evidence.
pub fn self_test_stdio() {
    let elf_image = match elf::parse(STDIOTEST_ELF_BYTES) {
        Ok(image) => {
            let _ = writeln!(
                serial(),
                "milestone 61: self-test -- parsed the embedded stdiotest.elf: e_entry={:#x} (expected {:#x}), {} PT_LOAD segment(s)",
                image.entry,
                usertest::USER_CODE_ADDR,
                image.segments.len()
            );
            image
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 61: self-test FAILED -- elf::parse(embedded stdiotest.elf) returned Err: {e}");
            return;
        }
    };

    let phys_mem_offset = memory::phys_mem_offset();
    let create_result = memory::with_frame_allocator(|frame_allocator| {
        process::create_loaded_elf_process(frame_allocator, phys_mem_offset, STDIOTEST_ELF_BYTES, &elf_image)
    });
    let result = match create_result {
        Some(Ok(())) => Some(process::run_loaded_elf_process(elf_image.entry)),
        Some(Err(e)) => Some(Err(e)),
        None => None,
    };

    match result {
        Some(Ok(())) => {
            let _ = writeln!(
                serial(),
                "milestone 61: self-test -- stdiotest.elf ran to completion and returned to the kernel cleanly (no panic, no double fault) -- see the 'milestone 61: stdiotest' lines above for the real string.h/stdio-correctness evidence written by the program itself"
            );
        }
        Some(Err(e)) => {
            let _ = writeln!(serial(), "milestone 61: self-test FAILED -- run_loaded_elf_process(stdiotest.elf) returned Err: {e}");
        }
        None => {
            let _ = writeln!(serial(), "milestone 61: self-test FAILED -- global frame allocator not installed yet (should never happen post-boot)");
        }
    }
}

/// MILESTONE 67/68: a real, boot-time, non-interactive proof that Tier 3's
/// first two slices -- the subset-C lexer + minimal recursive-descent
/// parser (Milestone 67), and real x86_64 machine-code generation from
/// the resulting AST (Milestone 68) -- both actually work when run as a
/// real spikeling-os ring-3 process. Same real create/run split, and the
/// same narrow, honest kernel-side scope, as self_test_stdio() above:
/// confirm the ELF parses, confirm run_loaded_elf_process() returns Ok
/// (no panic, no double fault, no hang), confirm the kernel is still
/// alive immediately afterward. Every actual lex/parse/codegen-
/// correctness check -- Milestone 67's full 19-token hand-computed
/// stream, the resulting AST's exact shape, two deliberate lex/parse
/// error cases, AND Milestone 68's real compile-then-EXECUTE cases
/// (the generated machine code is actually called through a Rust
/// function pointer and its real returned integer checked against a
/// hand-computed expected value) plus one deliberate codegen error case
/// -- is written directly to serial BY THE RING-3 PROGRAM ITSELF via its
/// own real write() syscalls (see tools/cc_src/main.rs's own inline
/// comments for exactly what each check predicts and why). Grep the
/// serial log around this point for the program's own "milestone 67:
/// cc" and "milestone 68: cc codegen" lines for the actual evidence.
pub fn self_test_cc() {
    let elf_image = match elf::parse(CC_ELF_BYTES) {
        Ok(image) => {
            let _ = writeln!(
                serial(),
                "milestone 67: self-test -- parsed the embedded cc.elf: e_entry={:#x} (expected {:#x}), {} PT_LOAD segment(s)",
                image.entry,
                usertest::USER_CODE_ADDR,
                image.segments.len()
            );
            image
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 67: self-test FAILED -- elf::parse(embedded cc.elf) returned Err: {e}");
            return;
        }
    };

    let phys_mem_offset = memory::phys_mem_offset();
    let create_result = memory::with_frame_allocator(|frame_allocator| {
        process::create_loaded_elf_process(frame_allocator, phys_mem_offset, CC_ELF_BYTES, &elf_image)
    });
    let result = match create_result {
        Some(Ok(())) => Some(process::run_loaded_elf_process(elf_image.entry)),
        Some(Err(e)) => Some(Err(e)),
        None => None,
    };

    match result {
        Some(Ok(())) => {
            let _ = writeln!(
                serial(),
                "milestone 67: self-test -- cc.elf ran to completion and returned to the kernel cleanly (no panic, no double fault) -- see the 'milestone 67: cc' lines above for the real lexer/parser-correctness evidence written by the program itself"
            );
        }
        Some(Err(e)) => {
            let _ = writeln!(serial(), "milestone 67: self-test FAILED -- run_loaded_elf_process(cc.elf) returned Err: {e}");
        }
        None => {
            let _ = writeln!(serial(), "milestone 67: self-test FAILED -- global frame allocator not installed yet (should never happen post-boot)");
        }
    }
}
