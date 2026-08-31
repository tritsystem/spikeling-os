// MILESTONE 58 test payload: a REAL, standalone, freestanding x86_64
// ELF64 executable (built with this project's own pinned Rust nightly
// toolchain, rustc --target x86_64-unknown-none + rust-lld, NOT hand-
// assembled) that calls the NEW EXECARGV syscall (16) with a real
// argv/envp array -- the actual "caller" half of Milestone 58's real
// argv/envp test. `tools/argvtarget_src` is the "receiver" half: this
// program exec()s straight into it (see TARGET_PATH below), and never
// returns to any code after that call on success -- real exec()
// semantics, the calling process's entire image is replaced.
//
// Syscall ABI (kernel/src/usertest.rs's syscall_dispatch, syscall 16):
//   rdi=path_ptr, rsi=path_len, rdx=argv_ptr, r10=argv_count,
//   r8=envp_ptr, r9=envp_count -- argv_ptr/envp_ptr each point to an
//   array of `#[repr(C)] struct ArgSpec { ptr: u64, len: u64 }`
//   entries, the SAME ptr+len idiom this kernel already uses for every
//   other string-bearing syscall argument (not NUL-terminated C
//   strings -- the kernel appends the actual NUL itself when it lays
//   the new process's stack out, see process::build_argv_envp_stack()).
#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

#[repr(C)]
struct ArgSpec {
    ptr: u64,
    len: u64,
}

const TARGET_PATH: &[u8] = b"argvtarget";
const ARG0: &[u8] = b"argvtarget";
const ARG1: &[u8] = b"hello";
const ARG2: &[u8] = b"world";
const ENV0: &[u8] = b"GREETING=hi";

const STARTUP_MSG: &[u8] = b"milestone 58: argvlauncher starting -- calling real EXECARGV with 3 argv + 1 envp entries\n";
const FALLBACK_MSG: &[u8] = b"milestone 58: argvlauncher fallback -- real EXECARGV failed, exec() never replaced this process\n";

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

/// syscall 16 = execargv(path_ptr, path_len, argv_ptr, argv_count,
/// envp_ptr, envp_count) -> u64::MAX on failure, never returns on
/// success (the calling process's own image is replaced -- exactly
/// like syscall 9's plain exec()).
unsafe fn exec_argv(path: &[u8], argv: &[ArgSpec], envp: &[ArgSpec]) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "int 0x80",
            inout("rax") 16u64 => ret,
            in("rdi") path.as_ptr() as u64,
            in("rsi") path.len() as u64,
            in("rdx") argv.as_ptr() as u64,
            in("r10") argv.len() as u64,
            in("r8") envp.as_ptr() as u64,
            in("r9") envp.len() as u64,
            out("rcx") _, out("r11") _,
            options(nostack)
        );
    }
    ret
}

#[unsafe(link_section = ".text.start")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    unsafe {
        w(STARTUP_MSG);

        let argv = [
            ArgSpec { ptr: ARG0.as_ptr() as u64, len: ARG0.len() as u64 },
            ArgSpec { ptr: ARG1.as_ptr() as u64, len: ARG1.len() as u64 },
            ArgSpec { ptr: ARG2.as_ptr() as u64, len: ARG2.len() as u64 },
        ];
        let envp = [ArgSpec { ptr: ENV0.as_ptr() as u64, len: ENV0.len() as u64 }];

        let _result = exec_argv(TARGET_PATH, &argv, &envp);
        // Reached ONLY on failure -- a real EXECARGV success never
        // returns here at all (control jumps straight into
        // argvtarget.elf's own _start instead).
        w(FALLBACK_MSG);
        exit(1);
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
