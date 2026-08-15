Build recipe for `kernel/assets/testelf_altentry.elf`, Milestone 44's real, externally-built
ELF64 test executable whose `e_entry` deliberately does NOT equal
`usertest::USER_CODE_ADDR` -- proof the generalized ELF loader actually
accepts and correctly runs a non-default entry point, not just the one
address every prior milestone's test ELF used.

```
rustc --target x86_64-unknown-none -C link-arg=-Ttestelf_altentry_src/linker.ld \
    -C code-model=large -C relocation-model=static -C link-arg=-z -C link-arg=max-page-size=16 \
    -C panic=abort --crate-type bin \
    testelf_altentry_src/testelf_altentry.rs -o kernel/assets/testelf_altentry.elf
```

**MILESTONE 45 correction to this recipe, found by actually running it for
the first time** (Milestone 44 left this file staged but never build-
verified -- its own commit message says so): the command as originally
documented here (without `-C relocation-model=static -C link-arg=-z
-C link-arg=max-page-size=16`) genuinely does NOT work --
`kernel/assets/testelf_altentry.elf` as committed by Milestone 44 was a
16-byte placeholder stub (all zero bytes), not a real ELF at all. Two
real, separate problems, both confirmed directly with `readelf`, not
guessed:
  1. Without `-C relocation-model=static`, `x86_64-unknown-none`'s
     default link produces `Type: DYN` (a position-independent
     executable) instead of `Type: EXEC` -- `elf.rs`'s parser requires
     `ET_EXEC` exactly (see its own module doc comment) and rejects a
     `DYN` file outright.
  2. Without `-C link-arg=-z -C link-arg=max-page-size=16`, the single
     `PT_LOAD` segment's file offset lands at the linker's default
     page-size boundary (`0x1000`), producing an 856-byte payload's
     worth of real code padded out to ~4.8 KB -- comfortably over
     `fs.rs`'s own `MAX_FILE_BYTES` (4096) cap, so the file could never
     even be written to this kernel's on-disk filesystem. Same fix
     `testelf_src`'s own README already documents for exactly this
     reason.

There was also a real, independent compile error in `testelf_altentry.rs`
itself (fixed in that file directly, not here): `MESSAGE`'s declared
array size (71) didn't match its actual string literal length (70) --
`rustc` correctly refused to build it (`E0308`) until fixed.

`linker.ld` places the single `PT_LOAD` segment (and `e_entry`) at
`0x0000_5555_5000_3000` -- three pages past `USER_CODE_ADDR`, still within the
same p4 index the kernel's per-process private PML4 slot covers, but a
genuinely different address. Verify with `readelf -h kernel/assets/testelf_altentry.elf`
after rebuilding -- `Entry point address` should read `0x555550003000`, `Type`
should read `EXEC (Executable file)` (not `DYN`), and the file should be well
under 4096 bytes.
