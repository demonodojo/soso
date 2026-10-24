# R41 — `Guid::parse` acepta cadenas que no son GUID y el MBR protector sale incompleto

## Qué hay hoy
El instalador (`soso-install`) lee GUID con `Guid::parse` y, tras clonar un disco, deja el MBR protector con
`protective_mbr_fix`. Ambos tienen comportamientos incorrectos observables:

- `Guid::parse("-2345678-1234-5678-1234-567812345678")` devuelve `Some(..)` con un GUID de basura, y lo
  mismo ocurre con `Guid::parse("12345678-1234-5678-1234-56781234567-")` o con un guion en cualquier
  otro lugar donde debería haber un dígito hexadecimal. Un texto así no es un GUID y debería dar `None`.
- Si el sector que se le pasa a `protective_mbr_fix` ya traía datos (el clon hereda el MBR híbrido del
  USB live, con otras entradas de partición), esas entradas siguen ahí junto a la protectora; y los campos
  CHS de la entrada quedan a cero, que es lo que ciertos firmware rechazan.
- `protective_mbr_fix(&mut mbr, 0)` entra en pánico en una compilación de depuración.

## Qué se pide
Corregir, sin cambiar ninguna firma pública:

1. `Guid::parse` sólo acepta la forma canónica de 36 caracteres con guiones exactamente en las posiciones
   8, 13, 18 y 23 (contando desde 0) y un dígito hexadecimal (mayúscula o minúscula) en las otras 32; en
   cualquier otro caso devuelve `None`. Lo que ya se aceptaba correctamente sigue igual (`Display` y
   `parse` son inversos).
2. `protective_mbr_fix` deja, además de lo que ya escribe hoy (byte 446 a 0, tipo `0xEE` en 450, primer LBA
   = 1 en 454..458, tamaño en 458..462, firma `55 AA`):
   - CHS inicial (bytes 447..450) = `00 02 00` y CHS final (bytes 451..454) = `FF FF FF`;
   - las otras tres entradas de partición (bytes 462..510) a cero;
   - los bytes 0..446 (código de arranque) sin tocar.
3. El tamaño (458..462, little-endian) es `disk_sectors - 1` saturado a `0xFFFFFFFF`, y es `0` cuando
   `disk_sectors` es `0` o `1` (sin pánico).

## Alcance
Solo `crates/gptdisk/src/lib.rs`: `Guid::parse` y `protective_mbr_fix`.

## Comprobación
`cargo test -p gptdisk`, antes y después de tu cambio.
