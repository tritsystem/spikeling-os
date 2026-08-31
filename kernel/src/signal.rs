//! MILESTONE 60: real POSIX signal-number constants -- the OTHER Tier 1
//! gap Milestone 58's own closing doc comment (and errno.rs's own top
//! doc comment, which explicitly sized this exact work: "a whole new
//! control-transfer mechanism: a real trampoline into a user-registered
//! handler, a `sigreturn`-equivalent to unwind back out of it, and
//! per-process handler-table bookkeeping") named as the bigger, riskier
//! lift deferred past Milestone 59's `errno` work. This module holds
//! only the real, standard signal NUMBERS (checked against the actual
//! POSIX/Linux x86_64 `signal.h` values, not invented -- a program
//! written against a real libc's `signal.h` would see the SAME numbers
//! this kernel now recognizes); the actual mechanism -- registration,
//! dispatch, and unwind -- lives in `process.rs`
//! (`sigaction()`/`raise_signal()`/`take_deliverable_signal()`/
//! `stash_signal_context()`/`take_saved_signal_context()`) and
//! `usertest.rs` (the real trampoline-injection + `SIGRETURN` syscall
//! dispatch arms).
//!
//! `NSIG` = 32, matching real Linux (`_NSIG`): signal numbers `1..=31`
//! are structurally valid (`Process::signal_handlers`'s own array bound);
//! `0` is never a real signal number, the same "0 is a safe,
//! never-otherwise-valid sentinel" convention `errno.rs`'s own `0`
//! ("no error recorded") already establishes. Deliberately a small, real
//! subset of names (not a full `signal.h` transcription) -- exactly the
//! numbers this milestone's own self-test and doc comments need to name,
//! same "small, real, honest" scoping discipline `errno.rs` already set.
#![allow(dead_code)]

pub const NSIG: usize = 32;

pub const SIGHUP: u8 = 1;
pub const SIGINT: u8 = 2;
pub const SIGQUIT: u8 = 3;
pub const SIGILL: u8 = 4;
pub const SIGTRAP: u8 = 5;
pub const SIGABRT: u8 = 6;
pub const SIGBUS: u8 = 7;
pub const SIGFPE: u8 = 8;
/// Real POSIX rule, actually enforced (not just documented): `process::
/// sigaction()` refuses to register a handler for this number, and
/// `process::raise_signal()` routes a `SIGKILL` request to the
/// PRE-EXISTING Milestone 41 `kill()` unconditional-terminate path
/// instead of this milestone's new handler-dispatch mechanism -- SIGKILL
/// was already real before this milestone; it is not reimplemented, just
/// recognized by its real number here so callers (and this milestone's
/// own doc comments) can name it precisely instead of a bare `9`.
pub const SIGKILL: u8 = 9;
pub const SIGUSR1: u8 = 10;
pub const SIGSEGV: u8 = 11;
pub const SIGUSR2: u8 = 12;
pub const SIGPIPE: u8 = 13;
pub const SIGALRM: u8 = 14;
pub const SIGTERM: u8 = 15;

/// Human-readable name for a real signal number, for serial-log messages
/// only (never itself returned to a caller) -- e.g. `name(10) ==
/// "SIGUSR1"`. `"SIGUNKNOWN"` for 0 or any number this module doesn't
/// name above -- a number in `1..=31` this module hasn't named is still
/// a real, structurally valid signal number (`NSIG`/`Process::
/// signal_handlers`'s own bound doesn't care), just unlabeled here.
pub fn name(value: u8) -> &'static str {
    match value {
        0 => "0 (no signal)",
        SIGHUP => "SIGHUP",
        SIGINT => "SIGINT",
        SIGQUIT => "SIGQUIT",
        SIGILL => "SIGILL",
        SIGTRAP => "SIGTRAP",
        SIGABRT => "SIGABRT",
        SIGBUS => "SIGBUS",
        SIGFPE => "SIGFPE",
        SIGKILL => "SIGKILL",
        SIGUSR1 => "SIGUSR1",
        SIGSEGV => "SIGSEGV",
        SIGUSR2 => "SIGUSR2",
        SIGPIPE => "SIGPIPE",
        SIGALRM => "SIGALRM",
        SIGTERM => "SIGTERM",
        _ => "SIGUNKNOWN",
    }
}
