# T57 — El medio cierre del cliente mataba la respuesta

**Hito:** SI-2 · **Tipo:** Corrección de kernel (red) + userspace · **Estado:** hecha (2026-09-23).

**Dependencias:** [T54](T54-accept-externo.md), [T55](T55-accept-sin-plazo.md),
[T56](T56-utf8-tool-parser.md). **La origina:** [T19](T19-qemu-e2e.md).

## Problema reproducido

Con T54–T56 resueltos, `curl` recibía su respuesta pero **el cliente del arnés
seguía sin recibir nada**, y al instante: la fase `core_invariants` entera
duraba 0,0 s y `chat_json` daba «sin línea de estado».

La diferencia entre los dos clientes es una línea:

```rust
// crates/soso-llm-api/src/e2e.rs, ApiClient
stream.shutdown(std::net::Shutdown::Write).ok();   // medio cierre tras la petición
```

`curl` no hace medio cierre; el arnés sí. Un medio cierre es corriente en HTTP
y sólo dice «no mando más», no «no quiero respuesta».

## Causa (dos capas)

1. **Kernel.** `poll_entry`, para un socket `Connected`, hacía:

   ```rust
   if s.state() == CloseWait && !s.can_recv() { s.close(); }
   ```

   Es decir, en cuanto el par mandaba su FIN, **el kernel cerraba nuestro lado
   por su cuenta**. La aplicación escribía después su respuesta sobre un socket
   ya cerrado y el cliente recibía cero bytes.

   Quitar ese cierre a secas **rompe otra cosa**: era también la única forma de
   que una lectura diera EOF, así que `soso-forja all --host …` se colgaba para
   siempre esperando al servidor remoto (lo cazó `cargo xtask test`).

   La separación correcta es la que ya existía para el loopback: **EOF de
   lectura no es cierre de la conexión**. `tcp_estado` devuelve ahora `Cerrado`
   —es decir, EOF para las lecturas— cuando el par ha mandado FIN y no queda
   nada por leer, y `entry.closed` sigue en `false`, de modo que
   `tcp_try_write` puede seguir enviando la respuesta.

2. **Userspace.** `serve_poll::active_peer_closed` trataba `read == 0` como
   «el cliente se ha ido» y cancelaba la generación. Con un cliente que hace
   medio cierre, eso aborta toda petición nada más empezar. Ahora EOF no
   cancela: si el cliente se ha ido de verdad, la escritura de la respuesta
   falla y ahí sí se sabe.

## Alcance

- `kernel/src/net/mod.rs`: `tcp_estado`.
- `kernel/src/net/tcp_user.rs`: `poll_entry` (rama `Connected`), reexporta los
  estados de smoltcp.
- `user/soso-llm/src/serve_poll.rs`: `active_peer_closed`.

## Comprobación

- `cargo xtask test` completo, con atención a `forja: hola-std remoto`, que es
  el que detecta la regresión de EOF.
- Campaña guest de [T19](T19-qemu-e2e.md): la fase `core_invariants` pasa de
  0,0 s (todo abortado) a tiempos reales de generación.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

Límite: un cliente que desaparece **durante** una generación ya no se detecta
por lectura; se detectará al fallar la escritura de la respuesta, es decir al
final. Si hiciera falta detectarlo antes, la vía es el estado del socket (RST),
no volver a tratar EOF como desconexión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La comprobación vive en la campaña de T19 y
debería repetirse desde el runner nativo de [T49](T49-pruebas-guest.md).

Validación nativa: **pendiente**.
