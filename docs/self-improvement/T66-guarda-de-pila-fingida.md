# T66 — La guarda de pila de los hilos no existe, y el código finge que sí

**Hito:** SI-4 · **Tipo:** Corrección de userspace (y decisión de ABI) · **Estado:** completada (2026-09-24).

**Dependencias:** ninguna. **La origina:** [T33](T33-sondas-abi.md), pase 6.

## Problema medido

`libsoso::thread::spawn` reserva la pila del hilo y a continuación intenta
dejar la primera página sin permisos, como guarda:

```rust
// user/libsoso/src/thread.rs
let _ = sys::mprotect(base as u64, GUARD, 0);
```

Esa llamada **siempre falla**, y el `let _` se lo traga:

    probe: hilos/guarda-de-pila-se-puede-instalar FALLO
      esperado="prot=0 aceptado"  observado="prot=0 → argumento inválido (-22)"
    probe: hilos/pagina-sin-lectura FALLO
      esperado="sólo escritura aceptado"  observado="PROT_WRITE sin READ → argumento inválido (-22)"

`sys_mprotect` (`kernel/src/task/syscall.rs`) rechaza `prot == 0` y también
rechaza quitar `PROT_READ`. **No hay forma de marcar una página como
inaccesible**, ni siquiera queriendo.

Reproducción: `soso-agent-probe hilos`, casos
`hilos/guarda-de-pila-se-puede-instalar` y `hilos/pagina-sin-lectura`.

## Por qué importa

Sin guarda, desbordar la pila de un hilo **no falla**: escribe en lo que haya
debajo. Y lo que hay debajo es memoria del proceso — otra pila, el montón—, así
que el síntoma aparece lejos del sitio y mucho después, como corrupción
inexplicable. Es el fallo más caro de diagnosticar que existe, y el más fácil de
prevenir.

Para un intérprete la recursión profunda es rutina, no un caso raro: es
exactamente lo que [T32](T32-opencode-inventario.md) pone encima de esto si
algún día corre JavaScriptCore.

## Son dos cosas, y conviene no mezclarlas

1. **El código finge.** Aunque el ABI no pudiera, `thread::spawn` no debería
   decir que instala una guarda y seguir como si la hubiera instalado. Esto se
   arregla hoy y es pequeño.
2. **El ABI no lo permite.** Que no se pueda expresar «sin acceso» es una
   decisión de diseño con la misma forma que la de
   [T33 pase 2](T33-sondas-abi.md) sobre W+X: soso no tiene protecciones de
   página más allá de la escritura. Cambiarlo es más grande y toca a T34.

## Alcance

`user/libsoso/src/thread.rs` para lo primero. Lo segundo se decide en
`kernel/src/task/syscall.rs` + `crates/soso-abi/src/lib.rs` y **no** entra en
esta ficha: aquí se registra como límite conocido.

## Pasos

1. Comprobar el resultado de `mprotect` en `spawn` en vez de descartarlo. Si
   falla, **no** callar: hoy la única salida honesta es dejar constancia de que
   la pila del hilo no tiene guarda.
2. Documentar en `DEFAULT_STACK` que la pila no está protegida y qué significa
   eso para quien elija su tamaño.
3. Decidir si `spawn` debe fallar cuando no puede instalar la guarda. Probablemente
   no —dejaría a soso sin hilos—, y entonces la decisión hay que escribirla,
   no dejarla implícita en un `let _`.
4. Anotar en el backlog de [T34](T34-tickets-port.md) la ampliación del ABI:
   un `PROT_NONE` de verdad.

## Resultado

`thread::spawn` **mira** el resultado de `mprotect` en vez de tragárselo, y
publica lo que salió en `libsoso::thread::hay_guarda_de_pila()`. Hoy es
`false` en soso, y cuando [N-002](native/N-002.md) traiga `PROT_NONE` pasará a
ser `true` sin que cambie nada más.

**No se falla el `spawn`.** Se pensó y se descartó: dejaría a soso sin hilos, y
la pila sin guarda es lo que hay hoy. Lo que no se hace es callarlo — la
decisión está escrita en el código, que era el sitio donde faltaba.

Y `DEFAULT_STACK` lo dice en su documentación, porque es donde alguien va a
mirar al elegir un tamaño: **la pila de un hilo no tiene guarda**, así que ese
número es lo único que separa una recursión profunda de corromper la memoria de
al lado; quien elija uno menor está eligiendo también a qué profundidad se
corrompe el montón en silencio.

## Comprobación

    cargo xtask test sys --only="hilos, guarda"    OK
    cargo xtask test                                TODO OK
    cargo xtask check                               TODO OK

Caso nuevo en la sonda de T33, `hilos/libsoso-no-promete-guarda`: comprueba que
lo que dice `hay_guarda_de_pila()` **coincide** con lo que contesta `mprotect`.
Sin él, la biblioteca podría volver a prometer lo que el kernel no da, que es
exactamente el defecto que esta ficha arregla.

Los otros dos casos siguen en negativo, y eso es correcto: la capacidad no
existe hasta N-002. Lo que cambia aquí es que el código deje de fingir.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

**Límite, y es el importante:** esto **no** añade una guarda de pila. Sigue sin
haberla, y desbordar la pila de un hilo sigue corrompiendo lo que haya debajo.
Lo que se arregla es que el sistema dejó de decir lo contrario. La capacidad la
trae [N-002](native/N-002.md).

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La sonda corre dentro de soso. Validación
nativa: **pendiente**.
