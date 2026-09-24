# Bootstrap rustc para soso (Hito 2–3)

1. Preparar árbol y parches:
   ```sh
   ./scripts/soso-rust-bootstrap.sh
   # o: cargo xtask rust-bootstrap
   ```
   Clona `rust-lang/rust` en **`~/.cache/soso-rust-vendor`** (fuera del workspace
   soso; override con `SOSO_RUST_VENDOR`).

2. Parches automáticos (`config/rust-soso/apply-patches.sh`):
   - `library/std/build.rs` — `target_os = "soso"`
   - `library/std/src/sys/pal/soso/` — PAL (runtime_entry + soso_rt)
   - `library/std/src/os/soso/` — módulo OS
   - `crates/soso-rt` — ABI de syscalls para std
   - Target JSON con `"os": "soso"`

3. Compilar **libstd** cruzada (host linux → target soso):
   ```sh
   cargo xtask rust-build-std
   ```
   Internamente: `./x.py build library/std --target x86_64-unknown-soso`

4. Ensamblador: `tools/sosoas`. Enlazador: `tools/wild-soso`.

5. Bootstrap **rustc stage2** (largo, manual):
   ```sh
   cd ~/.cache/soso-rust-vendor
   export PATH="$PWD/../../tools/sosoas/target/release:.../wild-soso/target/release:$PATH"
   ./x.py build --target x86_64-unknown-soso --stage 2
   ```

Sysroot destino en guest: `/usr/lib/rustlib/x86_64-unknown-soso/`.

Contingencia proc-macros: pre-expansión en host hasta que `dl.rs` cargue `.so`.
