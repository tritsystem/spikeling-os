// MILESTONE 44 test payload: a REAL, standalone, freestanding x86_64
// ELF64 executable whose e_entry does NOT equal
// spikeling-os's usertest::USER_CODE_ADDR (0x0000_5555_5000_0000) --
// the whole point of this milestone. A single PT_LOAD segment is placed
// at 0x0000_5555_5000_3000 (three pages past USER_CODE_ADDR, still
// within the same p4 index the kernel's per-process private PML4 slot
// covers) via the linker script alongside this file, and e_entry is set
// to that same address. If the kernel's ELF loader still silently
// assumed entry == USER_CODE_ADDR anywhere, this would either be
// rejected outright or jump to the wrong address and crash instead of
// reaching the real write+exit below.
//
// Syscall ABI matches spikeling-os's own (kernel/src/usertest.rs):
// int 0x80, rax = syscall number, rax=0 is write(rdi=ptr, rsi=len),
// rax=1 is exit.
#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

// MILESTONE 45: fixed a real, genuine off-by-one in this Milestone 44
// staged payload -- the declared array size (71) didn't match the
// actual string literal's length (70), which fails to compile
// (E0308: "expected an array with a size of 71, found one with a size
// of 70"). Found while actually building this payload for real for the
// first time (Milestone 44 left it staged but never build-verified --
// its own doc comment says so) rather than trusting it compiled.
#[unsafe(link_section = ".rodata.entry")]
static MESSAGE: [u8; 70] =
    *b"hello from a non-USER_CODE_ADDR entry point -- milestone 44 confirmed!";

#[unsafe(link_section = ".text.altentry")]
#[unsafe(no_mangle)]
pub extern "C" fn alt_entry() -> ! {
    let ptr = MESSAGE.as_ptr() as u64;
    let len = MESSAGE.len() as u64;
    unsafe {
        asm!(
            "int 0x80",
            in("rax") 0u64,
            in("rdi") ptr,
            in("rsi") len,
            options(nostack)
        );
        asm!(
            "int 0x80",
            in("rax") 1u64,
            options(nostack, noreturn)
        );
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
