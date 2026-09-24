# T34 — Convertir huecos del port en fichas implementables

**Hito:** SI-6 · **Tipo:** Especificación técnica basada en evidencia · **Estado:** completada (2026-09-24).

**Dependencias:** [T32](T32-opencode-inventario.md), [T33](T33-sondas-abi.md)

## Objetivo y entrega

Backlog técnico completo y trazable. T35 requiere además implementar y verificar las fichas N-xxx obligatorias.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1, C5–C6**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

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

## Resultado

[`native/backlog.json`](native/backlog.json) con **11 fichas N-xxx**, grafo
acíclico y sin dependencias colgando, y las tres primeras redactadas:
[N-001](native/N-001.md), [N-002](native/N-002.md), [N-003](native/N-003.md).

### La ruta: sustrato POSIX primero

Un programa escrito contra POSIX tiene que encontrarse POSIX, y
[T33](T33-sondas-abi.md) midió que varias semánticas de soso **no lo son**.
Ninguna se arregla en el runtime —el modelo de ficheros por descriptor, la
ausencia de `PROT_NONE`, el spawn sin cwd son del sistema—, así que van primero
sea cual sea lo que se ponga encima.

**Portar Bun tal cual** queda descartado por cuatro carencias medidas o
inventariadas: sin `dlopen` ningún `.node` es cargable ni recompilándolo; sin
pty ni `inotify` faltan dos dependencias del núcleo de OpenCode; y el modelo de
ficheros impide SQLite. **Un motor JS pequeño** evitaría `dlopen` —y el JIT no
haría falta, aunque T33 midió que sí sería viable— pero el código está escrito
contra las API de Bun y unos noventa paquetes de npm, y no elimina ni una ficha
de sustrato.

### La alternativa que no es un port, anotada igual

**No portar: un agente nativo en Rust que reutilice el protocolo.** Queda fuera
de lo que esta ficha elige, porque no es una ruta de port. Se deja escrita
porque el plan **ya la está construyendo** —`soso-llm` sirve la API (T16–T19),
`conversation` renderiza y parsea llamadas a herramientas (T06–T12),
`soso-improve` coordina (T23–T24)— y porque T33 midió que las herramientas de
un agente ya funcionan. Elegir entre portar y reimplementar es **decisión de
plan**, como el no-go de T14; aquí se pone al lado de las otras con la
evidencia delante.

## Comprobación

Validar grafo acíclico y cobertura total de requisitos T32. Cada N-xxx tiene prueba que falla antes y debe pasar después; el cierre de port requiere build real y sondas, no solo documentación.

**Hecho:** grafo acíclico y sin dependencias colgando, comprobado al generar el
JSON. Los cinco huecos de T33 tienen ficha (N-001, N-002, N-003, N-004, N-011)
y los inventariados por T32 sin sonda posible son N-005 a N-010.

**Límite honesto:** ocho de las once **no** pueden tener todavía una prueba que
falle antes y pase después, porque su capacidad no existe. Las tres redactadas
la nombran; en las otras ocho está dicho en vez de dejarlo en blanco.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [x] Implementación o artefactos de esta ficha terminados.
- [x] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

**Riesgo declarado.** N-001 es la de mayor riesgo del backlog: `StreamWrite`
existe porque reescribir un fichero entero en sosofs es caro, y las
actualizaciones se apoyan en ese camino. Su paso 1 es elegir forma **y medir el
coste**, no implementar.

No inventar firmas de Bun/JSC ni ABI POSIX ausentes: obtenerlas de la revisión fijada. Si el port resulta mayor, aumentar número de fichas, no tamaño de cada una.

Entregar `target/self-improvement/tasks/T34/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **no aplicable a esta entrega de laboratorio/especificación**. Condiciones adicionales: [T35](T35-opencode-nativo.md). Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
