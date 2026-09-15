# T01 — Capturar una base reproducible sin alterar el checkout

**Hito:** SI-0 · **Tipo:** Implementación host · **Estado:** completada (15 de septiembre de 2026; resumen en [seguimiento/T01.md](seguimiento/T01.md), evidencia en `target/self-improvement/tasks/T01/resultado.md`).  
**Dependencias:** Ninguna; puede iniciarse ahora.

## Objetivo y entrega

Manifiesto y copia reconstruible; ejecutar las suites base de C6 sobre la copia y registrar resultados, incluidos fallos previos.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md)
- [rust-toolchain.toml](../../rust-toolchain.toml)
- [xtask/src/check.rs](../../xtask/src/check.rs)

## Archivos que se pueden cambiar

Crear `scripts/self-improvement/baseline.py` y `tests/self-improvement/test_baseline.py`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Implementar con Python estándar `baseline.py --repo <ruta> --out <ruta>`. Exigir destino nuevo situado fuera de los archivos fuente; escribir `baseline.json`, diff binario y relación de archivos nuevos con SHA-256.
2. Registrar HEAD, estado Git con rutas delimitadas por NUL, cambios staged y unstaged, toolchain, OpenCode y herramientas de build. Versionar solo una lista explícita de variables SOSO pertinentes; nunca volcar todo el entorno.
3. Conservar contenido de archivos nuevos seleccionados como fuentes; para otros, registrar ruta/hash y exclusión. No recorrer discos de modelos, firmware ni `target/` indiscriminadamente. La captura debe permitir reconstruir exactamente la base declarada.
4. Detectar cambios del checkout durante la captura repitiendo estado y hashes; abortar como captura inestable si difieren. No hacer stash, reset, add ni commit.
5. Dar a cada comprobación un log y código de salida. El script registra disponibilidad; las suites base se ejecutan como paso explícito posterior para no ocultar un build largo.

## Comprobación

`python3 -m unittest discover -s tests/self-improvement -p test_baseline.py`. Fixtures: repo limpio, staged+unstaged sobre un mismo archivo, binario, ruta con espacios, archivo nuevo y modificación concurrente. Comparar estado del repo antes/después; reconstruir la captura en temporal y comparar hashes.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

Si la copia no reproduce los archivos, corregir la captura antes de editar el runtime. La falta de hardware se registra por separado.

Entregar `target/self-improvement/tasks/T01/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

