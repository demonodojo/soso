# T41 — Validar Cargo y fuentes reproducibles dentro de soso

**Hito:** SI-7 · **Tipo:** Integración condicionada · **Estado:** pendiente.

**Dependencias:** [T40](T40-compilador-nativo.md)

## Objetivo y entrega

Cargo guest produce artefactos nuevos a partir de fuentes identificadas; las dependencias externas restantes quedan listadas y bloquean el cierre que las necesite.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [Cargo.toml](../../Cargo.toml)
- [Cargo.lock](../../Cargo.lock)
- [user/Cargo.toml](../../user/Cargo.toml)
- [user/soso-git/src/main.rs](../../user/soso-git/src/main.rs)
- [docs/SELF-HOSTING.md](../../docs/SELF-HOSTING.md)

## Archivos que se pueden cambiar

Crear fixtures de workspace en `tests/self-improvement/native/cargo/`, receta de empaquetado y nuevas C-xxx para requisitos Cargo que falten.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Portar las dependencias Cargo identificadas T38 mediante fichas C-xxx individuales; comprobar procesos, archivos de bloqueo, tiempos y directorios temporales con sondas.
2. Provisionar vendor y lockfiles completos para una ejecución --offline; manifiesto de fuentes con hashes y revisión. Mantener registro de versiones reconstruible aunque Git completo siga pendiente.
3. Compilar un workspace mínimo de dos crates, después uno con build.rs y otro con proc macro si el perfil las necesita.
4. Cambiar una fuente en guest, reconstruir y ejecutar el resultado; comprobar que fallos de compilación llegan con exit no cero.
5. Compilar una crate real de soso para el target apropiado. Registrar explícitamente si alguna dependencia sigue precompilada externamente.

## Comprobación

Dentro del guest: Cargo real con modo offline y red de Forja deshabilitada para el ensayo. Confirmar versión propia, dependencia local, rebuild tras cambio y diagnóstico por error.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Si falta dlopen/proc macros, crear ficha específica; no preexpandir en Linux y contabilizar la campaña como completamente nativa.

Entregar `target/self-improvement/tasks/T41/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Ejecutar Cargo offline y sus build.rs/proc macros/generadores en soso, incluidos verificadores P/R. Fuentes por inventario y hash; Git CLI y descargas no son requisitos. Cualquier helper externo pendiente bloquea cobertura del perfil.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
