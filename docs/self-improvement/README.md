# Subplanes de automejora para modelos pequeños

**Plan padre:** [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md).

**Fecha:** revisión del 16 de septiembre de 2026; 53 fichas. **Estado:** planificación; ninguna tarea
de implementación se declara realizada por crear estas fichas.

## Por dónde empezar

**[T01](T01-base.md), [T02](T02-banco.md) y [T03](T03-perfil-modelo.md) están
completadas** (15–16 de septiembre de 2026; resúmenes en
[seguimiento/](seguimiento/)). T03 encontró que el tokenizer de soso **no
reproduce la segmentación oficial del modelo** y dejó dos fichas previas a T06;
el perfil del modelo está en [modelo.md](modelo.md). **[T52](T52-tokenizer-merges.md) y [T53](T53-tokenizer-bpe.md) están
completadas**: soso segmenta ya **exactamente igual** que el tokenizer oficial
del modelo (5/5 fixtures). **[T04](T04-dominio-chat.md)**, **[T05](T05-validacion-chat.md)** y
**[T06](T06-render-chat.md)**–**[T12](T12-respuestas-sse.md)** están
completadas (dominio chat + render + parse + informe + cancelación + API JSON + HTTP + respuestas SSE).
**[T13](T13-servidor-host.md)** (servidor de desarrollo host) está **completada** (2026-09-22;
[seguimiento/T13.md](seguimiento/T13.md)). **[T15](T15-sesion-residente.md)** (sesión residente guest) está **completada** (2026-09-22;
[seguimiento/T15.md](seguimiento/T15.md) … [seguimiento/T19.md](seguimiento/T19.md)). **SI-2 (T16–T19) tiene ya su evidencia guest**: el 23-sep-2026 `cargo xtask test-llm-api` pasó **12/12 invariantes contra soso en QEMU con los pesos reales** (`guest_ok`, exit 0). Llegar ahí costó cuatro defectos reales, con ficha cada uno: **[T54](T54-accept-externo.md)** (el `accept` de userspace era loopback puro), **[T55](T55-accept-sin-plazo.md)** (`tcp_accept(fd,0)` dormía al servidor para siempre), **[T56](T56-utf8-tool-parser.md)** (el parser partía caracteres UTF-8 y mataba el proceso) y **[T57](T57-medio-cierre.md)** (el medio cierre del cliente tiraba la respuesta). **[T14](T14-evaluacion-modelo.md) dio NO-GO**: **8 de 10** casos de protocolo sólidos frente a un umbral de 10, con **0 inestables** (el modelo es reproducible a temperatura 0). La investigación de **[T59](T59-tool-calls.md)** (24-sep) cerró la atribución: **el no-go es del modelo**. El Qwen2.5-Coder-3B no usa las herramientas en ninguno de los dos casos que el banco ejercita —en Q04 repite la llamada con el resultado delante; en Q07, con `tool_choice: "required"`, no emite ninguna—, mientras que el render es correcto y el servicio detecta la violación. Los otros dos casos ya pasan: uno era un defecto real del servicio (**[T60](T60-validacion-peticion.md)**, cerrada) y el otro, del propio arnés. Con este modelo el umbral de 10/10 **no se alcanza**, y **todo lo accionable está hecho**: la decisión siguiente es de plan —probar otro modelo con el mismo banco, sin tocarlo durante la comparación, o acotar SI-3 a lo que un agente sin herramientas pueda hacer—. Presupuestos medidos: mediana 60,7 s por tarea, máximo 278 s. **Un no-go no habilita a [T16](T16-servicio-guest.md) ni a [T22](T22-primera-mejora.md)**, así que **SI-2 no cierra**: fichas recomendadas ahora, T59 y T60, y repetir la campaña al cerrarlas. Criterio de la evaluación en [evaluacion.md](evaluacion.md).
**Camino SI-4**: **[T23](T23-estado-coordinador.md)** (formato de tareas y
estados), **[T63](T63-generacion-rota-encalla.md)** (el corte que encallaba la
referencia durable) y **[T24](T24-checkout.md)** (copia de tarea y exportación
de su parche) están **completadas** el 24-sep-2026, las tres con su sonda en
guest. Con eso el coordinador ya sabe llevar el estado de una tarea y preparar
y recoger el trabajo de un candidato **sin Git**.

De T23 salieron las decisiones que después no se pueden cambiar: sólo el
validador escribe «aceptada», lo que no se midió se informa con motivo —nunca
como cero— y en el estado van referencias, no logs. De T24, que lo que siembra
el coordinador no cuenta como cambio del candidato y que un enlace simbólico se
informa en vez de desaparecer del parche.

**Y aquí la cadena se para.** [T25](T25-ejecutor.md) necesita
[T21](T21-opencode-contrato.md), que necesita [T20](T20-opencode-config.md), que
depende del **no-go de [T14](T14-evaluacion-modelo.md)**; y
[T26](T26-validador.md)–[T29](T29-campana.md) van detrás de T25. El hito SI-4 no
avanza más sin la decisión de plan que T14 dejó sobre la mesa: probar otro
modelo con el mismo banco, sin tocarlo durante la comparación, o acotar SI-3 a
lo que un agente sin herramientas pueda hacer.

**[T62](T62-argv-en-los-programas.md) está cerrada** el 24-sep-2026, y resultó
ser de **cinco** sitios y no de uno: además de `entry!`, empaquetaban la línea
entera como un solo argumento `sosh`, `libsoso::spawn_io` y dos caminos del
kernel. El compilador encontró los 39 consumidores; los cinco productores no,
porque ahí no cambia el tipo, y salieron en QEMU disfrazados de otra cosa
—conexión rechazada, registros YMM, un HTTP 500 de Forja—. Con eso el «argv,
nunca una línea de shell» de C5 es cierto de punta a punta. Y
**[T64](T64-sosh-comillas.md)** cierra el otro extremo el mismo día: `sosh`
entiende ya `'…'`, `"…"` y `\`, así que una ruta con espacios se puede
**escribir** además de pasar.

**[T32](T32-opencode-inventario.md) está cerrada** el 24-sep-2026 con la
revisión de OpenCode fijada (v1.18.32, commit `545f51d2…`) y su matriz de
capacidades. Dos hallazgos cambian lo que SI-6 puede planear: el repositorio
**se movió** a `anomalyco/opencode`, y **`opencode run` no es un modo sin TUI**
—importa `@opentui/*` en el módulo, y lo no interactivo es una rama dentro de
él—. El obstáculo de fondo no es la lista de paquetes sino tres cosas: **no hay
`dlopen`** (ningún `.node` es cargable, ni recompilándolo), no hay pty ni
`inotify`, y **portar OpenCode es portar Bun**. Las incógnitas quedan escritas
como siete sondas para [T33](T33-sondas-abi.md).

**[T33](T33-sondas-abi.md) está completada** el 24-sep-2026: **siete sondas**
en un crate nuevo (`user/soso-agent-probe/`), ejecutadas en siete pases —uno por
capacidad, como la ficha exige— con la tabla en
[`native/probes.json`](native/probes.json), que lleva `esperado` y `observado`
por caso también en los que pasan. Hay además un `cargo xtask test-probe` que
arranca la imagen **dos veces** con un `halt` en medio, porque «sobrevive a
cerrar el descriptor» y «sobrevive al apagado» no son lo mismo.

**Lo que sale bien, y no era obvio**: páginas ejecutables **con recompilación**
—el JIT es viable—, señales suficientes para el timeout de una herramienta,
stdout y stderr separados, hilos con `join` que espera, relojes que no mienten,
tuberías que aguantan 64 KiB y avisan del EOF cuando el escritor muere, y TCP
que reconecta.

**Lo que sale mal**, con reproducción y archivos responsables: **el modelo de
ficheros es por descriptor, no por inodo** —dos descriptores no comparten el
fichero y una escritura parcial trunca la cola, así que SQLite no puede
funcionar, y no por falta de `pread`—; y **no hay protecciones de página más
allá de la escritura** —ni `PROT_EXEC` porque todo es ejecutable, ni `PROT_NONE`
así que no hay guarda de pila—, que es **una** decisión de diseño con los dos
signos. De ahí salen dos defectos abiertos, **[T65](T65-o-excl-no-excluye.md)**
y **[T66](T66-guarda-de-pila-fingida.md)**, y cinco huecos listos para
**[T34](T34-tickets-port.md)**, que es la recomendada ahora.

**[T34](T34-tickets-port.md) está completada** el mismo día, con
[`native/backlog.json`](native/backlog.json) —11 fichas N-xxx, grafo acíclico—
y las tres primeras redactadas. **La ruta elegida es «sustrato POSIX primero»**:
un programa escrito contra POSIX tiene que encontrarse POSIX, y lo que T33 midió
que no lo es no se arregla en el runtime. Portar Bun queda descartado por cuatro
carencias medidas —sin `dlopen` ningún `.node` es cargable, faltan pty e
`inotify`, y el modelo de ficheros impide SQLite—.

**Y queda una decisión de plan sobre la mesa, como la de T14**: el backlog anota
que la alternativa a portar es **un agente nativo en Rust que reutilice el
protocolo** — que es lo que el plan ya está construyendo (`soso-llm`,
`conversation`, `soso-improve`) y lo que las sondas de T33 dicen que ya
funciona. Elegir entre las dos no es de esta ficha.

**Lo que queda habilitado**: **[N-001](native/N-001.md)** (la primera del
backlog y la de mayor riesgo), los defectos
[T65](T65-o-excl-no-excluye.md) y [T66](T66-guarda-de-pila-fingida.md), y los
inventarios [T36](T36-forja-trazabilidad.md) y
[T38](T38-toolchain-inventario.md).
En paralelo siguen habilitadas **T18**, **T45** y **T46**. Los cierres
históricos de T01/T02 no acreditan aún su validación nativa completa.
Entregar al modelo una ficha por sesión, las secciones indicadas
del [contrato](CONTRATO.md), el [contrato nativo](NATIVO.md) y el contexto de
código que la ficha enumera. No necesita cargar todas las fichas.

Cada subplan define alcance, archivos, decisiones, pasos, pruebas, salida y
condiciones de bloqueo. Los nombres propuestos de módulos y comandos se crean
al implementar la ficha; los enlaces de contexto apuntan al código existente.

También pueden empezar sin otras tareas
**[T08](T08-resultado-generacion.md)** (resultado de generación) y
**[T18](T18-puertos-qemu.md)** (puertos QEMU). Trabajar una tarea cada vez por defecto;
varias fichas modifican los mismos manifiestos o módulos de integración.

### Prompt listo para copiar

```text
Implementa docs/self-improvement/T06-render-chat.md.

Lee las secciones del CONTRATO.md que indique la ficha, NATIVO.md y las instrucciones
locales aplicables. Revisa el estado del checkout antes de editar.
Comprueba las dependencias y trabaja solo dentro del alcance de esta ficha.
Usa los símbolos y los fragmentos de código pertinentes; no cargues todos
los planes ni todos los archivos grandes en tu contexto.

Implementa, ejecuta las comprobaciones y conserva sus salidas.
No declares éxito sin evidencia ni sustituyas una prueba de modelo real
por un mock. Si aparece una dependencia ausente, deja una reproducción
y una ficha concreta; no amplíes el alcance sin documentarlo.

Entrega diff, resultado de pruebas, artefactos y dependencias pendientes.
Actualiza solo el estado que puedas acreditar, incluida native_validation
por separado. Una prueba host o un ELF no demuestran ejecución guest.
No empieces la siguiente tarea.
```

Para continuar, sustituir la ruta por la ficha elegida y proporcionar la base,
los artefactos de dependencias y las entradas externas que requiera.

## Decisiones resueltas para reducir el trabajo de arquitectura

- Tipos de chat en core; HTTP/SSE en una crate API separada; sockets en
  userspace. Direcciones de dependencias y firmas en C1–C2.
- Una sesión residente, una generación activa y ninguna generación en cola.
  Cancelación cooperativa y conflicto de listeners definidos en C3–C4.
- Perfil de modelo fijado mediante evidencia; JSON Schema admitido explícito;
  errores y límites sin truncamiento silencioso.
- El primer servidor host es una herramienta de desarrollo. El primer cierre
  útil exige inferencia guest y un parche real validado independientemente.
- Core Rust no_std y adaptadores host/guest existentes. T45–T50 cubren CLI,
  durabilidad, procesos, red/reloj, runner y paquetes por contenido sin Git.
  T51 comprueba el circuito sin Forja ni ejecutores Linux; T44 lo repite.
- Estado, ejecutor, aislamiento, validación y recuperación del coordinador
  están separados para que cada ficha tenga una responsabilidad.
- Datos de evaluación y casos reservados tienen trazabilidad; las mejoras se
  cuentan al aceptarse, no cuando el modelo las afirma.

El contrato precisa el plan padre. En particular, exige token desde la primera
API guest porque la syscall actual escucha por puerto, sin bind por dirección;
y separa captura base de calibración, que requiere un servidor ejecutable.
Las implementaciones puras pueden avanzar antes de terminar la evaluación del
modelo. Esto no rebaja los criterios de cierre de SI-0/SI-1.

## Índice y dependencias

`depends_on` ordena entregas de desarrollo. `native_validation` registra
estado, condiciones `requires` y evidencia de ejecución en soso. Estas
condiciones adicionales se comprueban al validar, sin crear ciclos para
implementar el bootstrap. Todas siguen pendientes salvo entregas exclusivamente
de laboratorio/especificación, que remiten a su consumidor nativo.

Una dependencia indica entrega necesaria, no autorización para implementarla
junto con la ficha. **Pendiente** significa sin evidencia de implementación.
Un resultado de evaluación no-go se registra como bloqueo de consumidores.

Convención de estados y reanudación:
[seguimiento compartido de las skills](../../.claude/skills/soso-architecture/references/planes.md).
Sincronizar `tasks.json`, encabezado/casillas de la ficha y fila del índice;
registrar el resumen durable en `seguimiento/Txx.md` al comenzar esa tarea.

| ID | Entrega | Hito | Dependencias | Estado |
|---|---|---|---|---|
| [T01](T01-base.md) | Capturar una base reproducible sin alterar el checkout | SI-0 | — | Completada |
| [T02](T02-banco.md) | Definir los casos y sus verificadores | SI-0 | T01 | Completada |
| [T03](T03-perfil-modelo.md) | Fijar un modelo y obtener fixtures independientes de su chat | SI-0 / SI-1 | T01 | Completada |
| [T04](T04-dominio-chat.md) | Añadir tipos de conversación compartidos | SI-1 | — | Completada |
| [T05](T05-validacion-chat.md) | Validar historial y esquemas de herramientas | SI-1 | T04 | Completada |
| [T06](T06-render-chat.md) | Renderizar la familia elegida con historial completo | SI-1 | T03, T05 | Completada |
| [T07](T07-parse-herramientas.md) | Extraer llamadas válidas de la salida del modelo | SI-1 | T03, T05 | Completada |
| [T08](T08-resultado-generacion.md) | Devolver motivo de parada y consumo real del runtime | SI-1 / SI-2 | — | Completada |
| [T09](T09-cancelacion-runtime.md) | Añadir cancelación cooperativa a prefill y decode | SI-2 | T08 | Completada |
| [T10](T10-api-json.md) | Crear la crate API y adaptar peticiones JSON | SI-2 | T05, T06, T08 | Completada |
| [T11](T11-http.md) | Leer HTTP fragmentado con límites explícitos | SI-2 | T10 | Completada |
| [T12](T12-respuestas-sse.md) | Emitir respuestas completas y eventos SSE | SI-2 | T07, T08, T10 | Completada |
| [T13](T13-servidor-host.md) | Conectar el mismo runtime a un servidor de desarrollo en host | SI-1 / SI-2 | T09, T11, T12 | Completada |
| [T14](T14-evaluacion-modelo.md) | Medir calidad y fijar presupuestos antes de usar el agente | SI-0 / SI-1 | T02, T03, T13, T48 | Completada — **resultado NO-GO** |
| [T15](T15-sesion-residente.md) | Extraer la sesión residente manteniendo ask | SI-2 | T08, T09 | Completada |
| [T16](T16-servicio-guest.md) | Servir HTTP en guest con el modelo residente | SI-2 | T10, T11, T12, T14, T15 | Hecho (T14 dio **no-go**: no habilitada) |
| [T17](T17-admisiones-cancelacion.md) | Atender ocupado, health y desconexión durante inferencia | SI-2 | T09, T16 | Hecho (e2e T19 pend.) |
| [T18](T18-puertos-qemu.md) | Añadir reenvío HTTP configurable sin colisiones | SI-2 | — | Hecho (e2e T19) |
| [T19](T19-qemu-e2e.md) | Crear la prueba completa de API dentro de soso | SI-2 | T16, T17, T18 | Completada (guest 12/12 con pesos reales) |
| [T20](T20-opencode-config.md) | Configurar OpenCode para el proveedor soso | SI-3 | T14, T19 | Pendiente |
| [T21](T21-opencode-contrato.md) | Capturar el contrato real de OpenCode sin depender del modelo | SI-3 | T20, T47, T48 | Pendiente |
| [T22](T22-primera-mejora.md) | Resolver una tarea real usando la inferencia guest | SI-3 | T02, T19, T21 | Pendiente |
| [T23](T23-estado-coordinador.md) | Crear el formato de tareas y estados del coordinador | SI-4 | T01, T02, T46, T45 | Completada (guest acreditado; destapó T63) |
| [T24](T24-checkout.md) | Preparar una copia de tarea y exportar su parche | SI-4 | T23, T50 | Completada (guest acreditado) |
| [T25](T25-ejecutor.md) | Ejecutar OpenCode con límites y logs | SI-4 | T21, T24, T47, T48 | Pendiente |
| [T26](T26-validador.md) | Validar candidatos y promover solo los aceptados | SI-4 | T24, T25 | Pendiente |
| [T27](T27-reanudacion.md) | Reanudar tras caída sin repetir efectos | SI-4 | T23, T25, T26 | Pendiente |
| [T28](T28-conocimiento.md) | Guardar y recuperar aprendizaje verificado | SI-4 | T26 | Pendiente |
| [T29](T29-campana.md) | Ejecutar una campaña reproducible de diez tareas | SI-4 | T22, T27, T28, T49 | Pendiente |
| [T30](T30-hardware.md) | Medir servicio e inferencia en una máquina física identificada | SI-5 | T19, T29, T48 | Pendiente |
| [T31](T31-restauracion.md) | Demostrar recuperación completa de la instalación | SI-5 | T30 | Pendiente |
| [T32](T32-opencode-inventario.md) | Inventariar dependencias del OpenCode que se quiere portar | SI-6 | T01 | Completada (v1.18.32 fijada) |
| [T33](T33-sondas-abi.md) | Crear sondas pequeñas para las capacidades requeridas | SI-6 | T32, T49 | Completada (7 sondas; abrió T65 y T66) |
| [T65](T65-o-excl-no-excluye.md) | `O_EXCL` no excluye mientras el primer descriptor sigue abierto | SI-4 | — (deriva de T33) | Pendiente |
| [T66](T66-guarda-de-pila-fingida.md) | La guarda de pila de los hilos no existe, y el código finge que sí | SI-4 | — (deriva de T33) | Pendiente |
| [T34](T34-tickets-port.md) | Convertir huecos del port en fichas implementables | SI-6 | T32, T33 | Completada (11 fichas N-xxx) |
| [T35](T35-opencode-nativo.md) | Probar OpenCode nativo en modo no interactivo | SI-6 | T19, T34 | Pendiente |
| [T36](T36-forja-trazabilidad.md) | Vincular fuentes, build y artefacto de Forja | SI-6 | T01 | Pendiente |
| [T37](T37-mejora-nativa-forja.md) | Cerrar una mejora desde OpenCode nativo con build remoto | SI-6 | T22, T35, T36 | Pendiente |
| [T38](T38-toolchain-inventario.md) | Fijar revisiones y dependencias de la toolchain nativa | SI-7 | T01 | Pendiente |
| [T39](T39-bootstrap-libstd.md) | Hacer reproducible el bootstrap de libstd para soso | SI-7 | T38, T50 | Pendiente |
| [T40](T40-compilador-nativo.md) | Descomponer y acreditar el port del compilador | SI-7 | T38, T39 | Pendiente |
| [T41](T41-cargo-offline.md) | Validar Cargo y fuentes reproducibles dentro de soso | SI-7 | T40 | Pendiente |
| [T42](T42-c-link-imagen.md) | Cerrar C, ensamblador y empaquetado por perfil | SI-7 | T38, T41 | Pendiente |
| [T43](T43-validacion-actualizacion-nativa.md) | Validar y recuperar candidatos construidos en soso | SI-7 | T31, T37, T42 | Pendiente |
| [T44](T44-cierre-nativo.md) | Repetir tres mejoras con agente, modelo y build en soso | SI-7 | T29, T43, T51 | Pendiente |
| [T45](T45-cli-capacidades.md) | Unificar órdenes, capacidades y códigos de salida | SI-0 | T01, T02 | Completada |
| [T46](T46-archivos-durables.md) | Acreditar persistencia y actualización de referencias en sosofs | SI-4 | T01 | Completada (reinicio real) |
| [T47](T47-procesos-nativos.md) | Ejecutar procesos con argumentos, canales y límites exactos | SI-4 | T45, T48 | Completada (con límites: T61, T62) |
| [T48](T48-reloj-red.md) | Añadir reloj y transporte nativos para evaluaciones | SI-2 | T45 | Completada |
| [T49](T49-pruebas-guest.md) | Ejecutar casos compartidos desde un runner nativo | SI-0 / SI-4 | T45, T46, T47, T48 | Completada (con límites) |
| [T50](T50-cambios-contenido.md) | Aplicar y exportar cambios sin Git | SI-4 | T01, T46 | Completada (pasos 4–5 aparte) |
| [T51](T51-aceptacion-circuito-nativo.md) | Acreditar todo el circuito de automejora dentro de soso | SI-7 | T29, T35, T37, T41, T42, T43, T49, T50 | Pendiente |
| [T52](T52-tokenizer-merges.md) | Llevar las fusiones BPE al formato .som y al convertidor | SI-1 | — (deriva de T03) | Completada |
| [T53](T53-tokenizer-bpe.md) | Segmentar por fusiones BPE en soso-llm-core | SI-1 | T52 (deriva de T03) | Completada |
| [T54](T54-accept-externo.md) | Aceptar conexiones externas en un listener de userspace | SI-2 | — (deriva de T19) | Completada |
| [T55](T55-accept-sin-plazo.md) | `tcp_accept(fd, 0)` duerme el servidor para siempre | SI-2 | T54 (deriva de T19) | Completada |
| [T56](T56-utf8-tool-parser.md) | El parser de salida parte caracteres UTF-8 y mata el servidor | SI-1 / SI-2 | — (deriva de T19) | Completada |
| [T57](T57-medio-cierre.md) | El medio cierre del cliente mataba la respuesta | SI-2 | T54–T56 (deriva de T19) | Completada |
| [T58](T58-respuesta-con-prompt.md) | La respuesta de chat devolvía el prompt pegado a la generación | SI-2 | — (deriva de T14) | Completada |
| [T59](T59-tool-calls.md) | Herramientas: investigación — **resultado: límite del modelo** | SI-2 / SI-3 | — (deriva de T14) | Cerrada |
| [T60](T60-validacion-peticion.md) | Errores de la API: clasificación y doble respuesta en streaming | SI-2 | — (deriva de T14) | Completada |
| [T61](T61-pipe-no-bloqueante.md) | Lectura de tuberías con plazo, sin bloquear | SI-4 | — (deriva de T47) | Completada |
| [T62](T62-argv-en-los-programas.md) | Los programas reciben los argumentos juntados, no su argv | SI-4 | — (deriva de T47) | Completada (5 productores; abrió T64) |
| [T64](T64-sosh-comillas.md) | `sosh` no entiende comillas: una ruta con espacios es inescribible | SI-4 | T62 | Completada (guest acreditado) |
| [T63](T63-generacion-rota-encalla.md) | Desencallar una referencia durable con una generación rota | SI-4 | T46 | Completada (guest acreditado; desbloquea T27) |

## Condiciones adicionales de entrada

Los números del índice no describen por sí solos todas las condiciones. Estas
son obligatorias y también constan en [tasks.json](tasks.json):

- **[T03](T03-perfil-modelo.md)**: Pesos, tokenizer y plantilla originales de una revisión identificada.
- **[T06](T06-render-chat.md)**: Las divergencias de tokenizer detectadas en T03 están corregidas y verificadas.
- **[T14](T14-evaluacion-modelo.md)**: Pesos reales disponibles; un informe no-go no habilita las tareas consumidoras.
- **Criterio de la evaluación**: [evaluacion.md](evaluacion.md) — qué cuenta como éxito, los cinco desenlaces y de dónde salen los presupuestos.
- **[T16](T16-servicio-guest.md)**: T14 tiene resultado go, además de informe terminado.
- **[T19](T19-qemu-e2e.md)**: Modo de integración con pesos reales y entorno QEMU disponible.
- **[T54](T54-accept-externo.md)**: Entorno QEMU con reenvío de puertos (T18) y un servidor de userspace escuchando.
- **[T22](T22-primera-mejora.md)**: T14 mantiene resultado go para la misma identidad de modelo.
- **[T30](T30-hardware.md)**: Máquina física identificada, accesible y con red validada.
- **[T31](T31-restauracion.md)**: Destino de recuperación identificado y acceso de escritura autorizado.
- **[T35](T35-opencode-nativo.md)**: Todas las fichas N-xxx obligatorias de T34 están implementadas y sus sondas pasan.
- **[T39](T39-bootstrap-libstd.md)**: Revisión de toolchain de T38 y recursos de compilación disponibles.
- **[T40](T40-compilador-nativo.md)**: Fichas C-xxx necesarias implementadas antes del cierre de integración.
- **[T41](T41-cargo-offline.md)**: Cargo ejecutable en soso; las C-xxx adicionales se completan antes de la evaluación.
- **[T42](T42-c-link-imagen.md)**: Cada herramienta ausente se implementa mediante C-xxx antes del build de perfil.
- **[T43](T43-validacion-actualizacion-nativa.md)**: Destino de validación independiente y procedimiento de recuperación verificado.

**Los ports todavía no se pueden dividir honestamente por símbolo sin
inspeccionarlos.** T32/T38 fijan revisiones y dependencias; T33 produce sondas;
T34 y T40–T42 generan fichas N-xxx/C-xxx por hueco real, con firmas y archivos.
Estas fichas derivadas se ejecutan individualmente. T35/T37/T43/T44 son
integraciones condicionadas a su implementación, no encargos de «portar todo».

T33 y T40–T42 incluyen recetas repetibles por capacidad/componente. Ejecutar
un solo pase acotado cada vez y registrar cobertura de los restantes. Si se
necesita delegarlos a otro modelo, materializar primero la ficha N-xxx/C-xxx
correspondiente con la [plantilla](PLANTILLA.md).

## Cierres del plan padre

| Hito | Entregas y evidencia necesarias |
|---|---|
| SI-0 | T01–T03 y calibración T14; base reconstruible, banco y medidas |
| SI-1 | T04–T09, fixtures T03 y resultado go T14; compatibilidad y calidad |
| SI-2 | T10–T19 y evaluación T14 repetida contra el guest |
| SI-3 | T20–T22; herramienta real y primer parche aceptado |
| SI-4 | T23–T29 y mecanismos T45–T50; cinco mejoras de diez y recuperación; repetición íntegramente nativa en T51 |
| SI-5 | T30–T31; campaña física y restauración kernel/rootfs |
| SI-6 | T32–T37 más todas las N-xxx necesarias; OpenCode nativo y Forja trazable |
| SI-7 | T38–T44, T45–T51 y todas las N/C necesarias; campaña nativa T51 y tres ciclos finales T44 |

La numeración sirve de orientación. Por ejemplo, T23 puede prepararse después
del banco, y T32/T38 pueden investigarse sin esperar una campaña en hardware.

## Formato de entrega

Guardar por tarea en `target/self-improvement/tasks/Txx/` en host o
`/var/self-improvement/tasks/Txx/` en soso; raíz configurable según NATIVO.md:

```text
resultado.md     objetivo, cambio, evidencia, límites y siguiente estado
commands.json    argv, cwd, versiones, exit codes y duración
logs/            salida completa de verificaciones
artifacts.json   base, diff y hashes de artefactos relevantes
```

Para documentación basta verificar vínculos, estructura y coherencia. Para
código, ejecutar las pruebas de la ficha; al promover una base, las puertas
globales C6. Actualizar el estado del índice y de tasks.json juntos.

Este desglose crea documentación y contratos de trabajo. No instala modelos,
no ejecuta campañas ni modifica runtime/kernel.
