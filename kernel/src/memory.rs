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
            free_list: Vec::new(),
        }
    }

    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> {
        self.memory_regions
            .iter()
            .filter(|r| r.kind == MemoryRegionKind::Usable)
            .map(|r| r.start..r.end)
            .flat_map(|r| r.step_by(4096))
            .map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
    }

    /// MILESTONE 54: real count of frames currently sitting on the free
    /// list, waiting to be reused -- exposed so a self-test can prove
    /// reclamation actually happened (a real count change), not just
    /// that `deallocate_frame()` was called without erroring.
    pub fn free_list_len(&self) -> usize {
        self.free_list.len()
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        // MILESTONE 54: check the free list first -- a real reclaimed
        // frame is exactly as usable as a never-before-allocated one,
        // and preferring it keeps physical memory usage bounded across
        // repeated fork()/exit()/exec() cycles instead of only ever
        // growing.
        if let Some(frame) = self.free_list.pop() {
            return Some(frame);
        }
        let frame = self.usable_frames().nth(self.next);
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
