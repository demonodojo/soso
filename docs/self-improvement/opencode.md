# OpenCode para soso — configuración efectiva (T20)

Versión fijada: **OpenCode 1.18.34** (`~/.opencode/bin/opencode`). Esquema: `https://opencode.ai/config.json`, descargado el 2026-10-07 (`target/self-improvement/tasks/T20/config-schema.json`).

## Archivos
- [`opencode.json`](../../opencode.json): proveedor `soso` (`@ai-sdk/openai-compatible`, `http://127.0.0.1:7422/v1`), un solo modelo `soso-coder` (= `qwen2.5-coder-7b`, perfil con GO 10/10), `model` y `small_model` apuntan a él.
- [`.opencode/agents/soso-improve.md`](../../.opencode/agents/soso-improve.md): agente por defecto.

## Decisiones
- **Token**: `{env:SOSO_LLM_API_KEY}`; nunca en el repositorio (C3). La configuración efectiva lo muestra como `***`.
- **Sólo soso**: `enabled_providers: ["soso"]`; `opencode models` lista únicamente `soso/soso-coder`. `share: disabled`, `autoupdate: false`.
- **Agentes**: se desactivan `build`, `plan`, `general`, `explore` y **`title`** (T21: OpenCode manda la petición de título **en paralelo** a la principal y soso admite una sola generación a la vez, así que con un 7B real el título ocuparía el generador durante minutos y bloquearía el primer turno; sin `title` la sesión son sólo peticiones con herramientas); `summary` y `compaction` son internos y sin modelo propio, así que usan `small_model` = soso. Sin plugins ni comandos.
- **Protocolo en el prompt del agente (T22, 2026-10-08)**: con el prompt vago original el 7B narraba en vez de llamar a herramientas; con un protocolo explícito (una sola llamada por respuesta y nada más, no inventar resultados ni escribir `<tool_response>`, no editar sin leer, pasos 1–5) lee `TASK.md` y el archivo de verdad. Ver `seguimiento/T22.md`.
- **Permisos de `soso-improve`**: leer/buscar/editar sí; `bash` denegado salvo `cargo test|check|build|fmt` y `git status|diff|log`; `task`, `webfetch`, `websearch` y `external_directory` denegados. No hay autoaprobación universal. `steps: 40`.
- **Límites**: `limit.context` 32768 y `limit.output` 2048. **Son declarados, no medidos**: el banco de T14 sólo mide salidas de ≤19 tokens; T21 comprobará con tráfico real. El servidor devuelve 422 `context_length_exceeded` si se pasan (C3).
- **Timeouts** (de la campaña T14, `campana-t14-qwen2.5-coder-7b.json`: mediana 160 212 ms, máximo 593 024 ms): `timeout` y `chunkTimeout` = **1 186 048 ms** (el sugerido, 2× el máximo). `chunkTimeout` debe cubrir el primer token, que es lo más lento.

## Hashes (sha256)
- `opencode.json`: `73dc1e89e55a2da24496596f924e42afb1cc297ab9931802057ba787098188c9`
- `.opencode/agents/soso-improve.md`: `94a4b3e78970c8fcbf0160af6c33bafd468bad1e18ff9c2a9a0bd37b69034949`
- Proveedor: `@ai-sdk/openai-compatible` (OpenCode lo resuelve en ejecución; la versión exacta se fija en T35, que instala sin red).

## Cómo se comprobó (sin lanzar inferencia)
Entorno aislado (`env -i`, `HOME` y `XDG_*` en `target/self-improvement/tasks/T20/aislado`):

    opencode debug config      # config-efectiva.json (plugin: [], command: {})
    opencode models            # modelos-todos.txt: soso/soso-coder
    opencode agent list        # agentes.txt
    opencode debug agent {title,summary,compaction}   # agentes-internos.txt

Validación contra el esquema: sólo falla `model`/`small_model`, porque el `enum` del esquema lista los modelos del catálogo público y un proveedor propio no puede estar ahí; OpenCode los acepta (lo confirma `debug config`). Todo lo demás valida.

## Qué se aprendió contrastando con OpenCode real (T21)
- **Primera ejecución**: en un entorno nuevo OpenCode instala `@ai-sdk/openai-compatible` desde la red y puede quedarse en `init` más de un minuto; las pruebas comparten caché (`target/self-improvement/tasks/T21-cache`). Para T35 (nativo, sin red) el paquete tiene que venir ya instalado.
- **Fallos del servidor, vistos desde OpenCode**: un chunk SSE que no es JSON → error `JSON parsing failed` y exit 1; HTTP 500 o 429 → **reintenta sin fin** (6 peticiones en 45 s); una respuesta SSE que se corta sin `finish_reason` ni `[DONE]` → reintenta en bucle (440 peticiones en 45 s); una llamada con argumentos JSON incompletos → bucle de resultados `invalid` (170 peticiones en 45 s). Por eso el servidor debe rechazar con 4xx/5xx sólo cuando de verdad no pueda atender, y no devolver nunca una llamada a medias (C3 ya lo exige).
- **Herramienta que falla** (`edit` con `oldString` inexistente): OpenCode lo devuelve al modelo como resultado de herramienta y sigue; no rompe la sesión.

## Límites
No certifica inferencia con OpenCode (T21), ni la ejecución nativa en soso (T35: runtime, proveedor y herramientas offline, sin home Linux). Los permisos de bash admiten `cargo` y `git` de sólo lectura; los comandos concretos de cada tarea los ajusta `TASK.md`.
