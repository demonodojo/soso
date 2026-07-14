/* Binarios de usuario de soso: estáticos, cargados a dirección fija. */
ENTRY(_start)

SECTIONS {
    . = 0x400000;
    .text : {
        *(.text._start)
        *(.text .text.*)
    }
    .rodata : { *(.rodata .rodata.*) }
    .data : { *(.data .data.*) }
    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    }
    /DISCARD/ : { *(.eh_frame*) *(.comment) }
}
