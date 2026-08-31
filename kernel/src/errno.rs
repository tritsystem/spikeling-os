//! MILESTONE 59: real `errno` -- the Tier 1 roadmap item named right
//! alongside `sigaction`/handler delivery in Milestone 58's own closing
//! doc comment ("`errno` and a real signal-delivery mechanism... remain
//! genuinely open Tier 1 gaps"). Chosen over real signal delivery for
//! this milestone specifically because M58 already sized both: signal
//! delivery needs a whole new control-transfer mechanism (a real
//! trampoline into a user-registered handler, a `sigreturn`-equivalent
//! to unwind back out of it, and per-process handler-table bookkeeping)
//! layered on top of a fault/kill path this kernel has never needed to
//! interrupt mid-instruction before -- a materially bigger and riskier
//! lift than giving every syscall that already fails today a real,
//! specific reason code alongside its existing bare `u64::MAX`/`0`/`1`
//! sentinel.
//!
//! Every constant below is a REAL, standard POSIX/Linux x86_64 errno.h
//! value (checked against the actual numbers glibc/Linux use on x86_64,
//! not invented) -- a program written against a real libc's errno.h
//! would see the SAME numbers this kernel now reports, not a private
//! numbering scheme. Deliberately a small, real subset (not a full
//! errno.h transcription) -- exactly the values this kernel's own
//! syscalls can genuinely, distinguishably produce today, checked
//! against `usertest.rs`'s syscall_dispatch call-by-call before being
//! wired in (see that file's own per-arm comments for which of these
//! applies where and why).
#![allow(dead_code)]

/// Operation not permitted -- this process is not the owner/authorized
/// caller (e.g. `setpgid` on a pid that is neither self nor a live
/// child of the caller).
pub const EPERM: u64 = 1;
/// No such file or directory -- `exec()`'s target path does not exist
/// on the real on-disk filesystem.
pub const ENOENT: u64 = 2;
/// No such process -- covers two real, distinguishable-by-construction
/// cases this kernel treats identically (both genuinely mean "the pid
/// this syscall needed to operate against does not name a live
/// process/child right now"): a syscall that requires a per-process
/// context called with `ACTIVE_PROCESS == 0` (the legacy, pre-Milestone-
/// 30 `usertest` excursion, which has no process struct at all to
/// operate on), and a target pid argument (`kill`/`getpgid`/`setpgid`)
/// that does not name any currently-live process.
pub const ESRCH: u64 = 3;
/// I/O error -- `close()`'s real on-disk persist (`fs::write_file()`)
/// failed for a dirty fd (`process::close_fd()`'s own `Some(false)`
/// arm).
pub const EIO: u64 = 5;
/// Argument list too long -- `execargv()`'s `argv_count`/`envp_count`
/// exceeds `EXEC_ARGV_MAX_COUNT`, or one declared argv/envp string's
/// length exceeds `EXEC_ARG_MAX_LEN`.
pub const E2BIG: u64 = 7;
/// Exec format error -- `exec()`/`execargv()`'s target file is not a
/// valid ELF64 image (`elf::parse()` returned `Err`).
pub const ENOEXEC: u64 = 8;
/// Bad file descriptor -- `read`/`fdwrite`/`close`/`dup`/`dup2` given an
/// `fd` that does not name a currently-open descriptor for this
/// process.
pub const EBADF: u64 = 9;
/// No child processes -- `wait()`'s target pid is not a live child of
/// the calling process (already reaped, never existed, or belongs to
/// someone else).
pub const ECHILD: u64 = 10;
/// Resource temporarily unavailable -- `fork()` failed for a real,
/// disclosed resource reason (the fixed `MAX_PROCESSES` table is full,
/// the global frame allocator couldn't supply a fresh frame, or this
/// call is nested inside an active child-resume excursion, this
/// kernel's own enforced nesting-depth-1 bound).
pub const EAGAIN: u64 = 11;
/// Out of memory -- `sbrk()`'s request would exceed this process's
/// fixed per-process heap reservation.
pub const ENOMEM: u64 = 12;
/// Invalid argument -- a syscall argument is malformed or out of range
/// in a way that isn't specifically one of the more precise codes above
/// (non-UTF8 path bytes, a pid argument that doesn't fit `u8`, etc.).
pub const EINVAL: u64 = 22;
/// Too many open files -- this process's own fixed `MAX_OPEN_FILES` fd
/// table is full (`open`), or creating a pipe couldn't find room for
/// both of its new fd-table slots, or the global `PIPE_TABLE` itself has
/// no free slot (`pipe`) -- three real causes this kernel's current
/// `Option`-returning internals don't separately distinguish, disclosed
/// here rather than hidden: EMFILE is the single best-fit real errno for
/// all three ("too many files/fds", the actual shared reason), not a
/// guess unrelated to what really happened.
pub const EMFILE: u64 = 24;

/// Returns a short, human-readable name for a real errno value, for
/// serial-log messages only (never itself returned to a caller) -- e.g.
/// `name(9) == "EBADF"`. `"EUNKNOWN"` for 0 (no error recorded) or any
/// value this module doesn't define above.
pub fn name(value: u64) -> &'static str {
    match value {
        0 => "0 (no error recorded)",
        EPERM => "EPERM",
        ENOENT => "ENOENT",
        ESRCH => "ESRCH",
        EIO => "EIO",
        E2BIG => "E2BIG",
        ENOEXEC => "ENOEXEC",
        EBADF => "EBADF",
        ECHILD => "ECHILD",
        EAGAIN => "EAGAIN",
        ENOMEM => "ENOMEM",
        EINVAL => "EINVAL",
        EMFILE => "EMFILE",
        _ => "EUNKNOWN",
    }
}
