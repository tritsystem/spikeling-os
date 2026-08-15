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
        let _ = writeln!(
            serial(),
            "milestone 41: SIGSEGV -- process {active} page-faulted at {:?} (real hardware CPL=3, error code {:?}) -- terminating this process, kernel continues",
            Cr2::read(),
            error_code
        );
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
