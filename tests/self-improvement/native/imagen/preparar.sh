#!/usr/bin/env bash
# T42, paso 5: el constructor de imágenes arrancables dentro de soso.
#
# `bootloader` 0.11 compila sus etapas en el `build.rs` (cargo anidado con
# build-std para 16 bits y UEFI, linker PE, `llvm-objcopy`): nada de eso existe en
# el guest (C-117/C-120). Aquí se parte de las etapas ya compiladas por el
# `build.rs` del host (`blobs/`, dependencia EXTERNA declarada) y el guest ejecuta
# el resto: `DiskImageBuilder` (ESP FAT, GPT, MBR) sobre el kernel.
#
# Uso: preparar.sh [dir-out-del-build-del-bootloader]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../../.." && pwd)"
SRC="$ROOT/tests/self-improvement/native/imagen"
DST="$ROOT/rootfs/var/t42/img"
OUT="${1:-$(ls -d "$ROOT"/target/debug/build/bootloader-*/out | head -1)}"
BL="$(ls -d "$HOME"/.cargo/registry/src/*/bootloader-0.11.15 | head -1)"
rm -rf "$DST"
mkdir -p "$DST/blobs" "$DST/.cargo"
cp "$SRC/Cargo.toml" "$DST/"
cp -r "$SRC/imagen" "$DST/imagen"
cp -r "$BL" "$DST/bootloader-parcheado"
rm -rf "$DST/bootloader-parcheado/target" "$DST/bootloader-parcheado/tests" "$DST/bootloader-parcheado/docs"
for b in bootloader-x86_64-bios-boot-sector.bin bootloader-x86_64-bios-stage-2.bin \
         bootloader-x86_64-bios-stage-3.bin bootloader-x86_64-bios-stage-4.bin \
         bootloader-x86_64-uefi.efi; do
  cp "$OUT/bin/$b" "$DST/blobs/$b"
done
python3 - "$DST/bootloader-parcheado" <<'PY'
import re, sys
d = sys.argv[1]
t = open(d + "/Cargo.toml").read()
t = re.sub(r'\n\[\[test\]\]\nname = "[^"]*"\npath = "[^"]*"\n', "\n", t)
t = re.sub(r'\n\[build-dependencies\.[^\]]*\](\n[^\[\n][^\n]*)*', "", t)
t = t.replace('build = "build.rs"', 'build = "build.rs"')
open(d + "/Cargo.toml", "w").write(t)
open(d + "/build.rs", "w").write('''// Parcheado (T42): las etapas del cargador ya están compiladas en `../blobs`.
fn main() {
    let b = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../blobs");
    let p = |n: &str| b.join(n).display().to_string();
    println!("cargo:rustc-env=BIOS_BOOT_SECTOR_PATH={}", p("bootloader-x86_64-bios-boot-sector.bin"));
    println!("cargo:rustc-env=BIOS_STAGE_2_PATH={}", p("bootloader-x86_64-bios-stage-2.bin"));
    println!("cargo:rustc-env=BIOS_STAGE_3_PATH={}", p("bootloader-x86_64-bios-stage-3.bin"));
    println!("cargo:rustc-env=BIOS_STAGE_4_PATH={}", p("bootloader-x86_64-bios-stage-4.bin"));
    println!("cargo:rustc-env=UEFI_BOOTLOADER_PATH={}", p("bootloader-x86_64-uefi.efi"));
    println!("cargo:rerun-if-changed=../blobs");
}
''')
PY
# `tempfile` no tiene backend para `target_os = "soso"` («operation not supported»)
# y `env::temp_dir()` entra en pánico: la única pieza que `bootloader` usa de él es
# `NamedTempFile` (un fichero temporal con `path()` que se borra al soltarlo); se
# sustituye por uno mínimo en `/tmp`.
python3 - "$DST/bootloader-parcheado/src/lib.rs" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
s = s.replace("use tempfile::NamedTempFile;", """use std::sync::atomic::{AtomicU32, Ordering};

/// Fichero temporal en `/tmp` que se borra al soltarlo (T42: sustituye a `tempfile`).
pub struct NamedTempFile(PathBuf);

impl NamedTempFile {
    fn nuevo() -> std::io::Result<Self> {
        static N: AtomicU32 = AtomicU32::new(0);
        let ruta = PathBuf::from(format!(
            "/tmp/bootloader-{}-{}.fat",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::File::create(&ruta)?;
        Ok(Self(ruta))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn close(self) -> std::io::Result<()> {
        let r = std::fs::remove_file(&self.0);
        std::mem::forget(self);
        r
    }
}

impl Drop for NamedTempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}""")
s = s.replace("NamedTempFile::new()", "NamedTempFile::nuevo()")
open(p, "w").write(s)
PY
cat > "$DST/.cargo/config.toml" <<'CFG'
# `getrandom` 0.4 (por `tempfile`) no conoce `target_os = "soso"`: se le pide el
# backend de RDRAND (QEMU con `-cpu max` lo trae; soso no tiene /dev/random para él).
[build]
rustflags = ["--cfg", "getrandom_backend=\"rdrand\""]

[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
CFG
cd "$DST"
cargo vendor --versioned-dirs vendor > /dev/null
[ -f "$SRC/Cargo.lock" ] && cp "$DST/Cargo.lock" "$SRC/Cargo.lock" || cp "$DST/Cargo.lock" "$SRC/Cargo.lock"
find . -type f ! -name MANIFEST.sha256 ! -path './vendor/*/target/*' | LC_ALL=C sort | xargs sha256sum > MANIFEST.sha256
echo "preparado: $DST ($(wc -l < MANIFEST.sha256) ficheros, $(du -sh vendor | cut -f1) vendidos, blobs $(du -sh blobs | cut -f1))"
