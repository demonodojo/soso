# Cargo sin hijos, bloqueado en futex — 7 de octubre de 2026

## Conclusión y alcance

La frase de la sesión de Claude Code («ya van tres veces») cuenta tres
incidentes de `cargo` sin compiladores hijos, no tres fallos distintos
demostrados del futex. La atribución al kernel era una hipótesis.

La evidencia del intento **167** apunta al cierre del auxiliar de **jobserver
0.1.34**, cuya implementación `wasm.rs` usa soso. Ese cierre puede bloquearse
esperando un permiso que nunca llegará. El fallo se ha reproducido en Linux
con ese backend, sin ejecutar el kernel de soso. Es una explicación concreta
compatible con el incidente; no se ha repetido aún la carga completa en el
guest ni se han identificado individualmente las tres recurrencias.

Existe además un fallo independiente en `kernel/src/task/futex.rs`, reproducido
en host: una entrada expirada puede consumir un despertar de otra dirección.
Corregir únicamente ese fallo no acredita resolver el cierre de cargo.

## Evidencia del cuelgue de cargo

`target/self-improvement/tasks/T42/guest-boot-cargo-167.txt:226` conserva los
RIP y una ventana de pila del proceso principal. Simbolizados con el ELF
local `rootfs/bin/cargo`:

| Dato | Símbolo |
|---|---|
| PID 4, RIP `0x1b751e1` | `std::sys::thread::soso::Thread::join` |
| Pila, `0x1a426d6` | `jobserver::imp::Helper::join` |
| Pila, `0x1a43a77` | `jobserver::HelperThread::drop` |
| Pila, `0x10253b3` | `std::thread::scoped::scope`, cierre de `JobQueue::execute` |
| PID 17, RIP `0x1b7ffe4` | `std::thread::functions::park` |
| PID 18, RIP `0x1b74f1a` | `Condvar::wait` |

La ventana de pila contiene direcciones de retorno compatibles con esa cadena;
no es un backtrace obtenido por unwinding. No hay pila del PID 18 para distinguir
directamente entre las dos condvars de jobserver. Tampoco hay copia del contador
de permisos ni de `WAITERS` del instante del cuelgue.

El ELF analizado tiene SHA-256
`f78e1f58a0fe9d33c12313067ba1315a771a24247de30c0d456921e55be2b774`.
El `.d` de `jobserver-bec0e494e139f579` en el build de `stage2-tools` de T40
confirma las fuentes `jobserver-0.1.34/src/{lib,wasm}.rs`. El desensamblado
de `Helper::join` confirma que llama a `Thread::join` sin cancelar `acquire`.

Comando para repetir la simbolización:

```sh
addr2line -f -C -e rootfs/bin/cargo \
  0x1b751e1 0x1a426d6 0x1a43a77 0x10253b3 0x1b7ffe4 0x1b74f1a
```

## Secuencia de bloqueo en jobserver

1. Cargo crea `Client::new(jobs)` y consume un permiso con `acquire_raw()`
   para su trabajo implícito (`src/compiler/build_runner/mod.rs:109` del
   cargo en `/home/jmdiez/.cache/soso-rust-vendor/src/tools/cargo`). Con un
   trabajo permitido, quedan cero permisos disponibles.
2. `JobQueue::spawn_work_if_possible` solicita permisos anticipadamente si
   hay más de un trabajo activo o pendiente (`job_queue/mod.rs:578`). El
   auxiliar puede quedarse dentro de `Client::acquire`, esperando la condvar
   del contador de permisos. Esto no exige que siga vivo un compilador hijo.
3. Al terminar o fallar la compilación, `HelperThread::drop` escribe
   `producer_done` y notifica la condvar de **solicitudes** (`lib.rs:560`).
4. El auxiliar espera en la condvar de **permisos**, que es otra. La señal
   de cierre no cancela `acquire`. `Helper::join` espera al auxiliar para
   siempre (`wasm.rs:103`). El propio código contiene un TODO que reconoce
   esta limitación.

El port [C-104](self-improvement/native/C-104.md) y
`config/rust-soso/patch-jobserver-soso.py` corrigieron `configure` y
`string_arg`, pero no ese cierre. Tener una syscall futex correcta no hace
que se notifique automáticamente una condvar distinta.

## Reproducción aislada

Arnés: `target/futex-investigation/jobserver-host/`. Copia las fuentes locales
de jobserver 0.1.34 y selecciona `wasm.rs` también en Linux; sustituye la suite
original por dos pruebas de diagnóstico. No modifica el registro de Cargo,
el port ni el kernel.

La prueba principal crea un permiso, lo reserva como cargo, solicita otro y
espera a que el auxiliar consuma la solicitud. Inicia el cierre y comprueba
`producer_done`; el cierre no acaba en 200 ms. Liberar un permiso real permite
que termine en menos de 2 s. El control sin solicitud pendiente termina sin
liberar permisos. No se depende de un `sleep` para ordenar la solicitud y el
cierre: se observa el estado bajo su mutex.

```sh
cargo test --offline \
  --manifest-path target/futex-investigation/jobserver-host/Cargo.toml \
  --lib -- --nocapture
```

**Resultado:** exit 0, 2 pruebas correctas. La primera acredita la presencia
del bloqueo y su desbloqueo al devolver un permiso; no afirma una corrección.
Salida: `target/futex-investigation/jobserver-result.txt`.

## Hallazgos independientes en el kernel

- **Despertar consumido por una entrada expirada:** un hilo espera A con
  timeout, expira y espera B. Otro hilo espera A. `wake(A, 1)` consume la
  entrada antigua y despierta al de B, dejando al de A dormido. En
  `futex.rs:81` se comprueba `WaitingFutex`, pero no `(pml4, uaddr)`.
  Reproducido con las funciones reales del kernel incluidas por `#[path]`
  en un arnés host. Se simulan el reloj, CPU, CLI, scheduler y la transición
  de expiración. Comprobar ambas claves en una copia corrige la prueba.
- **Inversión de candados en SMP:** `wait_or_resume`/`wake` toman
  `WAITERS → PROCS`; `exit_current` tiene `PROCS` cuando llama a
  `forget_pid`, que toma `WAITERS` (`task/mod.rs:1272,1290`). Hallazgo por
  inspección, sin provocar el deadlock en QEMU. No explica por sí solo la
  ejecución con `-smp 1`.

Detalles y resultados: `target/futex-investigation/investigacion.txt`,
`result.txt`, `fixed-result.txt`. Las copias experimentales no se han integrado.

## Siguiente paso para resolver el caso recurrente

Plan de implementación y validación:
[corregir el bloqueo de cargo al cerrar jobserver](PLAN-CORRECCION-FUTEX-CARGO.md).

Implementar cancelación de la adquisición pendiente en el backend de jobserver
usado por soso, coordinada con el mutex de la condvar de permisos. El cierre
debe despertar al auxiliar y permitirle salir sin fabricar permisos ni depender
de que termine otro compilador. Cubrir los cierres con cero permisos, con
solicitudes pendientes y sin ellas; después reconstruir cargo y repetir la
carga guest. Un reintento que termine bien no demuestra que desaparezca el
fallo intermitente.

La validación guest y la atribución de las otras dos recurrencias quedan
pendientes. Durante esta investigación no se reinició el QEMU ajeno ni se
alteró su imagen; una consulta `ps` posterior ya no encontró cargo vivo.
