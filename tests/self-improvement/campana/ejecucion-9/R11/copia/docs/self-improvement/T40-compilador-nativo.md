# T40 — Descomponer y acreditar el port del compilador

**Hito:** SI-7 · **Tipo:** Especificación e integración condicionada · **Estado:** pendiente.  
**Dependencias:** [T38](T38-toolchain-inventario.md), [T39](T39-bootstrap-libstd.md)

## Objetivo y entrega

Todas las C-xxx necesarias están implementadas y rustc ejecutado en soso genera código nuevo. Un backlog redactado solo completa la parte de especificación.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1, C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [user/soso-rustc/src/main.rs](../../user/soso-rustc/src/main.rs)
- [config/rust-soso/sys/pal/soso/dl.rs](../../config/rust-soso/sys/pal/soso/dl.rs)
- [crates/soso-rt/src/lib.rs](../../crates/soso-rt/src/lib.rs)
- [targets/x86_64-unknown-soso.json](../../targets/x86_64-unknown-soso.json)

## Archivos que se pueden cambiar

Crear `docs/self-improvement/native/compiler-backlog.json`, fichas `C-xxx.md` y `compiler-probe.rs` de prueba. Implementación de cada C-xxx limitada a sus rutas.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Configurar una compilación donde la plataforma host de rustc sea soso y registrar dependencias reales de ejecución. Usar T38, no asumir que stage2 --target ya lo hace.
2. Convertir cada símbolo, biblioteca o comportamiento faltante en C-xxx con firma, archivo, reproducción y comprobación. Separar PAL, codegen, linker y carga de proc macros.
3. Implementar las fichas una a una; una sesión de modelo pequeño recibe solo una C-xxx. Mantener fallos y checkpoints de build en artefactos.
4. Cuando el compilador arranque, ejecutarlo en guest sobre una fuente recibida como entrada y emitir un objeto nuevo. Cambiar la fuente y verificar que cambia el artefacto.
5. Enlazar y ejecutar un programa que imprima ese dato y devuelva un código previsto. Acreditar invocación/proceso/artefacto; sustituir el stub solo cuando la interfaz real funcione.

## Comprobación

Sondas compilador: fuente válida, error sintáctico con diagnóstico/exit no cero, --emit=obj, fuente modificada y ejecutable nuevo. Ejecutarlas dentro de soso con fuentes y outputs de hashes conocidos.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No sustituir el stub por un wrapper a Forja y llamarlo rustc nativo; la evidencia debe mostrar compilación guest.

Entregar `target/self-improvement/tasks/T40/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

