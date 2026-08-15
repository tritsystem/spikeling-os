//! MILESTONE 39: a real, minimal libc for spikeling-os -- the first
//! user program built against genuine, reusable Rust syscall wrappers
//! instead of every test program hand-encoding its own raw `int 0x80`
//! sequences (usertest::USER_PROGRAM, process::PROCESS_PROGRAM,
//! loader::FDTEST_PROGRAM, and the fork/exec test program before this
//! all did exactly that). This is the real dependency the README's own
//! stated chain names: "a minimal libc (userspace programs need this to
//! make POSIX-shaped calls at all)".
//!
//! Every wrapper here matches kernel/src/usertest.rs's syscall_dispatch
//! ABI EXACTLY (checked against that file directly, not assumed):
//!   0 = write(ptr, len)              -- no return value
//!   1 = exit(code) -- never returns
//!   2 = sbrk(size) -> ptr (u64::MAX on failure)
//!   3 = open(path_ptr, path_len) -> fd (u64::MAX on failure)
//!   4 = read(fd, buf_ptr, len) -> bytes_read (u64::MAX on failure)
//!   5 = fdwrite(fd, ptr, len) -> bytes_written (u64::MAX on failure)
//!   6 = close(fd) -> status (0=ok, 1=invalid fd, 2=persist failed)
//!   7 = fork() -> child_pid to parent, 0 to child (u64::MAX on failure)
//!   8 = wait(child_pid) -> reaped_pid (u64::MAX on failure)
//!   9 = exec(path_ptr, path_len) -- never returns on success
//!
//! Deliberately minimal, matching this whole project's own "small,
//! disclosed, real" scoping convention rather than a general libc:
//!   - no errno, no C ABI compatibility, no dynamic linking -- these are
//!     ordinary `unsafe extern "C"` Rust functions, called directly by
//!     Rust code linked into the same static binary.
//!   - `u64::MAX` is the one, uniform failure sentinel across every
//!     call, matching the kernel's own actual convention (checked
//!     against usertest.rs, not invented) -- no separate error-code
//!     namespace.
//!   - MILESTONE 51 closes the "no heap allocator wired to sbrk() yet"
//!     gap disclosed right here at Milestone 39: `malloc()`/`free()`
//!     below are a real, minimal, intrusive free-list allocator built
//!     entirely on `sys_sbrk()` -- see that section's own doc comment
//!     for the exact design and its own disclosed limitations
//!     (no coalescing).

#[inline(always)]
pub unsafe fn sys_write(ptr: u64, len: u64) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 0u64,
            in("rdi") ptr,
            in("rsi") len,
            out("rcx") _, out("r11") _,
            options(nostack)
        );
    }
}

#[inline(always)]
pub unsafe fn sys_exit(code: u64) -> ! {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") 1u64,
            in("rdi") code,
            options(nostack, noreturn)
        );
    }
}

#[inline(always)]
pub unsafe fn sys_sbrk(size: u64) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") 2u64 => ret,
            in("rdi") size,
            out("rcx") _, out("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn sys_open(path_ptr: u64, path_len: u64) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") 3u64 => ret,
            in("rdi") path_ptr,
            in("rsi") path_len,
            out("rcx") _, out("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn sys_read(fd: u64, buf_ptr: u64, len: u64) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") 4u64 => ret,
            in("rdi") fd,
            in("rsi") buf_ptr,
            in("rdx") len,
            out("rcx") _, out("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn sys_fdwrite(fd: u64, ptr: u64, len: u64) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") 5u64 => ret,
            in("rdi") fd,
            in("rsi") ptr,
            in("rdx") len,
            out("rcx") _, out("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn sys_close(fd: u64) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") 6u64 => ret,
            in("rdi") fd,
            out("rcx") _, out("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn sys_fork() -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") 7u64 => ret,
            out("rcx") _, out("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn sys_wait(child_pid: u64) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") 8u64 => ret,
            in("rdi") child_pid,
            out("rcx") _, out("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn sys_exec(path_ptr: u64, path_len: u64) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") 9u64 => ret,
            in("rdi") path_ptr,
            in("rsi") path_len,
            out("rcx") _, out("r11") _,
            options(nostack)
        );
    }
    ret
}

pub const SYSCALL_FAIL: u64 = u64::MAX;

// ---------------------------------------------------------------------
// MILESTONE 51: a real malloc()/free() on top of sys_sbrk() -- the
// "no heap allocator wired to sbrk() yet" gap this file's own top doc
// comment named as genuine next-layer work back at Milestone 39.
//
// A classic minimal intrusive free-list allocator. Every block (free
// or in-use) is preceded by an 8-byte header holding just its own
// USABLE payload size (not counting the header itself). While a block
// is FREE, the first 8 bytes of its own payload area double as the
// "next free block" pointer (0 = end of list) -- the standard space-
// saving trick (no separate free-list node storage needed), safe
// because a free block's payload is otherwise unused and every block's
// payload is guaranteed to be at least MALLOC_MIN_PAYLOAD (8) bytes,
// so those 8 bytes always exist to write into.
//
// malloc() walks the free list first-fit; a fitting block big enough
// to leave a genuinely independently-usable remainder (header +
// MALLOC_MIN_PAYLOAD or more) gets SPLIT, with the leftover pushed back
// onto the free list as its own block -- real reuse of partial space,
// not just whole-block matching. No fitting free block falls back to
// growing the heap for real via sys_sbrk().
//
// Real, disclosed scope-cut, same "small, real, honest" convention as
// fs.rs's own allocator (Milestone 22's disclosed no-compaction gap):
// NO coalescing of adjacent freed blocks -- free() only ever pushes its
// block back onto the free list head, never merges it with a
// physically-adjacent free neighbour. On this process's fixed 16 KiB
// sbrk-backed heap (Milestone 33), a long enough alloc/free/alloc-
// bigger churn pattern can therefore exhaust the heap with genuinely
// free-but-fragmented memory before HEAP_SIZE is reached -- a real,
// present limitation, not hidden behind untested code. Not thread-safe
// (fine: one process, one single-threaded execution context -- this
// kernel's user-mode model has no threads yet).
//
// Alignment invariant this relies on (not separately enforced, just
// true by construction): HEAP_START is page-aligned (4096, an 8-byte
// multiple), and every sys_sbrk() request this file ever issues is
// `MALLOC_HEADER_SIZE (8) + an 8-byte-aligned size` -- itself always an
// 8-byte multiple -- so the heap's bump pointer, and therefore every
// block header address, stays 8-byte aligned inductively from boot.

const MALLOC_HEADER_SIZE: u64 = 8;
const MALLOC_MIN_PAYLOAD: u64 = 8; // must hold a "next" pointer while free
const MALLOC_ALIGN: u64 = 8;

static FREE_LIST_HEAD: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

#[inline]
fn align_up(n: u64, align: u64) -> u64 {
    (n + align - 1) & !(align - 1)
}

unsafe fn block_size(block_addr: u64) -> u64 {
    unsafe { core::ptr::read(block_addr as *const u64) }
}

unsafe fn set_block_size(block_addr: u64, size: u64) {
    unsafe { core::ptr::write(block_addr as *mut u64, size) };
}

unsafe fn block_next(block_addr: u64) -> u64 {
    unsafe { core::ptr::read((block_addr + MALLOC_HEADER_SIZE) as *const u64) }
}

unsafe fn set_block_next(block_addr: u64, next: u64) {
    unsafe { core::ptr::write((block_addr + MALLOC_HEADER_SIZE) as *mut u64, next) };
}

/// Real malloc(): first-fit over the free list, splitting a block that
/// has enough spare room left over for a new, independently-usable
/// free block of its own; falls back to growing the heap via
/// `sys_sbrk()` when no free block fits. Returns 0 (an address no valid
/// allocation can ever have -- page 0 is never mapped in this kernel)
/// on failure -- the ordinary C-`malloc`-shaped "null on failure" a
/// caller actually expects, distinct from `sys_sbrk`'s own raw
/// `SYSCALL_FAIL` sentinel.
pub unsafe fn malloc(requested: u64) -> u64 {
    use core::sync::atomic::Ordering;
    if requested == 0 {
        return 0;
    }
    let size = align_up(requested.max(MALLOC_MIN_PAYLOAD), MALLOC_ALIGN);

    let mut prev: u64 = 0;
    let mut cur = FREE_LIST_HEAD.load(Ordering::Relaxed);
    while cur != 0 {
        let cur_size = unsafe { block_size(cur) };
        if cur_size >= size {
            let next = unsafe { block_next(cur) };
            // splice `cur` out of the free list
            if prev == 0 {
                FREE_LIST_HEAD.store(next, Ordering::Relaxed);
            } else {
                unsafe { set_block_next(prev, next) };
            }

            // split off a fresh free block if there's real, independently
            // usable room left over
            let remainder = cur_size - size;
            if remainder >= MALLOC_HEADER_SIZE + MALLOC_MIN_PAYLOAD {
                let new_free = cur + MALLOC_HEADER_SIZE + size;
                let new_free_payload = remainder - MALLOC_HEADER_SIZE;
                unsafe { set_block_size(new_free, new_free_payload) };
                let head = FREE_LIST_HEAD.load(Ordering::Relaxed);
                unsafe { set_block_next(new_free, head) };
                FREE_LIST_HEAD.store(new_free, Ordering::Relaxed);
                unsafe { set_block_size(cur, size) };
            }
            // else: hand out the whole block as-is -- real, honest
            // internal fragmentation, disclosed above, not hidden.

            return cur + MALLOC_HEADER_SIZE;
        }
        prev = cur;
        cur = unsafe { block_next(cur) };
    }

    // No free block fit -- grow the heap for real via sbrk().
    let block = unsafe { sys_sbrk(MALLOC_HEADER_SIZE + size) };
    if block == SYSCALL_FAIL {
        return 0;
    }
    unsafe { set_block_size(block, size) };
    block + MALLOC_HEADER_SIZE
}

/// Real free(): pushes the block back onto the head of the free list --
/// see this section's top doc comment for why no coalescing happens. A
/// null (0) pointer is a documented no-op, matching real `free(NULL)`
/// semantics.
pub unsafe fn free(ptr: u64) {
    use core::sync::atomic::Ordering;
    if ptr == 0 {
        return;
    }
    let block = ptr - MALLOC_HEADER_SIZE;
    let head = FREE_LIST_HEAD.load(Ordering::Relaxed);
    unsafe { set_block_next(block, head) };
    FREE_LIST_HEAD.store(block, Ordering::Relaxed);
}
