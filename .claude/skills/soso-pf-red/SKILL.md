---
name: soso-pf-red
description: >-
  Seguimiento del page fault recurrente del kernel en placa al hacer
  soso-update comprobar (HTTPS a github.com y
  release-assets.githubusercontent.com). Puntero no mapeado ~0x153451e79a8,
  panic en arch/interrupts.rs, centinelas lxdde intactos. Usar cuando haya
  una foto o SOSOLOG con EXCEPTION page fault, «centinelas en #PF», talc,
  smoltcp o este fallo de red en el live USB. No es el #PF de usuario, ni el
  de la GPU, ni el rip=0 al cargar un modelo.
---

# Page fault de red en placa

Fallo abierto. Cada sesión **añade una fila** a
[seguimiento.md](seguimiento.md) y no reabre lo que esa ficha ya da por
descartado.

## Firma

Es este fallo solo si coinciden:

- Placa real, live USB, `soso-update comprobar` (o el mismo cliente HTTPS).
- Traza `red:`: `conectado (fd 4)` a `github.com`, DNS de
  `release-assets.githubusercontent.com`, `conectando` a
  `185.199.111.133:443`, y a veces otro `conectado (fd 4)`.
- `EXCEPTION: page fault at 0x153… rip=0x10000… rsp=0x10000… cs=0x8 err=0x0`
  en `kernel/src/arch/interrupts.rs` (`kernel_pf_panic_shim`).
- `err=0`: lectura en anillo 0 de una página no presente. `cs=0x8`.
- `rsp` dentro de `KSTACK` de la BSP. `lxdde: centinelas en #PF: N bloques
  vivos, 0 desbordados`.

La dirección falla estable es del estilo `0x153451e79a8`. El OCR de la foto
se come dígitos; no tratar dos lecturas distintas como dos punteros distintos
sin el volcado en crudo.

QEMU no lo reproduce: el HTTPS de la suite no coincide con el `memmove` de
rustls ni con el RX de la AX200/AX211.

## Traducir el RIP

El número `0x43a5ed` **no es una función**. Depende del ELF que había en el
pendrive.

1. Confirmar el binario flasheado (fecha de `SOSOLOG`, hash, o el ELF guardado
   de esa sesión). `target/kernel/x86_64-soso/debug/kernel` solo vale si es
   ese build.
2. Base del kernel: `0x10000000000`. Offset = `rip` menos esa base.
3. `addr2line -f -C -e <elf> <offset>` y `objdump -d` de unas 32 instrucciones
   alrededor. Si el offset cae a mitad de una instrucción, el RIP de la foto
   está mal leído: encajar el volcado (`+0x10`…`+0x28`) con los `mov` a
   `(%rsp)` de esa zona.
4. `rastro_de_pila` imprime **cualquier** qword de la pila que caiga en el
   kernel, no una cadena de frames. Un símbolo en el rastro no es el llamante.

Lecturas ya hechas, no reutilizarlas con otro ELF:

| ELF | Offset `0x43a5ed` |
|-----|-------------------|
| El del pánico del 21 sep | `talc::llist::IterMut::next` (`llist.rs:98`), `mov (%rax), %rdx`. `rax` ya era el puntero malo. `+0x40` = `0x1599e8` = `Talc::malloc`. |
| `target/…/debug/kernel` del 24 sep 09:18 | A mitad de `IndexMut<[u8; 12]>` (smoltcp). Esa función vuelca rdi/rdx/rsi en `+0x10`…`+0x28` y copia 12 bytes. El volcado de la foto de esa mañana encaja con esos spills **solo** si el pendrive llevaba ese ELF. |

## Qué no volver a investigar

- Centinelas de `lx_kmalloc`: 0 desbordados. El pool C no es el origen.
- `cld` en `irq::dispatch`, `page_fault_handler`, `syscall_entry` y
  `timer_isr`. El kernel de después de ese cambio sigue petando. El `memcpy`
  del kernel es un bucle hacia delante; en ese ELF no había `rep movs`.
- Copias acotadas del RX/TX iwlwifi (`deliver_rx` corta a 2040), rtl8169 con
  el enlace caído, anillo de smoltcp y scroll del framebuffer.
- `LlistNode::insert` / `remove` de talc. Si `next` fuera el puntero sin
  mapear, el page fault sería **dentro** del `free`, al desreferenciarlo. El
  que se ve es el `malloc` (o la copia) **siguiente**. Alguien escribió esos
  8 bytes en un qword que sí está mapeado, sin pasar por insert/remove.

## Qué queda abierto

Quién escribe el qword. Lo que se sabe del qword (fotos 17:11 y 19:33):
la coincidencia del CR2 en el heap cae en `0x44444444…`, es decir en los
primeros 64 KiB, donde `claim` deja la tabla `bins` de talc
(`HEAP_START+8`, 128 cabeceras). Los CR2 de todas las fotos son un `u16`
pequeño en los bytes +4/+5 (`0x55`, `0x32`, `0xe7`, `0x153`, `0x216`…) y a
veces conservan en la mitad baja un puntero del heap (`0xe7_4c81da30`).
Hipótesis de trabajo: un store de 16 bits en `bins[0]+4`.

Instrumentación en el kernel (sin `SOSO_HEAP_DEBUG` extra):

- **Watchpoint hardware** `arch::hwbp`: DR0/DR1 sobre `bins[0]`/`bins[1]`
  (escritura, 8 bytes), armado tras `claim` y en cada AP. El handler `#DB`
  deja pasar las escrituras con el candado de talc tomado y valor dentro del
  heap; cualquier otra → `heap: ESCRITURA EN bins (DRn) addr valor rip rsp
  cpu candado`, rastro de pila, últimos alloc/free y panic con el offset
  para `addr2line -e kernel-x86_64`. `rip` es la instrucción **siguiente**
  al store (es una trampa).
- `heap::vigilar_huecos` en cada `punto` y tras RX/TX WiFi → panic
  `HUECO ROTO` con bin, nodo, `next`, últimos 8 alloc/free (`ra`). Busca
  `bins` por valor (`TALC_BINS`), no por orden de campos: `Talc` es
  `repr(Rust)` y el espejo anterior leía la máscara de bits.
- En #PF: `localizar_en_pf` (dice `la coincidencia es bins[N] de talc`),
  luego `informar_dma_en_pf` (iwlwifi + DMA_FREE + cuarentena).

Flashear (`cargo xtask flash-usb-live /dev/sda --yes --only kernel`, con
sudo, lo hace el usuario) y repetir `soso-update comprobar`. Lectura:

- `ESCRITURA EN bins` → la CPU escribe; el `rip` (menos `0x10000000000`)
  es el store. Corregir ahí.
- `HUECO ROTO` o `nodo libre fuera del heap en bin 0` **sin** `#DB` antes →
  el store no usó la VA vigilada. Visto el 2026-09-24 en `aplicar`:
  `bins[0] = 0xa9700000000` (`u16` `0x0A97` en +4, mitad baja 0) y ningún
  `ESCRITURA EN bins`. Siguiente paso: la PA del frame de `HEAP_START` y si
  aparece en las tablas DMA de iwlwifi o en otra VA del mismo frame.

Sonda `mm::heap::punto` (no se libera): el pánico imprime `heap: último
punto intacto «…»`, el último `malloc` de sonda que **volvió**. También
imprime `red: última trama rx|tx ptr=… len=…` y `pf: ins` con los 8 bytes
de la instrucción en `rip`. Esos bytes se buscan en el ELF de la ESP
(`objdump -d`); el `rip` de la foto del 12:56 (`0x43ae41d`) cae fuera de
ese `.text` y no se simboliza.

| Punto | Cuándo |
|-------|--------|
| `dns` | al entrar en `resolve_hostname` |
| `poll-entra` | cada `net::poll`, antes de `try_attach` |
| `poll-nicho` | después de `try_attach` y del sondeo de enlace |
| `poll-rx` | después del primer `iface.poll` (RX) |
| `poll-sale` | al salir, tras purgar sockets |
| `tcp-buf` | justo antes de los dos búferes de 64 KiB |

No hace falta `SOSO_HEAP_DEBUG`. Hay que flashear este kernel: sin la línea
`heap: último punto` la foto es la de antes.

## Al cerrar una sesión

Añadir una fila a [seguimiento.md](seguimiento.md): fecha, ELF (ruta o
fecha), `rip`/`addr`/`err`, símbolos traducidos, y si se descartó algo nuevo.
Si el store se encuentra, sustituir «Qué queda abierto» por el sitio y el
arreglo, y dejar el resto como historial.
