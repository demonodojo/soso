# Fixtures wire de OpenCode (T21)

Capturados el 2026-10-07 con **OpenCode 1.18.34** (`opencode` sha256
`9ca0b9953d49997601655e54f846a3efa464f237e47c6f1b04716d0f2e64c4c2`, runtime bun 1.3.14,
`@ai-sdk/openai-compatible`) contra un servidor guionizado, con la configuración de
[`opencode.json`](../../../opencode.json) (agente `soso-improve`, sin agente `title`).

El guion de la sesión: leer `hola.txt` → editar (`mundo`→`soso`) → `git status --short` → texto final.

| Archivo | Qué es |
|---|---|
| `peticion-01-turno-1-lectura.json` … `peticion-04-turno-4-final.json` | Cuerpos `POST /v1/chat/completions` tal como los envía OpenCode |
| `cabeceras-0N.txt` | Cabeceras de cada una (token, ids de sesión y puerto redactados) |
| `titulo-sin-desactivar.json` | La petición auxiliar de título, que **sólo** aparece si no se desactiva `agent.title` |

Saneado: la ruta del repositorio temporal pasa a `/work/repo`; el token, `ses_<id>` y el puerto, redactados.

Qué se ve en ellas (y el servidor tiene que aceptar):
- `stream: true`, `stream_options.include_usage: true`, `max_tokens: 2048` (el `limit.output` de la config), sin `temperature`/`top_p`/`seed`.
- `tool_choice: "auto"`, sin `parallel_tool_calls`; 8 herramientas (`bash`, `edit`, `glob`, `grep`, `read`, `skill`, `todowrite`, `write`).
- Esquemas de herramienta con `$schema`, `minimum`, `maximum`, `exclusiveMinimum` (no estaban soportados: corregido en T21).
- El historial reenvía los ids de llamada que el servidor generó (`call_N`): tienen que ser únicos en la conversación (corregido en T21).
- Cabeceras: `Content-Length` (no chunked), `Authorization: Bearer`, `Connection: keep-alive`, `Accept-Encoding: gzip…`; el servidor contesta sin comprimir y con `Connection: close`.

Se contrastan con el decodificador y el validador reales en
[`crates/soso-llm-api/tests/opencode_contrato.rs`](../../../crates/soso-llm-api/tests/opencode_contrato.rs).
