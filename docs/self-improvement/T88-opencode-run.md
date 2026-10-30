# T88 — `opencode run` no interactivo (Sal)

**Hito:** SI-6 · **Tipo:** Agente · **Estado:** ver catálogo

**Dependencias:** T87

## Objetivo

CLI `opencode run`: `opencode.json`, `AGENTS.md`, bucle herramientas + SSE a `soso-llm`, límites, sesiones JSON en disco, `--format json`. Herramientas: read, write, edit, glob, grep, list, bash (vía sosh), todowrite.

## Comprobación

Host/guest con modelo mockeado (fixture SSE); una sesión que edita un fichero y termina con evento `done`.
