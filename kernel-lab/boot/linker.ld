ENTRY(_start)

SECTIONS {
    . = 1M;

    .boot : {
        /* Ensure multiboot header is at the very beginning of the ELF file */
        KEEP(*(.multiboot_header))
    }

    .text : {
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    .data : {
        *(.data .data.*)
    }

    .bss : {
        *(.bss .bss.*)
    }
}
