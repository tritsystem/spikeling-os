//! IDT setup: CPU exception handlers first (this file), hardware
//! interrupts (PIC remap + timer) added once exceptions are proven
//! working -- getting exception handling wrong produces a silent
//! triple-fault reboot with zero diagnostic output, so this is verified
//! as its own step before anything depends on it.

use crate::gdt;
use crate::usertest;
use crate::{hlt_loop, serial};
use core::fmt::Write;
use core::sync::atomic::{AtomicU64, Ordering};
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin::Mutex;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};
use x86_64::{PrivilegeLevel, VirtAddr};

pub const SYSCALL_VECTOR: u8 = 0x80;

/// MILESTONE 75: real, disclosed heuristic distinguishing a genuine
/// STACK OVERFLOW from an ordinary wild-pointer SIGSEGV, purely for
/// diagnostic clarity -- the actual termination path
/// (`terminate_faulted_process_and_resume_kernel()`, unchanged) is
/// byte-for-byte identical either way; this constant only changes
/// which log line `page_fault_handler` prints, never what gets caught
/// or how.
///
/// This is the kernel-side half of the one significant OPEN SAFETY
/// ITEM flagged since Milestone 73 and re-flagged, untouched, by
/// Milestone 74's own closing disclosure: this kernel gives every
/// process exactly ONE 4 KiB stack page
/// (`usertest::USER_STACK_ADDR`/`USER_STACK_SIZE`), growing DOWN from
/// `USER_STACK_ADDR + USER_STACK_SIZE`, with no stack-depth guard on
/// self-recursive, forward, or mutually-recursive call chains.
///
/// Checked directly against `process.rs` before writing this comment,
/// not assumed: `create_process_from_image()`/`create_process_from_elf()`
/// map exactly one code page and exactly one stack page per process;
/// `try_demand_page_heap()` only ever maps pages in
/// `[HEAP_START, HEAP_START + HEAP_SIZE)`, ABOVE the stack, never
/// below it; `try_demand_page_mmap()` similarly never lands below the
/// stack. So the entire real, ~256 MiB range between
/// `USER_CODE_ADDR`'s own single page and `USER_STACK_ADDR`
/// (`USER_STACK_ADDR - USER_CODE_ADDR - PAGE_SIZE`, computed directly
/// from those two constants, not a round or invented number) is
/// deliberately never mapped by ANY code path in this kernel -- it
/// already acts as a real (if enormous, and previously UN-exercised)
/// guard region: a stack push/access that runs off the bottom of the
/// single mapped page produces an ordinary NOT-PRESENT `#PF`, which
/// falls through both the heap and mmap demand-paging attempts below
/// (neither's own address range comes anywhere close) straight to the
/// SAME unconditional SIGSEGV-and-terminate path every other invalid
/// ring-3 access has used since Milestone 41 -- this was already true
/// before this milestone, just never verified end-to-end against a
/// program that actually recurses far enough to hit it (see
/// `tools/cc_src/main.rs`'s new CASE 34 for that real, on-disk-ELF +
/// kernel `exec()` + `wait()` verification).
///
/// `STACK_GUARD_REGION_SIZE` (1 MiB) is a deliberately conservative
/// SUBSET of that real ~256 MiB gap, used only to decide whether the
/// fault log line below says "STACK OVERFLOW" instead of the generic
/// SIGSEGV wording. A wild pointer that happens to land in this same
/// 1 MiB band immediately below the stack would be mislabeled by the
/// log line -- a real, disclosed, purely COSMETIC limitation (the
/// termination path is identical either way) -- not a new safety gap.
/// This constant does NOT raise the single-page stack to more than one
/// page, does NOT add a software recursion-depth counter (this kernel
/// has no visibility into arbitrary ring-3 `call`/`ret` depth without
/// per-call instrumentation this codegen doesn't emit), and does NOT
/// change which faults get caught -- it only makes an already-real,
/// already-caught failure mode diagnosable at a glance in the serial
/// log, and gives it a real, direct, end-to-end verification for the
/// first time.
const STACK_GUARD_REGION_SIZE: u64 = 0x10_0000; // 1 MiB

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.general_protection_fault.set_handler_fn(general_protection_fault_handler);
        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
        idt[InterruptIndex::Mouse.as_u8()].set_handler_fn(mouse_interrupt_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        unsafe {
            // MILESTONE 27: usertest::syscall_entry is a naked function,
            // not an `extern "x86-interrupt" fn` -- set_handler_fn's
            // signature requires that ABI, so the raw address goes in
            // via set_handler_addr instead. DPL=3 set explicitly: every
            // gate defaults to DPL=0 (Ring0), which would make a ring-3
            // `int 0x80` immediately #GP-fault (the CPU checks the
            // CALLER's CPL against the gate's DPL for software
            // interrupts) before the handler ever ran.
            idt[SYSCALL_VECTOR]
                .set_handler_addr(VirtAddr::new(usertest::syscall_entry as *const () as u64))
                .set_privilege_level(PrivilegeLevel::Ring3);
        }
        idt
    };
}

// STAGE B: PIC remap + timer interrupt -- the actual preemption clock.
// Hardware IRQs default to vectors 0-15, which collide with CPU
// exceptions (0-31); remapped to 32-47 (PIC_1_OFFSET/PIC_2_OFFSET),
// the standard convention.
pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

/// Real count of timer interrupts actually delivered -- the ground
/// truth Milestone 5 stage B is verified against (a busy hlt loop that
/// merely "returns" doesn't prove the timer fired periodically; this
/// atomic, incremented only inside the real handler, does).
pub static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard = PIC_1_OFFSET + 1,
    Mouse = PIC_1_OFFSET + 12, // IRQ12, on the slave PIC
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

pub fn init_pics() {
    unsafe { PICS.lock().initialize() };
}

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
    unsafe {
        // EOI first: the PIC needs to know this IRQ is acknowledged
        // before we potentially switch stacks underneath it, in case
        // another timer interrupt is already pending.
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }
    // MILESTONE 9: real LIF neuron dynamics ticked on every real
    // hardware timer interrupt, not a simulated/synthetic clock.
    // MILESTONE 21: neurons.rs no longer has its own tick() -- LeftKey/
    // RightKey/Motor are now ordinary named neurons inside this single
    // GenericNetwork (seeded by neurons::init() at boot), so one tick()
    // call now drives the fixed network AND every shell-defined
    // (`addneuron`) network together, on the same real hardware clock.
    crate::network::tick();

    // MILESTONE 5, STAGE C: may switch to a different task's stack and
    // not return here directly -- it returns to whichever context gets
    // switched back into later, at which point iretq resumes THAT
    // context's own interrupted point, not necessarily this one.
    crate::tasks::timer_tick_switch();
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    crate::keyboard::on_interrupt();
    unsafe {
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    crate::mouse::on_interrupt();
    unsafe {
        // notify_end_of_interrupt handles sending EOI to both PICs when
        // the interrupt is on the slave (IRQ >= 8), as this one is --
        // standard PIC cascading, built into the pic8259 crate.
        PICS.lock().notify_end_of_interrupt(InterruptIndex::Mouse.as_u8());
    }
}

pub fn init_idt() {
    IDT.load();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    writeln!(serial(), "EXCEPTION: BREAKPOINT\n{:#?}", stack_frame).unwrap();
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    // deliberately not using the shared serial() helper's own SerialPort
    // instance here -- if the double fault happened WHILE that was
    // mid-write, re-entering it could double-fault again; a fresh
    // handle to the same hardware port is safe either way
    let mut port = serial();
    let _ = writeln!(port, "EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
    hlt_loop();
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;
    // MILESTONE 41: SIGSEGV -- a page fault with a hardware-recorded
    // CPL=3 code segment (the CPU's OWN record of the privilege level
    // that was executing when the fault occurred, not anything
    // process-supplied code could fake) means real userspace code
    // faulted, not a kernel bug. Every prior milestone treated ANY page
    // fault as fatal (hlt_loop() below, unconditionally) -- meaning a
    // single buggy ring-3 program could take down the entire kernel.
    // This is real process isolation for faults, not just for memory:
    // terminate the offending process, keep the kernel running.
    // ACTIVE_PROCESS != 0 is the same "a process.rs-owned process is
    // really the one in ring 3 right now" check syscall_dispatch's own
    // `active` variable already uses.
    let active = crate::process::ACTIVE_PROCESS.load(Ordering::Relaxed);
    if stack_frame.code_segment.rpl() == PrivilegeLevel::Ring3 && active != 0 {
        // MILESTONE 57: real demand paging for the per-process heap --
        // tried BEFORE the unconditional SIGSEGV termination below, for
        // exactly one case: a genuine NOT-PRESENT fault (never a
        // protection violation -- a heap page this kernel has mapped is
        // always PRESENT|WRITABLE, so a protection-violation fault there
        // would mean something is actually wrong, not a legitimate lazy
        // allocation) whose address is inside the active process's own
        // heap AND within bytes it has already committed via sbrk(). See
        // process::try_demand_page_heap()'s own doc comment for the full
        // eligibility check. Every other fault -- an address outside the
        // heap entirely, or inside the heap's reserved-but-uncommitted
        // tail -- returns false here and falls straight through to the
        // SAME termination path every prior milestone already used,
        // completely unmodified.
        let fault_addr = Cr2::read().map(|a| a.as_u64()).unwrap_or(0);
        if !error_code.contains(PageFaultErrorCode::PROTECTION_VIOLATION) && crate::process::try_demand_page_heap(active, fault_addr) {
            // A demand-paged fault-class exception resumes by simply
            // returning: the CPU retries the exact faulting instruction,
            // which now succeeds against the freshly-mapped page. No
            // termination, no resume_kernel() unwind -- ring 3 execution
            // continues exactly where it was.
            return;
        }
        // MILESTONE 64: the mmap twin of the heap demand-paging attempt
        // just above -- tried SECOND (a fault this process's own mmap
        // table doesn't recognize at all just falls through, same as a
        // heap fault outside the heap range falls through the check
        // above), same `!PROTECTION_VIOLATION` guard (a mmap page this
        // kernel has mapped is always PRESENT, never WRITABLE by design
        // -- see process::try_demand_page_mmap()'s own doc comment for
        // why a protection violation there is the SECOND of its two
        // real, deliberate write-refusal paths, not a bug). Passes
        // `error_code`'s own CAUSED_BY_WRITE bit through explicitly --
        // the ONE piece of information try_demand_page_heap() never
        // needed (every heap page is always writable once mapped, so a
        // write-vs-read distinction on a not-present heap fault is
        // meaningless there) but this function's own FIRST refusal path
        // depends on entirely.
        if !error_code.contains(PageFaultErrorCode::PROTECTION_VIOLATION)
            && crate::process::try_demand_page_mmap(active, fault_addr, error_code.contains(PageFaultErrorCode::CAUSED_BY_WRITE))
        {
            return;
        }
        // MILESTONE 75: distinguish a genuine stack overflow from a
        // generic wild-pointer SIGSEGV in the log only -- see
        // STACK_GUARD_REGION_SIZE's own doc comment above for the full
        // reasoning. A NOT-PRESENT fault (never a protection violation
        // -- there is no mapped-but-wrong-permission page anywhere in
        // this 1 MiB band, only genuinely unmapped memory) whose CR2
        // falls strictly below USER_STACK_ADDR and within
        // STACK_GUARD_REGION_SIZE bytes of it is overwhelmingly likely
        // the active process running its own stack pointer off the
        // bottom of its single mapped page.
        let stack_bottom = usertest::USER_STACK_ADDR;
        let looks_like_stack_overflow = !error_code.contains(PageFaultErrorCode::PROTECTION_VIOLATION)
            && fault_addr < stack_bottom
            && fault_addr >= stack_bottom.saturating_sub(STACK_GUARD_REGION_SIZE);
        if looks_like_stack_overflow {
            let _ = writeln!(
                serial(),
                "milestone 75: STACK OVERFLOW -- process {active} ran off the bottom of its own single {}-byte stack page (fault address {:?}, {} bytes below USER_STACK_ADDR {:#x} -- inside this kernel's real, deliberately-unmapped guard gap below the stack, not a wild-pointer SIGSEGV) -- terminating this process, kernel continues",
                usertest::USER_STACK_SIZE,
                Cr2::read(),
                stack_bottom - fault_addr,
                stack_bottom
            );
        } else {
            let _ = writeln!(
                serial(),
                "milestone 41: SIGSEGV -- process {active} page-faulted at {:?} (real hardware CPL=3, error code {:?}) -- terminating this process, kernel continues",
                Cr2::read(),
                error_code
            );
        }
        crate::usertest::terminate_faulted_process_and_resume_kernel();
    }
    let mut port = serial();
    let _ = writeln!(port, "EXCEPTION: PAGE FAULT");
    let _ = writeln!(port, "Accessed Address: {:?}", Cr2::read());
    let _ = writeln!(port, "Error Code: {:?}", error_code);
    let _ = writeln!(port, "{:#?}", stack_frame);
    hlt_loop();
}

/// MILESTONE 41: a real, previously-undiscovered gap found while
/// debugging the SIGSEGV self-test's own boot-time verification: this
/// IDT never registered a `general_protection_fault` handler at all.
/// Every prior milestone's ring-3 work happened to never trigger a real
/// #GP, so this gap was invisible. An earlier round of that same
/// debugging session captured a real `-d int` hardware trace showing a
/// #GP (vector 0xd, error code 0x18 -- GDT index 3, the user code
/// selector) at roughly this point in execution, which was the original
/// motivation for adding this handler.
///
/// **Honestly disclosed: adding this handler did NOT fix the SIGSEGV
/// self-test's own double-fault crash** -- verified by direct A/B
/// testing (identical build, only this handler added/removed): the
/// double fault still occurs, with the exact same "garbled" field
/// values, and this handler's own log line never prints, meaning
/// whatever actually faults is not reaching IDT[13] in a form this
/// handler catches. That #GP trace may have been from a different
/// moment than what's crashing now (a lot of other fixes landed in
/// between the two investigation rounds), or the real fault is a
/// different vector entirely. Kept anyway because a real, previously
/// nonexistent #GP handler -- mirroring page_fault_handler's exact
/// CPL=3-terminates-the-process design -- is independently correct and
/// worth having regardless of whether it explains this specific bug;
/// same reasoning as this same investigation's earlier stack-alignment
/// fix. The double-fault root cause remains open.
extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    let active = crate::process::ACTIVE_PROCESS.load(Ordering::Relaxed);
    if stack_frame.code_segment.rpl() == PrivilegeLevel::Ring3 && active != 0 {
        let _ = writeln!(
            serial(),
            "milestone 41: GP fault -- process {active} general-protection-faulted (real hardware CPL=3, error code {error_code:#x}) -- terminating this process, kernel continues",
        );
        crate::usertest::terminate_faulted_process_and_resume_kernel();
    }
    let mut port = serial();
    let _ = writeln!(port, "EXCEPTION: GENERAL PROTECTION FAULT");
    let _ = writeln!(port, "Error Code: {error_code:#x}");
    let _ = writeln!(port, "{:#?}", stack_frame);
    hlt_loop();
}
