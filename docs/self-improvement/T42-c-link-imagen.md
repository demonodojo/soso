# T42 — Cerrar C, ensamblador y empaquetado por perfil

**Hito:** SI-7 · **Tipo:** Especificación e integración por componente · **Estado:** pendiente.  
**Dependencias:** [T38](T38-toolchain-inventario.md), [T41](T41-cargo-offline.md)

## Objetivo y entrega

Build y empaquetado nativos del perfil declarado, con ninguna herramienta oculta en Linux en ese recorrido.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1, C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [xtask/src/lx_build.rs](../../xtask/src/lx_build.rs)
- [xtask/src/release.rs](../../xtask/src/release.rs)
- [tools/sosoas/src/main.rs](../../tools/sosoas/src/main.rs)
- [tools/wild-soso/src/main.rs](../../tools/wild-soso/src/main.rs)
- [tools/mkfs-soso/src/main.rs](../../tools/mkfs-soso/src/main.rs)

## Archivos que se pueden cambiar

Crear `docs/self-improvement/native/build-profile.json`, sondas C/asm en `tests/self-improvement/native/` y fichas C-xxx por herramienta faltante.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Fijar lista exacta de herramientas del perfil objetivo. Auditar sosoas limitado y wild-soso wrapper frente al ensamblador/linker reales requeridos por ese perfil.
2. Para cada componente ausente crear ficha C-xxx con un input mínimo y salida verificable. Implementar/validar una por pase: C, ensamblado, enlace, mkfs y empaquetado.
3. Probar C con las ABI/atomics usadas por lxdde y un objeto ensamblador representativo; enlazar y ejecutar. Solo después compilar el port/driver del perfil.
4. Llevar creación de rootfs/imágenes/paquetes al guest. Verificar estructura y hashes con lectores independientes, además de que el comando termine.
5. Compilar kernel+userspace del perfil y crear imagen arrancable enteramente desde fuentes en soso. Registrar pasos aún externos como pendientes.

## Comprobación

Sondas por herramienta y un boot de la imagen nueva en destino independiente. El perfil QEMU mínimo y el perfil con lxdde tienen resultados separados.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No declarar todos los drivers soportados por haber construido un kernel mínimo; las capacidades adicionales exigen su propia cadena.

Entregar `target/self-improvement/tasks/T42/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

