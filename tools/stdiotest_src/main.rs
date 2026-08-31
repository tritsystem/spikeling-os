// MILESTONE 61 test payload: a REAL, standalone, freestanding x86_64
// ELF64 executable (built with this project's own pinned Rust nightly
// toolchain, rustc --target x86_64-unknown-none + rust-lld, same recipe
// as tools/malloctest_src's own Milestone 51 build -- NOT hand-
// assembled) exercising the real string.h + buffered stdio added to
// libc.rs at this milestone. This program's own libc.rs is a copy of
// tools/libc_test_src/libc.rs (these standalone single-file `rustc`
// builds have no shared-crate mechanism across tool directories, same
// reason every other tools/*_src program carries its own self-contained
// copy) -- keep both copies in sync if either changes.
//
// Every check below is a HAND-COMPUTED PREDICTION made before writing
// this program's own logic (documented inline), then verified against
// the real, live behavior of string.h/stdio -- same discipline as
// malloctest_src/main.rs's own free-list address-arithmetic predictions.
#![no_std]
#![no_main]

mod libc;
use libc::*;

unsafe fn w(bytes: &[u8]) {
    unsafe { sys_write(bytes.as_ptr() as u64, bytes.len() as u64) };
}

unsafe fn write_check(label: &[u8], ok: bool) {
    unsafe { w(label) };
    unsafe { w(if ok { b"PASS" } else { b"FAIL" }) };
    unsafe { w(b" ") };
}

unsafe fn write_u64_line(label: &[u8], val: u64) {
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
    unsafe { w(b"\n") };
}

unsafe fn bytes_equal(ptr: u64, expected: &[u8]) -> bool {
    unsafe { memcmp(ptr, expected.as_ptr() as u64, expected.len() as u64) == 0 }
}

#[unsafe(link_section = ".text.start")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    unsafe {
        w(b"milestone 61: stdiotest starting -- real string.h + buffered stdio\n");

        // ===================================================================
        // PART 1: string.h -- hand-computed predictions for every function.
        // ===================================================================

        // --- memcpy: dest gets an exact copy, source untouched, returns dest ---
        const SRC_ABCDE: &[u8] = b"ABCDE";
        let mc_dest = malloc(5);
        let mc_ret = memcpy(mc_dest, SRC_ABCDE.as_ptr() as u64, 5);
        let memcpy_ok = mc_ret == mc_dest && bytes_equal(mc_dest, SRC_ABCDE);
        write_check(b"memcpy=", memcpy_ok);

        // --- memset: every byte becomes 0x7A, returns dest ---
        let ms_dest = malloc(8);
        let ms_ret = memset(ms_dest, 0x7A, 8);
        let mut memset_ok = ms_ret == ms_dest;
        for i in 0..8u64 {
            if core::ptr::read((ms_dest + i) as *const u8) != 0x7A {
                memset_ok = false;
            }
        }
        write_check(b"memset=", memset_ok);

        // --- memmove: real overlap (dest > src), backward-copy-correct case.
        // buf = "0123456789" (10 bytes), memmove(buf+2, buf, 6) -- HAND-
        // COMPUTED expected result "0101234589" (see libc.rs's own memmove
        // doc comment for why a naive forward copy would corrupt this).
        let mm_buf = malloc(10);
        const DIGITS: &[u8] = b"0123456789";
        let digits_src = DIGITS.as_ptr() as u64;
        for i in 0..10u64 {
            let byte = core::ptr::read((digits_src + i) as *const u8);
            core::ptr::write((mm_buf + i) as *mut u8, byte);
        }
        let mm_ret = memmove(mm_buf + 2, mm_buf, 6);
        const MM_EXPECTED: &[u8] = b"0101234589";
        let memmove_ok = mm_ret == mm_buf + 2 && bytes_equal(mm_buf, MM_EXPECTED);
        write_check(b"memmove=", memmove_ok);

        // --- memcmp: signed first-difference, matching real C sign convention ---
        const CMP_ABC: &[u8] = b"ABC";
        const CMP_ABD: &[u8] = b"ABD";
        let memcmp_eq = memcmp(CMP_ABC.as_ptr() as u64, CMP_ABC.as_ptr() as u64, 3) == 0;
        let memcmp_lt = memcmp(CMP_ABC.as_ptr() as u64, CMP_ABD.as_ptr() as u64, 3) < 0;
        let memcmp_gt = memcmp(CMP_ABD.as_ptr() as u64, CMP_ABC.as_ptr() as u64, 3) > 0;
        write_check(b"memcmp=", memcmp_eq && memcmp_lt && memcmp_gt);

        // --- strlen ---
        const HELLO_Z: &[u8] = b"hello\0";
        const EMPTY_Z: &[u8] = b"\0";
        let strlen_ok = strlen(HELLO_Z.as_ptr() as u64) == 5 && strlen(EMPTY_Z.as_ptr() as u64) == 0;
        write_check(b"strlen=", strlen_ok);

        // --- strcmp ---
        const S_ABC: &[u8] = b"abc\0";
        const S_ABD: &[u8] = b"abd\0";
        const S_AB: &[u8] = b"ab\0";
        let strcmp_eq = strcmp(S_ABC.as_ptr() as u64, S_ABC.as_ptr() as u64) == 0;
        let strcmp_lt = strcmp(S_ABC.as_ptr() as u64, S_ABD.as_ptr() as u64) < 0;
        let strcmp_gt = strcmp(S_ABD.as_ptr() as u64, S_ABC.as_ptr() as u64) > 0;
        let strcmp_prefix_lt = strcmp(S_AB.as_ptr() as u64, S_ABC.as_ptr() as u64) < 0;
        write_check(b"strcmp=", strcmp_eq && strcmp_lt && strcmp_gt && strcmp_prefix_lt);

        // --- strncmp: equal within n, differ once n reaches the real difference ---
        const SN_A: &[u8] = b"abcXXX\0";
        const SN_B: &[u8] = b"abcYYY\0";
        let strncmp_eq3 = strncmp(SN_A.as_ptr() as u64, SN_B.as_ptr() as u64, 3) == 0;
        let strncmp_ne4 = strncmp(SN_A.as_ptr() as u64, SN_B.as_ptr() as u64, 4) != 0;
        write_check(b"strncmp=", strncmp_eq3 && strncmp_ne4);

        // --- strcpy: full copy including NUL, returns dest ---
        let sc_dest = malloc(6);
        let sc_ret = strcpy(sc_dest, HELLO_Z.as_ptr() as u64);
        let strcpy_ok = sc_ret == sc_dest && bytes_equal(sc_dest, HELLO_Z);
        write_check(b"strcpy=", strcpy_ok);

        // --- strncpy: real C zero-pad-to-n semantics, src shorter than n ---
        const HI_Z: &[u8] = b"hi\0";
        let sn_dest = malloc(5);
        let sn_ret = strncpy(sn_dest, HI_Z.as_ptr() as u64, 5);
        const SN_EXPECTED: &[u8] = &[b'h', b'i', 0, 0, 0];
        let strncpy_ok = sn_ret == sn_dest && bytes_equal(sn_dest, SN_EXPECTED);
        write_check(b"strncpy=", strncpy_ok);

        // --- strcat: append "bar\0" onto an existing "foo\0" -> "foobar\0" ---
        let cat_dest = malloc(7);
        core::ptr::write((cat_dest) as *mut u8, b'f');
        core::ptr::write((cat_dest + 1) as *mut u8, b'o');
        core::ptr::write((cat_dest + 2) as *mut u8, b'o');
        core::ptr::write((cat_dest + 3) as *mut u8, 0);
        const BAR_Z: &[u8] = b"bar\0";
        let cat_ret = strcat(cat_dest, BAR_Z.as_ptr() as u64);
        const CAT_EXPECTED: &[u8] = b"foobar\0";
        let strcat_ok = cat_ret == cat_dest && bytes_equal(cat_dest, CAT_EXPECTED);
        write_check(b"strcat=", strcat_ok);

        // --- strchr: found mid-string, not-found -> 0, and the real
        // strchr(s,'\0') terminator-match case ---
        let chr_l = strchr(HELLO_Z.as_ptr() as u64, b'l');
        let chr_z = strchr(HELLO_Z.as_ptr() as u64, b'z');
        let chr_nul = strchr(HELLO_Z.as_ptr() as u64, 0);
        let strchr_ok =
            chr_l == HELLO_Z.as_ptr() as u64 + 2 && chr_z == 0 && chr_nul == HELLO_Z.as_ptr() as u64 + 5;
        write_check(b"strchr=", strchr_ok);

        w(b"\n");

        // ===================================================================
        // PART 2: buffered stdio -- real write/read round trip PLUS
        // mechanism-level proof (via the debug accessors) that buffering
        // genuinely defers syscalls rather than being a thin pass-through.
        // ===================================================================

        const PATH_A: &[u8] = b"stdiotest_a";
        const CONTENT: &[u8] = b"0123456789ABCDEFGHIJ0123456789abcdefghij";
        // Real, hand-counted length check (not assumed): the literal above
        // must be EXACTLY 40 bytes for every prediction below to hold.
        let content_len_ok = CONTENT.len() == 40;
        write_check(b"content_len_is_40=", content_len_ok);

        // --- fopen("w") on a brand-new path (see libc.rs's own top stdio
        // doc comment for why a brand-new path sidesteps the disclosed
        // no-O_TRUNC limitation entirely, rather than working around it) ---
        let f1 = fopen(PATH_A.as_ptr() as u64, PATH_A.len() as u64, b'w');
        let fopen_w_ok = f1 != 0;
        write_check(b"fopen_w=", fopen_w_ok);

        // Three fwrite() calls of sizes 12, 25, 3 (sum = 40) -- HAND-
        // COMPUTED to cross the 32-byte STDIO_BUFSIZE boundary MID-CALL
        // during the second call (12 + 20 = 32 lands exactly on the 20th
        // byte of the 25-byte call), proving a real auto-flush fires
        // inside fwrite() itself, not just once at fclose():
        //   after call 1 (12 bytes):  wlen == 12          (no flush yet)
        //   after call 2 (+25=37):    wlen == 37-32 == 5   (one auto-flush)
        //   after call 3 (+3=8):      wlen == 5+3   == 8   (still buffered)
        let n1 = fwrite(CONTENT.as_ptr() as u64, 1, 12, f1);
        let wlen1 = stdio_debug_wlen(f1);
        let n2 = fwrite((CONTENT.as_ptr() as u64) + 12, 1, 25, f1);
        let wlen2 = stdio_debug_wlen(f1);
        let n3 = fwrite((CONTENT.as_ptr() as u64) + 37, 1, 3, f1);
        let wlen3 = stdio_debug_wlen(f1);
        write_u64_line(b"  wlen after 12B write=", wlen1);
        write_u64_line(b"  wlen after +25B write=", wlen2);
        write_u64_line(b"  wlen after +3B write=", wlen3);
        let write_counts_ok = n1 == 12 && n2 == 25 && n3 == 3;
        let buffering_write_ok = wlen1 == 12 && wlen2 == 5 && wlen3 == 8;
        write_check(b"fwrite_counts=", write_counts_ok);
        write_check(b"fwrite_real_buffering=", buffering_write_ok);

        let fclose1_ok = fclose(f1) == 0;
        write_check(b"fclose_w=", fclose1_ok);

        // --- fopen("r") the same path back -- real round-trip content check,
        // PLUS mechanism-level read-buffering proof via fread() calls of
        // sizes 5, 20, 10, 5, then a final 1-byte read past real EOF:
        //   fread(5):  first access refills (rpos==rlen==0) -> ONE real
        //              sys_read() pulls min(32, 40) == 32 bytes in one call
        //              -> rlen==32, then consumes 5 -> rpos==5
        //   fread(20): rpos 5->25, all served from the SAME buffer, no
        //              second sys_read() -> rlen stays 32, rpos==25
        //   fread(10): rpos 25->32 (7 bytes, still buffered) then a SECOND
        //              real sys_read() refill for the remaining 40-32==8
        //              bytes -> rlen==8, rpos consumes 3 more -> rpos==3
        //   fread(5):  consumes the remaining rpos 3->8 exactly, no refill
        //              needed (rpos never re-hits rlen mid-loop) -> rpos==8
        //   fread(1):  rpos==rlen(8) -> refill attempted -> real EOF
        //              (sys_read returns 0) -> returns 0 elements, feof=true
        let f2 = fopen(PATH_A.as_ptr() as u64, PATH_A.len() as u64, b'r');
        let fopen_r_ok = f2 != 0;
        write_check(b"fopen_r=", fopen_r_ok);

        let readback = malloc(40);
        let r1 = fread(readback, 1, 5, f2);
        let rlen1 = stdio_debug_rlen(f2);
        let rpos1 = stdio_debug_rpos(f2);
        let r2 = fread(readback + 5, 1, 20, f2);
        let rlen2 = stdio_debug_rlen(f2);
        let rpos2 = stdio_debug_rpos(f2);
        let r3 = fread(readback + 25, 1, 10, f2);
        let rlen3 = stdio_debug_rlen(f2);
        let rpos3 = stdio_debug_rpos(f2);
        let r4 = fread(readback + 35, 1, 5, f2);
        let rpos4 = stdio_debug_rpos(f2);
        write_u64_line(b"  rlen after 5B read=", rlen1);
        write_u64_line(b"  rpos after 5B read=", rpos1);
        write_u64_line(b"  rlen after +20B read=", rlen2);
        write_u64_line(b"  rpos after +20B read=", rpos2);
        write_u64_line(b"  rlen after +10B read=", rlen3);
        write_u64_line(b"  rpos after +10B read=", rpos3);

        let read_counts_ok = r1 == 5 && r2 == 20 && r3 == 10 && r4 == 5;
        let buffering_read_ok =
            rlen1 == 32 && rpos1 == 5 && rlen2 == 32 && rpos2 == 25 && rlen3 == 8 && rpos3 == 3 && rpos4 == 8;
        write_check(b"fread_counts=", read_counts_ok);
        write_check(b"fread_real_buffering=", buffering_read_ok);

        let content_roundtrip_ok = bytes_equal(readback, CONTENT);
        write_check(b"content_roundtrip=", content_roundtrip_ok);

        let eof_before_ok = !feof(f2);
        let r5 = fread(readback, 1, 1, f2); // past real EOF now
        let eof_after_ok = feof(f2);
        write_check(b"eof_semantics=", eof_before_ok && r5 == 0 && eof_after_ok);

        let fclose2_ok = fclose(f2) == 0;
        write_check(b"fclose_r=", fclose2_ok);

        w(b"\n");

        // --- append mode: reopen the SAME path 'a', write "TAIL", verify
        // the real on-disk result is the original 40 bytes UNCHANGED
        // followed immediately by "TAIL" -- real proof the "drain via
        // sys_read to reach EOF" append mechanism genuinely lands the
        // cursor at the true end, not just at 0 or somewhere wrong. ---
        const TAIL: &[u8] = b"TAIL";
        let f3 = fopen(PATH_A.as_ptr() as u64, PATH_A.len() as u64, b'a');
        let fopen_a_ok = f3 != 0;
        let tail_write_ok = fwrite(TAIL.as_ptr() as u64, 1, TAIL.len() as u64, f3) == TAIL.len() as u64;
        let fclose3_ok = fclose(f3) == 0;
        write_check(b"append_write=", fopen_a_ok && tail_write_ok && fclose3_ok);

        let f4 = fopen(PATH_A.as_ptr() as u64, PATH_A.len() as u64, b'r');
        let appended = malloc(44);
        let append_read_n = fread(appended, 1, 44, f4);
        let append_prefix_ok = bytes_equal(appended, CONTENT);
        let append_tail_ok = bytes_equal(appended + 40, TAIL);
        let fclose4_ok = fclose(f4) == 0;
        write_check(
            b"append_roundtrip=",
            append_read_n == 44 && append_prefix_ok && append_tail_ok && fclose4_ok,
        );

        w(b"\n");

        // ===================================================================
        // PART 3: fprintf-equivalent -- real end-to-end formatted output,
        // written to a fresh file then read back and compared byte-for-byte
        // against a HAND-COMPUTED expected string.
        // ===================================================================
        const PATH_B: &[u8] = b"stdiotest_b";
        const FMT: &[u8] = b"val=%d hex=%x str=%s ch=%c pct=%%\n";
        const OK_STR: &[u8] = b"ok";
        const EXPECTED: &[u8] = b"val=-42 hex=2a str=ok ch=Q pct=%\n"; // hand-computed, see below

        let f5 = fopen(PATH_B.as_ptr() as u64, PATH_B.len() as u64, b'w');
        let args = [
            FmtArg::Int(-42),
            FmtArg::Hex(0x2A),
            FmtArg::Str(OK_STR.as_ptr() as u64, OK_STR.len() as u64),
            FmtArg::Char(b'Q'),
        ];
        let printed = fprintf(f5, FMT.as_ptr() as u64, FMT.len() as u64, &args);
        let fclose5_ok = fclose(f5) == 0;
        // Hand-computed: "val=" 4 + "-42" 3 + " hex=" 5 + "2a" 2 + " str=" 5
        // + "ok" 2 + " ch=" 4 + "Q" 1 + " pct=" 5 + "%" 1 + "\n" 1 == 33,
        // and EXPECTED.len() must independently equal the same 33.
        let fprintf_len_ok = printed == 33 && EXPECTED.len() == 33;

        let f6 = fopen(PATH_B.as_ptr() as u64, PATH_B.len() as u64, b'r');
        let printed_back = malloc(33);
        let printed_back_n = fread(printed_back, 1, 33, f6);
        let fprintf_content_ok = printed_back_n == 33 && bytes_equal(printed_back, EXPECTED);
        let fclose6_ok = fclose(f6) == 0;

        write_check(b"fprintf_len=", fprintf_len_ok);
        write_check(b"fprintf_content=", fprintf_content_ok && fclose5_ok && fclose6_ok);

        w(b"\n");

        let overall = memcpy_ok
            && memset_ok
            && memmove_ok
            && memcmp_eq && memcmp_lt && memcmp_gt
            && strlen_ok
            && strcmp_eq && strcmp_lt && strcmp_gt && strcmp_prefix_lt
            && strncmp_eq3 && strncmp_ne4
            && strcpy_ok
            && strncpy_ok
            && strcat_ok
            && strchr_ok
            && content_len_ok
            && fopen_w_ok
            && write_counts_ok
            && buffering_write_ok
            && fclose1_ok
            && fopen_r_ok
            && read_counts_ok
            && buffering_read_ok
            && content_roundtrip_ok
            && eof_before_ok && r5 == 0 && eof_after_ok
            && fclose2_ok
            && fopen_a_ok && tail_write_ok && fclose3_ok
            && append_read_n == 44 && append_prefix_ok && append_tail_ok && fclose4_ok
            && fprintf_len_ok
            && fprintf_content_ok
            && fclose5_ok
            && fclose6_ok;

        w(b"OVERALL=");
        w(if overall { b"PASS" } else { b"FAIL" });
        w(b"\n");

        sys_exit(if overall { 0 } else { 1 });
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
