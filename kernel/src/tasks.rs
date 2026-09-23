//! MILESTONE 5, STAGE C: real per-task stacks and a genuine preemptive
//! context switch, driven by the timer interrupt (stage B) and
//! selecting which task runs next via the SAME TopologicalScheduler
//! from Milestone 4 -- closing the loop the README named as the actual
//! goal: "this is where Spikeling's spiking-network logic becomes a
//! real candidate to replace a conventional scheduler, once there's a
//! scheduler slot to replace." Now there is one, and it's real: the
//! three worker tasks below never call back into scheduling code
//! themselves -- they just spin, incrementing their own counter,
//! entirely at the mercy of the timer interrupt firing asynchronously
//! mid-loop. That's the actual distinction from Milestone 4's
//! cooperative closures.
//!
//! MILESTONE 25: real dynamic task lifecycle on top of the above --
//! spawn() and kill() let the shell create and destroy worker tasks at
//! runtime instead of the fixed 3 booted once by run_preemption_demo().
//! Two things had to change to make that real rather than cosmetic:
//! (1) TopologicalScheduler's slot bank, fixed-size since Milestone 4,
//! needed a genuine "grow" operation (add_slot) and a way to bring a
//! killed slot back to life (revive), not just kill(); (2) the original
//! demo's timer_tick_switch() stopped switching entirely once its
//! bounded MAX_TICKS window closed (by design, so kernel_main could
//! regain control and finish booting) -- spawned tasks would never
//! actually run if that stayed permanent, so kernel_main now flips on
//! BACKGROUND scheduling once, right before its own final hlt_loop(),
//! at which point it has nothing left to do and losing control to a
//! worker task forever is harmless. A killed task's stack is freed
//! immediately UNLESS it's the one currently running (a real case: kill
//! runs nested inside the keyboard ISR, on top of whatever task's stack
//! happened to be current when the key was pressed, including possibly
//! its own target) -- that case is queued in ZOMBIE and reaped lazily,
//! the first tick a real switch has actually carried execution off it.

use crate::scheduler::TopologicalScheduler;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;

// MILESTONE 104: `pub(crate)` (was private) so `heap_guard.rs`'s real
// admission gate can size its own checks against this crate's ACTUAL
// per-task stack request, rather than a second, independently
// maintained copy of "16 KiB" that could silently drift from this one.
pub(crate) const STACK_SIZE: usize = 4096 * 4; // 16 KiB per task
const N_TASKS: usize = 3; // original Milestone 5c fixed task count
const MAX_TASKS: usize = 8; // Milestone 25: hard cap on concurrent task slots

/// MILESTONE 131: `spawn()`'s own bounded retry budget for a
/// `DeniedGateConservative` refusal only (never for `DeniedHardFloor`,
/// which retrying can never fix). 3 attempts, real, deliberate: this
/// kernel's heap-pressure gate re-evaluates only every
/// `golden_period()` samples (a REAL process-teardown event, not a
/// clock tick), so a single retry could easily land between the same
/// two samples the gate is already averaging -- 3 gives real intervening
/// activity (a background task's own teardown, a shell command
/// completing) a genuine chance to land inside the wait window without
/// spawn() blocking indefinitely on a boot that never recovers.
const SPAWN_RETRY_ATTEMPTS: u32 = 3;
/// MILESTONE 131: real `hlt()` cycles per retry -- the same "wait via
/// real hlt, let pending interrupts actually run" shape Milestone 6's
/// own keyboard-wait window already established (`for _ in 0..150 {
/// hlt() }`), scaled down since this wait can repeat up to 3 times.
const SPAWN_RETRY_HLT_CYCLES: u32 = 50;
const MAX_TICKS: u64 = 60; // ~3.3s of real time at the default ~18.2Hz PIT rate

pub type TaskId = usize;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct TaskContext {
    rsp: u64,
}

struct Task {
    ctx: TaskContext,
    _stack: Box<[u8]>, // kept alive for the task's lifetime; never read directly after setup
}

impl Task {
    /// Pre-populates the new stack to look as if it already called
    /// switch_to() once, so the FIRST switch_to() INTO this task pops
    /// harmless zeroed register values and `ret`s straight into
    /// `entry` -- exactly like resuming a previously-interrupted task,
    /// just with a synthetic first "interruption".
    fn new(entry: extern "C" fn() -> !) -> Task {
        let mut stack = alloc::vec![0u8; STACK_SIZE].into_boxed_slice();
        let stack_top = unsafe { stack.as_mut_ptr().add(STACK_SIZE) };
        unsafe {
            let mut sp = stack_top as *mut u64;
            sp = sp.sub(1);
            *sp = entry as u64; // what switch_to's `ret` will jump to
            for _ in 0..6 {
                // rbx, rbp, r12, r13, r14, r15 -- matches switch_to's push order
                sp = sp.sub(1);
                *sp = 0;
            }
            Task {
                ctx: TaskContext { rsp: sp as u64 },
                _stack: stack,
            }
        }
    }
}

/// Minimal context switch: only the callee-saved registers + stack
/// pointer, following the standard "switch_to" pattern (Redox, xv6,
/// countless minimal-kernel tutorials). Correct specifically because
/// it's called as an ORDINARY nested function call from within the
/// timer interrupt handler's Rust body -- the compiler-generated
/// x86-interrupt prologue/epilogue already protects the interrupted
/// task's caller-saved registers on ITS OWN stack; this only needs to
/// additionally preserve what a normal call boundary requires anyway.
///
/// DIAGNOSED AND FIXED: the very first version of this hung completely
/// after exactly one real switch, never returning to kernel_main. Root
/// cause: switching stacks via a plain `ret` (not a real `iretq`) means
/// the CPU's interrupt flag -- automatically cleared on interrupt
/// entry -- never gets restored. Once the first switch happened from
/// inside the timer handler, interrupts stayed permanently disabled,
/// so no further timer tick could ever fire to drive another switch --
/// whichever task got landed on just ran forever. Fixed with an
/// explicit `sti` before `ret`, on every switch (harmless/idempotent
/// when interrupts are already enabled, e.g. the initial bootstrap
/// switch from ordinary kernel_main code).
#[unsafe(naked)]
unsafe extern "C" fn switch_to(_old_rsp_ptr: *mut u64, _new_rsp: u64) {
    core::arch::naked_asm!(
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "sti",
        "ret",
    );
}

pub static TASK_COUNTERS: [AtomicU64; MAX_TASKS] = [const { AtomicU64::new(0) }; MAX_TASKS];

static TASKS: Mutex<Vec<Option<Task>>> = Mutex::new(Vec::new());
static ZOMBIE: Mutex<Vec<TaskId>> = Mutex::new(Vec::new()); // MILESTONE 25: self-killed tasks awaiting a safe reap
static KERNEL_RSP: AtomicU64 = AtomicU64::new(0);
static CURRENT: AtomicUsize = AtomicUsize::new(usize::MAX); // MAX = "kernel_main/shell is running, no worker active"
static SCHED: Mutex<Option<TopologicalScheduler>> = Mutex::new(None);
static TOTAL_SWITCHES: AtomicU64 = AtomicU64::new(0);
static DEMO_ACTIVE: AtomicBool = AtomicBool::new(false); // bounded Milestone 5c window only
static BACKGROUND: AtomicBool = AtomicBool::new(false); // MILESTONE 25: perpetual post-boot scheduling

extern "C" fn worker_entry_0() -> ! {
    worker_loop(0)
}
extern "C" fn worker_entry_1() -> ! {
    worker_loop(1)
}
extern "C" fn worker_entry_2() -> ! {
    worker_loop(2)
}

/// Slots 0-2 always spin at the original Milestone 5c rate (kept exact,
/// so the fixed demo's reported counters are unchanged); spawned slots
/// (>= N_TASKS) alternate a faster/slower spin count by id parity --
/// two genuinely distinguishable demo bodies, per Milestone 25, rather
/// than clones differing only by which counter they touch.
fn worker_loop(id: usize) -> ! {
    let spin_count = if id < N_TASKS || id % 2 == 0 { 2000 } else { 6000 };
    loop {
        TASK_COUNTERS[id].fetch_add(1, Ordering::Relaxed);
        for _ in 0..spin_count {
            core::hint::spin_loop();
        }
    }
}

/// MILESTONE 25: one entry stub per possible task slot -- switch_to's
/// `ret` needs a plain fn pointer with no captured state, so the id a
/// spawned task counts under has to be baked in at compile time via a
/// distinct fn item per slot, not passed as an argument.
static DEMO_ENTRIES: [extern "C" fn() -> !; MAX_TASKS] = [
    { extern "C" fn e0() -> ! { worker_loop(0) } e0 },
    { extern "C" fn e1() -> ! { worker_loop(1) } e1 },
    { extern "C" fn e2() -> ! { worker_loop(2) } e2 },
    { extern "C" fn e3() -> ! { worker_loop(3) } e3 },
    { extern "C" fn e4() -> ! { worker_loop(4) } e4 },
    { extern "C" fn e5() -> ! { worker_loop(5) } e5 },
    { extern "C" fn e6() -> ! { worker_loop(6) } e6 },
    { extern "C" fn e7() -> ! { worker_loop(7) } e7 },
];

/// Bootstraps the 3 worker tasks, switches into the first one, and
/// blocks (from kernel_main's point of view) until the timer-driven
/// switcher below has counted MAX_TICKS timer interrupts and switched
/// back here. None of the worker loops above ever call this module's
/// scheduling code themselves -- the only thing moving execution
/// between them is the timer interrupt.
/// MILESTONE 104: real, honest kernel-heap-pressure guard, checked
/// BEFORE any of the three worker stacks below are allocated -- closes
/// the real Milestone 102-disclosed gap where this function's own first
/// `Task::new()` (a real 16 KiB `alloc::vec![0u8; STACK_SIZE]`) could
/// hit a kernel heap already run low by everything the self-test suite
/// did earlier in the SAME boot, and panic via
/// `alloc::alloc::handle_alloc_error()` rather than fail gracefully. All
/// 3 * STACK_SIZE (48 KiB) checked as ONE up-front admission, not
/// per-task, deliberately: partially creating this demo (1 or 2 workers
/// but not 3) would leave `DEMO_ACTIVE`/`TASK_COUNTERS` in a state no
/// caller of this function has ever had to handle -- an all-or-nothing
/// admission keeps this function's own existing contract (either the
/// real demo runs, or it doesn't) completely intact. See
/// `heap_guard.rs`'s own module doc for the real evidence-gated
/// admission design and the honest naive-vs-evidence-gated benchmark.
pub fn run_preemption_demo() {
    // MILESTONE 123: heap_guard::admit_ranked() distinguishes WHY a
    // refusal happened -- see that function's own doc. Logged here
    // purely for diagnostic legibility; the refuse-and-return behavior
    // itself is unchanged (this function still cannot retry either way).
    let verdict = crate::heap_guard::admit_ranked(3 * STACK_SIZE);
    if verdict != crate::heap_guard::AdmitVerdict::Admitted {
        let _ = writeln!(
            crate::serial(),
            "milestone 104: run_preemption_demo() SKIPPED -- heap_guard::admit() refused the real {} bytes this demo's 3 worker stacks need ({:?}); see the milestone 104 heap-pressure lines above for the real evidence that led here",
            3 * STACK_SIZE,
            verdict
        );
        return;
    }
    {
        let mut tasks = TASKS.lock();
        tasks.push(Some(Task::new(worker_entry_0)));
        tasks.push(Some(Task::new(worker_entry_1)));
        tasks.push(Some(Task::new(worker_entry_2)));
    }
    *SCHED.lock() = Some(TopologicalScheduler::new(N_TASKS, 0.6));
    TOTAL_SWITCHES.store(0, Ordering::SeqCst);
    CURRENT.store(0, Ordering::SeqCst);
    DEMO_ACTIVE.store(true, Ordering::SeqCst);

    let first_rsp = TASKS.lock()[0].as_ref().unwrap().ctx.rsp;
    unsafe {
        switch_to(KERNEL_RSP.as_ptr(), first_rsp);
    }
    // execution resumes HERE once the timer handler's "return to
    // kernel_main" branch below switches back -- reached via the
    // return address switch_to's `ret` restores from this call's own
    // stack frame, exactly as if this function call had simply
    // returned normally after some delay.
}

/// MILESTONE 25: called once, right before kernel_main's own final
/// hlt_loop() -- from that point on kernel_main has nothing left to
/// do, so it's safe for the timer tick to start permanently favoring
/// whichever worker task the scheduler picks over resuming "kernel".
pub fn enable_background_scheduling() {
    BACKGROUND.store(true, Ordering::SeqCst);
}

/// Locates the raw stack-pointer-storage location for `current`
/// (either a real task's ctx.rsp, or the dedicated KERNEL_RSP static
/// standing in for kernel_main/the shell) -- shared by every switch
/// site below instead of duplicating the usize::MAX special case.
fn current_rsp_ptr(current: usize) -> *mut u64 {
    if current == usize::MAX {
        KERNEL_RSP.as_ptr()
    } else {
        let mut tasks = TASKS.lock();
        &mut tasks[current]
            .as_mut()
            .expect("current task slot missing")
            .ctx
            .rsp as *mut u64
    }
}

/// MILESTONE 25: allocates a new task (reusing a dead slot's index if
/// one exists, otherwise growing the scheduler by one live slot) and
/// registers it to run the demo counting loop under its assigned id.
/// Returns None if the scheduler hasn't been bootstrapped yet or the
/// MAX_TASKS cap has been reached.
///
/// MILESTONE 104: also returns None -- no new failure mode for this
/// function's own existing callers, who already have to handle `None`
/// -- if `heap_guard::admit()` refuses this task's real STACK_SIZE
/// request. Checked FIRST, before any scheduler-slot state is touched
/// (revive/add_slot), so a refusal here never leaves a half-grown
/// scheduler slot with no task behind it.
///
/// MILESTONE 131: this is the "future milestone" Milestone 123's own
/// `heap_guard::AdmitVerdict` doc named -- `admit_ranked()`'s two
/// refusal reasons are now genuinely ACTED on here, not just logged.
/// `DeniedHardFloor` still fails immediately (unconditional -- retrying
/// cannot help until more heap is actually freed, so a retry loop here
/// would just waste real hlt cycles). `DeniedGateConservative` gets up
/// to `SPAWN_RETRY_ATTEMPTS` real, bounded retries, each separated by
/// `SPAWN_RETRY_HLT_CYCLES` real `hlt()`s -- the same "wait via real
/// hlt cycles, not a busy loop" shape Milestone 6's own keyboard-wait
/// window already established. A real `hlt()` lets pending interrupts
/// (including another process's real teardown, which is what feeds
/// `record_teardown_sample()` and can clear the gate) actually run
/// during the wait, unlike a pure CPU spin.
pub fn spawn() -> Option<TaskId> {
    let mut verdict = crate::heap_guard::admit_ranked(STACK_SIZE);
    let mut retries_used: u32 = 0;
    while verdict == crate::heap_guard::AdmitVerdict::DeniedGateConservative
        && retries_used < SPAWN_RETRY_ATTEMPTS
    {
        retries_used += 1;
        for _ in 0..SPAWN_RETRY_HLT_CYCLES {
            x86_64::instructions::hlt();
        }
        verdict = crate::heap_guard::admit_ranked(STACK_SIZE);
    }
    if verdict != crate::heap_guard::AdmitVerdict::Admitted {
        let _ = writeln!(
            crate::serial(),
            "milestone 131: spawn() refused -- {:?} after {} real retry attempt(s)",
            verdict,
            retries_used
        );
        return None;
    }
    let mut tasks = TASKS.lock();
    let mut sched_guard = SCHED.lock();
    let sched = sched_guard.as_mut()?;

    let id = match tasks.iter().position(|t| t.is_none()) {
        Some(id) => {
            sched.revive(id);
            id
        }
        None => {
            if tasks.len() >= MAX_TASKS {
                return None;
            }
            let id = tasks.len();
            tasks.push(None);
            let added = sched.add_slot();
            debug_assert_eq!(added, id);
            id
        }
    };

    TASK_COUNTERS[id].store(0, Ordering::Relaxed);
    tasks[id] = Some(Task::new(DEMO_ENTRIES[id]));
    Some(id)
}

/// MILESTONE 25: terminates a live task for real -- the scheduler
/// slot goes dead immediately (it will never be picked again), and the
/// stack is freed immediately too UNLESS `id` is the task currently
/// executing (e.g. this call is running nested inside the keyboard ISR
/// on top of task `id`'s own stack, because task `id` was current when
/// the key was pressed) -- freeing THAT stack out from under ourselves
/// would corrupt the very frame we're about to iretq back into. That
/// case is deferred to reap_zombies(), safe once a later tick's real
/// switch has actually carried execution off this stack for good.
pub fn kill(id: TaskId) -> bool {
    let mut sched_guard = SCHED.lock();
    let Some(sched) = sched_guard.as_mut() else {
        return false;
    };
    if id >= sched.slots.len() || !sched.slots[id].alive {
        return false;
    }
    sched.kill(id);
    drop(sched_guard);

    if CURRENT.load(Ordering::SeqCst) == id {
        ZOMBIE.lock().push(id);
    } else {
        TASKS.lock()[id] = None; // drops the Task, freeing its Box<[u8]> stack now
    }
    true
}

/// Frees any zombie's stack once it's no longer the currently-running
/// task -- called every tick so a deferred self-kill gets reaped the
/// first real opportunity, not left leaking forever.
fn reap_zombies() {
    let current = CURRENT.load(Ordering::SeqCst);
    let mut zombies = ZOMBIE.lock();
    zombies.retain(|&id| {
        if id == current {
            true
        } else {
            TASKS.lock()[id] = None;
            false
        }
    });
}

pub struct TaskReport {
    pub id: TaskId,
    pub counter: u64,
}

/// MILESTONE 25: every task the scheduler currently considers alive --
/// what the shell's `tasks` command reports, so spawn/kill are visible
/// as genuine growth/shrinkage of this list, not just a command that
/// returned success.
pub fn live_tasks() -> Vec<TaskReport> {
    let tasks = TASKS.lock();
    let sched_guard = SCHED.lock();
    let mut out = Vec::new();
    if let Some(sched) = sched_guard.as_ref() {
        for (id, slot) in tasks.iter().enumerate() {
            if slot.is_some() && sched.slots[id].alive {
                out.push(TaskReport {
                    id,
                    counter: TASK_COUNTERS[id].load(Ordering::Relaxed),
                });
            }
        }
    }
    out
}

/// Called from the timer interrupt handler (after EOI). Uses the same
/// TopologicalScheduler design as Milestone 4 to pick which worker
/// runs next, or switches back to kernel_main once MAX_TICKS is
/// reached (the original bounded Milestone 5c demo window). Once
/// Milestone 25's enable_background_scheduling() has been called, the
/// same selection logic keeps running indefinitely afterward with no
/// tick budget -- kernel_main/the shell is just another context that
/// can lose the CPU to a worker task and, since it has nothing left to
/// do but hlt forever at that point, never needs it back.
///
/// DIAGNOSED AND FIXED (Milestone 5c): step() legitimately returns
/// None on most ticks (no slot has crossed threshold yet -- accumulation
/// takes several ticks) -- that used to be conflated with the SEPARATE
/// "time budget exhausted, return to kernel_main" condition, both
/// falling into one `None` match arm. The very first ordinary "no
/// winner yet" tick was ending the whole demo immediately, which is
/// why only the first task ever ran. Fixed by checking the tick budget
/// unconditionally first, independent of whatever the scheduler does
/// or doesn't return.
pub fn timer_tick_switch() {
    reap_zombies();

    let current = CURRENT.load(Ordering::SeqCst);

    if DEMO_ACTIVE.load(Ordering::SeqCst) {
        let ticks = TOTAL_SWITCHES.fetch_add(1, Ordering::SeqCst);
        if ticks + 1 >= MAX_TICKS {
            DEMO_ACTIVE.store(false, Ordering::SeqCst);
            CURRENT.store(usize::MAX, Ordering::SeqCst);
            let new_rsp = KERNEL_RSP.load(Ordering::SeqCst);
            let old_rsp_ptr = current_rsp_ptr(current);
            unsafe {
                switch_to(old_rsp_ptr, new_rsp);
            }
            return;
        }
    } else if !BACKGROUND.load(Ordering::SeqCst) {
        return; // neither the bounded demo nor Milestone 25 background mode is active
    }

    // MILESTONE 117: real production scheduling now goes through
    // step_fairness_gated() -- the evidence-gated fairness intervention
    // (scheduler.rs's own module doc has the full mechanism and the
    // real, measured before/after numbers) -- not the plain step() this
    // call used since Milestone 5c. step() itself is unchanged and still
    // exercised directly by main.rs's own deterministic boot-time trial.
    let next = SCHED.lock().as_mut().and_then(|s| s.step_fairness_gated());
    if let Some(next_idx) = next {
        if current != next_idx {
            let old_rsp_ptr = current_rsp_ptr(current);
            let new_rsp = TASKS.lock()[next_idx]
                .as_ref()
                .expect("scheduled task slot missing")
                .ctx
                .rsp;
            CURRENT.store(next_idx, Ordering::SeqCst);
            unsafe {
                switch_to(old_rsp_ptr, new_rsp);
            }
        }
        // else: scheduler re-picked the currently running task -- nothing to do
    }
    // else: no slot crossed threshold yet this tick -- try again next tick
}
