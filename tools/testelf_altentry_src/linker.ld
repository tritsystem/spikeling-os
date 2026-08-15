PHDRS
{
    seg1 PT_LOAD FLAGS(5); /* PF_R | PF_X */
}

ENTRY(alt_entry)

SECTIONS
{
    . = 0x0000555550003000;
    .text.altentry : {
        KEEP(*(.text.altentry))
        *(.rodata.entry)
    } :seg1

    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note*)
        *(.comment)
        *(.got*)
        *(.rela*)
        *(.debug*)
    }
}
