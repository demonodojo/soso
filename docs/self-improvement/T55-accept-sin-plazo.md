# T55 — `tcp_accept(fd, 0)` duerme el servidor para siempre

**Hito:** SI-2 · **Tipo:** Corrección de userspace · **Estado:** hecha (2026-09-23).

**Dependencias:** [T54](T54-accept-externo.md). **La origina:** [T19](T19-qemu-e2e.md).

## Problema reproducido

Con [T54](T54-accept-externo.md) la API del guest ya contesta a `GET /health`,
pero **una petición de generación no termina nunca**. El cliente espera hasta su
propio plazo y recibe una respuesta vacía; como la sesión admite una sola
generación, todas las peticiones siguientes reciben `429` legítimamente y el
informe de T19 queda lleno de fallos que parecen de admisión.

Medición (2026-09-23, guest QEMU con el modelo real, 1 vCPU, 2 GiB):

```sh
curl -m 290 -H "Authorization: Bearer …" -H "Content-Type: application/json" \
  -d '{"model":"…","messages":[{"role":"user","content":"di hola"}],"max_tokens":2}' \
  http://127.0.0.1:17299/v1/chat/completions
# HTTP=000 t=290.00   ← ni un byte en casi cinco minutos, por dos tokens
```

Lo decisivo es que **el guest está parado, no lento**:

- QEMU acumula **7 s de CPU en 326 s** de reloj.
- `ps` en el guest: `7 6 socket /bin/soso-llm` — bloqueado en un socket, no
  `corriendo`.

Un modelo generando dos tokens consume CPU; éste no consume ninguna. No es
presupuesto de generación: es un bloqueo.

## Causa

`sys_tcp_accept` pasa su `timeout_ms` por `socket_deadline`, y ahí **`0`
significa «sin plazo»**, igual que en `read_timeout`: el proceso se bloquea
indefinidamente. Toda la userspace lo usa así (`init` pasa `5_000`), pero
`soso-llm` pasaba `0` creyendo que era «no bloquees»:

- `user/soso-llm/src/serve_poll.rs`, `try_accept_aux`:
  `sys::tcp_accept(env.http_listener_fd, 0)`. Se llama desde
  `poll_during_generation`, es decir, en **cada punto de control de la
  generación**. El primero se quedaba ahí para siempre y la generación no
  avanzaba ni un token.
- `user/soso-llm/src/serve.rs`, bucle principal:
  `TcpFd::accept(&http_listener, 0)` seguido de `TcpFd::accept(&ask_listener, 0)`.
  El primero no vuelve nunca, así que **el listener de `ask` no se atendía
  jamás** mientras `serve` estuviera vivo; ése era el fallo `ask_con_serve` de
  la campaña.

El propio módulo ya tenía la constante correcta y el nombre correcto —
`const POLL_READ_MS: u64 = 1; // no bloqueante efectivo` — y la usaba en las
lecturas; sólo el `accept` se quedó con el `0`.

## Contrato técnico

`tcp_accept(fd, timeout_ms)`: `0` = sin plazo (bloquea hasta que haya
conexión); `n > 0` = espera como mucho `n` ms y devuelve `EAGAIN`. No se cambia
la syscall: cambiarla rompería a quien ya usa `0` a propósito.

Un servidor que atiende **dos** listeners o que sondea mientras genera no puede
usar `0` en ninguno de los dos.

## Alcance

`user/soso-llm/src/serve.rs` y `user/soso-llm/src/serve_poll.rs`.

## Corrección

- `try_accept_aux` usa `POLL_READ_MS` (1 ms), como las lecturas de sondeo de su
  propio módulo.
- El bucle principal usa `ACCEPT_POLL_MS` (10 ms) en los dos `accept` y baja la
  siesta a 20 ms, de forma que alterna entre HTTP y `ask` sin quemar CPU.

## Comprobación

- `cargo xtask test-llm-api --model-dir … --profile …`: la generación termina y
  `ask_con_serve` deja de agotar su plazo.
- `cargo xtask test` sigue verde.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

Límite: no se ha medido el coste del sondeo de 1 ms por punto de control con
una generación larga; si apareciera, la salida es un `accept` realmente no
bloqueante en la ABI, no subir la constante a ciegas.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La comprobación vive en la campaña de T19 y
debería repetirse desde el runner nativo de [T49](T49-pruebas-guest.md).

Validación nativa: **pendiente**.
