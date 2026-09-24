# T65 — `O_EXCL` no excluye mientras el primer descriptor sigue abierto

**Hito:** SI-4 · **Tipo:** Corrección de kernel · **Estado:** completada (2026-09-24).

**Dependencias:** ninguna. **La origina:** [T33](T33-sondas-abi.md), pase 3.
**Afecta a:** [T46](T46-archivos-durables.md), cuyo `crear_exclusivo` es la
única exclusión mutua que tiene el coordinador.

## Problema medido

Dos `open(O_CREAT|O_EXCL)` sobre la misma ruta, **sin cerrar el primero**,
tienen éxito los dos:

    probe: compartir/o-excl-excluye FALLO esperado="gana uno" observado="p1=4 p2=5"

Reproducción (`soso-agent-probe compartir`, caso `compartir/o-excl-excluye`):

```rust
let p1 = sys::open(ruta, O_WRONLY | O_CREAT | O_EXCL);   // 4
let p2 = sys::open(ruta, O_WRONLY | O_CREAT | O_EXCL);   // 5  ← debería ser EEXIST
```

## Por qué pasa

`open(O_CREAT)` **no toca el disco**: la entrada de directorio se materializa
al cerrar o al hacer `fsync` (`kernel/src/task/syscall.rs`, el comentario del
volcado de `StreamWrite` lo dice con todas las letras). Mientras tanto el
`lookup` que comprueba la existencia no encuentra nada, así que el segundo
`O_EXCL` no ve al primero.

El comentario del kernel ya contempla un caso vecino —crear y cerrar sin
escribir tiene que dejar el fichero, «sin esto, un segundo `open(O_CREAT|O_EXCL)`
sobre él triunfaba en vez de dar `EEXIST` (lo cazaba `soso-test-sosofs`)»— y lo
arregla **para cuando el primero ya cerró**. La ventana con los dos abiertos a
la vez se quedó fuera, y es la que importa para excluir.

## Por qué importa más de lo que parece

soso **no tiene `flock` ni `fcntl`**, así que `O_EXCL` es la única exclusión
mutua disponible. [T46](T46-archivos-durables.md) construyó sobre ella el
contrato durable: `crear_exclusivo` se apoya en que «si otro escritor llegó
antes con esta misma generación, aquí falla». Esa garantía es hoy **más débil
de lo que su comentario promete**: sólo se cumple si el otro escritor ya cerró.

En la práctica T46 abre, escribe, sincroniza y cierra dentro de la misma
llamada, así que la ventana es estrecha —y por eso sus pruebas pasan—, pero
estrecha no es cerrada: dos procesos que publiquen a la vez pueden ganar los
dos. No es una carrera teórica, es el caso para el que se escribió la función.

## Alcance

`kernel/src/task/syscall.rs`: el camino de `open` con `O_CREAT|O_EXCL`. Lo que
falta es **reservar el nombre al abrir**, no al cerrar, para que el segundo
`lookup` lo encuentre. Materializar el fichero entero al abrir cambiaría la
semántica de streaming; reservar una entrada vacía, no.

## Pasos

1. Reproducir con la sonda de T33 (ya está: `compartir/o-excl-excluye`).
2. En `open`, con `O_CREAT|O_EXCL` y el `lookup` vacío, **crear la entrada
   inmediatamente** (fichero vacío) y dejar el `StreamWrite` apuntando a ese
   inodo. Un `O_EXCL` posterior encontrará la entrada y dará `EEXIST`.
3. Comprobar que no se rompe lo que el comentario del kernel ya protegía:
   crear y cerrar sin escribir sigue dejando un fichero vacío, y
   `soso-test-sosofs` sigue pasando.
4. Revisar si `crear_exclusivo` de T46 puede entonces prometer lo que dice, y
   ajustar su comentario si sigue habiendo límites.

## Resultado

`O_CREAT|O_EXCL` **reserva el nombre al abrir**: si el `lookup` no encuentra
nada, se crea el fichero vacío ahí mismo y el descriptor apunta a ese inodo. El
segundo `O_EXCL` llega, encuentra la entrada y recibe `EEXIST`.

    compartir/o-excl-excluye   FALLO → ok   (gana uno)

Escribir sigue costando lo mismo: el fichero reservado está vacío, así que el
volcado le **añade** el contenido —un `append_file`, una transacción— en vez de
crearlo. No se toca el camino de `WriteBuf` que N-001 introdujo, porque un
fichero recién reservado no tiene cola que conservar.

### Lo que cambia de conducta, y conviene saberlo

Si el proceso que reservó el nombre **muere antes de escribir**, ahora queda un
fichero **vacío**; antes no quedaba nada. Es el precio de que la exclusión
signifique algo, y es lo que hace POSIX.

Para el consumidor que motivó la ficha no es un problema: el contrato durable de
[T46](T46-archivos-durables.md) publica por generaciones y ya sabe convivir con
una generación a medias — [T63](T63-generacion-rota-encalla.md) se ocupó
precisamente de que un resto así no encalle la referencia. Las dos fichas
encajan: T63 enseñó a soso a seguir después de un corte, y T65 hace que la
exclusión sea real.

## Comprobación

    cargo xtask test sys --only="mismo fichero"   o-excl-excluye ok
    cargo xtask test                               TODO OK (47 pasos)
    cargo xtask check                              TODO OK

La suite entera importa aquí más que en otras fichas: toca `open`, y `O_EXCL` lo
usan `soso-test-sosofs`, el contrato durable y el instalador.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

**Límite.** Esto cierra la ventana entre dos `open`, que era la que importaba.
Lo que **no** da es un candado sobre un fichero ya existente: para eso hace
falta `flock` o equivalente, que no existe y está en el backlog como **N-004**
([`native/backlog.json`](native/backlog.json)) — detrás de N-001, porque
bloquear un fichero que los descriptores no comparten no significa nada.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Es kernel y la sonda corre dentro de soso.
Validación nativa: **verificada** (2026-09-24).
