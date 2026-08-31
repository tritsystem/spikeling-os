PHDRS
{
    seg1 PT_LOAD FLAGS(7); /* PF_R | PF_W | PF_X -- one segment: code + rodata,
                              same "one segment is enough, Milestone 36 already
                              proved multi-segment loading" reasoning as
                              tools/malloctest_src's own linker.ld */
}

ENTRY(_start)

SECTIONS
{
    . = 0x0000555550000000;
    .text : {
        KEEP(*(.text.start))
        *(.text*)
    } :seg1

    .rodata : {
        *(.rodata*)
        *(.lrodata*)
    } :seg1

    .data : {
        *(.data*)
    } :seg1

    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note*)
        *(.comment)
        *(.got*)
        *(.rela*)
        *(.debug*)
        *(.bss*)
    }
}
