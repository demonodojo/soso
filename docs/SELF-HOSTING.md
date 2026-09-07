# Self-hosting de soso — ruta A

Documentación del plan implementado.

## Hito 0 — Bucle remoto

- `soso-ed`: editor mínimo (`/bin/soso-ed RUTA`)
- Coreutils: `cp`, `mv`, `grep`, `diff`, `find`, `wc`, `head`, `tail`, `stat`
- Syscalls: `SYS_RENAME`, `SYS_TRUNCATE`, `O_TRUNC`, `O_EXCL`, `SYS_CLOCK_GETTIME`, RTC CMOS
- `/src/soso`: fuentes locales (excluidas del OTA via `PACK_SKIP_DIRS`)
- `soso-forja`: cliente (`sync`, `build`, `install`, `all`, `local`)
- `tools/soso-forja-server`: servidor host en `:8740` (tests HTTP mockeados)

```sh
cargo run -p soso-forja-server
soso-ed /src/soso/kernel/src/main.rs
soso-forja all --host 10.0.2.2
```

## Hito 1 — ABI para std

Syscalls 70–82: `dup2`, `fstat`, `utime`, `fsync`, `sched_yield`, `getrandom`,
`set_tls`, `mprotect`, `mremap`, `pwrite`. `SpawnIo` con `argv`/`envp`.
Escritura incremental (`StreamWrite`). ELF perezoso + `PT_TLS`. `NAME_MAX` 255
(formato `SOSOFS11`). Join de hilos por futex (`join_uaddr`).

## Hito 2 — soso-std

- Crate [`crates/soso-std`](../crates/soso-std)
- Demo [`user/hola-std`](../user/hola-std)
- Target JSON [`targets/x86_64-unknown-soso.json`](../targets/x86_64-unknown-soso.json)
- PAL scaffolding [`config/rust-soso/sys/pal/soso/`](../config/rust-soso/sys/pal/soso/)

## Hito 3 — Toolchain nativa (scaffolding)

- [`config/rust-soso/config.toml`](../config/rust-soso/config.toml): bootstrap rustc + Cranelift
- [`tools/sosoas`](../tools/sosoas): ensamblador GAS (stub)
- [`tools/wild-soso`](../tools/wild-soso): passthrough a `wild`
- [`config/rust-soso/sys/pal/soso/dl.rs`](../config/rust-soso/sys/pal/soso/dl.rs): dlopen (stub)
- `soso-forja local`: plan de unidades sin cargo

## Hito 4 — Cierre

- `soso-forja install` encadena OTA local + reinicio (revert automático del shim)
- Tests host: `cargo test -p sosofs --features std`, `cargo test -p soso-forja-server`
- Tests guest: `/bin/init test` (bloque self-hosting)
- Control de versiones: pendiente integración **gix** (gitoxide)
- Docs: `MANUAL-USUARIO.md`, skills `soso-dev`, `soso-architecture`

## Verificación rápida

```sh
cargo test -p sosofs --features std
cargo test -p soso-forja-server
cd user && cargo build --release
cargo xtask build
cargo xtask mkfs   # tras SOSOFS11
```
