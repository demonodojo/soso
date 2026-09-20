---
name: soso-update
description: >-
  soso OTA: transacción kernel+rootfs, vuelta atrás y transición de
  instalaciones antiguas. Úsala al tocar soso-update-core (txn/, punto,
  bootrec, journal, reconcile, migracion, descarga), kernel/drivers/{txnaplica,
  txnlock,updslot,modo}, user/soso-update, boot-shim/{rescate,provision}, o al
  ejecutar/ampliar `cargo xtask test-update`. Plan U0–U8 en
  docs/PLAN-ACTUALIZACIONES.md; contrato en docs/U0-CONTRATO-ACTUALIZACION.md.
---

# soso — Actualizaciones recuperables

**Estado (2026-09-18): U0–U7 cerradas. Queda U8 (placa) y dos cosas abiertas:
la corrupción del heap del kernel en hardware y que el USB live sólo rehace la
ESP con un flasheo completo.** El plan manda:
[`docs/PLAN-ACTUALIZACIONES.md`](../../../docs/PLAN-ACTUALIZACIONES.md).

## La idea, en una frase

Una actualización es **una pareja kernel + rootfs** que se confirma o se deshace
**entera**. El cliente no instala: **arma**. Instala el arranque siguiente, antes
de firmware e init, que es la única forma de no sustituir ejecutables por debajo
de procesos vivos.

## Invariantes que no se negocian

1. **Nada se publica en la ESP hasta que deshacer es posible sólo con lo que ya
   está en sosofs**, releído. Si el punto de vuelta atrás no se puede crear y
   verificar entero, **no se actualiza**.
2. **Un estado que describe un arranque sólo lo escribe ese arranque.**
   `probando` y `restaurado-a-prueba` significan «alguien lo intentó y no
   sabemos cómo acabó». Escribirlos desde fuera (live, shim) hace que el primer
   encendido diagnostique un fallo que no ocurrió.
3. **Publicar la decisión es parte de deshacer**, no un adorno: si el kernel
   revierte y no publica `revertido`, la ESP sigue diciendo `probando` y el
   primer programa que salde la acreditación da por buena la versión que acaba
   de quitarse.
4. **«No consta» no es «no hay».** Un registro de formato 1 no sabe de puntos;
   un hueco que no existe no es un hueco roto. Confundirlos manda a diagnosticar
   al sitio equivocado.
5. **Entre el respaldo y el reinicio, las rutas administradas son de la
   operación.** Leer nunca se bloquea.

## Piezas

| Dónde | Qué |
|---|---|
| `soso-update-core/txn/{mod,bootrec,journal,reconcile,aplicador,punto,rescate}.rs` | Contrato `no_std` sin E/S: estados, registro de arranque (formato 2), diario, tabla de reconciliación, aplicador idempotente, puntos retenidos |
| `soso-update-core/{record,migracion,descarga,compat,canal,identity}.rs` | Marco de registros durables (CRC32C), inventario de transición, plan de descarga y política de respuestas HTTP, compatibilidad, canales, identidad live/instalado |
| `kernel/drivers/txnaplica.rs` | El recuperador: lee `SOSOTXN.BIN` + diario, `reconcile`, aplica/revierte/rescata **antes** de firmware e init |
| `kernel/drivers/txnlock.rs` | Exclusión de escritores (`SYS_TXN_LOCK=91`) |
| `kernel/drivers/updslot.rs` | Huecos de la ESP; `ENOENT` = no está, `ENOTSUP` = está y no sirve |
| `user/soso-update` | `estado`/`comprobar`/`aplicar`/`revertir`/`transicion`/`recuperar` |
| `boot-shim/src/{rescate,provision}.rs` | Entrada UEFI de rescate y provisión de ESP antiguas |

## Las cuatro vías de vuelta atrás

1. **Automática:** el arranque que no se acredita se deshace en el siguiente.
2. **Manual:** `soso-update revertir` — enseña A→B, **verifica la copia**, deja
   cancelar, y la ejecuta el arranque siguiente.
3. **Desde el firmware:** segunda `Boot####` «soso — recuperar versión anterior»
   (mismo cargador, `rescatar` en OptionalData, última del `BootOrder`). El shim
   sólo **pide**; restaura el kernel, que sí puede verificar el punto entero.
4. **Desde el live:** `soso-update recuperar [--disco N] [--pedir|--restaurar]`
   sobre otro disco (GPT+ESP+sosofs en lectura). `--pedir` deja `rescatar` para
   su kernel; `--restaurar` escribe en su sosofs, para cuando ese kernel **no
   arranca**, y publica `revertido` (ver invariante 2).

## Transición de instalaciones antiguas (U6)

`soso-update transicion` inventaría y **separa lo que bloquea de lo que sólo
conviene**. Sin `SOSOTXN.BIN` no hay vuelta atrás y `aplicar` se planta **antes
de descargar**.

`soso-update transicion --disco N` (sólo desde el live) deja `PROVISION <guid>`
en `SOSOBOOT.TXT`; lo ejecuta `boot-shim/src/provision.rs`. **Tiene que ser el
shim**: crear ficheros FAT necesita el driver de UEFI, el kernel sólo sobrescribe
por LBA. Crea los huecos con tamaño exacto, **declara la identidad `installed`**
(un hueco en blanco se lee como «sin registro» y la máquina se cree un live),
sustituye `bootx64.efi` releyéndolo para compararlo, y registra las dos entradas.

Orden correcto de la secuencia puente: **transición primero, OTA recuperable
después**. Al revés, esa primera actualización iría sin vuelta atrás.

## Banco de pruebas

`cargo xtask test-update [filtro]` — 12 fases; el filtro casa por nombre:

| Filtro | Qué prueba |
|---|---|
| `aplicar` | Armar, exclusión de escritores, segunda instancia rechazada, inventario |
| `version` | La pareja aplicada y **acreditada** (espera `txn: pareja confirmada`) |
| `vuelta` | `revertir` manual + 4 arranques (incluye «restaurado a prueba») |
| `ajeno` | Desde el live: enumerar, verificar y **restaurar** otro disco, arrancándolo después |
| `recuperacion` | Corte del kernel (meta `applying`) |
| `cadena` | A→B→C: dos puntos conviven, y de C se vuelve a B con su contenido |
| `confirmacion` | Corte **entre** diario y ESP: hay que **completar**, no deshacer |
| `respaldo` | Respaldo corrupto → diagnóstico, sin lanzar la shell |
| `roto` | Release con `/bin/init` que no es ELF: se deshace **sola** |
| `manifiesto` | Manifiesto inválido rechazado |
| `saliente` | TCP **de salida** desde userland: ida y vuelta contra un eco del host, y puerto cerrado que falla rápido |
| `https` | Descarga real contra GitHub; sólo corre con `SOSO_TEST_RED=1` |

El banco fija `SOSO_MODELS_DIR=target/tiny-model` si no viene puesto: la imagen
sale con el modelo mínimo y no con los pesos grandes, que aquí no aportan nada y
cuestan minutos por pasada.

Inyección de averías: `xtask/src/sosofs_img.rs` escribe **dentro del sosofs de
una imagen** (estropear un respaldo, etc.). Para la ESP, `fat32_write`.

Banco host: `cargo test -p soso-update-core --features std` (208 pruebas).
`rangos_http` (respuestas HTTP), `manifiesto` (paquete), `migracion`,
`rutas_administradas`, `punto_*`, `txn_*`, `rescate_uefi`.

## Trampas que ya costaron una tarde

- **`cargo xtask build` no compila `soso-update`.** Dio verde con dos errores
  dentro. La verificación real de userspace pasa por empaquetar
  (`package-usb-live` / `test-update`).
- **Las releases de prueba viven en `rootfs/var/actualiza-*`** y el pack sale de
  `rootfs/`: sin excluir `var/actualiza-`, unas empaquetan a otras, se cuelan en
  la release publicada (28 MB de fixtures) y desbordan la partición de un USB ya
  flasheado. El banco las limpia al terminar.
- **Marcadores de fin de sesión SSH:** esperar «apagando» es flaky —lo imprime el
  kernel por la serie y llega al canal o no—. Usar algo que imprima la sesión.
- **`ABI_VERSION` se compara por igualdad exacta**: subirla al **añadir** una
  syscall deja fuera a todas las máquinas instaladas. La política está escrita en
  `soso-abi`: subir al cambiar o retirar, no al añadir.
- **Las features de Cargo se unifican en todo el grafo.** En `getrandom` 0.2 el
  backend `rdrand` se elige **antes** que `custom`, así que bastaba con que
  `soso-http` pidiera `rdrand` para anular el `register_custom_getrandom!` de
  `libsoso` aunque el binario pidiera `custom`. Se comprueba con `nm` sobre el
  ELF: `getrandom::custom::getrandom_inner` presente, ninguna cadena `rdrand`.
- **`soso-update --traza`** imprime cada `read`/`write` del transporte con bytes
  y milisegundos. Es la herramienta que distingue «la petición no salió» de «salió
  y no volvió nada» de «volvió y lo tiramos», que desde el mensaje de error se ven
  igual. Bandera y no variable de entorno: `sosh` no tiene variables.
- **Para depurar la red, no uses la fase del banco**: son 15 minutos por intento.
  Arranca a mano la imagen ya construida (`qemu-system-x86_64` con los argumentos
  de `lanzar_live`, sin `-monitor unix:` — la ruta del scratchpad pasa de 108 B) y
  entra por SSH; reproducir baja a 30 s y desde una **segunda** sesión puedes
  correr `ps` sobre la máquina rota. Alimenta el stdin con `(echo "orden"; sleep N)`
  o el shell se cierra antes de que llegue la salida.
- **Un marcador de fin que sólo sale si la orden tuvo éxito no es un marcador**:
  la fase `https` esperaba `local:` y convertía cualquier fallo en 900 s y en un
  «la sesión SSH no terminó» que culpa al SSH. Usa un `echo` propio.
- **`page fault de usuario en 0x28` no es un puntero nulo**: es `mov %fs:0x28`,
  el canario de `-fstack-protector` del C de `ring`, con la base FS sin fijar. Lo
  arregla `libsoso::tls_init()` desde `entry!`. Si vuelve a salir, mira `tls_base`
  antes que el código que falla —el rip cae en `curve25519.c`, que no tiene culpa—.
- **El orden de candados es NET → PROCS.** Todo lo que ya tenga PROCS debe usar
  `try_lock` sobre NET. Saltárselo no da un mensaje de interbloqueo: la conexión
  saliente no despierta nunca y el plazo tampoco vence, así que `aplicar` se
  cuelga mudo.
- **`test-install` necesita `bootindex`**: con el destino ya instalado el
  firmware arranca de él y la petición la atiende el shim equivocado.

## Abierto

- **Corrupción del heap del kernel en placa** (pánico dentro de `talc`, sólo en
  hardware). Instrumentado con centinelas en `lx_kmalloc` (`kernel/lxdde/mem.rs`):
  si un driver portado se pasa escribiendo, salta en el `kfree` con dirección y
  tamaño. Plan B: compilar sin `iwlwifi` y luego sin `nouveau`
  (`SOSO_LXDDE_MODE` es de compilación; `--only kernel` reflashea en minutos).
- **Sin prueba:** interrumpir la propia migración, y el descriptor abierto antes
  de armar (sosh no mantiene descriptores entre órdenes).
- **U8**: guion de siete pasos en el plan; necesita el ROG.
