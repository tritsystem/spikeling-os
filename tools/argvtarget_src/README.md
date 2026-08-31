Build recipe for `kernel/assets/argvtarget.elf`, Milestone 58's "receiver"
half of the real argv/envp exec() test (single genuine `PT_LOAD` segment at
`USER_CODE_ADDR`, built with this project's own pinned Rust nightly
toolchain + `rust-lld`, not hand-assembled -- same recipe as every other
tools/*_src build).

```
rustc --target x86_64-unknown-none -C link-arg=-Ttools/argvtarget_src/linker.ld \
    -C link-arg=--gc-sections -C link-arg=-znoseparate-code \
    -C link-arg=-zmax-page-size=16 \
    -C code-model=large -C relocation-model=static \
    -C panic=abort -C opt-level=z --crate-type bin \
    tools/argvtarget_src/main.rs -o kernel/assets/argvtarget.elf
```

Same flags, same reasons, as `tools/malloctest_src/README.md`.

One REAL build failure hit and fixed while writing this program (not
hidden): an earlier `write_dec()` used ordinary `buf[i] = ...` array
indexing to format a decimal number, with `i` only bounded by a runtime
`while val > 0` loop -- LLVM cannot prove that in-bounds at this crate's
`opt-level=z`, so it kept a real slice-bounds-check branch calling
`core::panicking::panic_bounds_check`, which formats its own message via
`core::fmt::Display for u64` -- and THAT pulled in GOT-relative
relocations the precompiled `x86_64-unknown-none` `core` distributed via
rustup cannot satisfy together with this program's own
`-C code-model=large` (`rust-lld: relocation R_X86_64_GOTPCREL out of
range`, a genuine link failure, not a guess). Fixed by rewriting
`write_dec()` to use raw pointer arithmetic (`ptr.offset(i)` /
`core::ptr::write`) instead of indexing -- no bounds check emitted at
all, correct AND avoids the dependency, not a workaround for broken
logic. `tools/malloctest_src/main.rs`'s own hex-printer avoids this same
trap by using a loop range LLVM CAN prove in-bounds at compile time
(`for i in 0..16`, fixed against a fixed-size array).

Real, verified result: 3040-byte file, `EXEC` type, one `PT_LOAD` segment
at `0x0000555550000000` (== `usertest::USER_CODE_ADDR`), `p_filesz ==
p_memsz == 0x531` (no `.bss` gap -- this program keeps no writable
statics of its own beyond what fits in `.data`). Verified with
`readelf -h`/`readelf -l` after building.

`_start` is `#[unsafe(naked)]` (same mechanism as
`kernel/src/usertest.rs`'s own `syscall_entry()`) so it can read the
hardware-set `rsp` BEFORE any Rust-generated function prologue could
disturb it -- `mov rdi, [rsp]` / `lea rsi, [rsp+8]` / a `lea`-computed
`rdx` for envp, tail-called into ordinary Rust via `call {main}`. This is
the real x86_64 SysV process-entry stack contract (the same one a real
kernel's `execve()` establishes, and a real libc's own `_start` assumes)
-- not a spikeling-os-specific convention invented for this milestone.
