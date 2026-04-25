SECTIONS
{
  . = SIZEOF_HEADERS;
  .text : { *(.text*) }
  .rodata : { *(.rodata*) }
  .data.rel.ro : { *(.data.rel.ro*) }
  .dynamic : { *(.dynamic) }
  .dynsym : { *(.dynsym) }
  .dynstr : { *(.dynstr) }
  .rel.dyn : { *(.rel.dyn) }
  /DISCARD/ : {
    *(.eh_frame*)
    *(.gnu.hash*)
    *(.hash*)
    *(.comment*)
    *(.symtab)
    *(.strtab)
    *(.debug_*)
  }
}
