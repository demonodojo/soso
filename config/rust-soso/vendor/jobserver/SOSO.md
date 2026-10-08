# jobserver para soso

Fuentes de `jobserver` **0.1.34**, la versión fijada por los lockfiles de Rust
y Cargo del bootstrap. Origen: archivo `jobserver-0.1.34.crate` de crates.io,
SHA-256 `9afb3de4395d6b3e67a780b6de64b51c978ecf11cb9a462c66be7d4ca9039d33`.
Se conservan las licencias MIT y Apache-2.0, los backends y las pruebas originales.

Cambios locales en `src/wasm.rs` (backend que usa soso):

- C-104: `configure` y `string_arg` no provocan un pánico; no hay herencia del
  jobserver entre procesos.
- C-144: el cierre del auxiliar cancela su adquisición pendiente bajo el mutex
  del contador, sin crear permisos ni cancelar otros auxiliares.

`apply-patches.sh` copia este árbol a `src/soso-jobserver` y configura
`[patch.crates-io]` en **ambos** workspaces, Rust y Cargo. Así se usa también
para las dependencias transitivas. No se modifica `~/.cargo/registry` para
jobserver. La primera resolución actualiza su entrada del lock a una ruta local.
Para actualizar la versión hay que revisar el código y los lockfiles juntos.

Prueba del backend de soso desde la raíz del repositorio:

```sh
tests/self-improvement/native/cargo/jobserver/probar-host.sh
```

La opción `soso-tests` selecciona ese backend en el host; sólo se activa en
las pruebas. Las cuatro regresiones están en `src/soso_tests.rs`, escritas en
Rust y compiladas directamente sin registry ni generadores Python. El script
acota la ejecución a 30 segundos. La compilación normal en Unix/Windows conserva
su backend original.
