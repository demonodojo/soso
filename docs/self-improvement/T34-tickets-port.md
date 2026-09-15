# T34 — Convertir huecos del port en fichas implementables

**Hito:** SI-6 · **Tipo:** Especificación técnica basada en evidencia · **Estado:** pendiente.  
**Dependencias:** [T32](T32-opencode-inventario.md), [T33](T33-sondas-abi.md)

## Objetivo y entrega

Backlog técnico completo y trazable. T35 requiere además implementar y verificar las fichas N-xxx obligatorias.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1, C5–C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [docs/SELF-HOSTING.md](../../docs/SELF-HOSTING.md)
- [crates/soso-abi/src/lib.rs](../../crates/soso-abi/src/lib.rs)
- [config/rust-soso/sys/pal/soso/mod.rs](../../config/rust-soso/sys/pal/soso/mod.rs)

## Archivos que se pueden cambiar

Crear `docs/self-improvement/native/backlog.json`, `N-001.md` y fichas sucesivas; actualizar dependencias del índice nativo.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Ordenar huecos por dependencia: runtime mínimo y almacenamiento/procesos/red antes que TUI. Elegir una ruta concreta de port a partir del inventario; registrar por qué las alternativas no cubren las sondas.
2. Para cada símbolo o comportamiento ausente, crear una ficha N-xxx con revisión de origen, 1–3 archivos de lógica, ABI/firma exacta, pasos y la sonda T33 que debe pasar.
3. Separar adaptaciones del runtime externo de ampliaciones de soso; cada ficha declara dónde se aplica y cómo construirla. No agrupar un motor JavaScript entero en una tarea.
4. Si una decisión todavía no tiene evidencia, crear primero una ficha de experimento con resultado binario y límite de alcance; sus consumidores quedan condicionados.
5. Definir en backlog los cierres runtime-arranca, HTTP, persistencia y herramientas. Entregar la primera ficha lista para ejecución; continuar generando fichas hasta cubrir todas las capacidades necesarias.

## Comprobación

Validar grafo acíclico y cobertura total de requisitos T32. Cada N-xxx tiene prueba que falla antes y debe pasar después; el cierre de port requiere build real y sondas, no solo documentación.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No inventar firmas de Bun/JSC ni ABI POSIX ausentes: obtenerlas de la revisión fijada. Si el port resulta mayor, aumentar número de fichas, no tamaño de cada una.

Entregar `target/self-improvement/tasks/T34/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

