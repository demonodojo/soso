# T72 — Compartir el alocador entre libsoso y la libstd de soso

**Hito:** SI-7 · **Estado:** completada el 26-sep-2026. Validación nativa parcial (sonda libsoso; el humo de std sigue en T39).
**Origen:** T39, 25-sep. **Dependencias:** ninguna entrega adicional.
**Consumidor:** [T39](T39-bootstrap-libstd.md). **Decisión:** D7, opción 3.
**Contrato:** C1, C6 y [NATIVO.md](NATIVO.md).
**Seguimiento:** [T72](seguimiento/T72.md).

## Problema y elección de la replanificación

El último `cargo xtask rust-build-std` conservado termina con rc=1 y dos errores:
`sys/alloc/mod.rs` no encuentra rama de `cfg_select` y no resuelve `imp`.
Son los dos errores observados, no una garantía de que el siguiente build
complete. `soso-rt` no proporciona el alocador que std necesita.

La revisión solicitada adopta **un crate no_std compartido por libsoso y
soso-rt**. `SbrkAllocator` y `HybridAllocator` ya resuelven pequeño/grande
con sbrk/mmap; conservar esa política, alineamientos y agrupación de syscalls.
El nombre y la API del crate son entregables, no APIs existentes.

Las alternativas consideradas fueron arrastrar libsoso y sus dependencias a
std, duplicar un alocador en soso-rt o usar un bump que no libera. La extracción
permite una implementación única y evita que un humo sin liberación se
convierta en la base del compilador. La extracción y el cierre están al final.

## Alcance y contexto

- `user/libsoso/src/lib.rs`: localizar alocadores y dependencias reales antes
  de extraer; mantener el mismo alocador global del consumidor.
- Crate compartido nuevo en `crates/`, `crates/soso-rt`, manifiestos/lockfiles
  de ambos workspaces y adaptadores mínimos de syscalls.
- `config/rust-soso/tree/library/std/src/sys/alloc/soso.rs` (nuevo), despacho
  en `apply-patches.sh` y receta `crates/soso-improve-core/src/receta.rs`.
- Sonda de asignación/liberación en userspace y humo std de T39.

No cambiar política de memoria, auditoría del montón del kernel (D5), ABI ni
las demás APIs de libsoso. Si hace falta ampliar el alcance, derivar una ficha.

## Pasos

1. Inventariar dependencias efectivas de los alocadores, ownership del estado
   y syscalls. Definir una capa mínima que no dependa de libsoso ni cree un
   ciclo `libsoso → alocador → libsoso`.
2. Extraer la implementación manteniendo semántica y política. Añadir sólo
   las dependencias necesarias; preparar `rustc-dep-of-std` también en las
   transitivas que lo requieran. La extracción no elimina ese trabajo.
3. Conectar ambos consumidores. En cada ejecutable debe existir un solo
   estado coherente del montón; no registrar dos alocadores globales ni dos
   gestores sbrk independientes.
4. Implementar la rama std y mantener script/receta sincronizados (T75).
   Probar fallos de reserva, alineamientos, alloc_zeroed, realloc que conserva
   datos y liberación de pequeños/grandes sin asumir que el RSS baja siempre.
5. Construir usuarios existentes y ejecutar la sonda guest. Reanudar el build
   de T39 y su programa std: repetir reservas/liberaciones suficientes para
   detectar un alocador que nunca libera. Registrar límites y memoria.

## Comprobación y cierre

Pruebas host del crate compartido y `cargo check --no-default-features`;
compilación del workspace `user/` afectado; sonda guest mediante el arnés
existente y `cargo xtask rust-build-std`. Definir el comando exacto de la
sonda al implementarla y conservar argv, exit code y resultado.

- [x] Una implementación consumida por libsoso y soso-rt, sin ciclos.
- [x] Comportamiento del alocador conservado y pruebas de fallo/alineamiento.
- [x] Asignación, realloc y liberación acreditados en guest con libsoso.
- [x] Rama std integrada y build intentado; errores nuevos derivados a T39.

## Cierre (26-sep-2026)

`soso-alloc` guarda el único montón. libsoso lo registra como alocador global;
`soso-rt` reexporta `alloc`/`dealloc`/`realloc`/`heap_init` para `sys/alloc/soso.rs`.
La sonda guest es `soso-agent-probe monton` (arnés `cargo xtask test -- --guest sys --only monton`).
El 26-sep se lanzó QEMU aparte, con NVRAM propia, porque el arnés mata el QEMU
`test-llm-api` y reescribe `target/OVMF_VARS.fd`. SSH `soso-agent-probe monton`
en el puerto 2244 salió 0: 7 casos, `malos` 0. El bloque pequeño se acredita
porque, con otro bloque vivo, la dirección liberada vuelve a salir; el arena
sólo rebobina el último. El build de std (rc=1) compila el alocador y falla
después, en casts de procedencia de `sys/random` y `sys/sync/futex`. Eso queda
en T39. Resumen: [seguimiento/T72.md](seguimiento/T72.md).

**Validación nativa:** parcial. La sonda guest de libsoso está acreditada.
El humo de std, con el programa que reserva y libera, sigue en T39. Compilar
en host no lo sustituye.
