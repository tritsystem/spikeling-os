//! Physical memory management: turns bootloader-reported memory regions
//! into a frame allocator, and provides an OffsetPageTable for creating
//! new virtual mappings -- both required before a heap allocator can
//! carve out and map new pages (see allocator.rs).
//!
//! MILESTONE 34: every frame allocation through Milestone 30 happened
//! with a `frame_allocator` local to kernel_main, threaded explicitly
//! through a handful of ONE-TIME boot setup calls (init_heap,
//! usertest::setup, process::init_test_processes). loader.rs's
//! `runfile` shell command is the first thing that needs to allocate a
//! fresh physical frame from ARBITRARY, LATER, shell-command-driven
//! code -- long after kernel_main's own boot sequence has finished --
//! so a real place to reach a live BootInfoFrameAllocator (and the
//! phys_mem_offset needed to translate its frames into accessible
//! pointers) from has to exist. install_frame_allocator()/
//! set_phys_mem_offset() publish exactly that, called once from
//! kernel_main right after the last boot-time consumer is done with
//! them.

use alloc::vec::Vec;
use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;
use x86_64::{
    PhysAddr, VirtAddr,
    registers::control::Cr3,
    structures::paging::{FrameAllocator, FrameDeallocator, OffsetPageTable, PageTable, PhysFrame, Size4KiB},
};

/// Builds an OffsetPageTable from the currently active level-4 page
/// table, using `physical_memory_offset` (mapped by the bootloader
/// because BOOTLOADER_CONFIG in main.rs requests it) to translate
/// physical frame addresses into accessible virtual addresses.
///
/// # Safety
/// Caller must guarantee the complete physical memory is actually
/// mapped at `physical_memory_offset`, and this must only be called
/// once (aliasing more than one `&mut` to the level 4 table is UB).
pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    let level_4_table = unsafe { active_level_4_table(physical_memory_offset) };
    unsafe { OffsetPageTable::new(level_4_table, physical_memory_offset) }
}

unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    let (level_4_table_frame, _) = Cr3::read();

    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    unsafe { &mut *page_table_ptr }
}

/// MILESTONE 157: an O(1)-amortized cursor over the same frame sequence
/// the old `usable_frames()` defined (usable regions in map order, 4 KiB steps from
/// each region's start). The old `allocate_frame()` did
/// `usable_frames().nth(self.next)` -- restarting the walk at the first
/// region on EVERY fresh frame, so the k-th frame cost O(k): Milestone 154
/// measured ~346k cycles per fresh frame at only ~600 frames in use, and
/// the cost grows linearly with the index (~30M cycles per frame near the
/// end of the ~29.5k-frame pool). The cursor remembers which usable region
/// it is in and the next address inside it.
struct FrameCursor {
    usable_idx: usize,
    addr: u64,
    end: u64,
    loaded: bool,
}

impl FrameCursor {
    const fn new() -> Self {
        FrameCursor { usable_idx: 0, addr: 0, end: 0, loaded: false }
    }

    /// The next frame in exactly the order the reference definition
    /// (`reference_nth_frame(regions, k)` for k = 0, 1, 2, ...) yields
    /// them -- `None` once the pool is exhausted. The current region's
    /// bounds are cached, so the region list is only re-filtered when a
    /// region runs out (the first version re-filtered on every call and
    /// measured 8,010 cycles/frame against a pre-registered < 5,000).
    fn next_frame(&mut self, regions: &MemoryRegions) -> Option<PhysFrame> {
        loop {
            if self.loaded && self.addr < self.end {
                let frame = PhysFrame::containing_address(PhysAddr::new(self.addr));
                self.addr += 4096;
                return Some(frame);
            }
            let idx = if self.loaded { self.usable_idx + 1 } else { self.usable_idx };
            let r = regions.iter().filter(|r| r.kind == MemoryRegionKind::Usable).nth(idx)?;
            self.usable_idx = idx;
            self.addr = r.start;
            self.end = r.end;
            self.loaded = true;
        }
    }
}

/// A frame allocator that returns usable frames from the bootloader's
/// memory map -- real physical memory the firmware reported as free,
/// not a synthetic pool. Bump-allocates from `next` for genuinely new
/// frames, but MILESTONE 54 adds a real free list checked FIRST: a
/// process that exits/is killed/is exec()'d over now actually returns
/// its old PML4/code/stack/heap/extra frames here (see process.rs's
/// `reclaim_process_frames()`), and a later allocation reuses them
/// before ever bumping `next` further. LIFO (`Vec::pop()`), not that it
/// matters for correctness -- any free frame is as good as any other,
/// this just avoids a separate front/back-pointer structure for no
/// real benefit.
pub struct BootInfoFrameAllocator {
    memory_regions: &'static MemoryRegions,
    next: usize,
    /// MILESTONE 157: where the next fresh frame comes from, so fetching it
    /// is O(1) instead of re-walking the whole frame list from the start.
    cursor: FrameCursor,
    free_list: Vec<PhysFrame<Size4KiB>>,
}

impl BootInfoFrameAllocator {
    /// # Safety
    /// The passed memory regions must be accurate -- frames marked
    /// `Usable` must genuinely not be used elsewhere (kernel code/data,
    /// the boot info structure itself, etc.). True of what
    /// bootloader_api reports; would not be true of an arbitrary or
    /// synthetic memory map.
    pub unsafe fn init(memory_regions: &'static MemoryRegions) -> Self {
        BootInfoFrameAllocator {
            memory_regions,
            next: 0,
            cursor: FrameCursor::new(),
            free_list: Vec::new(),
        }
    }

    /// MILESTONE 54: real count of frames currently sitting on the free
    /// list, waiting to be reused -- exposed so a self-test can prove
    /// reclamation actually happened (a real count change), not just
    /// that `deallocate_frame()` was called without erroring.
    pub fn free_list_len(&self) -> usize {
        self.free_list.len()
    }

    /// The reference (pre-M157) definition of the k-th fresh frame -- the
    /// O(k) walk, kept ONLY so `self_test_frame_cursor()` can prove the
    /// cursor yields the identical sequence.
    #[allow(dead_code)]
    fn reference_nth(&self, k: usize) -> Option<PhysFrame> {
        reference_nth_frame(self.memory_regions, k)
    }

    /// MILESTONE 154 MEASUREMENT: total 4 KiB frames the bootloader reported usable.
    pub fn total_usable_frames(&self) -> u64 {
        self.memory_regions
            .iter()
            .filter(|r| r.kind == MemoryRegionKind::Usable)
            .map(|r| (r.end - r.start + 4095) / 4096)
            .sum()
    }
}

static FRAME_FRESH_ALLOCS: AtomicU64 = AtomicU64::new(0);
static FRAME_FREELIST_ALLOCS: AtomicU64 = AtomicU64::new(0);
static FRAME_FRESH_CYCLES: AtomicU64 = AtomicU64::new(0);
static FRAME_LAST_CYCLES: AtomicU64 = AtomicU64::new(0);

/// MILESTONE 154 MEASUREMENT: (fresh_allocs, freelist_allocs, total_fresh_cycles,
/// last_fresh_cycles, total_usable_frames).
pub fn frame_stats(total_usable: u64) -> (u64, u64, u64, u64, u64) {
    (
        FRAME_FRESH_ALLOCS.load(Ordering::Relaxed),
        FRAME_FREELIST_ALLOCS.load(Ordering::Relaxed),
        FRAME_FRESH_CYCLES.load(Ordering::Relaxed),
        FRAME_LAST_CYCLES.load(Ordering::Relaxed),
        total_usable,
    )
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        // MILESTONE 54: check the free list first -- a real reclaimed
        // frame is exactly as usable as a never-before-allocated one,
        // and preferring it keeps physical memory usage bounded across
        // repeated fork()/exit()/exec() cycles instead of only ever
        // growing.
        if let Some(frame) = self.free_list.pop() {
            FRAME_FREELIST_ALLOCS.fetch_add(1, Ordering::Relaxed);
            return Some(frame);
        }
        // MILESTONE 154 MEASUREMENT (instrumentation only): cycles one fresh
        // frame costs. Milestone 154 measured ~346k cycles per frame here
        // (an O(next) `usable_frames().nth(next)` walk); Milestone 157
        // replaced it with `FrameCursor`.
        let t0 = unsafe { core::arch::x86_64::_rdtsc() };
        let frame = self.cursor.next_frame(self.memory_regions);
        let t1 = unsafe { core::arch::x86_64::_rdtsc() };
        FRAME_FRESH_ALLOCS.fetch_add(1, Ordering::Relaxed);
        FRAME_FRESH_CYCLES.fetch_add(t1 - t0, Ordering::Relaxed);
        FRAME_LAST_CYCLES.store(t1 - t0, Ordering::Relaxed);
        self.next += 1;
        frame
    }
}

impl FrameDeallocator<Size4KiB> for BootInfoFrameAllocator {
    /// # Safety
    /// Caller must guarantee `frame` is no longer referenced by ANY
    /// live page table -- freeing a frame still mapped somewhere would
    /// let a later allocation hand out the same physical memory twice
    /// to two genuinely different, simultaneously-live owners.
    /// process.rs's `reclaim_process_frames()` only ever calls this on
    /// a `Process` value it has just taken full ownership of (via
    /// `Option::take()`/`Mutex::replace()`), after that process's own
    /// slot is already gone from every table this kernel tracks, so no
    /// other code path can still be pointing at these frames.
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        self.free_list.push(frame);
    }
}

/// MILESTONE 34: the globally-published frame allocator -- see the
/// module doc comment above. None until install_frame_allocator() runs
/// (once, from kernel_main); Some(...) permanently after that, for the
/// rest of the kernel's uptime.
static FRAME_ALLOCATOR: Mutex<Option<BootInfoFrameAllocator>> = Mutex::new(None);

/// The physical-memory-offset VirtAddr, stored as a raw u64 (VirtAddr
/// itself isn't atomic-friendly) -- same pattern process.rs's
/// KERNEL_PML4_FRAME already established for publishing a single,
/// boot-computed value for later, arbitrary-time reads.
static PHYS_MEM_OFFSET: AtomicU64 = AtomicU64::new(0);

/// Called once from kernel_main, after `allocator` (the very same
/// BootInfoFrameAllocator that already ran init_heap/usertest::setup/
/// process::init_test_processes) has nothing left to do at boot time --
/// moves it into this static so loader.rs's `runfile` can allocate
/// fresh frames for a NEW process's private page tables at any later
/// point, driven by an arbitrary shell command instead of one of the
/// fixed boot-time setup calls.
pub fn install_frame_allocator(allocator: BootInfoFrameAllocator) {
    *FRAME_ALLOCATOR.lock() = Some(allocator);
}

/// Called once from kernel_main, alongside install_frame_allocator().
pub fn set_phys_mem_offset(offset: VirtAddr) {
    PHYS_MEM_OFFSET.store(offset.as_u64(), Ordering::SeqCst);
}

/// The published phys_mem_offset -- 0 (an invalid/unused value in
/// practice; the bootloader's real dynamic mapping is never placed at
/// virtual address 0) until set_phys_mem_offset() has run.
pub fn phys_mem_offset() -> VirtAddr {
    VirtAddr::new(PHYS_MEM_OFFSET.load(Ordering::SeqCst))
}

/// Runs `f` with mutable access to the globally-published frame
/// allocator, returning None (rather than panicking) if
/// install_frame_allocator() hasn't run yet -- shouldn't be reachable
/// once the shell is accepting commands (kernel_main installs it well
/// before shell::init()/interrupts are enabled), but reported honestly
/// through the Option rather than assumed.
pub fn with_frame_allocator<R>(f: impl FnOnce(&mut BootInfoFrameAllocator) -> R) -> Option<R> {
    let mut guard = FRAME_ALLOCATOR.lock();
    guard.as_mut().map(f)
}

/// MILESTONE 157: boot-time proof that `FrameCursor` yields EXACTLY the
/// frame sequence the old `usable_frames().nth(k)` did, and is far cheaper.
/// Pre-registered: (1) the cursor's full walk visits exactly
/// `total_usable_frames()` frames and then returns `None`; (2) at k = 0, 1,
/// 100, 5000, 15000 and the last index it returns the same frame as the
/// reference `nth(k)`, and the first 2000 frames match one-for-one; (3) the
/// cursor costs under 5,000 cycles per frame over the full walk while the
/// reference `nth(k)` near the end costs at least 100x that.
fn reference_nth_frame(regions: &MemoryRegions, k: usize) -> Option<PhysFrame> {
    regions
        .iter()
        .filter(|r| r.kind == MemoryRegionKind::Usable)
        .map(|r| r.start..r.end)
        .flat_map(|r| r.step_by(4096))
        .map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
        .nth(k)
}

pub fn self_test_frame_cursor() -> bool {
    use core::fmt::Write;
    fn tsc() -> u64 {
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    let Some((regions, counted)) = with_frame_allocator(|fa| (fa.memory_regions, fa.total_usable_frames())) else {
        return false;
    };
    // Independent count from the reference definition itself, cross-checked
    // against `total_usable_frames()` (the first boot's floor-division
    // undercount by one -- 29,502 vs the iterator's 29,503 -- is fixed).
    let total = regions
        .iter()
        .filter(|r| r.kind == MemoryRegionKind::Usable)
        .map(|r| r.start..r.end)
        .flat_map(|r| r.step_by(4096))
        .count();
    let counted_agrees = counted as usize == total;

    // Full cursor walk, timed, comparing against the reference definition
    // as it goes (no big stack arrays): probes, first 2000, last, exhaustion.
    const PROBES: [usize; 5] = [0, 1, 100, 5000, 15000];
    let mut cursor = FrameCursor::new();
    let mut matches_ok = total > 0;
    let mut last_frame: Option<PhysFrame> = None;
    let mut cursor_cycles: u64 = 0;
    let mut walked = 0usize;
    loop {
        let t = tsc();
        let f = cursor.next_frame(regions);
        cursor_cycles += tsc() - t;
        let Some(f) = f else { break };
        if walked < 2000 || PROBES.contains(&walked) {
            matches_ok &= reference_nth_frame(regions, walked) == Some(f);
        }
        last_frame = Some(f);
        walked += 1;
    }
    let exhausted_none = cursor.next_frame(regions).is_none();
    let count_ok = walked == total && exhausted_none && counted_agrees;
    matches_ok &= total > 0 && reference_nth_frame(regions, total - 1) == last_frame;
    matches_ok &= reference_nth_frame(regions, total).is_none();

    // Cost of the reference walk near the end of the pool.
    let t1 = tsc();
    let ref_last = reference_nth_frame(regions, total.saturating_sub(1));
    let ref_cycles = tsc() - t1;
    let _ = ref_last;

    let cursor_per_frame = if walked > 0 { cursor_cycles / walked as u64 } else { 0 };
    let cost_ok = cursor_per_frame < 5_000 && ref_cycles >= cursor_per_frame * 100;
    let ok = count_ok && matches_ok && cost_ok;
    let _ = writeln!(
        crate::serial(),
        "milestone 157: frame cursor self-test -- walked {walked} of {total} usable frames then None={exhausted_none} (count_ok={count_ok}) -- identical to reference nth(k) at probes {PROBES:?}, last index, past-the-end, and first 2000 (matches_ok={matches_ok}) -- cursor {cursor_per_frame} cycles/frame over the full walk vs reference nth({}) = {ref_cycles} cycles for ONE frame ({}x the cursor's per-frame cost) -- {}",
        total.saturating_sub(1),
        if cursor_per_frame > 0 { ref_cycles / cursor_per_frame } else { 0 },
        if ok { "PASS" } else { "FAIL" }
    );
    ok
}
