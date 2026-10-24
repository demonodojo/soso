# R48 — `CachePolicy` tipada y consultas sobre `ShardEntry` y `ModelEntry`

## Qué hay hoy
En `sosomfs`, la política de caché de un shard es un `u8` suelto (`ShardEntry::cache_policy`, con las
constantes `CACHE_NORMAL = 0`, `CACHE_STREAM = 1` y `CACHE_PIN = 2`), y cada llamador que quiere saber el
tamaño de un modelo, buscar un shard o listar los de un directorio recorre `shards` a mano. Los tipos
`ShardEntry` y `ModelEntry` están en `crates/sosomfs/src/layout.rs` y no tienen ningún método.

## Qué se pide
**Añadir**, sin cambiar nada de lo que ya existe (campos, constantes, `Catalog`, firmas actuales), en
`layout.rs` (ya se reexporta todo desde la raíz del crate con `pub use layout::*`):

1. `pub enum CachePolicy { Normal, Stream, Pin }` con `Clone, Copy, Debug, PartialEq, Eq`.
   - `impl TryFrom<u8> for CachePolicy` con `type Error = u8`: 0 → `Normal`, 1 → `Stream`, 2 → `Pin`
     (las constantes `CACHE_*`); cualquier otro valor `v` da `Err(v)`.
   - `impl From<CachePolicy> for u8` con la correspondencia inversa.
2. `impl core::fmt::Display for CachePolicy`: escribe exactamente `normal`, `stream` o `pin` (minúsculas,
   sin relleno ni adornos), y
   `pub struct ParseCachePolicyError;` (`Clone, Copy, Debug, PartialEq, Eq`) con
   `impl core::str::FromStr for CachePolicy` (`type Err = ParseCachePolicyError`) que acepta **sólo** esos
   tres textos tal cual: cualquier otra cadena (vacía, con mayúsculas, con espacios, otro nombre) da
   `Err(ParseCachePolicyError)`.
3. `ShardEntry::policy(&self) -> Option<CachePolicy>`: la política de su campo `cache_policy`, o `None` si
   el valor no es 0, 1 ni 2.
4. `ShardEntry::total_blocks(&self) -> u64`: suma de `block_count` de todos sus `extents` (0 si no tiene);
   la suma satura en `u64::MAX`, no desborda.
5. `ModelEntry::total_bytes(&self) -> u64`: suma de `byte_len` de todos sus shards (0 si no tiene); satura
   en `u64::MAX`.
6. `ModelEntry::shard(&self, rel_path: &str) -> Option<&ShardEntry>`: el shard cuya `rel_path` es
   **exactamente** `rel_path` (sin prefijos, sin distinguir nada más); si hay varios con la misma ruta,
   el primero.
7. `ModelEntry::shards_under<'a>(&'a self, dir: &'a str) -> impl Iterator<Item = &'a ShardEntry> + 'a`:
   los shards, en el orden de `shards`, cuya `rel_path` empieza por `dir` seguido de `/`
   (`dir = "weights"` incluye `weights/a.bin` y `weights/sub/d.bin`, pero no `weights-extra/c.bin`, ni
   `weights` a secas, ni `weight/x`). Con `dir = ""` devuelve todos los shards.

## Alcance
Solo `crates/sosomfs/src/layout.rs`. No cambies el formato en disco, `Catalog` ni los demás módulos.

## Comprobación
`cargo test -p sosomfs --features std`, antes y después de tu cambio.
