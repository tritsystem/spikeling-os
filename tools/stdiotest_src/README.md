Build recipe for `kernel/assets/stdiotest.elf`, Milestone 61's real, externally-built
ELF64 test executable exercising the new `string.h` (memcpy/memmove/memset/memcmp/
strlen/strcmp/strncmp/strcpy/strncpy/strcat/strchr) and buffered stdio
(fopen/fclose/fread/fwrite/fflush/fputc/fgetc/fputs/feof/fprintf-equivalent)
added to libc.rs at this milestone (single genuine `PT_LOAD` segment at
`USER_CODE_ADDR`, built with this project's own pinned Rust nightly toolchain
+ `rust-lld`, not hand-assembled -- same recipe as every other tools/*_src build).

```
rustc --target x86_64-unknown-none -C link-arg=-Ttools/stdiotest_src/linker.ld \
    -C link-arg=--gc-sections -C link-arg=-znoseparate-code \
    -C link-arg=-zmax-page-size=16 \
    -C code-model=large -C relocation-model=static \
    -C panic=abort -C opt-level=z --crate-type bin \
    tools/stdiotest_src/main.rs -o kernel/assets/stdiotest.elf
```

Same flags, same reasons, as `tools/malloctest_src/README.md` (`--gc-sections` to
drop unused wrappers this program never calls -- `sys_open`/`sys_close` are actually
called indirectly via `fopen`/`fclose`, but `sys_fork`/`sys_wait`/`sys_exec`/`fgetc`/
`fputs` all warn `never used` on a clean build, expected and harmless, matching every
prior tools/*_src build's own disclosed warning set).

Real, hand-computed predictions this program checks its own string.h/stdio
implementation against (see main.rs's own inline comments for the exact numbers):
memmove's overlap-correct byte-for-byte result, the exact internal `wlen`/`rlen`/
`rpos` values at each step of a real, multi-call buffered write and read (proving
`STDIO_BUFSIZE`-boundary auto-flush/auto-refill genuinely fire, not just that the
final round-tripped content happens to match), a real append-mode round trip, and a
byte-exact `fprintf`-equivalent output comparison.
