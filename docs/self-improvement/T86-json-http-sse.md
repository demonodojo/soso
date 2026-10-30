# T86 — `json`, `http` y `sse` en opencode-sal

**Hito:** SI-6 · **Tipo:** Biblioteca · **Estado:** ver catálogo

**Dependencias:** T84, T85

## Objetivo

Módulos `json.sal`, `http.sal`, `sse.sal` en `demonodojo/opencode-sal` (cliente HTTP/1.1 sin TLS, parser SSE). Pruebas host con servidor TCP falso y respuestas grabadas (sin HTTP real ni modelo).

## Comprobación

```sh
cd opencode-sal && python3 tests/mock_http_server.py & sal test tests/http_mock.sal
```
