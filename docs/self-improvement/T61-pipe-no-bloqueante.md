# T61 — Lectura de tuberías con plazo, sin bloquear

**Hito:** SI-4 · **Tipo:** Ampliación de ABI (kernel) · **Estado:** pendiente.

**Dependencias:** ninguna. **La origina:** [T47](T47-procesos-nativos.md).
**Bloquea:** el drenaje simultáneo de stdout y stderr en T47.

## Problema

Para recoger la salida de un hijo sin bloquearse hay que poder leer sus dos
tuberías —stdout y stderr— alternando. Si sólo se puede leer bloqueando, el
lector se queda esperando en una mientras el hijo llena la otra y se para:
**interbloqueo**, y el síntoma es un proceso que «tarda» sin consumir CPU.

Hoy no se puede:

- `sys::read` sobre `Fd::PipeRead` usa `pipe::try_read`; si devuelve 0 y el
  extremo de escritura sigue abierto, bloquea con `State::WaitingPipe`
  (`kernel/src/task/syscall.rs`).
- `sys::read_timeout` sólo trata `Fd::Tcp`; para lo demás cae en `sys_read` y
  bloquea igual.

El kernel **ya tiene** la pieza (`pipe::try_read`); lo que falta es exponerla.

## Contrato técnico

Extender `SYS_READ_TIMEOUT` para que acepte tuberías, con la misma semántica
que ya tiene para TCP y que [T48](T48-reloj-red.md) fijó:

| Resultado | Significado |
|---|---|
| `n > 0` | se leyeron `n` bytes |
| `0` | **EOF**: el extremo de escritura se cerró y no queda nada |
| `-EAGAIN` | no hay datos **todavía**; el escritor sigue abierto |

EOF y espera son cosas distintas y no se pueden confundir: es la misma regla
que T48 impuso al transporte, y por el mismo motivo.

`timeout_ms = 0` conserva el significado que ya tiene en esta syscall —sin
plazo, bloquea—, así que quien quiera sondear pasa un valor pequeño. Cambiarlo
rompería a los llamantes actuales ([T55](T55-accept-sin-plazo.md) documenta lo
caro que sale confundir ese cero).

## Alcance

`kernel/src/task/syscall.rs` (`sys_read_timeout`) y, si hace falta, un ayudante
en `kernel/src/task/pipe.rs`. Sin syscall nueva y sin cambiar la estructura de
la llamada: sólo se amplía qué descriptores acepta.

## Pasos

1. En `sys_read_timeout`, tratar `Fd::PipeRead` antes del camino genérico:
   `try_read`; si 0 y el escritor está cerrado, devolver EOF; si 0 y sigue
   abierto, esperar como mucho el plazo y devolver `-EAGAIN`.
2. Prueba en `init test`: un hijo que escribe en dos tuberías con pausas; el
   padre alterna con plazo corto y recompone ambas salidas sin bloquearse.
3. Prueba del caso que hoy se cuelga: el hijo llena stderr mientras el padre
   lee stdout.

## Comprobación

`cargo xtask test` (el shard `sys` corre `init test`). La prueba debe fallar
sin el cambio: un interbloqueo que no se reproduce no acredita nada.

## Resultado

El interbloqueo **sí se da**, y se reprodujo antes de arreglarlo, que es lo que
esta ficha exigía. Quitando la rama nueva de `sys_read_timeout` y ejecutando
sólo ese paso:

    (reintento 1/2 de «soso-improve: dos tuberías sin bloquear»…:
     "$ soso-improve tuberias"          ← y nada más: el proceso no vuelve

El hijo escribe 12 KiB en stderr —tres veces el búfer— y una línea en stdout.
Con el arreglo, el paso pasa.

Implementado en `sys_read_timeout` (rama `Fd::PipeRead` con `pipe::try_read` y
los tres desenlaces separados) y en `State::WaitingPipe`, que gana `deadline_ms`
y contesta `EAGAIN` al vencer, igual que ya hacía el camino TCP.
`timeout_ms = 0` conserva su significado para no romper a los llamantes
actuales.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

**Límite.** La sonda escribe 12 KiB porque el búfer son 4 KiB; si `PIPE_CAP`
cambia, hay que revisar la cifra. Y separar de verdad stdout y stderr en el
adaptador de [T47](T47-procesos-nativos.md) es ahora posible, pero es trabajo de
esa ficha, no de ésta.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Es kernel y la prueba corre **dentro de soso**,
entre dos procesos del guest: tanto el interbloqueo como su ausencia se observan
allí. Validación nativa: **verificada**.
