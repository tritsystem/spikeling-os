//! MILESTONE 27: real ring-3 (CPL=3) execution and a minimal `int 0x80`
//! syscall ABI. Every single thing that has run in this kernel through
//! Milestone 26 -- the shell, the neuron simulation, every device
//! driver, all 8 fixed+spawned worker tasks from Milestone 25 -- has run
//! at CPL=0. This module is the first code in spikeling-os that actually
//! drops to CPL=3 and proves it with hardware-recorded evidence, not a
//! self-reported claim -- the real prerequisite for the project's
//! longer-term goal of eventually running code not written specifically
//! for this kernel.
//!
//! Scope, deliberately minimal (see the milestone report for the full
//! list): ONE hardcoded ring-3 program, ONE mapped user code page, ONE
//! mapped user stack page, exactly two syscalls (0 = print a FIXED
//! kernel-owned message -- no general copy-from-user pointer-safety
//! mechanism exists yet, deliberately out of scope this milestone -- and
//! 1 = exit back to whatever called into ring 3). No per-process
//! isolation, no scheduler integration for user tasks: Milestone 28+
//! territory.
//!
//! MILESTONE 31: syscall 0 stops being "print one hardcoded kernel-side
//! string" and becomes a REAL `write(ptr, len)` syscall -- the ring-3
//! program now passes a real pointer (rdi) and length (rsi) in
//! registers, and syscall_dispatch reads those exact bytes out of
//! whatever address space is CURRENTLY loaded in CR3 (the calling
//! process's own private PML4 for a process.rs process, or the kernel's
//! own shared page tables for the legacy `usertest` path) and writes
//! them, raw, to the serial console. This is the generalization of
//! Milestone 30's read_active_message() (which only ever read ONE fixed
//! offset) to an arbitrary caller-supplied pointer+length. See
//! MAX_WRITE_LEN below for the one real safety net in place -- there is
//! still no general copy-from-user fault-recovery path, disclosed
//! honestly, not hidden.
//!
//! MILESTONE 35: real per-process file descriptors -- syscalls 3 (open),
//! 4 (read), 5 (fdwrite), 6 (close), all NEW syscall numbers, syscall 0
//! left completely untouched. See process.rs's own doc comment (right
//! above MAX_OPEN_FILES/OpenFile) for the full "why new syscalls instead
//! of generalizing write(ptr,len) into write(fd,ptr,len)" reasoning --
//! the short version: generalizing syscall 0 would require regenerating
//! every existing hand-assembled program's register setup (USER_PROGRAM
//! below, process.rs's PROCESS_PROGRAM, loader.rs's
//! build_test_program_image()), for no benefit that outweighs touching
//! three already-verified programs. All four new syscalls reuse the
//! EXACT SAME "read/write raw bytes at a caller-supplied pointer, through
//! whatever CR3 is currently loaded" technique syscall 0 established --
//! open()'s path string and fdwrite()'s data are read out of user memory
//! the same way syscall 0 always has, and read() writes its result back
//! into user memory the same way. The actual open/read/write/close
//! bookkeeping (the per-process fd table, buffering, close-time persist)
//! lives in process.rs, not here -- this file's job is exactly what it
//! already was for syscall 0: validate/cap the raw pointer+length
//! arguments and cross the user/kernel memory boundary safely, nothing
//! filesystem-specific.
//!
//! MILESTONE 59: real `errno` -- every syscall arm in `syscall_dispatch`
//! below that has a real, distinguishable failure reason now calls
//! `process::set_errno()` alongside its existing bare `u64::MAX`/`0`/`1`
//! return-value sentinel, using the real POSIX/Linux x86_64 constants
//! `errno.rs` defines (see that module's own doc comment for exactly
//! which value applies where and why -- checked call-by-call against
//! this file). A NEW syscall, 17 (GETERRNO, no arguments), reads a
//! process's own current errno back -- the thing that actually lets a
//! userspace program (see `process::ERRNO_TEST_PROGRAM`) distinguish
//! WHICH of several real failure reasons a bare sentinel used to
//! collapse into one indistinguishable value.

use crate::errno;
use crate::gdt;
use crate::serial;
use crate::signal;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use x86_64::VirtAddr;
use x86_64::structures::paging::{FrameAllocator, Mapper, Page, PageTableFlags, PhysFrame, Size4KiB};

pub const USER_CODE_ADDR: u64 = 0x_5555_5000_0000;
pub const USER_STACK_ADDR: u64 = 0x_5555_6000_0000;
/// The ONE page at `USER_STACK_ADDR` itself -- the top of the stack.
/// `rsp` still starts at `USER_STACK_ADDR + USER_STACK_SIZE` for every
/// process (unchanged since Milestone 30), and the argv/envp writer
/// still writes down from there. This constant is deliberately NOT the
/// total stack size -- see `USER_STACK_EXTRA_PAGES` below.
pub const USER_STACK_SIZE: u64 = 4096;
/// MILESTONE 100: extra stack pages mapped BELOW `USER_STACK_ADDR`,
/// i.e. backing `[USER_STACK_ADDR - USER_STACK_EXTRA_PAGES*4096,
/// USER_STACK_ADDR)`. The x86 stack grows DOWN from
/// `USER_STACK_ADDR + USER_STACK_SIZE`, so these pages extend how far
/// `rsp` can descend before an unmapped fault. Every full process
/// (`create_process_from_image`/`create_process_from_elf`, and via
/// them `fork`) maps all of them eagerly; the one-off Milestone 27
/// `usertest::setup()` ring-3 demo keeps a single page (its hand-
/// assembled program never pushes). 7 extra -> 8 pages -> 32 KiB total:
/// the subset-C compiler's recursive-descent parser (`tools/cc_src`)
/// ran a single 4 KiB page off the bottom at Milestone 100's first
/// attempt, and it will only get deeper on the way to self-hosting.
pub const USER_STACK_EXTRA_PAGES: u64 = 7;
/// Total mapped stack, in bytes: the top page plus every extra page.
pub const USER_STACK_TOTAL_BYTES: u64 = USER_STACK_SIZE + USER_STACK_EXTRA_PAGES * 4096;
/// Lowest address the mapped stack reaches. A fault strictly below this
/// (and not a protection violation) is a genuine stack overflow -- the
/// process ran `rsp` off the bottom of even the extra pages.
pub const USER_STACK_LIMIT: u64 = USER_STACK_ADDR - USER_STACK_EXTRA_PAGES * 4096;

/// MILESTONE 31: offset (from the start of a process's code page) and
/// fixed length (in bytes) of the message region the shared USER_PROGRAM
/// binary's `write(ptr, len)` syscall call reads and writes to serial.
/// USER_PROGRAM is ONE hand-assembled binary, copied byte-for-byte into
/// THREE different code pages (this module's own legacy usertest page,
/// and process.rs's process A / process B pages) -- since a naked
/// hand-assembled program can't compute "the length of whatever string
/// happens to live here" at runtime, both `ptr` and `len` are baked into
/// USER_PROGRAM as fixed immediates at assembly time below. Every caller
/// that installs a message here (this module's setup(), and process.rs's
/// create_process()) goes through write_fixed_message(), which
/// pads/truncates to exactly MESSAGE_LEN bytes -- the thing that keeps
/// the ONE shared `len` immediate valid for every copy of USER_PROGRAM.
/// Still no general program loader: a real one would compute `len` at
/// load/link time instead of requiring every message to fit one fixed
/// slot, explicitly Milestone 32+ territory, not pretended otherwise.
pub(crate) const MESSAGE_OFFSET: u64 = 128;
pub(crate) const MESSAGE_LEN: usize = 64;

/// MILESTONE 31: the legacy (ACTIVE_PROCESS == 0) `usertest` path's own
/// distinguishing message, installed into its code page by setup()
/// below -- deliberately different wording from process A/B's so all
/// three are unambiguous in a serial log.
const LEGACY_MESSAGE: &str = "hello from ring 3, CPL=3 confirmed -- real write() syscall";

/// MILESTONE 31: writes `message` into the code page pointed to by
/// `code_ptr`, at MESSAGE_OFFSET, padded with ASCII spaces (0x20 -- not
/// zero bytes, so the raw bytes the write syscall actually reads back
/// and prints stay human-readable instead of trailing NUL junk) out to
/// exactly MESSAGE_LEN bytes, truncated if the source string is longer.
/// Shared by this module's setup() (legacy usertest path) and
/// process.rs's create_process() (process A/B) so every code page's
/// message region is always exactly MESSAGE_LEN bytes regardless of the
/// source string's own length.
///
/// # Safety
/// `code_ptr` must point to at least `MESSAGE_OFFSET + MESSAGE_LEN`
/// writable bytes (true for any page mapped the way setup()/
/// create_process() map the code page).
pub(crate) unsafe fn write_fixed_message(code_ptr: *mut u8, message: &str) {
    let bytes = message.as_bytes();
    let n = bytes.len().min(MESSAGE_LEN);
    unsafe {
        let dst = code_ptr.add(MESSAGE_OFFSET as usize);
        core::ptr::write_bytes(dst, b' ', MESSAGE_LEN);
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, n);
    }
}

/// Hand-assembled x86_64 machine code, not bytes copied out of a
/// compiled Rust function: a naked Rust function's exact compiled
/// instruction-byte LENGTH isn't something Rust exposes (no linker
/// symbol for "end of this specific function" without a fragile
/// assumption about link order), whereas a handful of fixed, well-known
/// x86_64 instructions are trivial to encode by hand, so their length is
/// simply KNOWN rather than guessed at.
///
/// MILESTONE 31: regenerated from the Milestone 27 5-instruction program
/// -- syscall 0 now passes REAL arguments (rdi = pointer, rsi = length)
/// instead of taking none. `ptr` is baked in as USER_CODE_ADDR +
/// MESSAGE_OFFSET and `len` as MESSAGE_LEN, both compile-time constants
/// of THIS file, so the encoding below is regenerated deterministically
/// from them (verified with a standalone byte-for-byte re-derivation,
/// not hand-counted hex digits):
///
///   48 BF <8 bytes LE>   mov rdi, imm64   rdi = USER_CODE_ADDR+MESSAGE_OFFSET
///   BE <4 bytes LE>      mov esi, imm32   esi = MESSAGE_LEN
///   B8 00 00 00 00       mov eax, 0       syscall number 0 = write(ptr,len)
///   CD 80                int 0x80
///   B8 01 00 00 00       mov eax, 1       syscall number 1 = exit
///   CD 80                int 0x80
///   EB FE                jmp $            safety net: only reached if exit
///                                         somehow returns instead of resuming
///                                         kernel_main -- spins in ring 3
///                                         forever rather than running off into
///                                         whatever bytes follow in the page.
///
/// Total 31 bytes, comfortably inside MESSAGE_OFFSET (128) so it never
/// overlaps the message region that follows it in the same page.
pub(crate) static USER_PROGRAM: [u8; 31] = [
    0x48, 0xBF, 0x80, 0x00, 0x00, 0x50, 0x55, 0x55, 0x00, 0x00, // mov rdi, USER_CODE_ADDR+MESSAGE_OFFSET
    0xBE, 0x40, 0x00, 0x00, 0x00, // mov esi, MESSAGE_LEN (64 = 0x40)
    0xB8, 0x00, 0x00, 0x00, 0x00, // mov eax, 0
    0xCD, 0x80, // int 0x80
    0xB8, 0x01, 0x00, 0x00, 0x00, // mov eax, 1
    0xCD, 0x80, // int 0x80
    0xEB, 0xFE, // jmp $
];

static MAPPED: AtomicBool = AtomicBool::new(false);
static RUN_COUNT: AtomicU64 = AtomicU64::new(0);

/// Where enter_ring3() stashes the kernel-side rsp (plus, on that same
/// stack, the callee-saved registers and return address it pushed)
/// immediately before iretq-ing into ring 3 -- mirrors tasks.rs's
/// KERNEL_RSP exactly, just scoped to this module's own excursion
/// instead of the task scheduler's. The exit syscall hands this straight
/// to resume_kernel(), which pops back into run() as if enter_ring3()
/// had simply returned normally after some delay.
static KERNEL_RSP: AtomicU64 = AtomicU64::new(0);

/// MILESTONE 37: a SECOND, dedicated kernel-resume anchor, exactly
/// analogous to KERNEL_RSP above but used ONLY by
/// enter_ring3_as_forked_child() -- process::run_forked_child()'s own
/// nested ring-3 excursion, driven from inside the PARENT's own wait()
/// syscall dispatch, needs somewhere to stash ITS kernel-side rsp that
/// is NOT KERNEL_RSP, or it would clobber whatever the OUTER (top-level,
/// parent's own) excursion already stashed there, corrupting the
/// parent's own eventual resume point. A single dedicated static (not a
/// general per-nesting-level stack of anchors) is this milestone's
/// real, honest, ENFORCED v1 bound on nesting depth: exactly one nested
/// child excursion can be in flight at a time -- see
/// is_in_child_resume()'s own doc comment for how that's actually
/// checked, not just assumed.
static CHILD_KERNEL_RSP: AtomicU64 = AtomicU64::new(0);

/// MILESTONE 37: true for the ENTIRE duration a forked child is running
/// via enter_ring3_as_forked_child() (set just before that function's
/// own iretq, cleared just after it returns) -- read by TWO places: (1)
/// the exit-syscall arm below, to decide whether THIS excursion's
/// resume_kernel() call should use CHILD_KERNEL_RSP instead of the
/// top-level KERNEL_RSP; (2) process::fork()/wait_for_child(), to
/// refuse (not silently misbehave) a fork()/wait() call that would
/// require a SECOND, simultaneously-live nested anchor -- this design
/// only has one.
static IN_CHILD_RESUME: AtomicBool = AtomicBool::new(false);

/// MILESTONE 43: real exit-status capture for a forked child's OWN
/// exit() call, read out by process::wait_for_child() the instant
/// run_forked_child() returns (before anything else can touch it --
/// this design's existing nesting-depth-1 bound means at most one
/// child excursion is ever live at a time, so a single slot is honest
/// and sufficient, not a shortcut). The exit syscall's arm below only
/// writes here when IN_CHILD_RESUME is true -- a top-level process's
/// own exit() (PROCESS_A/B, the legacy path, etc.) has no parent
/// wait()ing on it in this design and leaves this alone.
static LAST_CHILD_EXIT_CODE: AtomicU8 = AtomicU8::new(0);

pub(crate) fn take_last_child_exit_code() -> u8 {
    LAST_CHILD_EXIT_CODE.load(Ordering::SeqCst)
}

/// MILESTONE 53: real signal-terminated status for a forked child --
/// set by terminate_faulted_process_and_resume_kernel() below when a
/// real hardware fault (page fault / #GP, both funnel through that one
/// function) kills a process WHILE `IN_CHILD_RESUME` is true, i.e. the
/// faulting process is a forked child currently being driven by
/// process::wait_for_child()'s own nested excursion. Closes a real gap
/// left open since M41/M43: before this milestone, a forked child that
/// page-faulted instead of calling its own exit() left
/// LAST_CHILD_EXIT_CODE completely untouched, so wait_for_child() read
/// out whatever STALE code an earlier, unrelated child's real exit()
/// last wrote (or the 0 default, on the very first wait() of a boot) and
/// reported it as `WaitOutcome::Exited(stale_code)` -- a real, silent
/// misreport that a crashed child ran to completion normally. `take_*()`
/// consumes (loads AND resets) in one atomic op, unlike
/// take_last_child_exit_code() above: that field only ever needs to be
/// read when a real exit() JUST wrote it fresh in this same excursion
/// (nesting-depth-1 bound), so staleness there was never actually
/// reachable; this flag has no such freshness guarantee -- it stays
/// `true` forever once ANY forked child ever faults unless explicitly
/// consumed, silently mislabeling every later child (even ones that
/// genuinely exit normally) as signaled if left un-reset.
static CHILD_FAULTED: AtomicBool = AtomicBool::new(false);

pub(crate) fn take_child_faulted() -> bool {
    CHILD_FAULTED.swap(false, Ordering::SeqCst)
}

/// MILESTONE 37: real, honest, ENFORCED nesting-depth check -- see
/// IN_CHILD_RESUME's own doc comment. `pub(crate)` so process.rs's
/// fork()/wait_for_child() can check it directly before ever attempting
/// a nested excursion, rather than attempting one and discovering the
/// collision after the fact.
pub(crate) fn is_in_child_resume() -> bool {
    IN_CHILD_RESUME.load(Ordering::SeqCst)
}

/// MILESTONE 27: allocates one physical frame for the user code page and
/// one for the user stack page, maps both PRESENT | WRITABLE |
/// USER_ACCESSIBLE (no NO_EXECUTE set, matching every other mapping
/// already made in this kernel -- allocator.rs's heap pages included --
/// so the code page is executable), and copies USER_PROGRAM into the
/// mapped code page. Idempotent: a second call is a no-op, so repeated
/// `usertest` shell invocations never re-map or re-copy.
///
/// Called once at boot time, while kernel_main's own mapper/
/// frame_allocator are still in scope -- simpler than threading a
/// `Mutex<Option<OffsetPageTable>>` through the rest of the kernel for a
/// one-time setup step that only this module ever needs again.
pub fn setup(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), &'static str> {
    if MAPPED.load(Ordering::SeqCst) {
        return Ok(());
    }

    let code_page = Page::<Size4KiB>::containing_address(VirtAddr::new(USER_CODE_ADDR));
    let stack_page = Page::<Size4KiB>::containing_address(VirtAddr::new(USER_STACK_ADDR));
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

    for page in [code_page, stack_page] {
        let frame: PhysFrame<Size4KiB> = frame_allocator.allocate_frame().ok_or("out of physical frames")?;
        unsafe {
            mapper
                .map_to(page, frame, flags, frame_allocator)
                .map_err(|_| "map_to failed")?
                .flush();
        }
    }

    unsafe {
        let code_ptr = USER_CODE_ADDR as *mut u8;
        core::ptr::copy_nonoverlapping(USER_PROGRAM.as_ptr(), code_ptr, USER_PROGRAM.len());
        // MILESTONE 31: the legacy path's own message, installed at the
        // same MESSAGE_OFFSET every copy of USER_PROGRAM reads from --
        // without this, the legacy `usertest` command's write syscall
        // would read 64 bytes of whatever happened to already be in this
        // freshly-allocated physical frame (zeroed by the bootloader's
        // frame allocator, but not guaranteed to STAY zero -- explicit
        // is safer than implicit here).
        write_fixed_message(code_ptr, LEGACY_MESSAGE);
    }

    MAPPED.store(true, Ordering::SeqCst);
    Ok(())
}

/// Register layout as saved by syscall_entry's push sequence, read back
/// by syscall_dispatch. Field order (top/lowest address first) mirrors
/// the naked trampoline's push order exactly: the LAST register pushed
/// ends up at the LOWEST address, i.e. this struct's FIRST field --
/// getting this backwards silently reads the wrong register into the
/// wrong field without any compiler error, so it was checked by hand
/// against the actual naked_asm! push list below, not assumed.
#[repr(C)]
struct SyscallRegs {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rbp: u64,
    rdi: u64,
    rsi: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    // CPU-pushed InterruptStackFrame, immediately following the GPRs we
    // pushed -- int 0x80 from ring 3 pushes no error code, so this
    // starts right after rax with no gap.
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

/// MILESTONE 27: the int 0x80 entry point itself. `extern "x86-interrupt"`
/// handlers do NOT expose general-purpose registers (only the
/// InterruptStackFrame), so syscall number/arguments passed in registers
/// (the standard convention, rax = syscall number here) are unreachable
/// from one -- this naked function pushes every GPR onto the stack
/// itself, calls the ordinary `extern "C"` syscall_dispatch with a
/// pointer to the saved registers, then pops them back and `iretq`s,
/// exactly the technique tasks.rs's switch_to established as this
/// codebase's precedent for hand-written stack/register manipulation.
///
/// Installed directly via `Entry::set_handler_addr` (not
/// `set_handler_fn`, which requires the `extern "x86-interrupt"`
/// signature this function deliberately does NOT have) at IDT vector
/// 0x80, DPL=3 -- interrupts.rs sets that explicitly, since a gate's
/// default DPL is 0 and a ring-3 `int 0x80` against a DPL=0 gate
/// immediately #GP-faults rather than running the handler.
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn syscall_entry() {
    core::arch::naked_asm!(
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push rbp",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov rdi, rsp",
        "call {dispatch}",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        "iretq",
        dispatch = sym syscall_dispatch,
    );
}

/// MILESTONE 31: hard cap on syscall 0 (write)'s `len` argument. This is
/// `unsafe` kernel code dereferencing a raw, ring-3-supplied pointer with
/// NO general copy-from-user fault-recovery path (e.g. a page-fault
/// handler that aborts the syscall cleanly instead of taking down the
/// kernel) -- that's real, disclosed, out-of-scope-for-this-milestone
/// work, not hidden behind this cap. What this cap DOES stop: a wildly
/// large `len` (e.g. an accidentally-sign-extended -1 arriving as
/// u64::MAX) walking the read loop off the end of the mapped code/stack
/// pages into unmapped address space and page-faulting the kernel. A
/// SHORT but genuinely bad pointer (len=8 at some unmapped address) can
/// still fault today -- that gap is real and this comment says so rather
/// than letting the cap read as a complete safety net.
const MAX_WRITE_LEN: u64 = 4096;

/// MILESTONE 35: cap on the `path_len` argument to syscall 3 (open).
/// Same reasoning as MAX_WRITE_LEN above (stops a wildly large/corrupt
/// length from walking a read loop off mapped memory), sized much
/// smaller than MAX_WRITE_LEN/MAX_FD_IO_LEN because fs.rs's own path
/// component names are already capped short (NAME_LEN=16 bytes per
/// component) -- 64 bytes is comfortably enough for this milestone's own
/// deepest test paths with real headroom, not a tight fit.
const MAX_PATH_LEN: u64 = 64;

/// MILESTONE 35: cap on the `len` argument to syscall 4 (read) and
/// syscall 5 (fdwrite) -- identical value and identical reasoning to
/// MAX_WRITE_LEN above, kept as a separate named constant (rather than
/// reusing MAX_WRITE_LEN directly) since it bounds a conceptually
/// different thing (fd I/O length, not the legacy write syscall's
/// length) even though the number happens to match fs::MAX_FILE_BYTES
/// today.
const MAX_FD_IO_LEN: u64 = 4096;

/// MILESTONE 60: real headroom (bytes) subtracted from a process's
/// current, real, hardware-interrupted `rsp` before real signal delivery
/// injects its trampoline+fake-return-address frame there -- clears the
/// real 128-byte SysV "red zone" a leaf function may have been using
/// below its own rsp at the moment it was interrupted (real ABI rule:
/// a leaf function may use that space without adjusting rsp), plus real,
/// disclosed extra headroom, same "generous, real, disclosed, not a
/// tight fit" spirit as MAX_PATH_LEN's own doc comment. The actual frame
/// this milestone ever writes is 15 bytes (7-byte trampoline + 8-byte
/// return-address slot) -- 256 is not a tight computation, it's real
/// safety margin against this kernel's single-page (4 KiB,
/// `usertest::USER_STACK_SIZE`) user stack ever running this frame off
/// the mapped page's own bottom for a process that was already deep in
/// its own stack usage when the signal arrived.
const SIGNAL_FRAME_GAP: u64 = 256;

/// MILESTONE 59: maps one of `process::exec_elf()`/`exec_elf_with_args()`'s
/// `Err(&str)` messages (which also cover their shared
/// `create_process_from_elf()`/`build_argv_envp_stack()` helpers) to the
/// real errno.rs value that best describes it -- these two functions run
/// AFTER `elf::parse()` has already succeeded (that earlier failure is
/// reported as ENOEXEC directly at its own call site, not through this
/// helper), so every message reaching here is either a genuine resource
/// exhaustion or a structural rejection of an otherwise-parseable ELF:
/// - "no such process" / "slot vanished" (defensive -- should not be
///   reachable in practice, since both call sites already checked
///   `active != 0` before ever calling in) -> ESRCH, the same
///   "no live process to operate on" bucket every other defensive
///   not-actually-reachable case in this file uses.
/// - "out of physical frames" / "frame allocator not installed" (a real
///   resource-exhaustion condition, checked against the actual message
///   text `create_process_from_elf()` returns) -> ENOMEM, the same real
///   POSIX bucket a real `execve()` uses for "insufficient kernel memory".
/// - "argv"/"envp" (build_argv_envp_stack()'s own defensive re-check of
///   the exact same EXEC_ARGV_MAX_COUNT/EXEC_ARG_MAX_LEN bounds syscall 16
///   already validates before ever calling in) -> E2BIG, matching syscall
///   16's own E2BIG usage for the identical real cause.
/// - everything else (create_process_from_elf()'s own structural ELF
///   rejections: bad segment count/alignment/p4 index/entry-not-in-
///   segment/etc) -> ENOEXEC, the real POSIX "exec format error" bucket --
///   the same family as the earlier elf::parse() failure this same
///   syscall already reports as ENOEXEC, just caught one validation pass
///   later.
fn exec_err_errno(msg: &str) -> u64 {
    if msg.contains("no such process") || msg.contains("slot vanished") {
        errno::ESRCH
    } else if msg.contains("out of physical frames") || msg.contains("frame allocator not installed") {
        errno::ENOMEM
    } else if msg.contains("argv") || msg.contains("envp") {
        errno::E2BIG
    } else {
        errno::ENOEXEC
    }
}

/// Ordinary Rust, called (via a plain "call") from syscall_entry's asm
/// with rdi = pointer to the just-saved SyscallRegs. Reads the syscall
/// number from the saved rax; for syscall 0 (MILESTONE 31: write(ptr,
/// len)) reads `len` bytes starting at `ptr` -- both real arguments now,
/// taken from the saved rdi/rsi exactly as the SysV-mirroring convention
/// enter_ring3()/USER_PROGRAM already use elsewhere in this file -- out
/// of whatever address space is CURRENTLY loaded in CR3, and writes them
/// raw to serial, then returns, letting syscall_entry pop registers and
/// iretq back into ring 3 for the second `int 0x80`; for syscall 1
/// (exit) never returns to syscall_entry at all -- calls resume_kernel()
/// directly, which abandons this call's own stack frame (on the
/// TSS.privilege_stack_table[0] syscall stack) entirely and jumps back
/// into run()'s caller instead.
extern "C" fn syscall_dispatch(regs: *mut SyscallRegs) {
    // MILESTONE 33: mutable now (was `&*regs`) -- syscall 2 (sbrk, below)
    // needs to write its return value into the saved rax slot so
    // syscall_entry's "pop rax" delivers it back into the real register
    // ring-3 code reads after `int 0x80` returns. Every read-only use
    // elsewhere in this function still works unchanged through a `&mut`.
    let regs = unsafe { &mut *regs };
    // MILESTONE 27, the crucial verification detail: this CS came from
    // the CPU's OWN interrupt-frame push, not anything the (potentially
    // buggy) ring-3 code claimed about itself -- its low 2 bits are the
    // hardware-recorded CPL at the moment `int 0x80` executed. This is
    // the actual proof CPL=3 was real, logged unconditionally on every
    // syscall regardless of which one it is.
    let hardware_cpl = regs.cs & 0b11;
    // MILESTONE 30: ACTIVE_PROCESS is nonzero exactly while a
    // process.rs-owned process is the one running in ring 3 (set by
    // process::run() right before the CR3 switch, cleared below right
    // after restoring the kernel's own CR3). 0 means this is a plain,
    // unmodified `usertest` excursion -- still under the kernel's own
    // page tables the whole time, exactly as Milestone 27 left it.
    let active = crate::process::ACTIVE_PROCESS.load(Ordering::SeqCst);
    match regs.rax {
        0 => {
            // MILESTONE 31: real write(ptr, len) -- rdi = ptr, rsi = len,
            // the same register convention USER_PROGRAM's hand-assembled
            // `mov rdi, imm64` / `mov esi, imm32` sets up before `int
            // 0x80`. See MAX_WRITE_LEN's own comment for exactly what
            // safety net this cap is (and is NOT).
            let ptr = regs.rdi;
            let requested_len = regs.rsi;
            let truncated = requested_len > MAX_WRITE_LEN;
            let len = if truncated { MAX_WRITE_LEN } else { requested_len } as usize;

            // MILESTONE 44 self-test support: see
            // process::LAST_WRITE_SYSCALL_PID's own doc comment -- real
            // proof a process's ring-3 execution reached THIS syscall,
            // independent of whether the excursion later returns cleanly
            // (Milestone 41's page-fault handler resumes kernel context
            // the same way a clean exit does, so "no panic" alone can't
            // tell the two apart).
            crate::process::LAST_WRITE_SYSCALL_PID.store(active, Ordering::SeqCst);

            if truncated {
                let _ = writeln!(
                    serial(),
                    "milestone 31: syscall WRITE -- requested len {requested_len} exceeds MAX_WRITE_LEN {MAX_WRITE_LEN}, truncating"
                );
            }

            if active != 0 {
                let _ = writeln!(
                    serial(),
                    "milestone 31: syscall WRITE (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- ptr={:#x} len={len} -- raw bytes >>>",
                    regs.cs, ptr
                );
            } else {
                let _ = writeln!(
                    serial(),
                    "milestone 31: syscall WRITE (legacy usertest, no active process) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- ptr={:#x} len={len} -- raw bytes >>>",
                    regs.cs, ptr
                );
            }

            // MILESTONE 31, the actual isolation proof, generalized from
            // Milestone 30's read_active_message(): this reads `len`
            // bytes starting at the CALLER-SUPPLIED virtual address `ptr`
            // through whatever CR3 is CURRENTLY loaded -- still the
            // active process's own private PML4 when active != 0 (the
            // exit syscall below is what switches CR3 back, not this
            // one), or the kernel's own shared page tables when active
            // == 0 -- so an identical virtual `ptr` genuinely resolves to
            // different physical bytes depending on which process called
            // in, exactly like Milestone 30 proved for one fixed offset,
            // now for an arbitrary pointer+length pair.
            let mut port = serial();
            for i in 0..len {
                let byte = unsafe { core::ptr::read((ptr as *const u8).wrapping_add(i)) };
                port.send(byte);
            }
            let _ = writeln!(port, "\n<<< end of write ({len} bytes)");

            if active != 0 {
                // MILESTONE 33: same technique, same timing (still under
                // this process's own CR3, the exit syscall below is what
                // restores the kernel's) -- reads the marker byte this
                // process's own machine code wrote via the sbrk syscall
                // into its own private heap, at the identical virtual
                // address HEAP_START every process uses. This is the
                // real per-process-heap isolation proof.
                //
                // MILESTONE 57: a REAL, pre-existing bug found and fixed
                // during this milestone's own verification (see
                // process::heap_page0_mapped()'s own doc comment for the
                // full story) -- heap page 0 is no longer guaranteed to
                // be mapped just because a process exists (demand paging
                // means it's only backed once something actually touches
                // it), so this now checks FIRST rather than
                // unconditionally dereferencing it. Most hardcoded test
                // processes (SIGSEGV_TEST_PROCESS, FAULT_TEST_PROCESS,
                // WAITSTATUS_TEST_PROCESS, EXEC_TEST_PROCESS, etc.) never
                // call sbrk() at all, so this is the common case now, not
                // an edge case -- reported honestly rather than papered
                // over with a fabricated marker value.
                if crate::process::heap_page0_mapped(active) {
                    let heap_marker = crate::process::read_active_heap_marker();
                    let _ = writeln!(
                        serial(),
                        "milestone 33: syscall WRITE (process {active}) also reads its own private heap at {:#x} -- marker byte {:#04x} ('{}')",
                        crate::process::HEAP_START,
                        heap_marker,
                        heap_marker as char
                    );
                } else {
                    let _ = writeln!(
                        serial(),
                        "milestone 57: syscall WRITE (process {active}) -- heap page 0 at {:#x} is not mapped yet (this process has never touched its heap -- demand paging means no physical frame exists there to read)",
                        crate::process::HEAP_START
                    );
                }
            }
        }
        1 => {
            // MILESTONE 43: real exit-status capture -- rdi holds the
            // caller's exit code (`mov edi, N` before `mov eax, 1`),
            // same rdi-as-first-argument convention every other syscall
            // in this dispatch already uses. Only meaningful -- and only
            // read -- for a forked child's OWN exit() (IN_CHILD_RESUME
            // true): a top-level process's exit() has no parent wait()ing
            // on it in this design, so there's nothing to report it to.
            if IN_CHILD_RESUME.load(Ordering::SeqCst) {
                LAST_CHILD_EXIT_CODE.store(regs.rdi as u8, Ordering::SeqCst);
            }
            let _ = writeln!(
                serial(),
                "milestone {}: syscall EXIT -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- discarding ring-3 context, resuming kernel context",
                if active != 0 { 30 } else { 27 },
                regs.cs
            );
            if active != 0 {
                // MILESTONE 30: restore the kernel's own original PML4
                // BEFORE resume_kernel() hands control back to ordinary
                // kernel code, per the milestone's required ordering --
                // real belt-and-suspenders here, since the shared-entry
                // design above means kernel code would actually stay
                // reachable even under the process's own CR3 (every
                // kernel-space PML4 entry is a shared pointer into the
                // SAME P3 tables), but restoring explicitly and
                // immediately is the honest, verified-not-assumed
                // choice rather than relying on that as an excuse to
                // skip it.
                crate::process::restore_kernel_cr3();
                crate::process::ACTIVE_PROCESS.store(0, Ordering::SeqCst);
            }
            // MILESTONE 37: this exit() might be unwinding a TOP-LEVEL
            // excursion (usertest::run()/process::run() and friends,
            // entered via enter_ring3_now(), whose own kernel-side
            // resume point lives in KERNEL_RSP) OR a NESTED forked-
            // child excursion (process::run_forked_child(), entered via
            // enter_ring3_as_forked_child(), whose resume point lives in
            // the separate CHILD_KERNEL_RSP instead) -- IN_CHILD_RESUME
            // is still true here if it's the latter (cleared only AFTER
            // enter_ring3_as_forked_child() itself returns, which
            // hasn't happened yet at this point in the call chain), so
            // reading it now picks the anchor that actually matches
            // which excursion is really unwinding.
            let saved = if IN_CHILD_RESUME.load(Ordering::SeqCst) {
                CHILD_KERNEL_RSP.load(Ordering::SeqCst)
            } else {
                KERNEL_RSP.load(Ordering::SeqCst)
            };
            unsafe { resume_kernel(saved) };
        }
        2 => {
            // MILESTONE 33: sbrk-style heap-grow syscall -- rdi holds the
            // requested byte count (ring-3 code loads it via `mov edi,
            // N` before `int 0x80`, per the SysV-ish convention this
            // ABI's SyscallRegs struct already captures every argument
            // register for). Only meaningful with an active per-process
            // heap (process.rs's create_process() pre-maps one per
            // process); the plain, unmodified `usertest` excursion (no
            // process.rs involvement, active == 0) has no heap mapped at
            // all, so it's honestly refused rather than silently
            // returning a pointer into nothing.
            if active != 0 {
                let size = regs.rdi;
                match crate::process::sbrk(active, size) {
                    Some(ptr) => {
                        let _ = writeln!(
                            serial(),
                            "milestone 33: syscall SBRK (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- requested {size} bytes, returning heap pointer {:#x}",
                            regs.cs, ptr
                        );
                        regs.rax = ptr;
                    }
                    None => {
                        let _ = writeln!(
                            serial(),
                            "milestone 33: syscall SBRK (process {active}) -- FAILED (requested {size} bytes would exceed this process's fixed per-process heap) -- returning 0"
                        );
                        // MILESTONE 59: real ENOMEM -- errno.rs's own doc
                        // comment on ENOMEM names exactly this case
                        // ("sbrk()'s request would exceed this process's
                        // fixed per-process heap reservation").
                        crate::process::set_errno(active, errno::ENOMEM);
                        regs.rax = 0;
                    }
                }
            } else {
                let _ = writeln!(
                    serial(),
                    "milestone 33: syscall SBRK called with no active process (plain usertest excursion has no per-process heap mapped) -- ignoring, returning 0"
                );
                // MILESTONE 59: no errno set here -- the legacy
                // ACTIVE_PROCESS==0 excursion has no Process struct at
                // all to hold one (set_errno(0, ...) would be a real,
                // silent no-op, not a meaningful ESRCH report).
            }
        }
        3 => {
            // MILESTONE 35: open(path_ptr, path_len) -> fd (u64::MAX on
            // failure). rdi = path_ptr, rsi = path_len -- reads the path
            // string out of whatever CR3 is CURRENTLY loaded, the EXACT
            // same technique syscall 0 (write) already uses to read its
            // own ptr/len argument, just applied to a path instead of a
            // message. See MAX_PATH_LEN's own comment for what this cap
            // does and does not protect against.
            // MILESTONE 82: this arm is UNCHANGED from Milestone 35 --
            // it only ever reads rdi/rsi, never rdx. Real O_TRUNC lives
            // in its OWN distinct syscall number (24, below), NOT bolted
            // onto this one as a new rdx argument -- the exact same
            // reasoning syscall 23 (mmap_writable) already documents for
            // staying separate from syscall 21: this kernel has six
            // hand-assembled OPEN callers (the Milestone 64/65 MMAP test
            // programs in process.rs) that never zero rdx before `int
            // 0x80`, so reading rdx here would misread their leftover
            // register as a truncate request and silently truncate the
            // file they meant only to read. `false` is passed literally.
            let path_ptr = regs.rdi;
            let requested_len = regs.rsi;
            let truncated = requested_len > MAX_PATH_LEN;
            let len = if truncated { MAX_PATH_LEN } else { requested_len } as usize;
            if truncated {
                let _ = writeln!(
                    serial(),
                    "milestone 35: syscall OPEN -- requested path_len {requested_len} exceeds MAX_PATH_LEN {MAX_PATH_LEN}, truncating"
                );
            }
            let mut path_bytes = alloc::vec::Vec::with_capacity(len);
            for i in 0..len {
                path_bytes.push(unsafe { core::ptr::read((path_ptr as *const u8).wrapping_add(i)) });
            }
            match (active, core::str::from_utf8(&path_bytes)) {
                (0, _) => {
                    let _ = writeln!(
                        serial(),
                        "milestone 35: syscall OPEN called with no active process (plain usertest excursion has no fd table) -- ignoring, returning u64::MAX"
                    );
                    // MILESTONE 59: no errno storage for the legacy
                    // ACTIVE_PROCESS==0 path -- see syscall 2's own
                    // comment above for why.
                    regs.rax = u64::MAX;
                }
                (_, Err(_)) => {
                    let _ = writeln!(serial(), "milestone 35: syscall OPEN (process {active}) -- path is not valid UTF-8, returning u64::MAX");
                    // MILESTONE 59: real EINVAL -- a malformed argument,
                    // not one of the more specific codes above.
                    crate::process::set_errno(active, errno::EINVAL);
                    regs.rax = u64::MAX;
                }
                (_, Ok(path)) => match crate::process::open_file(active, path, false) {
                    Some(fd) => {
                        let _ = writeln!(
                            serial(),
                            "milestone 35: syscall OPEN (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- path='{path}' -> fd {fd}",
                            regs.cs
                        );
                        regs.rax = fd;
                    }
                    None => {
                        let _ = writeln!(
                            serial(),
                            "milestone 35: syscall OPEN (process {active}) -- FAILED for path='{path}' (fd table full -- max {} open files per process) -- returning u64::MAX",
                            crate::process::MAX_OPEN_FILES
                        );
                        // MILESTONE 59: real EMFILE -- errno.rs's own doc
                        // comment on EMFILE names this exact case.
                        crate::process::set_errno(active, errno::EMFILE);
                        regs.rax = u64::MAX;
                    }
                },
            }
        }
        4 => {
            // MILESTONE 35: read(fd, buf_ptr, len) -> bytes_read
            // (u64::MAX if fd is invalid). rdi = fd, rsi = buf_ptr, rdx =
            // len. Copies out of the fd's already-buffered contents
            // (process::read_fd) into the CALLER's own memory at
            // buf_ptr, through whatever CR3 is currently loaded -- same
            // write-into-user-memory technique as the sbrk pointer
            // return, just copying a whole byte range instead of a
            // single pointer value.
            let fd = regs.rdi;
            let buf_ptr = regs.rsi;
            let requested_len = regs.rdx;
            let truncated = requested_len > MAX_FD_IO_LEN;
            let len = if truncated { MAX_FD_IO_LEN } else { requested_len } as usize;
            if truncated {
                let _ = writeln!(
                    serial(),
                    "milestone 35: syscall READ -- requested len {requested_len} exceeds MAX_FD_IO_LEN {MAX_FD_IO_LEN}, truncating"
                );
            }
            if active == 0 {
                let _ = writeln!(
                    serial(),
                    "milestone 35: syscall READ called with no active process (plain usertest excursion has no fd table) -- ignoring, returning u64::MAX"
                );
                regs.rax = u64::MAX;
            } else {
                match crate::process::read_fd(active, fd, len) {
                    Some(data) => {
                        let n = data.len();
                        for (i, b) in data.iter().enumerate() {
                            unsafe { core::ptr::write((buf_ptr as *mut u8).wrapping_add(i), *b) };
                        }
                        let _ = writeln!(
                            serial(),
                            "milestone 35: syscall READ (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- fd={fd} requested={len} actual={n} bytes",
                            regs.cs
                        );
                        regs.rax = n as u64;
                    }
                    None => {
                        let _ = writeln!(
                            serial(),
                            "milestone 35: syscall READ (process {active}) -- FAILED, fd {fd} is not open -- returning u64::MAX"
                        );
                        // MILESTONE 59: real EBADF -- errno.rs's own doc
                        // comment on EBADF names read/fdwrite/close/dup/
                        // dup2 given an fd that doesn't name a
                        // currently-open descriptor.
                        crate::process::set_errno(active, errno::EBADF);
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        5 => {
            // MILESTONE 35: fdwrite(fd, ptr, len) -> bytes_written
            // (u64::MAX if fd is invalid). rdi = fd, rsi = ptr, rdx =
            // len. Deliberately a DIFFERENT syscall number from syscall
            // 0 (write) -- see this file's own module doc comment and
            // process.rs's OpenFile doc comment for the full "new
            // syscalls, not a generalized write(fd,ptr,len)" reasoning.
            // Reads `len` bytes out of the CALLER's own memory at `ptr`
            // (same read-through-current-CR3 technique syscall 0/open
            // already use), hands them to process::write_fd(), which may
            // accept FEWER bytes than requested if this would overflow
            // fs::MAX_FILE_BYTES -- the real return value here is
            // whatever write_fd() actually accepted, not just an echo of
            // `len`.
            let fd = regs.rdi;
            let ptr = regs.rsi;
            let requested_len = regs.rdx;
            let truncated = requested_len > MAX_FD_IO_LEN;
            let len = if truncated { MAX_FD_IO_LEN } else { requested_len } as usize;
            if truncated {
                let _ = writeln!(
                    serial(),
                    "milestone 35: syscall FDWRITE -- requested len {requested_len} exceeds MAX_FD_IO_LEN {MAX_FD_IO_LEN}, truncating"
                );
            }
            if active == 0 {
                let _ = writeln!(
                    serial(),
                    "milestone 35: syscall FDWRITE called with no active process (plain usertest excursion has no fd table) -- ignoring, returning u64::MAX"
                );
                regs.rax = u64::MAX;
            } else {
                let mut data = alloc::vec::Vec::with_capacity(len);
                for i in 0..len {
                    data.push(unsafe { core::ptr::read((ptr as *const u8).wrapping_add(i)) });
                }
                match crate::process::write_fd(active, fd, &data) {
                    Some(n) => {
                        let _ = writeln!(
                            serial(),
                            "milestone 35: syscall FDWRITE (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- fd={fd} requested={len} accepted={n} bytes{}",
                            regs.cs,
                            if n < len { " (TRUNCATED -- would have exceeded fs::MAX_FILE_BYTES)" } else { "" }
                        );
                        regs.rax = n as u64;
                    }
                    None => {
                        let _ = writeln!(
                            serial(),
                            "milestone 35: syscall FDWRITE (process {active}) -- FAILED, fd {fd} is not open -- returning u64::MAX"
                        );
                        // MILESTONE 59: real EBADF -- same reasoning as
                        // syscall 4 (READ)'s own EBADF arm above.
                        crate::process::set_errno(active, errno::EBADF);
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        6 => {
            // MILESTONE 35: close(fd) -> status (0=success, 1=invalid
            // fd, 2=fd released but the on-disk persist failed). rdi =
            // fd. This is the ONLY point in this milestone's design
            // where a write actually reaches disk (process::close_fd's
            // own doc comment explains why) -- see the serial log lines
            // it emits for exactly what got persisted.
            let fd = regs.rdi;
            if active == 0 {
                let _ = writeln!(
                    serial(),
                    "milestone 35: syscall CLOSE called with no active process (plain usertest excursion has no fd table) -- ignoring, returning u64::MAX"
                );
                regs.rax = u64::MAX;
            } else {
                match crate::process::close_fd(active, fd) {
                    Some(true) => {
                        let _ = writeln!(
                            serial(),
                            "milestone 35: syscall CLOSE (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- fd={fd} closed successfully",
                            regs.cs
                        );
                        regs.rax = 0;
                    }
                    Some(false) => {
                        // MILESTONE 59: real EIO -- errno.rs's own doc
                        // comment on EIO names exactly this case (the
                        // fd was released, but its real on-disk persist
                        // failed).
                        let _ = writeln!(
                            serial(),
                            "milestone 35: syscall CLOSE (process {active}) -- fd={fd} released but its real on-disk persist FAILED -- returning status 2"
                        );
                        crate::process::set_errno(active, errno::EIO);
                        regs.rax = 2;
                    }
                    None => {
                        let _ = writeln!(
                            serial(),
                            "milestone 35: syscall CLOSE (process {active}) -- FAILED, fd {fd} was not open -- returning status 1"
                        );
                        // MILESTONE 59: real EBADF -- same reasoning as
                        // syscall 4 (READ)'s own EBADF arm above.
                        crate::process::set_errno(active, errno::EBADF);
                        regs.rax = 1;
                    }
                }
            }
        }
        7 => {
            // MILESTONE 37: fork() -- takes no arguments. Returns the
            // new child's pid in rax to the PARENT (this int 0x80
            // returns normally, exactly like every other syscall
            // above); the CHILD only ever sees rax=0 much LATER, when
            // process::run_forked_child() actually resumes it (forced
            // by usertest::enter_ring3_as_forked_child()'s own
            // "xor eax, eax" right before its iretq -- not decided
            // here). regs.rip/regs.rsp are the hardware-recorded
            // InterruptStackFrame values for THIS int 0x80 -- handed to
            // process::fork() unmodified as the child's own future
            // resume point.
            if active == 0 {
                let _ = writeln!(
                    serial(),
                    "milestone 37: syscall FORK called with no active process (plain usertest excursion cannot fork) -- ignoring, returning u64::MAX"
                );
                regs.rax = u64::MAX;
            } else {
                match crate::process::fork(active, regs.rip, regs.rsp) {
                    Some(child_pid) => {
                        let _ = writeln!(
                            serial(),
                            "milestone 37: syscall FORK (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- created child pid {child_pid}",
                            regs.cs
                        );
                        regs.rax = child_pid as u64;
                    }
                    None => {
                        let _ = writeln!(
                            serial(),
                            "milestone 37: syscall FORK (process {active}) -- FAILED (see process.rs's own log line just above for the real reason) -- returning u64::MAX"
                        );
                        // MILESTONE 59: real EAGAIN -- errno.rs's own doc
                        // comment on EAGAIN covers every real cause
                        // fork() can fail for today (table full, out of
                        // physical frames, or nested-excursion refusal).
                        crate::process::set_errno(active, errno::EAGAIN);
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        8 => {
            // MILESTONE 37: wait(child_pid) -- rdi = child_pid. See
            // process::wait_for_child()'s own doc comment for exactly
            // what this does and why it's a real, honest implementation
            // of "block until the child changes state" for this
            // milestone's synchronous execution model -- this int 0x80
            // genuinely does not return until the child has run all the
            // way to its own exit().
            if active == 0 {
                let _ = writeln!(serial(), "milestone 37: syscall WAIT called with no active process -- ignoring, returning u64::MAX");
                regs.rax = u64::MAX;
            } else {
                let child_pid_arg = regs.rdi;
                if child_pid_arg > u8::MAX as u64 {
                    let _ = writeln!(serial(), "milestone 37: syscall WAIT (process {active}) -- pid argument {child_pid_arg} out of range -- returning u64::MAX");
                    // MILESTONE 59: real EINVAL -- the pid argument
                    // doesn't fit u8, errno.rs's own EINVAL doc names
                    // this exact case.
                    crate::process::set_errno(active, errno::EINVAL);
                    regs.rax = u64::MAX;
                } else {
                    // MILESTONE 43: rax encoding for a successful wait()
                    // -- bits 0-7 = reaped child pid, bits 8-15 = real
                    // exit code (only meaningful if bit 16 is set),
                    // bit 16 = 1 if the child ran to its own exit()
                    // (WaitOutcome::Exited), 0 if it was killed before
                    // ever running (WaitOutcome::Killed) -- documented
                    // here AND in the hand-assembled test programs that
                    // decode it, checked by hand against each other, not
                    // assumed consistent.
                    //
                    // MILESTONE 53: bit 17 = 1 if the child actually ran
                    // but was signal-terminated by a real hardware fault
                    // instead of reaching its own exit()
                    // (WaitOutcome::Signaled) -- a real third state M43's
                    // original two-bit-16-values-only scheme (0=Killed,
                    // 1=Exited) had no room for. Mutually exclusive with
                    // bit 16 by construction (the match below sets
                    // exactly one of the three encodings), so no existing
                    // decoder that only ever checked bit 16 -- e.g. this
                    // milestone deliberately did NOT touch WAITSTATUS_
                    // TEST_PROGRAM's own decode logic -- misreads a
                    // Signaled child as Exited; it would see bit 16 = 0
                    // and correctly-if-incompletely read that as "not a
                    // normal exit", the same as it already does for
                    // Killed today.
                    match crate::process::wait_for_child(active, child_pid_arg as u8) {
                        Some((reaped, outcome)) => {
                            let encoded = match outcome {
                                crate::process::WaitOutcome::Exited(code) => {
                                    let _ = writeln!(
                                        serial(),
                                        "milestone 43: syscall WAIT (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- reaped child pid {reaped}, exited normally with code {code}",
                                        regs.cs
                                    );
                                    (reaped as u64) | ((code as u64) << 8) | (1u64 << 16)
                                }
                                crate::process::WaitOutcome::Killed => {
                                    let _ = writeln!(
                                        serial(),
                                        "milestone 43: syscall WAIT (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- child pid {reaped} was killed, never ran",
                                        regs.cs
                                    );
                                    reaped as u64
                                }
                                crate::process::WaitOutcome::Signaled => {
                                    let _ = writeln!(
                                        serial(),
                                        "milestone 53: syscall WAIT (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- child pid {reaped} was signal-terminated (real hardware fault) after actually running",
                                        regs.cs
                                    );
                                    (reaped as u64) | (1u64 << 17)
                                }
                            };
                            regs.rax = encoded;
                        }
                        None => {
                            let _ = writeln!(
                                serial(),
                                "milestone 37: syscall WAIT (process {active}) -- FAILED (pid {child_pid_arg} is not a live child of this process, or see process.rs's own log line above) -- returning u64::MAX"
                            );
                            // MILESTONE 59: real ECHILD -- errno.rs's own
                            // doc comment on ECHILD names exactly this
                            // case (wait()'s target pid is not a live
                            // child of the caller).
                            crate::process::set_errno(active, errno::ECHILD);
                            regs.rax = u64::MAX;
                        }
                    }
                }
            }
        }
        9 => {
            // MILESTONE 37 (ABI) / MILESTONE 45 (real implementation):
            // exec(path_ptr, path_len) -- rdi=path_ptr, rsi=path_len, the
            // same read-through-current-CR3 technique open() (syscall 3)
            // already uses for its own path argument -- the syscall's
            // OWN calling convention is completely unchanged from
            // Milestone 37. What runs underneath it is not: this reads
            // the target file, parses it as a REAL ELF64 image
            // (elf::parse(), the same parser `runelf` uses), and on a
            // valid ELF hands it to process::exec_elf() -- the real
            // teardown-and-rebuild implementation (see that function's
            // own doc comment). On SUCCESS this call never reaches the
            // bottom of this match arm at all -- exec_replace_and_enter()
            // diverges (iretq's straight into the new program's real,
            // parsed e_entry), the same "never return, resume ring 3
            // directly" shape syscall 1 (exit) already established for
            // resuming KERNEL context, applied here to resume RING-3
            // context instead.
            //
            // MILESTONE 45, an honest, disclosed behavior change from
            // Milestone 37: exec() now requires a real ELF64 image, the
            // same format `runelf` has always required -- a flat,
            // headerless binary (e.g. Milestone 34's `testprog`, built
            // by `seedtestprog`) is no longer exec()-able; a real
            // ELF64-vs-not check now runs where Milestone 37's own
            // placeholder never checked the file's format at all (it
            // just copied whatever bytes were there). This is the same
            // real-world tradeoff execve() itself makes (ENOEXEC for a
            // non-executable-format file) -- not a silently narrower
            // feature, a genuinely stricter and more real one. Any
            // caller relying on the old flat-binary target (e.g.
            // FORK_TEST_PROGRAM's own child, which exec()s "testprog")
            // now takes the SAME honest fallback path it already had for
            // "file does not exist" -- degrades cleanly, does not crash.
            let path_ptr = regs.rdi;
            let requested_len = regs.rsi;
            let truncated = requested_len > MAX_PATH_LEN;
            let len = if truncated { MAX_PATH_LEN } else { requested_len } as usize;
            if active == 0 {
                let _ = writeln!(serial(), "milestone 37: syscall EXEC called with no active process -- ignoring, returning u64::MAX");
                regs.rax = u64::MAX;
            } else {
                let mut path_bytes = alloc::vec::Vec::with_capacity(len);
                for i in 0..len {
                    path_bytes.push(unsafe { core::ptr::read((path_ptr as *const u8).wrapping_add(i)) });
                }
                match core::str::from_utf8(&path_bytes) {
                    Ok(path) => match crate::fs::read_file(path) {
                        Ok(bytes) => match crate::elf::parse(&bytes) {
                            Ok(elf_image) => match crate::process::exec_elf(active, &bytes, &elf_image) {
                                Ok(new_entry) => {
                                    let _ = writeln!(
                                        serial(),
                                        "milestone 45: syscall EXEC (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- REAL teardown-and-rebuild from '{path}' ({} real ELF bytes read from the on-disk filesystem), new address space, jumping directly into the real parsed entry {:#x}, never returning to the old one",
                                        regs.cs,
                                        bytes.len(),
                                        new_entry
                                    );
                                    exec_replace_and_enter(new_entry);
                                }
                                Err(e) => {
                                    let _ = writeln!(
                                        serial(),
                                        "milestone 45: syscall EXEC (process {active}) -- FAILED, {e} -- returning u64::MAX, old program continues running"
                                    );
                                    // MILESTONE 59: see exec_err_errno()'s
                                    // own doc comment for the real
                                    // reasoning behind this mapping.
                                    crate::process::set_errno(active, exec_err_errno(e));
                                    regs.rax = u64::MAX;
                                }
                            },
                            Err(e) => {
                                let _ = writeln!(
                                    serial(),
                                    "milestone 45: syscall EXEC (process {active}) -- FAILED, '{path}' is not a valid ELF64 image ({e}) -- returning u64::MAX, old program continues running"
                                );
                                // MILESTONE 59: real ENOEXEC -- errno.rs's
                                // own doc comment on ENOEXEC names
                                // exactly this case.
                                crate::process::set_errno(active, errno::ENOEXEC);
                                regs.rax = u64::MAX;
                            }
                        },
                        Err(e) => {
                            let _ = writeln!(
                                serial(),
                                "milestone 37: syscall EXEC (process {active}) -- FAILED, could not read '{path}' off the real on-disk filesystem: {e} -- returning u64::MAX, old program continues running"
                            );
                            // MILESTONE 59: real ENOENT -- errno.rs's own
                            // doc comment on ENOENT names exactly this
                            // case.
                            crate::process::set_errno(active, errno::ENOENT);
                            regs.rax = u64::MAX;
                        }
                    },
                    Err(_) => {
                        let _ = writeln!(serial(), "milestone 37: syscall EXEC (process {active}) -- path is not valid UTF-8 -- returning u64::MAX");
                        // MILESTONE 59: real EINVAL -- malformed argument.
                        crate::process::set_errno(active, errno::EINVAL);
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        10 => {
            // MILESTONE 40: pipe(pipefd_ptr) -> 0 on success, u64::MAX
            // on failure. rdi = pointer to TWO consecutive u64 slots in
            // the caller's own memory -- offset 0 gets the new read fd,
            // offset 8 gets the new write fd (POSIX's own
            // `pipe(int pipefd[2])` out-param shape, just u64-per-slot
            // instead of int-per-slot to match this ABI's existing
            // convention of passing/returning whole registers). Reads
            // 3/4/5/6 (open/read/fdwrite/close) already transparently
            // work on the fds this hands back -- process::read_fd()/
            // write_fd()/close_fd() dispatch on the fd table entry's
            // real kind (file vs. pipe end) internally, no separate
            // pipe-specific syscall needed for the actual I/O.
            let pipefd_ptr = regs.rdi;
            if active == 0 {
                let _ = writeln!(serial(), "milestone 40: syscall PIPE called with no active process (plain usertest excursion has no fd table) -- ignoring, returning u64::MAX");
                regs.rax = u64::MAX;
            } else {
                match crate::process::pipe_create(active) {
                    Some((read_fd, write_fd)) => {
                        unsafe {
                            core::ptr::write(pipefd_ptr as *mut u64, read_fd);
                            core::ptr::write((pipefd_ptr as *mut u64).wrapping_add(1), write_fd);
                        }
                        let _ = writeln!(
                            serial(),
                            "milestone 40: syscall PIPE (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- created pipe, read_fd={read_fd} write_fd={write_fd}",
                            regs.cs
                        );
                        regs.rax = 0;
                    }
                    None => {
                        let _ = writeln!(
                            serial(),
                            "milestone 40: syscall PIPE (process {active}) -- FAILED (fd table full, or all MAX_PIPES global pipe slots already in use) -- returning u64::MAX"
                        );
                        // MILESTONE 59: real EMFILE -- errno.rs's own doc
                        // comment on EMFILE explicitly names pipe()'s fd-
                        // table-full/pipe-table-full causes alongside
                        // open()'s.
                        crate::process::set_errno(active, errno::EMFILE);
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        11 => {
            // MILESTONE 40: dup(oldfd) -> new fd (u64::MAX on failure).
            // rdi = oldfd. See process::dup_fd()'s own doc comment for
            // the real-vs-honest-simplified sharing semantics (pipes:
            // real shared reference; files: disclosed deep copy, same
            // limitation fork() already has for files).
            let oldfd = regs.rdi;
            if active == 0 {
                let _ = writeln!(serial(), "milestone 40: syscall DUP called with no active process -- ignoring, returning u64::MAX");
                regs.rax = u64::MAX;
            } else {
                match crate::process::dup_fd(active, oldfd) {
                    Some(newfd) => {
                        let _ = writeln!(
                            serial(),
                            "milestone 40: syscall DUP (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- fd {oldfd} duplicated to new fd {newfd}",
                            regs.cs
                        );
                        regs.rax = newfd;
                    }
                    None => {
                        let _ = writeln!(
                            serial(),
                            "milestone 40: syscall DUP (process {active}) -- FAILED, fd {oldfd} not open or fd table full -- returning u64::MAX"
                        );
                        // MILESTONE 59: real EBADF -- errno.rs's own doc
                        // comment on EBADF explicitly names dup/dup2
                        // alongside read/fdwrite/close (the single
                        // best-fit real errno for this fd-table
                        // implementation's collapsed "not open OR table
                        // full" Option, same disclosed-collapse
                        // reasoning as EMFILE's own doc comment uses for
                        // open()/pipe()).
                        crate::process::set_errno(active, errno::EBADF);
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        12 => {
            // MILESTONE 40: dup2(oldfd, newfd) -> newfd on success
            // (u64::MAX on failure). rdi = oldfd, rsi = newfd. See
            // process::dup2_fd()'s own doc comment -- closes whatever
            // was already at `newfd` first (via the real close_fd()
            // path, not a silent overwrite), and fd==newfd is a real,
            // explicitly-checked no-op rather than accidentally closing
            // the fd it's supposed to preserve.
            let oldfd = regs.rdi;
            let newfd = regs.rsi;
            if active == 0 {
                let _ = writeln!(serial(), "milestone 40: syscall DUP2 called with no active process -- ignoring, returning u64::MAX");
                regs.rax = u64::MAX;
            } else {
                match crate::process::dup2_fd(active, oldfd, newfd) {
                    Some(true) => {
                        let _ = writeln!(
                            serial(),
                            "milestone 40: syscall DUP2 (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- fd {oldfd} duplicated onto fd {newfd}",
                            regs.cs
                        );
                        regs.rax = newfd;
                    }
                    _ => {
                        let _ = writeln!(
                            serial(),
                            "milestone 40: syscall DUP2 (process {active}) -- FAILED, fd {oldfd} not open or newfd {newfd} out of range -- returning u64::MAX"
                        );
                        // MILESTONE 59: real EBADF -- same reasoning as
                        // syscall 11 (DUP)'s own EBADF arm above.
                        crate::process::set_errno(active, errno::EBADF);
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        13 => {
            // MILESTONE 41: kill(pid) -- SIGKILL-equivalent, rdi=target
            // pid. Returns 1 on success, 0 on failure (no process this
            // kernel has ever needed a "returns u64::MAX on failure"
            // convention couldn't just as honestly express as 0/1 here,
            // since a valid pid is never 0). See process::kill()'s own
            // doc comment for exactly what this does and its one real,
            // disclosed limitation.
            if active == 0 {
                let _ = writeln!(serial(), "milestone 41: syscall KILL called with no active process -- ignoring, returning 0");
                regs.rax = 0;
            } else {
                let target_pid_arg = regs.rdi;
                if target_pid_arg > u8::MAX as u64 {
                    let _ = writeln!(serial(), "milestone 41: syscall KILL (process {active}) -- pid argument {target_pid_arg} out of range -- returning 0");
                    // MILESTONE 59: real EINVAL -- pid argument doesn't
                    // fit u8.
                    crate::process::set_errno(active, errno::EINVAL);
                    regs.rax = 0;
                } else {
                    let ok = crate::process::kill(active, target_pid_arg as u8);
                    let _ = writeln!(
                        serial(),
                        "milestone 41: syscall KILL (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- target pid {target_pid_arg}, result={ok}",
                        regs.cs
                    );
                    if !ok {
                        // MILESTONE 59: real ESRCH -- errno.rs's own doc
                        // comment on ESRCH explicitly names kill()'s
                        // target-pid failure alongside getpgid/setpgid.
                        crate::process::set_errno(active, errno::ESRCH);
                    }
                    regs.rax = if ok { 1 } else { 0 };
                }
            }
        }
        14 => {
            // MILESTONE 42: setpgid(target_pid, new_pgid) -- rdi=target
            // pid, rsi=new pgid. Returns 1 on success, 0 on failure, same
            // convention as KILL just above. See process::setpgid()'s own
            // doc comment for the real authorization rule (self or a live
            // child) and its disclosed session-leader scope-cut.
            if active == 0 {
                let _ = writeln!(serial(), "milestone 42: syscall SETPGID called with no active process -- ignoring, returning 0");
                regs.rax = 0;
            } else {
                let target_pid_arg = regs.rdi;
                let new_pgid_arg = regs.rsi;
                if target_pid_arg > u8::MAX as u64 || new_pgid_arg > u8::MAX as u64 {
                    let _ = writeln!(serial(), "milestone 42: syscall SETPGID (process {active}) -- argument out of range -- returning 0");
                    // MILESTONE 59: real EINVAL -- an argument doesn't
                    // fit u8.
                    crate::process::set_errno(active, errno::EINVAL);
                    regs.rax = 0;
                } else {
                    let ok = crate::process::setpgid(active, target_pid_arg as u8, new_pgid_arg as u8);
                    let _ = writeln!(
                        serial(),
                        "milestone 42: syscall SETPGID (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- target pid {target_pid_arg}, new pgid {new_pgid_arg}, result={ok}",
                        regs.cs
                    );
                    if !ok {
                        // MILESTONE 59: real EPERM -- errno.rs's own doc
                        // comment on EPERM names exactly this case
                        // (setpgid on a pid that is neither self nor a
                        // live child of the caller -- process::setpgid()'s
                        // real authorization check).
                        crate::process::set_errno(active, errno::EPERM);
                    }
                    regs.rax = if ok { 1 } else { 0 };
                }
            }
        }
        15 => {
            // MILESTONE 42: getpgid(target_pid) -- rdi=target pid.
            // Returns the real pgid on success, u64::MAX on failure (a
            // real pgid is never u64::MAX, so this is an honest,
            // unambiguous sentinel, matching DUP/DUP2's own convention
            // rather than KILL/SETPGID's 0/1 one -- a pgid IS meaningful
            // data being returned here, not just a yes/no result).
            if active == 0 {
                let _ = writeln!(serial(), "milestone 42: syscall GETPGID called with no active process -- ignoring, returning u64::MAX");
                regs.rax = u64::MAX;
            } else {
                let target_pid_arg = regs.rdi;
                if target_pid_arg > u8::MAX as u64 {
                    let _ = writeln!(serial(), "milestone 42: syscall GETPGID (process {active}) -- pid argument {target_pid_arg} out of range -- returning u64::MAX");
                    // MILESTONE 59: real EINVAL -- pid argument doesn't
                    // fit u8.
                    crate::process::set_errno(active, errno::EINVAL);
                    regs.rax = u64::MAX;
                } else {
                    match crate::process::getpgid(target_pid_arg as u8) {
                        Some(pgid) => {
                            let _ = writeln!(
                                serial(),
                                "milestone 42: syscall GETPGID (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- target pid {target_pid_arg}, pgid={pgid}",
                                regs.cs
                            );
                            regs.rax = pgid as u64;
                        }
                        None => {
                            let _ = writeln!(serial(), "milestone 42: syscall GETPGID (process {active}) -- FAILED, pid {target_pid_arg} not a live process -- returning u64::MAX");
                            // MILESTONE 59: real ESRCH -- errno.rs's own
                            // doc comment on ESRCH explicitly names
                            // getpgid's target-pid failure alongside
                            // kill/setpgid.
                            crate::process::set_errno(active, errno::ESRCH);
                            regs.rax = u64::MAX;
                        }
                    }
                }
            }
        }
        16 => {
            // MILESTONE 58: execargv(path_ptr, path_len, argv_ptr,
            // argv_count, envp_ptr, envp_count) -- rdi=path_ptr,
            // rsi=path_len (identical to syscall 9's own first two
            // args), rdx=argv_ptr, r10=argv_count, r8=envp_ptr,
            // r9=envp_count. `argv_ptr`/`envp_ptr` each point to a
            // caller-owned array of `argv_count`/`envp_count` 16-byte
            // entries -- (str_ptr: u64, str_len: u64), the SAME
            // ptr+len idiom this kernel already uses for every other
            // string-bearing syscall argument (path, write buffers,
            // etc.) rather than NUL-terminated C strings -- matching
            // `#[repr(C)] struct ArgSpec { ptr: u64, len: u64 }` on the
            // userspace side (see tools/argvlauncher_src/main.rs).
            // Deliberately a NEW syscall number rather than changing
            // syscall 9's own ABI: EXEC_TEST_PROGRAM (Milestone 45,
            // hand-assembled raw machine code) calls syscall 9 with
            // only rdi/rsi ever set, so extending syscall 9 in place
            // would have left rdx/r10/r8/r9 as whatever garbage
            // happened to be in those registers at that hand-assembled
            // call site -- a real regression risk for zero benefit.
            // This path is entirely additive; syscall 9's own arm above
            // is completely unchanged.
            let path_ptr = regs.rdi;
            let requested_len = regs.rsi;
            let argv_ptr = regs.rdx;
            let argv_count = regs.r10;
            let envp_ptr = regs.r8;
            let envp_count = regs.r9;
            let truncated = requested_len > MAX_PATH_LEN;
            let len = if truncated { MAX_PATH_LEN } else { requested_len } as usize;

            if active == 0 {
                let _ = writeln!(serial(), "milestone 58: syscall EXECARGV called with no active process -- ignoring, returning u64::MAX");
                regs.rax = u64::MAX;
            } else if argv_count as usize > crate::process::EXEC_ARGV_MAX_COUNT || envp_count as usize > crate::process::EXEC_ARGV_MAX_COUNT {
                let _ = writeln!(
                    serial(),
                    "milestone 58: syscall EXECARGV (process {active}) -- FAILED, argv_count={argv_count}/envp_count={envp_count} exceeds this loader's fixed cap {} -- returning u64::MAX",
                    crate::process::EXEC_ARGV_MAX_COUNT
                );
                // MILESTONE 59: real E2BIG -- errno.rs's own doc comment
                // on E2BIG explicitly names this exact case.
                crate::process::set_errno(active, errno::E2BIG);
                regs.rax = u64::MAX;
            } else {
                let mut path_bytes = alloc::vec::Vec::with_capacity(len);
                for i in 0..len {
                    path_bytes.push(unsafe { core::ptr::read((path_ptr as *const u8).wrapping_add(i)) });
                }

                // Real strings read through the CALLING process's own
                // still-active CR3 (this happens BEFORE
                // process::exec_elf_with_args() tears anything down),
                // same "read through current CR3" technique syscall 9
                // already uses for `path` above -- each entry's own
                // `str_len` is capped at EXEC_ARG_MAX_LEN before the
                // read, never trusted un-bounded.
                let read_arg_array = |arr_ptr: u64, count: u64| -> Result<alloc::vec::Vec<alloc::vec::Vec<u8>>, &'static str> {
                    let mut out = alloc::vec::Vec::with_capacity(count as usize);
                    for i in 0..count {
                        let entry_ptr = (arr_ptr as *const u64).wrapping_add((i * 2) as usize);
                        let str_ptr = unsafe { core::ptr::read(entry_ptr) };
                        let str_len = unsafe { core::ptr::read(entry_ptr.wrapping_add(1)) };
                        if str_len as usize > crate::process::EXEC_ARG_MAX_LEN {
                            return Err("exec: an argv/envp string's declared length exceeds this loader's fixed per-string cap");
                        }
                        let mut bytes = alloc::vec::Vec::with_capacity(str_len as usize);
                        for j in 0..str_len as usize {
                            bytes.push(unsafe { core::ptr::read((str_ptr as *const u8).wrapping_add(j)) });
                        }
                        out.push(bytes);
                    }
                    Ok(out)
                };

                match (read_arg_array(argv_ptr, argv_count), read_arg_array(envp_ptr, envp_count)) {
                    (Ok(argv), Ok(envp)) => match core::str::from_utf8(&path_bytes) {
                        Ok(path) => match crate::fs::read_file(path) {
                            Ok(bytes) => match crate::elf::parse(&bytes) {
                                Ok(elf_image) => match crate::process::exec_elf_with_args(active, &bytes, &elf_image, argv, envp) {
                                    Ok((new_entry, new_rsp)) => {
                                        let _ = writeln!(
                                            serial(),
                                            "milestone 58: syscall EXECARGV (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- REAL teardown-and-rebuild from '{path}' WITH real argv/envp, new entry={:#x}, new rsp={:#x}, never returning to the old one",
                                            regs.cs,
                                            new_entry,
                                            new_rsp
                                        );
                                        exec_replace_and_enter_with_rsp(new_entry, new_rsp);
                                    }
                                    Err(e) => {
                                        let _ = writeln!(serial(), "milestone 58: syscall EXECARGV (process {active}) -- FAILED, {e} -- returning u64::MAX, old program continues running");
                                        // MILESTONE 59: see
                                        // exec_err_errno()'s own doc
                                        // comment for the real reasoning
                                        // behind this mapping.
                                        crate::process::set_errno(active, exec_err_errno(e));
                                        regs.rax = u64::MAX;
                                    }
                                },
                                Err(e) => {
                                    let _ = writeln!(serial(), "milestone 58: syscall EXECARGV (process {active}) -- FAILED, '{path}' is not a valid ELF64 image ({e}) -- returning u64::MAX, old program continues running");
                                    // MILESTONE 59: real ENOEXEC -- same
                                    // reasoning as syscall 9 (EXEC)'s own
                                    // elf::parse() failure arm.
                                    crate::process::set_errno(active, errno::ENOEXEC);
                                    regs.rax = u64::MAX;
                                }
                            },
                            Err(e) => {
                                let _ = writeln!(serial(), "milestone 58: syscall EXECARGV (process {active}) -- FAILED, could not read '{path}' off the real on-disk filesystem: {e} -- returning u64::MAX, old program continues running");
                                // MILESTONE 59: real ENOENT -- same
                                // reasoning as syscall 9 (EXEC)'s own
                                // read_file() failure arm.
                                crate::process::set_errno(active, errno::ENOENT);
                                regs.rax = u64::MAX;
                            }
                        },
                        Err(_) => {
                            let _ = writeln!(serial(), "milestone 58: syscall EXECARGV (process {active}) -- path is not valid UTF-8 -- returning u64::MAX");
                            // MILESTONE 59: real EINVAL -- malformed
                            // argument.
                            crate::process::set_errno(active, errno::EINVAL);
                            regs.rax = u64::MAX;
                        }
                    },
                    (Err(e), _) | (_, Err(e)) => {
                        let _ = writeln!(serial(), "milestone 58: syscall EXECARGV (process {active}) -- FAILED reading argv/envp from the calling process's own memory: {e} -- returning u64::MAX");
                        // MILESTONE 59: real E2BIG -- this specific `Err`
                        // (from `read_arg_array`'s own closure above) is
                        // ALWAYS a per-string length exceeding
                        // EXEC_ARG_MAX_LEN, the same real cause
                        // errno.rs's own E2BIG doc comment already names
                        // for this syscall.
                        crate::process::set_errno(active, errno::E2BIG);
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        17 => {
            // MILESTONE 59: geterrno() -- takes no arguments. Returns
            // this process's own real, sticky `Process::errno` field
            // (see that field's own doc comment for the exact "process-
            // local, never cleared on success" semantics) -- u64::MAX if
            // there is no active process to query (the legacy
            // ACTIVE_PROCESS==0 excursion has no Process struct at all,
            // so there is genuinely no errno to read back, the same
            // honest "nothing meaningful to report" reasoning every
            // other no-active-process arm above already uses, just with
            // u64::MAX instead of 0 since a real errno value IS being
            // returned here, not a yes/no result).
            if active == 0 {
                let _ = writeln!(
                    serial(),
                    "milestone 59: syscall GETERRNO called with no active process (plain usertest excursion has no errno field) -- ignoring, returning u64::MAX"
                );
                regs.rax = u64::MAX;
            } else {
                let value = crate::process::get_errno(active);
                let _ = writeln!(
                    serial(),
                    "milestone 59: syscall GETERRNO (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- errno={value} ({})",
                    regs.cs,
                    errno::name(value)
                );
                regs.rax = value;
            }
        }
        18 => {
            // MILESTONE 60: sigaction(signum, handler) -- rdi=signum,
            // rsi=handler (a real user-space address; 0 clears back to
            // SIG_DFL). Returns 1 on success, 0 on failure, same
            // convention KILL/SETPGID already established. See
            // process::sigaction()'s own doc comment for the real
            // SIGKILL-refusal rule this enforces.
            if active == 0 {
                let _ = writeln!(serial(), "milestone 60: syscall SIGACTION called with no active process -- ignoring, returning 0");
                regs.rax = 0;
            } else {
                let signum_arg = regs.rdi;
                let handler_arg = regs.rsi;
                if signum_arg > u8::MAX as u64 {
                    let _ = writeln!(serial(), "milestone 60: syscall SIGACTION (process {active}) -- signum argument {signum_arg} out of range -- returning 0");
                    crate::process::set_errno(active, errno::EINVAL);
                    regs.rax = 0;
                } else {
                    match crate::process::sigaction(active, signum_arg as u8, handler_arg) {
                        Ok(()) => {
                            let _ = writeln!(
                                serial(),
                                "milestone 60: syscall SIGACTION (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- signum {signum_arg} ({}), handler={:#x}",
                                regs.cs,
                                signal::name(signum_arg as u8),
                                handler_arg
                            );
                            regs.rax = 1;
                        }
                        Err(e) => {
                            let _ = writeln!(serial(), "milestone 60: syscall SIGACTION (process {active}) -- REFUSED, {e} -- returning 0");
                            // MILESTONE 60: real EINVAL for both of
                            // sigaction()'s own reject reasons (signum
                            // out of range, or SIGKILL) -- "no such
                            // process" is defensively unreachable here
                            // (active != 0 was already checked above),
                            // same discipline errno.rs's own ESRCH doc
                            // comment already applies elsewhere.
                            crate::process::set_errno(active, errno::EINVAL);
                            regs.rax = 0;
                        }
                    }
                }
            }
        }
        19 => {
            // MILESTONE 60: sigsend(target_pid, signum) -- real signal
            // raising -- rdi=target pid, rsi=signum. Returns 1 on
            // success, 0 on failure, same convention as KILL/SETPGID/
            // SIGACTION above. See process::raise_signal()'s own doc
            // comment for the real authorization rule (self or a live
            // child), the SIGKILL special-case routing to the pre-
            // existing kill() path, and this milestone's own disclosed
            // "no default-disposition table" scope cut.
            if active == 0 {
                let _ = writeln!(serial(), "milestone 60: syscall SIGSEND called with no active process -- ignoring, returning 0");
                regs.rax = 0;
            } else {
                let target_pid_arg = regs.rdi;
                let signum_arg = regs.rsi;
                if target_pid_arg > u8::MAX as u64 || signum_arg > u8::MAX as u64 {
                    let _ = writeln!(serial(), "milestone 60: syscall SIGSEND (process {active}) -- argument out of range -- returning 0");
                    crate::process::set_errno(active, errno::EINVAL);
                    regs.rax = 0;
                } else {
                    match crate::process::raise_signal(active, target_pid_arg as u8, signum_arg as u8) {
                        Ok(()) => {
                            let _ = writeln!(
                                serial(),
                                "milestone 60: syscall SIGSEND (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- target pid {target_pid_arg}, signum {signum_arg} ({}) raised",
                                regs.cs,
                                signal::name(signum_arg as u8)
                            );
                            regs.rax = 1;
                        }
                        Err(e) => {
                            let _ = writeln!(serial(), "milestone 60: syscall SIGSEND (process {active}) -- REFUSED, {e} -- returning 0");
                            // MILESTONE 60: real errno picked by cause,
                            // checked against raise_signal()'s own actual
                            // Err messages -- "signum out of range" is a
                            // real EINVAL; "not authorized" mirrors
                            // SETPGID's own EPERM usage for the identical
                            // real authorization rule; "SIGKILL delivery
                            // failed"/"no such process" both mean no live
                            // target, ESRCH, same bucket KILL/GETPGID
                            // already use. "no handler registered" has no
                            // precise real POSIX equivalent (a real
                            // kill() always succeeds -- some default
                            // disposition always exists there, which this
                            // kernel doesn't implement yet, disclosed on
                            // raise_signal()'s own doc comment) -- EINVAL
                            // is the closest available real code, an
                            // honestly imperfect fit, same disclosed-
                            // imprecision spirit as errno.rs's own EMFILE
                            // doc comment.
                            let code = if e.contains("signum out of range") {
                                errno::EINVAL
                            } else if e.contains("not authorized") {
                                errno::EPERM
                            } else if e.contains("SIGKILL delivery failed") || e.contains("no such process") {
                                errno::ESRCH
                            } else {
                                errno::EINVAL
                            };
                            crate::process::set_errno(active, code);
                            regs.rax = 0;
                        }
                    }
                }
            }
        }
        20 => {
            // MILESTONE 60: sigreturn() -- takes no arguments. Real
            // unwind-back-out-of-a-handler mechanism: restores the FULL
            // hardware-interrupted context (see process::
            // SavedSignalContext's own doc comment) `usertest.rs` itself
            // stashed the moment it redirected execution into the
            // handler this call is unwinding out of, resuming exactly
            // where the signal preempted it -- SysV argument/return
            // registers included, not just rip/rflags/rsp.
            if active == 0 {
                let _ = writeln!(serial(), "milestone 60: syscall SIGRETURN called with no active process -- ignoring");
            } else {
                match crate::process::take_saved_signal_context(active) {
                    Some(ctx) => {
                        regs.r15 = ctx.r15;
                        regs.r14 = ctx.r14;
                        regs.r13 = ctx.r13;
                        regs.r12 = ctx.r12;
                        regs.r11 = ctx.r11;
                        regs.r10 = ctx.r10;
                        regs.r9 = ctx.r9;
                        regs.r8 = ctx.r8;
                        regs.rbp = ctx.rbp;
                        regs.rdi = ctx.rdi;
                        regs.rsi = ctx.rsi;
                        regs.rdx = ctx.rdx;
                        regs.rcx = ctx.rcx;
                        regs.rbx = ctx.rbx;
                        regs.rax = ctx.rax;
                        regs.rip = ctx.rip;
                        regs.rflags = ctx.rflags;
                        regs.rsp = ctx.rsp;
                        let _ = writeln!(
                            serial(),
                            "milestone 60: syscall SIGRETURN (process {active}) -- real context restored, resuming at rip={:#x} rsp={:#x}",
                            ctx.rip,
                            ctx.rsp
                        );
                    }
                    None => {
                        // MILESTONE 60: a stray SIGRETURN with no matching
                        // dispatch in flight -- real POSIX territory this
                        // kernel just refuses honestly (see
                        // take_saved_signal_context()'s own doc comment)
                        // rather than restoring garbage: `regs` is left
                        // completely untouched, so this int 0x80 simply
                        // "returns" normally to whatever instruction
                        // follows it in the CALLER's own code, same as any
                        // other syscall that declines to act.
                        let _ = writeln!(serial(), "milestone 60: syscall SIGRETURN (process {active}) -- REFUSED, no handler dispatch in flight for this process -- context left untouched");
                        crate::process::set_errno(active, errno::EINVAL);
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        21 => {
            // MILESTONE 64: mmap(fd) -> vaddr (u64::MAX on failure). rdi
            // = fd. Real, bounded first slice -- see process.rs's own
            // Milestone 64 module-level doc comment for the full scoping
            // reasoning. Maps `fd`'s ALREADY-BUFFERED content (open()
            // already read the whole file at open() time) read-only into
            // a fresh slot of this process's own mmap table -- no
            // physical frame is actually mapped until a real hardware
            // page fault demand-pages it (process::try_demand_page_mmap(),
            // wired into interrupts.rs's page_fault_handler()).
            let fd = regs.rdi;
            if active == 0 {
                let _ = writeln!(
                    serial(),
                    "milestone 64: syscall MMAP called with no active process (plain usertest excursion has no fd table or mmap table) -- ignoring, returning u64::MAX"
                );
                regs.rax = u64::MAX;
            } else {
                match crate::process::mmap_file(active, fd) {
                    Some(addr) => {
                        let _ = writeln!(
                            serial(),
                            "milestone 64: syscall MMAP (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- fd={fd} -> {addr:#x}",
                            regs.cs
                        );
                        regs.rax = addr;
                    }
                    None => {
                        let _ = writeln!(
                            serial(),
                            "milestone 64: syscall MMAP (process {active}) -- FAILED for fd={fd} (fd not open, not a real file, oversized content, or this process's mmap table is full) -- returning u64::MAX"
                        );
                        // MILESTONE 64: real ENOMEM -- best-fit real
                        // errno for "no room for this mapping" (this
                        // slice's None doesn't separately distinguish
                        // "bad fd" from "table full", so the single
                        // best-fit code is chosen honestly, same
                        // reasoning as EMFILE's own doc comment in
                        // errno.rs for a similarly-collapsed cause set).
                        crate::process::set_errno(active, errno::ENOMEM);
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        22 => {
            // MILESTONE 64: munmap(addr) -> status (0=success,
            // u64::MAX=failure). rdi = addr. `addr` must be exactly a
            // live slot's own base address -- see
            // process::munmap_region()'s own doc comment for the full
            // "why exact-match is the honest, complete check here"
            // reasoning.
            let addr = regs.rdi;
            if active == 0 {
                let _ = writeln!(
                    serial(),
                    "milestone 64: syscall MUNMAP called with no active process (plain usertest excursion has no mmap table) -- ignoring, returning u64::MAX"
                );
                regs.rax = u64::MAX;
            } else if crate::process::munmap_region(active, addr) {
                let _ = writeln!(
                    serial(),
                    "milestone 64: syscall MUNMAP (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- addr={addr:#x} unmapped successfully",
                    regs.cs
                );
                regs.rax = 0;
            } else {
                let _ = writeln!(
                    serial(),
                    "milestone 64: syscall MUNMAP (process {active}) -- FAILED, addr={addr:#x} does not name any of this process's own live mmap slots -- returning u64::MAX"
                );
                crate::process::set_errno(active, errno::EINVAL);
                regs.rax = u64::MAX;
            }
        }
        23 => {
            // MILESTONE 65: mmap_writable(fd) -> vaddr (u64::MAX on
            // failure). rdi = fd. Real `PROT_WRITE` first slice on top of
            // Milestone 64's own read-only `mmap()` -- see process.rs's
            // own Milestone 65 doc comments on `MmapSlot`/
            // `mmap_file_writable()`/`try_demand_page_mmap()` for the full
            // scoping reasoning. A DISTINCT syscall number from 21
            // (deliberately, not a new argument bolted onto mmap(fd)'s
            // existing rdi-only calling convention) so every one of
            // Milestone 64's own four hand-assembled test programs stays
            // byte-for-byte unmodified and their own self-test keeps
            // proving exactly what it always did, with zero risk of an
            // accidentally-nonzero leftover register in one of them being
            // misread as a new argument.
            let fd = regs.rdi;
            if active == 0 {
                let _ = writeln!(
                    serial(),
                    "milestone 65: syscall MMAP_WRITABLE called with no active process (plain usertest excursion has no fd table or mmap table) -- ignoring, returning u64::MAX"
                );
                regs.rax = u64::MAX;
            } else {
                match crate::process::mmap_file_writable(active, fd) {
                    Some(addr) => {
                        let _ = writeln!(
                            serial(),
                            "milestone 65: syscall MMAP_WRITABLE (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- fd={fd} -> {addr:#x}",
                            regs.cs
                        );
                        regs.rax = addr;
                    }
                    None => {
                        let _ = writeln!(
                            serial(),
                            "milestone 65: syscall MMAP_WRITABLE (process {active}) -- FAILED for fd={fd} (fd not open, not a real file, oversized content, or this process's mmap table is full) -- returning u64::MAX"
                        );
                        // MILESTONE 65: same real, best-fit ENOMEM
                        // reasoning as syscall 21's own MMAP arm.
                        crate::process::set_errno(active, errno::ENOMEM);
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        24 => {
            // MILESTONE 82: open_trunc(path_ptr, path_len) -> fd (u64::MAX
            // on failure). rdi = path_ptr, rsi = path_len -- byte-for-byte
            // syscall 3's own argument handling; the ONLY difference is
            // `open_file()` is called with `truncate = true`, so the
            // in-memory buffer starts genuinely empty (the path's existing
            // on-disk bytes are never read) and is marked dirty
            // immediately -- real O_TRUNC semantics on close() even when
            // zero writes are ever issued.
            //
            // A DISTINCT syscall number from 3 (deliberately -- the exact
            // reasoning syscall 23 already documents for staying separate
            // from syscall 21): syscall 3 stays byte-for-byte as Milestone
            // 35 shipped it, so every existing OPEN caller is completely
            // unaffected -- including this kernel's own six hand-assembled
            // OPEN callers (the Milestone 64/65 MMAP test programs in
            // process.rs), none of which zero rdx before `int 0x80`. An
            // earlier draft of this milestone overloaded syscall 3's rdx
            // as a truncate flag and silently broke exactly those six
            // programs' self-tests (they mmap a file they only meant to
            // read); a separate number has no such failure mode. Only
            // libc's own `fopen(path, "w")` (all four libc.rs copies)
            // routes here.
            let path_ptr = regs.rdi;
            let requested_len = regs.rsi;
            let truncated = requested_len > MAX_PATH_LEN;
            let len = if truncated { MAX_PATH_LEN } else { requested_len } as usize;
            if truncated {
                let _ = writeln!(
                    serial(),
                    "milestone 82: syscall OPEN_TRUNC -- requested path_len {requested_len} exceeds MAX_PATH_LEN {MAX_PATH_LEN}, truncating"
                );
            }
            let mut path_bytes = alloc::vec::Vec::with_capacity(len);
            for i in 0..len {
                path_bytes.push(unsafe { core::ptr::read((path_ptr as *const u8).wrapping_add(i)) });
            }
            match (active, core::str::from_utf8(&path_bytes)) {
                (0, _) => {
                    let _ = writeln!(
                        serial(),
                        "milestone 82: syscall OPEN_TRUNC called with no active process (plain usertest excursion has no fd table) -- ignoring, returning u64::MAX"
                    );
                    regs.rax = u64::MAX;
                }
                (_, Err(_)) => {
                    let _ = writeln!(serial(), "milestone 82: syscall OPEN_TRUNC (process {active}) -- path is not valid UTF-8, returning u64::MAX");
                    crate::process::set_errno(active, errno::EINVAL);
                    regs.rax = u64::MAX;
                }
                (_, Ok(path)) => match crate::process::open_file(active, path, true) {
                    Some(fd) => {
                        let _ = writeln!(
                            serial(),
                            "milestone 82: syscall OPEN_TRUNC (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- path='{path}' -> fd {fd} (truncated)",
                            regs.cs
                        );
                        regs.rax = fd;
                    }
                    None => {
                        let _ = writeln!(
                            serial(),
                            "milestone 82: syscall OPEN_TRUNC (process {active}) -- FAILED for path='{path}' (fd table full -- max {} open files per process) -- returning u64::MAX",
                            crate::process::MAX_OPEN_FILES
                        );
                        crate::process::set_errno(active, errno::EMFILE);
                        regs.rax = u64::MAX;
                    }
                },
            }
        }
        25 => {
            // MILESTONE 87: sbrk_reset() -- rewind the calling process's
            // heap break all the way to HEAP_START in one call (logically
            // free the whole heap; already-mapped frames stay mapped and
            // get reused). No arguments. Returns the new break in rax, or
            // u64::MAX with no active process. Used by the tools/cc_src
            // compiler self-test between compilations -- see
            // process::sbrk_reset()'s own doc comment.
            if active == 0 {
                let _ = writeln!(
                    serial(),
                    "milestone 87: syscall SBRK_RESET called with no active process -- ignoring, returning u64::MAX"
                );
                regs.rax = u64::MAX;
            } else {
                match crate::process::sbrk_reset(active) {
                    Some(brk) => {
                        let _ = writeln!(
                            serial(),
                            "milestone 87: syscall SBRK_RESET (process {active}) -- hardware-recorded CS={:#x} (CPL={hardware_cpl}) -- heap break rewound to {:#x}",
                            regs.cs, brk
                        );
                        regs.rax = brk;
                    }
                    None => {
                        regs.rax = u64::MAX;
                    }
                }
            }
        }
        other => {
            let _ = writeln!(
                serial(),
                "milestone 27: unknown syscall {other} from CPL={hardware_cpl} -- ignoring"
            );
        }
    }

    // MILESTONE 60: real signal delivery -- checked ONCE, here, at the
    // tail of EVERY syscall's return-to-userspace (the one real kernel/
    // user boundary this synchronous, one-ring-3-excursion-at-a-time
    // kernel actually has -- see process::take_deliverable_signal()'s own
    // doc comment for why this is the correct, complete delivery point
    // and not a partial stand-in). Naturally skipped for syscall 1
    // (exit): that arm diverges straight into resume_kernel() and never
    // reaches this point at all. Skipped when `active == 0` for the same
    // reason every other process-scoped syscall above already skips it:
    // the legacy bare `usertest` excursion has no Process struct to hold
    // signal state in.
    if active != 0 {
        if let Some((signum, handler_addr)) = crate::process::take_deliverable_signal(active) {
            // Stash the COMPLETE interrupted context before touching
            // anything -- SIGRETURN (syscall 20, above) hands this back
            // verbatim later.
            let saved = crate::process::SavedSignalContext {
                r15: regs.r15,
                r14: regs.r14,
                r13: regs.r13,
                r12: regs.r12,
                r11: regs.r11,
                r10: regs.r10,
                r9: regs.r9,
                r8: regs.r8,
                rbp: regs.rbp,
                rdi: regs.rdi,
                rsi: regs.rsi,
                rdx: regs.rdx,
                rcx: regs.rcx,
                rbx: regs.rbx,
                rax: regs.rax,
                rip: regs.rip,
                rflags: regs.rflags,
                rsp: regs.rsp,
            };
            crate::process::stash_signal_context(active, saved);

            // Real trampoline injection: builds a small, genuine "fake
            // call frame" directly on the SAME process's own already-
            // mapped user stack, through whatever CR3 is CURRENTLY
            // loaded -- this kernel is already, at this exact point,
            // running under the interrupted process's own page tables
            // (no CR3 switch happens for an ordinary syscall return), so
            // this is the SAME direct-user-pointer-write technique the
            // `read`/`fdwrite` syscall arms above already use for a
            // caller-supplied buffer, just writing fixed bytes instead of
            // syscall-supplied ones. `SIGNAL_FRAME_GAP` clears the real
            // 128-byte SysV red zone a leaf function may have been using
            // below its own rsp at the moment it was interrupted, plus
            // real headroom; 16-aligning `frame_top` and placing the
            // pushed return-address VALUE 8 bytes below it gives the
            // handler an entry `rsp` that is `frame_top - 8` -- exactly
            // `rsp mod 16 == 8`, the real SysV ABI invariant a function
            // expects to see immediately after being `call`ed.
            let old_rsp = regs.rsp;
            let frame_top = (old_rsp - SIGNAL_FRAME_GAP) & !0xF;
            // mov eax, 20 (sigreturn); int 0x80 -- the real trampoline a
            // handler's own `ret` lands on once it's done.
            let trampoline: [u8; 7] = [0xB8, 20, 0x00, 0x00, 0x00, 0xCD, 0x80];
            for (i, b) in trampoline.iter().enumerate() {
                unsafe { core::ptr::write((frame_top as *mut u8).wrapping_add(i), *b) };
            }
            let return_addr_slot = frame_top - 8;
            unsafe { core::ptr::write(return_addr_slot as *mut u64, frame_top) };

            let _ = writeln!(
                serial(),
                "milestone 60: real signal delivery -- process {active} redirected into handler {:#x} for signum {signum} ({}), interrupted rip={:#x} saved, trampoline frame at {:#x}",
                handler_addr,
                signal::name(signum),
                regs.rip,
                frame_top
            );

            regs.rip = handler_addr;
            regs.rdi = signum as u64;
            regs.rsp = return_addr_slot;
        }
    }
}

/// MILESTONE 27: switches from the current (kernel) context into ring 3.
/// Mirrors tasks.rs's switch_to's own opening half exactly -- saves the
/// callee-saved registers a normal `extern "C"` caller (run(), below)
/// would expect preserved across a call, and the resulting rsp, into
/// `*kernel_rsp_slot` -- then, instead of switching to another kernel
/// stack like switch_to does, builds a genuine iretq frame by hand on
/// the CURRENT stack and executes it. Args arrive per the SysV ABI (6
/// integer args: rdi, rsi, rdx, rcx, r8, r9), matching the parameter
/// list below 1:1 -- checked against the actual asm, not assumed.
#[unsafe(naked)]
unsafe extern "C" fn enter_ring3(
    _kernel_rsp_slot: *mut u64,
    _user_rsp: u64,
    _user_rip: u64,
    _user_cs: u64,
    _user_ss: u64,
    _rflags: u64,
) {
    core::arch::naked_asm!(
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "push r8",  // SS
        "push rsi", // RSP (user stack top)
        "push r9",  // RFLAGS
        "push rcx", // CS
        "push rdx", // RIP
        "iretq",
    );
}

/// MILESTONE 27: the exit syscall's actual mechanism -- switches rsp to
/// the kernel-side stack enter_ring3() saved, pops back the same 6
/// callee-saved registers it pushed there, then `ret`s using the return
/// address still sitting on that stack from enter_ring3()'s own call
/// site: control resumes in run(), immediately after its
/// `enter_ring3(...)` call, as if that call had simply returned
/// normally.
///
/// DIAGNOSED AND FIXED: the first real test of this (see the milestone
/// report) copied tasks.rs's switch_to pattern exactly, including its
/// unconditional `sti` before `ret` -- and hung the shell completely
/// after exactly one `usertest` run, identically every time. Root cause:
/// switch_to's `sti` is safe ONLY because every one of its call sites is
/// a single, well-understood nesting level (directly inside
/// timer_interrupt_handler, or ordinary kernel_main code). run() here is
/// nested arbitrarily deep inside the KEYBOARD interrupt handler's own
/// call chain instead (on_interrupt -> shell::on_char -> run_command ->
/// usertest::run), which itself is holding keyboard.rs's KEYBOARD mutex
/// guard for that entire chain's duration. An `sti` here re-enables
/// interrupts too early -- while still nested that deep, on whatever
/// stack was current -- opening a real window for a nested timer tick to
/// call tasks::timer_tick_switch() -> switch_to(), which saves the
/// CURRENT (mid-excursion, nested) rsp into some task's own context and
/// jumps away to a completely different task's stack, permanently
/// abandoning this whole call chain -- KEYBOARD mutex guard included,
/// never dropped -- and silently deadlocking every keystroke from then
/// on (confirmed via a real two-`usertest`-run serial+screenshot test:
/// the first run completed and even printed the next shell prompt, but
/// the second run's keystrokes were never even echoed to the console).
/// Removing `sti` fixes this: this `ret` always lands back inside code
/// nested inside the keyboard ISR, which does its OWN correct `iretq`
/// once it finally unwinds, naturally restoring the original RFLAGS
/// (IF=1) from before the keyboard interrupt fired -- exactly like every
/// other shell command already relies on, with no explicit re-enable
/// needed here at all.
#[unsafe(naked)]
unsafe extern "C" fn resume_kernel(_saved_rsp: u64) -> ! {
    core::arch::naked_asm!(
        "mov rsp, rdi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "ret",
    );
}

/// MILESTONE 41: SIGSEGV-equivalent -- called from interrupts.rs's
/// page_fault_handler when a real page fault occurs with a
/// hardware-recorded CPL=3 (genuinely inside a process.rs-owned ring-3
/// excursion, not a kernel bug). Does EXACTLY what the exit syscall
/// (rax=1) arm above does when `active != 0` -- restores the kernel's
/// own CR3, clears ACTIVE_PROCESS, and unwinds back to whichever kernel
/// resume point (KERNEL_RSP, or CHILD_KERNEL_RSP if nested inside a
/// forked-child excursion) is currently correct. The ONLY difference
/// from the exit syscall's own path is what triggers it: a hardware
/// fault instead of the process's own voluntary `int 0x80`.
/// resume_kernel() itself doesn't care which one drove it here -- the
/// CPU-pushed page-fault interrupt frame is simply discarded, never
/// `iretq`'d back into.
pub(crate) fn terminate_faulted_process_and_resume_kernel() -> ! {
    crate::process::restore_kernel_cr3();
    crate::process::ACTIVE_PROCESS.store(0, Ordering::SeqCst);
    let in_child = IN_CHILD_RESUME.load(Ordering::SeqCst);
    // MILESTONE 53: record that THIS excursion's child was signal-
    // terminated, not exited -- mirrors the exit syscall arm's own
    // `if IN_CHILD_RESUME { LAST_CHILD_EXIT_CODE.store(...) }` gate
    // immediately below, same reasoning: only meaningful (and only set)
    // when a forked child, not a top-level process, is what just faulted.
    if in_child {
        CHILD_FAULTED.store(true, Ordering::SeqCst);
    }
    let saved = if in_child {
        CHILD_KERNEL_RSP.load(Ordering::SeqCst)
    } else {
        KERNEL_RSP.load(Ordering::SeqCst)
    };
    unsafe { resume_kernel(saved) };
}

/// MILESTONE 27: the `usertest` shell command's entry point -- enters
/// ring 3 at the mapped user code page, lets the tiny hardcoded program
/// there call the print syscall then the exit syscall, and returns once
/// resume_kernel() has switched back. Safe to call repeatedly: setup()
/// already guarantees the pages are mapped exactly once, and every run
/// starts the user program from the same fixed RIP with a fresh top-of-
/// stack RSP, so there's no accumulated state between calls to leak or
/// corrupt.
pub fn run() -> Result<(), &'static str> {
    if !MAPPED.load(Ordering::SeqCst) {
        return Err("user test pages not mapped (setup() should have run at boot)");
    }

    let run_id = RUN_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = writeln!(
        serial(),
        "milestone 27: usertest run #{run_id} -- entering ring 3 at {:#x}",
        USER_CODE_ADDR
    );

    let user_cs = gdt::user_code_selector().0 as u64;
    let user_ss = gdt::user_data_selector().0 as u64;
    let user_stack_top = USER_STACK_ADDR + USER_STACK_SIZE;
    // bit 1 is reserved and must be 1; bit 9 (IF) set so ring-3 code
    // runs with interrupts enabled, per the milestone's own requirement
    // -- NOT a hardening measure against the timer/keyboard ISRs firing
    // mid-excursion, which is a real, disclosed, low-probability
    // limitation (see the milestone report).
    let rflags: u64 = 0x202;

    unsafe {
        enter_ring3(KERNEL_RSP.as_ptr(), user_stack_top, USER_CODE_ADDR, user_cs, user_ss, rflags);
    }

    let _ = writeln!(
        serial(),
        "milestone 27: usertest run #{run_id} -- resumed in kernel context after ring-3 exit syscall"
    );
    Ok(())
}

/// MILESTONE 30: the same enter_ring3 mechanism run() above uses,
/// factored out so process::run() can drive it too -- process.rs is
/// responsible for the CR3 switch (both directions) around this call;
/// this function only builds the iretq frame and executes it, identical
/// to what run() already did for the single-process Milestone 27 case.
/// Reuses the same KERNEL_RSP slot as run() -- safe because the shell
/// only ever runs one command (and therefore at most one ring-3
/// excursion, `usertest` or `runproc`) at a time.
///
/// MILESTONE 44: takes the real entry address as a parameter now,
/// instead of hardcoding `USER_CODE_ADDR` -- the underlying `enter_ring3`
/// naked function was ALREADY fully parameterized by entry RIP (it's
/// just its 3rd SysV argument, `rdx`); only this thin wrapper needed to
/// stop hardcoding it. This is NOT the "deep surgery on the ring-3
/// entry trampoline" Milestone 36 deliberately deferred -- the naked
/// asm itself is untouched, byte-for-byte. Every existing caller that
/// only ever ran flat-binary/hand-assembled programs (`process::run()`
/// for PROCESS_A/B/FDTEST/FORK_TEST/SIGSEGV_TEST/SIGKILL_TEST,
/// `run_loaded_process()` for the `runfile` path) now passes their own
/// process's real `entry` field, which is `USER_CODE_ADDR` for every
/// one of them -- zero behavior change. Only `load_and_run_elf()` now
/// passes a genuinely different value, the ELF's own real `e_entry`.
pub(crate) fn enter_ring3_now(entry: u64) {
    let user_cs = gdt::user_code_selector().0 as u64;
    let user_ss = gdt::user_data_selector().0 as u64;
    let user_stack_top = USER_STACK_ADDR + USER_STACK_SIZE;
    let rflags: u64 = 0x202;

    unsafe {
        enter_ring3(KERNEL_RSP.as_ptr(), user_stack_top, entry, user_cs, user_ss, rflags);
    }
}

/// MILESTONE 37: mirrors enter_ring3()'s exact push order/argument
/// mapping (see that function's own doc comment for the byte-for-byte
/// reasoning) with exactly one addition: `xor eax, eax` right before
/// `iretq`, forcing rax=0 on the way into ring 3 -- fork()'s own "the
/// child sees a return value of 0" contract, applied at the ONE moment
/// it actually matters (the child's very first, and only, resumption).
/// Every other GPR is left as whatever was last in these physical
/// registers (the SAME "fresh entry, no GPR restore" behavior
/// enter_ring3() already has) -- a real, honest, deliberate
/// simplification: FORK_TEST_PROGRAM (this milestone's own hand-
/// assembled test) is written to only ever depend on rax immediately
/// after its own fork() call (`mov rbx, rax; cmp rbx, 0; je
/// child_path`), so a full 15-register save/restore (which the parent's
/// own SyscallRegs snapshot technically has, but this function does not
/// thread through) is real, disclosed, unneeded surgery for what this
/// milestone's actual test program requires -- a general "resume this
/// process exactly as if fork() had simply returned" would need it, a
/// hand-assembled program under this project's own full control does
/// not.
#[unsafe(naked)]
unsafe extern "C" fn enter_ring3_as_child(
    _kernel_rsp_slot: *mut u64,
    _user_rsp: u64,
    _user_rip: u64,
    _user_cs: u64,
    _user_ss: u64,
    _rflags: u64,
) {
    core::arch::naked_asm!(
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "push r8",  // SS
        "push rsi", // RSP (child's resume_rsp)
        "push r9",  // RFLAGS
        "push rcx", // CS
        "push rdx", // RIP (child's resume_rip)
        "xor eax, eax",
        "iretq",
    );
}

/// MILESTONE 37: process::run_forked_child()'s own entry point into
/// this mechanism -- switches CR3 has ALREADY happened by the time this
/// is called (process.rs's own responsibility, same division of labor
/// as enter_ring3_now()'s own doc comment describes for the top-level
/// case). Sets IN_CHILD_RESUME for the ENTIRE duration the child is
/// running (including its own eventual exit() syscall's dispatch, which
/// reads this flag to pick CHILD_KERNEL_RSP over KERNEL_RSP -- see that
/// arm's own comment), and saves/restores through CHILD_KERNEL_RSP, NOT
/// KERNEL_RSP, so this nested excursion cannot clobber the OUTER
/// (top-level, parent's own) excursion's own already-saved resume
/// point.
pub(crate) fn enter_ring3_as_forked_child(resume_rip: u64, resume_rsp: u64) {
    let user_cs = gdt::user_code_selector().0 as u64;
    let user_ss = gdt::user_data_selector().0 as u64;
    let rflags: u64 = 0x202;

    IN_CHILD_RESUME.store(true, Ordering::SeqCst);
    unsafe {
        enter_ring3_as_child(CHILD_KERNEL_RSP.as_ptr(), resume_rsp, resume_rip, user_cs, user_ss, rflags);
    }
    // Reached only once the child's own exit() syscall has run
    // (resume_kernel(), driven by the exit arm's IN_CHILD_RESUME check
    // above, pops back to exactly the stack position saved into
    // CHILD_KERNEL_RSP by the naked push sequence above -- i.e. HERE).
    IN_CHILD_RESUME.store(false, Ordering::SeqCst);
}

/// MILESTONE 37: builds an iretq frame from `user_rsp`/`user_rip`/
/// `user_cs`/`user_ss`/`rflags` and executes it directly -- deliberately
/// takes NO kernel_rsp_slot and never returns (`-> !`), the same "never
/// return, iretq directly" shape resume_kernel() already established
/// for the exit syscall's "jump back to kernel" case, applied here to
/// "jump into a NEWLY exec()'d program's entry" instead. This does NOT
/// establish a new resume anchor because it doesn't need one: MILESTONE
/// 45's real exec() replaces the CURRENTLY-running process's entire
/// address space (new PML4, new physical frames, CR3 already switched
/// by process::exec_elf() before this function is even called) without
/// changing which EXCURSION (top-level or nested-child) is in flight --
/// whichever anchor (KERNEL_RSP or CHILD_KERNEL_RSP) was already
/// established when THIS process was originally entered is still the
/// correct one for its EVENTUAL exit() (whenever the newly-exec()'d
/// code calls it) to resume through, completely unaffected by exec()
/// itself -- that invariant held even under Milestone 37's old in-place
/// code-frame-overwrite implementation, and still holds now that exec()
/// genuinely tears down and rebuilds the address space instead: either
/// way, it's the SAME process (same pid, same table slot), just with a
/// new PML4/CR3 value and a new entry, and CR3/PML4 identity was never
/// what KERNEL_RSP/CHILD_KERNEL_RSP anchor against in the first place.
#[unsafe(naked)]
unsafe extern "C" fn exec_into_ring3(_user_rsp: u64, _user_rip: u64, _user_cs: u64, _user_ss: u64, _rflags: u64) -> ! {
    core::arch::naked_asm!(
        "push rcx", // SS
        "push rdi", // RSP
        "push r8",  // RFLAGS
        "push rdx", // CS
        "push rsi", // RIP
        "iretq",
    );
}

/// MILESTONE 37 (mechanism) / MILESTONE 45 (real caller): the `exec()`
/// syscall's own entry point into exec_into_ring3() -- called from
/// syscall_dispatch's syscall-9 arm AFTER process::exec_elf() has
/// already torn down and rebuilt the process's ENTIRE address space
/// (new PML4, new code/stack/heap frames, CR3 already switched) and
/// returned the new program's REAL, parsed `e_entry` as `user_rip` --
/// no longer hardcoded to USER_CODE_ADDR (that was Milestone 36's own
/// deliberate, now-closed deferral -- Milestone 44 generalized
/// `Process::entry`, and this is the first call site that ever hands
/// this function a genuinely different value at runtime). Always starts
/// the newly-exec()'d program at a FRESH top-of-stack (real exec()
/// semantics: the old stack's contents are gone, replaced by the new
/// program's own private stack frame, exactly like a real process image
/// replacement) rather than preserving whatever rsp value was in use
/// before the exec() call.
pub(crate) fn exec_replace_and_enter(user_rip: u64) -> ! {
    let user_cs = gdt::user_code_selector().0 as u64;
    let user_ss = gdt::user_data_selector().0 as u64;
    let user_stack_top = USER_STACK_ADDR + USER_STACK_SIZE;
    let rflags: u64 = 0x202;
    unsafe {
        exec_into_ring3(user_stack_top, user_rip, user_cs, user_ss, rflags);
    }
}

/// MILESTONE 58: identical to `exec_replace_and_enter()` above (same
/// `exec_into_ring3()` mechanism, same CS/SS/RFLAGS setup), except the
/// caller supplies the exact initial RSP to enter with instead of this
/// always defaulting to the bare `USER_STACK_ADDR + USER_STACK_SIZE`
/// top-of-stack. Real reason this exists as its own function rather
/// than a parameter added to `exec_replace_and_enter()`: the plain
/// path's every call site (syscall 9) has nothing useful to pass but
/// the fresh top-of-stack anyway, so keeping that call site's own
/// signature unchanged is real, zero-risk backward compatibility, not
/// just tidiness. Used by syscall 16 (EXECARGV) with the RSP
/// `process::exec_elf_with_args()` -> `build_argv_envp_stack()` already
/// computed and populated with a real argv/envp block.
pub(crate) fn exec_replace_and_enter_with_rsp(user_rip: u64, user_rsp: u64) -> ! {
    let user_cs = gdt::user_code_selector().0 as u64;
    let user_ss = gdt::user_data_selector().0 as u64;
    let rflags: u64 = 0x202;
    unsafe {
        exec_into_ring3(user_rsp, user_rip, user_cs, user_ss, rflags);
    }
}
