# Plan: corregir el bloqueo de cargo al cerrar jobserver

**Fecha:** 2026-10-07. **Estado:** corrección y regresiones host implementadas
en [C-144](self-improvement/native/C-144.md); validación guest pendiente.
**Actualización 2026-10-08:** el arreglo se conserva en fuentes versionadas
de jobserver; se retira el script que modificaba la caché del registro.
**Origen:** [diagnóstico del caso recurrente](DIAGNOSTICO-FUTEX-CARGO-2026-10-07.md).
**Consumidor:** T42, compilación de las etapas del cargador; antecedente
[C-104](self-improvement/native/C-104.md), port de jobserver para T41.

## Resultado esperado

`cargo` debe terminar y devolver su código de salida cuando finalicen sus
trabajos, también ante un error de compilación y con un solo trabajo permitido.
El auxiliar de jobserver debe poder cancelar una adquisición pendiente y salir
sin esperar que otro proceso devuelva un permiso.

El bloqueo de ese auxiliar está reproducido en host y encaja con la pila del
intento 167. La atribución de las tres recurrencias sigue pendiente de validación
guest. El plan acredita por separado la corrección del mecanismo y la ejecución
de la carga que lo detectó.

## Alcance y entregas

| Entrega | Archivos o artefactos |
|---|---|
| Backend corregido versionado | `config/rust-soso/vendor/jobserver/src/wasm.rs` (0.1.34) |
| Integración de las fuentes | `config/rust-soso/apply-patches.sh`: copia local y resolución en ambos workspaces |
| Regresión durable | Nueva carpeta `tests/self-improvement/native/cargo/jobserver/`, con preparación host y sonda Rust ejecutable en soso |
| Cargo corregido | Binario reconstruido, hash, receta, log y copia de prueba en una imagen aislada |
| Evidencia | `target/self-improvement/tasks/T42/jobserver/<intento>/` y resumen versionado en el seguimiento de T42 |

Las rutas nuevas son entregables propuestos, no comandos ya disponibles.
Esta entrega no cambia la ABI ni necesita modificar libstd o el kernel.
Los defectos independientes del kernel tienen su seguimiento al final del plan.

## P1 — Convertir el diagnóstico en una regresión

- [ ] Registrar commit/diff de soso, revisión del árbol de Rust/Cargo, versión
  exacta de jobserver usada por el build y SHA-256 del cargo de referencia.
- [ ] Llevar el arnés de `target/futex-investigation/jobserver-host/` a la
  carpeta de pruebas versionada. Compilar el backend real `wasm.rs` de la
  dependencia, seleccionado expresamente en host, sin sustituirlo por un modelo.
- [ ] Cambiar la expectativa de la prueba: después de pedir el cierre, debe
  terminar **sin devolver artificialmente un permiso**. En la base falla;
  con la corrección debe pasar. El arnés inicial que confirma el bloqueo no
  constituye por sí solo una regresión que exija el comportamiento correcto.
- [ ] Ordenar la carrera con barreras/estado observable: un permiso reservado
  por cargo, solicitud consumida por el auxiliar, adquisición pendiente y cierre.
  Usar un plazo externo solo para detectar el bloqueo y terminar la prueba.
- [ ] Conservar el control sin solicitudes pendientes y la comprobación de que
  devolver un permiso desbloquea la implementación vieja. Ejecutar cada caso
  bloqueante en un proceso aislado para que no congele toda la suite.

**Salida:** fallo reproducible y acotado en la base; fixtures independientes de
los ficheros temporales de otra sesión. No hace falta compilar todo cargo aún.

## P2 — Hacer cancelable la adquisición del auxiliar

Diseño elegido: cancelación **por auxiliar**, coordinada con el mutex del
contador de permisos de `Client`. Mantener el comportamiento público de
`Client::acquire`, `try_acquire` y `release` para los consumidores normales.

- [ ] Dar a `Helper` y a su hilo un indicador compartido de cancelación y acceso
  al cliente que contiene la condvar de permisos. El indicador pertenece a ese
  auxiliar; cerrar uno no debe cancelar otros que compartan cliente.
- [ ] Añadir un camino interno de adquisición cancelable: con el mutex del
  contador tomado, comprobar cancelación antes de consumir un permiso y después
  de cada despertar. Si se cancela, salir sin obtener un `Acquired`.
- [ ] En `Helper::join`, tomar ese mismo mutex, publicar cancelación, soltarlo
  y notificar la condvar de permisos antes de esperar al hilo. Notificar a todos
  los posibles esperadores de esa condvar: `notify_one` podría despertar otro
  auxiliar y dejar dormido al que se está cerrando.
- [ ] Conservar la notificación de `HelperThread::drop` a la condvar de
  solicitudes: cubre al auxiliar que aún no entró en adquisición. La nueva
  cancelación cubre al que ya está dentro. Evitar sostener ambos mutex a la vez.
- [ ] Cancelar no llama al callback con un permiso ficticio ni aumenta el
  contador. Si una adquisición ganó la carrera antes de cancelar, puede acabar
  su callback; su permiso conserva el ownership y devolución RAII normales.
  El cierre no puede interrumpir un callback arbitrariamente bloqueado.

**Invariantes:** no se pierde la cancelación entre comprobar y dormir; ningún
permiso se duplica o desaparece; el hilo termina antes de que vuelva `join`;
los demás clientes siguen funcionando. No sustituir estas garantías por sondeo,
esperas temporizadas periódicas o hilos abandonados.

## P3 — Instalar las fuentes corregidas

**Actualizado el 2026-10-08:** fuentes versionadas en lugar de transformar
el código de la caché; véase [C-144](self-improvement/native/C-144.md).

- [x] Conservar jobserver 0.1.34, fijado por ambos lockfiles, con C-104 y C-144
  integradas, procedencia y licencias.
- [x] Instalar esas fuentes y resolverlas por ruta en los dos workspaces.
- [x] Probar segunda ejecución sin duplicados, limpieza de restos y resolución
  local partiendo de un lock con la dependencia del registro.
- [x] Ejecutar las regresiones directamente sobre el código que se instala;
  control negativo con el backend original en una copia temporal.

**Salida:** el bootstrap aplica la corrección tanto desde cero como sobre el
entorno actual, sin depender de editar manualmente `~/.cargo/registry`.

## P4 — Validar las carreras en host

| Caso | Resultado exigido |
|---|---|
| Cierre sin solicitudes pendientes | Termina sin liberar permisos |
| Cero permisos y adquisición ya pendiente | El cierre cancela y termina |
| Cancelar antes de entrar en adquisición | No llega a dormirse para siempre |
| Devolución de permiso concurrente con cierre | Termina y conserva el número total de permisos |
| Varias solicitudes encoladas | El cierre no necesita satisfacerlas todas |
| Dos auxiliares comparten cliente | Cancelar uno no cancela ni atasca al otro |
| Adquisición ordinaria sin cancelación | Sigue bloqueando hasta recibir un permiso válido |

- [ ] Usar intercalaciones controladas para las ventanas relevantes; añadir
  estrés como complemento, no como sustituto de esas pruebas.
- [ ] Acotar cada prueba desde el arnés y comprobar terminación, callbacks y
  contabilidad, además del código de salida.
- [ ] Confirmar que al retirar el cambio la regresión principal vuelve a fallar.

**Salida:** pruebas correctas de la implementación parcheada, base que falla y
logs con comandos/códigos. Después se autoriza técnicamente pasar al build largo.

## P5 — Reconstruir cargo y probarlo dentro de soso

- [ ] Reconstruir cargo siguiendo `config/rust-soso/README.md`, con el backend
  corregido. Identificar el build-dir efectivo y las unidades obsoletas de
  jobserver/cargo: parchear el registro no garantiza que Cargo las recompile.
  Invalidar solo los artefactos afectados y verificar el nuevo enlace.
- [ ] Guardar hashes del backend parcheado y del binario resultante. Mantener
  el mismo kernel en la comparación para aislar el efecto del cambio de cargo.
- [ ] Preparar la sonda pequeña mediante bootstrap host y ejecutarla en soso;
  no depender del cargo defectuoso para construir la propia prueba de cancelación.
  La sonda usa Rust y las primitivas de soso, sin requerir Bash o Python en guest.
- [ ] Desplegar en una imagen de prueba aislada y arrancar un QEMU propio, con
  puertos y logs distintos. No escribir sobre la imagen de un QEMU activo.
  Comprobar expresamente que el cargo nuevo llegó a la imagen: `REUSE_DATA`
  por sí solo no actualiza `/bin/cargo`.
- [ ] Con `-smp 1` y `cargo ... -j 1`, probar un workspace con al menos dos
  trabajos independientes: compilación correcta, error intencional de rustc y
  segunda compilación incremental. Verificar salida y retorno real al llamante,
  sin hilos cargo residuales. `Finished` en un log no basta.
- [ ] Repetir diez ciclos de esos fixtures pequeños en un solo arranque; probar
  además `-smp 2` con `-j 1` y `-j 2` para cubrir otras intercalaciones.
- [ ] Volver a la carga de T42: `cd /var/t42/cg/uefi` y
  `cargo build --offline --release`. Conservar el mismo estado de entrada al
  comparar; si persiste un error ajeno de lld, cargo debe devolver el fallo y
  terminar. Eso valida el cierre, pero no acredita una imagen UEFI construida.

Cada ejecución larga tendrá plazo basado en su duración medida, vigilancia
de salida/procesos y directorio de evidencia propio. Ante un timeout, capturar
`ps`, RIP/pilas y estado de las esperas antes de terminar el QEMU propio. Durante
la validación no convertir los reintentos automáticos en éxitos: registrar cada
fallo y detenerse a investigarlo.

**Salida:** sonda nativa y cargas cargo terminan; el resultado funcional coincide
con la entrada; no hay bloqueos ni residuos. Las repeticiones apoyan la prueba
del mecanismo, sin demostrar por sí solas ausencia de toda carrera.

## P6 — Cierre y entrega

- [ ] Publicar parche, regresiones, receta reproducible y hashes; resumir los
  resultados de host y guest por separado.
- [ ] Actualizar el diagnóstico con la evidencia nueva. Si el cuelgue reaparece,
  conservar jobserver como defecto corregido y abrir el síntoma restante con
  sus trazas, sin declarar resueltas las tres recurrencias.
- [ ] Registrar la corrección derivada de T42 en su backlog nativo con el ID libre
  que corresponda al empezar la implementación; enlazar desde C-104 y T42.
  Sincronizar ficha, catálogo, índice y seguimiento cuando cambie un estado.
- [ ] Actualizar la skill de automejora con la causa y el procedimiento validado;
  `soso-dev` solo si se añaden comandos de pruebas. El manual de usuario no
  necesita cambios por una corrección interna sin nueva interfaz.

**Criterio de cierre:** regresión roja antes/verde después, parche actualizable
e idempotente, cargo reconstruido identificado y validación dentro de soso.
Completar este plan no cierra por sí solo T42 ni el hito de automejora.

## Seguimiento separado de los defectos del kernel

Después de aislar la validación de cargo, abordar estos dos cambios con sus
propias regresiones y builds:

1. **Entrada expirada que consume un despertar:** comprobar la clave completa
   `(pml4, uaddr)` al despertar y definir limpieza de entradas antiguas sin
   invertir candados. Probar A → timeout → B junto a un segundo esperador en A,
   reesperas en A y acumulación de timeouts. La prueba debe ejecutarse también
   dentro de soso.
2. **Inversión `WAITERS/PROCS` al salir:** sacar `forget_pid` de las secciones
   que sostienen `PROCS`, conservando la vida del proceso y su espacio hasta
   completar la limpieza. Auditar salida normal, hilos y muerte por señales.
   Probar salidas simultáneas con wait/wake en SMP y ejecutar la suite de kernel.

Estos cambios no son requisito para probar la cancelación de jobserver y no
se mezclarán en su comparación antes/después.
