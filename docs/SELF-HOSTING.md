# Self-hosting de soso — ruta A

## Hito 0 — Bucle remoto ✓

Ver sección anterior. `soso-forja all --host 10.0.2.2` + revert del shim.

## Hito 1 — ABI ✓

Syscalls 70–82, `SpawnIo` argv/envp, ELF perezoso + PT_TLS, `SOSOFS11`.

## Hito 2 — soso-std

- [`crates/soso-std`](../crates/soso-std): `fs`, `io`, `net`, `pipe`, `process`, `sync`, `thread`, `time`, `env`
- [`user/hola-std`](../user/hola-std), [`user/soso-std-test`](../user/soso-std-test)
- `init test` ejecuta humo de `hola-std`, `soso-std-test`, `soso-forja local`, `soso-rustc --version`
- PAL: [`config/rust-soso/`](../config/rust-soso/) + [`scripts/soso-rust-bootstrap.sh`](../scripts/soso-rust-bootstrap.sh)
- **`cargo xtask rust-bootstrap`** / **`rust-build-std`**: parches + `./x.py build library/std`
- **`crates/soso-rt`**: ABI syscalls para libstd
- **`cargo xtask sync-src`**: copia fuentes editables a `/src/soso` en la imagen (PACK_SKIP)
- Pendiente: fork real de `rust-lang/rust`, `mkfs-soso` nativo en guest
- **`/bin/soso-test-sosofs`**: tests rename/truncate/O_EXCL vía VFS (equivalente guest de `cargo test -p sosofs rename`)

## Hito 3 — Toolchain

- **`soso-forja local`**: cache incremental por hash (`/var/forja-cache`)
- **`soso-forja build-local`**: copia `/var/forja-out/*` → staging OTA
- **`soso-forja all-local`**: build-local + `soso-update aplicar --local`
- **`cargo xtask forja-out`**: host → `rootfs/var/forja-out/` tras `release`
- **`soso-update --channel dev`**: URL canal desarrollo (`UPD_CHANNEL_DEV`)
- **`SYS_GETENV` (83)**: entorno por proceso; herencia en spawn; `PATH`/`HOME` en init
- **`spawn_io_ex`**: argv/envp desde userspace (libsoso)
- [`tools/sosoas`](../tools/sosoas): GAS `.byte` → ELF64 ET_REL
- [`config/rust-soso/sys/pal/soso/dl.rs`](../config/rust-soso/sys/pal/soso/dl.rs): parse ELF + stub dlopen
- [`tools/wild-soso`](../tools/wild-soso): passthrough a `wild`
- **`/bin/soso-rustc`**: stub guest (`--version`, comprueba sysroot)
- Canal OTA **`dev`**: `UPD_CHANNEL_DEV` en `soso-update-core`
- Pendiente: rustc cruzado, relocations en dlopen, iced-x86 completo, sysroot en `/usr/lib/rustlib/`

## Hito 4 — Cierre

- **`soso-git status|log|diff|commit`**: hashes de `/src/soso` (puente hasta gix)
- **`soso-forja build-local` + `install`**: bucle OTA local
- Pendiente: gix + push HTTPS, tests host nativos completos en guest

## Verificación

```sh
cargo test -p sosoas
cargo test -p sosofs --features std
cargo test -p soso-forja-server
cd user && cargo build --release
cargo xtask build
cargo xtask sync-src
cargo xtask release && cargo xtask forja-out && cargo xtask mkfs
# imagen de datos: 32 GiB por defecto (`SOSO_ROOTFS_SIZE=64G` para más espacio)
# guest: /bin/init test
# bucle local: soso-forja all-local  (tras forja-out en imagen)
```
