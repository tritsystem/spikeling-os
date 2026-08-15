// MILESTONE 51 test payload: a REAL, standalone, freestanding x86_64
// ELF64 executable (built with this project's own pinned Rust
// toolchain, rustc --target x86_64-unknown-none + rust-lld, same recipe
// as tools/libc_test_src's own Milestone 39 build -- NOT hand-
// assembled) exercising the real malloc()/free() added to libc.rs at
// Milestone 51. This program's own libc.rs is a copy of
// tools/libc_test_src/libc.rs (these standalone single-file `rustc`
// builds have no shared-crate mechanism across tool directories, same
// reason pipetest_src/testelf_src each carry their own self-contained
// source rather than importing a sibling directory's file) -- keep both
// copies' malloc()/free() in sync if either changes.
//
// Every check below is a HAND-COMPUTED PREDICTION made before writing
// this program's own logic (documented inline), then verified against
// this allocator's real, live behavior -- same discipline as e.g.
// Milestone 10's STDP delta / Milestone 38's compression-ratio checks.
//
//   1. malloc(64) -> ptr_a, malloc(32) -> ptr_b. Free list starts empty,
//      so BOTH come from real sys_sbrk() growth, back to back:
//      ptr_b MUST equal ptr_a + 64 (ptr_a's own payload) + 8 (ptr_b's
//      own header) = ptr_a + 72, exactly.
//   2. Writing a 0xBB pattern into ptr_b's whole payload must NOT
//      disturb ptr_a's still-resident 0xAA pattern -- real proof the
//      two blocks don't overlap, not just that the addresses differ.
//   3. free(ptr_a), then malloc(64) again -> ptr_c. The free list holds
//      exactly one block, an EXACT size match (64), so ptr_c MUST equal
//      ptr_a exactly (real free-list reuse, not just always bumping the
//      heap further).
//   4. malloc(64) -> ptr_d (free list empty again after step 3 consumed
//      it whole), then free(ptr_d), then malloc(16) -> ptr_e. The freed
//      64-byte block is bigger than the 16 requested, with room left
//      over for an independent free block (64-16-8(header)=40 >=
//      8+8) -- so this MUST split: ptr_e == ptr_d exactly (the shrunk
//      front of the same block), and the leftover becomes a new
//      40-byte-payload free block starting right after ptr_e's own
//      16-byte payload + its header, i.e. at ptr_d + 16 + 8 = ptr_d+24.
//   5. malloc(40) -> ptr_f must then hand out EXACTLY that split
//      remainder (40 matches its payload size exactly, no further
//      split) -- ptr_f MUST equal ptr_d + 24 exactly.
//   6. Writing into ptr_e's and ptr_f's payloads independently must not
//      corrupt each other -- real proof the split produced two genuinely
//      separate, correctly-sized regions, not overlapping garbage.
#![no_std]
#![no_main]

mod libc;
use libc::*;

const STARTUP_MSG: &[u8] = b"milestone 51: malloctest starting -- real malloc()/free() over sys_sbrk()\n";
const NEWLINE: &[u8] = b"\n";
const PASS: &[u8] = b"PASS";
const FAIL: &[u8] = b"FAIL";

unsafe fn w(bytes: &[u8]) {
    unsafe { sys_write(bytes.as_ptr() as u64, bytes.len() as u64) };
}

unsafe fn write_hex_line(label: &[u8], val: u64) {
    unsafe { w(label) };
    let hex_digits = b"0123456789abcdef";
    let mut buf = [0u8; 18];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..16 {
        let shift = (15 - i) * 4;
        let nibble = ((val >> shift) & 0xf) as usize;
        buf[2 + i] = hex_digits[nibble];
    }
    unsafe { w(&buf) };
    unsafe { w(NEWLINE) };
}

unsafe fn write_check(label: &[u8], ok: bool) {
    unsafe { w(label) };
    unsafe { w(if ok { PASS } else { FAIL }) };
    unsafe { w(b" ") };
}

unsafe fn fill(ptr: u64, byte: u8, len: u64) {
    for i in 0..len {
        unsafe { core::ptr::write((ptr + i) as *mut u8, byte) };
    }
}

unsafe fn all_bytes_equal(ptr: u64, byte: u8, len: u64) -> bool {
    for i in 0..len {
        if unsafe { core::ptr::read((ptr + i) as *const u8) } != byte {
            return false;
        }
    }
    true
}

#[unsafe(link_section = ".text.start")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    unsafe {
        w(STARTUP_MSG);

        // --- checks 1-2: two fresh allocations, no overlap ---
        let ptr_a = malloc(64);
        fill(ptr_a, 0xAA, 64);
        let ptr_b = malloc(32);
        fill(ptr_b, 0xBB, 32);
        write_hex_line(b"  ptr_a=", ptr_a);
        write_hex_line(b"  ptr_b=", ptr_b);

        let distinct_ok = ptr_a != 0 && ptr_b != 0 && ptr_b == ptr_a + 64 + 8;
        let integrity_ok = all_bytes_equal(ptr_a, 0xAA, 64) && all_bytes_equal(ptr_b, 0xBB, 32);

        // --- check 3: free + realloc same size -> exact reuse ---
        free(ptr_a);
        let ptr_c = malloc(64);
        write_hex_line(b"  ptr_c=", ptr_c);
        let reuse_ok = ptr_c == ptr_a;
        fill(ptr_c, 0xCC, 64);
        let reuse_write_ok = all_bytes_equal(ptr_c, 0xCC, 64);

        // --- checks 4-6: free + smaller realloc -> real split ---
        let ptr_d = malloc(64);
        write_hex_line(b"  ptr_d=", ptr_d);
        free(ptr_d);
        let ptr_e = malloc(16);
        write_hex_line(b"  ptr_e=", ptr_e);
        let split_reuse_ok = ptr_e == ptr_d;

        let ptr_f = malloc(40);
        write_hex_line(b"  ptr_f=", ptr_f);
        let split_remainder_ok = ptr_f == ptr_d + 24;

        fill(ptr_e, 0xEE, 16);
        fill(ptr_f, 0xFF, 40);
        let split_integrity_ok = all_bytes_equal(ptr_e, 0xEE, 16) && all_bytes_equal(ptr_f, 0xFF, 40);

        write_check(b"distinct=", distinct_ok);
        write_check(b"integrity=", integrity_ok);
        write_check(b"reuse=", reuse_ok && reuse_write_ok);
        write_check(b"split_reuse=", split_reuse_ok);
        write_check(b"split_remainder=", split_remainder_ok);
        write_check(b"split_integrity=", split_integrity_ok);

        let overall = distinct_ok
            && integrity_ok
            && reuse_ok
            && reuse_write_ok
            && split_reuse_ok
            && split_remainder_ok
            && split_integrity_ok;
        w(b"OVERALL=");
        w(if overall { PASS } else { FAIL });
        w(NEWLINE);

        sys_exit(if overall { 0 } else { 1 });
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
