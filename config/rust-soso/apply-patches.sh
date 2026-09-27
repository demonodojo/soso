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
  # C-002: getrandom 0.3.3 no tiene backend para target_os = soso.
  GETRANDOM_DST="$RUST/src/soso-getrandom"
  rm -rf "$GETRANDOM_DST"
  mkdir -p "$GETRANDOM_DST"
  cp -a "$ROOT/config/rust-soso/vendor/getrandom/." "$GETRANDOM_DST/"
  if ! grep -q 'path = "src/soso-getrandom"' "$RUST/Cargo.toml"; then
    printf 'getrandom = { path = "src/soso-getrandom" }\n' >> "$RUST/Cargo.toml"
  fi
  # C-008: libc deja vacío un target no soportado. soso necesita los tipos C.
  LIBC_REG=$(find "$HOME/.cargo/registry/src" -maxdepth 2 -type d -name 'libc-0.2.189' | head -1)
  LIBC_DST="$RUST/src/soso-libc"
  if [[ -n "$LIBC_REG" ]]; then
    rm -rf "$LIBC_DST"
    mkdir -p "$LIBC_DST"
    cp -a "$LIBC_REG/." "$LIBC_DST/"
    cp "$ROOT/config/rust-soso/vendor/libc/soso.rs" "$LIBC_DST/src/soso.rs"
    python3 - "$LIBC_DST/src/lib.rs" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
old = """    } else {
        // non-supported targets: empty...
    }"""
new = """    } else if #[cfg(target_os = "soso")] {
        mod primitives;
        pub use crate::primitives::*;

        mod soso;
        pub use crate::soso::*;
    } else {
        // non-supported targets: empty...
    }"""
if 'target_os = "soso"' not in text:
    if old not in text:
        sys.exit("apply-patches: libc sin la rama vacía")
    text = text.replace(old, new, 1)
# El check niega avisos. `pub use new::*` queda vacío en soso.
text = text.replace(
    "#[allow(unused_imports)] // needed while the module is empty on some platforms\npub use new::*;",
    "#[allow(unused_imports, unreachable_pub)] // needed while the module is empty on some platforms\npub use new::*;",
    1,
)
path.write_text(text)
PY
    if ! grep -q 'path = "src/soso-libc"' "$RUST/Cargo.toml"; then
      printf 'libc = { path = "src/soso-libc" }\n' >> "$RUST/Cargo.toml"
    fi
  fi
  # C-010: ctrlc 3.5.1 sólo tiene plataforma unix y Windows.
  CTRLC_REG=$(find "$HOME/.cargo/registry/src" -maxdepth 2 -type d -name 'ctrlc-3.5.1' | head -1)
  CTRLC_DST="$RUST/src/soso-ctrlc"
  if [[ -n "$CTRLC_REG" ]]; then
    rm -rf "$CTRLC_DST"
    mkdir -p "$CTRLC_DST"
    cp -a "$CTRLC_REG/." "$CTRLC_DST/"
    cp "$ROOT/config/rust-soso/vendor/ctrlc/soso.rs" "$CTRLC_DST/src/platform/soso.rs"
    python3 - "$CTRLC_DST/src/platform/mod.rs" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
extra = """
#[cfg(target_os = "soso")]
mod soso;

#[cfg(target_os = "soso")]
pub use self::soso::*;
"""
if 'target_os = "soso"' not in text:
    path.write_text(text.rstrip() + "\n" + extra)
PY
    python3 - "$CTRLC_DST/src/error.rs" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
old = "fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {"
new = "fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {"
if old in text:
    path.write_text(text.replace(old, new, 1))
PY
    if ! grep -q 'path = "src/soso-ctrlc"' "$RUST/Cargo.toml"; then
      printf 'ctrlc = { path = "src/soso-ctrlc" }\n' >> "$RUST/Cargo.toml"
    fi
  fi
  # C-011: soso descarta dylib. Sin rlib, rustc-main no encuentra rustc_driver.
  DRIVER_TOML="$RUST/compiler/rustc_driver/Cargo.toml"
  if [[ -f "$DRIVER_TOML" ]] && grep -q 'crate-type = \["dylib"\]' "$DRIVER_TOML"; then
    perl -i -pe 's/crate-type = \["dylib"\]/crate-type = ["dylib", "rlib"]/' "$DRIVER_TOML"
  fi
  # C-012: download-rustc copia el rustc de CI (linux). Un host soso se compila.
  COMPILE_RS="$RUST/src/bootstrap/src/core/build_steps/compile.rs"
  if [[ -f "$COMPILE_RS" ]]; then
    python3 - "$COMPILE_RS" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
repls = [
(
"""            } else if builder.download_rustc() && compiler.stage != builder.top_stage {
                host_dir.join("ci-rustc-sysroot")""",
"""            } else if builder.download_rustc()
                && compiler.host == builder.config.host_target
                && compiler.stage != builder.top_stage
            {
                host_dir.join("ci-rustc-sysroot")""",
),
(
"""        if builder.download_rustc() && compiler.stage != 0 {
            assert_eq!(
                builder.config.host_target, compiler.host,
                "Cross-compiling is not yet supported with `download-rustc`",
            );
""",
"""        // C-012: el rustc de CI es el host de config.toml. Otro host se compila.
        if builder.download_rustc()
            && compiler.stage != 0
            && compiler.host == builder.config.host_target
        {
""",
),
(
"""        if builder.download_rustc() && build_compiler.stage != 0 {
            trace!(stage = build_compiler.stage, "`download_rustc` requested");
""",
"""        if builder.download_rustc()
            && build_compiler.stage != 0
            && target == builder.config.host_target
        {
            trace!(stage = build_compiler.stage, "`download_rustc` requested");
""",
),
(
"""        if builder.download_rustc() {
            trace!("`download-rustc` requested, reusing CI compiler for stage > 0");
""",
"""        if builder.download_rustc() && target_compiler.host == builder.config.host_target {
            trace!("`download-rustc` requested, reusing CI compiler for stage > 0");
""",
),
]
changed = False
for old, new in repls:
    if old in text:
        text = text.replace(old, new, 1)
        changed = True
    elif new not in text:
        sys.exit(f"apply-patches: no está el bloque de download-rustc:\n{old[:80]}")
if changed:
    path.write_text(text)
PY
  fi
  # C-014: CMAKE_SYSTEM_NAME=Generic deja LLVM_ON_UNIX sin definir y
  # file_status no tiene getSize. soso es familia unix; Linux enciende esa rama.
  LLVM_RS="$RUST/src/bootstrap/src/core/build_steps/llvm.rs"
  if [[ -f "$LLVM_RS" ]]; then
    python3 - "$LLVM_RS" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
old = """        } else if target.contains("watchos") {
            cfg.define("CMAKE_SYSTEM_NAME", "watchOS");
        } else if target.contains("none") {"""
new = """        } else if target.contains("watchos") {
            cfg.define("CMAKE_SYSTEM_NAME", "watchOS");
        } else if target.contains("soso") {
            // C-014: familia unix. Generic deja LLVM_ON_UNIX sin definir.
            cfg.define("CMAKE_SYSTEM_NAME", "Linux");
        } else if target.contains("none") {"""
if old in text:
    path.write_text(text.replace(old, new, 1))
elif new not in text:
    sys.exit("apply-patches: no está el bloque CMAKE_SYSTEM_NAME de llvm.rs")
PY
  fi
  # C-015: al cruzar, rustc_llvm cambia el triple del -I/-L del llvm-config
  # del host. El host es ci-llvm; el LLVM local de soso está en llvm/.
  RUSTC_LLVM_BUILD="$RUST/compiler/rustc_llvm/build.rs"
  if [[ -f "$RUSTC_LLVM_BUILD" ]]; then
    python3 - "$RUSTC_LLVM_BUILD" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
fn = '''
// C-015: el host descarga ci-llvm y un LLVM local queda en llvm/.
// Cambiar sólo el triple apunta a un directorio que no existe.
fn retarget_host_path(path: &str, host: &str, target: &str) -> String {
    let replaced = path.replace(host, target);
    if Path::new(&replaced).exists() {
        return replaced;
    }
    let alt = replaced.replacen("ci-llvm", "llvm", 1);
    if alt != replaced && Path::new(&alt).exists() {
        return alt;
    }
    let alt = replaced.replacen("/llvm/", "/ci-llvm/", 1);
    if alt != replaced && Path::new(&alt).exists() {
        return alt;
    }
    replaced
}

'''
anchor = "fn detect_llvm_link() -> (&'static str, &'static str) {\n"
if "fn retarget_host_path(" not in text:
    if anchor not in text:
        sys.exit("apply-patches: no está detect_llvm_link en rustc_llvm/build.rs")
    text = text.replace(anchor, fn + anchor, 1)
old_i = '''        // Include path contains host directory, replace it with target
        if is_crossed && flag.starts_with("-I") {
            cfg.flag(flag.replace(&host, &target));
            continue;
        }'''
new_i = '''        // Include path contains host directory, replace it with target
        if is_crossed && flag.starts_with("-I") {
            let dir = flag.strip_prefix("-I").unwrap();
            cfg.flag(format!("-I{}", retarget_host_path(dir, &host, &target)));
            continue;
        }'''
old_l = '''            if let Some(stripped) = lib.strip_prefix("-LIBPATH:") {
                println!("cargo:rustc-link-search=native={}", stripped.replace(&host, &target));
            } else if let Some(stripped) = lib.strip_prefix("-L") {
                println!("cargo:rustc-link-search=native={}", stripped.replace(&host, &target));
            }'''
new_l = '''            if let Some(stripped) = lib.strip_prefix("-LIBPATH:") {
                println!(
                    "cargo:rustc-link-search=native={}",
                    retarget_host_path(stripped, &host, &target)
                );
            } else if let Some(stripped) = lib.strip_prefix("-L") {
                println!(
                    "cargo:rustc-link-search=native={}",
                    retarget_host_path(stripped, &host, &target)
                );
            }'''
for old, new in ((old_i, new_i), (old_l, new_l)):
    if old in text:
        text = text.replace(old, new, 1)
    elif new not in text:
        sys.exit("apply-patches: no está el reemplazo de rutas de llvm-config")
path.write_text(text)
PY
  fi
  # C-016: --link-static contra el llvm-config de CI falla: no hay .a.
  # El llvm-config del target sí los lista, y en este cruce se puede ejecutar.
  if [[ -f "$RUSTC_LLVM_BUILD" ]]; then
    python3 - "$RUSTC_LLVM_BUILD" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
old = """    let is_crossed = target != host;

    let components = output(Command::new(&llvm_config).arg("--components"));"""
new = """    let is_crossed = target != host;

    // C-016: el llvm-config del host es el de CI y no tiene .a.
    // El del target sí, y en este cruce se puede ejecutar.
    let llvm_config_libs = {
        let alt = retarget_host_path(&llvm_config.to_string_lossy(), &host, &target);
        let alt_path = PathBuf::from(&alt);
        if is_crossed && alt_path != llvm_config && alt_path.exists() {
            alt_path
        } else {
            llvm_config.clone()
        }
    };

    let components = output(Command::new(&llvm_config).arg("--components"));"""
repls = [
(old, new),
(
"""    let mut cmd = Command::new(&llvm_config);
    cmd.arg(llvm_link_arg).arg("--libs");""",
"""    let mut cmd = Command::new(&llvm_config_libs);
    cmd.arg(llvm_link_arg).arg("--libs");""",
),
(
"""    let mut cmd = Command::new(&llvm_config);
    cmd.arg(llvm_link_arg).arg("--ldflags");""",
"""    let mut cmd = Command::new(&llvm_config_libs);
    cmd.arg(llvm_link_arg).arg("--ldflags");""",
),
]
for old, new in repls:
    if old in text:
        text = text.replace(old, new, 1)
    elif new not in text:
        sys.exit("apply-patches: no está el bloque de --libs de rustc_llvm")
path.write_text(text)
PY
  fi
  # C-015: instalar el std de soso no puede borrar el libstd del host. Sin él
  # el build script de rustc_llvm no recompila.
  if [[ -f "$COMPILE_RS" ]]; then
    python3 - "$COMPILE_RS" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
old = """            if builder.download_rustc() {
                // Ensure there are no CI-rustc std artifacts.
                let _ = fs::remove_dir_all(&libdir);
                let _ = fs::remove_dir_all(&hostdir);
            }"""
new = """            if builder.download_rustc() {
                // Ensure there are no CI-rustc std artifacts for this target.
                let _ = fs::remove_dir_all(&libdir);
                // C-015: si el target no es el host, hostdir es el libstd de CI
                // con el que se compilan los build scripts. No borrarlo.
                if target == compiler.host {
                    let _ = fs::remove_dir_all(&hostdir);
                }
            }"""
if old in text:
    path.write_text(text.replace(old, new, 1))
elif new not in text:
    sys.exit("apply-patches: no está el borrado de hostdir en StdLink")
PY
  fi
  # C-008: offload y Enzyme cargan con libloading::Library, sólo en unix/Windows.
  OFF_SRC="$ROOT/config/rust-soso/compiler/rustc_codegen_llvm/src/llvm/offload_ffi.rs"
  OFF_DST="$RUST/compiler/rustc_codegen_llvm/src/llvm/offload_ffi.rs"
  if [[ -f "$OFF_SRC" && -d "$(dirname "$OFF_DST")" ]]; then
    cp "$OFF_SRC" "$OFF_DST"
  fi
  ENZYME="$RUST/compiler/rustc_codegen_llvm/src/llvm/enzyme_ffi.rs"
  if [[ -f "$ENZYME" ]] && ! grep -q 'dynamic libraries are not supported on soso' "$ENZYME"; then
    python3 - "$ENZYME" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
text = text.replace(
    "        lib: libloading::Library,\n",
    "        #[cfg(not(target_os = \"soso\"))]\n        lib: libloading::Library,\n",
    1,
)
text = text.replace(
    "    fn load_ptr_by_symbol_mut_void(",
    "    #[cfg(not(target_os = \"soso\"))]\n    fn load_ptr_by_symbol_mut_void(",
    1,
)
old = """        #[allow(non_snake_case)]
        fn call_dynamic("""
new = """        #[cfg(target_os = "soso")]
        fn call_dynamic(
            sysroot: &rustc_session::config::Sysroot,
        ) -> Result<Self, EnzymeLibraryError> {
            let _ = sysroot;
            Err(EnzymeLibraryError::LoadFailed {
                err: "dynamic libraries are not supported on soso".to_string(),
            })
        }

        #[cfg(not(target_os = "soso"))]
        #[allow(non_snake_case)]
        fn call_dynamic("""
if old not in text:
    sys.exit("apply-patches: enzyme call_dynamic no encontrado")
text = text.replace(old, new, 1)
path.write_text(text)
PY
  fi
  if [[ -f "$ENZYME" ]]; then
    python3 - "$ENZYME" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
changed = False
for name in ("load_ptrs_by_symbols_mut_void", "load_ptrs_by_symbols_fn"):
    old = f"    macro_rules! {name} {{"
    new = f"    #[cfg(not(target_os = \"soso\"))]\n{old}"
    if new not in text and old in text:
        text = text.replace(old, new, 1)
        changed = True
if changed:
    path.write_text(text)
PY
  fi
  # C-003: path_to_c_string (el único uso de CString) no existe en soso.
  # El check niega avisos, así que el import muerto para el build.
  FSUTIL="$RUST/compiler/rustc_fs_util/src/lib.rs"
  if [[ -f "$FSUTIL" ]] && grep -q 'use std::ffi::{CString, OsStr};' "$FSUTIL"; then
    perl -i -0pe 's/use std::ffi::\{CString, OsStr\};/use std::ffi::OsStr;\n#[cfg(any(unix, windows, all(target_os = "wasi", target_env = "p1")))]\nuse std::ffi::CString;/' \
      "$FSUTIL"
  fi
  # C-009: rustc_codegen_llvm llama a path_to_c_string. En soso los bytes
  # del camino salen de OsStrExt, igual que en unix.
  if [[ -f "$FSUTIL" ]]; then
    python3 - "$FSUTIL" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
old_cfg = '#[cfg(any(unix, windows, all(target_os = "wasi", target_env = "p1")))]\nuse std::ffi::CString;'
new_cfg = '#[cfg(any(unix, windows, target_os = "soso", all(target_os = "wasi", target_env = "p1")))]\nuse std::ffi::CString;'
if old_cfg in text:
    text = text.replace(old_cfg, new_cfg, 1)
fn = '''#[cfg(target_os = "soso")]
pub fn path_to_c_string(p: &Path) -> CString {
    use std::os::soso::ffi::OsStrExt;
    let p: &OsStr = p.as_ref();
    CString::new(p.as_bytes()).unwrap()
}
'''
anchor = '''#[cfg(windows)]
pub fn path_to_c_string(p: &Path) -> CString {
    CString::new(p.to_str().unwrap()).unwrap()
}
'''
if '#[cfg(target_os = "soso")]\npub fn path_to_c_string' not in text:
    if anchor not in text:
        sys.exit("apply-patches: path_to_c_string de windows no encontrado")
    text = text.replace(anchor, anchor + "\n" + fn, 1)
path.write_text(text)
PY
  fi
  # C-004: current_dll_path llama a dll_path, que sólo existe en unix,
  # Windows y WASI. try_canonicalize sólo se usa dentro de esas funciones.
  FILESEARCH="$RUST/compiler/rustc_session/src/filesearch.rs"
  if [[ -f "$FILESEARCH" ]] && grep -q '^use rustc_fs_util::try_canonicalize;$' "$FILESEARCH"; then
    perl -i -pe 's/^use rustc_fs_util::try_canonicalize;$/#[cfg(any(unix, windows))]\nuse rustc_fs_util::try_canonicalize;/' \
      "$FILESEARCH"
  fi
  # C-005: host_dylib pide libloading::Library, que sólo existe en unix/Windows.
  META_SRC="$ROOT/config/rust-soso/compiler/rustc_metadata/src"
  META_DST="$RUST/compiler/rustc_metadata/src"
  if [[ -d "$META_SRC" && -d "$META_DST" ]]; then
    cp "$META_SRC/host_dylib.rs" "$META_SRC/lib.rs" "$META_DST/"
  fi
  # C-006: libc no publica size_t para soso. En x86_64 es usize.
  LLVM_LIB="$RUST/compiler/rustc_llvm/src/lib.rs"
  if [[ -f "$LLVM_LIB" ]]; then
    python3 - "$LLVM_LIB" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
good = """#[cfg(not(target_os = "soso"))]
use libc::size_t;
#[cfg(target_os = "soso")]
#[allow(non_camel_case_types)]
pub type size_t = usize;
#[cfg(target_os = "soso")]
use libc as _;
"""
if "non_camel_case_types" in text and "use libc as _;" in text and "\\nuse libc" not in text:
    sys.exit(0)
mid = """#[cfg(not(target_os = "soso"))]
use libc::size_t;
#[cfg(target_os = "soso")]
pub type size_t = usize;
"""
broken = """#[cfg(not(target_os = "soso"))]
#[cfg(not(target_os = "soso"))]\\nuse libc::size_t;\\n#[cfg(target_os = "soso")]\\n#[allow(non_camel_case_types)]\\npub type size_t = usize;\\n#[cfg(target_os = "soso")]\\nuse libc as _;
#[cfg(target_os = "soso")]
pub type size_t = usize;
"""
if broken in text:
    text = text.replace(broken, good, 1)
elif mid in text:
    text = text.replace(mid, good, 1)
elif "use libc::size_t;\n" in text:
    text = text.replace("use libc::size_t;\n", good, 1)
else:
    sys.exit(0)
path.write_text(text)
PY
  fi
  # C-007: rustc_sanitizers pide size_t y c_char, que libc no publica en soso.
  SAN_FFI="$RUST/compiler/rustc_sanitizers/src/ignorelist/ffi.rs"
  SAN_MOD="$RUST/compiler/rustc_sanitizers/src/ignorelist/mod.rs"
  if [[ -f "$SAN_FFI" && -f "$SAN_MOD" ]]; then
    python3 - "$SAN_FFI" "$SAN_MOD" <<'PY'
import pathlib, sys
ffi, mod = map(pathlib.Path, sys.argv[1:])
ffi_text = ffi.read_text()
ffi_good = """#[cfg(not(target_os = "soso"))]
use libc::size_t;
#[cfg(target_os = "soso")]
#[allow(non_camel_case_types)]
pub(crate) type size_t = usize;
#[cfg(target_os = "soso")]
use libc as _;
"""
if "pub(crate) type size_t = usize;" not in ffi_text and "use libc::size_t;\n" in ffi_text:
    ffi.write_text(ffi_text.replace("use libc::size_t;\n", ffi_good, 1))
mod_text = mod.read_text()
new = """        #[cfg(not(target_os = "soso"))]
        use libc::c_char;
        #[cfg(target_os = "soso")]
        use std::ffi::c_char;
        let c_ptrs: Vec<*const c_char> = c_paths.iter().map(|c| c.as_ptr()).collect();"""
old = "        let c_ptrs: Vec<*const libc::c_char> = c_paths.iter().map(|c| c.as_ptr()).collect();"
broken = """                #[cfg(not(target_os = "soso"))]
        use libc::c_char;
        #[cfg(target_os = "soso")]
        use std::ffi::c_char;
        let c_ptrs: Vec<*const c_char> = c_paths.iter().map(|c| c.as_ptr()).collect();"""
if broken in mod_text:
    mod.write_text(mod_text.replace(broken, new, 1))
elif old in mod_text:
    mod.write_text(mod_text.replace(old, new, 1))
PY
  fi
  if [[ -f "$FILESEARCH" ]] && ! grep -q 'target_os = "soso"' "$FILESEARCH"; then
    perl -i -0pe 's/(#\[cfg\(target_os = "wasi"\)\]\npub unsafe fn dll_path\(_function: \*mut std::ffi::c_void\) -> Result<PathBuf, String> \{\n    Err\("dll_path is not supported on WASI"\.to_string\(\)\)\n\})/$1\n\n#[cfg(target_os = "soso")]\npub unsafe fn dll_path(_function: *mut std::ffi::c_void) -> Result<PathBuf, String> {\n    Err("dll_path is not supported on soso".to_string())\n}/' \
      "$FILESEARCH"
  fi
fi

echo "apply-patches: OK → $RUST"
