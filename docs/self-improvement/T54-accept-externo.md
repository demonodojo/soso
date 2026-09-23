# T54 — Aceptar conexiones externas en un listener de userspace

**Hito:** SI-2 · **Tipo:** Implementación de kernel (red) · **Estado:** hecha (2026-09-23).

**Dependencias:** ninguna. **La originan:** [T16](T16-servicio-guest.md),
[T18](T18-puertos-qemu.md), [T19](T19-qemu-e2e.md), que quedan bloqueadas sin ella.

## Problema reproducido

`soso-llm serve` escucha y lo dice, pero **ninguna conexión desde fuera del
guest llega a su `accept`**. El reenvío de QEMU está bien y la pila de red del
guest está sana; lo que no existe es el camino.

Reproducción mínima (2026-09-23, base `95de3c162` + arnés T19 corregido):

```sh
cargo xtask test-llm-api --model-dir target/qwen2.5-coder-3b-model \
  --profile target/self-improvement/tasks/T19/model-lock.json
# guest: `log` dice
#   pid=9 /bin/soso-llm: serve: listo — ask=7420 http=7422 modelo=… backend=cpu
# guest: `ps` dice
#   10  8  socket  /bin/soso-llm
# host:
curl -s -o /dev/null -w "http=%{http_code} t=%{time_total}\n" \
  -H "Authorization: Bearer test-llm-api-token" http://127.0.0.1:17299/health
# http=000 t=0.000335   ← RST inmediato, cero bytes
```

Controles que descartan las explicaciones fáciles:

- El puerto de eco (`7799`→guest `7`) contesta en la misma máquina y el mismo
  `netdev`: la pila y el reenvío funcionan. Ese servidor es del **kernel**.
- SSH (`2299`→`22`) funciona; también es del kernel (`kernel/src/net/ssh.rs`).
- El reset es instantáneo (`t≈0,0008 s`), así que no es un timeout de lectura
  ni la carga del modelo: el modelo ya está listo a los 4,3 s de arranque.
- El token del fichero es el correcto (`cat /tmp/soso-llm-api.token`), así que
  no es un 401 perdido.

**Ningún servidor HTTP de userspace ha sido alcanzado nunca desde fuera**: el
eco y SSH, los dos casos que sí funcionan, no pasan por `tcp_listen`.

## Causa

[`kernel/src/net/mod.rs`](../../kernel/src/net/mod.rs):

- `tcp_listen` marca **siempre** `entry.loop_listener = true` y sólo intenta el
  `listen` de smoltcp «por si acaso» (`let _ = tcp_user::listen_start(…)`).
- `tcp_listener_ready` devuelve, para un `loop_listener`,
  `loopback::has_pending(slot)`: sólo mira pares de loopback.
- `tcp_accept` y `tcp_accept_wake` desvían todo `loop_listener` a
  `tcp_accept_loopback`.

Existe ya el camino externo —en `tcp_accept`, `is_established(handle)` promueve
el propio slot a `Connected`—, pero es inalcanzable porque la bandera se pone
incondicionalmente. El diseño de `accept` de userspace era, hasta ahora,
loopback puro: `ask` conecta a `127.0.0.1:7420` **dentro** del guest.

## Contexto mínimo

- `kernel/src/net/mod.rs`: `tcp_listen`, `tcp_accept`, `tcp_accept_wake`,
  `tcp_listener_ready`, `tcp_slot_loop_listener`, `tcp_connect_loopback`.
- `kernel/src/net/tcp_user.rs`: `UserTcp`, `alloc`, `free`, `listen_start`,
  `poll_entry`, `is_established`.
- `kernel/src/net/loopback.rs`: `has_pending`, `alloc_pair`, `close_side`.
- `user/soso-llm/src/net.rs` (`TcpFd`) y `user/soso-llm/src/serve.rs` (bucle de
  `accept`): el consumidor, que no debería cambiar.

## Contrato técnico

Un listener de userspace atiende **las dos** procedencias sin que la aplicación
elija:

- `tcp_listener_ready(slot)` = hay par de loopback pendiente **o** el socket
  smoltcp del listener está `Established`.
- `tcp_accept(slot)` prefiere el loopback si lo hay; si no, toma la conexión
  externa. Debe devolver un **fd nuevo** y dejar el listener escuchando, o
  documentar explícitamente por qué reutiliza el slot (hoy lo reutiliza, lo que
  deja el puerto sin escucha tras la primera conexión: con HTTP no sirve).
- `tcp_accept_wake` mantiene la distinción `None` (mismo fd) / `Some(slot)`.
- Errores: `EAGAIN` sin conexión pendiente; `EMFILE` sin slots; nunca un RST
  silencioso.
- Sin adaptador de red (`NicDev::Ninguno`) el listener sigue sirviendo
  loopback, como hoy: `askd` no puede depender de que haya NIC.

## Alcance

`kernel/src/net/mod.rs` y `kernel/src/net/tcp_user.rs`, más la prueba. No tocar
`user/soso-llm`. Si aparece que hace falta rearmar el listen de smoltcp tras
cada conexión, entra aquí; si aparece un fallo de `close` que manda RST con
datos sin enviar, se registra **aparte**.

## Pasos

1. Sonda mínima: un listener de userspace en un puerto reenviado y un cliente
   del host; hoy da RST inmediato. Dejarla como prueba, no como script suelto.
2. Separar «escucha loopback» de «escucha externa» en `UserTcp`: hoy son la
   misma bandera.
3. Hacer que `tcp_listener_ready` y `tcp_accept` consideren las dos, con el
   loopback con prioridad.
4. Devolver un fd nuevo y rearmar el `listen` de smoltcp, de forma que una
   segunda petición HTTP encuentre el puerto escuchando.
5. Reejecutar `cargo xtask test-llm-api --model-dir … --profile …` y adjuntar
   `/health` con código 200.

## Comprobación

- Prueba de kernel/integración del paso 1 (host).
- `cargo xtask test-llm-api …` llega al menos a `health_ready`.
- `cargo xtask test` sigue verde: `ask` y `askd` usan el mismo camino de
  loopback y no pueden regresar.

## Lo que resultó ser (tres defectos, no uno)

Al instrumentar el listener apareció que el diagnóstico inicial era correcto
pero incompleto. Hicieron falta tres arreglos, todos en el kernel:

1. **El `accept` sólo miraba el loopback.** `tcp_listen` marcaba
   `loop_listener = true` y `tcp_accept`/`tcp_listener_ready`/`tcp_accept_wake`
   desviaban todo a los pares de loopback. Ahora el loopback sigue teniendo
   prioridad —es el camino de `ask`— y, si no lo hay, se entrega la conexión
   externa: `ceder_establecida` cede el socket establecido a un slot nuevo y
   deja al listener con uno recién creado, escuchando otra vez. Sin ese relevo
   la primera petición dejaba el puerto sin escucha y la segunda moría.
2. **«Hay conexión» no es sólo `Established`.** Un cliente que manda su
   petición y cierra —un sondeo con plazo corto, `curl --max-time`, cualquier
   cosa detrás de slirp— deja el socket en `CloseWait` antes de que el `accept`
   del proceso lo mire. Con la condición estrecha la conexión se perdía **y** el
   listener se quedaba clavado en `CloseWait`: la primera petición mataba el
   puerto para siempre. `tiene_conexion` acepta también `CloseWait`,
   `FinWait1/2`, `Closing` y `LastAck`; la petición ya está en el búfer.
   Además `poll_entry` vuelve a escuchar si el socket del listener acaba muerto
   sin que nadie lo aceptara.
3. **`free()` tiraba la respuesta ya escrita.** Cerraba el socket y lo quitaba
   del `SocketSet` en la misma llamada, pero `close()` sólo pide el FIN: es
   smoltcp quien vacía el búfer en los `poll` siguientes. El servidor
   contestaba y el cliente veía «empty reply from server». Ahora el socket pasa
   a una cola de drenaje (`purgar_cerrados`) y se quita cuando llega a
   `Closed`/`TimeWait` o a los 3 s, para que un par mudo no retenga los 128 KiB
   del socket indefinidamente.

## Evidencia

```
curl -H "Authorization: Bearer test-llm-api-token" http://127.0.0.1:17299/health
HTTP/1.1 200 OK
{"status":"ready","model":"qwen2.5-coder-3b-merges","backend":"cpu"}
```

`cargo xtask test-llm-api --model-dir target/qwen2.5-coder-3b-model --profile
target/self-improvement/tasks/T19/model-lock.json` pasa ya
**health, models, auth_401, busy_429, health_en_generacion y servidor_apagado**.
Lo que falla ahora es de otra naturaleza —`chat_json` no termina dentro del
plazo del cliente y deja la sesión ocupada, así que las siguientes reciben
429— y es presupuesto de generación, no alcance de red: queda para T19/T14.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

Límite: no se ha medido el comportamiento con varias conexiones simultáneas al
mismo listener —`MAX_USER_TCP` son 8 slots— ni con un cliente que mantenga la
conexión viva (keep-alive); el servidor responde con `Connection: close`.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La prueba del paso 1 debe poder ejecutarse
también desde el runner nativo de [T49](T49-pruebas-guest.md).

Validación nativa: **pendiente**.
