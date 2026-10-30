/* Binarios de usuario de soso: estáticos, cargados a dirección fija. */
ENTRY(_start)

SECTIONS {
    . = 0x400000;
    .text : {
        *(.text._start)
        *(.text .text.*)
    }
    . = ALIGN(4096);
    .rodata : { *(.rodata .rodata.*) }
    . = ALIGN(4096);
    .data : { *(.data .data.*) }
    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    }
    /DISCARD/ : { *(.comment) }
}
