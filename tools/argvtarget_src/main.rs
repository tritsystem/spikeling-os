// MILESTONE 58 test payload: a REAL, standalone, freestanding x86_64
// ELF64 executable (built with this project's own pinned Rust nightly
// toolchain, rustc --target x86_64-unknown-none + rust-lld, NOT hand-
// assembled -- same recipe as every prior externally-built ELF test
// payload in tools/) that reads argc/argv/envp directly off its OWN
// initial stack, per the REAL x86_64 SysV process-entry contract:
// [rsp] = argc, [rsp+8 .. rsp+8+argc*8) = argv[0..argc] (real pointers
// into this same stack page), one NULL (0) terminator, then envp[0..]
// pointers, a second NULL terminator. This is exactly the contract a
// genuine kernel's `execve()` establishes for a real ELF's `_start`
// (this is what glibc's own `_start` assumes too) -- not a spikeling-os
// -specific convention invented for this milestone.
//
// `_start` is `#[unsafe(naked)]` (same mechanism as
// kernel/src/usertest.rs's own `syscall_entry()`) precisely so it can
// read `rsp` BEFORE any Rust-generated prologue could disturb it: three
// `lea`/`mov` instructions pull argc/argv/envp straight off the
// hardware-set RSP into the SysV C calling-convention registers
// (rdi/rsi/rdx) and tail-calls into ordinary Rust (`real_main`) --
// argv's own real length (argc) is used to compute exactly where
// envp begins (right after argv's own NULL terminator), not a
// hardcoded offset.
//
// The pass/fail evidence is a set of HAND-COMPUTED PREDICTIONS (same
// discipline as tools/malloctest_src/main.rs's own checks) against the
// exact argv/envp tools/argvlauncher_src/main.rs is known to send:
// argv = ["argvtarget", "hello", "world"], envp = ["GREETING=hi"].
#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

#[unsafe(link_section = ".text.start")]
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "mov rdi, [rsp]",     // argc
        "lea rsi, [rsp+8]",   // argv -- array of argc real u64 pointers, NULL-terminated
        "lea rax, [rdi*8+8]", // (argc+1)*8 == byte size of argv[] including its NULL terminator
        "lea rdx, [rsi+rax]", // envp starts immediately after argv's own NULL terminator
        "call {main}",
        "2:",
        "hlt",
        "jmp 2b",
        main = sym real_main,
    );
}

unsafe fn w(bytes: &[u8]) {
    unsafe {
        asm!(
            "int 0x80",
            in("rax") 0u64,
            in("rdi") bytes.as_ptr() as u64,
            in("rsi") bytes.len() as u64,
            options(nostack)
        );
    }
}

unsafe fn exit(code: u64) -> ! {
    unsafe {
        asm!(
            "int 0x80",
            in("rax") 1u64,
            in("rdi") code,
            options(nostack, noreturn)
        );
    }
}

/// Real strlen over a raw pointer -- scans for the NUL terminator this
/// milestone's own kernel-side `build_argv_envp_stack()` always appends
/// after each string, capped defensively (4096, this process's own
/// whole stack page size) so a corrupt/missing terminator can never
/// spin forever.
unsafe fn strlen(ptr: u64) -> u64 {
    let mut n: u64 = 0;
    while n < 4096 {
        if unsafe { core::ptr::read((ptr + n) as *const u8) } == 0 {
            break;
        }
        n += 1;
    }
    n
}

unsafe fn write_cstr(ptr: u64) {
    let len = unsafe { strlen(ptr) };
    unsafe { w(core::slice::from_raw_parts(ptr as *const u8, len as usize)) };
}

unsafe fn streq(ptr: u64, expected: &[u8]) -> bool {
    let len = unsafe { strlen(ptr) };
    if len as usize != expected.len() {
        return false;
    }
    for (i, &e) in expected.iter().enumerate() {
        if unsafe { core::ptr::read((ptr + i as u64) as *const u8) } != e {
            return false;
        }
    }
    true
}

// Deliberately raw-pointer-based, not `buf[i] = ...` array indexing:
// `i` here is only boundable by a runtime `while val > 0` loop, which
// LLVM cannot prove in-bounds at this crate's `opt-level=z` -- a real
// slice-index bounds check pulls in `core::panicking::panic_bounds_
// check`, which formats its message via `core::fmt::Display for u64`,
// which (confirmed by an actual failed link here during this
// milestone's own build) needs GOT-relative relocations the precompiled
// `x86_64-unknown-none` core distributed via rustup cannot satisfy
// together with this program's own `-C code-model=large` (same
// constraint tools/malloctest_src's own hex-printer avoids by using a
// PROVABLY in-bounds compile-time-constant loop range instead). Raw
// pointer writes have no such check at all -- correct AND avoids the
// dependency entirely, not a workaround for a bug in the logic itself.
unsafe fn write_dec(mut val: u64) {
    let mut buf = [0u8; 20];
    let ptr = buf.as_mut_ptr();
    if val == 0 {
        unsafe { w(b"0") };
        return;
    }
    let mut i: isize = 20;
    while val > 0 {
        i -= 1;
        unsafe { core::ptr::write(ptr.offset(i), b'0' + (val % 10) as u8) };
        val /= 10;
    }
    let len = (20 - i) as usize;
    unsafe { w(core::slice::from_raw_parts(ptr.offset(i), len)) };
}

unsafe fn write_check(label: &[u8], ok: bool) {
    unsafe {
        w(label);
        w(if ok { PASS } else { FAIL });
        w(b" ");
    }
}

const STARTUP: &[u8] = b"milestone 58: argvtarget starting -- reading real argv/envp off the real SysV process-entry stack\n";
const PASS: &[u8] = b"PASS";
const FAIL: &[u8] = b"FAIL";
const NL: &[u8] = b"\n";

extern "C" fn real_main(argc: u64, argv: *const u64, envp: *const u64) -> ! {
    unsafe {
        w(STARTUP);

        w(b"  argc=");
        write_dec(argc);
        w(NL);
        for i in 0..argc {
            let p = core::ptr::read(argv.wrapping_add(i as usize));
            w(b"  argv[");
            write_dec(i);
            w(b"]=");
            write_cstr(p);
            w(NL);
        }

        let mut envc: u64 = 0;
        loop {
            let p = core::ptr::read(envp.wrapping_add(envc as usize));
            if p == 0 {
                break;
            }
            w(b"  envp[");
            write_dec(envc);
            w(b"]=");
            write_cstr(p);
            w(NL);
            envc += 1;
        }
        w(b"  envc=");
        write_dec(envc);
        w(NL);

        // Hand-computed predictions against the exact argv/envp
        // tools/argvlauncher_src/main.rs is known to send -- see that
        // file's own ARG0/ARG1/ARG2/ENV0 constants.
        let argc_ok = argc == 3;
        let argv0_ok = argc_ok && streq(core::ptr::read(argv.wrapping_add(0)), b"argvtarget");
        let argv1_ok = argc_ok && streq(core::ptr::read(argv.wrapping_add(1)), b"hello");
        let argv2_ok = argc_ok && streq(core::ptr::read(argv.wrapping_add(2)), b"world");
        let envc_ok = envc == 1;
        let envp0_ok = envc_ok && streq(core::ptr::read(envp.wrapping_add(0)), b"GREETING=hi");

        write_check(b"argc=3: ", argc_ok);
        write_check(b"argv0=argvtarget: ", argv0_ok);
        write_check(b"argv1=hello: ", argv1_ok);
        write_check(b"argv2=world: ", argv2_ok);
        write_check(b"envc=1: ", envc_ok);
        write_check(b"envp0=GREETING=hi: ", envp0_ok);

        let overall = argc_ok && argv0_ok && argv1_ok && argv2_ok && envc_ok && envp0_ok;
        w(b"OVERALL=");
        w(if overall { PASS } else { FAIL });
        w(NL);

        exit(if overall { 0 } else { 1 });
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
