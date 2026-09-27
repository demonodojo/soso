# T73 — El futex de soso no tiene plazo, y `Condvar::wait_timeout` lo necesita

**Origen:** destapado el 2026-09-25 escribiendo la PAL de [T39](T39-bootstrap-libstd.md).
**Aplica en:** `kernel/src/task/syscall.rs` (`sys_futex`), `kernel/src/task/futex.rs`,
`crates/soso-abi`, `crates/soso-rt` · **Estado:** **hecha** (2026-09-26), acreditada en QEMU.

## Lo que bloquea

`std` no compila sin primitivas de sincronización, y el compilador **se niega**
a aceptar las falsas:

    error: Using no_threads implementation on a target with threads
     --> library/std/src/sys/sync/mutex/no_threads.rs:4:1

Son 4 de los 11 errores que le quedan a `std` (`condvar`, `mutex`, `once`,
`rwlock`). El `cfg_select` manda a soso a `no_threads` por descarte, y
`no_threads.rs` lleva un `#[cfg(target_has_threads)] compile_error!`. soso
**sí** tiene hilos, así que esa rama está bien cerrada.

La rama correcta es la de futex: soso tiene `SYS_FUTEX` con `FUTEX_WAIT` y
`FUTEX_WAKE`, que es justo lo que `sys/sync/*/futex.rs` usa.

## Por qué no se puede enchufar hoy

La firma que `std` espera lleva plazo:

```rust
pub fn futex_wait(futex: &Atomic<u32>, expected: u32, timeout: Option<Duration>) -> bool;
```

La de soso no:

```rust
// kernel/src/task/syscall.rs
abi::FUTEX_WAIT => {
    // No vuelve: wait_or_resume bloquea o replanifica.
    super::futex::wait_or_resume(pml4, uaddr, val as u32, ctx_from_frame(f));
}
```

El cuarto argumento (`nwake`) sólo se usa en `FUTEX_WAKE`. En `FUTEX_WAIT` se
ignora, y la espera es **indefinida**.

Implementar `futex_wait` ignorando el `timeout` haría que
`Condvar::wait_timeout` —y con él cualquier `recv_timeout`, cualquier plazo de
un pool de hilos— **durmiera para siempre**. Es exactamente el defecto de
[T55](T55-accept-sin-plazo.md), donde `tcp_accept(fd, 0)` dormía al servidor
sin vuelta atrás, un nivel más abajo y afectando a todo lo que sincronice.

Devolver `false` («expiró») sin esperar tampoco vale: convierte cada espera con
plazo en una espera activa.

## Lo que hay que hacer

1. **Plazo en `FUTEX_WAIT`.** El cuarto argumento ya existe y hoy se ignora en
   esa rama, así que puede llevar un plazo en milisegundos con **0 = sin
   plazo**: los que llaman hoy pasan 0 y siguen significando lo mismo. No hace
   falta tocar `ABI_VERSION`, que se compara por igualdad exacta y dejaría
   fuera a todas las máquinas instaladas.
2. **Distinguir «me despertaron» de «expiró»** en el valor de retorno, que es
   lo que `futex_wait` devuelve como `bool`. Sin esa distinción el llamante no
   puede saber si comprobar la condición o rendirse — la misma confusión que
   [N-009](native/N-009.md) y [N-010](native/N-010.md) quitaron del buscador y
   de la shell.
3. **`syscall4` en `soso-rt`**, que hoy sólo tiene `syscall0/1/3`. La llamada
   futex necesita cuatro argumentos.
4. Entonces sí: `sys/sync/futex/soso.rs` y soso en los cinco `cfg_select`
   (`futex/mod.rs`, `mutex`, `condvar`, `once`, `rwlock`).

## La forma ya está decidida por el resto del kernel

No hace falta inventarla, y conviene **no** hacerlo:

- **Relativo, no absoluto.** `read_timeout`, `ping`, `tcp_connect` y
  `tcp_accept` pasan todos `timeout_ms` **relativo** con **0 = bloqueante**, y
  el kernel calcula el instante límite. Encaja además con el `Option<Duration>`
  de `std`: `None` → 0.
- **El estado ya tiene precedente.** `State::WaitingPipe` lleva
  `deadline_ms: u64` con el comentario «`0` = sin plazo (T61)», y el bucle del
  planificador lo vence con
  `deadline_ms != 0 && crate::arch::pit::uptime_ms() >= deadline_ms`
  (`kernel/src/task/mod.rs`). `WaitingFutex { pml4, uaddr }` sólo tiene que
  crecer un campo igual y entrar en ese mismo bucle.
- **Al vencer hay que sacarlo de `WAITERS`**, o quedará un pid fantasma en la
  lista de `(pml4, uaddr)` que un `wake` posterior intentará despertar.

Y **cuidado con el reloj**: el PIT pierde tiempo durante el sondeo de disco, así
que un plazo medido con `uptime_ms` puede alargarse bajo E/S. Para esta ficha
vale —es la misma base que usan los demás plazos del kernel— pero conviene que
la prueba no afirme precisión, sólo que **expira** y que se distingue de un
despertar.

## Alcance

El 1 y el 2 son del **kernel**, que [T39](T39-bootstrap-libstd.md) no permite
cambiar; por eso esto es una ficha y no un apartado suyo. El 3 y el 4 sí son de
T39 y esperan a los otros dos.

## Hecho

| Pieza | Dónde |
|---|---|
| `State::WaitingFutex { …, deadline_ms }` | `kernel/src/task/mod.rs`, con la forma de `WaitingPipe` (T61) |
| vencimiento con `-ETIMEDOUT` | bucle del planificador, junto a los demás temporizadores |
| plazo relativo en el 4º argumento | `sys_futex`; los llamantes existentes pasaban 0 y siguen significando lo mismo |
| `futex_wait_timeout` | `user/libsoso/src/sys.rs` |
| `syscall4` | `crates/soso-rt` |
| `sys/sync/futex/soso.rs` + 5 `cfg_select` | `config/rust-soso/tree/` |

**El candado que no se toca.** Al vencer **no** se limpia `WAITERS`. El orden
del futex es WAITERS → PROCS, y el planificador ya tiene PROCS: pedirlo al
revés es como se produjo el abrazo mortal entre el candado de red y el de
procesos. No hace falta, y está razonado en el código: `wake` saca el pid de la
lista *antes* de mirar su estado y sólo cuenta como despertado al que seguía en
`WaitingFutex`, así que una entrada rancia **no se come un wakeup**; y
`forget_pid` la limpia al morir el proceso.

**El plazo se ancla dentro del lock**, no en `sys_futex`: entre una cosa y otra
hay un candado que puede tardar, y anclarlo antes lo acortaría en silencio.

**La conversión de unidades casi reintroduce T55.** `std` da
`Option<Duration>`; `d.as_millis()` de 100 µs da **0**, que en soso significa
**sin plazo**. Un `wait_timeout(100 µs)` se habría vuelto una espera eterna.
`plazo_ms` sube a 1 ms cualquier plazo menor y satura los enormes.

## Acreditado

Sonda `hilos` en QEMU, tres casos:

    probe: futex/la-espera-con-plazo-expira ok (-110)
    probe: futex/expirar-se-distingue-de-despertar ok (0)
    probe: futex/cuanto-tardo-el-plazo-de-300ms medido: 300 ms

El segundo es **control**: sin él, «devuelve ETIMEDOUT» lo cumpliría también
una implementación que devolviera ETIMEDOUT siempre. El tercero es
**medición, no veredicto**, porque el PIT pierde tiempo durante el sondeo de
disco y esa precisión no es algo que la sonda deba juzgar.

Primer intento: **falló**, y por culpa de la expectativa, no del sistema —
escribí `"ETIMEDOUT"` y `errno_str` devuelve `"timeout"`. Ahora se compara el
**código numérico**, que no puede desincronizarse con la tabla de mensajes.

## Un defecto encontrado de camino

Al escribir `syscall4` para `soso-rt` se comparó con la versión probada de
`libsoso`, y la suya lleva `clobber_abi("C")` con este motivo: *el kernel usa
SSE (cripto, `memcpy`), así que los registros vectoriales no sobreviven a una
syscall*. Las tres envolturas de `soso-rt` **no lo declaraban**, y encima
prometían `options(nostack, preserves_flags)`.

El target `x86_64-unknown-soso` compila con `+avx,+avx2,+fma`. No había
explotado **porque la libstd todavía no compila**; en cuanto compilase, lo
habría heredado entera. Corregidas las cuatro con la forma de libsoso.

Es la misma familia que los «registros YMM» de [T62](T62-argv-en-los-programas.md).

## Efecto

`std` para `x86_64-unknown-soso` pasa de **6 errores a 2**, y los dos son de
[T72](T72-heap-de-libstd.md). La única cosa entre el plan y una libstd que
compila es esa decisión.

## Reproducción

    cargo xtask test sys --only="hilos"     # los tres casos de futex
    cargo xtask rust-build-std              # quedan 2 errores, los de T72
