Build recipe for `kernel/assets/malloctest.elf`, Milestone 51's real,
externally-built ELF64 test executable exercising the new `malloc()`/`free()`
added to libc.rs at this milestone (single genuine `PT_LOAD` segment at
`USER_CODE_ADDR`, built with this project's own pinned Rust nightly toolchain
+ `rust-lld`, not hand-assembled -- same recipe as `tools/libc_test_src`'s own
Milestone 39 build, since this program also `mod libc;`s a real libc.rs).

```
rustc --target x86_64-unknown-none -C link-arg=-Ttools/malloctest_src/linker.ld \
    -C link-arg=--gc-sections -C link-arg=-znoseparate-code \
    -C link-arg=-zmax-page-size=16 \
    -C code-model=large -C relocation-model=static \
    -C panic=abort -C opt-level=z --crate-type bin \
    tools/malloctest_src/main.rs -o kernel/assets/malloctest.elf
```

Same flags, same reasons, as `tools/libc_test_src/README.md` (`--gc-sections`
to drop the unused syscall wrappers this program never calls -- `sys_open`/
`sys_read`/`sys_fdwrite`/`sys_close`/`sys_fork`/`sys_wait`/`sys_exec` all warn
`never used` on a clean build, expected and harmless; `relocation-model=static`
to avoid a `DYN` ELF with dynamic-linking metadata this kernel's loader has no
use for; `-zmax-page-size=16` to avoid page-size-alignment padding; `opt-level=z`
to minimize incidental code size).

Real, verified result: 3200-byte file (comfortably under `fs.rs`'s 4096-byte
on-disk cap, though this milestone's own self-test loads it directly via
`process::load_and_run_elf()` without touching the filesystem at all), `EXEC`
type, one `PT_LOAD` segment at `0x0000555550000000` (== `usertest::USER_CODE_ADDR`),
`p_filesz=0x596` / `p_memsz=0x5a0` -- the real 8-byte gap is `FREE_LIST_HEAD`
(an `AtomicU64`), which genuinely lands in a true `NOBITS` `.lbss` section, not
file-backed content; the kernel's own ELF loader zero-fills exactly that
`memsz - filesz` tail per its own documented segment-loading contract, so
`FREE_LIST_HEAD` starts real-zero (an empty free list) without this program
ever writing to it itself. Verified with `readelf -h`/`readelf -l`/`readelf -S`
after rebuilding.
