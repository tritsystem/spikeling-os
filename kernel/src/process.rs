//! MILESTONE 30: real per-process address space isolation. Milestone 27
//! got exactly one ring-3 program running, but disclosed honestly in its
//! own report that it ran under the KERNEL's own page tables -- the user
//! code page and user stack page were mapped into the SAME
//! OffsetPageTable kernel_main built at boot, alongside every other
//! kernel mapping, with nothing but the USER_ACCESSIBLE flag standing
//! between "ring 3" and "can see the whole kernel address space". This
//! module closes that gap: each `Process` gets its own top-level page
//! table (PML4) in its own physical frame, entered via a real CR3 switch
//! before `iretq` and switched back on exit -- proven by running two
//! distinct hardcoded processes at the IDENTICAL virtual code address
//! (usertest::USER_CODE_ADDR) and showing each one genuinely executes
//! its own distinct physical memory, invisible to the other.
//!
//! The design deliberately does NOT deep-copy the kernel's page table
//! hierarchy into each process -- a naive full copy would silently go
//! stale the instant the kernel maps anything new later (e.g. heap
//! growth), a real bug class this avoids rather than risks:
//!
//!   - every PML4 entry OUTSIDE the user-space index is a raw COPY OF
//!     THE ENTRY ITSELF (a pointer to the kernel's existing, already-
//!     built P3 table), not a deep copy of the hierarchy underneath it
//!     -- so every process's kernel-space view stays bit-for-bit
//!     identical to the kernel's own forever, automatically, because
//!     it's the literal same physical P3 table in memory. Any future
//!     kernel mapping change is instantly visible to every process, with
//!     zero ongoing sync cost and zero staleness risk.
//!   - the ONE PML4 entry covering both usertest::USER_CODE_ADDR and
//!     usertest::USER_STACK_ADDR -- computed for real below (p4_index()
//!     on the actual addresses, not assumed), both landing on index 170
//!     -- is left zeroed in the new PML4, then `OffsetPageTable::map_to`
//!     is used against that fresh table to build a genuinely new,
//!     private P3/P2/P1 chain backed by this process's own physical
//!     frames. create_process() checks the two addresses actually share
//!     a p4 index at runtime and fails loudly (not silently) if that
//!     assumption is ever violated.
//!
//! Still exactly two hardcoded processes (no general loader, no
//! per-process heap or file descriptors -- later work). The isolation
//! itself is real: verified by CR3 genuinely changing (logged before and
//! after every switch), by each process's write(ptr,len) syscall reading
//! its OWN message out of ITS OWN physical code-page frame at the SAME
//! virtual offset every other process uses, and by re-running process A
//! after process B with no cross-contamination.
//!
//! MILESTONE 31: the syscall that proves this per-process isolation
//! generalized from Milestone 27's fixed-string "print" (syscall 0 took
//! no arguments at all) to a real `write(ptr, len)` -- create_process()
//! below now installs each process's message through
//! usertest::write_fixed_message() (the exact same helper the legacy
//! usertest.rs code path uses for ITS message), and the old
//! read_active_message() helper that only ever read one hardcoded offset
//! is gone, superseded by usertest.rs's syscall_dispatch generically
//! reading whatever `ptr`/`len` the calling process's own USER_PROGRAM
//! passed in registers.
//!
//! MILESTONE 33: each process also gets its own private HEAP, a fixed
//! 16 KiB region at HEAP_START pre-mapped by create_process() the same
//! way code/stack are. `HEAP_START` shares USER_CODE_ADDR/
//! USER_STACK_ADDR's p4_index (170), so it lands inside the SAME
//! already-private PML4 slot Milestone 30 rebuilds per process --
//! genuinely private with zero new PML4-level reasoning needed. A real
//! syscall (2 = sbrk, in usertest.rs) is the process's only way to grow
//! into it, chosen over a kernel-only allocator because `int 0x80` is
//! this kernel's one sanctioned ring-3-to-ring-0 boundary: a kernel-side
//! allocator unreachable from ring 3 wouldn't give the PROCESS a real
//! way to allocate. Process A/B now run PROCESS_PROGRAM (this module's
//! own hand-assembled binary, not usertest::USER_PROGRAM) so they can
//! exercise sbrk before printing; usertest::USER_PROGRAM and the plain
//! `usertest` command are completely untouched.
//!
//! MILESTONE 34: a real general program loader (kernel/src/loader.rs)
//! removes the "hardcoded" half of the original limitation. The page-
//! table-building core of create_process() is factored out into
//! create_process_from_image(), which builds the SAME private
//! PML4/P3/P2/P1 chain (code, stack, AND heap -- every process gets one
//! uniformly, loaded-from-file or not) from an arbitrary `&[u8]` code
//! image instead of always copying PROCESS_PROGRAM. create_process()
//! (used by PROCESS_A/PROCESS_B) becomes a thin wrapper that builds its
//! PROCESS_PROGRAM+heap-marker+message image as a `Vec<u8>` first, then
//! hands it to the shared core -- loader.rs's `runfile` calls the new
//! create_loaded_process()/run_loaded_process() below (MILESTONE 35:
//! split from one original load_and_run_image() function -- see
//! run_loaded_process()'s own doc comment) with bytes read straight from
//! the real on-disk filesystem instead. No page-table code is duplicated between
//! the two paths.
//!
//! MILESTONE 36: a real ELF64 loader (kernel/src/elf.rs does the actual
//! parsing; create_process_from_elf() below does the loading).
//! Deliberately kept SEPARATE from create_process_from_image() above,
//! rather than folded into it or refactored to share its body -- partly
//! because the shapes genuinely differ (one fixed page at one fixed
//! address vs. an arbitrary number of pages at segment-declared
//! addresses), and partly a deliberate choice to minimize process.rs
//! churn while Milestone 35 (real per-process file descriptors) is
//! being built concurrently against this same file from the same
//! Milestone 34 baseline -- additive new functions merge far more
//! cleanly than a refactor of shared code both branches touch.
//!
//! The real, honest scoping decision (option (a) from the milestone
//! brief, not option (b)): this milestone does NOT make the ring-3
//! entry trampoline's jump target dynamic. usertest::enter_ring3_now()
//! still hardcodes USER_CODE_ADDR as the iretq target, exactly as
//! Milestone 27 left it. So create_process_from_elf() below requires --
//! checked for real, not assumed -- that the ELF's own e_entry equals
//! USER_CODE_ADDR exactly, and that every PT_LOAD segment is page-
//! aligned and fits a small, fixed per-segment/total page cap. This is
//! real, deeper surgery on Milestone 27/30's carefully-built ring-3
//! entry mechanism deliberately AVOIDED in favor of shipping a smaller,
//! fully-verified change: an ELF genuinely linked for this kernel's own
//! fixed entry address loads and runs for real, with real multi-page,
//! multi-segment mapping (not "arbitrary Linux ELF binaries", which
//! would additionally need position-independent/dynamic entry support,
//! a real page-fault-driven demand loader or relocations, and is
//! explicitly out of scope here, same as this project's own README
//! already scopes "full Linux ELF/libc compatibility" as separate,
//! much-later work).
//!
//! MILESTONE 37: real `fork`/`exec`/`wait` -- the "single biggest
//! structural gap toward a real Unix process model" the README's own
//! Linux-comparability roadmap named after Milestone 36. **The real,
//! honest scoping decision, stated up front**: this is option (a) from
//! the milestone brief, NOT option (b). `tasks.rs`/`scheduler.rs`'s real
//! preemptive, timer-interrupt-driven scheduler is UNCHANGED by this
//! milestone -- it still only schedules ring-0 kernel tasks, exactly as
//! Milestone 25 left it. Ring-3 process execution stays exactly what it
//! has been since Milestone 27: a synchronous "call out and block" model
//! where the kernel is not doing anything ELSE while a ring-3 process
//! runs. `fork()` genuinely creates a new process -- a new PML4, new
//! physical frames, and a REAL byte-for-byte copy of the parent's own
//! code/stack/heap frame CONTENTS (not shared references -- this kernel
//! still has no copy-on-write) -- but the child does not run
//! "concurrently" with the parent in any sense a real scheduler would
//! recognize. Instead, exactly as the milestone brief's own suggested
//! v1 describes: the child is created and its address space fully,
//! genuinely built at `fork()` time, but it only actually EXECUTES when
//! the parent's own `wait()` syscall explicitly drives it -- `wait()`
//! synchronously switches CR3 to the child, runs it all the way to ITS
//! OWN `exit()` syscall (nested inside the parent's `wait()` syscall's
//! own dispatch), then restores the parent's context and returns. This
//! is a genuine, real implementation of "block until a child changes
//! state" for a kernel with no other way to run "something else" in the
//! meantime -- not a simulation of it.
//!
//! Two further real, honest, DISCLOSED simplifications on top of that
//! core scoping decision:
//!   - A forked child resumes at the EXACT (rip, rsp) captured from the
//!     parent's own hardware-recorded `SyscallRegs` at the moment
//!     `fork()` was called (with rax forced to 0, fork()'s "0 in the
//!     child" contract) -- NOT at `USER_CODE_ADDR`. This matters for
//!     real correctness, not just fidelity: the ring-3 entry trampoline
//!     (`usertest::enter_ring3_now()`) was deliberately kept fixed at
//!     `USER_CODE_ADDR` rather than made dynamic (same Milestone 36
//!     decision, restated above) -- if the child were instead restarted
//!     from the top of its own program, it would immediately execute
//!     the SAME `fork()` call again (an infinite fork loop), not
//!     "continue past its own fork() call" the way real fork() must.
//!     `usertest::enter_ring3_as_forked_child()` is a second, dedicated
//!     entry trampoline built for exactly this -- see its own doc
//!     comment.
//!   - Nesting depth is bounded at exactly 1, enforced for real (checked
//!     and refused, not just documented): a forked child currently being
//!     resumed by `wait()` cannot itself `fork()`/`wait()`. This is a
//!     direct, honest consequence of using ONE dedicated kernel-resume
//!     anchor (`usertest::CHILD_KERNEL_RSP`) for the nested child
//!     excursion rather than a general per-nesting-level stack of
//!     anchors -- the same "small fixed cap, not full generality" spirit
//!     as `MAX_OPEN_FILES`/`MAX_LOAD_SEGMENTS` elsewhere in this
//!     codebase, applied to nesting depth instead of a table size.
//!
//! A real, dynamic, bounded PROCESS_TABLE (MAX_PROCESSES = 4, real PID
//! allocation starting at PID_TABLE_BASE = 10) backs `fork()`-created
//! children -- see that static's own doc comment for why the FOUR
//! pre-existing hardcoded/loaded-file slots (PROCESS_A, PROCESS_B,
//! LOADED_PROCESS, FDTEST_PROCESS) were left as legacy, separate statics
//! rather than migrated in. `exec()` reuses this same table/dispatch
//! uniformly (any process id, hardcoded or forked, can `exec()`).
//!
//! **MILESTONE 45 update -- this was the real, disclosed scope-cut this
//! milestone's own module doc comment flagged above, now closed**:
//! Milestone 37's own `exec()` only ever replaced the calling process's
//! code-frame CONTENTS in place (a raw flat-binary copy into the SAME
//! code page, same PML4, no ELF parsing at all -- disclosed at the time
//! as a placeholder, not a real loader). `exec()` now genuinely tears
//! down and rebuilds the calling process's ENTIRE address space -- a
//! new PML4, new physical code/stack/heap frames, one page per real
//! PT_LOAD segment -- via `create_process_from_elf()` (this module's own
//! Milestone 36 ELF loader, the EXACT SAME function `runelf` already
//! uses, not a duplicated one), reading and parsing a genuine ELF64
//! file off disk instead of copying an opaque byte blob. Same open fds,
//! same parent_pid, same pgid (real POSIX exec() invariants, explicitly
//! preserved -- see `exec_elf()`'s own doc comment), but a genuinely NEW
//! PML4/CR3 and a genuinely new entry point (the target ELF's own real,
//! parsed `e_entry`, per Milestone 44's generalization). See
//! `exec_elf()`'s own doc comment for the full real, honest teardown
//! story (this kernel's frame allocator has never reclaimed a physical
//! frame on exit/reap/kill either -- exec() inherits that existing,
//! already-disclosed gap, it does not introduce a new one).
//!
//! Verified for real: `runfork` (new shell command) runs a new
//! hand-assembled FORK_TEST_PROGRAM that calls `fork()`, branches on the
//! return value (0 = child), has the parent print a distinguishing
//! message and then `wait()` for the child, and has the child attempt
//! `exec()` into an on-disk program -- real, observable proof that
//! fork() created a genuinely separate process (different PID,
//! independently verified physical frames), with the parent's `wait()`
//! genuinely blocking until that whole sequence completes. See the
//! Milestone 37 report for the actual captured serial log of that
//! demo. **Honest, disclosed Milestone 45 behavior change**: `exec()`
//! now requires a real ELF64 image (same format `runelf` has always
//! required) -- FORK_TEST_PROGRAM's own child still targets `testprog`,
//! a Milestone 34 FLAT binary, which is no longer a valid exec()
//! target, so that specific demo now exercises the CHILD's own honest
//! fallback path (see FORK_TEST_PROGRAM's own doc comment) rather than
//! a successful exec(); real exec() itself is separately, freshly
//! verified by Milestone 45's own EXEC_TEST_PROCESS / `runexectest` /
//! `self_test_real_exec()` against a genuine ELF64 target instead. See
//! this milestone's own report for the real captured serial log.

use crate::elf;
use crate::errno;
use crate::fs;
use crate::memory;
use crate::serial;
use crate::signal;
use crate::usertest;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt::Write;
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use spin::Mutex;
use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::page_table::PageTableIndex;
use x86_64::structures::paging::{FrameAllocator, FrameDeallocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};

const PAGE_SIZE: usize = 4096;

/// MILESTONE 34: the real, honest bound loader.rs's `runfile` checks a
/// file's size against BEFORE reading it into a process's code frame --
/// one page, the code frame's actual real capacity, not a made-up
/// number. Exported so loader.rs's own pre-check (and its self-test)
/// use this SAME constant rather than a second, potentially-drifting
/// copy of "4096".
pub(crate) const MAX_CODE_IMAGE_BYTES: usize = PAGE_SIZE;

/// MILESTONE 33: per-process heap virtual range. Shares p4_index 170
/// with USER_CODE_ADDR/USER_STACK_ADDR (verified, not assumed -- see
/// create_process()'s own p4_index check) and p3_index 341 with them
/// too, but lands at a distinct p2_index (384, vs. 128/256 for
/// code/stack), so its 4 pages can never collide with the single code
/// or stack page, with 256 MiB of headroom either side. Fully disjoint
/// from the kernel's own heap (`allocator::HEAP_START`, p4_index 136).
pub const HEAP_START: u64 = 0x_5555_7000_0000;
/// MILESTONE 57: 64 4 KiB pages (256 KiB) of RESERVED virtual address
/// space -- `sbrk` still bumps within this fixed bound and fails cleanly
/// once exhausted (same "no free/realloc, bump-only" scope as before),
/// but as of this milestone the pages themselves are no longer
/// pre-mapped up front. Only bookkeeping (`heap_used`) claims virtual
/// range at `sbrk()` time; a real physical frame is allocated and mapped
/// for a given page ONLY the first time a real hardware page fault
/// touches it (see `try_demand_page_heap()`) -- genuine demand paging,
/// not "reserve == pay". This is why the reservation could grow 16x (4
/// pages -> 64) for zero eager physical cost: before Milestone 57 this
/// same 64-page bound would have cost every process 256 KiB of real
/// physical memory at creation whether it ever touched its heap or not;
/// now a process that never calls `sbrk()` maps zero heap frames, ever.
const HEAP_PAGE_COUNT: u64 = 64;
const HEAP_SIZE: u64 = HEAP_PAGE_COUNT * PAGE_SIZE as u64;

/// MILESTONE 64: real, bounded read-only file-backed `mmap()` -- the
/// "next real VM increment" the Tier 2 roadmap (see Milestone 63's own
/// dependency-reasoning writeup) named as a bigger, multi-subsystem lift
/// than a single milestone slice normally takes. Deliberately scoped down
/// to the smallest HONEST first slice rather than attempted whole: no
/// `PROT_WRITE`, no copy-on-write, no shared/anonymous mappings, no
/// `MAP_FIXED`/arbitrary length/offset -- exactly ONE real property, done
/// for real: mapping an already-open fd's file content READ-ONLY into a
/// fresh virtual range, backed by a genuine hardware page fault on first
/// touch (the SAME `try_demand_page_heap()`/`page_fault_handler()`
/// mechanism Milestone 57 built, generalized to a second kind of
/// eligible fault rather than a second unrelated implementation), with
/// the read-only-ness enforced by REAL hardware page-table permission
/// bits, not just an OS-level check -- proven by a write attempt genuinely
/// hardware-faulting and terminating the process, exactly like Milestone
/// 41's SIGSEGV path already does for every other illegal access.
///
/// **Why this is a real, bounded slice, not a stub**: `fs::MAX_FILE_BYTES`
/// (4096 bytes -- see that constant's own comment) is EXACTLY `PAGE_SIZE`,
/// so "map one already-open file" and "map one page" are the same
/// operation here -- no partial-page/multi-page complexity to fake or
/// defer. `OpenFile` (this file's own Milestone 35 struct) already
/// buffers a file's ENTIRE real content in memory at open() time, so
/// `mmap_file()` below needs no new disk-reading code at all -- it
/// snapshots that already-real buffer into a new, dedicated per-slot
/// store (see `MmapSlot`'s own doc comment for why a snapshot, not a
/// live view, is this slice's one disclosed, honest simplification).
///
/// **Virtual range**: `MMAP_START` sits 256 MiB past `HEAP_START` --
/// comfortably clear of the heap's own 64-page (256 KiB) reservation --
/// while still sharing p4_index 170 with `USER_CODE_ADDR`/
/// `USER_STACK_ADDR`/`HEAP_START` (verified the same way `HEAP_START`'s
/// own comment already verifies theirs: `(addr >> 39) & 0x1FF == 170` for
/// all four), which matters concretely: `reclaim_private_page_tables()`
/// only ever walks `user_p4_index`'s own PML4 slot when freeing a dead
/// process's P3/P2/P1 table frames (see its own doc comment on why that's
/// safe) -- an mmap region living in a DIFFERENT p4_index would silently
/// leak its own page-table frames on every process teardown, a real bug
/// this milestone's own frame-reclaim self-test extension below is
/// designed to catch.
pub(crate) const MMAP_START: u64 = HEAP_START + 0x1000_0000;
/// MILESTONE 64: real, small, fixed bound on concurrent mmap regions per
/// process -- same "small fixed cap, not full generality" discipline as
/// `MAX_OPEN_FILES`/`MAX_PROCESSES` elsewhere in this file. 4 is
/// comfortably more than this milestone's own self-test ever has mapped
/// at once (exactly one at a time, per test process) while leaving real
/// headroom. `mmap_file()` returns a real, disclosed ENOMEM failure once
/// a process's own table is full, rather than growing without bound.
pub(crate) const MAX_MMAPS: usize = 4;
/// MILESTONE 64: same real, enforced (not just documented) reasoning as
/// `_ASSERT_MAX_OPEN_FILES_IS_4` -- both `Process` constructors hardcode
/// `[None, None, None, None]` for `mmaps` (no `Clone`/`Copy` on
/// `MmapSlot`'s `Vec<u8>` field means the `[None; N]` repeat shorthand
/// isn't available), so a future change to `MAX_MMAPS` without updating
/// those literals fails the BUILD, not silently truncating a process's
/// mmap table.
const _ASSERT_MAX_MMAPS_IS_4: () = assert!(MAX_MMAPS == 4);

/// MILESTONE 64: one process's own mmap region -- `content` is a REAL
/// snapshot of the backing fd's `OpenFile::buffer` at the moment
/// `mmap()` was called (never re-read from the fd afterward, and never
/// written back to it either -- see this struct's own "disclosed scope
/// cuts" in the Milestone 64 module-level writeup for why that's an
/// honest simplification given this slice's read-only, no-COW scope, not
/// a hidden one), always `<= PAGE_SIZE` bytes since `fs::MAX_FILE_BYTES
/// == PAGE_SIZE` (checked, not assumed, by `mmap_file()` before ever
/// creating a slot). `frame` is `None` until the FIRST real hardware page
/// fault demand-pages it (`try_demand_page_mmap()`) -- exactly
/// `heap_frames`'s own `Option<PhysFrame<_>>` sparse-until-touched
/// pattern, reused rather than reinvented.
/// MILESTONE 65: `writable` extends Milestone 64's own `MmapSlot` with
/// real, per-slot `PROT_WRITE` support -- a private (never written back
/// to the fd, never shared with another mapping -- see `mmap_file()`'s
/// own "why this is real `MAP_PRIVATE`, not `MAP_SHARED`" reasoning)
/// writable mapping, the first item of Milestone 64's own disclosed
/// "real future work" list. `false` (Milestone 64's original read-only
/// behavior) for every slot created through `mmap_file()`; `true` only
/// for a slot created through this milestone's new `mmap_file_writable()`
/// -- `try_demand_page_mmap()` is the ONE place that reads this field,
/// and only to decide the page-table `WRITABLE` bit and whether a
/// first-touch WRITE fault is refused or serviced (see that function's
/// own updated doc comment).
#[derive(Clone)]
struct MmapSlot {
    content: Vec<u8>,
    frame: Option<PhysFrame<Size4KiB>>,
    writable: bool,
}

/// MILESTONE 33: hand-assembled machine code copied into each process's
/// code page INSTEAD of usertest::USER_PROGRAM (which stays completely
/// untouched, so the original Milestone 27 `usertest` command keeps
/// working exactly as it always has). Calls sbrk (syscall 2) first, then
/// writes a per-process marker byte into the returned heap pointer,
/// THEN sets up rdi/rsi and calls the Milestone 31 write(ptr,len)
/// syscall exactly like usertest::USER_PROGRAM does -- regenerated to
/// match that convention rather than the older no-argument "print".
///
///   offset  bytes                              instruction
///    0      B8 02 00 00 00                     mov eax, 2      (syscall 2 = sbrk)
///    5      BF 10 00 00 00                     mov edi, 16      (request 16 bytes)
///   10      CD 80                              int 0x80         -- rax = heap ptr
///   12      C6 00 00                           mov byte [rax], 0   (imm8 at offset 14,
///                                                                    patched per-process
///                                                                    by create_process()
///                                                                    below -- see
///                                                                    HEAP_MARKER_PATCH_OFFSET)
///   15      48 BF 80 00 00 50 55 55 00 00      mov rdi, imm64   (rdi = USER_CODE_ADDR+MESSAGE_OFFSET,
///                                                                 identical bytes to
///                                                                 usertest::USER_PROGRAM's own --
///                                                                 same constants)
///   25      BE 40 00 00 00                     mov esi, 64      (esi = usertest::MESSAGE_LEN)
///   30      B8 00 00 00 00                     mov eax, 0       (syscall 0 = write(ptr,len))
///   35      CD 80                              int 0x80
///   37      B8 01 00 00 00                     mov eax, 1       (syscall 1 = exit)
///   42      CD 80                              int 0x80
///   44      EB FE                              jmp $            (safety net, same
///                                                                 reasoning as
///                                                                 USER_PROGRAM's own)
const PROCESS_PROGRAM: [u8; 46] = [
    0xB8, 0x02, 0x00, 0x00, 0x00, 0xBF, 0x10, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xC6, 0x00, 0x00, 0x48,
    0xBF, 0x80, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x40, 0x00, 0x00, 0x00, 0xB8, 0x00,
    0x00, 0x00, 0x00, 0xCD, 0x80, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xEB, 0xFE,
];
/// Index into PROCESS_PROGRAM of the `mov byte [rax], imm8` instruction's
/// immediate byte -- checked by hand against the byte table in
/// PROCESS_PROGRAM's own doc comment above, not assumed. create_process()
/// patches this to a distinct value per process (b'A' / b'B') after
/// copying the template in.
const HEAP_MARKER_PATCH_OFFSET: usize = 14;

/// MILESTONE 35: real per-process file descriptors on top of fs.rs's
/// already-real on-disk filesystem (Milestones 18/22/28/32). Design
/// decision, mirroring Milestone 33's own documented sbrk-vs-kernel-only
/// choice:
///
/// **NEW syscall numbers (3=open, 4=read, 5=fdwrite, 6=close), NOT a
/// generalized `write(fd, ptr, len)`.** The alternative -- folding fd
/// support into syscall 0 by adding an `fd` argument (`fd=1` meaning
/// "serial console" for backward compatibility) -- was rejected because
/// it would change syscall 0's calling convention for every EXISTING
/// hand-assembled program already baked into this kernel binary
/// (usertest::USER_PROGRAM, process.rs's own PROCESS_PROGRAM, and
/// loader.rs's build_test_program_image()) -- each would need its
/// `mov rdi/rsi, ...` sequence regenerated with a new leading `fd`
/// argument, for a benefit (one fewer syscall number) that doesn't
/// outweigh touching three already-verified programs. New, additive
/// syscall numbers is also this project's own established pattern --
/// Milestone 33 added sbrk as syscall 2 rather than overloading an
/// existing one -- and keeps this milestone's diff isolated to NEW code
/// paths, which matters concretely right now: Milestone 36 (a real ELF
/// loader, developed in parallel against the same Milestone 34 baseline)
/// also touches usertest.rs/process.rs, and a hand-merge is far safer
/// against additive new match arms than against a changed argument
/// convention on the one syscall every existing program already calls.
///
/// **Buffered-at-open-time, not real streaming I/O.** open() reads the
/// WHOLE file into a kernel-owned Vec<u8> up front via fs::read_file()
/// (or starts an empty Vec if the path doesn't exist yet -- see
/// open_file()'s own doc comment for why that's the honest choice given
/// this syscall ABI has no separate O_CREAT flag). read()/fdwrite() then
/// only ever touch that in-memory buffer, never re-touching the disk
/// until close(). This is a real, disclosed limitation, not hidden: it's
/// only reasonable because fs.rs already caps every file at
/// fs::MAX_FILE_BYTES (4096 bytes, 8 sectors) -- the entire file always
/// fits in one small heap allocation, so "buffer the whole thing" costs
/// nothing meaningful. A real streaming implementation (partial reads
/// from disk sector-by-sector, incremental writes) is a substantially
/// bigger feature (needs a real per-fd on-disk cursor concept fs.rs
/// doesn't have today) and explicitly out of scope here.
///
/// **Writes persist to disk on close(), and ONLY on close(), and ONLY if
/// the fd was actually written to.** fdwrite() only ever mutates the
/// in-kernel buffer; close_fd() below calls fs::write_file(path, &buffer)
/// exactly when `dirty` is true. A file opened read-only and never
/// written is never rewritten (avoids an unnecessary disk write, and
/// avoids ever calling fs::write_file with a path that was only ever
/// read). There is no separate `fsync`/flush syscall -- data written via
/// fdwrite() and not yet close()'d is genuinely NOT on disk yet if
/// something (a crash, a later `runfile`) interrupts before close() runs.
/// Disclosed, not hidden.
/// MILESTONE 37: Clone derived so fork() can give a child process a
/// real, independent DEEP COPY of each of the parent's open files (own
/// path/buffer/pos/dirty, not a shared reference) -- see fork()'s own
/// doc comment for why a deep copy, not POSIX's real shared-fd-table
/// semantics, is this milestone's honest, disclosed simplification.
#[derive(Clone)]
struct OpenFile {
    /// Root-relative path this fd was opened against (fs.rs's own path
    /// format, exactly what was passed to open()) -- kept so close() can
    /// call fs::write_file() against the SAME path, without the caller
    /// having to pass it a second time.
    path: String,
    /// The file's entire contents, read once at open() time (or empty,
    /// for a not-yet-existing path) and mutated in place by fdwrite() --
    /// see this struct's own doc comment for why buffering the whole
    /// file is an honest choice here, not a shortcut hidden from the
    /// milestone report.
    buffer: Vec<u8>,
    /// Byte offset into `buffer` that the NEXT read() or fdwrite() call
    /// starts at -- shared between the two (Unix-style: one file
    /// position per open fd, not separate read/write cursors), and
    /// advanced by however many bytes that call actually transferred.
    pos: usize,
    /// Set by fdwrite() the first time it actually changes `buffer`;
    /// close() only calls fs::write_file() when this is true, so a
    /// read-only open never triggers a redundant disk write.
    dirty: bool,
}

/// MILESTONE 35: real, bounded per-process fd table size. Fixed at 4,
/// matching this project's existing "small fixed bound with an honest
/// disclosed limit" style (heap_frames' own HEAP_PAGE_COUNT is the
/// direct precedent -- 4 at the time this comment was written, 64 as of
/// Milestone 57) rather than a dynamically-growing Vec<Option<..>>
/// of open files -- 4 is comfortably more than this milestone's own test
/// program ever has open at once (at most 2: one read-only fd against an
/// existing file, one write fd against a new one, opened and closed in
/// sequence rather than concurrently), while still leaving headroom for
/// a real program to have, say, an input and output file open
/// simultaneously. A full table returns u64::MAX from open() (the same
/// failure sentinel as "path not found" for a to-be-fair-can't-happen
/// case here, given no file ever fails to "exist" -- see open_file()'s
/// own comment) rather than growing without bound.
pub(crate) const MAX_OPEN_FILES: usize = 4;

/// MILESTONE 37: fork()'s own per-element fd-table clone (see its own
/// comment for why it can't just call `.clone()` on the whole array)
/// hardcodes 4 element accesses matching MAX_OPEN_FILES's CURRENT value
/// -- this compile-time assertion fails loudly if that ever drifts,
/// rather than silently truncating a process's fd table on fork().
const _ASSERT_MAX_OPEN_FILES_IS_4: () = assert!(MAX_OPEN_FILES == 4);

/// MILESTONE 40: an fd slot now names one of two genuinely different
/// underlying resources, not just a file -- `File` is exactly the
/// pre-Milestone-40 `OpenFile` case (unchanged), `PipeRead`/`PipeWrite`
/// name an END of a pipe by its index into the global PIPE_TABLE below,
/// NOT owned data. That index-not-data distinction is the whole design:
/// deriving Clone on this enum still gives fork() its existing "one
/// `.clone()` per fd slot, one `let` statement per slot" pattern
/// unchanged (see fork()'s own comment on why combining multiple
/// fallible clones into one expression broke the build once already),
/// but for a pipe end, cloning an index is exactly the reference-sharing
/// a real fork() needs -- the parent and child end up with two DIFFERENT
/// Process structs whose fd slot holds the SAME index into ONE shared
/// PIPE_TABLE entry, not two independent copies of a buffer (which
/// would silently break the pipe's entire reason to exist). `File`
/// keeps its own pre-existing honest limitation: dup()/dup2() of a
/// File fd (below) deep-copies the same way fork() already does,
/// documented there, not pretended away here.
#[derive(Clone)]
enum FdEntry {
    File(OpenFile),
    PipeRead(usize),
    PipeWrite(usize),
}

/// MILESTONE 40: fixed ring-buffer capacity per pipe, matching this
/// project's small-fixed-bound style (MAX_OPEN_FILES/MAX_PROCESSES's own
/// precedent) rather than a dynamically-growing buffer.
const PIPE_CAPACITY: usize = 512;

/// MILESTONE 40: how many pipes can exist system-wide at once (NOT
/// per-process -- a pipe is a global resource two different processes'
/// fd tables can both reference, so it can't live inside either one's
/// own Process struct).
const MAX_PIPES: usize = 4;

/// MILESTONE 40: a real, fixed-size ring buffer plus real open-end
/// reference counts. The counts exist for exactly one reason: a pipe's
/// storage is only actually freed (PIPE_TABLE slot -> None) once EVERY
/// fd across EVERY process that ever referenced either end -- including
/// ends duplicated by fork() or dup()/dup2() -- has been close()'d.
/// Without real counting, closing the fd in ONE of two processes sharing
/// a forked pipe would either free it out from under the other (a
/// dangling reference) or never free it at all (a permanent leak of one
/// of only MAX_PIPES=4 global slots). Both `read_open`/`write_open`
/// start at 1 (the fd pipe() itself hands back) and are incremented by
/// fork()/dup()/dup2() and decremented by close_fd() -- see each site's
/// own comment.
struct Pipe {
    buffer: [u8; PIPE_CAPACITY],
    /// Index of the next byte to be read (wraps modulo PIPE_CAPACITY).
    read_pos: usize,
    /// Index of the next byte to be written (wraps modulo PIPE_CAPACITY).
    write_pos: usize,
    /// How many real, unread bytes are currently buffered -- distinct
    /// from `read_pos`/`write_pos` alone because those two can be equal
    /// at BOTH "completely empty" and "completely full" (a ring buffer
    /// needs a third piece of state to disambiguate the two).
    len: usize,
    read_open: u32,
    write_open: u32,
}

/// MILESTONE 40: the global pipe table -- see Pipe's own doc comment for
/// why this can't be per-process state. `spin::Mutex<[Option<T>; N]>` is
/// the exact same pattern PROCESS_TABLE already established for a
/// dynamic, bounded set of global kernel objects.
static PIPE_TABLE: Mutex<[Option<Pipe>; MAX_PIPES]> = Mutex::new([const { None }; MAX_PIPES]);

/// MILESTONE 40: allocates a real PIPE_TABLE slot (both ends starting at
/// `read_open`/`write_open` = 1, `len` = 0) and gives the calling
/// process two new fd-table entries -- `FdEntry::PipeRead`/`PipeWrite`
/// -- pointing at it. Returns `(read_fd, write_fd)`, or `None` if either
/// the process's own fd table can't fit both new entries (needs 2 free
/// slots, not just 1) or PIPE_TABLE itself is full (all MAX_PIPES
/// already live).
pub(crate) fn pipe_create(id: u8) -> Option<(u64, u64)> {
    let mut table = PIPE_TABLE.lock();
    let pipe_index = table.iter().position(|p| p.is_none())?;
    table[pipe_index] = Some(Pipe {
        buffer: [0u8; PIPE_CAPACITY],
        read_pos: 0,
        write_pos: 0,
        len: 0,
        read_open: 1,
        write_open: 1,
    });
    drop(table);
    let result = with_process_mut(id, |proc| {
        let mut free = proc.fds.iter().enumerate().filter(|(_, f)| f.is_none()).map(|(i, _)| i);
        let read_slot = free.next()?;
        let write_slot = free.next()?;
        proc.fds[read_slot] = Some(FdEntry::PipeRead(pipe_index));
        proc.fds[write_slot] = Some(FdEntry::PipeWrite(pipe_index));
        Some((read_slot as u64, write_slot as u64))
    })?;
    if result.is_none() {
        // Process's fd table couldn't fit both ends -- release the
        // PIPE_TABLE slot we just allocated rather than leaking it.
        PIPE_TABLE.lock()[pipe_index] = None;
    }
    result
}

/// MILESTONE 40: reads up to `len` bytes out of the pipe fd names.
/// Real, honest, DISCLOSED non-blocking semantics: this kernel has no
/// scheduler-level blocking primitive a syscall could suspend a process
/// on (Milestone 25's background scheduling is cooperative task
/// switching, not a wait-queue) -- so reading an empty pipe returns
/// `Some(vec![])` (0 bytes, not an error) immediately rather than really
/// blocking until a writer produces data, the same "explicit partial/
/// empty result, never silently wrong" discipline read_fd() already
/// established for end-of-file. Returns `None` only if `fd` doesn't name
/// a currently-open PipeRead entry for this process.
fn pipe_read(id: u8, fd: u64, len: usize) -> Option<Vec<u8>> {
    let pipe_index = with_process_mut(id, |proc| match proc.fds.get(fd as usize)? {
        Some(FdEntry::PipeRead(idx)) => Some(*idx),
        _ => None,
    })??;
    let mut table = PIPE_TABLE.lock();
    let pipe = table[pipe_index].as_mut()?;
    let n = len.min(pipe.len);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(pipe.buffer[(pipe.read_pos + i) % PIPE_CAPACITY]);
    }
    pipe.read_pos = (pipe.read_pos + n) % PIPE_CAPACITY;
    pipe.len -= n;
    Some(out)
}

/// MILESTONE 40: writes as many bytes of `data` as currently fit into
/// the pipe fd names, returning the ACTUAL count accepted -- a real,
/// honest partial write if the ring buffer doesn't have room for all of
/// it (same discipline as write_fd()'s own MAX_FILE_BYTES truncation),
/// never blocking for room to free up (see pipe_read()'s doc comment for
/// why -- no blocking primitive exists to wait on). Returns `None` only
/// if `fd` doesn't name a currently-open PipeWrite entry for this
/// process.
fn pipe_write(id: u8, fd: u64, data: &[u8]) -> Option<usize> {
    let pipe_index = with_process_mut(id, |proc| match proc.fds.get(fd as usize)? {
        Some(FdEntry::PipeWrite(idx)) => Some(*idx),
        _ => None,
    })??;
    let mut table = PIPE_TABLE.lock();
    let pipe = table[pipe_index].as_mut()?;
    let room = PIPE_CAPACITY - pipe.len;
    let n = data.len().min(room);
    for (i, byte) in data[..n].iter().enumerate() {
        pipe.buffer[(pipe.write_pos + i) % PIPE_CAPACITY] = *byte;
    }
    pipe.write_pos = (pipe.write_pos + n) % PIPE_CAPACITY;
    pipe.len += n;
    Some(n)
}

pub struct Process {
    label: &'static str,
    pml4_frame: PhysFrame<Size4KiB>,
    code_frame: PhysFrame<Size4KiB>,
    /// The TOP stack page, at `usertest::USER_STACK_ADDR`. Unchanged in
    /// meaning since Milestone 30 -- `rsp` starts just above it and the
    /// argv/envp writer works relative to it, so every reader of this
    /// field keeps working as-is.
    stack_frame: PhysFrame<Size4KiB>,
    /// MILESTONE 100: `usertest::USER_STACK_EXTRA_PAGES` frames backing
    /// the pages immediately BELOW `stack_frame`, `extra_stack_frames[i]`
    /// backing `[USER_STACK_ADDR - (i+1)*4096, USER_STACK_ADDR - i*4096)`.
    /// Always fully populated (mapped eagerly at process creation, unlike
    /// the lazy `heap_frames`/`extra_frames`) -- a stack page that is not
    /// there when `rsp` reaches it is a fault, not a demand-page
    /// opportunity. Copied byte-for-byte on `fork`, freed on teardown.
    extra_stack_frames: Vec<PhysFrame<Size4KiB>>,
    /// MILESTONE 33/57: this process's own private heap frames, ALWAYS
    /// exactly `HEAP_PAGE_COUNT` entries long, indexed by heap page number
    /// (`heap_frames[i]` backs `HEAP_START + i*PAGE_SIZE`). As of
    /// Milestone 57 this is genuinely sparse -- `None` means "reserved
    /// virtual range, no physical frame backing it yet" (the common case
    /// for most of a fresh process's life), `Some(frame)` means a real
    /// hardware page fault already demand-paged that specific page (see
    /// `try_demand_page_heap()`). Before Milestone 57 this was a plain
    /// `Vec<PhysFrame<_>>`, always fully populated at process-creation
    /// time -- callers that only cared about "how many heap frames does
    /// this process actually have mapped right now" (e.g. Milestone 54's
    /// own frame-reclaim self-test) now count `Some` entries rather than
    /// reading `.len()` directly, since `.len()` is now always
    /// `HEAP_PAGE_COUNT` regardless of how many pages are really backed.
    heap_frames: Vec<Option<PhysFrame<Size4KiB>>>,
    /// MILESTONE 33: bump offset (bytes, from HEAP_START) of this
    /// process's next `sbrk` allocation -- persists across repeated
    /// `runproc` calls for the SAME process (never reset), so re-running
    /// a process twice hands out a fresh, later region on the second
    /// call rather than silently reusing the first.
    heap_used: u64,
    /// MILESTONE 35: this process'''s own real fd table -- `None` is an
    /// unused slot, `Some(OpenFile)` an fd the process currently has open
    /// (its index in this array IS its fd number, per open_file()'''s own
    /// comment). Never reset between runs of the SAME process, matching
    /// heap_used'''s own persists-across-runs precedent -- a fd left open
    /// by a buggy program stays open (and stays counted against
    /// MAX_OPEN_FILES) until an explicit close(), exactly like a real OS.
    fds: [Option<FdEntry>; MAX_OPEN_FILES],
    /// MILESTONE 36: extra physical frames backing a real ELF-loaded
    /// process'''s PT_LOAD segment pages BEYOND the one page already
    /// tracked as `code_frame` above (which, for an ELF-loaded process,
    /// is specifically whichever mapped page backs USER_CODE_ADDR --
    /// see create_process_from_elf()). Always empty for the flat-binary
    /// path (create_process_from_image/create_process): a flat image is
    /// still exactly one code page, so it never needs this.
    ///
    /// MILESTONE 69: now carries each page's real `VirtAddr` alongside
    /// its frame (was `Vec<PhysFrame<Size4KiB>>` alone) -- a real, honest
    /// bug found and fixed by this milestone's own end-to-end
    /// verification, not a speculative generalization: `fork()` only ever
    /// copied `code_frame` (page 0) and `stack_frame`, silently leaving a
    /// forked child with NO mapping at all for any of a multi-page
    /// ELF-loaded process's OTHER code pages -- invisible for every prior
    /// milestone's own fork() self-test (every process that ever called
    /// fork() before this one fit in a single 4 KiB code page), but a
    /// real, hardware-recorded `SIGSEGV`/`INSTRUCTION_FETCH` page fault
    /// the moment cc.elf (now 3 real code pages, Milestone 67/68/69
    /// combined) forked itself to exec() a freshly-compiled program and
    /// the child's resume `rip` happened to land on page 1 or 2. Fixed by
    /// giving fork() (see `fork_build_child()`) the real vaddr it needs
    /// to remap each extra page at the SAME address in the child's own
    /// private PML4, not just its bytes -- previously nothing read this
    /// field back out at all (a real, disclosed "kept for future
    /// reclaim-path bookkeeping" gap since Milestone 36); reclaim
    /// (`reclaim_process_frames()`) and this milestone's fork() fix are
    /// now the two real readers.
    extra_frames: Vec<(VirtAddr, PhysFrame<Size4KiB>)>,
    /// MILESTONE 37: `Some(parent_pid)` if this process was created by
    /// `fork()`; `None` for every process created any other way (all
    /// four pre-existing hardcoded/loaded-file paths). Checked by
    /// `wait_for_child()` so only a process's REAL parent can reap it.
    parent_pid: Option<u8>,
    /// MILESTONE 37: `Some((resume_rip, resume_rsp))` captured from the
    /// PARENT's own hardware-recorded `SyscallRegs` at the exact moment
    /// `fork()` created this process, if it has never actually been run
    /// yet -- `run_forked_child()` consumes this via `.take()` the one
    /// time it enters ring 3 for this child. `None` for every
    /// non-forked process (nothing ever resumes them via this
    /// mechanism -- they always start fresh at `USER_CODE_ADDR` through
    /// the ordinary `run()`/`run_loaded_process()`/`load_and_run_elf()`
    /// paths).
    pending_resume: Option<(u64, u64)>,
    /// MILESTONE 42: this process's process group id (PGID) -- real Unix
    /// job-control semantics: a freshly-created (non-forked) process is
    /// the founder of its own group (its init_*_process() caller sets
    /// this to its own well-known id right after construction, below);
    /// a `fork()`-created child INHERITS its parent's pgid (see fork()'s
    /// own handling) until an explicit `setpgid()` call moves it.
    /// `0` here (from either shared constructor, create_process_from_image/
    /// create_process_from_elf) is a real, never-otherwise-valid
    /// "not yet assigned by the real owning caller" sentinel -- PIDs in
    /// this kernel start at 1, so a live process reporting pgid 0 would
    /// itself be a bug, not a legitimate group.
    pgid: u8,
    /// MILESTONE 44: the real virtual address ring-3 execution starts at
    /// for THIS process -- `create_process_from_image()` always sets this
    /// to `usertest::USER_CODE_ADDR` (every hand-assembled/flat-binary
    /// program in this kernel's history is built to start there);
    /// `create_process_from_elf()` sets this to the real, parsed `e_entry`
    /// from the ELF itself, which no longer has to equal `USER_CODE_ADDR`
    /// (Milestone 36's own deliberately-deferred restriction -- see that
    /// milestone's module doc comment, now generalized). `run()`/
    /// `load_and_run_elf()` read this back out and pass it to
    /// `usertest::enter_ring3_now(entry)` instead of that function
    /// hardcoding `USER_CODE_ADDR` itself.
    entry: u64,
    /// MILESTONE 59: this process's own real, persistent errno --
    /// `0` means "no error recorded yet" (this kernel's own honest
    /// sentinel, never a value any real failure path below sets: every
    /// `errno.rs` constant is >= 1, matching real errno.h, where 0 is
    /// never a defined error code either). Set by `set_errno()` from
    /// usertest.rs's syscall_dispatch on the SAME failure paths that
    /// already return a bare `u64::MAX`/`0`/`1` sentinel today -- this
    /// field is the thing that finally lets a caller tell WHICH of
    /// several real failure reasons a bare sentinel used to collapse
    /// into one indistinguishable value. Same real POSIX semantics as a
    /// real libc's `errno`: NEVER implicitly cleared on a later
    /// successful call (a caller that doesn't check its return value
    /// first and reads errno anyway can see a stale value from an
    /// earlier, unrelated failure -- exactly as real, exactly as
    /// documented, not a bug this kernel needs to paper over). Persists
    /// across repeated `runproc` calls for the SAME process, same
    /// "never reset between runs" precedent `heap_used`/`fds` already
    /// established.
    errno: u64,
    /// MILESTONE 60: real signal delivery -- this process's own handler
    /// table, indexed by real POSIX signal number (`signal::NSIG`
    /// entries, index 0 unused/never valid). `0` (this array's own
    /// default) is `SIG_DFL` -- "no handler registered" -- the ONLY
    /// disposition this milestone actually implements: there is no
    /// default-action table yet (real POSIX default-terminates most
    /// signals; this kernel does not, see `raise_signal()`'s own doc
    /// comment for the honest reason and scope cut), so a signal raised
    /// against a slot still holding `0` here is a real, documented no-op,
    /// never a silently-pretended delivery. A nonzero value is a real
    /// user-space virtual address (checked to be nonzero at registration
    /// time only -- like every other raw user pointer this kernel already
    /// hands to a syscall, e.g. `write`'s `ptr`, it is NOT validated to
    /// actually be mapped/executable code until control genuinely
    /// transfers there, same disclosed "no general copy-from-user
    /// fault-recovery path" limitation `MAX_WRITE_LEN`'s own doc comment
    /// already names elsewhere in this codebase).
    signal_handlers: [u64; signal::NSIG],
    /// MILESTONE 60: a signal raised against this process (`raise_signal()`)
    /// but not yet actually delivered into its handler. Real POSIX
    /// semantics for a NON-realtime signal: not queued -- a second raise
    /// of the same (or a different) number before the first is delivered
    /// simply overwrites this slot (the earlier one is lost), exactly
    /// like a real kernel's single-bit-per-signal pending set collapses
    /// repeat raises of the same signal; this kernel goes one step
    /// further and collapses ACROSS different signal numbers too (a real,
    /// disclosed scope cut for this first slice -- a full per-signal
    /// pending bitmask is real future work, not silently pretended here).
    /// Consumed (cleared) by `take_deliverable_signal()` the moment
    /// delivery actually happens, not when the handler finishes running.
    pending_signal: Option<u8>,
    /// MILESTONE 60: `Some(saved context)` exactly while this process is
    /// genuinely executing inside a real, kernel-dispatched signal
    /// handler -- set by `stash_signal_context()` the moment
    /// `take_deliverable_signal()` redirects a real hardware-interrupted
    /// context into a handler, cleared by `take_saved_signal_context()`
    /// (`SIGRETURN`, syscall 20) once that handler unwinds back out. A
    /// real, disclosed simplification of POSIX's finer-grained per-signal
    /// `sa_mask`/pending-set semantics: while this is `Some`,
    /// `take_deliverable_signal()` refuses to deliver ANY further signal
    /// to this process (not just the one currently being handled) --
    /// real POSIX only blocks the signal(s) named in the handler's own
    /// `sa_mask` (which defaults to just the signal itself, not all of
    /// them) by default. Blocking everything is strictly safer (never
    /// re-enters a handler this kernel has never made reentrant-safe) at
    /// the real, honest cost of a signal that arrives DURING another's
    /// handler being dropped rather than queued -- same "small, real,
    /// honest" scope-cut discipline as `pending_signal`'s own doc comment
    /// just above, not hidden.
    in_signal_handler: Option<SavedSignalContext>,
    /// MILESTONE 64: this process's own real, bounded mmap table -- `None`
    /// is an unused slot, `Some(MmapSlot)` a live read-only file-backed
    /// mapping (its index in this array determines its virtual address:
    /// slot `i` lives at `MMAP_START + i*PAGE_SIZE`, exactly the same
    /// index-determines-address convention `heap_frames` already
    /// established for the heap). Never reset between runs of the SAME
    /// process, matching `fds`/`heap_used`'s own persists-across-runs
    /// precedent. **Disclosed, not hidden: a forked child does NOT
    /// inherit the parent's mmap regions** -- `fork_build_child()` goes
    /// through the same `create_process_from_image()` constructor every
    /// fresh process does and never copies this field across afterward,
    /// the exact same disclosed gap `extra_frames`' own doc comment
    /// already carries for ELF segment frames (fork() only ever forks a
    /// flat-image process in this design, so a REAL forked-parent-with-
    /// live-mmaps case cannot occur yet either way).
    mmaps: [Option<MmapSlot>; MAX_MMAPS],
}

/// MILESTONE 60: a real, complete snapshot of a process's hardware-
/// interrupted register/PC/flags/stack-pointer state at the exact moment
/// a signal handler was dispatched -- `usertest.rs`'s `syscall_dispatch`
/// builds one (copied field-for-field from the SAME `SyscallRegs` the
/// interrupted `int 0x80` itself produced) immediately before overwriting
/// that live struct's `rip`/`rdi`/`rsp` to redirect execution into the
/// handler, and `take_saved_signal_context()` (backing `SIGRETURN`,
/// syscall 20) hands it right back so `usertest.rs` can restore every
/// field verbatim, resuming the interrupted context exactly as if the
/// signal had never arrived (SysV argument/return registers included --
/// this is a full GPR snapshot, not just PC+flags).
///
/// Real, disclosed, deliberate scope cut for this first slice: this is a
/// KERNEL-side stash (one slot per process, in `Process::
/// in_signal_handler` itself), not a real POSIX `ucontext_t` written onto
/// the process's OWN user stack the way a real kernel's `rt_sigreturn`
/// reads its restore context back from user memory. That real design
/// would need a safe copy-to/from-user path with fault recovery this
/// kernel has never had (see `usertest.rs`'s own `MAX_WRITE_LEN` doc
/// comment, which already discloses this exact gap for ordinary syscall
/// pointers) -- a kernel-side stash sidesteps that entirely, at the real,
/// honest cost that a signal can only ever unwind through EXACTLY the
/// mechanism that dispatched it (no cross-process/cross-boot
/// `sigreturn` forgery is even possible, which is its own small real
/// upside), and only one handler can ever be "in flight" per process at
/// a time (see `in_signal_handler`'s own doc comment on why that is
/// enforced anyway, for an unrelated reason).
#[derive(Clone, Copy)]
pub(crate) struct SavedSignalContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub rip: u64,
    pub rflags: u64,
    pub rsp: u64,
}

static PROCESS_A: Mutex<Option<Process>> = Mutex::new(None);
static PROCESS_B: Mutex<Option<Process>> = Mutex::new(None);

/// MILESTONE 35: a THIRD hardcoded, boot-time-created process slot,
/// exactly like PROCESS_A/PROCESS_B in every structural way (built once
/// at boot by init_fdtest_process(), run via the SAME `run(id)`
/// CR3-switch+enter_ring3_now() mechanism as A/B) but running
/// loader::FDTEST_PROGRAM (the Milestone 35 open/read/fdwrite/close
/// syscall test) instead of PROCESS_PROGRAM.
///
/// **Why this exists alongside `runfile fdtestprog`, not instead of
/// it**: this milestone's fd syscalls were ALSO verified end-to-end via
/// `runfile` (see run_loaded_process()'s doc comment) -- the real
/// on-disk-filesystem open/read/fdwrite/close syscall trace came back
/// byte-for-byte correct there too (confirmed in the serial log: OPEN
/// 'fdtest'->fd0, READ 22/22 bytes matching exactly, CLOSE, OPEN
/// 'fdout'->fd0, FDWRITE 69/69 bytes accepted, CLOSE persisted 69 bytes
/// to disk). But a SEPARATE, pre-existing, unrelated Milestone 34 bug
/// (real heap corruption in the `runfile`/LOADED_PROCESS path --
/// reproduces even with completely unmodified Milestone 34 code, e.g.
/// plain `seedtestprog`+`runfile testprog`, no Milestone 35 syscalls
/// involved at all; confirmed NOT present in the `runproc` path even
/// under repeated/extended testing; symbolized to a null-pointer read
/// inside linked_list_allocator::Heap::allocate_first_fit, timing-
/// dependent on background task scheduling) crashes the kernel shortly
/// AFTER that syscall trace completes successfully, before the shell can
/// accept the follow-up `read fdout` keystrokes needed to close the loop
/// with a live, screendumped confirmation. This process slot -- reusing
/// the exact mechanism `runproc` already uses safely -- exists so this
/// milestone's own verification isn't blocked by a bug in someone else's
/// (Milestone 34's) code path. That bug is disclosed, not fixed here
/// (out of scope for a fd-syscall milestone to debug a heap allocator);
/// `runfile fdtestprog` is left exactly as capable (and exactly as
/// broken, for now) as `runfile testprog` already was.
static FDTEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);

/// ACTIVE_PROCESS / with_process_mut id for FDTEST_PROCESS -- distinct
/// from PROCESS_A (1), PROCESS_B (2), and LOADED_PROCESS_ID (3, a
/// different process entirely, reachable only via `runfile`).
pub(crate) const FDTEST_PROCESS_ID: u8 = 4;

/// MILESTONE 37: a FIFTH hardcoded, boot-time-created process slot,
/// structurally identical to PROCESS_A/PROCESS_B/FDTEST_PROCESS (built
/// once at boot, run via the same `run(id)` mechanism), running
/// FORK_TEST_PROGRAM -- this milestone's own fork/exec/wait test
/// program (see that constant's own doc comment). A hardcoded slot,
/// not a PROCESS_TABLE entry, for the same reason PROCESS_A/PROCESS_B/
/// FDTEST_PROCESS are: it is the ONE process that must exist before any
/// `fork()` call could ever happen (fork() is what actually populates
/// PROCESS_TABLE), so it can't itself be a member of that table.
static FORK_TEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);

/// ACTIVE_PROCESS / with_process_mut id for FORK_TEST_PROCESS -- 5,
/// keeping the non-overlapping id space PROCESS_A(1)/PROCESS_B(2)/
/// LOADED_PROCESS_ID(3)/FDTEST_PROCESS_ID(4) already established going,
/// distinct from PID_TABLE_BASE(10)'s own dynamic range below.
pub(crate) const FORK_TEST_PROCESS_ID: u8 = 5;

/// MILESTONE 41: a deliberately-faulting ring-3 test program -- writes
/// a short distinguishing message, then dereferences a definitely-
/// unmapped address (0x0000_1234_5678_0000, nowhere near
/// USER_CODE_ADDR/USER_STACK_ADDR/HEAP_START's own 0x5555... range),
/// triggering a real page fault at real CPL=3. The `mov eax,1`/`int
/// 0x80`/`jmp $` tail is UNREACHABLE -- pure safety net matching every
/// other hand-assembled program's own convention, in case the fault
/// mechanism itself turns out to be broken and execution somehow
/// continues past the fault.
///
///   offset  bytes                                    instruction
///    0      B8 00 00 00 00                           mov eax, 0        (write syscall)
///    5      48 BF 2C 00 00 50 55 55 00 00             mov rdi, imm64    (USER_CODE_ADDR+44, the message below)
///   15      BE 0D 00 00 00                            mov esi, 13       (message length)
///   20      CD 80                                     int 0x80
///   22      48 BB 00 00 78 56 34 12 00 00             mov rbx, imm64    (0x0000_1234_5678_0000, unmapped)
///   32      48 8B 03                                  mov rax, [rbx]    -- FAULTS HERE
///   35      B8 01 00 00 00                            mov eax, 1        (unreachable)
///   40      CD 80                                     int 0x80          (unreachable)
///   42      EB FE                                     jmp $             (unreachable)
///   44      "SIGSEGV test\n"                          the message itself (13 bytes)
pub(crate) const SIGSEGV_TEST_PROGRAM: [u8; 57] = [
    0xB8, 0x00, 0x00, 0x00, 0x00, 0x48, 0xBF, 0x2C, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE,
    0x0D, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0xBB, 0x00, 0x00, 0x78, 0x56, 0x34, 0x12, 0x00, 0x00,
    0x48, 0x8B, 0x03, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xEB, 0xFE, 0x53, 0x49, 0x47, 0x53,
    0x45, 0x47, 0x56, 0x20, 0x74, 0x65, 0x73, 0x74, 0x0A,
];
/// The real message embedded in SIGSEGV_TEST_PROGRAM at byte offset 44
/// -- checked by self_test_signals() before trusting the program at
/// runtime, same discipline as FORK_TEST_PROGRAM's own layout self-test.
const SIGSEGV_MSG: &[u8] = b"SIGSEGV test\n";
const SIGSEGV_MSG_OFFSET: usize = 44;

/// MILESTONE 41: a SIXTH hardcoded, boot-time-created process slot --
/// runs SIGSEGV_TEST_PROGRAM. See that constant's own doc comment.
static SIGSEGV_TEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const SIGSEGV_TEST_PROCESS_ID: u8 = 6;

/// MILESTONE 41: fork()s exactly one child then immediately kill()s it
/// (syscall 13, rdi=child pid) INSTEAD of wait()ing it -- real proof
/// SIGKILL bypasses the normal "run to completion, then reap" contract.
/// Forks a SECOND time afterward: if the first child's PROCESS_TABLE
/// slot was genuinely freed (not just flagged), this second fork()
/// succeeds and reuses it -- self_test_signals() checks PROCESS_TABLE's
/// real occupancy afterward as the actual proof, not just trusting this
/// program ran without crashing.
///
///   offset  bytes                        instruction
///    0      B8 07 00 00 00               mov eax, 7   (fork)
///    5      CD 80                        int 0x80     -- rax = child pid
///    7      89 C7                        mov edi, eax (child pid -> kill's arg)
///    9      B8 0D 00 00 00               mov eax, 13  (kill)
///   14      CD 80                        int 0x80
///   16      B8 07 00 00 00               mov eax, 7   (fork again)
///   21      CD 80                        int 0x80
///   23      B8 01 00 00 00               mov eax, 1   (exit)
///   28      CD 80                        int 0x80
///   30      EB FE                        jmp $        (unreachable)
pub(crate) const SIGKILL_TEST_PROGRAM: [u8; 32] = [
    0xB8, 0x07, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x89, 0xC7, 0xB8, 0x0D, 0x00, 0x00, 0x00, 0xCD, 0x80,
    0xB8, 0x07, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xEB, 0xFE,
];

/// MILESTONE 41: a SEVENTH hardcoded, boot-time-created process slot --
/// runs SIGKILL_TEST_PROGRAM. See that constant's own doc comment.
static SIGKILL_TEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const SIGKILL_TEST_PROCESS_ID: u8 = 7;

/// MILESTONE 43: a deliberately minimal program whose ENTIRE body is a
/// real, deterministic `exit(42)` -- used purely as a fork() SOURCE
/// (see self_test_wait_status()) so the forked CHILD's own code page,
/// resumed from offset 0, genuinely executes this exact exit call with
/// no ambiguity about what value rdi holds -- unlike forking from an
/// already-complex program (FORK_TEST_PROGRAM/SIGKILL_TEST_PROGRAM)
/// whose subsequent bytes weren't written with a deterministic exit
/// code in mind.
///
///   offset  bytes                        instruction
///    0      BF 2A 00 00 00               mov edi, 42  (real exit code)
///    5      B8 01 00 00 00               mov eax, 1   (exit)
///   10      CD 80                        int 0x80
pub(crate) const WAITSTATUS_TEST_PROGRAM: [u8; 12] = [
    0xBF, 0x2A, 0x00, 0x00, 0x00, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xCD, 0x80,
];

/// MILESTONE 43: an EIGHTH hardcoded, boot-time-created process slot --
/// runs WAITSTATUS_TEST_PROGRAM. See that constant's own doc comment.
static WAITSTATUS_TEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const WAITSTATUS_TEST_PROCESS_ID: u8 = 8;

/// MILESTONE 45: a deliberately minimal program whose entire job is to
/// call the real, rebuilt `exec()` syscall exactly once -- `mov rdi,
/// <ptr to "altentry">; mov esi, 8; mov eax, 9; int 0x80`. On SUCCESS
/// this never returns here at all: control jumps straight into
/// `testelf_altentry.elf`'s own real, parsed `e_entry`
/// (`0x0000_5555_5000_3000`, Milestone 44's staged non-USER_CODE_ADDR
/// test payload -- see loader.rs's ALTENTRY_TEST_ELF_BYTES and
/// seed_test_elf_altentry(), wired into a real self-test for the first
/// time by THIS milestone), which write()s its own distinguishing
/// message and exit()s for real. On FAILURE (e.g. `seedaltentry` was
/// never run, so "altentry" doesn't exist on disk) execution falls
/// through to write() a distinguishing FALLBACK message instead, then
/// exit()s -- the same "degrade cleanly and honestly either way rather
/// than crashing" contract FORK_TEST_PROGRAM's own child path already
/// established.
///
/// Assembled + verified via a standalone Python script using the
/// keystone-engine assembler, with a capstone disassembly round-trip
/// confirming the intended control flow byte for byte -- same
/// discipline as every other hand-assembled program in this file.
///
/// Layout (verified against the actual byte array below by
/// self_test_exec_test_program()):
///   offset   0..53   the syscall sequence itself (53 bytes of real
///                     instructions)
///   offset  53..61   "altentry" (8 real bytes, the exec() path)
///   offset  61..115  EXEC_FALLBACK_MSG (54 bytes)
pub(crate) const EXEC_TEST_PROGRAM: [u8; 115] = [
    0x48, 0xBF, 0x35, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x08,
    0x00, 0x00, 0x00, 0xB8, 0x09, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0xBF,
    0x3D, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x36, 0x00, 0x00,
    0x00, 0xB8, 0x00, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xB8, 0x01, 0x00, 0x00,
    0x00, 0xCD, 0x80, 0xEB, 0xFE, 0x61, 0x6C, 0x74, 0x65, 0x6E, 0x74, 0x72,
    0x79, 0x6D, 0x69, 0x6C, 0x65, 0x73, 0x74, 0x6F, 0x6E, 0x65, 0x20, 0x34,
    0x35, 0x3A, 0x20, 0x45, 0x58, 0x45, 0x43, 0x5F, 0x54, 0x45, 0x53, 0x54,
    0x20, 0x66, 0x61, 0x6C, 0x6C, 0x62, 0x61, 0x63, 0x6B, 0x20, 0x2D, 0x2D,
    0x20, 0x72, 0x65, 0x61, 0x6C, 0x20, 0x65, 0x78, 0x65, 0x63, 0x28, 0x29,
    0x20, 0x66, 0x61, 0x69, 0x6C, 0x65, 0x64,
];
const EXEC_TEST_PATH_OFFSET: usize = 53;
const EXEC_TEST_FALLBACK_OFFSET: usize = 61;
/// The real path EXEC_TEST_PROGRAM exec()s into -- see
/// loader::seed_test_elf_altentry(), which must have already written
/// ALTENTRY_TEST_ELF_BYTES to this exact path before EXEC_TEST_PROCESS
/// ever runs (kernel_main calls it directly, non-interactively, before
/// self_test_real_exec() -- no shell command needed).
pub(crate) const EXEC_TEST_PATH: &str = "altentry";
/// The real message EXEC_TEST_PROGRAM prints if exec() fails -- checked
/// by self_test_exec_test_program() before trusting the program at
/// runtime, same discipline as FORK_TEST_PROGRAM's own layout self-test.
pub(crate) const EXEC_TEST_FALLBACK_MSG: &str = "milestone 45: EXEC_TEST fallback -- real exec() failed";

/// MILESTONE 58: the real path `tools/argvlauncher_src/main.rs`'s
/// ARGVLAUNCHER_ELF_BYTES exec()s into (via the new EXECARGV syscall) --
/// see `loader::seed_argvtarget_elf()`, which must have already written
/// `loader`'s ARGVTARGET_ELF_BYTES to this exact path before
/// `loader::self_test_execargv()` runs the launcher.
pub(crate) const EXECARGV_TARGET_PATH: &str = "argvtarget";

/// MILESTONE 45: a NINTH hardcoded, boot-time-created process slot --
/// runs EXEC_TEST_PROGRAM. See that constant's own doc comment.
static EXEC_TEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const EXEC_TEST_PROCESS_ID: u8 = 9;

/// MILESTONE 53: a deliberately minimal program whose ENTIRE body
/// dereferences a definitely-unmapped address -- same unmapped target
/// (0x0000_1234_5678_0000) M41's own SIGSEGV_TEST_PROGRAM already uses,
/// deliberately reused rather than picked fresh: it's already proven
/// unmapped by that milestone's own self-test. Unlike SIGSEGV_TEST_
/// PROGRAM, this one skips the write-a-message-first preamble entirely
/// (WAITSTATUS_TEST_PROGRAM's own "deterministic, nothing-else-going-on"
/// reasoning applies here too) -- this program exists purely as a
/// fork() SOURCE for self_test_fault_status() below, so the forked
/// CHILD resumed from offset 0 faults on literally its first
/// instruction, with no ambiguity about what ran before it.
///
///   offset  bytes                                    instruction
///    0      48 BB 00 00 78 56 34 12 00 00             mov rbx, imm64   (0x0000_1234_5678_0000, unmapped)
///   10      48 8B 03                                  mov rax, [rbx]   -- FAULTS HERE
///   13      EB FE                                     jmp $            (unreachable safety net)
pub(crate) const FAULT_TEST_PROGRAM: [u8; 15] = [
    0x48, 0xBB, 0x00, 0x00, 0x78, 0x56, 0x34, 0x12, 0x00, 0x00, 0x48, 0x8B, 0x03, 0xEB, 0xFE,
];

/// MILESTONE 53: a TENTH hardcoded, boot-time-created process slot --
/// runs FAULT_TEST_PROGRAM. See that constant's own doc comment; mirrors
/// WAITSTATUS_TEST_PROCESS's own "never run() directly, exists purely as
/// a fork() source" pattern exactly. PID 10 (not 9 -- this branch was
/// started before Milestone 45 claimed 9 for EXEC_TEST_PROCESS_ID; real
/// collision found and resolved at merge time, not by original design).
static FAULT_TEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const FAULT_TEST_PROCESS_ID: u8 = 10;

/// MILESTONE 57: the POSITIVE demand-paging case. `sbrk()`s 258056 bytes
/// in one call (deliberately more than the pre-Milestone-57 16 KiB heap
/// cap ever allowed -- proof the reservation genuinely grew), then
/// touches a byte at offset 258048 (0x3F000) -- the FIRST byte of heap
/// page index 63, the very LAST page of the new 64-page reservation, not
/// an arbitrary early one -- via a plain `mov byte [rbx], 0x51`. That
/// page has never been mapped by anything (create_process_from_image()
/// no longer eagerly maps any heap page, see that function's own
/// comment), so this deliberately triggers a real hardware #PF; this
/// program has no idea whether that fault gets resolved transparently or
/// terminates it -- `self_test_demand_paging_heap()` is what checks,
/// from the kernel side, that it actually DID get resolved (real,
/// specific frame now backing page 63, holding the exact marker byte)
/// rather than merely "didn't crash".
///
///   offset  bytes                              instruction
///    0      BF 08 F0 03 00                     mov edi, 258056   (sbrk request)
///    5      B8 02 00 00 00                     mov eax, 2        (syscall 2 = sbrk)
///   10      CD 80                              int 0x80          -- rax = heap ptr (HEAP_START+0)
///   12      48 89 C3                           mov rbx, rax
///   15      48 81 C3 00 F0 03 00                add rbx, 258048   (byte 0 of heap page 63)
///   22      C6 03 51                           mov byte [rbx], 0x51   -- FAULTS HERE, then resumes
///   25      B8 01 00 00 00                     mov eax, 1        (exit)
///   30      CD 80                              int 0x80
///   32      EB FE                              jmp $             (unreachable safety net)
pub(crate) const DEMAND_PAGE_TEST_PROGRAM: [u8; 34] = [
    0xBF, 0x08, 0xF0, 0x03, 0x00, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC3, 0x48,
    0x81, 0xC3, 0x00, 0xF0, 0x03, 0x00, 0xC6, 0x03, 0x51, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xCD, 0x80,
    0xEB, 0xFE,
];
/// The exact heap byte-offset (from HEAP_START) DEMAND_PAGE_TEST_PROGRAM
/// writes its marker byte to -- checked by self_test_demand_paging_heap()
/// against its own independent arithmetic (258048 = 63 * PAGE_SIZE)
/// before trusting it, same discipline as SIGSEGV_MSG_OFFSET's own
/// pre-flight check.
const DEMAND_PAGE_TEST_HEAP_OFFSET: u64 = 258048;
const DEMAND_PAGE_TEST_MARKER: u8 = 0x51;

/// MILESTONE 57: an ELEVENTH hardcoded, boot-time-created process slot --
/// runs DEMAND_PAGE_TEST_PROGRAM.
static DEMAND_PAGE_TEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const DEMAND_PAGE_TEST_PROCESS_ID: u8 = 11;

/// MILESTONE 57: the NEGATIVE case, proving the committed-vs-reserved
/// boundary `try_demand_page_heap()` checks is actually enforced, not
/// just documented. `sbrk()`s a small, ordinary 64 bytes (heap_used
/// becomes 64), then dereferences a FIXED absolute address deep inside
/// the SAME heap page (page 63, HEAP_START+HEAP_SIZE-8) the positive-case
/// test above successfully demand-pages -- except THIS process never
/// grew its `heap_used` anywhere near that far, so the identical virtual
/// page that was a legitimate lazy allocation for
/// DEMAND_PAGE_TEST_PROCESS is an genuinely illegal access here: still
/// inside the heap's RESERVED virtual range, but nowhere near what this
/// process has actually committed via sbrk(). Expected outcome: a real
/// SIGSEGV (Milestone 41's unchanged termination path), not a silently
/// "helpful" demand-page.
///
///   offset  bytes                                    instruction
///    0      BF 40 00 00 00                           mov edi, 64        (small, ordinary sbrk)
///    5      B8 02 00 00 00                           mov eax, 2         (syscall 2 = sbrk)
///   10      CD 80                                    int 0x80
///   12      48 BB F8 FF 03 70 55 55 00 00            mov rbx, imm64     (HEAP_START+HEAP_SIZE-8, uncommitted)
///   22      48 8B 03                                 mov rax, [rbx]     -- FAULTS HERE, should SIGSEGV
///   25      EB FE                                    jmp $              (unreachable safety net)
pub(crate) const DEMAND_PAGE_OOB_TEST_PROGRAM: [u8; 27] = [
    0xBF, 0x40, 0x00, 0x00, 0x00, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0xBB, 0xF8, 0xFF,
    0x03, 0x70, 0x55, 0x55, 0x00, 0x00, 0x48, 0x8B, 0x03, 0xEB, 0xFE,
];

/// MILESTONE 57: a TWELFTH hardcoded, boot-time-created process slot --
/// runs DEMAND_PAGE_OOB_TEST_PROGRAM.
static DEMAND_PAGE_OOB_TEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const DEMAND_PAGE_OOB_TEST_PROCESS_ID: u8 = 12;

/// MILESTONE 37: real, dynamic PID allocation for `fork()`-created
/// children. PIDs 1-12 stay permanently reserved for the twelve
/// pre-existing hardcoded/loaded-file ids above (1/2/3/4/5, with 6-12 now
/// ALL used: Milestones 41/43/45/53/57 claim 6, 7, 8, 9, 10, 11, and 12
/// respectively -- no more headroom left in this range) so a forked
/// child's PID can never collide with any of them; PROCESS_TABLE's own
/// slot `i` is always PID `PID_TABLE_BASE + i`.
///
/// MILESTONE 59: ERRNO_TEST_PROCESS_ID (below) deliberately picks 17, NOT
/// the next free-looking number after 12 -- PIDs 13-16 are this exact
/// dynamic range (`PID_TABLE_BASE` + 0..MAX_PROCESSES), already live,
/// reusable `fork()` targets; claiming one of them as a FOURTEENTH
/// permanent hardcoded id would silently alias a real forked child's own
/// pid the next time `fork()` handed one out, a genuine collision bug,
/// not a cosmetic numbering one. 17 is the first id strictly past this
/// whole reserved block.
pub(crate) const PID_TABLE_BASE: u8 = 13;

/// MILESTONE 37: real, honest, small bound on concurrently-live forked
/// processes -- same "small fixed cap, not full generality" spirit as
/// MAX_OPEN_FILES/MAX_LOAD_SEGMENTS elsewhere in this codebase. 4 is
/// comfortably more than this milestone's own fork/exec/wait test
/// program ever has alive at once (exactly one child at a time, reaped
/// by `wait()` before the parent exits) while leaving real headroom for
/// a more elaborate future test.
pub(crate) const MAX_PROCESSES: usize = 4;

/// MILESTONE 37: the real, dynamic, bounded process table `fork()`
/// allocates from -- `None` is a free slot, `Some(Process)` a live
/// forked child. **Left ADDITIVE, alongside the four pre-existing
/// hardcoded statics above, rather than migrating PROCESS_A/PROCESS_B/
/// LOADED_PROCESS/FDTEST_PROCESS into it**: those four are all built
/// once at boot (or once per `runfile`/`runelf` call) through their own
/// already-verified constructors, and every one of Milestones 30-36's
/// own verification depends on their exact, individually-named
/// identity (`runproc 1`/`runproc 2`/`runfdtest`, `sbrk`'s own id
/// checks, etc.) -- folding them into a generic array would touch
/// every one of those already-working call sites for a rename with no
/// real benefit, adding regression risk for its own sake. This table
/// exists purely to give `fork()` somewhere to put processes that don't
/// have (and structurally can't have, since there can be arbitrarily
/// many of them within the bound) their own named `static`.
static PROCESS_TABLE: Mutex<[Option<Process>; MAX_PROCESSES]> = Mutex::new([const { None }; MAX_PROCESSES]);

/// MILESTONE 59: hand-assembled x86_64 machine code -- deliberately
/// triggers FOUR genuinely different syscall failure causes, back to
/// back, in ONE continuous ring-3 excursion, reading errno back after
/// each via the new syscall 17 (GETERRNO) so `usertest.rs`'s own serial
/// log carries live, real proof that distinct failures really do produce
/// distinct, correct errno values -- not just that the field exists.
/// Never touches syscall 0 (write), so it needs no MESSAGE_OFFSET region
/// installed (same as FAULT_TEST_PROGRAM's own minimal shape).
///
/// Sequence (each `mov r32, imm32` is opcode 0xB8+reg, `int 0x80` is
/// `CD 80`, checked by hand against the actual encodings USER_PROGRAM/
/// DEMAND_PAGE_OOB_TEST_PROGRAM above already use, not guessed):
///
///   read(fd=99, buf=0, len=1)   -- fd 99 was never open()ed by this
///                                  process -- expect EBADF (9)
///   geterrno()                  -- expect rax=9, logged
///   wait(pid=250)                -- 250 is not a live child of this
///                                  process (not even in the dynamic
///                                  PID_TABLE_BASE..+MAX_PROCESSES range)
///                                  -- expect ECHILD (10)
///   geterrno()                  -- expect rax=10, logged
///   sbrk(0xFFFFFFFF)             -- ~4 GiB request against a 64-page
///                                  (256 KiB) fixed heap reservation --
///                                  expect ENOMEM (12)
///   geterrno()                  -- expect rax=12, logged
///   getpgid(pid=250)             -- same not-a-live-process pid as the
///                                  wait() above, different syscall --
///                                  expect ESRCH (3)
///   geterrno()                  -- expect rax=3, logged
///   exit(0)
///
/// `buf=0`/`len=1` on the read() call are never actually dereferenced --
/// syscall 4's own None arm (fd not open) returns before touching either
/// argument, the same "failure path never reaches the pointer" property
/// every other syscall's argument-validation-before-dereference ordering
/// already relies on elsewhere in this file.
pub(crate) const ERRNO_TEST_PROGRAM: [u8; 98] = [
    // read(fd=99, buf=0, len=1) -> EBADF
    0xBF, 0x63, 0x00, 0x00, 0x00, // mov edi, 99
    0xBE, 0x00, 0x00, 0x00, 0x00, // mov esi, 0
    0xBA, 0x01, 0x00, 0x00, 0x00, // mov edx, 1
    0xB8, 0x04, 0x00, 0x00, 0x00, // mov eax, 4 (read)
    0xCD, 0x80, // int 0x80
    // geterrno() -> expect EBADF
    0xB8, 0x11, 0x00, 0x00, 0x00, // mov eax, 17 (geterrno)
    0xCD, 0x80, // int 0x80
    // wait(pid=250) -> ECHILD
    0xBF, 0xFA, 0x00, 0x00, 0x00, // mov edi, 250
    0xB8, 0x08, 0x00, 0x00, 0x00, // mov eax, 8 (wait)
    0xCD, 0x80, // int 0x80
    // geterrno() -> expect ECHILD
    0xB8, 0x11, 0x00, 0x00, 0x00, // mov eax, 17
    0xCD, 0x80, // int 0x80
    // sbrk(0xFFFFFFFF) -> ENOMEM
    0xBF, 0xFF, 0xFF, 0xFF, 0xFF, // mov edi, 0xFFFFFFFF
    0xB8, 0x02, 0x00, 0x00, 0x00, // mov eax, 2 (sbrk)
    0xCD, 0x80, // int 0x80
    // geterrno() -> expect ENOMEM
    0xB8, 0x11, 0x00, 0x00, 0x00, // mov eax, 17
    0xCD, 0x80, // int 0x80
    // getpgid(pid=250) -> ESRCH
    0xBF, 0xFA, 0x00, 0x00, 0x00, // mov edi, 250
    0xB8, 0x0F, 0x00, 0x00, 0x00, // mov eax, 15 (getpgid)
    0xCD, 0x80, // int 0x80
    // geterrno() -> expect ESRCH
    0xB8, 0x11, 0x00, 0x00, 0x00, // mov eax, 17
    0xCD, 0x80, // int 0x80
    // exit(0)
    0xBF, 0x00, 0x00, 0x00, 0x00, // mov edi, 0
    0xB8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1 (exit)
    0xCD, 0x80, // int 0x80
];

/// MILESTONE 59: a THIRTEENTH hardcoded, boot-time-created process slot
/// -- runs ERRNO_TEST_PROGRAM. Id 17, not 13-16 -- see PID_TABLE_BASE's
/// own doc comment just above for why those four specifically are off
/// limits (they're `fork()`'s own live, reusable dynamic range, not free
/// numbering headroom).
static ERRNO_TEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const ERRNO_TEST_PROCESS_ID: u8 = 17;

/// MILESTONE 60: hand-assembled x86_64 machine code -- real, end-to-end
/// proof of real signal delivery: register a handler, self-signal, get
/// genuinely redirected into the handler mid-execution, and unwind back
/// out via `SIGRETURN` to exactly the interrupted point, with the full
/// register file (not just the instruction pointer) proven intact.
///
/// **Every byte below was produced by a real assembler, not hand-encoded
/// by counting opcode lengths by eye** (this program's control flow --
/// a handler whose own `ret` must land on a kernel-injected on-stack
/// trampoline at exactly the right runtime address -- has far more real
/// opportunity for a silent off-by-one than any earlier hand-assembled
/// test program in this codebase, which are all straight-line syscall
/// sequences): assembled via `as --64` (Intel syntax) from real x86_64
/// assembly source, symbol offsets read back from the resulting object
/// file's own symbol table (`nm`), and the `HANDLER_ADDR` immediate
/// patched in afterward from the REAL relocation record `as` emitted for
/// it (not guessed) -- then independently re-disassembled with `objdump
/// -D -b binary -m i386:x86-64` against the FINAL patched bytes below to
/// confirm every instruction decodes back to exactly what was intended,
/// including the patched absolute address. See this milestone's own
/// report for the exact source/toolchain commands used.
///
/// Layout (`SIGNAL_TEST_RESUME_OFFSET`/`SIGNAL_TEST_HANDLER_OFFSET`
/// below name the two real jump targets a human reader needs to trust
/// without re-disassembling by hand):
///
///   0x00  sbrk(64)                    -- commits the first real heap
///                                        page (HEAP_START) so the raw
///                                        stores below don't fault
///   0x0c  r12d = 0x1234ABCD           -- the canary: must survive being
///                                        redirected through the handler
///                                        and back via SIGRETURN
///   0x12  HEAP_START[0] = 0xAA        -- marker: main-before-signal code
///                                        genuinely ran
///   0x1f  sigaction(SIGUSR1, handler) -- registers the real handler
///   0x35  sigsend(self_pid, SIGUSR1)  -- self-signal (real POSIX
///                                        raise()); this int 0x80 is
///                                        the ONE that never actually
///                                        "returns" here -- the kernel
///                                        redirects execution into
///                                        `handler` instead
///  =0x46  RESUME (SIGNAL_TEST_RESUME_OFFSET) -- reached ONLY via
///                                        SIGRETURN restoring this exact
///                                        rip, i.e. exactly where the
///                                        sigsend() call's own int 0x80
///                                        would have returned to had no
///                                        signal ever been involved
///         HEAP_START[16..24] = r12    -- proves r12 (the canary) came
///                                        back with its ORIGINAL value,
///                                        not the handler's clobbered one
///         HEAP_START[24] = 0xCC       -- marker: main-after-resume code
///                                        genuinely ran
///         exit(0)
///  =0x64  HANDLER (SIGNAL_TEST_HANDLER_OFFSET)
///         HEAP_START[8] = 0xBB        -- marker: the handler itself
///                                        genuinely ran
///         r12d = 0xDEADBEEF           -- deliberately clobbers the
///                                        canary, so RESUME's later
///                                        check is a real proof of
///                                        restoration, not an accident
///         HEAP_START[32..40] = rdi    -- proves the handler received
///                                        the real signal number (10,
///                                        SIGUSR1) as its SysV first
///                                        argument, the real convention
///                                        a C `void handler(int signum)`
///                                        expects
///         ret                          -- pops the kernel-injected fake
///                                        return address off the stack,
///                                        landing on the kernel's own
///                                        on-stack trampoline (built
///                                        fresh at delivery time by
///                                        usertest.rs, NOT part of this
///                                        static image), which itself
///                                        issues SIGRETURN
pub(crate) const SIGNAL_TEST_PROGRAM: [u8; 125] = [
    // -- 0x00: sbrk(64) --
    0xBF, 0x40, 0x00, 0x00, 0x00, // mov edi, 64
    0xB8, 0x02, 0x00, 0x00, 0x00, // mov eax, 2 (sbrk)
    0xCD, 0x80, // int 0x80
    // -- 0x0c: canary register --
    0x41, 0xBC, 0xCD, 0xAB, 0x34, 0x12, // mov r12d, 0x1234ABCD
    // -- 0x12: HEAP_START[0] = 0xAA --
    0x48, 0xB8, 0x00, 0x00, 0x00, 0x70, 0x55, 0x55, 0x00, 0x00, // movabs rax, 0x555570000000 (HEAP_START)
    0xC6, 0x00, 0xAA, // mov byte [rax], 0xAA
    // -- 0x1f: sigaction(SIGUSR1=10, HANDLER_ADDR) --
    0xBF, 0x0A, 0x00, 0x00, 0x00, // mov edi, 10 (SIGUSR1)
    0x48, 0xBE, 0x64, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, // movabs rsi, 0x555550000064 (USER_CODE_ADDR+0x64, HANDLER_ADDR)
    0xB8, 0x12, 0x00, 0x00, 0x00, // mov eax, 18 (sigaction)
    0xCD, 0x80, // int 0x80
    // -- 0x35: sigsend(self_pid=18, SIGUSR1=10) -- self-signal --
    0xBF, 0x12, 0x00, 0x00, 0x00, // mov edi, 18 (SIGNAL_TEST_PROCESS_ID, targeting self)
    0xBE, 0x0A, 0x00, 0x00, 0x00, // mov esi, 10 (SIGUSR1)
    0xB8, 0x13, 0x00, 0x00, 0x00, // mov eax, 19 (sigsend)
    0xCD, 0x80, // int 0x80 -- redirected into HANDLER, does NOT fall through to the next byte
    // == 0x46 == RESUME (SIGNAL_TEST_RESUME_OFFSET) -- reached only via SIGRETURN
    0x48, 0xB8, 0x00, 0x00, 0x00, 0x70, 0x55, 0x55, 0x00, 0x00, // movabs rax, 0x555570000000 (HEAP_START)
    0x4C, 0x89, 0x60, 0x10, // mov [rax+0x10], r12  -- proves the canary survived
    0xC6, 0x40, 0x18, 0xCC, // mov byte [rax+0x18], 0xCC  -- marker: resume code ran
    0xBF, 0x00, 0x00, 0x00, 0x00, // mov edi, 0
    0xB8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1 (exit)
    0xCD, 0x80, // int 0x80
    // == 0x64 == HANDLER (SIGNAL_TEST_HANDLER_OFFSET)
    0x48, 0xB8, 0x00, 0x00, 0x00, 0x70, 0x55, 0x55, 0x00, 0x00, // movabs rax, 0x555570000000 (HEAP_START)
    0xC6, 0x40, 0x08, 0xBB, // mov byte [rax+0x8], 0xBB  -- marker: handler ran
    0x41, 0xBC, 0xEF, 0xBE, 0xAD, 0xDE, // mov r12d, 0xDEADBEEF  -- deliberately clobbers the canary
    0x48, 0x89, 0x78, 0x20, // mov [rax+0x20], rdi  -- proves rdi==signum was really delivered
    0xC3, // ret -- pops the kernel-injected trampoline return address
];

/// See `SIGNAL_TEST_PROGRAM`'s own doc comment -- the real offset (within
/// that image, i.e. relative to `usertest::USER_CODE_ADDR`) execution
/// resumes at once `SIGRETURN` restores the interrupted context. Checked
/// against the actual assembled bytes at boot by
/// `self_test_signal_delivery()`, not just asserted here.
const SIGNAL_TEST_RESUME_OFFSET: u64 = 0x46;
/// See `SIGNAL_TEST_PROGRAM`'s own doc comment -- the real offset (within
/// that image) of the handler entry point, baked into the program's own
/// `sigaction()` call as `USER_CODE_ADDR + SIGNAL_TEST_HANDLER_OFFSET`.
const SIGNAL_TEST_HANDLER_OFFSET: u64 = 0x64;
/// Real POSIX signal number `SIGNAL_TEST_PROGRAM` registers/raises --
/// `signal::SIGUSR1`, kept as its own local constant (rather than
/// referencing `signal::SIGUSR1` directly from inside the byte array's
/// own comments/self-test arithmetic) purely so this file's own layout
/// doc comment above and `self_test_signal_delivery()` below both read
/// the same literal `10` a hex-dump/serial-log reader would actually see.
const SIGNAL_TEST_SIGNUM: u8 = signal::SIGUSR1;
/// Canary value `SIGNAL_TEST_PROGRAM` loads into r12 before self-
/// signaling -- must come back unchanged after SIGRETURN despite the
/// handler deliberately overwriting r12 with `SIGNAL_TEST_CANARY_CLOBBER`
/// in between.
const SIGNAL_TEST_CANARY: u64 = 0x1234ABCD;
const SIGNAL_TEST_CANARY_CLOBBER: u32 = 0xDEADBEEF;

/// MILESTONE 60: a FOURTEENTH hardcoded, boot-time-created process slot
/// -- runs SIGNAL_TEST_PROGRAM. Id 18, not 13-16 -- see PID_TABLE_BASE's
/// own doc comment (those four are fork()'s own live, reusable dynamic
/// range, not free numbering headroom); also baked directly into
/// SIGNAL_TEST_PROGRAM's own hand-assembled `sigsend(self_pid=18, ...)`
/// call, so this constant and that program's own bytes must stay in
/// sync (checked by self_test_signal_delivery() at boot, not just
/// asserted).
static SIGNAL_TEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const SIGNAL_TEST_PROCESS_ID: u8 = 18;

/// MILESTONE 64: the real on-disk file every one of the four mmap test
/// programs below opens and maps -- written once at boot by
/// `self_test_mmap()` itself (same self-contained-fixture pattern
/// `fs::self_test_disk_write()` already established: the self-test sets
/// up its own real data, rather than main.rs pre-seeding it). Exactly 32
/// bytes, comfortably under `fs::MAX_FILE_BYTES`/`PAGE_SIZE`.
pub(crate) const MMAP_TEST_FILE_PATH: &str = "mmapf";
pub(crate) const MMAP_TEST_FILE_CONTENT: &str = "milestone64-mmap-selftest-real!";

/// MILESTONE 64: hand-assembled x86_64 machine code, the POSITIVE case --
/// `sbrk(64)` for scratch heap space, `open("mmapf")`, `mmap(fd)`, then
/// FOUR real ring-3 byte reads through the mapped address (`mov al,
/// [rdx+i]` for i in 0..4) each immediately persisted into the heap
/// (`mov [rbx+i], al`) so `self_test_mmap()` can verify the exact real
/// bytes afterward the same way `self_test_demand_paging_heap()` verifies
/// its own marker byte. Only the FIRST read (offset 0) faults (real
/// not-present -> demand-paged); the next three prove the mapping stays
/// resident across repeated accesses, not re-faulted every time. Finally
/// `munmap()`s the region and persists its (0 = success) result byte too.
/// Regenerated deterministically by a standalone Python assembler script
/// (same discipline as every other hand-assembled program in this file --
/// each instruction's encoding hand-derived and cross-checked against
/// this file's own already-verified encodings: `48 89 C3`-style
/// register-to-register MOV, `8A`/`88`-style single-byte MOV, `CD 80`
/// syscalls -- not hand-counted hex digits).
///
/// Layout (verified against the array below by self_test_mmap()):
///   offset   0..112   the syscall sequence itself (117 bytes, padded to
///                      112... see path_offset below -- code is 117 bytes,
///                      rounded UP to the next 16-byte boundary for the
///                      path region, so path starts at 112 only once the
///                      code itself is confirmed <= 112; checked directly)
///   offset 112..117   "mmapf" (PATH, 5 real bytes, path for syscall 3)
pub(crate) const MMAP_READ_TEST_PROGRAM: [u8; 117] = [
    0xBF, 0x40, 0x00, 0x00, 0x00, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC3, 0x48,
    0xBF, 0x70, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x05, 0x00, 0x00, 0x00, 0xB8, 0x03,
    0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC7, 0xB8, 0x15, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48,
    0x89, 0xC2, 0x8A, 0x02, 0x88, 0x03, 0x8A, 0x42, 0x01, 0x88, 0x43, 0x01, 0x8A, 0x42, 0x02, 0x88,
    0x43, 0x02, 0x8A, 0x42, 0x03, 0x88, 0x43, 0x03, 0xB8, 0x16, 0x00, 0x00, 0x00, 0x48, 0x89, 0xD7,
    0xCD, 0x80, 0x88, 0x43, 0x04, 0xBF, 0x00, 0x00, 0x00, 0x00, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xCD,
    0x80, 0xEB, 0xFE, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x6D, 0x6D, 0x61, 0x70, 0x66,
];
const MMAP_READ_TEST_PATH_OFFSET: usize = 112;

/// MILESTONE 64: a FIFTEENTH hardcoded, boot-time-created process slot --
/// runs MMAP_READ_TEST_PROGRAM. `self_test_mmap()` `run()`s it directly
/// (top-level excursion, not a fork() source) and inspects its heap
/// afterward, same pattern as DEMAND_PAGE_TEST_PROCESS.
static MMAP_READ_TEST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const MMAP_READ_TEST_PROCESS_ID: u8 = 19;

/// MILESTONE 64: hand-assembled x86_64 machine code, NEGATIVE case #1 --
/// `open("mmapf")`, `mmap(fd)`, then IMMEDIATELY attempts `mov byte
/// [rdx], 0xFF` with NO prior read -- the page has never been mapped at
/// all, so this is a not-present fault whose `CAUSED_BY_WRITE` bit is
/// set. `try_demand_page_mmap()`'s own doc comment part (1) is exactly
/// what this exercises: real refusal, real fall-through to Milestone
/// 41's unmodified SIGSEGV termination. The `exit(0)`/`jmp $` tail is
/// UNREACHABLE if the refusal works -- pure safety net, same convention
/// FAULT_TEST_PROGRAM's own tail already established.
pub(crate) const MMAP_WRITE_BEFORE_READ_FAULT_PROGRAM: [u8; 69] = [
    0x48, 0xBF, 0x40, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x05, 0x00, 0x00, 0x00, 0xB8,
    0x03, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC7, 0xB8, 0x15, 0x00, 0x00, 0x00, 0xCD, 0x80,
    0x48, 0x89, 0xC2, 0xC6, 0x02, 0xFF, 0xBF, 0x00, 0x00, 0x00, 0x00, 0xB8, 0x01, 0x00, 0x00, 0x00,
    0xCD, 0x80, 0xEB, 0xFE, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x6D, 0x6D, 0x61, 0x70, 0x66,
];
const MMAP_WRITE_BEFORE_READ_FAULT_PATH_OFFSET: usize = 64;

/// MILESTONE 64: a SIXTEENTH hardcoded, boot-time-created process slot --
/// runs MMAP_WRITE_BEFORE_READ_FAULT_PROGRAM. EXPECTED to fault and be
/// terminated -- `self_test_mmap()` checks the mmap slot's own `frame`
/// stayed `None` afterward (the real, direct proof the not-present+write
/// refusal actually fired), not just "didn't hang".
static MMAP_WRITE_BEFORE_READ_FAULT_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const MMAP_WRITE_BEFORE_READ_FAULT_PROCESS_ID: u8 = 20;

/// MILESTONE 64: hand-assembled x86_64 machine code, NEGATIVE case #2 --
/// `sbrk(64)`, `open("mmapf")`, `mmap(fd)`, a REAL read first (persisted
/// to heap[0], proving it genuinely succeeded and demand-paged the page
/// PRESENT-but-not-WRITABLE), THEN `mov byte [rdx], 0xFF` -- this time a
/// genuine hardware PROTECTION_VIOLATION fault (the page IS present),
/// exercising the SECOND, structurally different refusal path
/// `page_fault_handler()`'s own pre-existing
/// `!error_code.contains(PROTECTION_VIOLATION)` guard provides --
/// `try_demand_page_mmap()` is never even CALLED for this fault, unlike
/// case #1 above. An unreachable `0xEE` marker at heap[1] proves (by its
/// ABSENCE) that execution never continued past the faulting write.
pub(crate) const MMAP_WRITE_AFTER_READ_FAULT_PROGRAM: [u8; 85] = [
    0xBF, 0x40, 0x00, 0x00, 0x00, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC3, 0x48,
    0xBF, 0x50, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x05, 0x00, 0x00, 0x00, 0xB8, 0x03,
    0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC7, 0xB8, 0x15, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48,
    0x89, 0xC2, 0x8A, 0x02, 0x88, 0x03, 0xC6, 0x02, 0xFF, 0xC6, 0x43, 0x01, 0xEE, 0xBF, 0x00, 0x00,
    0x00, 0x00, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xEB, 0xFE, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x6D, 0x6D, 0x61, 0x70, 0x66,
];
const MMAP_WRITE_AFTER_READ_FAULT_PATH_OFFSET: usize = 80;

/// MILESTONE 64: a SEVENTEENTH hardcoded, boot-time-created process slot
/// -- runs MMAP_WRITE_AFTER_READ_FAULT_PROGRAM. EXPECTED to fault and be
/// terminated AFTER its first heap marker is genuinely set --
/// `self_test_mmap()` checks heap[0] (the read succeeded) AND heap[1]
/// staying zero (the 0xEE marker was never reached).
static MMAP_WRITE_AFTER_READ_FAULT_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const MMAP_WRITE_AFTER_READ_FAULT_PROCESS_ID: u8 = 21;

/// MILESTONE 64: hand-assembled x86_64 machine code, NEGATIVE case #3 --
/// `sbrk(64)`, `open("mmapf")`, `mmap(fd)`, a real read (persisted to
/// heap[0]), a real `munmap()` (its result persisted to heap[1]), THEN a
/// SECOND read attempt at the SAME address -- proving `munmap()` didn't
/// just release the physical frame but genuinely cleared the slot:
/// `try_demand_page_mmap()` no longer finds a live `Some(MmapSlot)` at
/// this address (real `unmap()` also removed the page-table entry
/// itself), so this second access is a real not-present fault with no
/// eligible slot behind it -- real refusal, real termination, not a
/// silent re-grant of the just-released mapping. An unreachable `0xEE`
/// marker at heap[2] proves execution never continued past it.
pub(crate) const MMAP_USE_AFTER_UNMAP_FAULT_PROGRAM: [u8; 101] = [
    0xBF, 0x40, 0x00, 0x00, 0x00, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC3, 0x48,
    0xBF, 0x60, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x05, 0x00, 0x00, 0x00, 0xB8, 0x03,
    0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC7, 0xB8, 0x15, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48,
    0x89, 0xC2, 0x8A, 0x02, 0x88, 0x03, 0xB8, 0x16, 0x00, 0x00, 0x00, 0x48, 0x89, 0xD7, 0xCD, 0x80,
    0x88, 0x43, 0x01, 0x8A, 0x02, 0x88, 0x43, 0x02, 0xBF, 0x00, 0x00, 0x00, 0x00, 0xB8, 0x01, 0x00,
    0x00, 0x00, 0xCD, 0x80, 0xEB, 0xFE, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x6D, 0x6D, 0x61, 0x70, 0x66,
];
const MMAP_USE_AFTER_UNMAP_FAULT_PATH_OFFSET: usize = 96;

/// MILESTONE 64: an EIGHTEENTH hardcoded, boot-time-created process slot
/// -- runs MMAP_USE_AFTER_UNMAP_FAULT_PROGRAM. EXPECTED to fault and be
/// terminated AFTER both of its first two heap markers are genuinely
/// set -- `self_test_mmap()` checks heap[0] (first read succeeded),
/// heap[1] (munmap succeeded, byte 0), and heap[2] staying zero (the
/// second, post-unmap read never got far enough to write its 0xEE
/// marker).
static MMAP_USE_AFTER_UNMAP_FAULT_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const MMAP_USE_AFTER_UNMAP_FAULT_PROCESS_ID: u8 = 22;

/// MILESTONE 65: hand-assembled x86_64 machine code, the first WRITABLE
/// positive case -- `sbrk(64)`, `open("mmapf")`, `mmap_writable(fd)`
/// (syscall 23), a real READ first (`mov al, [rdx]`, persisted to
/// heap[0] -- the real not-present fault, demand-paged WITH the
/// `WRITABLE` bit this time), THEN a real WRITE (`mov byte [rdx], 0x99`)
/// against the now-ALREADY-mapped page -- an ordinary hardware store,
/// no second fault at all, unlike Milestone 64's own read-only
/// MMAP_WRITE_AFTER_READ_FAULT_PROGRAM which genuinely faults at the
/// analogous point. A second real read (`mov al, [rdx]`) immediately
/// after, persisted to heap[1], proves the write's new byte (0x99)
/// genuinely stuck in the mapped page rather than being silently
/// discarded or faulting invisibly. Finally `munmap()`s the region and
/// persists its (0 = success) result byte to heap[2].
///
/// Layout (verified against the array below by
/// `self_test_mmap_writable()`):
///   offset  0..89   the syscall sequence itself (89 bytes)
///   offset 89..94   "mmapf" (PATH, 5 real bytes, path for syscall 3)
pub(crate) const MMAP_WRITABLE_READ_THEN_WRITE_PROGRAM: [u8; 94] = [
    0xBF, 0x40, 0x00, 0x00, 0x00, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC3, 0x48,
    0xBF, 0x59, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x05, 0x00, 0x00, 0x00, 0xB8, 0x03,
    0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC7, 0xB8, 0x17, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48,
    0x89, 0xC2, 0x8A, 0x02, 0x88, 0x03, 0xC6, 0x02, 0x99, 0x8A, 0x02, 0x88, 0x43, 0x01, 0xB8, 0x16,
    0x00, 0x00, 0x00, 0x48, 0x89, 0xD7, 0xCD, 0x80, 0x88, 0x43, 0x02, 0xBF, 0x00, 0x00, 0x00, 0x00,
    0xB8, 0x01, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xEB, 0xFE,
    0x6D, 0x6D, 0x61, 0x70, 0x66,
];
const MMAP_WRITABLE_READ_THEN_WRITE_PATH_OFFSET: usize = 89;

/// MILESTONE 65: a NINETEENTH hardcoded, boot-time-created process slot --
/// runs MMAP_WRITABLE_READ_THEN_WRITE_PROGRAM. `self_test_mmap_writable()`
/// `run()`s it directly and inspects its heap afterward, same pattern as
/// `MMAP_READ_TEST_PROCESS`.
static MMAP_WRITABLE_READ_THEN_WRITE_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const MMAP_WRITABLE_READ_THEN_WRITE_PROCESS_ID: u8 = 23;

/// MILESTONE 65: hand-assembled x86_64 machine code, the second WRITABLE
/// positive case -- `sbrk(64)`, `open("mmapf")`, `mmap_writable(fd)`
/// (syscall 23), then IMMEDIATELY a real WRITE (`mov byte [rdx], 0x77`)
/// with NO prior read -- the page has never been mapped at all, so this
/// is a genuine not-present fault whose `CAUSED_BY_WRITE` bit is set,
/// EXACTLY the same real hardware condition Milestone 64's own
/// MMAP_WRITE_BEFORE_READ_FAULT_PROGRAM exercises -- except this slot is
/// `writable`, so `try_demand_page_mmap()`'s `is_write && !writable`
/// refusal never fires: the fault is serviced instead, proving the
/// genuinely different first-touch-write-succeeds code path. A real read
/// back (`mov al, [rdx]`) persisted to heap[0] proves the written byte
/// (0x77) is really there; a SECOND real read at `[rdx+1]` persisted to
/// heap[1] proves the rest of the page still holds the real snapshotted
/// file content (only the single touched byte changed, nothing else was
/// zeroed or corrupted). Finally `munmap()`s the region and persists its
/// result byte to heap[2].
///
/// Layout (verified against the array below by
/// `self_test_mmap_writable()`):
///   offset  0..90   the syscall sequence itself (90 bytes)
///   offset 90..95   "mmapf" (PATH, 5 real bytes, path for syscall 3)
pub(crate) const MMAP_WRITABLE_WRITE_FIRST_PROGRAM: [u8; 95] = [
    0xBF, 0x40, 0x00, 0x00, 0x00, 0xB8, 0x02, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC3, 0x48,
    0xBF, 0x5A, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x05, 0x00, 0x00, 0x00, 0xB8, 0x03,
    0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC7, 0xB8, 0x17, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48,
    0x89, 0xC2, 0xC6, 0x02, 0x77, 0x8A, 0x02, 0x88, 0x03, 0x8A, 0x42, 0x01, 0x88, 0x43, 0x01, 0xB8,
    0x16, 0x00, 0x00, 0x00, 0x48, 0x89, 0xD7, 0xCD, 0x80, 0x88, 0x43, 0x02, 0xBF, 0x00, 0x00, 0x00,
    0x00, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xEB, 0xFE,
    0x6D, 0x6D, 0x61, 0x70, 0x66,
];
const MMAP_WRITABLE_WRITE_FIRST_PATH_OFFSET: usize = 90;

/// MILESTONE 65: a TWENTIETH hardcoded, boot-time-created process slot --
/// runs MMAP_WRITABLE_WRITE_FIRST_PROGRAM. `self_test_mmap_writable()`
/// `run()`s it directly and inspects its heap afterward, same pattern as
/// `MMAP_WRITABLE_READ_THEN_WRITE_PROCESS` above.
static MMAP_WRITABLE_WRITE_FIRST_PROCESS: Mutex<Option<Process>> = Mutex::new(None);
pub(crate) const MMAP_WRITABLE_WRITE_FIRST_PROCESS_ID: u8 = 24;

/// The kernel's own PML4 physical frame + CR3 flags, saved ONCE at boot
/// (save_kernel_cr3(), called from kernel_main before any process is
/// created) via a real Cr3::read() -- restored verbatim on every
/// process's exit syscall, never reconstructed or guessed at.
static KERNEL_PML4_FRAME: AtomicU64 = AtomicU64::new(0);
static KERNEL_CR3_FLAGS_BITS: AtomicU64 = AtomicU64::new(0);

/// 0 = no process currently running (plain `usertest`, or idle);
/// nonzero = the id of the process currently executing in ring 3.
/// usertest.rs's syscall_dispatch reads this to decide whether the write
/// syscall's log line should say "legacy usertest" or "process N", and
/// whether syscall 1 needs to restore the kernel's own CR3 before
/// resuming kernel code.
pub(crate) static ACTIVE_PROCESS: AtomicU8 = AtomicU8::new(0);

/// MILESTONE 44 self-test support: records which process id most
/// recently reached syscall WRITE (rax=0) -- set unconditionally inside
/// usertest.rs's syscall_dispatch WRITE arm, right alongside the
/// `active` value it already reads. Exists purely so
/// loader::self_test_altentry_elf() can assert REAL proof that ring-3
/// execution reached the write syscall from inside the new, non-default
/// entry point's own mapped page -- strictly stronger than "the kernel
/// didn't panic", which (since Milestone 41's page-fault handler now
/// gracefully terminates a faulting ring-3 process instead of
/// panicking) can no longer by itself distinguish "really executed" from
/// "faulted immediately and was cleanly recovered from". This
/// milestone's own test ELF is deliberately built with its only mapped
/// page three pages past `USER_CODE_ADDR`, so a regression to the old
/// "entry forced to USER_CODE_ADDR" bug would jump into an UNMAPPED page
/// and fault before ever reaching this syscall -- making "did WRITE ever
/// run for this process id" a real, load-bearing check, not a redundant
/// one.
pub(crate) static LAST_WRITE_SYSCALL_PID: AtomicU8 = AtomicU8::new(0);

/// POST-MILESTONE-65 FIX: the real root cause of the disclosed
/// "fresh-second-ATA-drive triple fault" -- found via a real `-d int`
/// QEMU hardware trace (see this fix pass's own README entry for the
/// full trace/analysis), not guessed at. `create_process_from_image()`/
/// `create_process_from_elf()` both used to determine which PML4 to
/// copy the 511 shared "kernel-space" entries FROM by calling
/// `Cr3::read()` -- "whatever is CURRENTLY loaded" -- rather than this
/// function's own canonically-saved `KERNEL_PML4_FRAME`. For every
/// ORDINARY process-creation call site (every boot-time self-test,
/// every `runfile`/`runelf` shell command, `fork()`'s own child) this
/// silently happened to coincide with the real kernel PML4, since
/// those all run from genuine kernel context (CR3 already restored to
/// `KERNEL_PML4_FRAME` by the time they're reached). The ONE real
/// exception: `exec_elf()`/`exec_elf_with_args()` (Milestones 45/58)
/// call `create_process_from_elf()` from INSIDE the EXEC/EXECARGV
/// syscall's own dispatch -- i.e. while CR3 is still the DYING
/// process's OWN private PML4, not the kernel's -- the one call path
/// in this whole file where `Cr3::read()` and `KERNEL_PML4_FRAME`
/// genuinely diverge. Returning the real, saved value here instead
/// closes that gap for both call sites at once, rather than patching
/// each one separately with its own `Cr3::read()`.
fn kernel_pml4_for_new_process() -> (PhysFrame<Size4KiB>, Cr3Flags) {
    let frame_addr = KERNEL_PML4_FRAME.load(Ordering::SeqCst);
    let frame = PhysFrame::<Size4KiB>::from_start_address(PhysAddr::new(frame_addr))
        .expect("saved kernel PML4 frame address was not 4KiB-aligned");
    let flags = Cr3Flags::from_bits_truncate(KERNEL_CR3_FLAGS_BITS.load(Ordering::SeqCst));
    (frame, flags)
}

/// MILESTONE 30: called once at boot, before any process's PML4 could
/// ever be loaded into CR3, so there's always a known-good value to
/// restore to on every process exit -- never inferred, never assumed
/// still-current.
pub fn save_kernel_cr3() {
    let (frame, flags) = Cr3::read();
    KERNEL_PML4_FRAME.store(frame.start_address().as_u64(), Ordering::SeqCst);
    KERNEL_CR3_FLAGS_BITS.store(flags.bits(), Ordering::SeqCst);
    let _ = writeln!(
        serial(),
        "milestone 30: saved kernel's own PML4 frame {:#x} (cr3 flags {:#x}) for restore-on-exit",
        frame.start_address().as_u64(),
        flags.bits()
    );
}

/// MILESTONE 30: switches CR3 back to the kernel's own, original PML4.
/// Called from usertest.rs's syscall_dispatch, exit-syscall arm, BEFORE
/// resume_kernel() hands control back to ordinary kernel code -- see
/// that call site's own comment for why this ordering is enforced
/// explicitly rather than relied upon implicitly.
pub(crate) fn restore_kernel_cr3() {
    let frame_addr = KERNEL_PML4_FRAME.load(Ordering::SeqCst);
    let frame = PhysFrame::<Size4KiB>::from_start_address(PhysAddr::new(frame_addr))
        .expect("saved kernel PML4 frame address was not 4KiB-aligned");
    let flags = Cr3Flags::from_bits_truncate(KERNEL_CR3_FLAGS_BITS.load(Ordering::SeqCst));
    unsafe { Cr3::write(frame, flags) };
}

/// MILESTONE 33: reads one byte back out of HEAP_START -- called from
/// usertest.rs's syscall_dispatch, write-syscall arm, same timing/
/// reasoning as the message read just above it (CR3 is still the active
/// process's own PML4 at this point, the exit syscall is what restores
/// the kernel's). Real isolation proof: identical virtual address every
/// time, resolves through whichever process's own private heap mapping
/// is currently loaded.
pub(crate) fn read_active_heap_marker() -> u8 {
    let ptr = HEAP_START as *const u8;
    unsafe { core::ptr::read(ptr) }
}

/// MILESTONE 57: a real, pre-existing bug this milestone's own
/// verification found and fixed -- `read_active_heap_marker()` above
/// unconditionally dereferences page 0 of the active process's heap,
/// called from usertest.rs's WRITE-syscall arm purely for a diagnostic
/// log line, on EVERY write() call, regardless of whether that process
/// has ever touched its heap at all. Before this milestone, heap page 0
/// was ALWAYS eagerly mapped at process-creation time, so this never
/// faulted. As of Milestone 57's demand-paged heap, a process that never
/// calls `sbrk()` (SIGSEGV_TEST_PROCESS/FAULT_TEST_PROCESS/
/// WAITSTATUS_TEST_PROCESS/EXEC_TEST_PROCESS/etc. -- most of the
/// hardcoded test processes in this file) genuinely has NO physical frame
/// backing page 0, and a first real boot with this milestone's changes
/// crashed the kernel for real: a page fault with a Ring0 code segment
/// (this dereference happens from KERNEL context, mid-syscall-dispatch,
/// not ring 3) at exactly HEAP_START, immediately after
/// SIGSEGV_TEST_PROCESS's own write() call -- confirmed via a real QEMU
/// boot log, not by inspection. Callers must check this FIRST and skip
/// the read entirely if it returns `false`.
pub(crate) fn heap_page0_mapped(id: u8) -> bool {
    with_process_mut(id, |p| p.heap_frames[0].is_some()).unwrap_or(false)
}

/// MILESTONE 33: the `sbrk` syscall's actual kernel-side implementation
/// -- called from usertest.rs's syscall_dispatch, syscall-2 arm, with the
/// CURRENTLY-active process's id (from ACTIVE_PROCESS) and the requested
/// byte count (from the caller's rdi). Bumps that process's OWN
/// heap_used counter and returns a pointer into ITS OWN heap's RESERVED
/// (as of Milestone 57, not necessarily physically mapped -- see
/// `try_demand_page_heap()`) virtual region -- `None` if the request
/// would run past the fixed `HEAP_SIZE` reservation or if `id` doesn't
/// name a live process. This function itself still never maps or
/// touches a single physical frame -- purely virtual bookkeeping, same
/// as before Milestone 57; a real frame only shows up once the returned
/// pointer is actually dereferenced and a hardware page fault demand-
/// pages it in.
///
/// MILESTONE 37: now routed through with_process_mut() like every other
/// per-process syscall, rather than matching only PROCESS_A/PROCESS_B
/// directly -- a real, disclosed Milestone 34 limitation (sbrk only ever
/// recognized ids 1/2, flagged in loader.rs's own doc comment) is fixed
/// as a direct, low-risk consequence of this milestone's own
/// with_process_mut() generalization, not a separate change: every
/// existing caller (ids 1/2) behaves bit-for-bit identically, and every
/// OTHER process (loaded-from-file, fdtest, fork-test, and now forked
/// children) gets a genuinely working sbrk() for free.
pub(crate) fn sbrk(id: u8, size: u64) -> Option<u64> {
    with_process_mut(id, |proc| {
        let new_used = proc.heap_used.checked_add(size)?;
        if new_used > HEAP_SIZE {
            return None;
        }
        let ptr = HEAP_START + proc.heap_used;
        proc.heap_used = new_used;
        Some(ptr)
    })?
}

/// MILESTONE 87: rewind this process's `sbrk` bump pointer all the way
/// back to `HEAP_START` -- i.e. logically free its entire heap in one
/// call, so the next `sbrk()` re-hands the very same virtual range.
/// Any physical frames Milestone 57's demand paging already mapped for
/// that range STAY mapped (they are PRESENT|WRITABLE|USER, so a later
/// write just overwrites them -- no fault, no leak, no re-map cost);
/// only the `heap_used` accounting is reset. Returns `Some(HEAP_START)`
/// on success (the new break), `None` if `id` doesn't name a live
/// process. The `tools/cc_src` compiler self-test calls this between
/// compilations (its per-compile tokens/AST/CodeBuf are deliberately
/// never individually freed -- see its own Milestone 67 scope), which
/// is what keeps ~50 back-to-back compiles inside the 64-page
/// reservation instead of leaking past it. A real userspace program
/// that genuinely needs its heap contents would simply never call this;
/// it is an explicit "throw the whole heap away now" request, not
/// automatic.
pub(crate) fn sbrk_reset(id: u8) -> Option<u64> {
    with_process_mut(id, |proc| {
        proc.heap_used = 0;
        HEAP_START
    })
}

/// MILESTONE 57: real page-fault-driven demand paging for the per-process
/// heap. Called from interrupts.rs's page_fault_handler, BEFORE that
/// handler's own unconditional SIGSEGV-termination path, for exactly one
/// case: a genuine hardware NOT-PRESENT page fault (never a protection
/// violation -- checked by the caller, not here -- a heap page this
/// kernel has already mapped is always PRESENT|WRITABLE|USER_ACCESSIBLE,
/// so a protection-violation fault there would mean something is
/// actually wrong, not a legitimate lazy-allocation request) whose
/// faulting address falls inside `pid`'s own heap virtual range AND
/// within bytes it has already legitimately committed via `sbrk()`
/// (`fault_addr < HEAP_START + heap_used`) -- exactly matching real
/// POSIX brk/demand-paging semantics: you can only fault in a page you
/// already own via brk, not anywhere in the reserved-but-uncommitted
/// tail of the address space. Everything else -- an address outside the
/// heap range entirely, or inside the heap's RESERVED but not-yet-
/// `sbrk()`'d region -- returns `false` and falls straight through to
/// the existing, completely unmodified SIGSEGV path. See
/// `self_test_demand_paging_heap()`'s own negative case, which proves
/// this boundary is actually enforced, not just documented.
///
/// On success: allocates one fresh physical frame, zeroes it (a demand-
/// paged heap page reads as all-zero on first touch, same as a real
/// OS's anonymous memory -- and the same zero-fill this process's heap
/// pages always got when they were eagerly mapped, before this
/// milestone), maps it into `pid`'s OWN private page tables (reconstructed
/// from its stored `pml4_frame`, the same raw-pointer-to-`&mut
/// PageTable` pattern `create_process_from_image()` uses to build a
/// fresh one), records it in `heap_frames[page_index]`, and returns
/// `true`. The caller (`page_fault_handler`) then simply returns without
/// terminating anything -- for a fault-class exception, returning from
/// the handler naturally retries the SAME faulting instruction, which
/// now succeeds against the freshly-mapped page. No register state needs
/// saving/restoring here; the CPU's own interrupt-frame mechanism already
/// handles the retry.
pub(crate) fn try_demand_page_heap(pid: u8, fault_addr: u64) -> bool {
    if pid == 0 || fault_addr < HEAP_START || fault_addr >= HEAP_START + HEAP_SIZE {
        return false;
    }
    let offset = fault_addr - HEAP_START;
    let page_index = (offset / PAGE_SIZE as u64) as usize;

    let eligible = with_process_mut(pid, |p| {
        offset < p.heap_used && page_index < p.heap_frames.len() && p.heap_frames[page_index].is_none()
    });
    if eligible != Some(true) {
        return false;
    }

    let phys_mem_offset = memory::phys_mem_offset();
    let pml4_frame = match with_process_mut(pid, |p| p.pml4_frame) {
        Some(f) => f,
        None => return false,
    };

    let new_frame = match memory::with_frame_allocator(|fa| fa.allocate_frame()) {
        Some(Some(f)) => f,
        _ => {
            let _ = writeln!(
                serial(),
                "milestone 57: demand-page FAILED (process {pid}, fault addr {fault_addr:#x}, page {page_index}) -- out of physical frames"
            );
            return false;
        }
    };

    // Zeroed through the phys-mem-offset direct view, same reasoning as
    // every other fresh-frame zeroing in this file: this is the new
    // frame's own physical memory, written directly rather than through
    // a virtual address that could currently resolve through someone
    // else's page tables.
    let frame_virt = phys_mem_offset + new_frame.start_address().as_u64();
    unsafe { core::ptr::write_bytes::<u8>(frame_virt.as_mut_ptr(), 0, PAGE_SIZE) };

    let pml4_ptr: *mut PageTable = (phys_mem_offset + pml4_frame.start_address().as_u64()).as_mut_ptr();
    let pml4: &mut PageTable = unsafe { &mut *pml4_ptr };
    let mut mapper = unsafe { OffsetPageTable::new(pml4, phys_mem_offset) };
    let heap_page = Page::<Size4KiB>::containing_address(VirtAddr::new(HEAP_START + (page_index as u64) * PAGE_SIZE as u64));
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

    let map_result = memory::with_frame_allocator(|fa| unsafe { mapper.map_to(heap_page, new_frame, flags, fa) });
    match map_result {
        Some(Ok(flush)) => flush.flush(),
        _ => {
            unsafe { memory::with_frame_allocator(|fa| fa.deallocate_frame(new_frame)) };
            let _ = writeln!(
                serial(),
                "milestone 57: demand-page FAILED (process {pid}, fault addr {fault_addr:#x}, page {page_index}) -- map_to failed"
            );
            return false;
        }
    }

    with_process_mut(pid, |p| {
        p.heap_frames[page_index] = Some(new_frame);
    });

    let _ = writeln!(
        serial(),
        "milestone 57: demand-paged heap page {page_index} for process {pid} (fault addr {fault_addr:#x}) -- fresh zeroed physical frame {:#x} mapped, resuming the faulting instruction",
        new_frame.start_address().as_u64()
    );
    true
}

/// MILESTONE 64: the `mmap` syscall's (21) kernel-side implementation --
/// maps `fd`'s current, ALREADY-BUFFERED content (see `OpenFile`'s own
/// doc comment -- open() already read the whole file at open() time, so
/// no disk access happens here either) read-only into a fresh slot of
/// `pid`'s own `mmaps` table. Returns the new mapping's virtual address,
/// or `None` on any of three real, honestly-refused failures: `fd`
/// doesn't name a currently-open `FdEntry::File` for this process (a
/// pipe end can't be mmap()'d -- it has no fixed byte content to
/// snapshot), the file's buffered content exceeds `PAGE_SIZE` (can only
/// happen if `fs::MAX_FILE_BYTES` and `PAGE_SIZE` ever drift apart --
/// checked defensively even though today they're provably equal, same
/// discipline as this module's other pre-flight arithmetic checks), or
/// `pid`'s own `MAX_MMAPS`-sized table is already full.
///
/// Deliberately does NOT map any physical frame or touch the page tables
/// at all -- exactly like `sbrk()` only ever bumps `heap_used` and lets
/// `try_demand_page_mmap()` do the real work on first touch, this only
/// RESERVES the virtual slot and snapshots the content; the first real
/// hardware page fault against this address is what actually backs it
/// with a physical frame. See `MmapSlot`'s own doc comment for why a
/// snapshot, not a live view of the fd, is this slice's one disclosed
/// simplification.
pub(crate) fn mmap_file(pid: u8, fd: u64) -> Option<u64> {
    mmap_file_impl(pid, fd, false)
}

/// MILESTONE 65: the `mmap_writable` syscall's (23) kernel-side
/// implementation -- real, private (`MAP_PRIVATE`-equivalent) `PROT_WRITE`
/// support, the first item of Milestone 64's own disclosed "real future
/// work" list, re-checked directly against the actual code (not just
/// Milestone 64's own write-up) before picking it: `MmapSlot` already had
/// everywhere else needed (a per-process, per-slot snapshot with no
/// sharing between mappings -- see `mmaps` field's own doc comment
/// disclosing fork() never copies it, and no two mmap() calls anywhere in
/// this kernel can ever alias the same physical frame), so a writable
/// slot is ALREADY structurally private; the only real gap was the
/// hardcoded refusal in `try_demand_page_mmap()` and the missing
/// `WRITABLE` page-table bit. Otherwise identical to `mmap_file()` above
/// -- same fd/size/table-full checks, same snapshot-not-live-view
/// semantics (a write here NEVER reaches the backing fd's buffer or the
/// on-disk file -- proven directly by `self_test_mmap_writable()` re-
/// reading the real on-disk file afterward and finding it unchanged).
pub(crate) fn mmap_file_writable(pid: u8, fd: u64) -> Option<u64> {
    mmap_file_impl(pid, fd, true)
}

/// MILESTONE 64/65: shared real implementation behind `mmap_file()`
/// (read-only, Milestone 64) and `mmap_file_writable()` (Milestone 65) --
/// identical fd/size/table-full checks and identical "reserve the virtual
/// slot, snapshot the content, map nothing yet" behavior either way; only
/// `writable` (persisted onto the new `MmapSlot`) differs, consumed later
/// by `try_demand_page_mmap()`.
fn mmap_file_impl(pid: u8, fd: u64, writable: bool) -> Option<u64> {
    let content = with_process_mut(pid, |proc| match proc.fds.get(fd as usize)?.as_ref()? {
        FdEntry::File(f) => Some(f.buffer.clone()),
        _ => None,
    })??;

    if content.len() > PAGE_SIZE {
        let _ = writeln!(
            serial(),
            "milestone 64: syscall MMAP{} (process {pid}) -- FAILED, fd {fd}'s buffered content ({} bytes) exceeds PAGE_SIZE ({PAGE_SIZE}) -- fs::MAX_FILE_BYTES should make this unreachable, refusing defensively rather than truncating silently",
            if writable { "_WRITABLE" } else { "" },
            content.len()
        );
        return None;
    }

    with_process_mut(pid, |proc| {
        let slot_index = proc.mmaps.iter().position(|s| s.is_none())?;
        proc.mmaps[slot_index] = Some(MmapSlot { content, frame: None, writable });
        let addr = MMAP_START + (slot_index as u64) * PAGE_SIZE as u64;
        let _ = writeln!(
            serial(),
            "milestone {}: syscall MMAP{} (process {pid}) -- fd={fd} slot={slot_index} -- reserved {addr:#x} (not yet backed by any physical frame -- real demand paging on first touch, same mechanism as the per-process heap)",
            if writable { "65" } else { "64" },
            if writable { "_WRITABLE" } else { "" }
        );
        Some(addr)
    })?
}

/// MILESTONE 64: the `munmap` syscall's (22) kernel-side implementation.
/// `addr` must be EXACTLY a slot's own base address (`MMAP_START +
/// i*PAGE_SIZE` for some live slot `i`) -- real, disclosed EINVAL
/// otherwise (this slice never hands out a length/offset a caller could
/// legitimately round from, so exact-match is the honest, complete
/// check, not a shortcut). Unmaps the page table entry if one was ever
/// actually demand-paged in (`frame.is_some()` -- a slot that was
/// reserved but never touched has nothing mapped to undo), frees the
/// physical frame back to the global allocator, and clears the slot so a
/// later `mmap()` call can reuse it. Returns `true` on success, `false`
/// if `addr` doesn't name any of `pid`'s own live slots.
pub(crate) fn munmap_region(pid: u8, addr: u64) -> bool {
    if addr < MMAP_START || addr >= MMAP_START + (MAX_MMAPS as u64) * PAGE_SIZE as u64 {
        return false;
    }
    let offset = addr - MMAP_START;
    if offset % PAGE_SIZE as u64 != 0 {
        return false;
    }
    let slot_index = (offset / PAGE_SIZE as u64) as usize;

    let (had_slot, frame) = with_process_mut(pid, |proc| match proc.mmaps.get(slot_index) {
        Some(Some(slot)) => (true, slot.frame),
        _ => (false, None),
    })
    .unwrap_or((false, None));
    if !had_slot {
        return false;
    }

    if let Some(frame) = frame {
        let phys_mem_offset = memory::phys_mem_offset();
        if let Some(pml4_frame) = with_process_mut(pid, |p| p.pml4_frame) {
            let pml4_ptr: *mut PageTable = (phys_mem_offset + pml4_frame.start_address().as_u64()).as_mut_ptr();
            let pml4: &mut PageTable = unsafe { &mut *pml4_ptr };
            let mut mapper = unsafe { OffsetPageTable::new(pml4, phys_mem_offset) };
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(addr));
            if let Ok((_, flush)) = mapper.unmap(page) {
                flush.flush();
            }
        }
        unsafe { memory::with_frame_allocator(|fa| fa.deallocate_frame(frame)) };
    }

    with_process_mut(pid, |proc| {
        proc.mmaps[slot_index] = None;
    });

    let _ = writeln!(
        serial(),
        "milestone 64: syscall MUNMAP (process {pid}) -- slot={slot_index} addr={addr:#x} -- unmapped (physical frame {}), slot freed for reuse",
        if frame.is_some() { "released" } else { "was never actually backed -- nothing to release" }
    );
    true
}

/// MILESTONE 64: the mmap twin of `try_demand_page_heap()` above --
/// called from `page_fault_handler()` for exactly one additional
/// eligible case: a genuine NOT-PRESENT fault (never a protection
/// violation -- see this function's own `is_write` handling below for
/// why that split matters here specifically, unlike the heap's case)
/// whose address falls inside `pid`'s own `[MMAP_START, MMAP_START +
/// MAX_MMAPS*PAGE_SIZE)` range AND names a slot that is currently `Some`
/// (a real, live reservation) with `frame == None` (never yet backed).
///
/// **The real, hardware-enforced read-only proof, in two parts**: (1) if
/// the fault that got us here was itself caused by a WRITE
/// (`is_write == true`, from `error_code.contains(CAUSED_BY_WRITE)`,
/// checked by the caller before this runs), this function refuses to
/// populate the page at all and returns `false` -- the caller then falls
/// through to the SAME unmodified Milestone 41 SIGSEGV termination path
/// every other illegal access already uses. This is the "first-ever
/// touch is itself a write" case: no page table entry has been created
/// yet, so the CPU reports a not-present fault regardless of whether the
/// attempted access was a read or a write -- only `error_code`'s own
/// `CAUSED_BY_WRITE` bit distinguishes them, and this is exactly where
/// that distinction earns its keep. (2) A READ fault DOES get populated
/// (real content, real physical frame, mapped `PRESENT | USER_ACCESSIBLE`
/// -- deliberately WITHOUT `WRITABLE`), so any LATER write attempt
/// against the now-present page produces a genuine hardware
/// PROTECTION_VIOLATION fault instead -- which `page_fault_handler()`'s
/// own pre-existing `!error_code.contains(PROTECTION_VIOLATION)` guard
/// already excludes from ever reaching this function at all, falling
/// straight through to termination completely unmodified. Both paths
/// converge on the same real, honest outcome (a write to a read-only
/// mapping always terminates the process), verified by TWO separate
/// self-test cases below specifically because they exercise these two
/// genuinely different code paths (not-present-refusal vs.
///
/// **MILESTONE 65 update**: both parts above describe a READ-ONLY slot
/// (`writable == false`, every slot `mmap_file()` ever creates) --
/// completely unchanged behavior. A WRITABLE slot (`writable == true`,
/// only ever produced by this milestone's new `mmap_file_writable()`)
/// takes a genuinely different path at part (1)'s own decision point: a
/// first-touch WRITE is no longer refused -- it demand-pages exactly like
/// a first-touch READ would (real file content copied in first, THEN the
/// page is mapped WITH `WRITABLE`, and the CPU retries the very
/// instruction that faulted, which now succeeds and overwrites whatever
/// byte(s) it targets). A first-touch READ on a writable slot also maps
/// `WRITABLE` immediately (prot is a property of the mapping, decided
/// once, not of which access happens to arrive first -- see the
/// `flags` local below), so a LATER write against an already-mapped
/// writable page is simply an ordinary hardware store with no fault at
/// all, never reaching this function a second time. See
/// `self_test_mmap_writable()` for the real proof of both of THESE two
/// new paths, verified genuinely private (never written back to the
/// backing fd or the on-disk file).
/// protection-violation) rather than assuming one implies the other.
///
/// On success: allocates one fresh physical frame, copies the slot's
/// snapshotted `content` into it (zero-padded past `content.len()` --
/// real, honest "short reads as zero-fill" semantics, same as a real
/// `mmap()`'s trailing partial page), maps it, records the frame in the
/// slot, and returns `true` -- the caller (`page_fault_handler`) returns
/// without terminating anything, the CPU retries the faulting
/// instruction, which now succeeds against the freshly-mapped page.
pub(crate) fn try_demand_page_mmap(pid: u8, fault_addr: u64, is_write: bool) -> bool {
    if pid == 0 || fault_addr < MMAP_START || fault_addr >= MMAP_START + (MAX_MMAPS as u64) * PAGE_SIZE as u64 {
        return false;
    }
    let offset = fault_addr - MMAP_START;
    let slot_index = (offset / PAGE_SIZE as u64) as usize;

    // MILESTONE 65: fetch `writable` ALONGSIDE `content` -- the refusal
    // decision right below now depends on it. A slot created through the
    // original (Milestone 64) `mmap_file()` always has `writable == false`,
    // so this `is_write && !writable` check is EXACTLY Milestone 64's own
    // unconditional `if is_write { return false }` for every pre-existing
    // caller -- zero behavioral change for a read-only slot, verified by
    // Milestone 64's own CASE 2/CASE 3 self-test cases still passing
    // unmodified. Only a slot created through this milestone's new
    // `mmap_file_writable()` (`writable == true`) reaches the new success
    // path below.
    let (content, writable) = match with_process_mut(pid, |p| match p.mmaps.get(slot_index) {
        Some(Some(slot)) if slot.frame.is_none() => Some((slot.content.clone(), slot.writable)),
        _ => None,
    }) {
        Some(Some(c)) => c,
        _ => return false,
    };
    if is_write && !writable {
        // Real, deliberate refusal -- see this function's own doc comment,
        // part (1). Falls through to termination in the caller. Unchanged
        // from Milestone 64 for every read-only slot.
        return false;
    }

    let phys_mem_offset = memory::phys_mem_offset();
    let pml4_frame = match with_process_mut(pid, |p| p.pml4_frame) {
        Some(f) => f,
        None => return false,
    };

    let new_frame = match memory::with_frame_allocator(|fa| fa.allocate_frame()) {
        Some(Some(f)) => f,
        _ => {
            let _ = writeln!(
                serial(),
                "milestone 64: mmap demand-page FAILED (process {pid}, fault addr {fault_addr:#x}, slot {slot_index}) -- out of physical frames"
            );
            return false;
        }
    };

    // Real content, zero-padded past content.len() -- same direct
    // phys-mem-offset write technique try_demand_page_heap() uses to
    // zero a fresh heap frame, just filled with real file bytes here
    // instead of zeros for the first content.len() of them.
    let frame_virt = phys_mem_offset + new_frame.start_address().as_u64();
    unsafe { core::ptr::write_bytes::<u8>(frame_virt.as_mut_ptr(), 0, PAGE_SIZE) };
    unsafe { core::ptr::copy_nonoverlapping(content.as_ptr(), frame_virt.as_mut_ptr::<u8>(), content.len()) };

    let pml4_ptr: *mut PageTable = (phys_mem_offset + pml4_frame.start_address().as_u64()).as_mut_ptr();
    let pml4: &mut PageTable = unsafe { &mut *pml4_ptr };
    let mut mapper = unsafe { OffsetPageTable::new(pml4, phys_mem_offset) };
    let mmap_page = Page::<Size4KiB>::containing_address(VirtAddr::new(MMAP_START + (slot_index as u64) * PAGE_SIZE as u64));
    // MILESTONE 64/65: no PageTableFlags::WRITABLE for a read-only slot --
    // see this function's own doc comment, part (2): this absence IS the
    // real read-only enforcement mechanism, not a cosmetic default.
    // MILESTONE 65: a WRITABLE slot gets the real hardware WRITABLE bit
    // set HERE, at first-touch mapping time (not later, not conditionally
    // on which access triggered the fault) -- real POSIX `mmap()` prot
    // permissions are a property of the MAPPING, decided once at map time,
    // not of whichever individual access happens to touch the page first.
    // This is what lets a subsequent plain store instruction succeed with
    // NO further fault at all once the page is present, exactly like an
    // already-demand-paged heap page.
    let flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | if writable { PageTableFlags::WRITABLE } else { PageTableFlags::empty() };

    let map_result = memory::with_frame_allocator(|fa| unsafe { mapper.map_to(mmap_page, new_frame, flags, fa) });
    match map_result {
        Some(Ok(flush)) => flush.flush(),
        _ => {
            unsafe { memory::with_frame_allocator(|fa| fa.deallocate_frame(new_frame)) };
            let _ = writeln!(
                serial(),
                "milestone 64: mmap demand-page FAILED (process {pid}, fault addr {fault_addr:#x}, slot {slot_index}) -- map_to failed"
            );
            return false;
        }
    }

    with_process_mut(pid, |p| {
        if let Some(slot) = p.mmaps[slot_index].as_mut() {
            slot.frame = Some(new_frame);
        }
    });

    let _ = writeln!(
        serial(),
        "milestone {}: demand-paged mmap slot {slot_index} for process {pid} (fault addr {fault_addr:#x}, is_write={is_write}) -- fresh physical frame {:#x} mapped {} with real file content, resuming the faulting instruction",
        if writable { "65" } else { "64" },
        new_frame.start_address().as_u64(),
        if writable { "READ-WRITE" } else { "READ-ONLY" }
    );
    true
}

/// MILESTONE 35: locates whichever process `id` names and runs `f`
/// against its live `Process`, returning `None` if `id` doesn't name a
/// live process at all. MILESTONE 37: generalized from the original
/// fixed four-way match (PROCESS_A/PROCESS_B/LOADED_PROCESS/
/// FDTEST_PROCESS) to also cover FORK_TEST_PROCESS_ID and, for any
/// `id >= PID_TABLE_BASE`, the real dynamic PROCESS_TABLE -- so every
/// per-process syscall (sbrk/open/read/fdwrite/close) works uniformly
/// for a forked child exactly like it already did for every hardcoded
/// process, with zero special-casing anywhere else in this file.
fn with_process_mut<R>(id: u8, f: impl FnOnce(&mut Process) -> R) -> Option<R> {
    match id {
        1 => {
            let mut guard = PROCESS_A.lock();
            Some(f(guard.as_mut()?))
        }
        2 => {
            let mut guard = PROCESS_B.lock();
            Some(f(guard.as_mut()?))
        }
        LOADED_PROCESS_ID => {
            let mut guard = LOADED_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        FDTEST_PROCESS_ID => {
            let mut guard = FDTEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        FORK_TEST_PROCESS_ID => {
            let mut guard = FORK_TEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        SIGSEGV_TEST_PROCESS_ID => {
            let mut guard = SIGSEGV_TEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        SIGKILL_TEST_PROCESS_ID => {
            let mut guard = SIGKILL_TEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        WAITSTATUS_TEST_PROCESS_ID => {
            let mut guard = WAITSTATUS_TEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        EXEC_TEST_PROCESS_ID => {
            let mut guard = EXEC_TEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        FAULT_TEST_PROCESS_ID => {
            let mut guard = FAULT_TEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        DEMAND_PAGE_TEST_PROCESS_ID => {
            let mut guard = DEMAND_PAGE_TEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        DEMAND_PAGE_OOB_TEST_PROCESS_ID => {
            let mut guard = DEMAND_PAGE_OOB_TEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        ERRNO_TEST_PROCESS_ID => {
            let mut guard = ERRNO_TEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        SIGNAL_TEST_PROCESS_ID => {
            let mut guard = SIGNAL_TEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        MMAP_READ_TEST_PROCESS_ID => {
            let mut guard = MMAP_READ_TEST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        MMAP_WRITE_BEFORE_READ_FAULT_PROCESS_ID => {
            let mut guard = MMAP_WRITE_BEFORE_READ_FAULT_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        MMAP_WRITE_AFTER_READ_FAULT_PROCESS_ID => {
            let mut guard = MMAP_WRITE_AFTER_READ_FAULT_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        MMAP_USE_AFTER_UNMAP_FAULT_PROCESS_ID => {
            let mut guard = MMAP_USE_AFTER_UNMAP_FAULT_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        MMAP_WRITABLE_READ_THEN_WRITE_PROCESS_ID => {
            let mut guard = MMAP_WRITABLE_READ_THEN_WRITE_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        MMAP_WRITABLE_WRITE_FIRST_PROCESS_ID => {
            let mut guard = MMAP_WRITABLE_WRITE_FIRST_PROCESS.lock();
            Some(f(guard.as_mut()?))
        }
        id if id >= PID_TABLE_BASE && ((id - PID_TABLE_BASE) as usize) < MAX_PROCESSES => {
            let idx = (id - PID_TABLE_BASE) as usize;
            let mut guard = PROCESS_TABLE.lock();
            Some(f(guard[idx].as_mut()?))
        }
        _ => None,
    }
}

/// MILESTONE 59: sets process `id`'s own real, sticky `errno` field --
/// the ONLY writer of `Process::errno` anywhere in this kernel.
/// `usertest.rs`'s `syscall_dispatch` calls this on every syscall failure
/// path that has a real, distinguishable errno.rs constant to report (see
/// that file's own per-arm comments), immediately before setting the
/// syscall's own bare return-value sentinel (`u64::MAX`/`0`/`1`) --
/// exactly like a real libc's `errno` is set alongside (not instead of) a
/// syscall's own `-1`/`NULL` return. A no-op (silently) if `id` doesn't
/// name a live process -- the ONE real caller of this with a possibly-
/// stale `id` is `syscall_dispatch`'s own `active` value, which is only
/// ever nonzero while a real process.rs-owned process is genuinely
/// running, so this should never actually miss in practice; still handled
/// honestly rather than assumed.
pub(crate) fn set_errno(id: u8, value: u64) {
    with_process_mut(id, |p| p.errno = value);
}

/// MILESTONE 59: reads process `id`'s own current `errno` value -- real
/// POSIX semantics, same as a real libc's `errno`: whatever the LAST
/// `set_errno()` call for this process wrote, still there even if a later
/// syscall SUCCEEDED in between (this kernel's own syscalls never clear
/// errno on success, exactly like a real libc never does either -- only a
/// later FAILURE overwrites it). `0` if `id` names a live process that has
/// never failed a syscall with a real errno.rs-covered cause yet (the same
/// "0 means no error recorded" sentinel `Process::errno`'s own doc comment
/// documents), OR if `id` doesn't name a live process at all -- the two
/// are intentionally not distinguished here (a caller with no live process
/// to query has nothing more meaningful to report than "no error", the
/// same honest fallback `getpgid()`'s own None case already uses
/// elsewhere in this file for an unresolvable id). Backs syscall 17
/// (GETERRNO) in `usertest.rs`.
pub(crate) fn get_errno(id: u8) -> u64 {
    with_process_mut(id, |p| p.errno).unwrap_or(0)
}

/// MILESTONE 60: same real authorization rule `setpgid()` already
/// established (self, or a live child of the caller) -- reused here
/// rather than shared as a helper `setpgid()` itself also calls, since
/// that function's own check is inlined directly into its `authorized`
/// let-binding, not already factored out; duplicating this small, already-
/// working shape carries far less regression risk than refactoring a
/// working, previously-verified function for this milestone's sake.
fn is_self_or_live_child(caller_id: u8, target_pid: u8) -> bool {
    if target_pid == caller_id {
        return true;
    }
    if target_pid >= PID_TABLE_BASE && ((target_pid - PID_TABLE_BASE) as usize) < MAX_PROCESSES {
        let table = PROCESS_TABLE.lock();
        let idx = (target_pid - PID_TABLE_BASE) as usize;
        return matches!(table[idx].as_ref(), Some(child) if child.parent_pid == Some(caller_id));
    }
    false
}

/// MILESTONE 60: real signal-handler registration -- backs syscall 18
/// (SIGACTION). Real POSIX rule actually enforced: `signum ==
/// signal::SIGKILL` is refused outright (SIGKILL can never be caught or
/// ignored) -- see `signal::SIGKILL`'s own doc comment. `handler == 0`
/// is a real, legal request too: it explicitly CLEARS any previously
/// registered handler back to `SIG_DFL`, the same "0 means no handler"
/// meaning `Process::signal_handlers`'s own doc comment already gives
/// that value everywhere else. No authorization check -- a process may
/// only ever register a handler for ITSELF (`id` here is always
/// `ACTIVE_PROCESS`, the caller's own pid, from `usertest.rs`'s
/// dispatch -- there is no "register a handler on someone else's
/// behalf" call shape in this syscall at all, unlike `raise_signal()`
/// below).
pub(crate) fn sigaction(id: u8, signum: u8, handler: u64) -> Result<(), &'static str> {
    if signum == 0 || signum as usize >= signal::NSIG {
        return Err("signum out of range");
    }
    if signum == signal::SIGKILL {
        return Err("SIGKILL cannot be caught or ignored");
    }
    with_process_mut(id, |p| p.signal_handlers[signum as usize] = handler).ok_or("no such process")
}

/// MILESTONE 60: real signal raising -- backs syscall 19 (SIGSEND).
/// `signum == signal::SIGKILL` is deliberately routed to the
/// PRE-EXISTING Milestone 41 `kill()` path instead of this milestone's
/// new pending-signal/handler-dispatch mechanism (SIGKILL was already
/// real; this just gives callers a single syscall that does the real
/// thing for either case, matching real `kill(2)`'s own unified
/// interface). Authorization: same real rule `setpgid()` already
/// established (self, or a live child of the caller) -- self-signaling
/// (real POSIX `raise()`) always works; signaling an unrelated process
/// does not, this kernel having no uid/permission model yet (disclosed,
/// same real scope-cut `setpgid()`'s own doc comment already names).
///
/// **Real, honest, disclosed scope cut for THIS milestone specifically**:
/// unlike a real kernel, this one does NOT implement any default
/// signal disposition (real POSIX: most signals default-terminate the
/// target if it has no handler registered; a few default-ignore). A
/// signal sent to a process with no handler registered for `signum` is
/// therefore a real, documented no-op here (`Err`, not a silently
/// pretended delivery) -- building a full default-action table is real,
/// legitimately separate future work (would need its own per-signal
/// dispositions, not just this milestone's "catch it for real" slice).
pub(crate) fn raise_signal(caller_id: u8, target_pid: u8, signum: u8) -> Result<(), &'static str> {
    if signum == 0 || signum as usize >= signal::NSIG {
        return Err("signum out of range");
    }
    if signum == signal::SIGKILL {
        return if kill(caller_id, target_pid) {
            Ok(())
        } else {
            Err("SIGKILL delivery failed (see kill()'s own authorization rule -- target must be a live child)")
        };
    }
    if !is_self_or_live_child(caller_id, target_pid) {
        return Err("not authorized -- target is neither the caller itself nor a live child of it");
    }
    let handler_registered = with_process_mut(target_pid, |p| p.signal_handlers[signum as usize] != 0).ok_or("no such process")?;
    if !handler_registered {
        return Err("no handler registered for this signal -- no default-disposition table implemented yet, real no-op");
    }
    with_process_mut(target_pid, |p| p.pending_signal = Some(signum)).ok_or("no such process")?;
    Ok(())
}

/// MILESTONE 60: called from `usertest.rs`'s `syscall_dispatch`, once,
/// immediately before EVERY syscall's return-to-userspace (skipped only
/// when `active == 0`, the legacy bare `usertest` excursion with no
/// `Process` struct to hold signal state in the first place -- and
/// naturally never reached at all for syscall 1/exit, which diverges
/// straight into `resume_kernel()` and never returns to that check
/// point). This is the real, honest "kernel/user boundary" this
/// synchronous, one-ring-3-excursion-at-a-time kernel actually has --
/// there is no preemptive mid-instruction redelivery here (a timer
/// interrupt during ring-3 execution returns to the SAME interrupted
/// point via the CPU's own ordinary `iretq`, not through this dispatch
/// path at all), so checking here, on every real transition back into
/// ring 3, is the correct and complete delivery point for this design,
/// not a partial stand-in for one.
///
/// Returns `Some((signum, handler_addr))` -- and, as a side effect,
/// CONSUMES (clears) `pending_signal` -- exactly when `id` has a real
/// pending signal AND is not already executing inside a still-unresumed
/// handler (see `Process::in_signal_handler`'s own doc comment for why
/// ALL further delivery is blocked, not just the same signal number,
/// while one is in flight). `None` otherwise, including the defensive
/// (should-be-unreachable, `raise_signal()` already checks this before
/// ever setting `pending_signal`) case where the registered handler was
/// somehow cleared back to `SIG_DFL` in between raise and delivery.
pub(crate) fn take_deliverable_signal(id: u8) -> Option<(u8, u64)> {
    with_process_mut(id, |p| {
        if p.in_signal_handler.is_some() {
            return None;
        }
        let signum = p.pending_signal.take()?;
        let handler = p.signal_handlers[signum as usize];
        if handler == 0 {
            return None;
        }
        Some((signum, handler))
    })
    .flatten()
}

/// MILESTONE 60: stashes the real, hardware-interrupted context
/// `usertest.rs` is ABOUT to overwrite (`rip`/`rdi`/`rsp`, to redirect
/// into the handler `take_deliverable_signal()` just named) -- see
/// `SavedSignalContext`'s own doc comment for exactly what this is and
/// its own disclosed kernel-side-stash scope cut. The ONLY writer of
/// `Process::in_signal_handler`'s `Some` state.
pub(crate) fn stash_signal_context(id: u8, ctx: SavedSignalContext) {
    with_process_mut(id, |p| p.in_signal_handler = Some(ctx));
}

/// MILESTONE 60: real `sigreturn` -- backs syscall 20 (SIGRETURN).
/// `.take()`s (clears) `Process::in_signal_handler`, handing the saved
/// context back to `usertest.rs` so it can restore every field of the
/// live `SyscallRegs` verbatim, resuming the interrupted execution
/// exactly where the signal preempted it. `None` if `id` is not
/// currently inside a handler this kernel itself dispatched -- a stray
/// `SIGRETURN` call with nothing to unwind (real POSIX: undefined
/// behavior; this kernel just refuses it honestly -- see this syscall's
/// own dispatch arm in `usertest.rs` for what happens to the caller in
/// that case) rather than restoring garbage.
pub(crate) fn take_saved_signal_context(id: u8) -> Option<SavedSignalContext> {
    with_process_mut(id, |p| p.in_signal_handler.take()).flatten()
}

/// MILESTONE 54: walks and frees the PRIVATE P3/P2/P1 page-table frames
/// backing `pml4_frame`'s own USER_CODE_ADDR-range mapping -- the
/// intermediate frames `map_to()` allocates on demand while building a
/// process's address space (see create_process_from_image()'s own
/// `map_to()` calls, which pass the SAME frame_allocator through for
/// exactly this reason). `reclaim_process_frames()`'s first version only
/// freed the LEAF data frames (pml4/code/stack/heap/extra) and missed
/// these entirely -- found via an actual QEMU boot, not by inspection:
/// this milestone's own real-physical-reuse self-test failed on its
/// first honest run because a second fork() needed more real frames
/// than the leaf-only version had freed, and the missing count matched
/// exactly one P3 + one P2 + one P1 table.
///
/// Safe to walk and free EVERY present entry found this way: only
/// `user_p4_index` (a single PML4 slot -- see create_process_from_image()'s
/// own doc comment on the "share the ENTRY, not the hierarchy" design)
/// is ever populated by process-private `map_to()` calls in this kernel.
/// Every other PML4 slot is a raw copy of the KERNEL's own PML4 entry,
/// pointing at page-table frames the kernel owns permanently -- this
/// function only ever reads `pml4[user_p4_index]` and walks downward
/// from there, so it can never reach (let alone free) a kernel-owned
/// table. Never frees the LEAF data frames the P1 entries point at
/// (those are code_frame/stack_frame/heap_frames/extra_frames, freed
/// separately by their own tracked fields in `reclaim_process_frames()`
/// below) -- only the P3/P2/P1 TABLE frames themselves. No huge pages
/// exist anywhere in this kernel (every `map_to()` call in this file
/// uses `Size4KiB`), so a PRESENT P2 entry is always a real P1 table,
/// never a 2MB leaf mapping.
fn reclaim_private_page_tables(pml4_frame: PhysFrame<Size4KiB>, phys_mem_offset: VirtAddr, fa: &mut memory::BootInfoFrameAllocator) -> usize {
    let user_p4_index = VirtAddr::new(usertest::USER_CODE_ADDR).p4_index();
    let pml4_ptr: *const PageTable = (phys_mem_offset + pml4_frame.start_address().as_u64()).as_ptr();
    let pml4: &PageTable = unsafe { &*pml4_ptr };
    let p3_frame = match pml4[user_p4_index].frame() {
        Ok(f) => f,
        Err(_) => return 0,
    };

    let mut freed = 0usize;
    let p3_ptr: *const PageTable = (phys_mem_offset + p3_frame.start_address().as_u64()).as_ptr();
    let p3: &PageTable = unsafe { &*p3_ptr };
    for p3_entry in p3.iter() {
        let p2_frame = match p3_entry.frame() {
            Ok(f) => f,
            Err(_) => continue,
        };
        let p2_ptr: *const PageTable = (phys_mem_offset + p2_frame.start_address().as_u64()).as_ptr();
        let p2: &PageTable = unsafe { &*p2_ptr };
        for p2_entry in p2.iter() {
            if let Ok(p1_frame) = p2_entry.frame() {
                unsafe { fa.deallocate_frame(p1_frame) };
                freed += 1;
            }
        }
        unsafe { fa.deallocate_frame(p2_frame) };
        freed += 1;
    }
    unsafe { fa.deallocate_frame(p3_frame) };
    freed += 1;
    freed
}

/// MILESTONE 54: read-only twin of `reclaim_private_page_tables()` above
/// -- counts the same PRESENT P3/P2/P1 entries without freeing anything.
/// Exists ONLY for `self_test_frame_reclaim()`'s own independent
/// verification: the self-test needs to predict the TRUE expected
/// free-list delta before a kill/reap happens, and computing that by
/// calling the exact same code path being tested would prove nothing
/// (a bug shared by both the walker and its own prediction would still
/// "match"). Deliberately kept structurally identical to the real
/// walker's traversal logic (same PRESENT check, same three-level walk)
/// -- only what happens at each node differs (count vs. free).
fn count_private_page_tables(pml4_frame: PhysFrame<Size4KiB>, phys_mem_offset: VirtAddr) -> usize {
    let user_p4_index = VirtAddr::new(usertest::USER_CODE_ADDR).p4_index();
    let pml4_ptr: *const PageTable = (phys_mem_offset + pml4_frame.start_address().as_u64()).as_ptr();
    let pml4: &PageTable = unsafe { &*pml4_ptr };
    let p3_frame = match pml4[user_p4_index].frame() {
        Ok(f) => f,
        Err(_) => return 0,
    };

    let mut count = 0usize;
    let p3_ptr: *const PageTable = (phys_mem_offset + p3_frame.start_address().as_u64()).as_ptr();
    let p3: &PageTable = unsafe { &*p3_ptr };
    for p3_entry in p3.iter() {
        let p2_frame = match p3_entry.frame() {
            Ok(f) => f,
            Err(_) => continue,
        };
        let p2_ptr: *const PageTable = (phys_mem_offset + p2_frame.start_address().as_u64()).as_ptr();
        let p2: &PageTable = unsafe { &*p2_ptr };
        for p2_entry in p2.iter() {
            if p2_entry.frame().is_ok() {
                count += 1;
            }
        }
        count += 1;
    }
    count + 1
}

/// MILESTONE 54: real physical frame reclamation -- returns a dead
/// process's PML4/code/stack/heap/extra frames to the global frame
/// allocator's free list (see memory.rs's `BootInfoFrameAllocator`),
/// closing the gap `kill()`/`wait_for_child()`/`exec_elf()` have each
/// disclosed since Milestones 37/41/45: this kernel's frame allocator
/// used to only ever bump-allocate, never free, so every dead process's
/// frames were permanently abandoned (not corrupted, just never
/// reusable again) for the rest of the boot. `heap_frames` and
/// `extra_frames` existed as real, tracked per-process state since
/// Milestones 33/36 specifically so a future reclaim path would have
/// something to free -- this is that path, finally reading them back
/// out instead of only ever writing them.
///
/// Takes `p` by value (full ownership), not `&Process` -- the whole
/// point is that NOTHING else can still be holding a reference to these
/// frames once this runs, and taking ownership is what makes that a
/// compile-time guarantee rather than a caller convention to trust.
/// Safe to call unconditionally: every one of these frames was
/// allocated through `memory::with_frame_allocator()` in the first
/// place (see `create_process_from_image()`/`create_process_from_elf()`),
/// so returning them to that same allocator is always well-formed.
fn reclaim_process_frames(p: Process) {
    let phys_mem_offset = memory::phys_mem_offset();
    let freed = memory::with_frame_allocator(|fa| {
        let mut n = reclaim_private_page_tables(p.pml4_frame, phys_mem_offset, fa);
        unsafe {
            fa.deallocate_frame(p.pml4_frame);
            fa.deallocate_frame(p.code_frame);
            fa.deallocate_frame(p.stack_frame);
        }
        n += 3;
        // MILESTONE 100: the extra stack pages below USER_STACK_ADDR --
        // always fully populated (mapped eagerly at creation), so unlike
        // heap_frames/extra_frames there is nothing sparse to skip.
        for frame in p.extra_stack_frames.iter().copied() {
            unsafe { fa.deallocate_frame(frame) };
            n += 1;
        }
        // MILESTONE 57: heap_frames is now sparse (`Option` per slot,
        // `None` for a never-demand-paged page) -- only free the ones
        // that actually got a real physical frame mapped in.
        for frame in p.heap_frames.iter().flatten().copied() {
            unsafe { fa.deallocate_frame(frame) };
            n += 1;
        }
        for (_vaddr, frame) in p.extra_frames.iter().copied() {
            unsafe { fa.deallocate_frame(frame) };
            n += 1;
        }
        // MILESTONE 64: mmap frames are sparse exactly like heap_frames
        // (`Option` per slot, `None` for a reserved-but-never-demand-
        // paged mmap region) -- same "only free the ones that actually
        // got a real physical frame mapped in" reasoning.
        for frame in p.mmaps.iter().flatten().filter_map(|slot| slot.frame) {
            unsafe { fa.deallocate_frame(frame) };
            n += 1;
        }
        n
    });
    let _ = writeln!(
        serial(),
        "milestone 54: reclaim_process_frames('{}') -- freed {} physical frame(s) back to the allocator's free list",
        p.label,
        freed.unwrap_or(0)
    );
}

/// MILESTONE 45: like `with_process_mut()` above, but replaces the
/// WHOLE `Process` value at `id`'s slot instead of mutating the existing
/// one in place -- `exec_elf()`'s own real teardown-and-rebuild step
/// needs this (a genuinely different PML4/code/stack/heap, not a field
/// tweak on the old one), so it needs the identical id-to-slot dispatch
/// `with_process_mut()` already established, just returning ownership of
/// the slot instead of a `&mut` into it. Returns `true` if `id` named a
/// real slot (occupied or not -- exec() is only ever called on an
/// ALREADY-live, currently-running process, so every real caller's slot
/// is occupied; this still returns `true` for an empty slot rather than
/// treating "process not yet initialized" as "no such process", the same
/// distinction `with_process_mut()` makes by returning `Some(f(..))` only
/// for an occupied slot), `false` if `id` doesn't name any real slot at
/// all (mirrors `with_process_mut()` returning `None` for an unknown id).
fn replace_process(id: u8, new_proc: Process) -> bool {
    // MILESTONE 54: `.replace()` (not a plain assignment) so the OLD
    // value is captured rather than silently dropped -- exec()'s own
    // "dropped, never reclaimed" limitation, disclosed on exec_elf()'s
    // own doc comment, ends here: the old process's frames are real
    // reusable memory again the instant this returns.
    let old = match id {
        1 => PROCESS_A.lock().replace(new_proc),
        2 => PROCESS_B.lock().replace(new_proc),
        LOADED_PROCESS_ID => LOADED_PROCESS.lock().replace(new_proc),
        FDTEST_PROCESS_ID => FDTEST_PROCESS.lock().replace(new_proc),
        FORK_TEST_PROCESS_ID => FORK_TEST_PROCESS.lock().replace(new_proc),
        SIGSEGV_TEST_PROCESS_ID => SIGSEGV_TEST_PROCESS.lock().replace(new_proc),
        SIGKILL_TEST_PROCESS_ID => SIGKILL_TEST_PROCESS.lock().replace(new_proc),
        WAITSTATUS_TEST_PROCESS_ID => WAITSTATUS_TEST_PROCESS.lock().replace(new_proc),
        EXEC_TEST_PROCESS_ID => EXEC_TEST_PROCESS.lock().replace(new_proc),
        // Note: FAULT_TEST_PROCESS_ID (10) was already, pre-existingly,
        // never listed here either (only ever used as a fork() source,
        // never exec()'d directly) -- left as-is, not this milestone's
        // gap to fix. DEMAND_PAGE_TEST_PROCESS_ID/DEMAND_PAGE_OOB_TEST_
        // PROCESS_ID are added for the same reason every OTHER
        // hardcoded-process id is: consistency with with_process_mut()'s
        // own dispatch, even though neither is ever exec()'d by this
        // milestone's own self-test.
        DEMAND_PAGE_TEST_PROCESS_ID => DEMAND_PAGE_TEST_PROCESS.lock().replace(new_proc),
        DEMAND_PAGE_OOB_TEST_PROCESS_ID => DEMAND_PAGE_OOB_TEST_PROCESS.lock().replace(new_proc),
        // MILESTONE 59: ERRNO_TEST_PROCESS_ID added for the same
        // consistency-with-with_process_mut() reason as the two entries
        // just above, even though it's never exec()'d by this
        // milestone's own self-test either.
        ERRNO_TEST_PROCESS_ID => ERRNO_TEST_PROCESS.lock().replace(new_proc),
        // MILESTONE 60: same consistency-with-with_process_mut() reason
        // as ERRNO_TEST_PROCESS_ID just above.
        SIGNAL_TEST_PROCESS_ID => SIGNAL_TEST_PROCESS.lock().replace(new_proc),
        // MILESTONE 64: same consistency-with-with_process_mut() reason as
        // every entry just above -- none of the four mmap test processes
        // is ever exec()'d by this milestone's own self-test either.
        MMAP_READ_TEST_PROCESS_ID => MMAP_READ_TEST_PROCESS.lock().replace(new_proc),
        MMAP_WRITE_BEFORE_READ_FAULT_PROCESS_ID => MMAP_WRITE_BEFORE_READ_FAULT_PROCESS.lock().replace(new_proc),
        MMAP_WRITE_AFTER_READ_FAULT_PROCESS_ID => MMAP_WRITE_AFTER_READ_FAULT_PROCESS.lock().replace(new_proc),
        MMAP_USE_AFTER_UNMAP_FAULT_PROCESS_ID => MMAP_USE_AFTER_UNMAP_FAULT_PROCESS.lock().replace(new_proc),
        // MILESTONE 65: same consistency-with-with_process_mut() reason as
        // every MMAP_* entry just above -- neither of these two new
        // writable-mmap test processes is ever exec()'d by this
        // milestone's own self-test either.
        MMAP_WRITABLE_READ_THEN_WRITE_PROCESS_ID => MMAP_WRITABLE_READ_THEN_WRITE_PROCESS.lock().replace(new_proc),
        MMAP_WRITABLE_WRITE_FIRST_PROCESS_ID => MMAP_WRITABLE_WRITE_FIRST_PROCESS.lock().replace(new_proc),
        id if id >= PID_TABLE_BASE && ((id - PID_TABLE_BASE) as usize) < MAX_PROCESSES => {
            let idx = (id - PID_TABLE_BASE) as usize;
            PROCESS_TABLE.lock()[idx].replace(new_proc)
        }
        _ => return false,
    };
    if let Some(old_proc) = old {
        reclaim_process_frames(old_proc);
    }
    true
}

/// MILESTONE 35: the `open` syscall's kernel-side implementation. `path`
/// has already been copied out of the calling process's own address
/// space by usertest.rs's syscall_dispatch (same read-through-current-
/// CR3 technique the write syscall established). Tries fs::read_file()
/// first; a path that already exists gets its real on-disk contents
/// buffered in (see OpenFile's own doc comment for why buffering the
/// whole file is honest here). A path that does NOT exist is not treated
/// as an error -- this syscall ABI has no separate O_CREAT/O_RDONLY flag
/// argument (deliberately kept minimal, matching sbrk's own single-
/// argument simplicity), so open() always succeeds against a well-formed
/// path/fd-table state and starts an empty buffer for a not-yet-existing
/// file, exactly like a real `open(path, O_RDWR|O_CREAT)` would -- the
/// file only actually comes into existence on disk if something is
/// later fdwrite()'n and the fd is close()'d (see close_fd()). Returns
/// the new fd (an index into the process's own fds table) or `None` if
/// the table is already full (MAX_OPEN_FILES) or `id` doesn't name a
/// live process.
/// MILESTONE 82: `truncate` (real O_TRUNC semantics) -- when true, the
/// in-memory buffer starts genuinely EMPTY regardless of whatever this
/// path already holds on disk, `fs::read_file()` is never even called
/// (real, not a masked read). `dirty` is set `true` immediately in this
/// case, not left for a later fdwrite() to set -- real O_TRUNC
/// truncates the on-disk file the moment it's opened, even if this fd
/// is closed with zero writes ever issued; without forcing `dirty`
/// here, close_fd()'s own dirty-gated persist would skip writing back
/// at all when nothing was ever fdwrite()'n, silently leaving the OLD,
/// pre-truncate content on disk -- the exact opposite of what a
/// truncating open means, checked against this before relying on it,
/// not assumed safe. `truncate=false` is byte-for-byte the original
/// Milestone 35 behavior, unchanged -- every existing caller of the
/// plain (non-truncating) open path is unaffected.
pub(crate) fn open_file(id: u8, path: &str, truncate: bool) -> Option<u64> {
    let (buffer, dirty) = if truncate {
        (Vec::new(), true)
    } else {
        match fs::read_file(path) {
            Ok(data) => (data, false),
            Err(_) => (Vec::new(), false), // honest "doesn't exist yet" case, not an error -- see doc comment above
        }
    };
    let path_owned = path.to_string();
    with_process_mut(id, move |proc| {
        let slot_index = proc.fds.iter().position(|f| f.is_none())?;
        proc.fds[slot_index] = Some(FdEntry::File(OpenFile { path: path_owned, buffer, pos: 0, dirty }));
        Some(slot_index as u64)
    })?
}

/// MILESTONE 35: the `read` syscall's kernel-side implementation. Copies
/// up to `len` bytes starting at this fd's current cursor `pos` out of
/// its already-buffered contents (see OpenFile's doc comment -- no disk
/// access happens here, the buffering already happened at open() time),
/// advances `pos` by however many bytes were actually available (0 at
/// EOF, which is a real, valid, non-error result -- the caller's cap on
/// `len` was already applied by usertest.rs before this is called).
/// Returns `None` (distinct from a legitimate `Some(vec![])` at EOF) if
/// `fd` doesn't name a currently-open descriptor for this process, or
/// `id` doesn't name a live process.
/// MILESTONE 40: now also the entry point for pipe reads -- checks
/// whether `fd` names a File or a PipeRead end and dispatches to
/// pipe_read() for the latter (which takes its own lock on PIPE_TABLE,
/// separate from this function's `with_process_mut`, so the two locks
/// are never held nested/at once). A PipeWrite fd, or any `fd` that
/// simply isn't open, both correctly fall through to `None` -- reading
/// from a pipe's WRITE end is exactly as invalid as reading a closed fd.
pub(crate) fn read_fd(id: u8, fd: u64, len: usize) -> Option<Vec<u8>> {
    let is_pipe_read = with_process_mut(id, |proc| matches!(proc.fds.get(fd as usize), Some(Some(FdEntry::PipeRead(_)))))?;
    if is_pipe_read {
        return pipe_read(id, fd, len);
    }
    with_process_mut(id, |proc| {
        let entry = match proc.fds.get_mut(fd as usize)?.as_mut()? {
            FdEntry::File(f) => f,
            _ => return None,
        };
        let start = entry.pos.min(entry.buffer.len());
        let end = (start + len).min(entry.buffer.len());
        let out = entry.buffer[start..end].to_vec();
        entry.pos = end;
        Some(out)
    })?
}

/// MILESTONE 35: the `fdwrite` syscall's kernel-side implementation.
/// `data` has already been copied out of the calling process's own
/// address space (same technique open()'s `path` used). Writes `data`
/// into this fd's buffer starting at its current cursor `pos`, extending
/// the buffer as needed but never past fs::MAX_FILE_BYTES (the SAME cap
/// fs::write_file() itself enforces -- reused directly, not a second,
/// potentially-drifting copy of "4096"), truncating a write that would
/// overflow it and returning the ACTUAL number of bytes accepted (which
/// callers must check -- a real, honest partial-write result, the same
/// "don't silently drop data without saying so" discipline
/// usertest.rs's own MAX_WRITE_LEN already established for syscall 0).
/// Marks the fd dirty so close_fd() below knows to persist it. Returns
/// `None` if `fd`/`id` don't name a currently-open descriptor.
/// MILESTONE 40: now also the entry point for pipe writes -- same
/// dispatch pattern as read_fd() above, see its comment for why the two
/// locks (per-process, PIPE_TABLE) are never nested.
pub(crate) fn write_fd(id: u8, fd: u64, data: &[u8]) -> Option<usize> {
    let is_pipe_write = with_process_mut(id, |proc| matches!(proc.fds.get(fd as usize), Some(Some(FdEntry::PipeWrite(_)))))?;
    if is_pipe_write {
        return pipe_write(id, fd, data);
    }
    with_process_mut(id, |proc| {
        let entry = match proc.fds.get_mut(fd as usize)?.as_mut()? {
            FdEntry::File(f) => f,
            _ => return None,
        };
        let start = entry.pos;
        let room = fs::MAX_FILE_BYTES.saturating_sub(start);
        let n = data.len().min(room);
        if entry.buffer.len() < start + n {
            entry.buffer.resize(start + n, 0);
        }
        entry.buffer[start..start + n].copy_from_slice(&data[..n]);
        entry.pos = start + n;
        entry.dirty = true;
        Some(n)
    })?
}

/// MILESTONE 35: the `close` syscall's kernel-side implementation --
/// releases this process's fd table slot (`Some(_)` -> `None`, freeing
/// it for a later open() to reuse) and, if fdwrite() ever actually
/// touched this fd's buffer (`dirty`), persists it for real via
/// fs::write_file(path, &buffer) -- the SAME on-disk write path the
/// shell's own `write` command already uses, so a `read PATH` shell
/// command run afterward sees genuinely identical bytes. This is the
/// ONLY point in this milestone's whole design where a write actually
/// reaches disk -- see OpenFile's own doc comment for why that timing
/// (close-only, no separate flush/fsync) is disclosed as a real
/// limitation, not hidden. Returns `Some(true)` on success (persisted,
/// or nothing needed persisting), `Some(false)` if fs::write_file()
/// itself failed (fd is still released either way -- a failed persist
/// doesn't leak the table slot), or `None` if `fd`/`id` didn't name a
/// currently-open descriptor at all.
pub(crate) fn close_fd(id: u8, fd: u64) -> Option<bool> {
    let entry = with_process_mut(id, |proc| {
        let slot = proc.fds.get_mut(fd as usize)?;
        slot.take()
    })??;
    let entry = match entry {
        FdEntry::File(f) => f,
        // MILESTONE 40: closing a pipe end decrements its real ref
        // count (see Pipe's own doc comment for why counting, not
        // unconditional free, is required once fork()/dup() can share
        // an end across processes/fds) and only actually frees the
        // PIPE_TABLE slot once EVERY read end AND EVERY write end
        // anyone ever held has been closed -- a pipe with its write end
        // still open but read end closed, or vice versa, correctly
        // stays allocated.
        FdEntry::PipeRead(idx) => {
            let mut table = PIPE_TABLE.lock();
            if let Some(pipe) = table[idx].as_mut() {
                pipe.read_open = pipe.read_open.saturating_sub(1);
                if pipe.read_open == 0 && pipe.write_open == 0 {
                    table[idx] = None;
                }
            }
            return Some(true);
        }
        FdEntry::PipeWrite(idx) => {
            let mut table = PIPE_TABLE.lock();
            if let Some(pipe) = table[idx].as_mut() {
                pipe.write_open = pipe.write_open.saturating_sub(1);
                if pipe.read_open == 0 && pipe.write_open == 0 {
                    table[idx] = None;
                }
            }
            return Some(true);
        }
    };
    if !entry.dirty {
        return Some(true);
    }
    match fs::write_file(&entry.path, &entry.buffer) {
        Ok(()) => {
            let _ = writeln!(
                serial(),
                "milestone 35: syscall CLOSE (process {id}) -- fd {fd} was dirty, persisted {} bytes to '{}' on the real on-disk filesystem",
                entry.buffer.len(),
                entry.path
            );
            Some(true)
        }
        Err(e) => {
            let _ = writeln!(
                serial(),
                "milestone 35: syscall CLOSE (process {id}) -- fd {fd} was dirty, but fs::write_file('{}') FAILED: {e} -- fd released anyway, data NOT persisted",
                entry.path
            );
            Some(false)
        }
    }
}

/// MILESTONE 40: the `dup` syscall's kernel-side implementation --
/// duplicates `fd` into the LOWEST-numbered free slot in this process's
/// own fd table, returning the new fd number. For a PipeRead/PipeWrite
/// entry this is a REAL, correct dup(): both fd numbers end up pointing
/// at the SAME PIPE_TABLE index (its ref count incremented, see Pipe's
/// own doc comment), genuinely sharing one buffer/cursor pair. For a
/// File entry this is an honest, disclosed simplification, NOT real
/// POSIX dup() semantics -- it deep-copies the OpenFile (own `pos`), the
/// exact same "independent copy, not a shared reference" choice fork()
/// already made for File entries, for the identical reason (this
/// codebase currently has no shared/reference-counted File object to
/// point two fd numbers at, only owned-by-value buffers). Returns `None`
/// if `fd` isn't currently open or the table has no free slot.
pub(crate) fn dup_fd(id: u8, fd: u64) -> Option<u64> {
    let cloned = with_process_mut(id, |proc| proc.fds.get(fd as usize)?.clone())?;
    let cloned = cloned?;
    if let FdEntry::PipeRead(idx) | FdEntry::PipeWrite(idx) = cloned {
        let mut table = PIPE_TABLE.lock();
        if let Some(pipe) = table[idx].as_mut() {
            match cloned {
                FdEntry::PipeRead(_) => pipe.read_open = pipe.read_open.saturating_add(1),
                FdEntry::PipeWrite(_) => pipe.write_open = pipe.write_open.saturating_add(1),
                FdEntry::File(_) => unreachable!(),
            }
        }
    }
    with_process_mut(id, move |proc| {
        let slot_index = proc.fds.iter().position(|f| f.is_none())?;
        proc.fds[slot_index] = Some(cloned);
        Some(slot_index as u64)
    })?
}

/// MILESTONE 40: the `dup2` syscall's kernel-side implementation --
/// same sharing semantics as dup_fd() above (real for pipes, honest
/// deep-copy for files), but into a CALLER-CHOSEN fd number `newfd`
/// rather than the lowest free slot. If `newfd` already names an open
/// descriptor, it is properly closed first (its own ref count
/// decremented / its own dirty buffer persisted, via the SAME
/// close_fd() every other close path uses -- not a silent overwrite
/// that would leak a pipe ref or drop unpersisted file data). A dup2 of
/// an fd onto itself (`fd == newfd`) is a real no-op, matching POSIX,
/// checked explicitly before the close-then-clone sequence so it can't
/// close (and thereby destroy) the very descriptor it's supposed to
/// preserve. Returns `Some(true)` on success, `None` if `fd` isn't open
/// or `newfd` is out of range.
pub(crate) fn dup2_fd(id: u8, fd: u64, newfd: u64) -> Option<bool> {
    if fd == newfd {
        let is_open = with_process_mut(id, |proc| proc.fds.get(fd as usize).map(|f| f.is_some()))??;
        return if is_open { Some(true) } else { None };
    }
    if newfd as usize >= MAX_OPEN_FILES {
        return None;
    }
    let cloned = with_process_mut(id, |proc| proc.fds.get(fd as usize)?.clone())?;
    let cloned = cloned?;
    if with_process_mut(id, |proc| proc.fds.get(newfd as usize).map(|f| f.is_some()))? == Some(true) {
        close_fd(id, newfd)?;
    }
    if let FdEntry::PipeRead(idx) | FdEntry::PipeWrite(idx) = cloned {
        let mut table = PIPE_TABLE.lock();
        if let Some(pipe) = table[idx].as_mut() {
            match cloned {
                FdEntry::PipeRead(_) => pipe.read_open = pipe.read_open.saturating_add(1),
                FdEntry::PipeWrite(_) => pipe.write_open = pipe.write_open.saturating_add(1),
                FdEntry::File(_) => unreachable!(),
            }
        }
    }
    with_process_mut(id, move |proc| {
        proc.fds[newfd as usize] = Some(cloned);
        Some(true)
    })?
}

/// MILESTONE 100: map `usertest::USER_STACK_EXTRA_PAGES` extra stack
/// pages into `process_mapper`, immediately BELOW `USER_STACK_ADDR`
/// (returned frame `i` backs `USER_STACK_ADDR - (i+1)*PAGE_SIZE`). Used
/// by both real process constructors so the "grow the stack down by N
/// pages" change lives in exactly one place. Not zeroed -- matching the
/// single `stack_frame` map above it, which never was either (nothing
/// reads stack memory it hasn't first written). `flags` is the same
/// PRESENT|WRITABLE|USER_ACCESSIBLE every other user page uses.
fn map_extra_stack_pages(
    process_mapper: &mut OffsetPageTable,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    flags: PageTableFlags,
) -> Result<Vec<PhysFrame<Size4KiB>>, &'static str> {
    let mut frames = Vec::with_capacity(usertest::USER_STACK_EXTRA_PAGES as usize);
    for i in 0..usertest::USER_STACK_EXTRA_PAGES {
        let frame = frame_allocator
            .allocate_frame()
            .ok_or("out of physical frames (extra stack)")?;
        let va = usertest::USER_STACK_ADDR - (i + 1) * PAGE_SIZE as u64;
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(va));
        unsafe {
            process_mapper
                .map_to(page, frame, flags, frame_allocator)
                .map_err(|_| "map_to failed (extra stack page)")?
                .flush();
        }
        frames.push(frame);
    }
    Ok(frames)
}

/// MILESTONE 34: the actual page-table-building mechanism, factored out
/// so BOTH the hardcoded PROCESS_A/PROCESS_B path (create_process(),
/// below) and loader.rs's real-file path (create_loaded_process(), the
/// MILESTONE 35 split of what used to be load_and_run_image()) run
/// through identical, unduplicated unsafe mapping code -- the only
/// difference between them is where `image`'s bytes came from. Every
/// process gets code, stack, AND heap pages mapped uniformly here
/// (Milestone 33's heap isn't special-cased to the hardcoded path);
/// `image` becomes the new process's code page content verbatim
/// (zero-padded to a full page) -- rejected up front, before any frame
/// is even allocated, if it's bigger than MAX_CODE_IMAGE_BYTES (one
/// page), the code frame's real capacity.
fn create_process_from_image(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
    label: &'static str,
    image: &[u8],
) -> Result<Process, &'static str> {
    if image.len() > MAX_CODE_IMAGE_BYTES {
        return Err("program image exceeds the 4096-byte code page capacity (one page, no multi-page programs yet)");
    }

    let user_p4_index = VirtAddr::new(usertest::USER_CODE_ADDR).p4_index();
    let stack_p4_index = VirtAddr::new(usertest::USER_STACK_ADDR).p4_index();
    if user_p4_index != stack_p4_index {
        return Err("USER_CODE_ADDR/USER_STACK_ADDR fall in different PML4 indices -- design assumption violated");
    }
    let _ = writeln!(
        serial(),
        "milestone 30: process {label} -- user p4 index computed as {} (code {:#x}, stack {:#x})",
        u16::from(user_p4_index),
        usertest::USER_CODE_ADDR,
        usertest::USER_STACK_ADDR
    );

    let new_pml4_frame = frame_allocator.allocate_frame().ok_or("out of physical frames (pml4)")?;
    let new_pml4_ptr: *mut PageTable = (phys_mem_offset + new_pml4_frame.start_address().as_u64()).as_mut_ptr();
    let new_pml4: &mut PageTable = unsafe { &mut *new_pml4_ptr };
    new_pml4.zero();

    // POST-MILESTONE-65 FIX: was `Cr3::read()` (whatever is CURRENTLY
    // loaded) -- now the canonically-saved kernel PML4, always correct
    // regardless of which process's own PML4 happens to be loaded at
    // this specific call site. See kernel_pml4_for_new_process()'s own
    // doc comment for the real bug this closes.
    let (kernel_pml4_frame, _) = kernel_pml4_for_new_process();
    let kernel_pml4_ptr: *const PageTable = (phys_mem_offset + kernel_pml4_frame.start_address().as_u64()).as_ptr();
    let kernel_pml4: &PageTable = unsafe { &*kernel_pml4_ptr };

    // MILESTONE 30, the core design step: share every kernel-space PML4
    // entry by copying the ENTRY (a pointer to the kernel's existing P3
    // table), NOT the hierarchy underneath it -- only user_p4_index is
    // left zeroed here, so map_to() below gives it a genuinely fresh,
    // private chain instead.
    for i in 0u16..512 {
        let idx = PageTableIndex::new(i);
        if idx != user_p4_index {
            new_pml4[idx] = kernel_pml4[idx].clone();
        }
    }
    let _ = writeln!(
        serial(),
        "milestone 30: process {label} -- new pml4 {:#x} populated: 511 kernel-space entries shared, index {} left private",
        new_pml4_frame.start_address().as_u64(),
        u16::from(user_p4_index)
    );

    let mut process_mapper = unsafe { OffsetPageTable::new(new_pml4, phys_mem_offset) };

    let code_frame = frame_allocator.allocate_frame().ok_or("out of physical frames (code)")?;
    let stack_frame = frame_allocator.allocate_frame().ok_or("out of physical frames (stack)")?;
    let code_page = Page::<Size4KiB>::containing_address(VirtAddr::new(usertest::USER_CODE_ADDR));
    let stack_page = Page::<Size4KiB>::containing_address(VirtAddr::new(usertest::USER_STACK_ADDR));
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

    unsafe {
        process_mapper
            .map_to(code_page, code_frame, flags, frame_allocator)
            .map_err(|_| "map_to failed (code page)")?
            .flush();
        process_mapper
            .map_to(stack_page, stack_frame, flags, frame_allocator)
            .map_err(|_| "map_to failed (stack page)")?
            .flush();
    }
    let _ = writeln!(
        serial(),
        "milestone 30: process {label} -- private code page mapped to frame {:#x}, private stack page mapped to frame {:#x}",
        code_frame.start_address().as_u64(),
        stack_frame.start_address().as_u64()
    );

    // MILESTONE 100: extra stack pages below USER_STACK_ADDR.
    let extra_stack_frames = map_extra_stack_pages(&mut process_mapper, frame_allocator, flags)?;
    let _ = writeln!(
        serial(),
        "milestone 100: process {label} -- {} extra stack page(s) mapped below USER_STACK_ADDR ({} KiB total stack)",
        extra_stack_frames.len(),
        usertest::USER_STACK_TOTAL_BYTES / 1024
    );

    // MILESTONE 57: the heap's virtual range (HEAP_START..HEAP_START+
    // HEAP_SIZE, same private-P3/P2/P1-chain territory as code/stack
    // above -- HEAP_START shares USER_CODE_ADDR/USER_STACK_ADDR's
    // p4_index (170)) is RESERVED here but genuinely NOT mapped: zero
    // physical frames, zero map_to() calls, zero page-table frames
    // allocated for it at process-creation time. Every one of this
    // process's 64 heap-page slots starts `None` -- a real hardware page
    // fault is what maps a page in, lazily, the first time it's actually
    // touched (see `try_demand_page_heap()`, called from
    // interrupts.rs's page_fault_handler). Before this milestone, this
    // exact spot eagerly allocated+mapped+zeroed every heap page up
    // front, unconditionally, whether `sbrk()` was ever called or not.
    let heap_frames: Vec<Option<PhysFrame<Size4KiB>>> = vec![None; HEAP_PAGE_COUNT as usize];
    let _ = writeln!(
        serial(),
        "milestone 57: process {label} -- private heap RESERVED (not mapped): {} pages at {:#x}..{:#x}, 0 physical frames committed until a real page fault demand-pages one",
        HEAP_PAGE_COUNT,
        HEAP_START,
        HEAP_START + HEAP_SIZE - 1
    );

    // Written through the phys-mem-offset DIRECT view of this process's
    // own physical frame -- deliberately NOT through USER_CODE_ADDR
    // under the currently-loaded (kernel) CR3, which would write into
    // whatever THAT table's user_p4_index chain currently maps (a
    // different physical frame entirely) instead of this new process's
    // own, freshly-mapped one.
    let code_virt = phys_mem_offset + code_frame.start_address().as_u64();
    let code_ptr: *mut u8 = code_virt.as_mut_ptr();
    unsafe {
        core::ptr::write_bytes(code_ptr, 0, PAGE_SIZE);
        core::ptr::copy_nonoverlapping(image.as_ptr(), code_ptr, image.len());
    }
    let _ = writeln!(
        serial(),
        "milestone 34: process {label} -- copied {} image bytes into private code frame {:#x}",
        image.len(),
        code_frame.start_address().as_u64()
    );

    Ok(Process {
        label,
        pml4_frame: new_pml4_frame,
        code_frame,
        stack_frame,
        extra_stack_frames,
        heap_frames,
        heap_used: 0,
        // MILESTONE 35: every process gets a real, empty fd table
        // uniformly -- hardcoded PROCESS_A/PROCESS_B and file-loaded
        // processes alike, same "no path-dependent special-casing"
        // principle create_process_from_image already applies to
        // code/stack/heap above.
        fds: [None, None, None, None],
        // MILESTONE 36: this is create_process_from_image's own
        // constructor (the flat-binary path) -- extra_frames stays
        // empty here always, per its own doc comment above; only
        // create_process_from_elf populates it.
        extra_frames: Vec::new(),
        // MILESTONE 37: create_process_from_image() builds a brand-new,
        // never-forked process along every one of its callers' paths
        // (PROCESS_A/PROCESS_B, FDTEST_PROCESS, FORK_TEST_PROCESS,
        // LOADED_PROCESS, and fork()'s own fork_build_child() helper,
        // which overwrites these two fields itself right after this
        // call returns -- see fork()'s own doc comment) -- both start
        // None here uniformly; only fork() ever sets them.
        parent_pid: None,
        pending_resume: None,
        // MILESTONE 42: real sentinel, see the field's own doc comment --
        // this constructor doesn't know its own future id (assigned by
        // the caller after this returns); fork()'s fork_build_child()
        // caller overwrites this with the real inherited pgid right
        // after construction, same as parent_pid/pending_resume above.
        pgid: 0,
        // MILESTONE 44: every flat-binary/hand-assembled program this
        // kernel has ever run starts at the same fixed address -- see
        // this field's own doc comment on the struct definition.
        entry: usertest::USER_CODE_ADDR,
        // MILESTONE 59: every fresh process (including a freshly forked
        // child -- fork_build_child() goes through this same
        // constructor, see its own doc comment) starts with a clean
        // slate, same real convention as a genuine new process image
        // never having called anything that could have set errno yet.
        errno: 0,
        // MILESTONE 60: every fresh process (including a freshly forked
        // child) starts with NO handlers registered (all SIG_DFL/0), no
        // signal pending, and not inside any handler -- real, honest
        // "clean slate" convention, same as errno just above. Disclosed
        // real gap, not silently done or silently skipped: a genuine
        // fork()'d child does NOT inherit its parent's registered
        // handlers here (real POSIX fork() DOES inherit signal
        // disposition) -- fork_build_child() goes through this exact
        // constructor and never copies `signal_handlers` across from the
        // parent afterward, unlike its own explicit heap/pgid copying.
        // Real future work, not pretended complete.
        signal_handlers: [0; signal::NSIG],
        pending_signal: None,
        in_signal_handler: None,
        // MILESTONE 64: every fresh process (including a freshly forked
        // child -- see `mmaps`' own doc comment for the disclosed
        // fork()-inheritance gap) starts with an empty mmap table, same
        // real "clean slate" convention as errno/signal_handlers above.
        mmaps: [None, None, None, None],
    })
}

/// MILESTONE 30's original entry point, preserved for PROCESS_A/
/// PROCESS_B: builds the PROCESS_PROGRAM-at-offset-0 + patched heap-
/// marker + message-at-MESSAGE_OFFSET layout as one in-memory image
/// (mirroring usertest::write_fixed_message()'s own pad/truncate-to-
/// MESSAGE_LEN behavior by hand, since that helper wants a pointer into
/// an already-mapped page, not a plain Vec), then hands it to
/// create_process_from_image() (MILESTONE 34) -- which no longer cares,
/// and never did semantically, whether those bytes came from this
/// hardcoded array or an arbitrary file.
fn create_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
    label: &'static str,
    message: &str,
    heap_marker: u8,
) -> Result<Process, &'static str> {
    let image_len = (usertest::MESSAGE_OFFSET as usize + usertest::MESSAGE_LEN).max(PROCESS_PROGRAM.len());
    let mut image = vec![0u8; image_len];
    image[..PROCESS_PROGRAM.len()].copy_from_slice(&PROCESS_PROGRAM);
    image[HEAP_MARKER_PATCH_OFFSET] = heap_marker;

    let msg_bytes = message.as_bytes();
    let n = msg_bytes.len().min(usertest::MESSAGE_LEN);
    let start = usertest::MESSAGE_OFFSET as usize;
    for b in image[start..start + usertest::MESSAGE_LEN].iter_mut() {
        *b = b' ';
    }
    image[start..start + n].copy_from_slice(&msg_bytes[..n]);

    create_process_from_image(frame_allocator, phys_mem_offset, label, &image)
}

/// MILESTONE 30: creates process A and process B, each with its own
/// PML4/code frame/stack frame, printing distinct messages through the
/// Milestone 31 write(ptr,len) syscall -- called once at boot, mirroring
/// usertest::setup()'s own once-at-boot pattern, while a live
/// frame_allocator is conveniently already in scope in kernel_main.
pub fn init_test_processes(frame_allocator: &mut impl FrameAllocator<Size4KiB>, phys_mem_offset: VirtAddr) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 30: creating process A's private address space...");
    let mut a = create_process(
        frame_allocator,
        phys_mem_offset,
        "A",
        "hello from process A -- via real write(ptr,len)!",
        b'A',
    )?;
    let _ = writeln!(
        serial(),
        "milestone 30: process A created -- pml4={:#x} code={:#x} stack={:#x}",
        a.pml4_frame.start_address().as_u64(),
        a.code_frame.start_address().as_u64(),
        a.stack_frame.start_address().as_u64()
    );
    // MILESTONE 42: process A is the founder of its own group -- real
    // Unix semantics, pgid == pid for a process nobody explicitly placed
    // into an existing group.
    a.pgid = 1;
    let _ = writeln!(serial(), "milestone 42: process A -- pgid={} (founder of its own group)", a.pgid);
    *PROCESS_A.lock() = Some(a);

    let _ = writeln!(serial(), "milestone 30: creating process B's private address space...");
    let mut b = create_process(
        frame_allocator,
        phys_mem_offset,
        "B",
        "hello from process B -- via real write(ptr,len)!",
        b'B',
    )?;
    let _ = writeln!(
        serial(),
        "milestone 30: process B created -- pml4={:#x} code={:#x} stack={:#x}",
        b.pml4_frame.start_address().as_u64(),
        b.code_frame.start_address().as_u64(),
        b.stack_frame.start_address().as_u64()
    );
    // MILESTONE 42: process B is likewise the founder of its own,
    // separate group -- distinct from process A's, real proof two
    // independently-created processes don't accidentally share a pgid.
    b.pgid = 2;
    let _ = writeln!(serial(), "milestone 42: process B -- pgid={} (founder of its own group)", b.pgid);
    *PROCESS_B.lock() = Some(b);

    Ok(())
}

/// MILESTONE 35: creates FDTEST_PROCESS -- see that static's own doc
/// comment for why this third hardcoded slot exists. Called once at
/// boot from kernel_main, right after init_test_processes(), while the
/// same local frame_allocator/phys_mem_offset are still conveniently in
/// scope -- mirrors init_test_processes()'s own pattern exactly, just
/// building from an arbitrary `image: &[u8]` (loader::FDTEST_PROGRAM)
/// via create_process_from_image() directly, the same core Milestone 34
/// factored out for loader.rs's file-loaded path, rather than going
/// through create_process()'s PROCESS_PROGRAM-specific wrapper.
pub fn init_fdtest_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
    image: &[u8],
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 35: creating FDTEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "fdtest", image)?;
    let _ = writeln!(
        serial(),
        "milestone 35: FDTEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    // MILESTONE 42: founder of its own group, same reasoning as A/B.
    p.pgid = FDTEST_PROCESS_ID;
    *FDTEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 37: creates FORK_TEST_PROCESS -- see that static's own doc
/// comment. Called once at boot from kernel_main, mirroring
/// init_fdtest_process()'s own pattern exactly, from FORK_TEST_PROGRAM
/// (this module's own fork/exec/wait test payload).
pub fn init_fork_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 37: creating FORK_TEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "fork-test", &FORK_TEST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 37: FORK_TEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    // MILESTONE 42: founder of its own group -- also the process
    // self_test_process_groups() below actually forks from, to prove
    // real pgid inheritance.
    p.pgid = FORK_TEST_PROCESS_ID;
    *FORK_TEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 41: creates SIGSEGV_TEST_PROCESS -- see that static's own
/// doc comment. Mirrors init_fork_test_process()'s exact pattern.
pub fn init_sigsegv_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 41: creating SIGSEGV_TEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "sigsegv-test", &SIGSEGV_TEST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 41: SIGSEGV_TEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    // MILESTONE 42: founder of its own group, same reasoning as A/B.
    p.pgid = SIGSEGV_TEST_PROCESS_ID;
    *SIGSEGV_TEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 41: creates SIGKILL_TEST_PROCESS -- see that static's own
/// doc comment. Mirrors init_fork_test_process()'s exact pattern.
pub fn init_sigkill_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 41: creating SIGKILL_TEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "sigkill-test", &SIGKILL_TEST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 41: SIGKILL_TEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    // MILESTONE 42: founder of its own group, same reasoning as A/B.
    p.pgid = SIGKILL_TEST_PROCESS_ID;
    *SIGKILL_TEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 43: creates WAITSTATUS_TEST_PROCESS -- see that static's
/// own doc comment. Mirrors init_sigkill_test_process()'s exact
/// pattern. This process is never `run()` directly as a top-level
/// process by the self-test -- it exists purely as a fork() SOURCE,
/// so its own code page (a copy of WAITSTATUS_TEST_PROGRAM) is what a
/// forked CHILD actually executes.
pub fn init_waitstatus_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 43: creating WAITSTATUS_TEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "waitstatus-test", &WAITSTATUS_TEST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 43: WAITSTATUS_TEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    // MILESTONE 42: founder of its own group, same reasoning as A/B.
    p.pgid = WAITSTATUS_TEST_PROCESS_ID;
    *WAITSTATUS_TEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 45: creates EXEC_TEST_PROCESS -- see that static's own doc
/// comment. Mirrors init_waitstatus_test_process()'s exact pattern.
pub fn init_exec_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 45: creating EXEC_TEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "exec-test", &EXEC_TEST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 45: EXEC_TEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    // MILESTONE 42: founder of its own group, same reasoning as A/B.
    p.pgid = EXEC_TEST_PROCESS_ID;
    *EXEC_TEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 53: creates FAULT_TEST_PROCESS -- see that static's own
/// doc comment. Mirrors init_waitstatus_test_process()'s exact pattern:
/// never `run()` directly as a top-level process, exists purely as a
/// fork() SOURCE for self_test_fault_status() below.
pub fn init_fault_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 53: creating FAULT_TEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "fault-test", &FAULT_TEST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 53: FAULT_TEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    // MILESTONE 42: founder of its own group, same reasoning as A/B.
    p.pgid = FAULT_TEST_PROCESS_ID;
    *FAULT_TEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 57: creates DEMAND_PAGE_TEST_PROCESS -- see that static's
/// own doc comment. Mirrors init_fault_test_process()'s exact pattern,
/// except this one IS `run()` directly (a top-level excursion, not a
/// fork() source) -- self_test_demand_paging_heap() below runs it for
/// real and then inspects its heap_frames afterward.
pub fn init_demand_page_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 57: creating DEMAND_PAGE_TEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "demand-page-test", &DEMAND_PAGE_TEST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 57: DEMAND_PAGE_TEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    p.pgid = DEMAND_PAGE_TEST_PROCESS_ID;
    *DEMAND_PAGE_TEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 57: creates DEMAND_PAGE_OOB_TEST_PROCESS -- see that
/// static's own doc comment. Also `run()` directly, same as
/// DEMAND_PAGE_TEST_PROCESS above; this one is EXPECTED to fault and be
/// terminated by the unchanged Milestone 41 SIGSEGV path.
pub fn init_demand_page_oob_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 57: creating DEMAND_PAGE_OOB_TEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "demand-page-oob-test", &DEMAND_PAGE_OOB_TEST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 57: DEMAND_PAGE_OOB_TEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    p.pgid = DEMAND_PAGE_OOB_TEST_PROCESS_ID;
    *DEMAND_PAGE_OOB_TEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 59: creates ERRNO_TEST_PROCESS -- see ERRNO_TEST_PROGRAM's
/// own doc comment. Mirrors init_demand_page_test_process()'s exact
/// pattern (a top-level excursion `run()`s directly, not a fork()
/// source) -- self_test_errno() below runs it for real and then checks
/// the process's own final `errno` field afterward.
pub fn init_errno_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 59: creating ERRNO_TEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "errno-test", &ERRNO_TEST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 59: ERRNO_TEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    p.pgid = ERRNO_TEST_PROCESS_ID;
    *ERRNO_TEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 60: creates SIGNAL_TEST_PROCESS -- see SIGNAL_TEST_PROGRAM's
/// own doc comment. Mirrors init_errno_test_process()'s exact pattern (a
/// top-level excursion `run()`s directly, not a fork() source) --
/// self_test_signal_delivery() below runs it for real and then checks its own
/// heap markers afterward.
pub fn init_signal_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 60: creating SIGNAL_TEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "signal-test", &SIGNAL_TEST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 60: SIGNAL_TEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    p.pgid = SIGNAL_TEST_PROCESS_ID;
    *SIGNAL_TEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 64: creates MMAP_READ_TEST_PROCESS -- see
/// MMAP_READ_TEST_PROGRAM's own doc comment. Mirrors
/// init_demand_page_test_process()'s exact pattern (a top-level excursion
/// `run()`s directly, not a fork() source) -- self_test_mmap() below runs
/// it for real and then checks its own heap markers afterward.
pub fn init_mmap_read_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 64: creating MMAP_READ_TEST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "mmap-read-test", &MMAP_READ_TEST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 64: MMAP_READ_TEST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    p.pgid = MMAP_READ_TEST_PROCESS_ID;
    *MMAP_READ_TEST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 64: creates MMAP_WRITE_BEFORE_READ_FAULT_PROCESS -- see
/// MMAP_WRITE_BEFORE_READ_FAULT_PROGRAM's own doc comment. Same pattern
/// as init_mmap_read_test_process() above; this one is EXPECTED to fault
/// and be terminated by the unchanged Milestone 41 SIGSEGV path.
pub fn init_mmap_write_before_read_fault_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 64: creating MMAP_WRITE_BEFORE_READ_FAULT_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "mmap-write-before-read-fault-test", &MMAP_WRITE_BEFORE_READ_FAULT_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 64: MMAP_WRITE_BEFORE_READ_FAULT_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    p.pgid = MMAP_WRITE_BEFORE_READ_FAULT_PROCESS_ID;
    *MMAP_WRITE_BEFORE_READ_FAULT_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 64: creates MMAP_WRITE_AFTER_READ_FAULT_PROCESS -- see
/// MMAP_WRITE_AFTER_READ_FAULT_PROGRAM's own doc comment. Same pattern as
/// init_mmap_read_test_process() above; this one is EXPECTED to fault and
/// be terminated by the unchanged Milestone 41 SIGSEGV path, AFTER its
/// first heap marker is genuinely set.
pub fn init_mmap_write_after_read_fault_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 64: creating MMAP_WRITE_AFTER_READ_FAULT_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "mmap-write-after-read-fault-test", &MMAP_WRITE_AFTER_READ_FAULT_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 64: MMAP_WRITE_AFTER_READ_FAULT_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    p.pgid = MMAP_WRITE_AFTER_READ_FAULT_PROCESS_ID;
    *MMAP_WRITE_AFTER_READ_FAULT_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 64: creates MMAP_USE_AFTER_UNMAP_FAULT_PROCESS -- see
/// MMAP_USE_AFTER_UNMAP_FAULT_PROGRAM's own doc comment. Same pattern as
/// init_mmap_read_test_process() above; this one is EXPECTED to fault and
/// be terminated by the unchanged Milestone 41 SIGSEGV path, AFTER its
/// first two heap markers are genuinely set.
pub fn init_mmap_use_after_unmap_fault_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 64: creating MMAP_USE_AFTER_UNMAP_FAULT_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "mmap-use-after-unmap-fault-test", &MMAP_USE_AFTER_UNMAP_FAULT_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 64: MMAP_USE_AFTER_UNMAP_FAULT_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    p.pgid = MMAP_USE_AFTER_UNMAP_FAULT_PROCESS_ID;
    *MMAP_USE_AFTER_UNMAP_FAULT_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 65: creates MMAP_WRITABLE_READ_THEN_WRITE_PROCESS -- see
/// MMAP_WRITABLE_READ_THEN_WRITE_PROGRAM's own doc comment. Same pattern
/// as init_mmap_read_test_process() above.
pub fn init_mmap_writable_read_then_write_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 65: creating MMAP_WRITABLE_READ_THEN_WRITE_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "mmap-writable-read-then-write-test", &MMAP_WRITABLE_READ_THEN_WRITE_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 65: MMAP_WRITABLE_READ_THEN_WRITE_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    p.pgid = MMAP_WRITABLE_READ_THEN_WRITE_PROCESS_ID;
    *MMAP_WRITABLE_READ_THEN_WRITE_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 65: creates MMAP_WRITABLE_WRITE_FIRST_PROCESS -- see
/// MMAP_WRITABLE_WRITE_FIRST_PROGRAM's own doc comment. Same pattern as
/// init_mmap_read_test_process() above.
pub fn init_mmap_writable_write_first_test_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
) -> Result<(), &'static str> {
    let _ = writeln!(serial(), "milestone 65: creating MMAP_WRITABLE_WRITE_FIRST_PROCESS's private address space...");
    let mut p = create_process_from_image(frame_allocator, phys_mem_offset, "mmap-writable-write-first-test", &MMAP_WRITABLE_WRITE_FIRST_PROGRAM)?;
    let _ = writeln!(
        serial(),
        "milestone 65: MMAP_WRITABLE_WRITE_FIRST_PROCESS created -- pml4={:#x} code={:#x} stack={:#x}",
        p.pml4_frame.start_address().as_u64(),
        p.code_frame.start_address().as_u64(),
        p.stack_frame.start_address().as_u64()
    );
    p.pgid = MMAP_WRITABLE_WRITE_FIRST_PROCESS_ID;
    *MMAP_WRITABLE_WRITE_FIRST_PROCESS.lock() = Some(p);
    Ok(())
}

/// MILESTONE 30: the `runproc N` shell command's entry point. Switches
/// CR3 to process N's own PML4, enters ring 3 at the SAME virtual
/// address usertest.rs always uses, lets the syscalls run (write reads
/// this process's own embedded message out of its own code page and
/// prints it to serial, exit restores the kernel's own CR3 and returns),
/// and comes back here once resume_kernel() has unwound back through
/// enter_ring3_now()'s call. Safe to call repeatedly and in any order (1,
/// 2, 1, 2, 2, 1, ...): each call reads the process's CURRENT pml4_frame
/// out of its Mutex slot fresh, and the process's own frames are never
/// mutated by a run, only by create_process() at boot.
pub fn run(id: u8) -> Result<(), &'static str> {
    let slot = match id {
        1 => &PROCESS_A,
        2 => &PROCESS_B,
        FDTEST_PROCESS_ID => &FDTEST_PROCESS,
        FORK_TEST_PROCESS_ID => &FORK_TEST_PROCESS,
        SIGSEGV_TEST_PROCESS_ID => &SIGSEGV_TEST_PROCESS,
        SIGKILL_TEST_PROCESS_ID => &SIGKILL_TEST_PROCESS,
        WAITSTATUS_TEST_PROCESS_ID => &WAITSTATUS_TEST_PROCESS,
        EXEC_TEST_PROCESS_ID => &EXEC_TEST_PROCESS,
        FAULT_TEST_PROCESS_ID => &FAULT_TEST_PROCESS,
        DEMAND_PAGE_TEST_PROCESS_ID => &DEMAND_PAGE_TEST_PROCESS,
        DEMAND_PAGE_OOB_TEST_PROCESS_ID => &DEMAND_PAGE_OOB_TEST_PROCESS,
        ERRNO_TEST_PROCESS_ID => &ERRNO_TEST_PROCESS,
        SIGNAL_TEST_PROCESS_ID => &SIGNAL_TEST_PROCESS,
        MMAP_READ_TEST_PROCESS_ID => &MMAP_READ_TEST_PROCESS,
        MMAP_WRITE_BEFORE_READ_FAULT_PROCESS_ID => &MMAP_WRITE_BEFORE_READ_FAULT_PROCESS,
        MMAP_WRITE_AFTER_READ_FAULT_PROCESS_ID => &MMAP_WRITE_AFTER_READ_FAULT_PROCESS,
        MMAP_USE_AFTER_UNMAP_FAULT_PROCESS_ID => &MMAP_USE_AFTER_UNMAP_FAULT_PROCESS,
        MMAP_WRITABLE_READ_THEN_WRITE_PROCESS_ID => &MMAP_WRITABLE_READ_THEN_WRITE_PROCESS,
        MMAP_WRITABLE_WRITE_FIRST_PROCESS_ID => &MMAP_WRITABLE_WRITE_FIRST_PROCESS,
        _ => {
            return Err(
                "no such process -- use 1, 2, 4 (FDTEST_PROCESS_ID), 5 (FORK_TEST_PROCESS_ID), 6 (SIGSEGV_TEST_PROCESS_ID), 7 (SIGKILL_TEST_PROCESS_ID), 8 (WAITSTATUS_TEST_PROCESS_ID), 9 (EXEC_TEST_PROCESS_ID), 10 (FAULT_TEST_PROCESS_ID), 11 (DEMAND_PAGE_TEST_PROCESS_ID), 12 (DEMAND_PAGE_OOB_TEST_PROCESS_ID), 17 (ERRNO_TEST_PROCESS_ID), 18 (SIGNAL_TEST_PROCESS_ID), 19 (MMAP_READ_TEST_PROCESS_ID), 20 (MMAP_WRITE_BEFORE_READ_FAULT_PROCESS_ID), 21 (MMAP_WRITE_AFTER_READ_FAULT_PROCESS_ID), or 22 (MMAP_USE_AFTER_UNMAP_FAULT_PROCESS_ID)",
            );
        }
    };
    let (pml4_frame, label, entry) = {
        let guard = slot.lock();
        let proc = guard.as_ref().ok_or("process not initialized")?;
        (proc.pml4_frame, proc.label, proc.entry)
    };

    let _ = writeln!(
        serial(),
        "milestone 30: runproc {id} (process {label}) -- about to switch CR3 to process pml4 {:#x}",
        pml4_frame.start_address().as_u64()
    );
    ACTIVE_PROCESS.store(id, Ordering::SeqCst);

    let flags = Cr3Flags::from_bits_truncate(KERNEL_CR3_FLAGS_BITS.load(Ordering::SeqCst));
    unsafe { Cr3::write(pml4_frame, flags) };
    let _ = writeln!(
        serial(),
        // MILESTONE 44: real per-process entry now (was always
        // USER_CODE_ADDR before this milestone; still is for every one
        // of these six hardcoded processes -- see Process::entry's own
        // doc comment).
        "milestone 30: CR3 switched -- entering ring 3 for process {id} at {:#x}",
        entry
    );

    usertest::enter_ring3_now(entry);

    let _ = writeln!(
        serial(),
        "milestone 30: runproc {id} -- resumed in kernel context (CR3 already restored by the exit syscall before this point)"
    );
    Ok(())
}

/// MILESTONE 34: holds the single process most recently built by
/// loader.rs's `runfile` from a real file's bytes -- one slot, replaced
/// (not accumulated) on each call, matching PROCESS_A/PROCESS_B's own
/// one-shot-per-slot design rather than a general process table with
/// PIDs. Kept alive here (rather than dropped once run_loaded_process()
/// returns) purely so its PML4/code/stack/heap frames aren't freed out
/// from under a process that could, in principle, still be referenced --
/// in practice run_loaded_process() always runs it to completion
/// (write + exit) before returning, same as run() does for A/B.
static LOADED_PROCESS: Mutex<Option<Process>> = Mutex::new(None);

/// Distinguishes a file-loaded process from PROCESS_A (id 1) / PROCESS_B
/// (id 2) in ACTIVE_PROCESS -- usertest::syscall_dispatch's syscall arms
/// only check `active != 0` before reading the currently-active
/// process's own message/heap, so any nonzero value works; 3 simply
/// keeps the id space non-overlapping and gives the serial log a
/// distinct, greppable number.
pub(crate) const LOADED_PROCESS_ID: u8 = 3;

/// MILESTONE 34: builds a fresh process from an arbitrary in-memory code
/// image -- loader.rs's `runfile` is the only real caller, always AFTER
/// it has already read a file's bytes off the real on-disk filesystem
/// and checked their length against MAX_CODE_IMAGE_BYTES -- and runs it
/// once immediately. Otherwise IDENTICAL to run()'s own CR3-switch /
/// enter_ring3_now() / (exit syscall restores CR3) sequence: the only
/// real difference is that this process's code frame was populated from
/// `image` (an arbitrary runtime byte slice) instead of one of the two
/// fixed PROCESS_A/PROCESS_B slots created once at boot. Like them, it
/// also gets its own private heap mapped by create_process_from_image()
/// -- unused by a plain print+exit test program, but not withheld
/// either, since every process gets one uniformly now.
/// MILESTONE 35: split out of what used to be one function
/// (`load_and_run_image`, which built the process AND entered ring 3 in
/// a single call, taking `frame_allocator` for its entire duration) --
/// see run_loaded_process()'s own doc comment for the real, pre-existing
/// Milestone 34 bug this split fixes. This half does ONLY the frame
/// allocation and page-table building (via create_process_from_image)
/// and stores the result in LOADED_PROCESS; it never touches ring 3, so
/// it's safe for the caller to hold frame_allocator's lock for exactly
/// this call and no longer.
pub fn create_loaded_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
    image: &[u8],
) -> Result<(), &'static str> {
    let mut proc = create_process_from_image(frame_allocator, phys_mem_offset, "loaded", image)?;
    // MILESTONE 42: founder of its own group, same reasoning as A/B.
    proc.pgid = LOADED_PROCESS_ID;
    *LOADED_PROCESS.lock() = Some(proc);
    Ok(())
}

/// MILESTONE 35: the other half of what used to be `load_and_run_image`
/// -- switches CR3 to LOADED_PROCESS's pml4 and enters ring 3, exactly
/// like the old function's second half did, but takes NO frame_allocator
/// argument at all, so loader.rs's `run_file` can (and now does) call
/// this AFTER releasing memory::with_frame_allocator's lock instead of
/// from inside its closure.
///
/// **Real, pre-existing Milestone 34 bug, found and fixed while
/// verifying Milestone 35's own fd syscalls, not caused by them**: the
/// original `load_and_run_image` took `frame_allocator: &mut impl
/// FrameAllocator<Size4KiB>` for its WHOLE body, and loader.rs's
/// `run_file` called it from inside `memory::with_frame_allocator`'s
/// closure -- which holds the global FRAME_ALLOCATOR spin::Mutex for as
/// long as the closure runs. That meant the ENTIRE ring-3 excursion
/// (enter_ring3_now(), which runs with interrupts enabled per usertest
/// design) executed with that lock held, for no reason -- nothing in
/// the excursion itself ever touches frame_allocator. Verified as the
/// real root cause, not guessed: `runproc` (Milestone 30's PROCESS_A/B
/// path, which never touches memory::with_frame_allocator at all since
/// A/B are allocated once at boot) never crashed under repeated testing,
/// while `runfile` (which DID route the whole excursion through that
/// lock) reproducibly page-faulted at instruction address 0 immediately
/// after "resumed in kernel context" -- with UNMODIFIED Milestone 34
/// code (loader.rs's own pre-existing `seedtestprog`/`testprog`, no
/// Milestone 35 fd syscalls involved at all), confirming this was
/// already broken before this milestone touched anything. This split
/// (create_loaded_process does the part that needs the lock,
/// run_loaded_process does the part that doesn't) removes the
/// lock-held-across-a-whole-ring-3-excursion-with-interrupts-on window
/// entirely.
pub fn run_loaded_process() -> Result<(), &'static str> {
    let pml4_frame = {
        let guard = LOADED_PROCESS.lock();
        let proc = guard.as_ref().ok_or("no loaded process -- create_loaded_process must succeed first")?;
        proc.pml4_frame
    };

    let _ = writeln!(
        serial(),
        "milestone 34: loaded process -- about to switch CR3 to pml4 {:#x}",
        pml4_frame.start_address().as_u64()
    );
    ACTIVE_PROCESS.store(LOADED_PROCESS_ID, Ordering::SeqCst);

    let flags = Cr3Flags::from_bits_truncate(KERNEL_CR3_FLAGS_BITS.load(Ordering::SeqCst));
    unsafe { Cr3::write(pml4_frame, flags) };
    let _ = writeln!(
        serial(),
        "milestone 34: CR3 switched -- entering ring 3 for the file-loaded process at {:#x}",
        usertest::USER_CODE_ADDR
    );

    // MILESTONE 44: the flat-binary `runfile` path always loads at
    // USER_CODE_ADDR (create_process_from_image() sets Process::entry
    // to exactly that -- see its own doc comment) -- unaffected by this
    // milestone, passed explicitly rather than reading it back out of
    // LOADED_PROCESS a second time.
    usertest::enter_ring3_now(usertest::USER_CODE_ADDR);

    let _ = writeln!(
        serial(),
        "milestone 34: loaded process -- resumed in kernel context (CR3 already restored by the exit syscall before this point)"
    );
    Ok(())
}

/// MILESTONE 36: real per-segment page-count caps, enforced BEFORE any
/// frame is allocated (same "reject up front, never silently overflow"
/// discipline as MAX_CODE_IMAGE_BYTES). Deliberately small: fs.rs's own
/// MAX_FILE_BYTES cap (4096 bytes total, same constant loader.rs's own
/// self_test_size_check() comment already discusses for the flat-binary
/// path) already puts a hard ceiling on how much any single ELF file
/// COULD ask for, but page-count caps are checked independently here
/// too -- a pathological program header table could, in principle,
/// declare a p_memsz far larger than the file itself (memsz > filesz is
/// completely legal ELF, e.g. for .bss), so file size alone doesn't
/// bound how many PHYSICAL FRAMES a malicious/malformed ELF could try
/// to claim.
/// MILESTONE 70 UPDATE: raised 4 -> 64 (16 KiB -> 256 KiB per segment).
/// Real trigger: Tier 3's own toolchain binary, the embedded `cc.elf`
/// (loaded straight out of the kernel image via `include_bytes!` in
/// loader.rs -- NOT subject to fs.rs's separate, unrelated, still-
/// unraised `MAX_FILE_BYTES` 4096-byte on-disk-file cap, which only
/// bounds files cc.elf itself later WRITES to disk, e.g. its own
/// compiled output) grew from Milestone 69's 17952 bytes to Milestone
/// 70's 24112 bytes adding real comparison-operator and if/else
/// codegen, needing 6 pages in its one PT_LOAD segment -- already past
/// the old 4-page cap, confirmed the hard way via a real fresh-QEMU-
/// boot failure: `milestone 67: self-test FAILED -- run_loaded_elf_
/// process(cc.elf) returned Err: elf: a PT_LOAD segment needs more
/// pages than this loader's fixed per-segment cap` (see
/// `m70_fresh_boot.log` in the repo root).
///
/// Same class of situation as Milestone 57's `HEAP_PAGE_COUNT`
/// increase (4 -> 64 pages): a real, legitimately growing component
/// outgrew an initially-conservative limit, not a bug to route around.
/// The cap's own job (per its ORIGINAL doc comment above -- still true)
/// is defending against a MALICIOUS/malformed ELF's `p_memsz` claiming
/// far more physical frames than its file size would suggest; it was
/// never meant to track this one specific, honest, ever-growing
/// toolchain binary at its exact current size, which is why it keeps
/// needing to move as Tier 3 continues (subset-C grammar growth already
/// planned next, then a real assembler, then a real linker, per the
/// Tier 3 roadmap and Milestone 69's own "still genuinely open" list).
///
/// 64 pages (256 KiB) is chosen, not an unbounded/very large number,
/// for a real reason: this is still meant as a genuine "reject up
/// front" safety cap, not a formality -- QEMU's default boot (no `-m`
/// flag anywhere in `src/main.rs`/`launch_qemu.ps1`) gives this kernel
/// 128 MiB (32768 4 KiB pages) of physical memory total, so even a
/// single process claiming the full 64-page cap would use under 0.2%
/// of it; 64 pages is roughly 10x cc.elf's real current need (6 pages),
/// real headroom for several more milestones of grammar/codegen growth
/// without being "whatever the file happens to ask for". If a future
/// Tier 3 milestone (the assembler/linker work named above) genuinely
/// outgrows this again, the fix is the same as this one: bump it again,
/// with the same real-growth justification, not delete the cap.
const MAX_PAGES_PER_ELF_SEGMENT: u64 = 64;
/// Real, disclosed bound on PT_LOAD segment count this loader will
/// actually map -- elf::parse() already enforces its own
/// elf::MAX_LOAD_SEGMENTS cap on how many it will even PARSE; this is
/// the separate, mapping-side check (kept as its own named constant
/// rather than silently reusing elf::MAX_LOAD_SEGMENTS, so the two
/// layers' caps are independently visible in a diff instead of one
/// hidden re-export away).
const MAX_ELF_LOAD_SEGMENTS: usize = elf::MAX_LOAD_SEGMENTS;
/// MILESTONE 70 UPDATE: raised 16 -> 128 (64 KiB -> 512 KiB total).
/// Kept at 2x the new `MAX_PAGES_PER_ELF_SEGMENT` (same 2x ratio the
/// ORIGINAL 16-vs-4 pair already used) rather than raised independently
/// -- cc.elf itself is still ONE PT_LOAD segment (see
/// `tools/cc_src/linker.ld`'s own single `seg1` PHDR), so this total
/// only actually matters today for a future multi-segment ELF (e.g. a
/// real linker emitting separate .text/.rodata/.data segments, named as
/// still-not-built in Milestone 69's own "still genuinely open" list);
/// 128 pages (512 KiB) is still under 0.4% of this kernel's real 128
/// MiB default QEMU memory, same "comfortably generous, still a real
/// disclosed ceiling" reasoning as the per-segment cap just above, not
/// re-derived from a different principle.
const MAX_TOTAL_ELF_PAGES: u64 = 128;

/// MILESTONE 36: the real page-table-building mechanism for a genuine
/// ELF64 executable -- maps ONE PHYSICAL PAGE PER PT_LOAD-SEGMENT-PAGE
/// at that segment's OWN `p_vaddr` (page-aligned, checked, not just the
/// single fixed USER_CODE_ADDR page create_process_from_image() always
/// uses), copies each page's real `p_filesz`-covered bytes out of
/// `image` at that segment's real `p_offset`, and zero-fills the rest
/// (memsz > filesz, e.g. bss) -- genuinely reading and placing the
/// parsed ELF structure, not just re-running the flat-binary path with
/// extra logging.
///
/// Real, disclosed limitations of THIS function specifically (elf.rs's
/// parse() itself has none of these -- they're loading-side, not
/// parsing-side):
///   - MILESTONE 44: `entry` no longer has to equal `USER_CODE_ADDR` --
///     Milestone 36's own deliberate deferral (see this module's
///     original MILESTONE 36 doc comment, still here for the real
///     history) is now closed: `usertest::enter_ring3_now()` takes the
///     real entry address as a parameter, so this function only needs
///     `entry` to fall within a mapped PT_LOAD segment (checked below)
///     and within this kernel's one fixed private-PML4 user-space
///     region (also checked below, via `user_p4_index`) -- NOT a
///     specific constant. `code_frame` is now whichever mapped page
///     actually backs `entry`'s own page, not literally the page at
///     `USER_CODE_ADDR`.
///   - every PT_LOAD segment's `p_vaddr` MUST be 4 KiB-page-aligned --
///     this loader maps whole pages at page-aligned addresses; a
///     segment starting mid-page (legal ELF, common in real Linux
///     binaries that pack multiple segments' sub-page tails/heads
///     together to save file space) is rejected with a clear error
///     rather than silently misaligned or corrupted.
///   - bounded per-segment and total page counts (see the constants
///     just above) -- rejected up front if exceeded.
///   - every segment (like create_process_from_image()'s single code
///     page) is mapped PRESENT | WRITABLE | USER_ACCESSIBLE regardless
///     of its own `p_flags` -- p_flags is parsed and logged for real
///     (proving the parser read it), but not yet used to differentiate
///     actual page permissions (e.g. a read-only or non-executable
///     PT_LOAD segment is still mapped writable+executable here, same
///     as every other page this kernel has ever mapped for ring 3 --
///     see usertest::setup()'s own comment on why no page in this
///     kernel sets NO_EXECUTE anywhere yet). Disclosed, not hidden:
///     enforcing p_flags for real would be genuine, separate follow-up
///     work.
fn create_process_from_elf(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
    label: &'static str,
    entry: u64,
    segments: &[elf::ProgramSegment],
    image: &[u8],
) -> Result<Process, &'static str> {
    // MILESTONE 44: the old "entry MUST equal USER_CODE_ADDR" rejection
    // lived here -- removed. `entry`'s real validation is the
    // `entry_in_segment` check below (a malformed ELF claiming an
    // entry address no PT_LOAD segment actually covers is still
    // rejected, same as before) plus the per-page `user_p4_index`
    // check every segment page already goes through.
    if segments.is_empty() {
        return Err("elf: no PT_LOAD segments to map");
    }
    if segments.len() > MAX_ELF_LOAD_SEGMENTS {
        return Err("elf: more PT_LOAD segments than this loader's fixed cap");
    }

    let user_p4_index = VirtAddr::new(usertest::USER_CODE_ADDR).p4_index();
    let stack_p4_index = VirtAddr::new(usertest::USER_STACK_ADDR).p4_index();
    if user_p4_index != stack_p4_index {
        return Err("USER_CODE_ADDR/USER_STACK_ADDR fall in different PML4 indices -- design assumption violated");
    }

    // Real, honest ELF-loading sanity check: e_entry must actually fall
    // within SOME loaded segment's declared [p_vaddr, p_vaddr+p_memsz)
    // range -- checked against the ACTUAL parsed segments, not just
    // assumed true because entry == USER_CODE_ADDR was already checked
    // above (a malformed ELF could still claim that entry address while
    // no PT_LOAD segment actually covers it).
    let entry_in_segment = segments
        .iter()
        .any(|s| entry >= s.p_vaddr && entry < s.p_vaddr.saturating_add(s.p_memsz));
    if !entry_in_segment {
        return Err("elf: e_entry does not fall within any PT_LOAD segment's [p_vaddr, p_vaddr+p_memsz) range");
    }

    // Validate EVERY segment's page range up front -- page alignment,
    // per-segment page cap, and (mirroring Milestone 30's own discipline
    // for the single code/stack/heap pages) that every page genuinely
    // computed p4_index()'s to the SAME private PML4 slot this process's
    // new PML4 will actually rebuild below, checked for real rather than
    // assumed true just because USER_CODE_ADDR/HEAP_START happen to
    // share it.
    let mut total_pages: u64 = 0;
    for seg in segments {
        if seg.p_vaddr % PAGE_SIZE as u64 != 0 {
            return Err(
                "elf: a PT_LOAD segment's p_vaddr is not page-aligned -- this loader requires page-aligned \
                 segments (a real, disclosed limitation, not silently rounded)",
            );
        }
        if seg.p_memsz == 0 {
            return Err("elf: a PT_LOAD segment has p_memsz == 0");
        }
        let pages = seg.p_memsz.div_ceil(PAGE_SIZE as u64);
        if pages > MAX_PAGES_PER_ELF_SEGMENT {
            return Err("elf: a PT_LOAD segment needs more pages than this loader's fixed per-segment cap");
        }
        for i in 0..pages {
            let va = VirtAddr::new(seg.p_vaddr + i * PAGE_SIZE as u64);
            if va.p4_index() != user_p4_index {
                return Err(
                    "elf: a PT_LOAD segment page falls outside the process's private PML4 slot -- refusing to map",
                );
            }
        }
        total_pages = total_pages.checked_add(pages).ok_or("elf: total PT_LOAD page count overflows")?;
    }
    if total_pages > MAX_TOTAL_ELF_PAGES {
        return Err("elf: total PT_LOAD page count across all segments exceeds this loader's fixed cap");
    }

    let _ = writeln!(
        serial(),
        "milestone 36: process {label} -- ELF validated: e_entry={:#x} falls inside a mapped segment, {} PT_LOAD segment(s), {} total page(s), all within private p4 index {}",
        entry,
        segments.len(),
        total_pages,
        u16::from(user_p4_index)
    );

    let new_pml4_frame = frame_allocator.allocate_frame().ok_or("out of physical frames (pml4)")?;
    let new_pml4_ptr: *mut PageTable = (phys_mem_offset + new_pml4_frame.start_address().as_u64()).as_mut_ptr();
    let new_pml4: &mut PageTable = unsafe { &mut *new_pml4_ptr };
    new_pml4.zero();

    // POST-MILESTONE-65 FIX: same real fix as create_process_from_image()
    // above -- was `Cr3::read()`, now the canonically-saved kernel PML4.
    // THIS call site is the one that actually matters: exec_elf()/
    // exec_elf_with_args() call this function from INSIDE the EXEC/
    // EXECARGV syscall's own dispatch, while CR3 is still the DYING
    // process's own private PML4 -- see kernel_pml4_for_new_process()'s
    // own doc comment for the full, real, `-d int`-trace-confirmed
    // story.
    let (kernel_pml4_frame, _) = kernel_pml4_for_new_process();
    let kernel_pml4_ptr: *const PageTable = (phys_mem_offset + kernel_pml4_frame.start_address().as_u64()).as_ptr();
    let kernel_pml4: &PageTable = unsafe { &*kernel_pml4_ptr };

    // Same private-PML4-slot construction as create_process_from_image()
    // above -- every kernel-space entry shared by copying the ENTRY
    // (not the hierarchy underneath it), only user_p4_index left zeroed
    // for map_to() to build a genuinely fresh, private chain into.
    for i in 0u16..512 {
        let idx = PageTableIndex::new(i);
        if idx != user_p4_index {
            new_pml4[idx] = kernel_pml4[idx].clone();
        }
    }
    let _ = writeln!(
        serial(),
        "milestone 36: process {label} -- new pml4 {:#x} populated: 511 kernel-space entries shared, index {} left private",
        new_pml4_frame.start_address().as_u64(),
        u16::from(user_p4_index)
    );

    let mut process_mapper = unsafe { OffsetPageTable::new(new_pml4, phys_mem_offset) };
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

    let mut code_frame: Option<PhysFrame<Size4KiB>> = None;
    let mut extra_frames = Vec::new();
    // Collected across the whole loop and logged ONCE at the end (see
    // the single writeln! after this loop) -- matches
    // create_process_from_image()'s own pattern of one summary log
    // after its code/stack mapping, not one log per page. Several
    // per-page `writeln!` calls each construct a fresh SerialPort and
    // block on real (emulated but genuinely time-costed) UART I/O;
    // batching to one write keeps this function's total real-time cost
    // closer to the already-proven-stable flat-binary path's, rather
    // than adding up N page-mapping log lines' worth of extra latency
    // before ever reaching the ring-3 entry below.
    let mut summary: Vec<(u64, u64, u64, u32)> = Vec::new();

    // MILESTONE 44: which mapped page becomes `code_frame` is now the
    // page that actually backs `entry` -- generalized from the old
    // literal `page_va == usertest::USER_CODE_ADDR` check, which only
    // ever worked because entry was forced to equal that constant.
    // Page-aligned the same way every PT_LOAD page address already is
    // in this loop (p_vaddr is checked page-aligned above; entry itself
    // needn't be, so this rounds down to the page containing it).
    let entry_page = entry & !(PAGE_SIZE as u64 - 1);

    for seg in segments {
        let pages = seg.p_memsz.div_ceil(PAGE_SIZE as u64);
        for i in 0..pages {
            let page_va = seg.p_vaddr + i * PAGE_SIZE as u64;
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_va));
            let frame = frame_allocator.allocate_frame().ok_or("out of physical frames (elf segment)")?;
            unsafe {
                process_mapper
                    .map_to(page, frame, flags, frame_allocator)
                    .map_err(|_| "map_to failed (elf segment page)")?
                    .flush();
            }

            // Zeroed through the phys-mem-offset direct view of THIS
            // process's own freshly-allocated frame -- same reasoning as
            // create_process_from_image()'s code/heap page zeroing:
            // written directly through phys_mem_offset rather than
            // through the page_va under whatever CR3 is currently
            // loaded (the kernel's own, at this point), which could
            // resolve to someone else's mapping entirely.
            let frame_virt = phys_mem_offset + frame.start_address().as_u64();
            unsafe { core::ptr::write_bytes::<u8>(frame_virt.as_mut_ptr(), 0, PAGE_SIZE) };

            // Copy however many of this segment's REAL p_filesz bytes
            // fall into THIS specific page -- p_vaddr is page-aligned
            // (checked above), so page i's file-backed byte range is
            // simply [i*PAGE_SIZE, i*PAGE_SIZE+PAGE_SIZE) clamped to
            // p_filesz; anything beyond p_filesz (up to p_memsz) is real
            // bss and stays zeroed from the write_bytes above.
            let page_off_in_seg = i * PAGE_SIZE as u64;
            let copied = if page_off_in_seg < seg.p_filesz {
                let copy_len = core::cmp::min(PAGE_SIZE as u64, seg.p_filesz - page_off_in_seg);
                let src_start = (seg.p_offset + page_off_in_seg) as usize;
                let src = &image[src_start..src_start + copy_len as usize];
                unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), frame_virt.as_mut_ptr(), copy_len as usize) };
                copy_len
            } else {
                0
            };
            summary.push((page_va, frame.start_address().as_u64(), copied, seg.p_flags));

            if page_va == entry_page {
                code_frame = Some(frame);
            } else {
                // MILESTONE 69: now also carries page_va -- see
                // extra_frames' own doc comment on Process for why
                // fork() genuinely needs it (real address, not
                // recomputed/assumed) to remap this exact page at the
                // exact SAME virtual address in a forked child's own
                // private PML4.
                extra_frames.push((VirtAddr::new(page_va), frame));
            }
        }
    }
    let _ = writeln!(
        serial(),
        "milestone 36: process {label} -- mapped {} PT_LOAD page(s): {:?}",
        summary.len(),
        summary
    );

    let code_frame = code_frame.ok_or(
        "elf: internal error -- entry-containment check passed but no mapped page backs entry's own page",
    )?;

    let stack_frame = frame_allocator.allocate_frame().ok_or("out of physical frames (stack)")?;
    let stack_page = Page::<Size4KiB>::containing_address(VirtAddr::new(usertest::USER_STACK_ADDR));
    unsafe {
        process_mapper
            .map_to(stack_page, stack_frame, flags, frame_allocator)
            .map_err(|_| "map_to failed (stack page)")?
            .flush();
    }
    let _ = writeln!(
        serial(),
        "milestone 36: process {label} -- private stack page mapped to frame {:#x}",
        stack_frame.start_address().as_u64()
    );

    // MILESTONE 100: extra stack pages below USER_STACK_ADDR -- same
    // helper, same flags, as the flat-image constructor.
    let extra_stack_frames = map_extra_stack_pages(&mut process_mapper, frame_allocator, flags)?;
    let _ = writeln!(
        serial(),
        "milestone 100: process {label} -- {} extra stack page(s) mapped below USER_STACK_ADDR ({} KiB total stack)",
        extra_stack_frames.len(),
        usertest::USER_STACK_TOTAL_BYTES / 1024
    );

    // MILESTONE 57: same reserved-but-not-mapped heap as
    // create_process_from_image() -- see that function's own comment for
    // the full reasoning; sbrk() itself still only recognizes
    // PROCESS_A/PROCESS_B ids, an existing, already-disclosed
    // Milestone 34 limitation this milestone doesn't change.
    let heap_frames: Vec<Option<PhysFrame<Size4KiB>>> = vec![None; HEAP_PAGE_COUNT as usize];
    let _ = writeln!(
        serial(),
        "milestone 57: process {label} -- private heap RESERVED (not mapped): {} pages at {:#x}..{:#x}, 0 physical frames committed until a real page fault demand-pages one",
        HEAP_PAGE_COUNT,
        HEAP_START,
        HEAP_START + HEAP_SIZE - 1
    );

    Ok(Process {
        label,
        pml4_frame: new_pml4_frame,
        code_frame,
        stack_frame,
        extra_stack_frames,
        heap_frames,
        heap_used: 0,
        // MILESTONE 35: create_process_from_elf predates the fd table --
        // give ELF-loaded processes the same real, empty fd table every
        // other process gets uniformly (see create_process_from_image's
        // own identical field for the reasoning).
        fds: [None, None, None, None],
        extra_frames,
        // MILESTONE 37: an ELF-loaded process is never itself a forked
        // child (fork()'s own fork_build_child() helper always goes
        // through create_process_from_image(), not this function) --
        // see that function's own identical fields for the reasoning.
        parent_pid: None,
        pending_resume: None,
        // MILESTONE 42: real sentinel, see the field's own doc comment --
        // an ELF-loaded process is never itself a forked child (same
        // reasoning as parent_pid above), and its caller (run_elf()) sets
        // this to its own well-known id right after construction.
        pgid: 0,
        // MILESTONE 44: the real, parsed e_entry -- no longer forced to
        // equal USER_CODE_ADDR. See Process::entry's own doc comment.
        entry,
        // MILESTONE 59: same real convention as create_process_from_image's
        // identical field -- a freshly loaded ELF process has never called
        // anything that could have set errno yet.
        errno: 0,
        // MILESTONE 60: same real "clean slate" convention as
        // create_process_from_image's identical fields -- see that
        // constructor's own doc comment (including the disclosed
        // fork()-inheritance gap, which doesn't apply here anyway since
        // an ELF-loaded process is never itself a forked child).
        signal_handlers: [0; signal::NSIG],
        pending_signal: None,
        in_signal_handler: None,
        // MILESTONE 64: same real "clean slate" convention as
        // create_process_from_image's identical field.
        mmaps: [None, None, None, None],
    })
}

/// MILESTONE 36: builds a fresh process from a genuine multi-segment
/// ELF64 executable's PT_LOAD segments -- the "needs frame_allocator"
/// half of what used to be one combined `load_and_run_elf()` function.
/// Reuses the SAME LOADED_PROCESS slot and LOADED_PROCESS_ID as the
/// flat-binary `runfile` path -- both are real, single-slot "one loaded
/// process at a time" designs, and since `runfile`/`runelf` are never
/// both mid-flight at once (the shell runs one command to completion
/// before reading the next), sharing the slot is safe, not a race.
///
/// MILESTONE 57: split out of the original combined `load_and_run_elf()`
/// for the EXACT SAME real reason Milestone 35 already split
/// `load_and_run_image()` into `create_loaded_process()`/
/// `run_loaded_process()` (see `run_file()`'s own doc comment in
/// loader.rs) -- except this time the bug is freshly real rather than
/// merely wasteful. Before this milestone, running the WHOLE ring-3
/// excursion from inside `memory::with_frame_allocator()`'s closure was
/// harmless (the excursion itself never touched `frame_allocator`). As
/// of Milestone 57's demand-paged heap, it is NOT harmless: a heap page
/// fault occurring DURING that excursion needs `try_demand_page_heap()`
/// to call `memory::with_frame_allocator()` itself to get a fresh frame
/// -- and `spin::Mutex` is not reentrant, so a heap fault occurring while
/// still inside the OUTER `with_frame_allocator()` call (exactly what
/// `run_elf()`/`self_test_altentry_elf()`/`self_test_malloc()` all did,
/// unchanged since Milestone 36) spins forever trying to re-acquire a
/// lock this same execution context already holds. **Found via a real,
/// reproducible QEMU crash, not by inspection**: `self_test_malloc()`'s
/// real malloctest.elf is the first ELF-loaded program in this project's
/// history to actually touch its heap (a real `sys_sbrk()` call followed
/// by a real write to the returned pointer) -- every boot up to and
/// including this milestone's first attempt reproducibly hung/crashed
/// (QEMU exit code 2, no further kernel output, confirmed via a real
/// `-d int` trace showing the CPU correctly took the #PF vector and then
/// nothing further was ever logged) at exactly that first heap touch,
/// while `self_test_altentry_elf()` (which never touches its heap) and
/// every hardcoded-process `runproc N` path (which never calls
/// `load_and_run_elf()` / never holds this lock at all) kept working
/// fine in the SAME boot -- real evidence isolating the bug to this one
/// specific lock-scoping gap, not demand paging itself being broken.
pub fn create_loaded_elf_process(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
    image: &[u8],
    elf_image: &elf::ElfImage,
) -> Result<(), &'static str> {
    let mut proc = create_process_from_elf(
        frame_allocator,
        phys_mem_offset,
        "elf-loaded",
        elf_image.entry,
        &elf_image.segments,
        image,
    )?;
    // MILESTONE 42: founder of its own group, same reasoning as A/B.
    proc.pgid = LOADED_PROCESS_ID;
    *LOADED_PROCESS.lock() = Some(proc);
    Ok(())
}

/// MILESTONE 36/57: the ring-3-excursion half of what used to be one
/// combined `load_and_run_elf()` -- called AFTER
/// `create_loaded_elf_process()`'s own `memory::with_frame_allocator()`
/// call has already returned (lock dropped), mirroring
/// `run_loaded_process()`'s exact precedent so a real page fault DURING
/// this excursion (Milestone 57's demand-paged heap) can safely call
/// `memory::with_frame_allocator()` itself without deadlocking. `entry`
/// is passed explicitly (Milestone 44's real, non-`USER_CODE_ADDR`-
/// forced entry point) rather than re-read out of LOADED_PROCESS, same
/// reasoning `run_loaded_process()` already documents for its own
/// USER_CODE_ADDR case.
pub fn run_loaded_elf_process(entry: u64) -> Result<(), &'static str> {
    let pml4_frame = {
        let guard = LOADED_PROCESS.lock();
        let proc = guard.as_ref().ok_or("no loaded ELF process -- create_loaded_elf_process must succeed first")?;
        proc.pml4_frame
    };

    let _ = writeln!(
        serial(),
        "milestone 36: elf-loaded process -- about to switch CR3 to pml4 {:#x}",
        pml4_frame.start_address().as_u64()
    );
    ACTIVE_PROCESS.store(LOADED_PROCESS_ID, Ordering::SeqCst);

    let flags = Cr3Flags::from_bits_truncate(KERNEL_CR3_FLAGS_BITS.load(Ordering::SeqCst));
    unsafe { Cr3::write(pml4_frame, flags) };
    let _ = writeln!(
        serial(),
        "milestone 36: CR3 switched -- entering ring 3 for the ELF-loaded process at {:#x} (real e_entry)",
        entry
    );

    // MILESTONE 44: the real point of this milestone -- entry no longer
    // has to be USER_CODE_ADDR, and this is a genuinely different value
    // for a real test ELF built with a different linker script entry
    // point.
    usertest::enter_ring3_now(entry);

    let _ = writeln!(
        serial(),
        "milestone 36: elf-loaded process -- resumed in kernel context (CR3 already restored by the exit syscall before this point)"
    );
    Ok(())
}


/// MILESTONE 37: this milestone's fork/exec/wait test program -- hand-
/// assembled machine code, assembled + verified via a standalone Python
/// script using the keystone-engine assembler (with a capstone
/// disassembly round-trip confirming the intended control flow byte for
/// byte), the same "verified with a standalone re-derivation, not
/// hand-counted hex digits" discipline established by every other
/// hand-assembled program in this file.
///
/// Control flow (confirmed against the actual disassembly):
///   1. `mov eax, 7; int 0x80` -- syscall 7 = fork(). rax = child pid in
///      the parent, or 0 in the child (forced by
///      usertest::enter_ring3_as_forked_child()'s "xor eax, eax" before
///      the child's very first iretq -- see that function's own doc
///      comment).
///   2. `mov rbx, rax; cmp rbx, 0; je child_path` -- rbx is callee-
///      preserved across every syscall in this ABI (syscall_entry pushes
///      and pops every GPR except rax around syscall_dispatch), so it
///      safely holds the fork() result across the write()/wait() calls
///      below.
///   3. PARENT path (rbx != 0): write()s FORK_MSG_PARENT_OFFSET's
///      distinguishing message, then `mov rdi, rbx; mov eax, 8;
///      int 0x80` -- syscall 8 = wait(child_pid) -- which does not
///      return until the child has run to completion (see
///      process::wait_for_child()'s own doc comment) -- then write()s
///      FORK_MSG_WAITDONE_OFFSET's message and exits.
///   4. CHILD path (rbx == 0, reached only when process::
///      run_forked_child() resumes this exact process at this exact
///      instruction -- NOT by restarting from offset 0, which would
///      immediately re-execute step 1 and fork() again): `mov rdi,
///      FORK_EXEC_PATH_OFFSET; mov esi, 8; mov eax, 9; int 0x80` --
///      syscall 9 = exec("testprog"). On success this NEVER returns
///      here (control jumps straight into testprog's own entry --
///      see usertest::exec_replace_and_enter()); on failure (e.g.
///      `seedtestprog` was never run, so "testprog" doesn't exist on
///      disk) execution falls through to write() a distinguishing
///      FALLBACK message instead, so `runfork` degrades cleanly and
///      honestly either way rather than crashing.
///
///      **MILESTONE 45 note**: "testprog" is Milestone 34's own FLAT
///      binary (`seedtestprog`'s output), not a real ELF64 image --
///      real exec() (this milestone) now requires one, the same
///      requirement `runelf` has always had. So as of this milestone,
///      this specific exec() call ALWAYS takes the FALLBACK branch
///      above (a real, honestly-reported "not a valid ELF64 image"
///      failure, not a crash) -- disclosed here rather than left to
///      look like a still-succeeding demo. Real exec() itself is
///      verified separately by this milestone's own EXEC_TEST_PROGRAM
///      (below) against a genuine ELF64 target.
///
/// Layout (verified against the actual byte array below by
/// self_test_fork_test_program()):
///   offset   0..125   the syscall sequence itself (125 bytes of real
///                      instructions)
///   offset 512..576   FORK_MSG_PARENT (space-padded to 64 bytes)
///   offset 576..640   FORK_MSG_WAITDONE (space-padded to 64 bytes)
///   offset 640..704   FORK_MSG_CHILD_FALLBACK (space-padded to 64 bytes)
///   offset 704..712   "testprog" (8 real bytes, the exec() path)
pub(crate) const FORK_TEST_PROGRAM: [u8; 712] = [
    0xB8, 0x07, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xC3, 0x48, 0x83, 0xFB, 0x00, 0x74, 0x38,
    0x48, 0xBF, 0x00, 0x02, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x40, 0x00, 0x00, 0x00, 0xB8,
    0x00, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0x89, 0xDF, 0xB8, 0x08, 0x00, 0x00, 0x00, 0xCD, 0x80,
    0x48, 0xBF, 0x40, 0x02, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x40, 0x00, 0x00, 0x00, 0xB8,
    0x00, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xEB, 0x2C, 0x48, 0xBF, 0xC0, 0x02, 0x00, 0x50, 0x55, 0x55,
    0x00, 0x00, 0xBE, 0x08, 0x00, 0x00, 0x00, 0xB8, 0x09, 0x00, 0x00, 0x00, 0xCD, 0x80, 0x48, 0xBF,
    0x80, 0x02, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, 0xBE, 0x40, 0x00, 0x00, 0x00, 0xB8, 0x00, 0x00,
    0x00, 0x00, 0xCD, 0x80, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xCD, 0x80, 0xEB, 0xFE, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x6D, 0x69, 0x6C, 0x65, 0x73, 0x74, 0x6F, 0x6E, 0x65, 0x20, 0x33, 0x37, 0x3A, 0x20, 0x68, 0x65,
    0x6C, 0x6C, 0x6F, 0x20, 0x66, 0x72, 0x6F, 0x6D, 0x20, 0x74, 0x68, 0x65, 0x20, 0x50, 0x41, 0x52,
    0x45, 0x4E, 0x54, 0x20, 0x61, 0x66, 0x74, 0x65, 0x72, 0x20, 0x66, 0x6F, 0x72, 0x6B, 0x28, 0x29,
    0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
    0x6D, 0x69, 0x6C, 0x65, 0x73, 0x74, 0x6F, 0x6E, 0x65, 0x20, 0x33, 0x37, 0x3A, 0x20, 0x50, 0x41,
    0x52, 0x45, 0x4E, 0x54, 0x20, 0x6F, 0x62, 0x73, 0x65, 0x72, 0x76, 0x65, 0x64, 0x20, 0x77, 0x61,
    0x69, 0x74, 0x28, 0x29, 0x20, 0x72, 0x65, 0x61, 0x70, 0x20, 0x74, 0x68, 0x65, 0x20, 0x63, 0x68,
    0x69, 0x6C, 0x64, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
    0x6D, 0x69, 0x6C, 0x65, 0x73, 0x74, 0x6F, 0x6E, 0x65, 0x20, 0x33, 0x37, 0x3A, 0x20, 0x43, 0x48,
    0x49, 0x4C, 0x44, 0x20, 0x66, 0x61, 0x6C, 0x6C, 0x62, 0x61, 0x63, 0x6B, 0x20, 0x2D, 0x2D, 0x20,
    0x65, 0x78, 0x65, 0x63, 0x28, 0x27, 0x74, 0x65, 0x73, 0x74, 0x70, 0x72, 0x6F, 0x67, 0x27, 0x29,
    0x20, 0x66, 0x61, 0x69, 0x6C, 0x65, 0x64, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
    0x74, 0x65, 0x73, 0x74, 0x70, 0x72, 0x6F, 0x67,
];

const FORK_MSG_PARENT_OFFSET: usize = 512;
const FORK_MSG_WAITDONE_OFFSET: usize = 576;
const FORK_MSG_CHILD_FALLBACK_OFFSET: usize = 640;
const FORK_EXEC_PATH_OFFSET: usize = 704;

/// The exact messages FORK_TEST_PROGRAM prints via the write(ptr,len)
/// syscall along each path -- named here so self_test_fork_test_program()
/// below and the milestone report can both point at a single source of
/// truth, same convention loader::FDOUT_WRITE_CONTENT/FDTEST_READ_PATH
/// already established.
pub(crate) const FORK_MSG_PARENT: &str = "milestone 37: hello from the PARENT after fork()";
pub(crate) const FORK_MSG_WAITDONE: &str = "milestone 37: PARENT observed wait() reap the child";
pub(crate) const FORK_MSG_CHILD_FALLBACK: &str = "milestone 37: CHILD fallback -- exec('testprog') failed";
pub(crate) const FORK_EXEC_PATH: &str = "testprog";

/// MILESTONE 37: a direct, filesystem-independent proof that
/// FORK_TEST_PROGRAM's byte layout actually matches what its own doc
/// comment claims -- same discipline as loader::self_test_fdtest_program().
/// Called once at boot from kernel_main.
pub fn self_test_fork_test_program() {
    let msg_parent_ok =
        &FORK_TEST_PROGRAM[FORK_MSG_PARENT_OFFSET..FORK_MSG_PARENT_OFFSET + FORK_MSG_PARENT.len()] == FORK_MSG_PARENT.as_bytes();
    let msg_waitdone_ok = &FORK_TEST_PROGRAM[FORK_MSG_WAITDONE_OFFSET..FORK_MSG_WAITDONE_OFFSET + FORK_MSG_WAITDONE.len()]
        == FORK_MSG_WAITDONE.as_bytes();
    let msg_fallback_ok = &FORK_TEST_PROGRAM
        [FORK_MSG_CHILD_FALLBACK_OFFSET..FORK_MSG_CHILD_FALLBACK_OFFSET + FORK_MSG_CHILD_FALLBACK.len()]
        == FORK_MSG_CHILD_FALLBACK.as_bytes();
    let path_ok =
        &FORK_TEST_PROGRAM[FORK_EXEC_PATH_OFFSET..FORK_EXEC_PATH_OFFSET + FORK_EXEC_PATH.len()] == FORK_EXEC_PATH.as_bytes();
    let _ = writeln!(
        serial(),
        "milestone 37: self-test -- FORK_TEST_PROGRAM layout check: parent_msg={msg_parent_ok} waitdone_msg={msg_waitdone_ok} fallback_msg={msg_fallback_ok} exec_path={path_ok} -- {}",
        if msg_parent_ok && msg_waitdone_ok && msg_fallback_ok && path_ok { "all match, layout confirmed" } else { "FAILED -- byte layout drifted from doc comment" }
    );
}

/// MILESTONE 40: a real, boot-time, non-interactive proof that
/// pipe()/dup()/dup2() actually work -- same motivation as fs.rs's
/// self_test_disk_write() (interactive shell-command testing has been
/// unreliable in this environment; this exercises the exact kernel-side
/// functions usertest.rs's syscall handlers call, with zero interactive
/// input, so every boot's serial log carries direct proof). Deliberately
/// scoped to the kernel-side fd/pipe mechanics themselves (pipe_create,
/// write_fd/read_fd dispatching correctly onto FdEntry::Pipe*, dup_fd's
/// real ref-counted sharing) rather than a full ring-3 syscall round
/// trip -- the same "layout/mechanism, not full live execution" scope
/// self_test_fork_test_program() above already established as this
/// project's honest self-test pattern. Reuses FORK_TEST_PROCESS_ID's
/// already-initialized fd table rather than allocating a new process.
pub fn self_test_pipe_mechanics() {
    const MSG: &[u8] = b"pipe self-test payload";
    let id = FORK_TEST_PROCESS_ID;

    let Some((rfd, wfd)) = pipe_create(id) else {
        let _ = writeln!(serial(), "milestone 40: self-test -- FAILED, pipe_create returned None");
        return;
    };

    let write_ok = write_fd(id, wfd, MSG) == Some(MSG.len());
    let read_back = read_fd(id, rfd, MSG.len());
    let roundtrip_ok = read_back.as_deref() == Some(MSG);

    // dup_fd: a second fd cloned from the write end must share the SAME
    // underlying pipe -- write through the dup, read through the
    // original read end, real evidence they're not independent copies.
    let dup_ok = match dup_fd(id, wfd) {
        Some(dup_wfd) => {
            const MSG2: &[u8] = b"via-dup";
            let wrote = write_fd(id, dup_wfd, MSG2) == Some(MSG2.len());
            let got = read_fd(id, rfd, MSG2.len());
            wrote && got.as_deref() == Some(MSG2)
        }
        None => false,
    };

    // dup2_fd: force the read end onto a caller-chosen fd number, then
    // confirm reads through THAT number still reach the same pipe.
    let dup2_target: u64 = 3;
    let dup2_ok = match dup2_fd(id, rfd, dup2_target) {
        Some(true) => {
            const MSG3: &[u8] = b"via-dup2";
            let wrote = write_fd(id, wfd, MSG3) == Some(MSG3.len());
            let got = read_fd(id, dup2_target, MSG3.len());
            wrote && got.as_deref() == Some(MSG3)
        }
        _ => false,
    };

    let all_ok = write_ok && roundtrip_ok && dup_ok && dup2_ok;
    let _ = writeln!(
        serial(),
        "milestone 40: self-test -- pipe mechanics: write_ok={write_ok} roundtrip_ok={roundtrip_ok} dup_ok={dup_ok} dup2_ok={dup2_ok} -- {}",
        if all_ok { "all real, real bytes matched through every path" } else { "FAILED -- see individual flags above" }
    );
}

/// MILESTONE 37: allocates a genuinely new PID slot in PROCESS_TABLE --
/// finds the first free slot and returns `PID_TABLE_BASE + index`, or
/// `None` if the table is already full (MAX_PROCESSES live children).
fn alloc_pid_slot(table: &[Option<Process>; MAX_PROCESSES]) -> Option<usize> {
    table.iter().position(|p| p.is_none())
}

/// MILESTONE 37: the real page-table-building half of fork() -- reads
/// the PARENT's own code/stack/heap frame CONTENTS (real bytes, copied
/// through the phys-mem-offset direct view of the parent's OWN already-
/// allocated physical frames, the same technique every other frame
/// read/write in this file uses) and builds a brand-new process from
/// them via create_process_from_image() (the SAME, unduplicated PML4/
/// code/stack/heap-mapping mechanism every process in this file already
/// goes through) -- then OVERWRITES that fresh process's stack and heap
/// pages (which create_process_from_image() always zero-fills for a
/// brand-new process) with the parent's REAL stack/heap CONTENTS. This
/// is the actual "not shared references" proof: the child's frames are
/// physically distinct addresses (allocate_frame() never returns an
/// already-in-use frame) holding independently-copied bytes.
fn fork_build_child(
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    phys_mem_offset: VirtAddr,
    parent_code_frame: PhysFrame<Size4KiB>,
    parent_stack_frame: PhysFrame<Size4KiB>,
    parent_extra_stack_frames: &[PhysFrame<Size4KiB>],
    parent_heap_frames: &[Option<PhysFrame<Size4KiB>>],
    parent_extra_frames: &[(VirtAddr, PhysFrame<Size4KiB>)],
) -> Result<Process, &'static str> {
    let code_virt = phys_mem_offset + parent_code_frame.start_address().as_u64();
    let mut code_bytes = vec![0u8; PAGE_SIZE];
    unsafe { core::ptr::copy_nonoverlapping(code_virt.as_ptr::<u8>(), code_bytes.as_mut_ptr(), PAGE_SIZE) };

    let mut child = create_process_from_image(frame_allocator, phys_mem_offset, "forked-child", &code_bytes)?;

    let child_stack_virt = phys_mem_offset + child.stack_frame.start_address().as_u64();
    let parent_stack_virt = phys_mem_offset + parent_stack_frame.start_address().as_u64();
    unsafe {
        core::ptr::copy_nonoverlapping(parent_stack_virt.as_ptr::<u8>(), child_stack_virt.as_mut_ptr::<u8>(), PAGE_SIZE)
    };

    // MILESTONE 100: the extra stack pages. `create_process_from_image`
    // already allocated and mapped the child's own set (same count,
    // same virtual addresses) -- fork only needs to copy the parent's
    // real bytes into each, index for index, exactly like the single
    // top page just above. Counts always match: both went through the
    // same constructor with the same `USER_STACK_EXTRA_PAGES`.
    for (child_frame, parent_frame) in child
        .extra_stack_frames
        .iter()
        .copied()
        .zip(parent_extra_stack_frames.iter().copied())
    {
        let cv = phys_mem_offset + child_frame.start_address().as_u64();
        let pv = phys_mem_offset + parent_frame.start_address().as_u64();
        unsafe { core::ptr::copy_nonoverlapping(pv.as_ptr::<u8>(), cv.as_mut_ptr::<u8>(), PAGE_SIZE) };
    }

    // MILESTONE 57: `child.heap_frames` came back from
    // create_process_from_image() as 64 `None`s (nothing mapped yet --
    // see that function's own comment). Only actually map+copy a heap
    // page for the child where the PARENT has one -- i.e. only pages the
    // parent's own `sbrk()`-driven usage has genuinely demand-paged in.
    // A heap page neither process has ever touched stays `None` on both
    // sides: unmapped-but-zero is exactly what an untouched heap page
    // conceptually IS, on either side of a fork(), so there's nothing to
    // copy and nothing lost by leaving it lazy. This is strictly cheaper
    // than the pre-Milestone-57 behavior (which unconditionally mapped
    // and copied all `HEAP_PAGE_COUNT` pages on every fork, regardless of
    // how much of the heap was actually in use).
    let heap_flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
    let pml4_ptr: *mut PageTable = (phys_mem_offset + child.pml4_frame.start_address().as_u64()).as_mut_ptr();
    let child_pml4: &mut PageTable = unsafe { &mut *pml4_ptr };
    let mut child_mapper = unsafe { OffsetPageTable::new(child_pml4, phys_mem_offset) };
    for (i, parent_slot) in parent_heap_frames.iter().enumerate() {
        let Some(parent_frame) = parent_slot else {
            continue;
        };
        let child_frame = frame_allocator.allocate_frame().ok_or("out of physical frames (heap, fork)")?;
        let heap_page = Page::<Size4KiB>::containing_address(VirtAddr::new(HEAP_START + (i as u64) * PAGE_SIZE as u64));
        unsafe {
            child_mapper
                .map_to(heap_page, child_frame, heap_flags, frame_allocator)
                .map_err(|_| "map_to failed (heap page, fork)")?
                .flush();
        }
        let cv = phys_mem_offset + child_frame.start_address().as_u64();
        let pv = phys_mem_offset + parent_frame.start_address().as_u64();
        unsafe { core::ptr::copy_nonoverlapping(pv.as_ptr::<u8>(), cv.as_mut_ptr::<u8>(), PAGE_SIZE) };
        child.heap_frames[i] = Some(child_frame);
    }

    // MILESTONE 69: real fix for the real bug extra_frames' own doc
    // comment on Process describes -- a multi-page ELF-loaded process
    // (cc.elf, now 3 real code pages) has REAL pages beyond `code_frame`
    // (page 0), and before this milestone fork() never even looked at
    // them, so a forked child got NO mapping at all for page 1/2 -- a
    // real, hardware-recorded `INSTRUCTION_FETCH` page fault the instant
    // the child's own resume `rip` (captured at the parent's real fork()
    // call site) happened to land past page 0. Fixed the same way the
    // heap-frame loop just above already handles "map a fresh child
    // frame at a known real vaddr, then copy the parent's bytes into it"
    // -- reusing the SAME real vaddr `extra_frames` now carries (see that
    // field's own doc comment for why it had to start carrying it),
    // exactly the same fixed PRESENT|WRITABLE|USER_ACCESSIBLE flags
    // every OTHER page in this process (code/stack/heap) already maps
    // with (this kernel enforces no per-segment R/W/X distinction
    // anywhere yet -- a real, disclosed, PRE-EXISTING limitation, not
    // something this fix changes).
    let code_flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
    let mut child_extra_frames: Vec<(VirtAddr, PhysFrame<Size4KiB>)> = Vec::with_capacity(parent_extra_frames.len());
    for (page_va, parent_frame) in parent_extra_frames.iter().copied() {
        let child_frame = frame_allocator.allocate_frame().ok_or("out of physical frames (extra code page, fork)")?;
        let page = Page::<Size4KiB>::containing_address(page_va);
        unsafe {
            child_mapper
                .map_to(page, child_frame, code_flags, frame_allocator)
                .map_err(|_| "map_to failed (extra code page, fork)")?
                .flush();
        }
        let cv = phys_mem_offset + child_frame.start_address().as_u64();
        let pv = phys_mem_offset + parent_frame.start_address().as_u64();
        unsafe { core::ptr::copy_nonoverlapping(pv.as_ptr::<u8>(), cv.as_mut_ptr::<u8>(), PAGE_SIZE) };
        child_extra_frames.push((page_va, child_frame));
    }
    let extra_count = child_extra_frames.len();
    child.extra_frames = child_extra_frames;

    let _ = writeln!(
        serial(),
        "milestone 37: fork() -- child's code frame {:#x} holds a REAL byte-for-byte copy of the parent's code frame {:#x} (independently verifiable: same virtual address USER_CODE_ADDR under either process's own CR3, genuinely different physical frames)",
        child.code_frame.start_address().as_u64(),
        parent_code_frame.start_address().as_u64()
    );
    if extra_count > 0 {
        let _ = writeln!(
            serial(),
            "milestone 69: fork() -- child also got {extra_count} extra code page(s) REALLY remapped+copied (real fix -- see extra_frames' own doc comment on Process for the bug this closes; a multi-page ELF-loaded parent's pages beyond page 0 used to be silently dropped by fork())"
        );
    }

    Ok(child)
}

/// MILESTONE 37: the `fork()` syscall's actual kernel-side
/// implementation -- see this module's own top-of-file MILESTONE 37 doc
/// comment for the full scoping decision. Called from usertest.rs's
/// syscall_dispatch with `parent_id` (ACTIVE_PROCESS at the moment of
/// the call) and the parent's own hardware-recorded (resume_rip,
/// resume_rsp) for THIS int 0x80 -- exactly the CPU's own pushed
/// InterruptStackFrame.rip/rsp, unmodified, i.e. the instruction right
/// after the fork() syscall in the parent's own code. Returns the new
/// child's PID, or `None` if `parent_id` doesn't name a live process,
/// the fixed MAX_PROCESSES table is already full, the global frame
/// allocator isn't installed, or (see is_in_child_resume()'s own doc
/// comment) this call is itself nested inside an active child-resume
/// excursion -- this milestone's honest, ENFORCED (not just documented)
/// bound of nesting depth 1.
pub(crate) fn fork(parent_id: u8, resume_rip: u64, resume_rsp: u64) -> Option<u8> {
    if usertest::is_in_child_resume() {
        let _ = writeln!(
            serial(),
            "milestone 37: syscall FORK (process {parent_id}) -- REFUSED, a forked child cannot itself fork() while being resumed by wait() (this design's honest nesting-depth-1 bound) -- returning failure"
        );
        return None;
    }

    // MILESTONE 37: each fallible (heap-allocating) clone is its own
    // separate `let` statement, deliberately NOT combined into one
    // tuple/array-literal expression with several fallible sub-
    // expressions -- confirmed the hard way that combining them (e.g.
    // `(a, b, c.clone(), d.clone())` or `[x.clone(), y.clone(), ...]`
    // in one expression) makes rustc synthesize an unwind-based
    // cleanup path for the earlier, already-heap-allocated Drop values
    // in case a LATER clone panics mid-expression -- genuine unwind
    // landing pads this kernel's panic=abort, no_std target cannot
    // provide, which failed the build with a real `error: unwinding
    // panics are not supported without std`. Once each clone is its
    // own statement, there is nothing "in flight" for a later panic to
    // unwind through, so no landing pad is ever needed.
    let snapshot = with_process_mut(parent_id, |p| {
        let code_frame = p.code_frame;
        let stack_frame = p.stack_frame;
        // MILESTONE 100: the extra stack pages travel with the single
        // top page -- fork_build_child copies each one's real bytes into
        // the child's own already-mapped set.
        let extra_stack_frames = p.extra_stack_frames.clone();
        let heap_frames = p.heap_frames.clone();
        let heap_used = p.heap_used;
        let fd0 = p.fds[0].clone();
        let fd1 = p.fds[1].clone();
        let fd2 = p.fds[2].clone();
        let fd3 = p.fds[3].clone();
        let label = p.label;
        let pgid = p.pgid;
        // MILESTONE 69: real fork() fix -- see extra_frames' own doc
        // comment on Process for the real bug this closes (a multi-page
        // ELF-loaded process's pages beyond page 0 were silently never
        // copied into a forked child at all).
        let extra_frames = p.extra_frames.clone();
        (code_frame, stack_frame, extra_stack_frames, heap_frames, heap_used, [fd0, fd1, fd2, fd3], label, pgid, extra_frames)
    })?;
    let (parent_code, parent_stack, parent_extra_stack_frames, parent_heap_frames, parent_heap_used, parent_fds, parent_label, parent_pgid, parent_extra_frames) =
        snapshot;

    let phys_mem_offset = memory::phys_mem_offset();
    let build_result = memory::with_frame_allocator(|frame_allocator| {
        fork_build_child(frame_allocator, phys_mem_offset, parent_code, parent_stack, &parent_extra_stack_frames, &parent_heap_frames, &parent_extra_frames)
    });
    let mut child = match build_result {
        Some(Ok(c)) => c,
        Some(Err(e)) => {
            let _ = writeln!(serial(), "milestone 37: syscall FORK (process {parent_id}) -- FAILED building child address space: {e}");
            return None;
        }
        None => {
            let _ = writeln!(serial(), "milestone 37: syscall FORK (process {parent_id}) -- FAILED, global frame allocator not installed (should never happen post-boot)");
            return None;
        }
    };
    child.heap_used = parent_heap_used;
    // MILESTONE 40: the child's fd table now holds its OWN clone of any
    // pipe ends the parent had open (see FdEntry's own doc comment for
    // why cloning a PipeRead/PipeWrite index is real sharing, not a
    // data copy) -- so PIPE_TABLE's ref counts must go up to match, the
    // same real-accounting requirement close_fd()/dup_fd()/dup2_fd()
    // already established. Without this, the child's two new fd-table
    // entries would reference a pipe PIPE_TABLE still thinks only the
    // parent holds -- the parent closing its own end would then free
    // the pipe out from under the child (a real dangling-reference bug,
    // not hypothetical).
    for entry in parent_fds.iter().flatten() {
        if let FdEntry::PipeRead(idx) | FdEntry::PipeWrite(idx) = entry {
            let mut table = PIPE_TABLE.lock();
            if let Some(pipe) = table[*idx].as_mut() {
                match entry {
                    FdEntry::PipeRead(_) => pipe.read_open = pipe.read_open.saturating_add(1),
                    FdEntry::PipeWrite(_) => pipe.write_open = pipe.write_open.saturating_add(1),
                    FdEntry::File(_) => unreachable!(),
                }
            }
        }
    }
    child.fds = parent_fds;
    child.parent_pid = Some(parent_id);
    // MILESTONE 42: real Unix inheritance -- a forked child starts in
    // the SAME process group as its parent, not a new one of its own.
    // Callers that want it in its own group call setpgid() afterward,
    // exactly like a real shell does after forking a new pipeline.
    child.pgid = parent_pgid;
    child.pending_resume = Some((resume_rip, resume_rsp));
    let child_pml4 = child.pml4_frame;
    let child_code_frame = child.code_frame;

    let child_pid = {
        let mut table = PROCESS_TABLE.lock();
        let idx = match alloc_pid_slot(&table) {
            Some(i) => i,
            None => {
                let _ = writeln!(
                    serial(),
                    "milestone 37: syscall FORK (process {parent_id}) -- FAILED, PROCESS_TABLE is full ({MAX_PROCESSES} live forked processes already) -- child address space built then discarded"
                );
                return None;
            }
        };
        table[idx] = Some(child);
        PID_TABLE_BASE + idx as u8
    };

    let _ = writeln!(
        serial(),
        "milestone 37: syscall FORK (process {parent_id}, '{parent_label}') -- created child pid {child_pid}: pml4={:#x} code_frame={:#x} resume point rip={:#x} rsp={:#x}",
        child_pml4.start_address().as_u64(),
        child_code_frame.start_address().as_u64(),
        resume_rip,
        resume_rsp
    );

    Some(child_pid)
}

/// MILESTONE 37: switches CR3 to `child_pid`'s own private PML4 and
/// resumes it at EXACTLY the (rip, rsp) fork() captured -- NOT
/// USER_CODE_ADDR -- via usertest::enter_ring3_as_forked_child(), so the
/// child genuinely continues past its own fork() call (with rax forced
/// to 0) instead of restarting the whole program from the top. Uses a
/// SEPARATE, dedicated kernel-resume anchor from the top-level one
/// usertest::run()/enter_ring3_now() use (usertest::CHILD_KERNEL_RSP,
/// via enter_ring3_as_forked_child()) -- exactly one such anchor exists,
/// this milestone's real, disclosed, ENFORCED v1 bound on nesting depth
/// (see fork()'s and wait_for_child()'s own is_in_child_resume() checks).
/// Called only from wait_for_child() below, never directly.
fn run_forked_child(child_pid: u8) -> Result<(), &'static str> {
    let (pml4_frame, resume) = with_process_mut(child_pid, |p| (p.pml4_frame, p.pending_resume.take()))
        .ok_or("run_forked_child: no such child")?;
    let (resume_rip, resume_rsp) = resume.ok_or("run_forked_child: child has no pending resume point (already run once)")?;

    let _ = writeln!(
        serial(),
        "milestone 37: wait() -- switching CR3 to forked child pid {child_pid}'s own pml4 {:#x}, resuming at rip={:#x} rsp={:#x} (its own fork()-time snapshot, rax forced to 0)",
        pml4_frame.start_address().as_u64(),
        resume_rip,
        resume_rsp
    );
    ACTIVE_PROCESS.store(child_pid, Ordering::SeqCst);
    let flags = Cr3Flags::from_bits_truncate(KERNEL_CR3_FLAGS_BITS.load(Ordering::SeqCst));
    unsafe { Cr3::write(pml4_frame, flags) };

    // MILESTONE 37: real, diagnosed bug fix -- see
    // gdt::set_syscall_stack_top()'s own doc comment for the full
    // story (found via an actual page fault during real QEMU testing,
    // not guessed). Every syscall the CHILD makes from here on (this
    // milestone's own test child immediately calls exec(), then the
    // exec()'d program calls write()+exit()) needs to land on a
    // DIFFERENT ring-0 stack than the one the PARENT's own in-flight
    // wait() syscall is using, or the CPU's own next `int 0x80` would
    // silently overwrite the parent's saved resume state.
    let saved_stack_top = crate::gdt::set_syscall_stack_top(crate::gdt::child_excursion_stack_top());
    usertest::enter_ring3_as_forked_child(resume_rip, resume_rsp);
    crate::gdt::set_syscall_stack_top(saved_stack_top);

    let _ = writeln!(
        serial(),
        "milestone 37: wait() -- forked child pid {child_pid} resumed in kernel context (its own exit() syscall ran; CR3 is currently the KERNEL's own, restored to the parent's by wait_for_child() next)"
    );
    Ok(())
}

/// MILESTONE 43: real, checkable wait() outcome -- distinguishes a
/// child that ran to its own exit() from one that was SIGKILL-
/// equivalent-terminated before it ever ran, closing the real gap
/// M41 left open (wait_for_child() previously just returned the pid
/// or None, with no way to tell "exited normally" from "was killed"
/// from "never was my child at all").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// WIFEXITED-equivalent + WEXITSTATUS-equivalent: ran to its own
    /// real exit() syscall, carrying the real status code it passed.
    Exited(u8),
    /// The child was kill()ed (M41) before it ever ran -- never
    /// reached its own exit() at all.
    Killed,
    /// MILESTONE 53: WIFSIGNALED-equivalent -- the child actually ran
    /// (unlike `Killed`, which is pre-run only) and was terminated by a
    /// real hardware fault (SIGSEGV-equivalent page fault, or a #GP)
    /// instead of reaching its own exit(). Closes the real gap this
    /// milestone found: wait_for_child() previously had no way to tell
    /// this case apart from `Exited` at all, and reported a STALE exit
    /// code left over from a completely different, earlier child as if
    /// this one had genuinely exited normally. Honest scope-cut, same
    /// as M41's own: this doesn't distinguish page fault from #GP, or
    /// carry a real signal number -- both fault types funnel through
    /// the identical `terminate_faulted_process_and_resume_kernel()`
    /// path and this design has no signal-number concept yet.
    Signaled,
}

/// MILESTONE 43: side channel `kill()` populates so a LATER wait()
/// call can still learn a child was killed, even though kill() itself
/// (unchanged from M41, verified working, not touched here) frees the
/// PROCESS_TABLE slot IMMEDIATELY for reuse -- by the time wait()
/// might be called, the slot itself carries no trace the child ever
/// existed. Deliberately separate from PROCESS_TABLE for exactly that
/// reason: a real record that survives the slot being wiped/reused,
/// without weakening kill()'s own already-verified "instantly
/// reusable" contract (M41's self-test forks a SECOND child right
/// after killing the first, with no wait() in between, and that must
/// keep working unchanged).
static LAST_KILLED: Mutex<Option<(u8, u8)>> = Mutex::new(None); // (child_pid, parent_id)

/// MILESTONE 37: the `wait()` syscall's actual kernel-side
/// implementation -- see this module's own top-of-file MILESTONE 37 doc
/// comment for the real, honest semantics this implements. Real,
/// literal blocking for THIS milestone's synchronous execution model:
/// switches CR3 to the child, drives it all the way to ITS OWN exit()
/// (nested inside this syscall's own dispatch), then restores the
/// PARENT's own CR3/ACTIVE_PROCESS -- restore_kernel_cr3() (called by
/// the child's own exit()) leaves CR3 pointed at the KERNEL's PML4, not
/// the parent's, so this function's own explicit Cr3::write() back to
/// the parent's pml4_frame is required, not optional, for the parent's
/// OWN in-flight int 0x80 to correctly resume into its own private code
/// page afterward.
///
/// MILESTONE 43: returns `(child_pid, WaitOutcome)` on success --
/// `WaitOutcome::Exited(code)` with the child's own real exit code if
/// it ran to its own exit() and was reaped (removed from
/// PROCESS_TABLE, freeing its PML4/frames via Drop, and freeing the
/// slot for a later fork() to reuse), or `WaitOutcome::Killed` if it
/// was kill()ed (M41) before it ever ran (nothing to reap, the slot
/// was already freed by kill() itself). Returns `None` if `child_pid`
/// doesn't name a live child OF `parent_id` specifically AND wasn't
/// recently killed by `parent_id` either (a real ownership check: one
/// process cannot wait() on another's child) or nesting depth would
/// exceed this design's bound of 1.
pub(crate) fn wait_for_child(parent_id: u8, child_pid: u8) -> Option<(u8, WaitOutcome)> {
    if usertest::is_in_child_resume() {
        let _ = writeln!(
            serial(),
            "milestone 37: syscall WAIT (process {parent_id}) -- REFUSED, cannot wait() while already inside a nested child-resume excursion (nesting-depth-1 bound) -- returning failure"
        );
        return None;
    }
    if child_pid < PID_TABLE_BASE {
        return None;
    }
    let idx = (child_pid - PID_TABLE_BASE) as usize;
    if idx >= MAX_PROCESSES {
        return None;
    }
    // MILESTONE 43: check the kill side-channel FIRST -- if this exact
    // (child_pid, parent_id) pair was kill()ed, the PROCESS_TABLE slot
    // is already gone (possibly even reused by a new fork() since),
    // so this is the ONLY place that information still exists. Real
    // ownership check preserved: a parent can only learn about ITS
    // OWN killed child, matching every other authorization check in
    // this file.
    {
        let mut last_killed = LAST_KILLED.lock();
        if *last_killed == Some((child_pid, parent_id)) {
            *last_killed = None;
            let _ = writeln!(
                serial(),
                "milestone 43: syscall WAIT (process {parent_id}) -- pid {child_pid} was killed before it ever ran, reporting WaitOutcome::Killed"
            );
            return Some((child_pid, WaitOutcome::Killed));
        }
    }
    {
        let table = PROCESS_TABLE.lock();
        let child = table[idx].as_ref()?;
        if child.parent_pid != Some(parent_id) {
            let _ = writeln!(
                serial(),
                "milestone 37: syscall WAIT (process {parent_id}) -- REFUSED, pid {child_pid} is not a live child OF this process -- returning failure"
            );
            return None;
        }
    }

    let parent_pml4 = with_process_mut(parent_id, |p| p.pml4_frame)?;

    let run_result = run_forked_child(child_pid);

    // Restore the PARENT's own context regardless of whether the
    // nested child excursion itself succeeded -- the parent's own
    // in-flight `int 0x80` for THIS wait() call still needs to resume
    // correctly either way.
    let flags = Cr3Flags::from_bits_truncate(KERNEL_CR3_FLAGS_BITS.load(Ordering::SeqCst));
    unsafe { Cr3::write(parent_pml4, flags) };
    ACTIVE_PROCESS.store(parent_id, Ordering::SeqCst);

    if let Err(e) = run_result {
        let _ = writeln!(serial(), "milestone 37: syscall WAIT (process {parent_id}) -- FAILED running child pid {child_pid}: {e}");
        return None;
    }

    // MILESTONE 53: check the fault flag BEFORE reading the exit code --
    // run_forked_child() returns Ok(()) identically whether the child
    // reached its own real exit() OR was fault-terminated mid-execution
    // (both funnel through resume_kernel() the same way), so this flag
    // is the ONLY thing that tells the two apart. See
    // usertest::CHILD_FAULTED's own doc comment for the real,
    // previously-silent misreport this closes.
    if usertest::take_child_faulted() {
        // MILESTONE 54: `.take()` instead of `= None` -- see kill()'s
        // identical change just below in this file for the real reason
        // (captures the dead child so its frames can be reclaimed,
        // rather than silently dropping them).
        let old_proc = PROCESS_TABLE.lock()[idx].take();
        if let Some(p) = old_proc {
            reclaim_process_frames(p);
        }
        let _ = writeln!(
            serial(),
            "milestone 53: syscall WAIT (process {parent_id}) -- child pid {child_pid} was signal-terminated (real hardware fault, not its own exit()) and was reaped, CR3 restored to parent's own pml4 {:#x}",
            parent_pml4.start_address().as_u64()
        );
        return Some((child_pid, WaitOutcome::Signaled));
    }

    // MILESTONE 43: real exit code, captured by usertest.rs's exit
    // syscall arm the instant the child (just resumed above, via
    // run_forked_child()) reached ITS OWN exit(). Safe to read here
    // with no explicit reset needed: this design's nesting-depth-1
    // bound (enforced by is_in_child_resume()'s check at the top of
    // this function) means at most one child excursion is ever live,
    // so nothing else could have overwritten it since -- and the
    // MILESTONE 53 check just above already ruled out the one other
    // case (a fault) that could otherwise have left this stale.
    let exit_code = usertest::take_last_child_exit_code();

    // MILESTONE 54: same reclamation as the Signaled arm just above.
    let old_proc = PROCESS_TABLE.lock()[idx].take();
    if let Some(p) = old_proc {
        reclaim_process_frames(p);
    }
    let _ = writeln!(
        serial(),
        "milestone 43: syscall WAIT (process {parent_id}) -- child pid {child_pid} ran to completion and was reaped (real exit code {exit_code}), CR3 restored to parent's own pml4 {:#x}",
        parent_pml4.start_address().as_u64()
    );
    Some((child_pid, WaitOutcome::Exited(exit_code)))
}

/// MILESTONE 41: SIGKILL-equivalent -- unconditionally terminates a
/// live, never-yet-run forked child, bypassing the normal "run to its
/// own exit(), then reap" contract wait_for_child() enforces. Only the
/// child's REAL parent may kill it (same authorization check
/// wait_for_child() already makes). Structurally, any entry still
/// present in PROCESS_TABLE has, by construction, never been run --
/// wait_for_child() is the ONLY code that ever runs a forked child, and
/// it always reaps (clears) the slot immediately afterward -- so there
/// is no separate "already ran, not yet reaped" case to guard against
/// here.
///
/// **MILESTONE 54 update**: this used to disclose a real limitation
/// here -- frees the PROCESS_TABLE slot for reuse but does NOT reclaim
/// the child's physical frames (PML4/code/stack/heap), matching
/// wait_for_child()'s own identical, then-also-unfixed gap. As of
/// Milestone 54 both now actually call `reclaim_process_frames()`
/// before dropping the slot -- the frames go back to the global
/// allocator's free list for real reuse, not just a freed table index.
pub(crate) fn kill(caller_id: u8, target_pid: u8) -> bool {
    if target_pid < PID_TABLE_BASE {
        return false;
    }
    let idx = (target_pid - PID_TABLE_BASE) as usize;
    if idx >= MAX_PROCESSES {
        return false;
    }
    let mut table = PROCESS_TABLE.lock();
    let is_own_child = matches!(table[idx].as_ref(), Some(child) if child.parent_pid == Some(caller_id));
    if !is_own_child {
        let _ = writeln!(
            serial(),
            "milestone 41: syscall KILL (process {caller_id}) -- REFUSED, pid {target_pid} is not a live child OF this process -- returning failure"
        );
        return false;
    }
    // MILESTONE 54: `.take()` instead of `= None` -- captures the dead
    // child's Process value so its frames can actually be reclaimed
    // below, closing this function's own previously-disclosed
    // limitation (see its doc comment above, now out of date -- frames
    // ARE reclaimed as of this milestone).
    let old_proc = table[idx].take();
    // MILESTONE 43: record this kill in the side channel BEFORE
    // dropping the table lock, so a later wait() call can learn the
    // real outcome even though the slot above is already free for a
    // brand-new fork() to land in (unchanged M41 behavior -- this is
    // additive, not a replacement).
    *LAST_KILLED.lock() = Some((target_pid, caller_id));
    drop(table);
    if let Some(p) = old_proc {
        reclaim_process_frames(p);
    }
    let _ = writeln!(
        serial(),
        "milestone 41: syscall KILL (process {caller_id}) -- pid {target_pid} terminated WITHOUT ever running (bypassed wait()'s normal run-then-reap contract), slot freed for reuse"
    );
    true
}

/// MILESTONE 42: real process-group assignment -- syscall 14. A process
/// may set its OWN pgid (`target_pid == caller_id`, the common real-shell
/// case: "put myself in a new/existing group right after forking"), or a
/// live PARENT may set a live CHILD's pgid (same authorization check
/// `kill()` already makes: `child.parent_pid == Some(caller_id)`) --
/// mirroring how a real shell calls `setpgid()` on a just-forked child
/// from the parent's own side too, so the group is established before a
/// race with the child's own first instructions. Real, honest scope-cut:
/// this kernel has no controlling-terminal/session concept yet, so there
/// is no "only if this is the child's session leader's own session"
/// restriction real POSIX setpgid() also enforces -- deferred, not
/// silently dropped, same discipline as kill()'s own disclosed
/// frame-reclamation gap just above.
pub(crate) fn setpgid(caller_id: u8, target_pid: u8, new_pgid: u8) -> bool {
    if new_pgid == 0 {
        return false;
    }
    let authorized = if target_pid == caller_id {
        true
    } else if target_pid >= PID_TABLE_BASE && ((target_pid - PID_TABLE_BASE) as usize) < MAX_PROCESSES {
        let table = PROCESS_TABLE.lock();
        let idx = (target_pid - PID_TABLE_BASE) as usize;
        matches!(table[idx].as_ref(), Some(child) if child.parent_pid == Some(caller_id))
    } else {
        false
    };
    if !authorized {
        let _ = writeln!(
            serial(),
            "milestone 42: syscall SETPGID (process {caller_id}) -- REFUSED, pid {target_pid} is neither this process itself nor a live child OF it -- returning failure"
        );
        return false;
    }
    match with_process_mut(target_pid, |p| p.pgid = new_pgid) {
        Some(()) => {
            let _ = writeln!(
                serial(),
                "milestone 42: syscall SETPGID (process {caller_id}) -- pid {target_pid}'s pgid set to {new_pgid}"
            );
            true
        }
        None => {
            let _ = writeln!(
                serial(),
                "milestone 42: syscall SETPGID (process {caller_id}) -- FAILED, pid {target_pid} authorized but not actually a live process -- returning failure"
            );
            false
        }
    }
}

/// MILESTONE 42: real process-group query -- syscall 15. Returns
/// `target_pid`'s current pgid, or `None` if `target_pid` doesn't name a
/// live process. No authorization check -- a process's own pgid isn't
/// sensitive information, matching real Unix `getpgid()`'s own
/// unrestricted-read semantics.
pub(crate) fn getpgid(target_pid: u8) -> Option<u8> {
    with_process_mut(target_pid, |p| p.pgid)
}

/// MILESTONE 42: real, boot-time, non-interactive proof of process-group
/// assignment/inheritance/reassignment -- pure kernel-side calls
/// (fork()/setpgid()/getpgid()/kill(), all called directly, never via a
/// real `int 0x80`), so unlike self_test_signals() below this needs no
/// ring-3 entry at all and is safe to run EARLY, alongside
/// self_test_pipe_mechanics()/fs::self_test_disk_write() before
/// interrupts::init_pics()/sti() -- see Milestone 41's own hard-won
/// lesson (main.rs) for why that ordering matters for any self-test that
/// DOES enter ring 3, which this one deliberately avoids needing to.
/// Cleans up its own forked child via kill() before returning, so it
/// doesn't consume one of PROCESS_TABLE's MAX_PROCESSES=4 slots that
/// self_test_signals()'s own later, real SIGKILL test also needs.
pub fn self_test_process_groups() {
    let parent_pgid_before = getpgid(FORK_TEST_PROCESS_ID);
    let _ = writeln!(
        serial(),
        "milestone 42: self-test -- FORK_TEST_PROCESS's own pgid: {:?} (expected Some({FORK_TEST_PROCESS_ID}), founder of its own group)",
        parent_pgid_before
    );

    let child_pid = match fork(FORK_TEST_PROCESS_ID, 0, 0) {
        Some(pid) => pid,
        None => {
            let _ = writeln!(serial(), "milestone 42: self-test -- FAILED, fork() itself failed");
            return;
        }
    };
    let inherited = getpgid(child_pid);
    let inheritance_ok = inherited == Some(FORK_TEST_PROCESS_ID);
    let _ = writeln!(
        serial(),
        "milestone 42: self-test -- forked child pid {child_pid}, its pgid: {:?} (expected Some({FORK_TEST_PROCESS_ID}), real inheritance from the parent) -- {}",
        inherited,
        if inheritance_ok { "confirmed" } else { "MISMATCH" }
    );

    // Real proof this is genuine per-process state, not a shared/aliased
    // value: move the CHILD into its own new group (parent-authorizes-
    // child, the real setpgid() call site a shell makes right after
    // fork()) and confirm the PARENT's own pgid is completely unaffected.
    let moved = setpgid(FORK_TEST_PROCESS_ID, child_pid, child_pid);
    let child_pgid_after = getpgid(child_pid);
    let parent_pgid_after = getpgid(FORK_TEST_PROCESS_ID);
    let divergence_ok = moved && child_pgid_after == Some(child_pid) && parent_pgid_after == parent_pgid_before;
    let _ = writeln!(
        serial(),
        "milestone 42: self-test -- setpgid(child, child) result={moved}, child's pgid now {:?} (expected Some({child_pid})), parent's pgid still {:?} (expected unchanged {:?}) -- {}",
        child_pgid_after,
        parent_pgid_after,
        parent_pgid_before,
        if divergence_ok { "confirmed, real independent per-process state" } else { "MISMATCH" }
    );

    // Real proof the authorization rule actually rejects an unrelated
    // caller, not just documented as a rule -- PROCESS_A (pgid 1) is
    // neither this child's parent nor the child itself.
    let unauthorized_refused = !setpgid(1, child_pid, 42);
    let _ = writeln!(
        serial(),
        "milestone 42: self-test -- setpgid() from an unrelated process (PROCESS_A, not this child's parent) correctly refused: {unauthorized_refused}"
    );

    // Clean up: free the table slot for self_test_signals()'s own real
    // SIGKILL test right after this.
    let cleaned_up = kill(FORK_TEST_PROCESS_ID, child_pid);
    let _ = writeln!(
        serial(),
        "milestone 42: self-test -- cleanup kill(child) result={cleaned_up} (frees the PROCESS_TABLE slot for later self-tests)"
    );

    let all_ok = inheritance_ok && divergence_ok && unauthorized_refused && cleaned_up;
    let _ = writeln!(
        serial(),
        "milestone 42: self-test -- OVERALL: {}",
        if all_ok { "PASS" } else { "FAIL" }
    );
}

/// MILESTONE 41: real, boot-time, non-interactive proof of both new
/// signal behaviors -- sendkey is confirmed unreliable in this
/// environment (see fs::self_test_disk_write()'s own doc comment for
/// the identical reasoning), so this runs unattended on every boot
/// instead of depending on an interactive shell command.
pub fn self_test_signals() {
    let msg_ok = &SIGSEGV_TEST_PROGRAM[SIGSEGV_MSG_OFFSET..SIGSEGV_MSG_OFFSET + SIGSEGV_MSG.len()] == SIGSEGV_MSG;
    let _ = writeln!(
        serial(),
        "milestone 41: self-test -- SIGSEGV_TEST_PROGRAM layout check: message={msg_ok} -- {}",
        if msg_ok { "confirmed" } else { "MISMATCH" }
    );

    let _ = writeln!(
        serial(),
        "milestone 41: self-test -- running SIGSEGV_TEST_PROCESS, expecting a real page fault to terminate it without panicking the kernel..."
    );
    match run(SIGSEGV_TEST_PROCESS_ID) {
        Ok(()) => {
            let active_after = ACTIVE_PROCESS.load(Ordering::SeqCst);
            let _ = writeln!(
                serial(),
                "milestone 41: self-test -- run() returned normally after the fault (the kernel did not panic/hlt_loop) -- ACTIVE_PROCESS reset to {active_after} (expected 0)"
            );
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 41: self-test -- FAILED, run() itself errored: {e}");
        }
    }

    let _ = writeln!(
        serial(),
        "milestone 41: self-test -- running PROCESS_A right after the fault, real proof of genuine recovery, not just 'hasn't crashed yet'..."
    );
    match run(1) {
        Ok(()) => {
            let _ = writeln!(serial(), "milestone 41: self-test -- PROCESS_A ran normally after the SIGSEGV -- kernel genuinely recovered, not just still standing");
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 41: self-test -- FAILED, PROCESS_A could not run after the fault: {e}");
        }
    }

    let _ = writeln!(
        serial(),
        "milestone 41: self-test -- running SIGKILL_TEST_PROCESS (fork, kill without running, fork again to prove the slot was really freed)..."
    );
    match run(SIGKILL_TEST_PROCESS_ID) {
        Ok(()) => {
            let table = PROCESS_TABLE.lock();
            let occupied: alloc::vec::Vec<usize> = table
                .iter()
                .enumerate()
                .filter(|(_, p)| p.is_some())
                .map(|(i, _)| i)
                .collect();
            let _ = writeln!(
                serial(),
                "milestone 41: self-test -- SIGKILL_TEST_PROCESS completed -- PROCESS_TABLE occupied slots: {:?} (expected exactly [0]: the SECOND fork's child, since the FIRST was killed unrun and its slot reused)",
                occupied
            );
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 41: self-test -- FAILED, SIGKILL_TEST_PROCESS run() errored: {e}");
        }
    }
}

/// MILESTONE 43: real, boot-time, non-interactive proof of both new
/// WaitOutcome variants -- same "sendkey is unreliable, run unattended
/// instead" reasoning as every other self_test_* in this file. Must
/// run after interrupts::init_pics()/sti() in main.rs's boot sequence
/// (same real reason as self_test_signals(): this enters ring 3 via
/// wait_for_child() -> run_forked_child()).
pub fn self_test_wait_status() {
    // Part 1: a child that genuinely runs to its own real exit(42).
    // Forked from WAITSTATUS_TEST_PROCESS (whose entire code page IS
    // that exact exit call), resumed from offset 0 -- real, valid
    // (resume_rip, resume_rsp), not the (0,0) placeholder M42's own
    // self-test could get away with (that test kill()ed its child
    // before ever running it; this one must actually run).
    let stack_top = usertest::USER_STACK_ADDR + usertest::USER_STACK_SIZE;
    let exited_child = fork(WAITSTATUS_TEST_PROCESS_ID, usertest::USER_CODE_ADDR, stack_top);
    let exited_ok = match exited_child {
        Some(pid) => match wait_for_child(WAITSTATUS_TEST_PROCESS_ID, pid) {
            Some((reaped, WaitOutcome::Exited(code))) => {
                let ok = reaped == pid && code == 42;
                let _ = writeln!(
                    serial(),
                    "milestone 43: self-test -- forked+ran child pid {pid}, wait() reported Exited(reaped={reaped}, code={code}) (expected Exited({pid}, 42)) -- {}",
                    if ok { "confirmed" } else { "MISMATCH" }
                );
                ok
            }
            Some((_, WaitOutcome::Killed)) => {
                let _ = writeln!(serial(), "milestone 43: self-test -- FAILED, wait() reported Killed for a child that should have exited normally");
                false
            }
            Some((_, WaitOutcome::Signaled)) => {
                let _ = writeln!(serial(), "milestone 53: self-test -- FAILED, wait() reported Signaled for a child that should have exited normally (no fault in WAITSTATUS_TEST_PROGRAM)");
                false
            }
            None => {
                let _ = writeln!(serial(), "milestone 43: self-test -- FAILED, wait_for_child() itself returned None for the exit-status child");
                false
            }
        },
        None => {
            let _ = writeln!(serial(), "milestone 43: self-test -- FAILED, fork() itself failed for the exit-status child");
            false
        }
    };

    // Part 2: a child kill()ed before it ever runs -- real proof
    // wait() distinguishes this from a normal exit, not just refuses.
    // Dummy (0,0) resume point is genuinely fine here, same as M42's
    // own self-test: this child is never run() at all.
    let killed_ok = match fork(WAITSTATUS_TEST_PROCESS_ID, 0, 0) {
        Some(pid) => {
            let killed = kill(WAITSTATUS_TEST_PROCESS_ID, pid);
            match wait_for_child(WAITSTATUS_TEST_PROCESS_ID, pid) {
                Some((reaped, WaitOutcome::Killed)) => {
                    let ok = killed && reaped == pid;
                    let _ = writeln!(
                        serial(),
                        "milestone 43: self-test -- forked+killed child pid {pid}, wait() reported Killed(reaped={reaped}) -- {}",
                        if ok { "confirmed" } else { "MISMATCH" }
                    );
                    ok
                }
                Some((_, WaitOutcome::Exited(code))) => {
                    let _ = writeln!(serial(), "milestone 43: self-test -- FAILED, wait() reported Exited({code}) for a child that was killed, never ran");
                    false
                }
                Some((_, WaitOutcome::Signaled)) => {
                    let _ = writeln!(serial(), "milestone 53: self-test -- FAILED, wait() reported Signaled for a child that was killed pre-run, never actually ran to fault");
                    false
                }
                None => {
                    let _ = writeln!(serial(), "milestone 43: self-test -- FAILED, wait_for_child() returned None for the killed child");
                    false
                }
            }
        }
        None => {
            let _ = writeln!(serial(), "milestone 43: self-test -- FAILED, fork() itself failed for the killed child");
            false
        }
    };

    let _ = writeln!(
        serial(),
        "milestone 43: self-test -- OVERALL: {}",
        if exited_ok && killed_ok { "PASS" } else { "FAIL" }
    );
}

/// MILESTONE 45: the `exec()` syscall's REAL kernel-side implementation
/// -- replaces Milestone 37's own disclosed placeholder (which only
/// ever overwrote the SAME code frame's bytes in place, reusing the
/// calling process's existing PML4/stack/heap unchanged -- see this
/// project's own git history for that version). This is the actual
/// "generalized ELF loader" milestone: a genuinely fresh private
/// address space -- new PML4, new code/stack/heap physical frames, one
/// physical page per PT_LOAD segment page -- built by
/// `create_process_from_elf()`, the EXACT SAME function `runelf`
/// (`load_and_run_elf()`) and `fork()`'s own child-builder
/// (indirectly, via `create_process_from_image()`'s sibling) already
/// use, not a duplicated loader. The calling process's OLD pml4/code/
/// stack/heap frames are replaced -- as of Milestone 54, genuinely
/// reclaimed (see this function's own teardown comment below), not just
/// dropped -- and CR3 is switched to the new address space before
/// returning, exactly like `load_and_run_elf()` does for a brand-new
/// process.
///
/// Real POSIX exec() contract, checked field by field:
///   - SAME pid (the process's own table slot is replaced in place, not
///     removed and recreated under a new id).
///   - SAME open file descriptors (`fds`, snapshotted BEFORE the new
///     `Process` is built, then written into it afterward) -- a real
///     shell's `cmd < file` / `cmd > file` redirection depends on the
///     exec()'d program inheriting fds the shell itself opened before
///     calling exec(), exactly this.
///   - SAME parent_pid and pgid (a process's ancestry and process-group
///     membership are real Unix invariants exec() must never change).
///   - a genuinely NEW address space and a genuinely NEW entry point --
///     `entry` is now the target ELF's own real, parsed `e_entry`
///     (Milestone 44's generalization, exercised for real here for the
///     first time by an exec() call rather than only a fresh `runelf`),
///     NOT forced back to `USER_CODE_ADDR`.
///   - `heap_used` reset to 0 (a fresh program gets a fresh heap,
///     already `create_process_from_elf()`'s own default for a
///     brand-new process).
///
/// **MILESTONE 54 update**: this doc comment used to disclose a real
/// limitation here, matching `kill()`/`wait_for_child()`'s own
/// then-identical gap -- the old process's pml4/code/stack/heap/extra
/// physical frames were replaced but never reclaimed, because this
/// kernel's frame allocator had never freed a physical frame on process
/// exit, reap, or kill at all. As of Milestone 54, `replace_process()`
/// (called below via `replace_process(id, new_proc)`) captures the OLD
/// `Process` value via `Mutex::replace()` instead of discarding it, and
/// calls `reclaim_process_frames()` on it -- the real "teardown" half of
/// "teardown-and-rebuild" now means "returned to the free list", not
/// just "stopped referencing".
pub(crate) fn exec_elf(id: u8, image: &[u8], elf_image: &elf::ElfImage) -> Result<u64, &'static str> {
    // Snapshot exactly what real exec() must carry across unchanged --
    // each fallible/heap-touching read its own statement, same reasoning
    // as fork()'s own snapshot (see that function's own comment on why
    // combining these into one expression broke this kernel's
    // panic=abort, no_std build).
    let old_fds = with_process_mut(id, |p| p.fds.clone()).ok_or("exec: no such process")?;
    let old_parent_pid = with_process_mut(id, |p| p.parent_pid).ok_or("exec: no such process")?;
    let old_pgid = with_process_mut(id, |p| p.pgid).ok_or("exec: no such process")?;

    let phys_mem_offset = memory::phys_mem_offset();
    let build_result = memory::with_frame_allocator(|frame_allocator| {
        create_process_from_elf(frame_allocator, phys_mem_offset, "exec'd", elf_image.entry, &elf_image.segments, image)
    });
    let mut new_proc = match build_result {
        Some(Ok(p)) => p,
        Some(Err(e)) => {
            let _ = writeln!(serial(), "milestone 45: syscall EXEC (process {id}) -- FAILED building the new address space: {e}");
            return Err(e);
        }
        None => {
            let _ = writeln!(serial(), "milestone 45: syscall EXEC (process {id}) -- FAILED, global frame allocator not installed (should never happen post-boot)");
            return Err("exec: global frame allocator not installed yet");
        }
    };

    new_proc.fds = old_fds;
    new_proc.parent_pid = old_parent_pid;
    new_proc.pgid = old_pgid;

    let new_pml4 = new_proc.pml4_frame;
    let new_entry = new_proc.entry;

    // The real teardown-and-rebuild step: this REPLACES the slot's
    // entire Process value (old pml4/code/stack/heap dropped -- see this
    // function's own doc comment above for the honest "dropped, not
    // reclaimed" caveat), not a field-by-field mutation of the old one.
    if !replace_process(id, new_proc) {
        return Err("exec: no such process (slot vanished during rebuild)");
    }

    // Switch CR3 to the NEW address space now, before returning to
    // usertest.rs's syscall dispatch -- exactly like load_and_run_elf()
    // does right after building a brand-new process, and required here
    // for the same reason: the old PML4 this process was running under a
    // moment ago no longer exists in any Process struct this kernel
    // tracks, so continuing to run under it (even briefly) would be
    // pointing CR3 at now-untracked frames.
    let flags = Cr3Flags::from_bits_truncate(KERNEL_CR3_FLAGS_BITS.load(Ordering::SeqCst));
    unsafe { Cr3::write(new_pml4, flags) };

    let _ = writeln!(
        serial(),
        "milestone 45: syscall EXEC (process {id}) -- REAL teardown-and-rebuild complete: new pml4={:#x}, new entry={:#x} (real parsed e_entry), fd table/parent_pid/pgid preserved, CR3 switched",
        new_pml4.start_address().as_u64(),
        new_entry
    );
    Ok(new_entry)
}

/// MILESTONE 58: real, disclosed, checked-before-any-write caps on
/// argv/envp support -- the single 4KiB user stack page every process
/// gets (`usertest::USER_STACK_SIZE`, unchanged since Milestone 30) has
/// to hold this milestone's header+string region AND whatever real
/// stack space the newly exec()'d program needs at runtime, so this
/// deliberately stays small rather than claiming the whole page.
/// `build_argv_envp_stack()` below rejects (with a clear `Err`, never a
/// silent truncation or an out-of-bounds write) anything that doesn't
/// fit inside these caps, or that doesn't fit in the page at all even
/// within them.
pub(crate) const EXEC_ARGV_MAX_COUNT: usize = 8;
pub(crate) const EXEC_ARG_MAX_LEN: usize = 128;

/// MILESTONE 58: lays out a real argv/envp block on a NEWLY built
/// process's own private stack page, following the actual x86_64 SysV
/// process-entry stack contract a real kernel's `execve()` uses (this
/// is what a real ELF binary's `_start` expects to find at the initial
/// RSP it's handed, NOT a `call`-return-address convention): from the
/// returned RSP upward, `argc` (8 bytes), then `argc` real pointers
/// into this same page (one per argv string), a NULL (0) terminator,
/// then `envp.len()` real pointers (one per envp string), a second
/// NULL terminator -- with the actual NUL-terminated string bytes
/// themselves packed at the TOP of the page, argv's strings first then
/// envp's. Returns the new value the new process's ring-3 entry should
/// use as its initial RSP: `usertest::USER_STACK_ADDR + header_start`,
/// 16-byte aligned (the real alignment a process's initial stack
/// pointer must have per the ABI -- checked and enforced here, not
/// assumed).
///
/// Writes through the direct `phys_mem_offset` view of the NEW
/// process's own already-allocated `stack_frame` -- the exact same
/// technique `fork_build_child()` already uses to copy a parent's real
/// stack bytes into a child's own frame (see that function's own doc
/// comment) -- valid regardless of which CR3 happens to be loaded right
/// now, so this can run (and does, from `exec_elf_with_args()` below)
/// BEFORE the CR3 switch into the new address space.
fn build_argv_envp_stack(
    phys_mem_offset: VirtAddr,
    stack_frame: PhysFrame<Size4KiB>,
    argv: &[Vec<u8>],
    envp: &[Vec<u8>],
) -> Result<u64, &'static str> {
    if argv.len() > EXEC_ARGV_MAX_COUNT {
        return Err("exec: argv entry count exceeds this loader's fixed cap");
    }
    if envp.len() > EXEC_ARGV_MAX_COUNT {
        return Err("exec: envp entry count exceeds this loader's fixed cap");
    }
    for s in argv.iter().chain(envp.iter()) {
        if s.len() > EXEC_ARG_MAX_LEN {
            return Err("exec: an argv/envp string exceeds this loader's fixed per-string cap");
        }
    }

    let page_size: u64 = PAGE_SIZE as u64;
    let stack_base_virt = phys_mem_offset + stack_frame.start_address().as_u64();

    // Real layout, fully computed BEFORE any write happens -- pack
    // every string (argv's, in order, then envp's, in order), each
    // NUL-terminated, recording each one's own BYTE OFFSET within this
    // packed region so the pointer table below can compute its real
    // final virtual address.
    let mut string_bytes: Vec<u8> = Vec::new();
    let mut argv_offsets: Vec<u64> = Vec::with_capacity(argv.len());
    let mut envp_offsets: Vec<u64> = Vec::with_capacity(envp.len());
    for s in argv {
        argv_offsets.push(string_bytes.len() as u64);
        string_bytes.extend_from_slice(s);
        string_bytes.push(0);
    }
    for s in envp {
        envp_offsets.push(string_bytes.len() as u64);
        string_bytes.extend_from_slice(s);
        string_bytes.push(0);
    }

    let header_len: u64 = 8 // argc
        + (argv.len() as u64 + 1) * 8 // argv pointers + NULL terminator
        + (envp.len() as u64 + 1) * 8; // envp pointers + NULL terminator
    let strings_len = string_bytes.len() as u64;

    let strings_start = page_size
        .checked_sub(strings_len)
        .ok_or("exec: argv/envp string bytes too large for the single-page stack")?;
    let header_start_unaligned = strings_start
        .checked_sub(header_len)
        .ok_or("exec: argv/envp header too large for the single-page stack")?;
    // Real SysV process-entry alignment requirement: align DOWN to 16,
    // never up (up could overlap into the string region computed
    // above) -- the small gap this can open between the header's real
    // end and strings_start is real, harmless padding, never read by
    // anything (the new program only ever dereferences the exact
    // pointers this function itself hands it).
    let header_start = header_start_unaligned & !0xFu64;
    if header_start < 8 {
        return Err("exec: argv/envp layout does not fit on the single-page stack even after alignment");
    }

    unsafe {
        core::ptr::copy_nonoverlapping(
            string_bytes.as_ptr(),
            (stack_base_virt + strings_start).as_mut_ptr::<u8>(),
            string_bytes.len(),
        );
    }

    let string_user_addr = |off: u64| usertest::USER_STACK_ADDR + strings_start + off;

    let mut cursor = header_start;
    unsafe { core::ptr::write((stack_base_virt + cursor).as_mut_ptr::<u64>(), argv.len() as u64) };
    cursor += 8;
    for &off in &argv_offsets {
        unsafe { core::ptr::write((stack_base_virt + cursor).as_mut_ptr::<u64>(), string_user_addr(off)) };
        cursor += 8;
    }
    unsafe { core::ptr::write((stack_base_virt + cursor).as_mut_ptr::<u64>(), 0u64) }; // argv NULL terminator
    cursor += 8;
    for &off in &envp_offsets {
        unsafe { core::ptr::write((stack_base_virt + cursor).as_mut_ptr::<u64>(), string_user_addr(off)) };
        cursor += 8;
    }
    unsafe { core::ptr::write((stack_base_virt + cursor).as_mut_ptr::<u64>(), 0u64) }; // envp NULL terminator

    Ok(usertest::USER_STACK_ADDR + header_start)
}

/// MILESTONE 58: real argv/envp threading through `exec()` -- the
/// "real exec with argv/envp" item the README's own Tier 1 roadmap
/// names, closed as its own additive entry point rather than a
/// modification of Milestone 45's `exec_elf()` above (kept completely
/// untouched, still exercised by the existing EXEC_TEST_PROCESS self
/// -test exactly as before -- zero regression risk to that already-
/// verified path). Does everything `exec_elf()` does (real teardown-
/// and-rebuild address space, fd table/parent_pid/pgid preserved
/// across the call, CR3 switched at the end) PLUS the one real thing
/// `exec_elf()` never had: a caller-supplied argv/envp array laid out
/// on the NEW program's own initial stack via `build_argv_envp_stack()`
/// above, per the real x86_64 SysV process-entry contract -- not
/// registers, not a kernel-private side channel a real `execve()`
/// wouldn't have. Returns `(new_entry, new_user_rsp)` since the caller
/// (usertest.rs's new EXECARGV syscall arm) needs BOTH to enter the new
/// program correctly (`exec_elf()`'s callers all reuse the fixed
/// `USER_STACK_ADDR + USER_STACK_SIZE` top-of-stack instead, since they
/// never populate anything below it).
pub(crate) fn exec_elf_with_args(
    id: u8,
    image: &[u8],
    elf_image: &elf::ElfImage,
    argv: Vec<Vec<u8>>,
    envp: Vec<Vec<u8>>,
) -> Result<(u64, u64), &'static str> {
    let old_fds = with_process_mut(id, |p| p.fds.clone()).ok_or("exec: no such process")?;
    let old_parent_pid = with_process_mut(id, |p| p.parent_pid).ok_or("exec: no such process")?;
    let old_pgid = with_process_mut(id, |p| p.pgid).ok_or("exec: no such process")?;

    let phys_mem_offset = memory::phys_mem_offset();
    let build_result = memory::with_frame_allocator(|frame_allocator| {
        create_process_from_elf(frame_allocator, phys_mem_offset, "exec'd-argv", elf_image.entry, &elf_image.segments, image)
    });
    let mut new_proc = match build_result {
        Some(Ok(p)) => p,
        Some(Err(e)) => {
            let _ = writeln!(serial(), "milestone 58: syscall EXECARGV (process {id}) -- FAILED building the new address space: {e}");
            return Err(e);
        }
        None => {
            let _ = writeln!(
                serial(),
                "milestone 58: syscall EXECARGV (process {id}) -- FAILED, global frame allocator not installed (should never happen post-boot)"
            );
            return Err("exec: global frame allocator not installed yet");
        }
    };

    new_proc.fds = old_fds;
    new_proc.parent_pid = old_parent_pid;
    new_proc.pgid = old_pgid;

    let new_pml4 = new_proc.pml4_frame;
    let new_entry = new_proc.entry;
    let new_stack_frame = new_proc.stack_frame;
    let argv_count = argv.len();
    let envp_count = envp.len();

    // Real argv/envp stack construction happens BEFORE replace_process()
    // moves new_proc away and BEFORE the CR3 switch below -- see
    // build_argv_envp_stack()'s own doc comment for why that ordering
    // is safe (it writes through phys_mem_offset directly, independent
    // of the currently-loaded CR3).
    let new_rsp = match build_argv_envp_stack(phys_mem_offset, new_stack_frame, &argv, &envp) {
        Ok(rsp) => rsp,
        Err(e) => {
            // MILESTONE 58: new_proc's pml4/code/stack/heap frames were
            // already really allocated by create_process_from_elf()
            // above -- reclaim them for real (Milestone 54's own
            // mechanism) rather than letting this Err path silently
            // leak them, the exact bug class M54 itself closed for
            // every OTHER process-replacement path in this file.
            reclaim_process_frames(new_proc);
            let _ = writeln!(
                serial(),
                "milestone 58: syscall EXECARGV (process {id}) -- FAILED laying out argv/envp on the new stack: {e} -- new address space's frames reclaimed, OLD program continues running"
            );
            return Err(e);
        }
    };

    if !replace_process(id, new_proc) {
        return Err("exec: no such process (slot vanished during rebuild)");
    }

    let flags = Cr3Flags::from_bits_truncate(KERNEL_CR3_FLAGS_BITS.load(Ordering::SeqCst));
    unsafe { Cr3::write(new_pml4, flags) };

    let _ = writeln!(
        serial(),
        "milestone 58: syscall EXECARGV (process {id}) -- REAL teardown-and-rebuild complete WITH argv/envp: new pml4={:#x}, new entry={:#x} (real parsed e_entry), new rsp={:#x} ({argv_count} argv string(s), {envp_count} envp string(s), real SysV process-entry stack layout), fd table/parent_pid/pgid preserved, CR3 switched",
        new_pml4.start_address().as_u64(),
        new_entry,
        new_rsp
    );
    Ok((new_entry, new_rsp))
}

/// MILESTONE 53: real, boot-time, non-interactive proof of the new
/// `WaitOutcome::Signaled` variant -- same "sendkey is unreliable, run
/// unattended instead" reasoning as every other self_test_* in this
/// file. Must run after interrupts::init_pics()/sti() in main.rs's boot
/// sequence, same real reason as self_test_signals()/self_test_wait_
/// status(): this enters ring 3 via wait_for_child() -> run_forked_
/// child() -> enter_ring3_as_forked_child(), which sets RFLAGS.IF=1 the
/// same way top-level entry does.
///
/// The real bug this closes, found by direct code inspection while
/// finishing Milestone 44 (generalized ELF entry) and re-reading
/// wait_for_child() for MILESTONE 53's own dependency-chain check:
/// before this milestone, a forked child that page-faulted (or
/// #GP-faulted) instead of calling its own exit() left LAST_CHILD_
/// EXIT_CODE completely untouched -- run_forked_child() returns Ok(())
/// identically either way, since both a real exit() and a real fault
/// funnel through resume_kernel() the same way -- so wait_for_child()
/// silently read out whatever STALE exit code an earlier, unrelated
/// child's real exit() last wrote (or the AtomicU8 default, 0, on the
/// very first wait() of a boot) and reported `WaitOutcome::Exited
/// (stale_code)`, mislabeling a crashed child as one that exited
/// normally. This self-test is what actually PROVES the fix, not just
/// the code review that found the gap: it deliberately forks a child
/// whose entire program is a single faulting instruction (no exit()
/// call anywhere in it) and checks wait() reports `Signaled`, not
/// `Exited` with a leftover code from a completely different child.
pub fn self_test_fault_status() {
    let stack_top = usertest::USER_STACK_ADDR + usertest::USER_STACK_SIZE;

    let _ = writeln!(
        serial(),
        "milestone 53: self-test -- forking FAULT_TEST_PROCESS's child (its entire code page is a single faulting instruction, no exit() anywhere), expecting wait() to report Signaled, not a stale Exited..."
    );
    let faulted_ok = match fork(FAULT_TEST_PROCESS_ID, usertest::USER_CODE_ADDR, stack_top) {
        Some(pid) => match wait_for_child(FAULT_TEST_PROCESS_ID, pid) {
            Some((reaped, WaitOutcome::Signaled)) => {
                let ok = reaped == pid;
                let _ = writeln!(
                    serial(),
                    "milestone 53: self-test -- forked+faulted child pid {pid}, wait() reported Signaled(reaped={reaped}) -- {}",
                    if ok { "confirmed" } else { "MISMATCH" }
                );
                ok
            }
            Some((reaped, WaitOutcome::Exited(code))) => {
                let _ = writeln!(
                    serial(),
                    "milestone 53: self-test -- FAILED, wait() reported Exited(reaped={reaped}, code={code}) for a child that never called exit() at all -- this is EXACTLY the stale-exit-code bug this milestone exists to fix"
                );
                false
            }
            Some((_, WaitOutcome::Killed)) => {
                let _ = writeln!(serial(), "milestone 53: self-test -- FAILED, wait() reported Killed for a child that actually ran (and faulted), not one killed pre-run");
                false
            }
            None => {
                let _ = writeln!(serial(), "milestone 53: self-test -- FAILED, wait_for_child() itself returned None for the faulted child");
                false
            }
        },
        None => {
            let _ = writeln!(serial(), "milestone 53: self-test -- FAILED, fork() itself failed for the faulted child");
            false
        }
    };

    // Real proof of genuine recovery, not just "hasn't crashed yet" --
    // same discipline as M41's own SIGSEGV self-test: run an entirely
    // unrelated top-level process (PROCESS_A) right after and confirm
    // it's unaffected by the fault-during-wait() path above.
    let _ = writeln!(
        serial(),
        "milestone 53: self-test -- running PROCESS_A right after, real proof the kernel genuinely recovered from a fault inside a nested wait() excursion specifically (not just a top-level fault, which M41 already covers)..."
    );
    let recovery_ok = match run(1) {
        Ok(()) => {
            let _ = writeln!(serial(), "milestone 53: self-test -- PROCESS_A ran normally after the nested-wait() fault -- kernel genuinely recovered");
            true
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 53: self-test -- FAILED, PROCESS_A could not run after the fault: {e}");
            false
        }
    };

    // Real proof the PROCESS_TABLE slot was actually freed by the
    // Signaled reap above, not just reported correctly -- fork a SECOND
    // child from the same source and confirm the slot is reusable, same
    // "fork again to prove reuse" discipline as M41's own SIGKILL
    // self-test. Dummy (0,0) resume point + immediate kill() is
    // deliberate here (same M42/M43 precedent): the point is slot
    // bookkeeping, not a second real ring-3 excursion.
    //
    // Checks only THIS test's own slot, not that PROCESS_TABLE is
    // globally empty -- M41's own SIGKILL_TEST_PROCESS self-test
    // (process.rs, ~line 3026) deliberately leaves its own second
    // forked child permanently unreaped in slot 0 for the rest of the
    // boot ("expected exactly [0]"), so asserting global emptiness here
    // was a real, false assumption about unrelated earlier-milestone
    // state, not a genuine slot-reuse bug -- caught via an actual QEMU
    // boot (occupied=[0] every run), not by inspection.
    let reuse_ok = match fork(FAULT_TEST_PROCESS_ID, 0, 0) {
        Some(second_pid) => {
            let killed = kill(FAULT_TEST_PROCESS_ID, second_pid);
            let own_idx = (second_pid - PID_TABLE_BASE) as usize;
            let table = PROCESS_TABLE.lock();
            let occupied: alloc::vec::Vec<usize> = table.iter().enumerate().filter(|(_, p)| p.is_some()).map(|(i, _)| i).collect();
            drop(table);
            let own_slot_freed = !occupied.contains(&own_idx);
            let _ = writeln!(
                serial(),
                "milestone 53: self-test -- second fork() from FAULT_TEST_PROCESS_ID succeeded (pid {second_pid}, killed={killed}) -- real proof the first faulted child's slot was genuinely freed, not just marked. PROCESS_TABLE occupied slots after cleanup: {:?}, own slot {own_idx} freed={own_slot_freed} (other slots may be legitimately occupied by unrelated earlier milestones' own self-tests, e.g. M41's SIGKILL_TEST_PROCESS)",
                occupied
            );
            killed && own_slot_freed
        }
        None => {
            let _ = writeln!(serial(), "milestone 53: self-test -- FAILED, second fork() from FAULT_TEST_PROCESS_ID did not succeed -- the first faulted child's slot may not have been freed");
            false
        }
    };

    let _ = writeln!(
        serial(),
        "milestone 53: self-test -- OVERALL: {}",
        if faulted_ok && recovery_ok && reuse_ok { "PASS" } else { "FAIL" }
    );
}

/// MILESTONE 57: real, boot-time, non-interactive proof that per-process
/// heap pages are genuinely demand-paged -- not eagerly mapped at
/// process-creation time, and mapped ONLY when a real hardware page
/// fault legitimately earns it. Two real, independent halves, proving
/// both directions (same "prove both directions" discipline as
/// Milestone 39's gate/evidence self-test):
///
///   1. POSITIVE case (DEMAND_PAGE_TEST_PROCESS): before it ever runs,
///      EVERY one of its `HEAP_PAGE_COUNT` heap_frames slots is confirmed
///      `None` -- real, direct proof this process was created with ZERO
///      heap frames mapped (the actual point of this milestone). It then
///      genuinely runs in ring 3, `sbrk()`s past the OLD 16 KiB cap, and
///      touches the LAST page of its new 64-page reservation -- a real
///      hardware #PF this milestone's page_fault_handler change must
///      resolve transparently, mid-instruction, with no crash. Checked
///      afterward: `heap_frames[63]` is now `Some`, and the actual
///      physical byte it points at (read directly through
///      phys_mem_offset, not trusted from the process's own say-so) is
///      exactly the marker byte the program wrote AFTER the fault
///      resolved -- real, end-to-end proof the retried instruction
///      genuinely completed. `heap_frames[0]`, never touched by this
///      program, is confirmed to STILL be `None` afterward -- proof
///      demand paging is genuinely per-page, not "the whole reservation
///      gets mapped on the first fault".
///   2. NEGATIVE case (DEMAND_PAGE_OOB_TEST_PROCESS): targets the SAME
///      page index (63) the positive case just proved demand-pages
///      correctly -- except this process's own `heap_used` never grew
///      anywhere near it (a small, ordinary 64-byte `sbrk()`), so the
///      identical virtual address is a genuinely illegal access here:
///      still inside the heap's reserved range, but nowhere near what
///      THIS process actually committed. `run()` itself cannot
///      distinguish "ran to completion" from "faulted and was gracefully
///      terminated" (both converge on `Ok(())` -- see Milestone 44's own
///      doc comment on exactly this), so the real check is
///      `heap_frames[63]` staying `None` afterward (demand paging
///      correctly refused it) AND an entirely unrelated top-level
///      process (PROCESS_A) running normally right afterward -- real
///      proof Milestone 41's UNMODIFIED SIGSEGV path is what actually
///      caught it, not a silent hang or a corrupted kernel.
pub fn self_test_demand_paging_heap() {
    let phys_mem_offset = memory::phys_mem_offset();
    let far_page: usize = (DEMAND_PAGE_TEST_HEAP_OFFSET / PAGE_SIZE as u64) as usize;

    // Sanity-check this test's own arithmetic before trusting it, same
    // discipline as SIGSEGV_MSG_OFFSET's own pre-flight check elsewhere
    // in this file.
    let offset_ok = far_page == 63 && DEMAND_PAGE_TEST_HEAP_OFFSET == 63 * PAGE_SIZE as u64;
    let _ = writeln!(
        serial(),
        "milestone 57: self-test -- layout check: far heap page index={far_page} (expect 63), offset={:#x} -- {}",
        DEMAND_PAGE_TEST_HEAP_OFFSET,
        if offset_ok { "confirmed" } else { "MISMATCH" }
    );

    // -- POSITIVE CASE --------------------------------------------------
    let pre_run_all_none = with_process_mut(DEMAND_PAGE_TEST_PROCESS_ID, |p| p.heap_frames.iter().all(|f| f.is_none()));
    let _ = writeln!(
        serial(),
        "milestone 57: self-test -- BEFORE running DEMAND_PAGE_TEST_PROCESS: all {} heap slots unmapped -- {:?} (expect Some(true), real proof create_process_from_image() no longer eagerly maps any heap page)",
        HEAP_PAGE_COUNT,
        pre_run_all_none
    );

    let ran_ok = match run(DEMAND_PAGE_TEST_PROCESS_ID) {
        Ok(()) => {
            let _ = writeln!(serial(), "milestone 57: self-test -- DEMAND_PAGE_TEST_PROCESS ran and returned to the kernel (run() returned Ok)");
            true
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 57: self-test -- FAILED, DEMAND_PAGE_TEST_PROCESS could not even run: {e}");
            false
        }
    };

    let (page_now_mapped, marker_correct) = with_process_mut(DEMAND_PAGE_TEST_PROCESS_ID, |p| match p.heap_frames[far_page] {
        Some(frame) => {
            let byte = unsafe { core::ptr::read((phys_mem_offset + frame.start_address().as_u64()).as_ptr::<u8>()) };
            (true, byte == DEMAND_PAGE_TEST_MARKER)
        }
        None => (false, false),
    })
    .unwrap_or((false, false));

    let _ = writeln!(
        serial(),
        "milestone 57: self-test -- AFTER running: heap_frames[{far_page}] mapped={page_now_mapped}, marker byte correct={marker_correct} (expect true, true -- a real physical frame was demand-paged for the FAR page specifically, and real content survived the fault+retry)"
    );

    let page0_still_none = with_process_mut(DEMAND_PAGE_TEST_PROCESS_ID, |p| p.heap_frames[0].is_none()).unwrap_or(false);
    let _ = writeln!(
        serial(),
        "milestone 57: self-test -- heap_frames[0] still unmapped={page0_still_none} (expect true -- proves demand paging is genuinely per-page, not per-reservation)"
    );

    let positive_ok = pre_run_all_none == Some(true) && ran_ok && page_now_mapped && marker_correct && page0_still_none;

    // -- NEGATIVE CASE ----------------------------------------------------
    let _ = writeln!(
        serial(),
        "milestone 57: self-test -- running DEMAND_PAGE_OOB_TEST_PROCESS: same target page ({far_page}) as the positive case, but this process never sbrk()'d anywhere near it -- expecting a real SIGSEGV, not a silent demand-page"
    );
    let oob_run_result = run(DEMAND_PAGE_OOB_TEST_PROCESS_ID);
    let oob_page_stayed_unmapped = with_process_mut(DEMAND_PAGE_OOB_TEST_PROCESS_ID, |p| p.heap_frames[far_page].is_none()).unwrap_or(false);
    let _ = writeln!(
        serial(),
        "milestone 57: self-test -- DEMAND_PAGE_OOB_TEST_PROCESS: run() returned {:?}, heap_frames[{far_page}] still unmapped={oob_page_stayed_unmapped} (expect true -- the committed-vs-reserved boundary was actually enforced, not silently allowed through)",
        oob_run_result
    );

    // Real proof of genuine recovery, not just "hasn't crashed yet" --
    // same discipline as Milestones 41/53's own self-tests.
    let recovery_ok = match run(1) {
        Ok(()) => {
            let _ = writeln!(serial(), "milestone 57: self-test -- PROCESS_A ran normally right after the OOB fault -- kernel genuinely recovered");
            true
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 57: self-test -- FAILED, PROCESS_A could not run after the OOB fault: {e}");
            false
        }
    };

    let negative_ok = oob_run_result.is_ok() && oob_page_stayed_unmapped && recovery_ok;

    let _ = writeln!(
        serial(),
        "milestone 57: self-test -- OVERALL: {}",
        if offset_ok && positive_ok && negative_ok { "PASS" } else { "FAIL" }
    );
}

/// MILESTONE 64: real, boot-time, non-interactive proof of the whole
/// read-only file-backed `mmap()` slice -- four real ring-3 excursions,
/// each proving a genuinely different piece:
///
///   1. MMAP_READ_TEST_PROCESS (positive): a real not-present fault on
///      first touch is transparently resolved with REAL file content
///      (checked byte-for-byte against MMAP_TEST_FILE_CONTENT, read
///      directly through the process's own heap physical frame, never
///      trusted from the process's own say-so -- same discipline
///      self_test_demand_paging_heap() already established), three more
///      reads against the SAME already-mapped page succeed without
///      re-faulting, and a real munmap() both succeeds (result byte 0)
///      AND clears the process's own mmap slot back to `None`.
///   2. MMAP_WRITE_BEFORE_READ_FAULT_PROCESS (negative #1): a write to a
///      NEVER-touched mapping is refused at the not-present+write
///      decision point (`try_demand_page_mmap()`'s own `is_write` check)
///      -- checked directly: the slot's `frame` stays `None`.
///   3. MMAP_WRITE_AFTER_READ_FAULT_PROCESS (negative #2): a write to an
///      ALREADY-mapped (via a prior real read) page is refused by a
///      structurally different path -- a genuine hardware
///      PROTECTION_VIOLATION, excluded from ever reaching
///      `try_demand_page_mmap()` at all by `page_fault_handler()`'s own
///      pre-existing guard. Checked directly: the FIRST heap marker (the
///      real read) is present, the SECOND (a marker only reachable by
///      surviving the write) is absent.
///   4. MMAP_USE_AFTER_UNMAP_FAULT_PROCESS (negative #3): a second READ
///      at an address already `munmap()`'d is refused -- proving
///      `munmap()` genuinely cleared the slot (not just freed the frame),
///      so a stale address can't be silently re-granted. Checked
///      directly: the first two heap markers (read, then munmap result)
///      are present, the third (only reachable by surviving the second
///      read) is absent.
///
/// Every case ends with PROCESS_A run directly afterward -- same real
/// "the kernel genuinely recovered, not just didn't crash yet" proof
/// self_test_demand_paging_heap()'s own negative case already
/// established.
pub fn self_test_mmap() {
    let phys_mem_offset = memory::phys_mem_offset();

    // -- Layout sanity check (same discipline as FDTEST_PROGRAM's own
    // path1_ok/path2_ok checks and DEMAND_PAGE_TEST_HEAP_OFFSET's own
    // pre-flight check) -- confirms each program's embedded path bytes
    // really do live at the offset this file's own constants claim,
    // before trusting anything downstream.
    let path_bytes = MMAP_TEST_FILE_PATH.as_bytes();
    let layout_ok = &MMAP_READ_TEST_PROGRAM[MMAP_READ_TEST_PATH_OFFSET..MMAP_READ_TEST_PATH_OFFSET + path_bytes.len()] == path_bytes
        && &MMAP_WRITE_BEFORE_READ_FAULT_PROGRAM[MMAP_WRITE_BEFORE_READ_FAULT_PATH_OFFSET..MMAP_WRITE_BEFORE_READ_FAULT_PATH_OFFSET + path_bytes.len()] == path_bytes
        && &MMAP_WRITE_AFTER_READ_FAULT_PROGRAM[MMAP_WRITE_AFTER_READ_FAULT_PATH_OFFSET..MMAP_WRITE_AFTER_READ_FAULT_PATH_OFFSET + path_bytes.len()] == path_bytes
        && &MMAP_USE_AFTER_UNMAP_FAULT_PROGRAM[MMAP_USE_AFTER_UNMAP_FAULT_PATH_OFFSET..MMAP_USE_AFTER_UNMAP_FAULT_PATH_OFFSET + path_bytes.len()] == path_bytes
        && fs::MAX_FILE_BYTES == PAGE_SIZE;
    let _ = writeln!(
        serial(),
        "milestone 64: self-test -- layout check: all four programs' embedded '{MMAP_TEST_FILE_PATH}' path bytes match their own declared offsets, and fs::MAX_FILE_BYTES == PAGE_SIZE -- {}",
        if layout_ok { "confirmed" } else { "MISMATCH" }
    );

    // Real fixture -- this self-test's own real file, written for real to
    // the real on-disk filesystem (same self-contained-fixture pattern
    // fs::self_test_disk_write() already established).
    let write_ok = fs::write_file(MMAP_TEST_FILE_PATH, MMAP_TEST_FILE_CONTENT.as_bytes()).is_ok();
    let _ = writeln!(
        serial(),
        "milestone 64: self-test -- wrote real fixture file '{MMAP_TEST_FILE_PATH}' ({} bytes) to the real on-disk filesystem -- {}",
        MMAP_TEST_FILE_CONTENT.len(),
        if write_ok { "confirmed" } else { "FAILED" }
    );

    let expected = MMAP_TEST_FILE_CONTENT.as_bytes();

    // Reads a byte out of process `pid`'s own heap page 0 physical frame,
    // directly through phys_mem_offset -- never trusted from the
    // process's own say-so, same discipline as
    // self_test_demand_paging_heap()'s own marker check.
    let read_heap_byte = |pid: u8, offset: u64| -> Option<u8> {
        with_process_mut(pid, |p| p.heap_frames[0]).flatten().map(|frame| unsafe {
            core::ptr::read((phys_mem_offset + frame.start_address().as_u64() + offset).as_ptr::<u8>())
        })
    };

    // -- CASE 1: positive read + munmap -----------------------------------
    let ran1_ok = run(MMAP_READ_TEST_PROCESS_ID).is_ok();
    let bytes_match = (0..4u64).all(|i| read_heap_byte(MMAP_READ_TEST_PROCESS_ID, i) == expected.get(i as usize).copied());
    let munmap_result_byte = read_heap_byte(MMAP_READ_TEST_PROCESS_ID, 4);
    let slot_cleared_after_munmap = with_process_mut(MMAP_READ_TEST_PROCESS_ID, |p| p.mmaps[0].is_none()).unwrap_or(false);
    let case1_ok = ran1_ok && bytes_match && munmap_result_byte == Some(0) && slot_cleared_after_munmap;
    let _ = writeln!(
        serial(),
        "milestone 64: self-test -- CASE 1 (positive read+munmap): ran={ran1_ok} first-4-bytes-match-real-file-content={bytes_match} munmap-result-byte={:?} (expect Some(0)) slot-cleared-after-munmap={slot_cleared_after_munmap} -- {}",
        munmap_result_byte,
        if case1_ok { "PASS" } else { "FAIL" }
    );

    // -- CASE 2: write before any read (not-present+write refusal) --------
    let ran2_ok = run(MMAP_WRITE_BEFORE_READ_FAULT_PROCESS_ID).is_ok();
    let case2_frame_stayed_none = with_process_mut(MMAP_WRITE_BEFORE_READ_FAULT_PROCESS_ID, |p| p.mmaps[0].as_ref().map(|s| s.frame.is_none()))
        .flatten()
        .unwrap_or(false);
    let case2_ok = ran2_ok && case2_frame_stayed_none;
    let _ = writeln!(
        serial(),
        "milestone 64: self-test -- CASE 2 (write before read, expect not-present+write refusal): run()_ok={ran2_ok} mmap-slot-frame-stayed-none={case2_frame_stayed_none} -- {}",
        if case2_ok { "PASS" } else { "FAIL" }
    );

    // -- CASE 3: read then write (protection-violation refusal) -----------
    let ran3_ok = run(MMAP_WRITE_AFTER_READ_FAULT_PROCESS_ID).is_ok();
    let case3_read_succeeded = read_heap_byte(MMAP_WRITE_AFTER_READ_FAULT_PROCESS_ID, 0) == expected.first().copied();
    let case3_never_reached_marker = read_heap_byte(MMAP_WRITE_AFTER_READ_FAULT_PROCESS_ID, 1) == Some(0);
    let case3_ok = ran3_ok && case3_read_succeeded && case3_never_reached_marker;
    let _ = writeln!(
        serial(),
        "milestone 64: self-test -- CASE 3 (read then write, expect protection-violation refusal): run()_ok={ran3_ok} first-read-succeeded={case3_read_succeeded} unreachable-0xEE-marker-absent={case3_never_reached_marker} -- {}",
        if case3_ok { "PASS" } else { "FAIL" }
    );

    // -- CASE 4: use after unmap (cleared-slot refusal) --------------------
    let ran4_ok = run(MMAP_USE_AFTER_UNMAP_FAULT_PROCESS_ID).is_ok();
    let case4_read_succeeded = read_heap_byte(MMAP_USE_AFTER_UNMAP_FAULT_PROCESS_ID, 0) == expected.first().copied();
    let case4_munmap_succeeded = read_heap_byte(MMAP_USE_AFTER_UNMAP_FAULT_PROCESS_ID, 1) == Some(0);
    let case4_never_reached_marker = read_heap_byte(MMAP_USE_AFTER_UNMAP_FAULT_PROCESS_ID, 2) == Some(0);
    let case4_slot_stayed_cleared = with_process_mut(MMAP_USE_AFTER_UNMAP_FAULT_PROCESS_ID, |p| p.mmaps[0].is_none()).unwrap_or(false);
    let case4_ok = ran4_ok && case4_read_succeeded && case4_munmap_succeeded && case4_never_reached_marker && case4_slot_stayed_cleared;
    let _ = writeln!(
        serial(),
        "milestone 64: self-test -- CASE 4 (use after unmap, expect cleared-slot refusal): run()_ok={ran4_ok} first-read-succeeded={case4_read_succeeded} munmap-succeeded={case4_munmap_succeeded} unreachable-0xEE-marker-absent={case4_never_reached_marker} slot-stayed-cleared={case4_slot_stayed_cleared} -- {}",
        if case4_ok { "PASS" } else { "FAIL" }
    );

    // Real proof of genuine recovery after every negative case's real
    // hardware fault, same discipline as
    // self_test_demand_paging_heap()'s own negative-case recovery check.
    let recovery_ok = match run(1) {
        Ok(()) => {
            let _ = writeln!(serial(), "milestone 64: self-test -- PROCESS_A ran normally right after all four mmap test processes -- kernel genuinely recovered");
            true
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 64: self-test -- FAILED, PROCESS_A could not run after the mmap tests: {e}");
            false
        }
    };

    let _ = writeln!(
        serial(),
        "milestone 64: self-test -- OVERALL: {}",
        if layout_ok && write_ok && case1_ok && case2_ok && case3_ok && case4_ok && recovery_ok { "PASS" } else { "FAIL" }
    );
}

/// MILESTONE 65: real, boot-time, non-interactive proof of real
/// `PROT_WRITE` support on top of Milestone 64's own read-only `mmap()`
/// -- two real ring-3 excursions, each proving a genuinely different real
/// success path (mirroring Milestone 64's own discipline of proving two
/// structurally different REFUSAL paths with two separate cases):
///
///   5. MMAP_WRITABLE_READ_THEN_WRITE_PROCESS: a real not-present READ
///      fault is resolved WITH the hardware `WRITABLE` bit set (checked
///      indirectly: the immediately following real WRITE succeeds with
///      NO second fault at all), and the written byte is read back and
///      found to genuinely match what was written -- proving the byte is
///      really sitting in mapped, writable physical memory, not just
///      "the write instruction didn't crash".
///   6. MMAP_WRITABLE_WRITE_FIRST_PROCESS: a real not-present fault whose
///      OWN triggering access is itself a WRITE (`is_write == true`) is
///      genuinely SERVICED rather than refused -- the exact opposite
///      outcome from Milestone 64's own MMAP_WRITE_BEFORE_READ_FAULT_
///      PROGRAM case, which is the SAME real hardware condition
///      (not-present + `CAUSED_BY_WRITE`) hitting a read-only slot
///      instead. The written byte is read back and found correct, AND a
///      second, untouched byte elsewhere on the same page is read back
///      and found to still hold the real snapshotted file content --
///      proving the frame was genuinely populated with real content
///      first, not left zeroed except for the touched byte.
///
/// Both cases finish with a real, direct re-read of the ACTUAL on-disk
/// file (`fs::read_file()`, bypassing both test processes entirely) to
/// prove the disclosed "private, never written back" semantics ARE real:
/// despite two separate processes each genuinely overwriting a byte of
/// their own mapped copy, the file on disk is confirmed byte-for-byte
/// unchanged from what `self_test_mmap()` originally wrote.
///
/// Reuses the SAME on-disk fixture file (`MMAP_TEST_FILE_PATH`/
/// `MMAP_TEST_FILE_CONTENT`) `self_test_mmap()` already wrote -- this
/// function is called from main.rs immediately after `self_test_mmap()`,
/// so the fixture is guaranteed to already exist; same "don't duplicate
/// a fixture an earlier self-test already established" discipline
/// `self_test_mmap()` itself used when it reused `MAX_FILE_BYTES`.
pub fn self_test_mmap_writable() {
    let path_bytes = MMAP_TEST_FILE_PATH.as_bytes();
    let layout_ok = &MMAP_WRITABLE_READ_THEN_WRITE_PROGRAM
        [MMAP_WRITABLE_READ_THEN_WRITE_PATH_OFFSET..MMAP_WRITABLE_READ_THEN_WRITE_PATH_OFFSET + path_bytes.len()]
        == path_bytes
        && &MMAP_WRITABLE_WRITE_FIRST_PROGRAM[MMAP_WRITABLE_WRITE_FIRST_PATH_OFFSET..MMAP_WRITABLE_WRITE_FIRST_PATH_OFFSET + path_bytes.len()] == path_bytes;
    let _ = writeln!(
        serial(),
        "milestone 65: self-test -- layout check: both writable-mmap programs' embedded '{MMAP_TEST_FILE_PATH}' path bytes match their own declared offsets -- {}",
        if layout_ok { "confirmed" } else { "MISMATCH" }
    );

    let phys_mem_offset = memory::phys_mem_offset();
    let expected = MMAP_TEST_FILE_CONTENT.as_bytes();
    let read_heap_byte = |pid: u8, offset: u64| -> Option<u8> {
        with_process_mut(pid, |p| p.heap_frames[0]).flatten().map(|frame| unsafe {
            core::ptr::read((phys_mem_offset + frame.start_address().as_u64() + offset).as_ptr::<u8>())
        })
    };

    // -- CASE 5: read (demand-paged WRITABLE) then write, no second fault --
    let ran5_ok = run(MMAP_WRITABLE_READ_THEN_WRITE_PROCESS_ID).is_ok();
    let case5_original_byte_ok = read_heap_byte(MMAP_WRITABLE_READ_THEN_WRITE_PROCESS_ID, 0) == expected.first().copied();
    let case5_write_stuck = read_heap_byte(MMAP_WRITABLE_READ_THEN_WRITE_PROCESS_ID, 1) == Some(0x99);
    let case5_munmap_ok = read_heap_byte(MMAP_WRITABLE_READ_THEN_WRITE_PROCESS_ID, 2) == Some(0);
    let case5_ok = ran5_ok && case5_original_byte_ok && case5_write_stuck && case5_munmap_ok;
    let _ = writeln!(
        serial(),
        "milestone 65: self-test -- CASE 5 (writable: read then write, expect write to succeed with no second fault): run()_ok={ran5_ok} original-first-byte-matched-real-file={case5_original_byte_ok} write-of-0x99-stuck={case5_write_stuck} munmap-result-byte={:?} (expect Some(0)) -- {}",
        read_heap_byte(MMAP_WRITABLE_READ_THEN_WRITE_PROCESS_ID, 2),
        if case5_ok { "PASS" } else { "FAIL" }
    );

    // -- CASE 6: write as the FIRST-EVER touch, genuinely serviced ---------
    let ran6_ok = run(MMAP_WRITABLE_WRITE_FIRST_PROCESS_ID).is_ok();
    let case6_write_stuck = read_heap_byte(MMAP_WRITABLE_WRITE_FIRST_PROCESS_ID, 0) == Some(0x77);
    let case6_rest_of_page_intact = read_heap_byte(MMAP_WRITABLE_WRITE_FIRST_PROCESS_ID, 1) == expected.get(1).copied();
    let case6_munmap_ok = read_heap_byte(MMAP_WRITABLE_WRITE_FIRST_PROCESS_ID, 2) == Some(0);
    let case6_ok = ran6_ok && case6_write_stuck && case6_rest_of_page_intact && case6_munmap_ok;
    let _ = writeln!(
        serial(),
        "milestone 65: self-test -- CASE 6 (writable: write as first-ever touch, expect real demand-page-and-service instead of refusal): run()_ok={ran6_ok} write-of-0x77-stuck={case6_write_stuck} untouched-byte-still-real-file-content={case6_rest_of_page_intact} munmap-result-byte={:?} (expect Some(0)) -- {}",
        read_heap_byte(MMAP_WRITABLE_WRITE_FIRST_PROCESS_ID, 2),
        if case6_ok { "PASS" } else { "FAIL" }
    );

    // -- Real proof of "private, never written back" -----------------------
    // Bypasses BOTH test processes entirely and re-reads the actual
    // on-disk file directly, the same way fs::self_test_disk_write() or
    // any other fs.rs self-test would -- if either writable mapping's
    // write had leaked back to the fd's buffer or the disk, this would
    // catch it directly, not just trust the disclosed scope-cut's wording.
    let on_disk_after = fs::read_file(MMAP_TEST_FILE_PATH).ok();
    let file_unchanged = on_disk_after.as_deref() == Some(expected);
    let _ = writeln!(
        serial(),
        "milestone 65: self-test -- real on-disk file '{MMAP_TEST_FILE_PATH}' re-read directly after both writable mappings wrote into their own private copies -- unchanged-from-original={file_unchanged}"
    );

    let recovery_ok = match run(1) {
        Ok(()) => {
            let _ = writeln!(serial(), "milestone 65: self-test -- PROCESS_A ran normally right after both writable-mmap test processes -- kernel genuinely recovered");
            true
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 65: self-test -- FAILED, PROCESS_A could not run after the writable-mmap tests: {e}");
            false
        }
    };

    let _ = writeln!(
        serial(),
        "milestone 65: self-test -- OVERALL: {}",
        if layout_ok && case5_ok && case6_ok && file_unchanged && recovery_ok { "PASS" } else { "FAIL" }
    );
}

/// MILESTONE 54: real, boot-time, non-interactive proof that
/// `reclaim_process_frames()` returns a dead process's physical frames
/// to the allocator's free list AND that a later allocation genuinely
/// reuses them -- not just that the bookkeeping runs without erroring.
/// Reuses FAULT_TEST_PROCESS_ID as its own fork source (same "don't pick
/// a fresh test process/program when an existing one already fits"
/// discipline as every other self-test in this file); the dummy (0,0)
/// resume point + immediate kill() pattern is Milestone 53's own reuse_ok
/// check's exact precedent -- the point here is frame bookkeeping, not a
/// real ring-3 excursion.
///
/// Real history worth keeping, not smoothed over: this self-test went
/// through THREE honest failures before this version, all real bugs (or
/// real wrong test assumptions) it found, none hidden or worked around:
///   - v1 hardcoded "expected +3 frames" (pml4/code/stack), assuming a
///     forked child's heap is lazily populated. First QEMU boot: real
///     MISMATCH -- `fork()` eagerly maps a private 4-page heap for every
///     child, so the true leaf count is 7. Fixed by reading
///     `heap_frames.len()` off the live process instead of guessing.
///   - v2 (leaf count now correct) asserted a second fork()'s frames are
///     ALL drawn from the exact set just freed. Second QEMU boot: real
///     MISMATCH -- `map_to()` also allocates PRIVATE P3/P2/P1 page-table
///     frames on demand (see `create_process_from_image()`), which
///     `reclaim_process_frames()` didn't reclaim yet, so a second
///     process needs MORE real frames than the 7 leaf ones freed. Fixed
///     by actually implementing page-table reclamation
///     (`reclaim_private_page_tables()`) rather than weakening this test
///     to stop checking for it.
///   - v3 (page-table reclaim now real, total count independently
///     confirmed correct) STILL asserted the second fork()'s LEAF frames
///     specifically matched the freed LEAF set. Third QEMU boot: real
///     MISMATCH again -- leaf and page-table allocate_frame() calls are
///     INTERLEAVED during construction (pml4, code, stack, then
///     map_to()'s on-demand P3/P2/P1, then each heap page immediately
///     followed by its own map_to()), so the LIFO free list's pop order
///     does not cleanly hand leaf frames back to the leaf fields alone.
///     Fixed by testing the TOTAL instead (see check 2 below) -- simpler
///     and strictly stronger than trying to pin down which specific
///     frame landed where.
///
/// Two real, independent checks, not one, both accounting for the FULL
/// real frame cost (leaf + page-table), never just the leaf fields:
///   1. `free_list_len()` grows by exactly `real_frame_cost()` (leaf
///      fields plus `count_private_page_tables()`'s independent,
///      read-only walk -- see that function's own doc comment for why
///      it's kept separate from the freeing walker) the instant the
///      first forked child is killed.
///   2. A second, freshly-forked child from the SAME source has its own
///      real frame cost checked to be exactly the same as the first
///      (proving determinism, not assumed), and its construction drains
///      the free list to exactly zero -- proving every single one of
///      the frames just freed was genuinely reused, not that merely
///      "enough" of them were.
pub fn self_test_frame_reclaim() {
    let phys_mem_offset = memory::phys_mem_offset();

    // Real, independent per-process frame cost: pml4 + code + stack +
    // heap_frames + extra_frames (the tracked leaf fields), PLUS
    // whatever `count_private_page_tables()` finds by walking the SAME
    // process's own page tables read-only. Captured before the process
    // is killed -- its page tables are still valid to walk right up
    // until `kill()`/`wait_for_child()` actually reclaims them.
    fn real_frame_cost(pid: u8, phys_mem_offset: VirtAddr) -> Option<usize> {
        with_process_mut(pid, |p| {
            // MILESTONE 57: heap_frames is now always HEAP_PAGE_COUNT
            // entries long (sparse `Option`s), so `.len()` alone no
            // longer means "how many heap frames this process actually
            // has mapped" -- count the `Some` entries instead. For a
            // process that's never called sbrk() and touched the result
            // (true of every process this self-test forks), that count
            // is genuinely 0 now, not HEAP_PAGE_COUNT -- a real,
            // disclosed reduction in this test's own expected numbers
            // versus Milestone 54's original eager-heap-mapping design,
            // computed live rather than hardcoded either way.
            let mapped_heap = p.heap_frames.iter().filter(|f| f.is_some()).count();
            // 3 = pml4 + code + top stack page; MILESTONE 100 adds the
            // eagerly-mapped extra stack pages (always a fixed count).
            let leaf = 3 + p.extra_stack_frames.len() + mapped_heap + p.extra_frames.len();
            count_private_page_tables(p.pml4_frame, phys_mem_offset) + leaf
        })
    }

    // MILESTONE 69: `first_cost` now carries `(cost, before)`, not just
    // `cost` -- `before` (the free list's real size measured right before
    // THIS self-test's own first fork(), whatever else already ran
    // earlier in THIS boot left sitting there) is what the second check
    // below now compares against, replacing a hardcoded `expected 0`. See
    // that check's own comment for the real, disclosed reason: a genuine
    // regression THIS milestone's own testing found, not a hypothetical
    // one -- Milestone 69's cc.elf self-test (which `self_test_cc()` runs
    // BEFORE this one, per main.rs's own existing, UNCHANGED call order)
    // is the first self-test in this whole codebase to fork()+exec()+
    // wait() a MULTI-PAGE process (three full cycles, CASE 8/9/10),
    // genuinely reclaiming more real physical frames back onto the SAME
    // global free list than any earlier milestone's self-test ever did in
    // this exact boot sequence -- a real, correct consequence of real,
    // correct reclaim work, not a leak. This self-test's OWN original
    // `expected 0` literal only ever held because nothing between boot
    // and this exact call site had freed anything yet; it was never
    // actually testing "the free list is empty" so much as "the free list
    // is back to whatever it was when this function started" -- this fix
    // makes it test that ACTUAL, real intent explicitly, so it stays
    // correct regardless of what any OTHER earlier self-test (this one or
    // a future one) has already reclaimed by the time it runs.
    let first_cost = fork(FAULT_TEST_PROCESS_ID, 0, 0).and_then(|pid| {
        let cost = real_frame_cost(pid, phys_mem_offset);
        let before = memory::with_frame_allocator(|fa| fa.free_list_len()).unwrap_or(0);
        let killed = kill(FAULT_TEST_PROCESS_ID, pid);
        let after = memory::with_frame_allocator(|fa| fa.free_list_len()).unwrap_or(0);
        match cost {
            Some(cost) => {
                let count_ok = killed && after == before + cost;
                let _ = writeln!(
                    serial(),
                    "milestone 54: self-test -- forked+killed pid {pid} (independently counted real frame cost, leaf+page-tables: {cost}), free list grew {before} -> {after} -- {}",
                    if count_ok { "confirmed" } else { "MISMATCH" }
                );
                count_ok.then_some((cost, before))
            }
            None => {
                let _ = writeln!(serial(), "milestone 54: self-test -- FAILED, could not read back the forked child's own frame cost before killing it");
                None
            }
        }
    });
    if first_cost.is_none() {
        let _ = writeln!(serial(), "milestone 54: self-test -- FAILED, fork() itself failed for the reclaim-count check");
    }

    // Real proof of REUSE, not just a count that happens to match: a
    // second, freshly-forked child (same source, so deterministically
    // the SAME real frame cost as the first -- checked explicitly, not
    // assumed) needs `cost` frames, and the free list holds EXACTLY
    // `cost` frames sitting there from the first child's reclaim. If
    // its construction draws every single one of them (free list drains
    // to exactly zero) rather than falling through to fresh bump
    // allocation for any of them, that's real, complete physical reuse
    // -- checking the TOTAL drains cleanly, rather than trying to match
    // individual freed/reused addresses field-by-field, sidesteps a
    // real wrinkle this self-test found the hard way: leaf and
    // page-table frames are allocated in an INTERLEAVED order during
    // construction (pml4, code, stack, THEN map_to()'s own on-demand
    // P3/P2/P1 allocations, THEN each heap page followed immediately by
    // ITS OWN map_to()), so the free list's LIFO pop order does not
    // cleanly hand the leaf frames back to the leaf fields alone --
    // asserting the TOTAL is exact and complete is both simpler and
    // strictly stronger than trying to pin down which specific frame
    // landed where.
    let reuse_ok = match first_cost {
        Some((cost, before)) => match fork(FAULT_TEST_PROCESS_ID, 0, 0) {
            Some(pid2) => {
                let second_cost = real_frame_cost(pid2, phys_mem_offset);
                let remaining = memory::with_frame_allocator(|fa| fa.free_list_len()).unwrap_or(usize::MAX);
                let _ = kill(FAULT_TEST_PROCESS_ID, pid2);
                let symmetric = second_cost == Some(cost);
                // MILESTONE 69: compares against `before` (this SAME
                // self-test's own real free-list size measured right
                // before its first fork(), NOT a hardcoded 0) -- see
                // `first_cost`'s own doc comment above for the real,
                // disclosed reason this changed: proving "the second
                // fork() drew every frame the first fork()'s own kill
                // just freed, fully reused rather than falling through to
                // fresh bump allocation" only ever needed the free list
                // to return to WHATEVER it was at this function's own
                // entry, never actually needed it to be the literal
                // absolute value 0.
                let fully_drained = remaining == before;
                let _ = writeln!(
                    serial(),
                    "milestone 54: self-test -- second fork()'s own real frame cost: {:?} (expected {cost}, symmetric={symmetric}); free list after its construction: {remaining} (expected {before}, fully drained={fully_drained})",
                    second_cost
                );
                symmetric && fully_drained
            }
            None => {
                let _ = writeln!(serial(), "milestone 54: self-test -- FAILED, second fork() itself failed for the real-reuse check");
                false
            }
        },
        None => {
            let _ = writeln!(serial(), "milestone 54: self-test -- skipping real-reuse check, the reclaim-count check above already failed");
            false
        }
    };

    let _ = writeln!(
        serial(),
        "milestone 54: self-test -- OVERALL: {}",
        if first_cost.is_some() && reuse_ok { "PASS" } else { "FAIL" }
    );
}

/// MILESTONE 59: real, boot-time, non-interactive proof that distinct
/// syscall failures set distinct, correct errno values, and that a
/// userspace program can genuinely read them back -- runs
/// ERRNO_TEST_PROCESS (see ERRNO_TEST_PROGRAM's own doc comment for its
/// exact syscall-by-syscall sequence) through ONE real ring-3 excursion
/// that deliberately fails four DIFFERENT syscalls for four DIFFERENT
/// real reasons (EBADF, ECHILD, ENOMEM, ESRCH), reading errno back via
/// the new GETERRNO syscall after each one. Every individual read is
/// only really checkable by a human/log reader in real time (syscall 17's
/// own dispatch arm in usertest.rs logs the value AND its errno.rs name
/// to serial as it happens -- see that arm's own comment) -- what THIS
/// function can automatically assert, the same "check a specific
/// kernel-side field after a real ring-3 excursion completes" discipline
/// self_test_demand_paging_heap()/self_test_fault_status() already
/// established, is the LAST word this process's own sticky `errno` field
/// holds once the whole excursion (and its final exit()) has finished:
/// ESRCH, from the getpgid(250) call immediately before exit() in
/// ERRNO_TEST_PROGRAM's own fixed sequence -- real, direct proof the
/// field really did end up holding the LAST real failure's code, exactly
/// the "sticky, not reset by unrelated later syscalls" semantics
/// `Process::errno`'s own doc comment promises (exit() itself never
/// touches errno, so this is a genuine end-to-end check, not a tautology).
pub fn self_test_errno() {
    let _ = writeln!(
        serial(),
        "milestone 59: self-test -- running ERRNO_TEST_PROCESS (read/wait/sbrk/getpgid, each deliberately failing for a different real reason, GETERRNO read back after each -- watch the syscall log lines just below for the four distinct errno values)..."
    );
    let ran_ok = match run(ERRNO_TEST_PROCESS_ID) {
        Ok(()) => {
            let _ = writeln!(serial(), "milestone 59: self-test -- ERRNO_TEST_PROCESS ran to completion (its own exit() syscall returned normally)");
            true
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 59: self-test -- FAILED, ERRNO_TEST_PROCESS could not run: {e}");
            false
        }
    };

    let final_errno = get_errno(ERRNO_TEST_PROCESS_ID);
    let final_ok = final_errno == errno::ESRCH;
    let _ = writeln!(
        serial(),
        "milestone 59: self-test -- ERRNO_TEST_PROCESS's own final errno field: {final_errno} ({}) -- expected {} (ESRCH, the getpgid(250) call's real failure, the LAST syscall before exit()) -- {}",
        errno::name(final_errno),
        errno::ESRCH,
        if final_ok { "confirmed" } else { "MISMATCH" }
    );

    let _ = writeln!(
        serial(),
        "milestone 59: self-test -- OVERALL: {}",
        if ran_ok && final_ok { "PASS" } else { "FAIL" }
    );
}

/// MILESTONE 60: real, boot-time, non-interactive, end-to-end proof of
/// real signal delivery -- runs SIGNAL_TEST_PROCESS (see
/// SIGNAL_TEST_PROGRAM's own doc comment for its exact instruction-by-
/// instruction sequence) through ONE real ring-3 excursion that
/// registers a handler, self-signals, gets genuinely redirected into
/// that handler mid-execution (proven by a marker byte only the handler
/// writes), and unwinds back out via a real SIGRETURN to exactly the
/// interrupted point (proven by a SECOND marker byte only the resumed
/// original code writes, AFTER the handler already ran) -- with the full
/// register file proven intact across the whole round trip via a canary
/// register the handler deliberately clobbers.
///
/// Same "kernel-side automated check of a specific physical byte/field
/// after a real ring-3 excursion completes" discipline
/// self_test_demand_paging_heap()/self_test_errno() already established:
/// every check below reads the process's own HEAP_START page DIRECTLY
/// through phys_mem_offset (not trusted from the process's own say-so),
/// the same real technique self_test_demand_paging_heap() already uses.
pub fn self_test_signal_delivery() {
    // Real, checked-not-assumed layout cross-check: SIGNAL_TEST_PROGRAM's
    // own assembler-produced bytes must agree with the offset/pid
    // constants this self-test (and the program's own doc comment) rely
    // on, same discipline as SIGSEGV_MSG_OFFSET's own pre-flight check
    // elsewhere in this file.
    let resume_opcode_ok = SIGNAL_TEST_PROGRAM[SIGNAL_TEST_RESUME_OFFSET as usize] == 0x48
        && SIGNAL_TEST_PROGRAM[SIGNAL_TEST_RESUME_OFFSET as usize + 1] == 0xB8;
    let handler_opcode_ok = SIGNAL_TEST_PROGRAM[SIGNAL_TEST_HANDLER_OFFSET as usize] == 0x48
        && SIGNAL_TEST_PROGRAM[SIGNAL_TEST_HANDLER_OFFSET as usize + 1] == 0xB8;
    // The `mov edi, SIGNAL_TEST_PROCESS_ID` (self pid) operand byte, at a
    // fixed offset within the program's own sigsend() call -- see
    // SIGNAL_TEST_PROGRAM's own byte listing (offset 0x35's `0xBF, 0x12,
    // ...`).
    let self_pid_ok = SIGNAL_TEST_PROGRAM[0x36] == SIGNAL_TEST_PROCESS_ID;
    let layout_ok = resume_opcode_ok && handler_opcode_ok && self_pid_ok;
    let _ = writeln!(
        serial(),
        "milestone 60: self-test -- layout check: resume opcode ok={resume_opcode_ok}, handler opcode ok={handler_opcode_ok}, embedded self-pid ok={self_pid_ok} -- {}",
        if layout_ok { "confirmed" } else { "MISMATCH" }
    );

    let _ = writeln!(
        serial(),
        "milestone 60: self-test -- running SIGNAL_TEST_PROCESS (register a real SIGUSR1 handler, self-signal, expect genuine mid-execution redirection into it and a real SIGRETURN back out)..."
    );
    let ran_ok = match run(SIGNAL_TEST_PROCESS_ID) {
        Ok(()) => {
            let _ = writeln!(serial(), "milestone 60: self-test -- SIGNAL_TEST_PROCESS ran to completion (its own exit() syscall returned normally)");
            true
        }
        Err(e) => {
            let _ = writeln!(serial(), "milestone 60: self-test -- FAILED, SIGNAL_TEST_PROCESS could not run: {e}");
            false
        }
    };

    let phys_mem_offset = memory::phys_mem_offset();
    let heap_bytes = with_process_mut(SIGNAL_TEST_PROCESS_ID, |p| {
        p.heap_frames[0].map(|frame| {
            let base = phys_mem_offset + frame.start_address().as_u64();
            let mut buf = [0u8; 40];
            unsafe { core::ptr::copy_nonoverlapping(base.as_ptr::<u8>(), buf.as_mut_ptr(), buf.len()) };
            buf
        })
    })
    .flatten();

    let (before_ok, handler_ran_ok, canary_restored_ok, resume_ran_ok, signum_ok) = match heap_bytes {
        Some(buf) => {
            let canary = u64::from_le_bytes(buf[16..24].try_into().unwrap());
            let signum_seen = u64::from_le_bytes(buf[32..40].try_into().unwrap());
            (
                buf[0] == 0xAA,
                buf[8] == 0xBB,
                canary == SIGNAL_TEST_CANARY,
                buf[24] == 0xCC,
                signum_seen == SIGNAL_TEST_SIGNUM as u64,
            )
        }
        None => (false, false, false, false, false),
    };

    let _ = writeln!(
        serial(),
        "milestone 60: self-test -- HEAP_START markers: main-before-signal ran={before_ok} (expect true), handler genuinely ran={handler_ran_ok} (expect true), canary register (0x{SIGNAL_TEST_CANARY:X}, clobbered by the handler to 0x{SIGNAL_TEST_CANARY_CLOBBER:X} in between) survived SIGRETURN intact={canary_restored_ok} (expect true), resumed-after-handler code ran={resume_ran_ok} (expect true), handler received the real signum ({SIGNAL_TEST_SIGNUM}, SIGUSR1) as its SysV first argument={signum_ok} (expect true)"
    );

    let overall = layout_ok && ran_ok && before_ok && handler_ran_ok && canary_restored_ok && resume_ran_ok && signum_ok;
    let _ = writeln!(serial(), "milestone 60: self-test -- OVERALL: {}", if overall { "PASS" } else { "FAIL" });
}

/// MILESTONE 45: a direct, filesystem-independent proof that
/// EXEC_TEST_PROGRAM's own byte layout actually matches what its doc
/// comment claims -- same discipline as self_test_fork_test_program().
pub fn self_test_exec_test_program() {
    let path_ok = &EXEC_TEST_PROGRAM[EXEC_TEST_PATH_OFFSET..EXEC_TEST_PATH_OFFSET + EXEC_TEST_PATH.len()] == EXEC_TEST_PATH.as_bytes();
    let fallback_ok = &EXEC_TEST_PROGRAM[EXEC_TEST_FALLBACK_OFFSET..EXEC_TEST_FALLBACK_OFFSET + EXEC_TEST_FALLBACK_MSG.len()]
        == EXEC_TEST_FALLBACK_MSG.as_bytes();
    let _ = writeln!(
        serial(),
        "milestone 45: self-test -- EXEC_TEST_PROGRAM layout check: exec_path={path_ok} fallback_msg={fallback_ok} -- {}",
        if path_ok && fallback_ok { "all match, layout confirmed" } else { "FAILED -- byte layout drifted from doc comment" }
    );
}

/// MILESTONE 45's real, boot-time, non-interactive proof that `exec()`
/// genuinely tears down and rebuilds a process's address space -- same
/// "interactive shell-command testing via QEMU sendkey has been
/// repeatedly unreliable" reasoning self_test_signals() already
/// established, applied here to the newly-real exec() syscall instead.
/// Must run AFTER `interrupts::init_pics()`/`sti()`, same ordering
/// requirement as self_test_signals()/self_test_wait_status() (real
/// ring-3 entry sets RFLAGS.IF=1 immediately -- see self_test_signals()'s
/// own doc comment for the full double-fault story this ordering avoids).
///
/// Real evidence gathered, not assumed:
///   1. Opens a real fd (`open_file`, kernel-side, no ring-3 involved)
///      on EXEC_TEST_PROCESS BEFORE running it -- proof material for
///      claim 4 below.
///   2. Snapshots EXEC_TEST_PROCESS's pml4_frame/entry BEFORE run() --
///      at this point still the flat-binary create_process_from_image()
///      values (entry == USER_CODE_ADDR).
///   3. Runs EXEC_TEST_PROCESS for real (`process::run()`, genuine CR3
///      switch + `iretq` into ring 3) -- its own code calls the REAL
///      `int 0x80` exec() syscall into "altentry"
///      (testelf_altentry.elf, seeded to disk by
///      loader::seed_test_elf_altentry() just before this call), which
///      this milestone's own exec_elf() handles by tearing down and
///      rebuilding EXEC_TEST_PROCESS's address space mid-flight, then
///      resuming ring 3 at the NEW program's own entry -- which writes
///      its own distinguishing message and exit()s for real.
///   4. Snapshots pml4_frame/entry/fds[0]/pgid AFTER run() returns and
///      checks, for real, that:
///        - pml4_frame genuinely CHANGED (a fresh PML4 was really
///          built, not the same one reused)
///        - entry genuinely changed to 0x0000_5555_5000_3000 -- the
///          REAL, parsed e_entry of testelf_altentry.elf, deliberately
///          NOT USER_CODE_ADDR (Milestone 44's own staged, not-yet-
///          wired-in altentry payload -- wired into a real, running
///          self-test for the first time by this milestone)
///        - fds[0] is STILL Some(..) -- the fd opened in step 1
///          survived the teardown-and-rebuild, real proof exec()
///          preserves the calling process's open files
///        - pgid is UNCHANGED -- real proof exec() preserves process-
///          group membership
pub fn self_test_real_exec() {
    let seed_ok = crate::loader::seed_test_elf_altentry().is_ok();
    let _ = writeln!(
        serial(),
        "milestone 45: self-test -- seeded 'altentry' (testelf_altentry.elf) to the real on-disk filesystem: {seed_ok}"
    );

    // Real, kernel-side (no ring 3 involved) fd-survives-exec() setup --
    // a path that doesn't exist yet still gets a real fd (open_file()'s
    // own "always succeeds, starts empty" contract, same as every other
    // caller of it in this file).
    let fd_opened = open_file(EXEC_TEST_PROCESS_ID, "exectest_marker", false).is_some();

    let before = with_process_mut(EXEC_TEST_PROCESS_ID, |p| (p.pml4_frame, p.entry, p.pgid));
    if let Some((pml4, entry, pgid)) = before {
        let _ = writeln!(
            serial(),
            "milestone 45: self-test -- fd opened before exec()={fd_opened}, before-state: pml4={:#x} entry={:#x} pgid={pgid}",
            pml4.start_address().as_u64(),
            entry
        );
    }

    let run_ok = run(EXEC_TEST_PROCESS_ID).is_ok();

    let after = with_process_mut(EXEC_TEST_PROCESS_ID, |p| (p.pml4_frame, p.entry, p.pgid, p.fds[0].is_some()));
    if let Some((pml4, entry, pgid, fd0)) = after {
        let _ = writeln!(
            serial(),
            "milestone 45: self-test -- run_ok={run_ok}, after-state: pml4={:#x} entry={:#x} pgid={pgid} fds[0]_present={fd0}",
            pml4.start_address().as_u64(),
            entry
        );
    }

    const ALT_ENTRY: u64 = 0x0000_5555_5000_3000;
    let overall = match (before, after) {
        (Some((pml4_before, entry_before, pgid_before)), Some((pml4_after, entry_after, pgid_after, fd0_survived))) => {
            let pml4_changed = pml4_before != pml4_after;
            let entry_is_real_altentry = entry_after == ALT_ENTRY;
            let entry_changed = entry_before != entry_after;
            let pgid_preserved = pgid_before == pgid_after;
            let _ = writeln!(
                serial(),
                "milestone 45: self-test -- pml4_changed={pml4_changed} (before={:#x} after={:#x}), entry_changed={entry_changed} (before={:#x} after={:#x}, expected real e_entry={:#x}), fd_survived_exec={fd0_survived}, pgid_preserved={pgid_preserved}",
                pml4_before.start_address().as_u64(),
                pml4_after.start_address().as_u64(),
                entry_before,
                entry_after,
                ALT_ENTRY
            );
            seed_ok && run_ok && pml4_changed && entry_is_real_altentry && entry_changed && fd0_survived && pgid_preserved
        }
        _ => false,
    };
    let _ = writeln!(
        serial(),
        "milestone 45: self-test -- OVERALL: {}",
        if overall { "PASS" } else { "FAIL" }
    );
}
