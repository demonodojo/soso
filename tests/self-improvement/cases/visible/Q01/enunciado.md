# Q01 — Texto simple: una pregunta, una respuesta

Se manda `peticion.json` a `POST /v1/chat/completions`. La respuesta debe ser
200 con un único mensaje de asistente con texto, sin llamadas a herramientas,
`finish_reason: stop` y `usage` coherente.
