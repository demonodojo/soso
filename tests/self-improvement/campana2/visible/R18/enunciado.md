# R18 — `kde::find_pmkid`

## Qué hay hoy
`soso_wpa2::kde` sabe extraer la GTK del Key Data del 4-way handshake (`find_gtk`) recorriendo los
KDE RSN con `for_each_rsn_kde`. El **PMKID** (KDE de tipo 4, 16 bytes) que algunos puntos de
acceso envían en el mensaje 1 no se puede leer.

## Qué se pide
Añadir:

```rust
pub const KDE_DATA_TYPE_PMKID: u8 = 4;
pub fn find_pmkid(key_data: &[u8]) -> Option<[u8; 16]>
```

- busca el primer KDE RSN con `data_type == 4`; su payload son los 16 bytes del PMKID;
- si el payload mide **menos de 16** bytes se ignora ese KDE; si mide más, se toman los primeros 16;
- `None` si no hay ninguno; se salta lo que no es KDE RSN (como el RSN IE, id `0x30`) respetando su
  longitud, y un elemento con longitud que se sale del búfer corta el recorrido (igual que `find_gtk`);
- no cambia `find_gtk` ni `for_each_rsn_kde`.

## Alcance
Solo `crates/soso-wpa2/src/kde.rs`.

## Comprobación
`cargo test -p soso-wpa2`, antes y después de tu cambio.
