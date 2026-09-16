# Contrato común de las tareas

**Estado:** diseño revisado el 16 de septiembre de 2026. Reutilizar core y
adaptadores de soso-improve ya existentes; crear solo las capacidades pendientes.
El requisito de ejecución completa dentro de soso se concreta en [NATIVO.md](NATIVO.md).
Las rutas se expresan desde la raíz del repositorio salvo indicación contraria.

## Cómo ejecutar una ficha

1. Leer su objetivo, las secciones pertinentes de este contrato y los archivos
   de «Contexto mínimo». No cargar todos los subplanes en el contexto.
2. Comprobar la evidencia de las dependencias. Una casilla marcada sin diff,
   pruebas o artefactos no acredita que una dependencia esté satisfecha.
3. Revisar el inventario de fuentes (y `git status --short` si hay Git) y las instrucciones locales aplicables. Mantener
   los cambios existentes; trabajar sobre una base identificada en copia aislada.
4. Implementar exclusivamente la ficha. Objetivo orientativo: 1–3 archivos de
   lógica y sus pruebas; manifiestos y documentación pueden añadir archivos.
   Si exige resolver otra arquitectura, registrar la dependencia y separarla.
5. Ejecutar sus comprobaciones. Una prueba sin casos ejecutados no es un pase.
   Ante un fallo ajeno, conservar el log y explicar su relación con la tarea.
6. Entregar el paquete de cambios y un informe bajo la raíz de artefactos de NATIVO.md, en `tasks/Txx/`, con
   `resultado.md`, comandos, códigos de salida y artefactos. Actualizar la fila
   del índice solo al acreditar el cierre. No marcar como cerrado el hito padre
   hasta pasar su evaluación de integración.

Las tareas de inspección generan evidencia o fichas de implementación nuevas.
Las de integración necesitan componentes ya implementados. Ninguna ficha de
auditoría por sí sola completa un port nativo.

## C1. Arquitectura y dependencias entre crates

```text
sosomodel
    ↑
soso-llm-core   dominio de chat + render + generación; no HTTP
    ↑
soso-llm-api    JSON del proveedor + HTTP + SSE; no sockets ni syscalls
    ↑                       ↑
soso-llm (guest)       ejemplo servidor host

soso-improve-core (no_std + alloc) → política, evaluación, estado y casos
    ↑                       ↑
tools/soso-improve      user/soso-improve
adaptador std, lab      adaptador libsoso, producto
```

- `soso-llm-core` conserva `no_std + alloc` por defecto. `soso-llm-api` también.
  HTTP nunca se implementa en el kernel. `soso-http` es el cliente existente;
  no convertirlo incidentalmente en servidor.
- Serialización: `serde` con `default-features=false`, `alloc` y `derive`;
  `serde_json` con `default-features=false` y `alloc`. Ya hay precedente de
  `serde_json` en `user/soso-hf/Cargo.toml`. Comprobar que el build guest no
  active `std` accidentalmente. Fijar la resolución en los lockfiles propios
  de los workspaces afectados.
- Dominio propuesto en `soso-llm-core/src/conversation.rs`:
  `Role`, `Message`, `ToolDefinition`, `ToolCall`, `ToolChoice`, `ChatInput`,
  `ModelProfile`, `ChatError`. HTTP adapta estos tipos, no los duplica.
- `Message`: rol, `content: Option<String>`, `tool_calls: Vec<ToolCall>` y
  `tool_call_id: Option<String>`. `ToolCall`: id, nombre y argumentos JSON
  serializados en `String`; `ToolDefinition`: nombre, descripción y
  `parameters: serde_json::Value`. El campo `type: function` pertenece al wire.
- `ChatInput`: mensajes, herramientas y selección `Auto | None | Required |
  Named(String)`. El primer perfil genera como máximo una llamada por turno,
  pero el historial acepta y valida varias llamadas de turnos anteriores.
- El perfil tiene id estable `soso-coder`, directorio real, familia, hashes de
  pesos/tokenizer/plantilla, contexto validado, máximo de salida y stop IDs.
  El fichero `model-lock.json` es metadato de la evaluación; su ubicación se
  entrega como entrada. No descubrir el modelo por «primero del catálogo».

## C2. Chat, generación y separación de salida

- Mantener `chat::render` y los métodos públicos de generación actuales como
  envoltorios compatibles. Añadir `conversation::render_messages` para el
  perfil nuevo, sin cambiar el significado de `ask` antiguo.
- Una familia por entrega. Candidato inicial: Qwen2.5-Coder-3B-Instruct ya
  contemplado por el proyecto; T03 debe comprobar tokenizer y plantilla reales.
  Su capacidad de usar herramientas no se da por demostrada.
- No inventar una plantilla a partir de su nombre. T03 aporta entrada,
  plantilla original, texto renderizado y tokens de referencia. T06 debe
  identificar divergencias; corregir un tokenizer incompatible será una ficha
  separada, no un parche oculto al renderer.
- El runtime devuelve tokens generados y motivo real de parada. Dominio
  propuesto: `GenerationReport { generated, prompt_tokens, sampled_tokens,
  stop }`, `StopReason { StopToken, Limit, Cancelled }`; errores numéricos o
  de E/S se devuelven como error, no como terminación correcta.
- `sampled_tokens` cuenta también el stop token consumido; `generated` excluye
  delimitadores de parada. Las estadísticas HTTP conservan esa convención.
- `GenerationObserver` recibe tokens y puntos de control de prefill/capa/decode;
  puede cancelar. Los callbacks antiguos siguen funcionando con un adaptador.
  No añadir procesos ni hilos al runtime para atender la red.
- Las respuestas HTTP conservan Unicode, saltos de línea y código literalmente.
  No pasar por `texto_ask_seguro`, puntos de progreso ni filtros de terminal.
  Escribir diagnósticos a serie/stderr. Limpiar callbacks, estado KV por petición
  y workers también cuando haya desconexión o error.

## C3. API inicial

| Elemento | Decisión |
|---|---|
| Puerto guest / ejemplo host | 7422; configurable; host escucha en 127.0.0.1 |
| Endpoints | `GET /health`, `GET /v1/models`, `POST /v1/chat/completions` |
| Cabeceras / cuerpo | Máximo 16 KiB / 1 MiB; aritmética comprobada |
| Mensajes / herramientas | Máximo 64 / 32; máximo 64 KiB por argumentos de llamada |
| HTTP | 1.1, una petición por conexión, `Connection: close` |
| Cuerpo de entrada | `Content-Length`; sin compresión; rechazar transferencias chunked inicialmente |
| Validación | GET sin cuerpo; POST JSON UTF-8 completo; ambigüedad de longitud = error |
| Generación | Una activa; cero peticiones de generación en cola |
| Admisión durante generación | `/health` operativo y generación adicional → 429 |
| SSE | `text/event-stream`; `data: <JSON>\n\n`; cierre `data: [DONE]\n\n` |
| Fallo tras enviar HTTP 200 | Evento de error y cierre, sin fingir `finish_reason: stop` |
| Errores antes de cabeceras | JSON `error: {message,type,code}` y estado HTTP significativo |

Mapeo: 400 petición/historial inválido, 401 autenticación inválida, 404 ruta o
modelo desconocido, 405 método, 413 cuerpo, 415 tipo de contenido, 422 contexto
o esquema no soportado, 429 ocupado, 500 inferencia, 503 modelo no disponible.
No convertir un error en texto del asistente. T21 contrastará este contrato
con peticiones reales de la versión fijada de OpenCode y añadirá fixtures
para cualquier ampliación necesaria.

Campos iniciales: `model`, `messages`, `tools`, `tool_choice`, `stream`,
`max_tokens`/`max_completion_tokens` (si llegan ambos distintos, error),
`temperature`, `top_p`, `seed`, `stream_options.include_usage` y
`parallel_tool_calls: false`. Aceptar `n` ausente o 1. Rechazar modalidades
no implementadas; registrar campos adicionales para resolverlos con evidencia.
El límite de contexto es `tokens(render completo) + reserva_salida <= límite`;
no truncar historial ni invocar descarte KV para esconder un desbordamiento.

`tool_choice: none` produce texto; `required` y nombre fijado exigen una llamada
válida de esa clase o un error. `auto` permite texto o llamada. Los argumentos
se validan completos antes de emitir una llamada ejecutable. En la primera
versión se puede emitir la llamada completa como un único delta SSE al acabar;
las pruebas del cliente también incluyen deltas fragmentados.

En guest `tcp_listen` recibe solo un puerto: **no hay bind por dirección en esa
API**. El servicio HTTP exige token desde su primera versión, incluso en QEMU.
Recibirlo de un fichero provisionado fuera del árbol y no registrarlo. El host
lo recibe por `SOSO_LLM_API_KEY`. Restringir el forward QEMU a 127.0.0.1. No
suponer que el listener guest está limitado a loopback por el texto del banner.

## C4. Servicio residente y red

`soso-llm serve --model <nombre> --port <puerto> --token-file <ruta>` es la
interfaz guest propuesta. Un único proceso puede poseer los listeners de
`ask` (7420) y HTTP, y una única sesión cargada. Si otro proceso ya ocupa 7420,
informar del conflicto; no matar procesos ni abrir una segunda carga oculta.
El arranque perezoso de `askd` mantiene su comportamiento anterior cuando el
servicio HTTP no está habilitado.

T15 extrae el dueño de la sesión; T16 implementa el camino HTTP secuencial;
T17 añade sondeo cooperativo acotado entre capas/tokens para atender health,
ocupado y cancelación. Ese sondeo nunca llama recursivamente a generación.
No usar `read_timeout(..., 0)` sin comprobar su semántica: distinguir EAGAIN,
EOF y error con llamadas breves y buffers parciales persistentes.

La latencia de cancelación está limitada por el tramo de cálculo indivisible
en curso más un sondeo. Medirla; no prometer interrupción inmediata de una
matvec o un DMA. Los timeouts se configuran a partir de T14, con límites
finitos de cabeceras/cuerpo y tiempo total de generación.

## C5. Coordinador, evidencia y aceptación

Extender `crates/soso-improve-core` y los dos adaptadores existentes
`tools/soso-improve` / `user/soso-improve`; comandos propuestos `prepare`, `run`,
`validate`, `resume`, `report`. T45 unifica órdenes y errores; T46–T48 aportan
almacenamiento, procesos, reloj y red. JSON versionado con `schema_version: 1`.
Los comandos se ejecutan con arrays de argv y cwd explícito, sin `sh -c` para
interpolar datos del modelo. Logs completos fuera del prompt, resumen acotado
para el agente. Evitar secretos y variables de entorno ajenas en manifiestos.

- Estados: `pendiente`, `reproduciendo`, `editando`, `verificando`, `aceptada`,
  `rechazada`, `bloqueada`. Solo el validador puede escribir `aceptada`.
- Una tarea contiene id, base, problema, rutas editables, comprobaciones como
  argv, criterios, límites y hashes de entradas. Los criterios reservados se
  almacenan fuera de la instancia modificable por el agente/candidato.
- 3 intentos, 30 herramientas; tokens y tiempo según calibración. Si la versión
  de OpenCode no expone un dato, declararlo no disponible y no inventarlo.
- Copia de fuentes por contenido T01/T50 = separación de cambios. El candidato
  se ejecuta en una instancia distinta de la autoridad validadora o con
  contención acreditada. T26/T43 definen receptor, ejecución y recuperación
  nativos; directorios separados y permisos de OpenCode no bastan.
- Publicar un paquete candidato por hash no implica desplegar. No ejecutar OTA
  ni escribir discos físicos desde el coordinador inicial.
- Estado durable mediante el contrato T46 y semántica sosofs comprobada. Diario de
  intención/resultado para operaciones con efectos; tras caída, comprobar
  evidencia antes de repetir. Un PID reciclado no identifica una ejecución.

## C6. Comprobaciones y documentación

Los tests host de HTTP usan entradas o servidores locales controlados; los
tests de modelos reales son una suite de integración explícita. Las pruebas
unitarias no descargan pesos ni llaman a servicios externos.

Desde raíz: `cargo test -p soso-llm-core --features std`. Para el binario guest:
ejecutar `cargo build --release -p soso-llm` **con cwd `user/`**, que contiene
su configuración de target y linker. Para crates nuevas usar los comandos
de la ficha después de crearlas. Antes de promover una base, ejecutar
`cargo xtask check` y `cargo xtask test` durante desarrollo, conservando logs
de ambos. En el circuito nativo T49/T43 deben mapear todas las comprobaciones
obligatorias a ejecutores soso equivalentes. Un check sin sustitución nativa
bloquea T51/T44; omitirlo no acredita paridad. No exigir Cargo/libtest nativos
para el runner temprano T49; los candidatos compilados sí requieren T40/T41.

Actualizar las skills de dominio y sus espejos al cambiar arquitectura o
procedimientos; `MANUAL-USUARIO.md` cuando aparezcan comandos o UX nuevos.
Estas modificaciones de documentación están permitidas junto a los archivos
de cada ficha. Un cambio solo documental no requiere arrancar QEMU.

Fuentes OpenCode: [proveedores](https://opencode.ai/docs/providers/),
[configuración](https://opencode.ai/docs/config/),
[CLI](https://opencode.ai/docs/cli/) y
[permisos](https://opencode.ai/docs/permissions/), consultadas para el plan
principal. Revalidar contra la revisión fijada antes de implementar T20–T21.

## C7. Ejecución nativa y cierre

Leer [NATIVO.md](NATIVO.md) para toda ficha de automejora. Política compartida
en no_std, adapters guest reales y evidencia de ejecución son requisitos
distintos. No introducir Python ni dependencias funcionales Linux permanentes.
T45–T50 resuelven mecanismos; T51 integra el circuito; T44 repite tres mejoras.
`depends_on` ordena desarrollo y `native_validation.requires` su validación
nativa posterior. Registrar ambos sin ciclos artificiales de bootstrap y
sin convertir un cierre histórico host en ejecución guest verificada.
