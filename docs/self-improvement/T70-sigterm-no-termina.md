# T70 — SIGTERM no termina un proceso dormido

**Origen:** destapado el 2026-09-25 al cerrar un agujero en las sondas.
**Aplica en:** `kernel/src/task/mod.rs` (`signal_one`) · **Estado:** pendiente.
**Decisión, no sólo corrección.**

## Lo que pasa

    $ soso-agent-probe senales
    senales/sigterm-sobre-un-proceso-dormido  código 0 (se esperaría 143 si matara)
    senales/sigkill-se-distingue-de-sigterm   ok (137)

Un hijo que duerme 60 s recibe `SIGTERM` y **sale con 0**: no lo mata.
`SIGKILL` sí (137).

## Por qué

`signal_one` trata `SIGINT`/`SIGTERM` sobre un proceso **bloqueado** —
`WaitingTty`, `WaitingSocket`, `WaitingPipe` o `Sleeping` — como una
**interrupción**: le pone `EINTR` en `rax` y lo marca `Runnable`. `SIGKILL`, en
cambio, va directo a `deliver_death`.

Esa lógica es de septiembre y es razonable **en un sistema con manejadores de
señal**: interrumpir la llamada para que el proceso decida qué hacer. Pero soso
**no tiene manejadores** (lo midió [T33](T33-sondas-abi.md), sonda `senales`:
«un proceso se puede matar; no puede *reaccionar*»). Sin manejador, interrumpir
significa que el `sleep` devuelve y el programa sigue como si nada — y termina
normalmente.

Es decir: **SIGTERM no termina**, y quien lo envía no tiene forma de saberlo,
porque `kill` devuelve éxito.

## Por qué no se había visto

El caso de la sonda pasaba… por suerte de reloj. Esperaba 300 ms tras lanzar al
hijo y luego mataba: en esos 300 ms el hijo **todavía no había llegado a
dormirse**, así que no estaba «bloqueado» y caía en la rama que sí mata.

Las mejoras de [N-012](native/N-012.md) —`stat` 20× más rápido— hicieron que el
hijo arrancara a tiempo de dormirse, y el caso empezó a fallar. La medida no
cambió: cambió **qué rama del kernel se ejecutaba**.

Dicho de otro modo: el caso nunca probó «SIGTERM mata». Probó «SIGTERM mata a un
proceso que aún no se ha dormido».

## Por qué importa

La sonda lo dice en su propio comentario: este caso es **el timeout de
herramienta**. Un agente lanza `bash`, `git` o una compilación con plazo, y
cuando vence tiene que cortarla. Hoy, si la herramienta está dormida o
esperando E/S —que es lo normal en algo que se ha colgado—, `SIGTERM` la
despierta y la deja seguir, y el padre ve un código de salida **normal**.

No puede distinguir «la corté» de «terminó sola», que es exactamente la clase
de confusión que [N-009](native/N-009.md) y [N-010](native/N-010.md) quitaron
del buscador y de la shell.

## Las opciones

1. **Que SIGTERM mate también a los bloqueados**, como SIGKILL. Es lo que
   espera quien lo usa hoy, porque no hay nada que pueda reaccionar. La rama de
   `EINTR` quedaría para `SIGINT`, o se iría también.
2. **Conservar `EINTR` y declararlo**: «SIGTERM interrumpe, no termina; para
   terminar, SIGKILL». Barato y honesto, pero deja a quien escriba un timeout
   con una trampa que recordar.
3. **Manejadores de señal de verdad**, que es lo que haría útil el `EINTR`. Es
   una ficha grande y ninguna del plan la pide todavía.

La 1 y la 2 son media hora; la 3 es otra cosa. **No lo decido yo**: cambiar qué
hace una señal afecta a `sosh`, al arnés de pruebas y a cualquier guion que ya
dependa de la conducta actual.

## Mientras tanto

La sonda **mide y no juzga**: informa del código observado en vez de dar por
buena la conducta o de fingir que se esperaba. Cuando esta ficha se cierre, ese
caso vuelve a ser un veredicto.

## Reproducción

    cargo xtask test sys --only="señales"
