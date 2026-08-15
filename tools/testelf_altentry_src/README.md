Build recipe for `kernel/assets/testelf_altentry.elf`, Milestone 44's real, externally-built
ELF64 test executable whose `e_entry` deliberately does NOT equal
`usertest::USER_CODE_ADDR` -- proof the generalized ELF loader actually
accepts and correctly runs a non-default entry point, not just the one
address every prior milestone's test ELF used.

```
rustc --target x86_64-unknown-none -C link-arg=-Ttestelf_altentry_src/linker.ld \
    -C code-model=large -C panic=abort --crate-type bin \
    testelf_altentry_src/testelf_altentry.rs -o kernel/assets/testelf_altentry.elf
```

`linker.ld` places the single `PT_LOAD` segment (and `e_entry`) at
`0x0000_5555_5000_3000` -- three pages past `USER_CODE_ADDR`, still within the
same p4 index the kernel's per-process private PML4 slot covers, but a
genuinely different address. Verify with `readelf -h kernel/assets/testelf_altentry.elf`
after rebuilding -- `Entry point address` should read `0x5555500003000`, not
`0x5555500000000`.
