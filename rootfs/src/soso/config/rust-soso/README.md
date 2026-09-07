# Bootstrap rustc para soso (Hito 2–3)

1. Clonar fork mínimo de `rust-lang/rust` sobre el nightly de `rust-toolchain.toml`.
2. Copiar `config/rust-soso/config.toml` → `config.toml` del fork.
3. Copiar `config/rust-soso/sys/pal/soso/` → `library/std/src/sys/pal/soso/`.
4. Añadir target `targets/x86_64-unknown-soso.json` al fork.
5. Bootstrap:
   ```sh
   ./scripts/soso-rust-bootstrap.sh
   cd vendor/rust && ./x.py build --host x86_64-unknown-soso --stage 2
   ```
6. Ensamblador externo: `tools/sosoas` (PATH o `rustc.assembler`).
7. Enlazador: `wild` vía `tools/wild-soso`.

Canal OTA `dev`: constantes `UPD_CHANNEL_DEV` en `soso-update-core`.
Sysroot destino en guest: `/usr/lib/rustlib/x86_64-unknown-soso/`.

Contingencia proc-macros: pre-expansión en host hasta que `dl.rs` cargue `.so`.
