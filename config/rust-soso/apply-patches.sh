#!/usr/bin/env bash
# Aplica parches de soso a vendor/rust (PAL, build.rs, exit, Cargo.toml).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RUST="${SOSO_RUST_VENDOR:-${XDG_CACHE_HOME:-$HOME/.cache}/soso-rust-vendor}"
TREE="$ROOT/config/rust-soso/tree"

if [[ ! -d "$RUST/library/std" ]]; then
  echo "apply-patches: falta $RUST — ejecuta scripts/soso-rust-bootstrap.sh" >&2
  exit 1
fi

# Dos regímenes distintos, a propósito:
#
# 1. Directorios que el árbol posee **enteros**: espejo con `--delete`, para que
#    quitar un fichero aquí lo quite del vendor. Sin eso, un resto se queda para
#    siempre — así llegó `dl.rs` a estar en el vendor sin que nadie lo compile
#    (T69).
for d in library/std/src/os/soso library/std/src/sys/pal/soso; do
  mkdir -p "$RUST/$d"
  rsync -a --delete "$TREE/$d/" "$RUST/$d/"
done

# 2. Ficheros sueltos que el árbol **añade** dentro de directorios de rust
#    (p. ej. `sys/io/error/soso.rs`): copia uno a uno y **sin** `--delete`, que
#    ahí se llevaría por delante el std original.
( cd "$TREE" && find library -type f -name '*.rs' \
    ! -path 'library/std/src/os/soso/*' \
    ! -path 'library/std/src/sys/pal/soso/*' -print0 ) |
while IFS= read -r -d '' f; do
  mkdir -p "$RUST/$(dirname "$f")"
  cp "$TREE/$f" "$RUST/$f"
done
# T69: la generación antigua (`config/rust-soso/sys/pal/soso/`, con `dl.rs`)
# no la declara el `mod.rs` de la PAL nueva. El espejo de arriba la retira
# del vendor; no se vuelve a copiar.

grep -q 'target_os == "soso"' "$RUST/library/std/build.rs" || \
  sed -i 's/|| target_os == "vexos"/|| target_os == "vexos"\n        || target_os == "soso"/' \
    "$RUST/library/std/build.rs"

grep -q 'target_os = "soso"' "$RUST/library/std/src/sys/pal/mod.rs" || \
  perl -i -0pe 's/(\s+target_os = "zkvm" => \{\s+mod zkvm;\s+pub use self::zkvm::\*;\s+\})\s+_ =>/\1\n    target_os = "soso" => {\n        mod soso;\n        pub use self::soso::*;\n    }\n    _ =>/s' \
    "$RUST/library/std/src/sys/pal/mod.rs"

# Los paréntesis **no** se escapan en una expresión básica: `\)` sin `\(`
# delante es un error de sintaxis, y `sed` sale 1. Con `set -e` eso abortaba el
# script justo aquí, así que los cuatro parches siguientes no llegaban a
# ejecutarse nunca en un vendor limpio — y el mensaje final «OK» tampoco se
# imprimía, pero quien mirara el vendor a medio parchear no tenía forma de
# saber por dónde se había quedado. Ver T39.
# `os/mod.rs`, dos cosas. Primero **reparar** lo que dejó la versión anterior de
# este parche: insertaba después de `#[cfg(target_os = "hermit")]`, con lo que
# `pub mod hermit;` se quedaba sin guarda —y se compilaba para soso, pidiendo
# `hermit_abi`— y `pub mod soso;` heredaba DOS `#[cfg]` apilados, que es un AND
# (hermit **y** soso): no se compilaba jamás. El guardián de abajo ve
# `pub mod soso` y no lo habría tocado nunca. Ver T39.
perl -i -0pe 's/\#\[cfg\(target_os = "hermit"\)\]\n\#\[cfg\(target_os = "soso"\)\]\npub mod soso;\npub mod hermit;/#[cfg(target_os = "soso")]\npub mod soso;\n#[cfg(target_os = "hermit")]\npub mod hermit;/s' \
  "$RUST/library/std/src/os/mod.rs"

grep -q 'pub mod soso' "$RUST/library/std/src/os/mod.rs" || \
  perl -i -0pe 's/(\#\[cfg\(target_os = "hermit"\)\]\npub mod hermit;)/#[cfg(target_os = "soso")]\npub mod soso;\n$1/s' \
    "$RUST/library/std/src/os/mod.rs"

# `os::fd` **no** se habilita para soso todavía. El parche anterior añadía
# `target_os = "soso",` a la lista de `#[cfg(any(...))] pub mod fd;`, y eso
# arrastra `os/fd/net.rs` y los `impl` de `AsRawFd`/`FromRawFd` para `Pipe`,
# `TcpStream`, `TcpListener` y `UdpSocket` — que la PAL de hoy no tiene, porque
# es la de `unsupported`. Son ~20 de los errores de `std`. soso sí tiene
# descriptores en su ABI, así que esto vuelve cuando la PAL los exponga; hasta
# entonces, declararlo ausente es más honesto que arrastrar `impl`s imposibles.
sed -i '/^    target_os = "soso",$/d' "$RUST/library/std/src/os/mod.rs"

grep -q 'target_os = "soso"' "$RUST/library/std/src/sys/exit.rs" || \
  sed -i 's/target_os = "hermit" => unsafe { hermit_abi::exit(code) },/target_os = "hermit" => unsafe { hermit_abi::exit(code) },\n        target_os = "soso" => soso_rt::exit(code),/' \
    "$RUST/library/std/src/sys/exit.rs"

# Aquí no basta con «¿ya está?»: esta línea la escribe este script y su
# contenido cambia —la ruta del checkout, las features—. Un guardián que sólo
# mire si existe la deja **vieja para siempre**: pasó con
# `features = ["dep-of-std"]`, que siguió en el vendor después de corregirla
# aquí, y el build volvió a fallar igual. Idempotente no es convergente.
# `public = true` como `moto-rt`: `os/soso/mod.rs` reexporta el crate
# (`pub extern crate soso_rt as soso_abi;`), y sin marcarlo público rustc
# avisa de «crate from private dependency is re-exported» — que con
# `build.warnings` en deny tumba el build.
SOSO_DEP="soso-rt = { path = \"$ROOT/crates/soso-rt\", features = [\"rustc-dep-of-std\"], public = true }"
if ! grep -q 'target_os = "soso"' "$RUST/library/std/Cargo.toml"; then
  printf '\n[target.%s.dependencies]\n%s\n' "'cfg(target_os = \"soso\")'" "$SOSO_DEP" \
    >>"$RUST/library/std/Cargo.toml"
elif ! grep -qF "$SOSO_DEP" "$RUST/library/std/Cargo.toml"; then
  echo "apply-patches: la dependencia soso-rt estaba desfasada; se reescribe"
  sed -i "s|^soso-rt = {.*|$SOSO_DEP|" "$RUST/library/std/Cargo.toml"
fi

# `env_consts`: el bloque de soso tenía FAMILY/OS/ARCH y le faltaban las cinco
# de bibliotecas y ejecutables, así que `std` no compilaba
# («cannot find value `DLL_PREFIX` in module `os`», y cuatro más). Van vacías
# como las de hermit, no `lib`/`.so`: soso no carga objetos compartidos —lo
# midió T32, no hay `dlopen`—, y anunciar una convención de nombres para algo
# que no existe es justo la clase de mentira que este plan persigue.
# Primero se repara el bloque corto, porque el guardián de abajo lo daría por
# bueno al ver `target_os = "soso"` (idempotente no es convergente).
perl -i -0pe 's/\#\[cfg\(target_os = "soso"\)\]\npub mod os \{\n    pub const FAMILY: &str = "unix";\n    pub const OS: &str = "soso";\n    pub const ARCH: &str = env!\("STD_ENV_ARCH"\);\n\}/#[cfg(target_os = "soso")]\npub mod os {\n    pub const FAMILY: &str = "unix";\n    pub const OS: &str = "soso";\n    pub const DLL_PREFIX: &str = "";\n    pub const DLL_SUFFIX: &str = "";\n    pub const DLL_EXTENSION: &str = "";\n    pub const EXE_SUFFIX: &str = "";\n    pub const EXE_EXTENSION: &str = "";\n}/s' \
  "$RUST/library/std/src/sys/env_consts.rs"

grep -q 'target_os = "soso"' "$RUST/library/std/src/sys/env_consts.rs" || \
  perl -i -0pe 's/(#\[cfg\(target_os = "hermit"\)\]\npub mod os \{)/#[cfg(target_os = "soso")]\npub mod os {\n    pub const FAMILY: \&str = "unix";\n    pub const OS: \&str = "soso";\n    pub const DLL_PREFIX: \&str = "";\n    pub const DLL_SUFFIX: \&str = "";\n    pub const DLL_EXTENSION: \&str = "";\n    pub const EXE_SUFFIX: \&str = "";\n    pub const EXE_EXTENSION: \&str = "";\n}\n\n$1/s' \
    "$RUST/library/std/src/sys/env_consts.rs"

# `std::env::consts::ARCH` ya es `env!("STD_ENV_ARCH")`; esta línea en el bloque
# soso no la usa nadie y con `build.warnings` en deny acaba en error.
sed -i '/#\[cfg(target_os = "soso")\]/,/^\}/ { /pub const ARCH: &str = env!("STD_ENV_ARCH");/d; }' \
  "$RUST/library/std/src/sys/env_consts.rs"

# --- ramas de `cfg_select` para soso -----------------------------------------
# Estos `cfg_select` **no tienen rama por defecto**: si soso no aparece, el
# error es «none of the predicates in this `cfg_select` evaluated to true», que
# no dice cuál falta. Cada uno se añade delante de la rama de hermit.

# Errores de E/S: implementación propia (`sys/io/error/soso.rs`), no `generic`,
# porque soso tiene errnos de verdad y `generic` contesta «operation
# successful» a cualquier fallo.
grep -q 'target_os = "soso"' "$RUST/library/std/src/sys/io/error/mod.rs" || \
  perl -i -0pe 's/(    target_os = "hermit" => \{\n        mod hermit;)/    target_os = "soso" => {\n        mod soso;\n        pub use soso::*;\n    }\n$1/s' \
    "$RUST/library/std/src/sys/io/error/mod.rs"

# Alocador (T72): sin esta rama el `cfg_select` no define `imp` y std no
# compila. El módulo `sys/alloc/soso.rs` lo copia el árbol y delega en
# `soso-rt`, que usa el mismo `soso-alloc` que libsoso. La marca es
# `use soso as imp`, no un `target_os = "soso"` cualquiera: la línea de
# `alloc_zeroed` ya lo nombra y no trae el módulo.
grep -q 'use soso as imp' "$RUST/library/std/src/sys/alloc/mod.rs" || \
  perl -i -0pe 's/(    target_os = "hermit" => \{\n        mod hermit;\n        use hermit as imp;\n    \})/    target_os = "soso" => {\n        mod soso;\n        use soso as imp;\n    }\n$1/s' \
    "$RUST/library/std/src/sys/alloc/mod.rs"

# Segundo `cfg_select` del mismo fichero: `alloc_zeroed`. soso no tiene
# calloc; esta rama reserva y pone ceros.
grep -q 'target_os = "soso", target_os = "solid_asp3"' "$RUST/library/std/src/sys/alloc/mod.rs" || \
  perl -i -0pe 's/any\(target_os = "hermit", target_os = "solid_asp3"/any(target_os = "hermit", target_os = "soso", target_os = "solid_asp3"/s' \
    "$RUST/library/std/src/sys/alloc/mod.rs"

# Argumentos y entorno: soso los tiene de verdad, así que no puede caer en la
# rama `_ => unsupported`, que devuelve lista vacía y `getenv` = None. Hay que
# tocar dos sitios por fichero: la rama del `cfg_select` y el `#[cfg(any(…))]`
# que habilita `common` (de donde salen los iteradores `Args` y `Env`).
for m in args env; do
  f="$RUST/library/std/src/sys/$m/mod.rs"
  grep -q 'target_os = "soso"' "$f" || {
    perl -i -0pe 's/(\n#\[cfg\(any\(\n)/$1    target_os = "soso",\n/s' "$f"
    perl -i -0pe 's/(    _ => \{\n        mod unsupported;)/    target_os = "soso" => {\n        mod soso;\n        pub use soso::*;\n    }\n$1/s' "$f"
  }
done

# Aleatoriedad: el `cfg_select` de `sys/random/mod.rs` tiene rama `_ => {}`, que
# **no** define `fill_bytes` — así que no cubre a nadie, sólo evita el error del
# `cfg_select` y deja el fallo más tarde y peor explicado.
grep -q 'target_os = "soso"' "$RUST/library/std/src/sys/random/mod.rs" || \
  perl -i -0pe 's/(    target_os = "hermit" => \{\n        mod hermit;\n        pub use hermit::fill_bytes;\n    \})/    target_os = "soso" => {\n        mod soso;\n        pub use soso::fill_bytes;\n    }\n$1/s' \
    "$RUST/library/std/src/sys/random/mod.rs"

# TLS: soso va con la rama de hermit/xous del `guard`, cuyo comentario describe
# exactamente nuestro caso —«std es el único runtime, así que llama él mismo a
# los destructores»—: la PAL invoca `thread_local::destructors::run()` desde
# `runtime_entry`. La rama por defecto (`key`) pide una API de claves TLS que
# soso no tiene y que no necesita, porque el cargador monta PT_TLS.
grep -q 'target_os = "soso", target_os = "xous"' "$RUST/library/std/src/sys/thread_local/mod.rs" || \
  perl -i -0pe 's/any\(target_os = "hermit", target_os = "xous"\)/any(target_os = "hermit", target_os = "soso", target_os = "xous")/s' \
    "$RUST/library/std/src/sys/thread_local/mod.rs"

# Sincronización (T73): soso tiene hilos y futex, así que va a la rama de
# futex, no a `no_threads` —que lleva un `compile_error!` bajo
# `target_has_threads` justamente para que nadie la use por descarte—.
# Cinco sitios: el despacho del futex y los cuatro primitivos.
grep -q 'target_os = "soso"' "$RUST/library/std/src/sys/sync/futex/mod.rs" || \
  perl -i -0pe 's/(    target_os = "hermit" => \{\n        mod hermit;\n        pub use hermit::\*;\n    \})/    target_os = "soso" => {\n        mod soso;\n        pub use soso::*;\n    }\n$1/s' \
    "$RUST/library/std/src/sys/sync/futex/mod.rs"

for m in mutex condvar once rwlock; do
  f="$RUST/library/std/src/sys/sync/$m/mod.rs"
  grep -q 'target_os = "soso"' "$f" || \
    perl -i -0pe 's/        target_os = "hermit",\n/        target_os = "hermit",\n        target_os = "soso",\n/s' "$f"
done

# stdio: sin rama propia cae en `unsupported`, cuyo `write` devuelve Ok(len)
# sin escribir nada — un humo con `println!` parecería pasar y no lo haría.
grep -q 'target_os = "soso"' "$RUST/library/std/src/sys/stdio/mod.rs" || \
  perl -i -0pe 's/(    _ => \{\n        mod unsupported;)/    target_os = "soso" => {\n        mod soso;\n        pub use soso::*;\n    }\n$1/s' \
    "$RUST/library/std/src/sys/stdio/mod.rs"

# memcpy/memset para enlaces estáticos sin libc (igual que zkvm; incompatible con
# rust.std-features en config.toml cuando download-rustc = true).
SESSION="$RUST/src/bootstrap/src/core/session.rs"
if [[ -f "$SESSION" ]] && ! grep -q 'target.contains("soso")' "$SESSION"; then
  sed -i 's/if target.contains("zkvm") {/if target.contains("zkvm") || target.contains("soso") {/' \
    "$SESSION"
fi

# C-001: memmap2 0.2.3 cae en stub.rs para soso y esa firma no acepta
# `populate`. Sólo si el árbol es el de rust completo: el test de la receta
# exporta únicamente library/std y no tiene Cargo.toml.
if [[ -f "$RUST/Cargo.toml" ]]; then
  MEMMAP_DST="$RUST/src/soso-memmap2"
  rm -rf "$MEMMAP_DST"
  mkdir -p "$MEMMAP_DST"
  cp -a "$ROOT/config/rust-soso/vendor/memmap2/." "$MEMMAP_DST/"
  if ! grep -q 'path = "src/soso-memmap2"' "$RUST/Cargo.toml"; then
    printf '\n[patch.crates-io]\nmemmap2 = { path = "src/soso-memmap2" }\n' >> "$RUST/Cargo.toml"
  fi
fi

echo "apply-patches: OK → $RUST"
