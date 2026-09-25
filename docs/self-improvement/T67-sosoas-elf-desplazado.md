# T67 — El ELF de `sosoas` tiene la cabecera desplazada dos bytes

**Origen:** medido en [T38](T38-toolchain-inventario.md), fuera de su alcance.
**Aplica en:** `tools/sosoas/src/main.rs` · **Estado:** **hecha** (2026-09-25).

## Reproducción

    $ cat bytes.s
    	.text
    	.globl suma
    	.byte 0x89, 0xf8, 0x01, 0xf0, 0xc3

    $ cargo run -q -p sosoas -- -o bytes.o bytes.s
    sosoas: bytes.s → bytes.o (5 B .text)

    $ nm bytes.o
    nm: bytes.o: file format not recognized

    $ readelf -h bytes.o
      Type:     NONE (None)
      Machine:  WE32100
      Version:  0x3e
    $ readelf -S bytes.o
    readelf: Error: The e_shentsize field ... is less than the size of an ELF section header

## El diagnóstico, campo por campo

En ELF64 la cabecera es `e_type` en 0x10, `e_machine` en 0x12, `e_version`
(u32) en 0x14, `e_ehsize` en 0x34, `e_phentsize` 0x36, `e_phnum` 0x38,
`e_shentsize` 0x3a, `e_shnum` 0x3c, `e_shstrndx` 0x3e.

El código escribe:

| Escribe | En | Debería ir en | Efecto |
|---|---|---|---|
| `ET_REL` (1) | 0x12 | 0x10 | acaba en `e_machine` → **WE32100** |
| `EM_X86_64` (0x3e) | 0x14 | 0x12 | acaba en `e_version` → **Version: 0x3e** |
| `shnum` | 0x3a | 0x3c | acaba en `e_shentsize` → el error de `readelf -S` |
| `shstrndx` | 0x3c | 0x3e | acaba en `e_shnum` → «There are 2 section headers» |

**Todo desde `e_type` va corrido dos bytes**, y `e_type`, `e_ehsize`,
`e_shentsize` y `e_shstrndx` no se escriben nunca: quedan a cero. Cada síntoma
que `readelf` reporta corresponde a una de estas escrituras, así que el
diagnóstico no es una hipótesis.

## Por qué no lo había visto nadie

Porque **nada consume esos objetos todavía**. `sosoas` no participa en el
camino que hoy compila y arranca (eso usa `rust-lld` con los targets de
`user/`), así que su salida nunca pasa por un enlazador que se queje. El
defecto lleva ahí desde que se escribió y sólo aparece si alguien mira el
resultado con una herramienta estándar — que es justo lo que T38 obliga a
hacer.

## Alcance

Corregir los desplazamientos y escribir los campos que faltan. **No** convertir
`sosoas` en un ensamblador de verdad: eso es otra cosa y merece su propia
decisión (hoy sólo entiende `.byte`, y eso está documentado en
[toolchain-deps.md](native/toolchain-deps.md)).

## Lo que además apareció al arreglarlo

Los **section headers estaban igual de mal**, y eso la ficha no lo había
documentado porque `readelf -S` se paraba antes, en el `e_shentsize` roto:

| Escribía | En | Debería ir en |
|---|---|---|
| `text_off` | `sh_addr` (0x10) | `sh_offset` (0x18) |
| `text.len()` | `sh_offset` (0x18) | `sh_size` (0x20) |
| `sh_link` | `sh_size` (0x20) | `sh_link` (0x28) |
| `sh_addralign` | `sh_link` (0x28) | `sh_addralign` (0x30) |

Ocho bytes corridos desde `sh_addr`. Y `sh_name` y `sh_type` no se escribían
nunca, así que las dos secciones salían **sin nombre y de tipo NULL**. El
`sh_flags` del `.text` valía 1 (`SHF_WRITE`) con el comentario
`// SHF_ALLOC|EXEC` al lado: debía ser 6.

## Hecho

Los offsets viven ahora en dos módulos con nombre (`eh::`, `sh::`) y se
escriben con `put16/put32/put64`. Con números pelados, escribir en el sitio de
al lado se lee exactamente igual de bien esté bien o mal — que es cómo duró
esto.

Verificado con herramientas externas:

    readelf -h → Type: REL · Machine: Advanced Micro Devices X86-64 · Version: 0x1
    readelf -S → 3 secciones, .text PROGBITS AX, .shstrtab STRTAB, sin errores
    nm         → «no hay símbolos» (correcto: no hay symtab, ver límite)
    objdump -d → mov %edi,%eax / add %esi,%eax / ret

Ese último es la comprobación de verdad: los bytes que entraron por `.byte`
salen desensamblados como las instrucciones que representaban.

## Comprobación

Tres tests en `tools/sosoas` que **no dependen de binutils** —la toolchain
nativa tiene que poder comprobarse a sí misma—: cada campo de la cabecera
contra las constantes de ELF64, los nombres y tipos de las secciones, y que el
`.text` está donde su section header dice.

Se comprobó que **pueden fallar**, reintroduciendo las dos escrituras
originales por separado y restaurando después (md5 verificado):

| Mutación | La caza |
|---|---|
| `ET_REL` en `e_machine` | `la_cabecera_elf_tiene_cada_campo_en_su_sitio` |
| `sh_offset` en el sitio de `sh_addr` | esa y `el_texto_esta_donde_dice_el_section_header` |

    cargo test -p sosoas      5/5

## Límite

**No hay tabla de símbolos**: `.globl` se sigue ignorando, así que el objeto es
válido pero no exporta nada y no sirve para enlazar. Leer símbolos es trabajo
de ensamblador, y `sosoas` no lo es — está declarado en
[toolchain-deps.md](native/toolchain-deps.md), no escondido aquí.
