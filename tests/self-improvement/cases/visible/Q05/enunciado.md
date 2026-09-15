# Q05 — Contenido null en el historial

Un mensaje de asistente con `content: null` y `tool_calls` es lo que manda un
cliente OpenAI corriente. Debe aceptarse: 200 y respuesta válida, nunca un 400
por el null.
