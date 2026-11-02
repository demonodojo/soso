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

   `jobserver` 0.1.34 se conserva ya corregido en
   [`vendor/jobserver`](vendor/jobserver/SOSO.md). La preparación copia esas
   fuentes y configura `[patch.crates-io]` en los workspaces de Rust y Cargo;
   no parchea jobserver en la caché global. Sus regresiones se ejecutan con
   `tests/self-improvement/native/cargo/jobserver/probar-host.sh`.

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

6. Bootstrap **cargo** (stage2-tools, host → target soso):
   ```sh
   cd ~/.cache/soso-rust-vendor
   REPO=/ruta/al/repo/soso
   env -u RUSTFLAGS_BOOTSTRAP RUST_TARGET_PATH="$PWD" \
     PATH="$REPO/target/release:$PATH" \
     ./x.py build cargo --host x86_64-unknown-soso \
     --keep-stage-std 0 --keep-stage-std 1
   ```
   Binario: `build-soso/…/stage2-tools/x86_64-unknown-soso/release/cargo`.
   Requiere **`wild-soso` en `$REPO/target/release`** (C-090/C-091).

Sysroot destino en guest: `/usr/lib/rustlib/x86_64-unknown-soso/`.

Contingencia proc-macros: pre-expansión en host. La PAL antigua con `dl.rs` se retiró (T69): libstd no la declara.

7. **Reconstruir y desplegar en el guest** (lo que hace falta tras tocar el PAL,
   `apply-patches.sh` o el kernel; T41):
   ```sh
   REPO=/ruta/al/repo/soso
   bash $REPO/config/rust-soso/apply-patches.sh      # idempotente
   cd ~/.cache/soso-rust-vendor
   ENV="env -u RUSTFLAGS_BOOTSTRAP RUST_TARGET_PATH=$PWD PATH=$REPO/target/release:$PATH"
   # rustc (usa el build-dir de T40; --warnings warn por los avisos de libc):
   $ENV ./x.py build compiler/rustc --host x86_64-unknown-soso -j 8 --warnings warn \
        --build-dir $REPO/target/self-improvement/tasks/T40/build
   $ENV ./x.py build cargo --host x86_64-unknown-soso
   cp $REPO/target/self-improvement/tasks/T40/build/x86_64-unknown-soso/stage2/bin/rustc $REPO/rootfs/bin/
   cp build-soso/x86_64-unknown-linux-gnu/stage2-tools/x86_64-unknown-soso/release/cargo $REPO/rootfs/bin/
   rsync -a --delete build-soso/x86_64-unknown-linux-gnu/ci-rustc-sysroot/lib/rustlib/x86_64-unknown-soso/lib \
         $REPO/rootfs/lib/rustlib/x86_64-unknown-soso/
   tests/self-improvement/native/cargo/real/preparar.sh   # fixture de soso-abi
   cp -r tests/self-improvement/native/cargo/ws2 tests/self-improvement/native/cargo/ws3 rootfs/var/t41/
   cargo xtask run                                         # arranca; SSH en 2222
   ```
   Dentro (`ssh -tt -i target/soso_test_key -p 2222 soso@localhost`):
   `cd /var/t41/ws3; cargo build --offline; /var/t41/ws3/target/debug/t41app`.
   `rootfs/bin/` y `rootfs/lib/rustlib/` están ignorados por git. Los crates
   registrados ya parcheados no se recompilan solos: borrar
   `build-soso/.../stage2-tools/.../release/{build/<crate>,.fingerprint/<crate>-*}`.
   Un binario C enlazado a glibc estática solo funciona con los shims del PAL
   (`libc_alloc.rs`, `libc_tls.rs`, `libc_time.rs`, `libc_sys.rs` e
   `iniciar_ctype`); los símbolos de `malloc` son débiles para que rustc, que
   enlaza la glibc por LLVM, conserve los suyos.
