# Self-hosting de soso — ruta A

## Hito 0 — Bucle remoto ✓

Tres caminos distintos (no mezclarlos):

| Comando | Qué hace |
|---------|----------|
| `soso-forja local` | Planifica unidades por hash (`/var/forja-cache`); **no** compila. |
| `soso-forja build-local` / `all-local` | Copia artefactos ya hechos en `/var/forja-out` y opcionalmente `soso-update`. |
| `soso-forja sync\|build\|all --host IP` | Sube fuentes al servidor host (`soso-forja-server`); el build corre en un árbol aislado (`target/forja-work`). Un HTTP 4xx/5xx no se anuncia como éxito; un fallo de `aplicar` no hace `halt`. |

`soso-forja all --host 10.0.2.2 --token …` + revert del shim. Token: `SOSO_FORJA_TOKEN` (Bearer en POST y GET); obligatorio si `SOSO_FORJA_BIND` no es loopback (`127.0.0.1`/`::1`/`localhost`). Cliente: `--token`. `SOSO_FORJA_RELEASE=hola-std` compila solo ese binario y entrega el ELF como pack con `X-Forja-Build-Id`. Demo B3: `soso-forja write-hola --msg …` + `all --host 10.0.2.2 --token …` aplica `/bin/hola-std` sin halt (`cargo xtask test -- --guest sys --only forja`).

### El recibo: qué liga las fuentes con el artefacto

`manifest.txt` **es un recibo versionado** (T36), no una etiqueta. El servidor
lo emite junto a los artefactos y el cliente lo verifica **antes** de escribir
nada en `/var/actualiza-prueba`, porque de ahí se aplica.

```
forja-recibo=1
build-id=<16 hex del sha256 del manifiesto de fuentes>
source-manifest-sha256=<sha256 completo del manifiesto aceptado en /sync>
source-files=<cuántos ficheros>
perfil=<cargo-xtask-release|hola-std|fake-ok|fake-fail>
artefacto=rootfs.pack sha256=<hex> bytes=<n>
artefacto=kernel-x86_64 sha256=<hex> bytes=<n>
```

`sync` guarda en `/var/forja-cache/ultimo-sync.txt` el `build-id` y el hash de
las fuentes **que envió**, y `build` rechaza el pack si el recibo habla de otro
build o de otras fuentes, si falta un campo, o si los bytes descargados no
tienen el sha256 que el recibo declara. Un rechazo no deja nada escrito.

Dos cosas que el cliente ya no acepta: construir sin un `sync` previo en esa
máquina (no hay contra qué comparar), y un servidor que no emita recibo — se
dice con un error, no se interpreta a medias.

Del lado del servidor, `POST /build` devuelve **409** si no hubo `sync`, y
**500** si el build no dejó un recibo con el `build-id` de esa petición: lo que
quedara en el directorio de release es de otra, y servirlo sería entregar un
artefacto viejo como si fuera el pedido.

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
