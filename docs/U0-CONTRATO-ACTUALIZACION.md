# U0 — Contrato de la transacción de actualización

Entrega **U0** de [PLAN-ACTUALIZACIONES.md](PLAN-ACTUALIZACIONES.md).
Fecha: **2026-09-16**. Base: `92531cd2d` más el árbol de trabajo.
Estado: **especificación y banco host cerrados**; nada de esto está todavía
conectado al arranque, al cliente ni al instalador. Los módulos son `no_std`
puros, sin E/S: quien los use les da los bytes ya leídos.

Implementación: [`crates/soso-update-core/src/txn/`](../crates/soso-update-core/src/txn/),
[`record.rs`](../crates/soso-update-core/src/record.rs),
[`identity.rs`](../crates/soso-update-core/src/identity.rs),
[`compat.rs`](../crates/soso-update-core/src/compat.rs).

## 1. Qué fija este contrato

Una actualización es **una pareja kernel + rootfs** que se confirma entera o se
deshace entera. El contrato define:

1. la identidad de la operación y sus estados durables;
2. los dos medios donde se escribe y el formato de sus registros;
3. el orden exacto de escrituras de armado, aplicación, confirmación y reversión;
4. qué hacer al arrancar con **cualquier** pareja de registros, incluidas las
   que sólo produce un corte de corriente;
5. la capacidad que hay que comprobar antes de empezar;
6. qué declara una release para poder aplicarse aquí;
7. la identidad `live`/`installed`, que decide si el log va a la ESP o a sosofs.

Lo que **no** fija: descarga, HTTP, empaquetado, rotación de logs, migración de
instalaciones antiguas y el aplicador real. Son U1–U6 y usan estas piezas.

## 2. Identidad de la operación

`TxnId` es el **SHA-256 del manifiesto**, no la versión. Dos builds de `0.3.0`
tienen el mismo número y son operaciones distintas: reconocerlas como la misma
mezclaría artefactos de releases diferentes al reanudar una descarga. El
directorio de trabajo es `/var/lib/soso-update/<16 hex del ID>/`.

## 3. Estados

| Estado | Significado | Sistema activo |
|---|---|---|
| `descargando` | Bajando y verificando artefactos | intacto |
| `preparado` | Todo verificado y respaldado, diario durable | intacto |
| `armado` | Registro de arranque publicado; se aplicará al reiniciar | intacto |
| `aplicando` | Aplicación en curso en el arranque temprano | **puede estar mezclado** |
| `probando` | Pareja nueva activa, sin acreditar | nuevo |
| `confirmado` | Decisión durable de quedarse con la versión nueva | nuevo |
| `revirtiendo` | Restauración en curso | **puede estar mezclado** |
| `revertido` | Pareja anterior restaurada | anterior |
| `descartado` | Cancelada antes de armar | intacto |

Transiciones (`TxnState::next`): las que no aparecen son errores, no atajos.

```
descargando ──DescargaVerificada──▶ preparado ──RegistroArranquePublicado──▶ armado
descargando/preparado ──Abortada──▶ descartado
armado ──AplicacionIniciada──▶ aplicando ──AplicacionIniciada──▶ aplicando   (reentrante)
aplicando ──AplicacionCompleta──▶ probando ──PruebaSuperada──▶ confirmado
probando/aplicando ──PruebaFallida|ReversionSolicitada──▶ revirtiendo ──▶ revertido
confirmado ──ReversionSolicitada──▶ revirtiendo
armado ──ReversionSolicitada──▶ revertido        (no se aplicó nada que deshacer)
```

Desde `armado` en adelante el diario tiene que bastar para deshacer:
`Journal::validate` exige respaldo de todo lo que se reemplaza o borra y la
pareja de kernels. Antes de armar, el mismo diario sin respaldos es legítimo.

## 4. Medios y formatos

No hay commit atómico entre FAT y sosofs. Por eso cada medio guarda **varias
copias** del registro y se escribe por turnos: una escritura cortada sólo puede
romper la copia en curso.

| Registro | Dónde | Tamaño | Copias | Contenido |
|---|---|---|---|---|
| Diario | sosofs `/var/lib/soso-update/<id>/diario.{0,1}` | variable | 2, por secuencia par/impar | inventario, hashes, respaldos, progreso por fichero |
| Arranque | ESP `SOSOTXN.BIN` | 4 KiB | 4 ranuras de 1 KiB | decisión, ID, pareja de versiones, directorio |
| Identidad | ESP `SOSOMODE.TXT` | 4 KiB | 4 ranuras de 1 KiB | `live`/`installed`, fecha, GUID de la ESP |

Envoltura común ([`record.rs`](../crates/soso-update-core/src/record.rs)): magic,
`formato=`, `seq=`, cuerpo y `sum=` con 16 bytes de SHA-256. Se distingue
«nunca escrito» (ranura a ceros, a `\n` o a `0xff`) de «roto» (magic, formato o
suma que no cuadran): lo primero es un sistema sin transacción, lo segundo exige
diagnóstico. Un `formato=` mayor del que entiende el recuperador se rechaza; no
se adivina un registro del futuro.

**Reparto de autoridad:** el diario es la autoridad sobre el *contenido* (qué
cambia, qué había antes, hasta dónde se llegó). El registro de arranque es la
autoridad sobre la *decisión*, porque el shim y el arranque temprano lo leen sin
montar sosofs. `/etc/soso-release` se reconcilia con la decisión
(`BootRecord::version_efectiva`) y nunca decide una recuperación.

**Huecos que hay que reservar (pendiente para U2/U5):** `package-usb-live` e
`install-disk` tendrán que pre-crear `SOSOTXN.BIN` y `SOSOMODE.TXT` como el
resto de ficheros 8.3, porque el kernel no crea entradas en FAT. Sin esos
huecos el contrato no se puede armar y `preflight` lo rechaza.

## 5. Orden de escrituras

El corte se define **después** de cada paso. Las constantes `ORDEN_*` de
[`txn/mod.rs`](../crates/soso-update-core/src/txn/mod.rs) son la misma lista y
el banco de cortes comprueba que tiene tantos cortes como pasos.

**Armado** — nada se publica en la ESP hasta que completar o deshacer es posible
sólo con lo que ya está en sosofs, releído tras escribirlo:

1. sosofs: artefactos verificados en el área de preparación
2. sosofs: respaldo de cada fichero que cambia o se borra
3. sosofs: diario `preparado` con inventario, hashes y versión anterior
4. ESP: kernel nuevo al hueco y `SOSOKRN.MET` staged
5. ESP: registro de arranque `armado` con el ID  ← **punto de compromiso**
6. sosofs: diario `armado`

**Aplicación** (arranque temprano, antes de cargar firmware de `/lib` y antes de
`/bin/init`):

1. sosofs: diario `aplicando`
2. sistema: cada acción del inventario, idempotente, con progreso en el diario
3. sosofs: diario `probando`
4. ESP: registro de arranque `probando`

**Confirmación** — primero la evidencia en sosofs, después la decisión en la ESP:

1. sosofs: diario `confirmado` con la evidencia del arranque
2. ESP: registro de arranque `confirmado`  ← **punto de confirmación**
3. sosofs: `/etc/soso-release` reconciliado con la decisión

**Reversión** — la petición manual se hace durable antes de tocar nada. La
reversión automática no tiene el paso 1: su disparador es encontrar `probando`
en el registro al arrancar.

1. ESP: registro de arranque `revertir` (sólo reversión manual)
2. sosofs: diario `revirtiendo`
3. sistema: restaurar cada acción desde su respaldo, idempotente
4. ESP: kernel anterior restaurado desde `SOSOKRN.BIN`
5. sosofs: diario `revertido`
6. ESP: registro de arranque `revertido`

Confirmar no borra el respaldo de la versión anterior: se conserva una versión
confirmada hacia atrás.

## 6. Tabla de reconciliación

La pareja se lee **al empezar el arranque**. Lo que el recuperador escriba
después no vuelve a entrar en la tabla hasta el arranque siguiente: por eso
encontrar `probando` ya publicado significa que el arranque anterior no llegó a
acreditarse. `reconcile()` es total: cada celda tiene una acción y sólo una.

| registro ESP \ diario | ausente | roto | descargando | preparado | armado | aplicando | probando | confirmado | revirtiendo | revertido | descartado |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **ausente** | normal | normal | descartar | descartar | retroceder | **revertir** | **revertir** | completar confirmación | **revertir** | normal | descartar |
| **roto** | normal | diag. rotos | descartar | descartar | descartar | diag. sin decisión | diag. sin decisión | diag. sin decisión | diag. sin decisión | completar reversión | descartar |
| **idle** | normal | normal | descartar | descartar | descartar | **revertir** | **revertir** | completar confirmación | **revertir** | normal | descartar |
| **armado** | diag. sin diario | diag. sin diario | diag. imposible | **aplicar** | **aplicar** | **aplicar** | publicar probando | diag. imposible | **revertir** | completar reversión | diag. imposible |
| **probando** | diag. sin diario | diag. sin diario | diag. imposible | diag. imposible | diag. imposible | **revertir** | **revertir** | completar confirmación | **revertir** | completar reversión | diag. imposible |
| **confirmado** | normal | normal | diag. imposible | diag. imposible | diag. imposible | diag. imposible | completar confirmación | normal | **revertir** | completar reversión | diag. imposible |
| **revertir** | diag. sin diario | diag. sin diario | descartar | descartar | **revertir** | **revertir** | **revertir** | **revertir** | **revertir** | completar reversión | descartar |
| **revertido** | normal | normal | diag. imposible | diag. imposible | diag. imposible | diag. imposible | diag. imposible | diag. imposible | **revertir** | normal | normal |

Acciones: `normal` arranca hacia init sin tocar nada; `descartar` recoge el área
de preparación; `retroceder` devuelve el diario a `preparado` porque el armado
nunca llegó a publicarse; `aplicar` continúa de forma idempotente por el
progreso del inventario; `diag.` no es un arranque normal, sino diagnóstico
local y recuperación desde live.

Decisiones de la tabla que conviene no reinventar:

- **ESP `armado` + diario `preparado` → aplicar.** El registro sólo se publica
  cuando ya se puede completar o deshacer con lo que hay en sosofs, así que el
  diario que se quedó atrás no impide nada.
- **ESP `probando` + diario `confirmado` → completar confirmación.** Es la
  confirmación interrumpida entre los dos medios. El arranque sí se acreditó;
  revertir un sistema que funciona es peor que cerrar la decisión. El respaldo
  se conserva, así que sigue habiendo camino de vuelta.
- **ESP `confirmado` + diario `revertido` → completar reversión.** Al revés que
  la anterior: el rootfs ya volvió atrás, y una decisión vieja en la ESP no
  manda sobre un sistema restaurado.
- **ESP ausente o `idle` + diario a medias → revertir.** Sin registro no se pudo
  armar nada, así que un diario en `aplicando` significa que el registro se
  perdió con el rootfs a medias: se deshace.
- **ESP `armado` sin diario legible → diagnóstico.** No se puede comprobar que
  exista con qué deshacer. Aplicar a ciegas es justo lo que este contrato evita.
- **ESP ausente + diario roto → normal.** Sin registro de arranque no hubo
  armado, luego el sistema activo está intacto; un diario corrupto por su cuenta
  no es motivo para dejar de arrancar. Se pone en cuarentena y se recoge.

Invariante que comprueba el banco: **ninguna** pareja con el rootfs posiblemente
mezclado (`aplicando`, `revirtiendo`) acaba en un arranque normal.

## 7. Capacidad previa

`preflight(&Necesidad, &Capacidad)` se ejecuta antes de bajar nada. Suma
preparación, respaldo y crecimiento, más `RESERVA_LOGS` (12 MiB, los tres flujos
de U1) y `RESERVA_RECUPERACION` (8 MiB, para que una recuperación pueda escribir
en un disco lleno). Rechaza además un kernel que no cabe en el hueco de la ESP
—o de tamaño cero— y huecos de registro insuficientes. Quedarse sin sitio a
mitad es una de las formas típicas de dejar una pareja incoherente.

## 8. Compatibilidad de la release

El manifiesto declara `arch`, `perfil`, `drivers`, `abi`, `fs`, `min_shim` y
`min_recuperador`. `Manifest::parse` los lee si están y deja `compat: None` en
los manifiestos anteriores; `compat::exigir` rechaza un paquete **sin**
declaración, así que un manifiesto viejo se lee pero no se aplica.

`min_shim`/`min_recuperador` mayores que los de este equipo son exactamente el
caso de U6: hace falta una release puente, no sobrescribir ficheros. Falta un
driver declarado (el del disco de arranque, el de la red de la OTA) se rechaza
antes de descargar.

La política de firma del manifiesto y distribución de claves **sigue sin
decidirse**: los hashes acreditan integridad, no autoría. U3 tendrá que fijarla.

## 9. Identidad live / instalado

`SOSOMODE.TXT` se lee antes de montar sosofs. De él dependen el log FAT y el
recuperador. No se deduce del medio ni de que exista `SOSOLOG.TXT`.

| Situación | `Identidad` | Modo efectivo | ¿Retirar `fatlog`? |
|---|---|---|---|
| Registro `installed` con el GUID de esta ESP | `Explicita` | `installed` | sí |
| Registro `live` | `Explicita` | `live` | no |
| Sin registro (stick o instalación anterior a U2) | `Heredada` | `live` | no |
| Registro ilegible | `Rota` | `live` | no |
| Registro con el GUID del origen (clon sin finalizar) | `Ajena` | `live` | no |

Sólo una identidad explícita y propia autoriza a apagar el log FAT y a borrar
`SOSOLOG.TXT`. Ante la duda se conserva el comportamiento actual: un log de más
no rompe nada, y perder el único canal de diagnóstico de una máquina que no
monta sosofs sí.

## 10. Banco host

```sh
cargo test -p soso-update-core --features std --tests
cargo build -p soso-update-core            # sin std: lo que compilan shim y userspace
```

| Fichero | Cubre |
|---|---|
| `tests/txn_estados.rs` | transiciones válidas y prohibidas, reentrada de la aplicación, `preflight` con espacio insuficiente, hueco pequeño y necesidades desbordadas |
| `tests/txn_registros.rs` | envoltura, ranura vacía frente a rota, corte antes de la suma, bit cambiado, suma de otro registro, formato desconocido, turnos de ranura, diario roundtrip y sus invariantes |
| `tests/txn_cortes.rs` | un corte por cada paso de los cuatro órdenes, confirmación interrumpida, ID discordante, registros rotos y el barrido de todas las parejas |
| `tests/txn_tabla_doc.rs` | que la tabla de la sección 6 sea la que produce `reconcile()` |
| `tests/identidad_compat.rs` | identidad live/instalado, clon sin finalizar, declaración de compatibilidad y manifiesto antiguo |

## 11. Siguiente paso

**U1**: persistencia y rotación de los rings de log en sosofs, usando ya la
identidad de la sección 9 para decidir destino. U2 añade la finalización de la
instalación y los huecos ESP de la sección 4. El aplicador real (U5) consume
`reconcile()` y el progreso del diario; hasta entonces nada de esto se ejecuta
en el arranque.
