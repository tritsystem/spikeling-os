Build recipe for `kernel/assets/argvlauncher.elf`, Milestone 58's "caller"
half of the real argv/envp exec() test (single genuine `PT_LOAD` segment at
`USER_CODE_ADDR`, built with this project's own pinned Rust nightly
toolchain + `rust-lld`, not hand-assembled -- same recipe as every other
tools/*_src build).

```
rustc --target x86_64-unknown-none -C link-arg=-Ttools/argvlauncher_src/linker.ld \
    -C link-arg=--gc-sections -C link-arg=-znoseparate-code \
    -C link-arg=-zmax-page-size=16 \
    -C code-model=large -C relocation-model=static \
    -C panic=abort -C opt-level=z --crate-type bin \
    tools/argvlauncher_src/main.rs -o kernel/assets/argvlauncher.elf
```

Same flags, same reasons, as `tools/malloctest_src/README.md`.

Real, verified result: 1104-byte file, `EXEC` type, one `PT_LOAD` segment
at `0x0000555550000000` (== `usertest::USER_CODE_ADDR`), `p_filesz ==
p_memsz == 0x172`. Verified with `readelf -h`/`readelf -l` after building.

Calls the kernel's new syscall 16 (EXECARGV, see
`kernel/src/usertest.rs`'s syscall_dispatch and
`kernel/src/process.rs`'s `exec_elf_with_args()`) with a real 3-entry
argv array (`["argvtarget", "hello", "world"]`) and a real 1-entry envp
array (`["GREETING=hi"]`), each entry a `#[repr(C)] struct ArgSpec { ptr:
u64, len: u64 }` -- the same ptr+len idiom this kernel already uses for
every other string-bearing syscall argument, not NUL-terminated C
strings (the kernel appends the actual NUL itself while laying the new
process's stack out). On success this never returns -- control jumps
straight into `kernel/assets/argvtarget.elf`'s own `_start`, which reads
that same argv/envp data back off its own real initial stack and checks
it against these exact values.
