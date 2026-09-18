# Actualizaciones de soso instalado y logs en sosofs

Fecha: **2026-09-17**. Estado: **U0–U5 cerradas; U6 a medias (inventario,
provisión, retirada del log FAT y rescate desde el live hechos; falta la
restauración offline escribiendo en el destino); U7–U8 pendientes**.
El contrato está en [U0-CONTRATO-ACTUALIZACION.md](U0-CONTRATO-ACTUALIZACION.md)
y ya gobierna el arranque, el cliente, el instalador y el shim: el kernel
reconcilia el registro de la ESP con el diario antes de firmware e init, y
`soso-update` arma en vez de instalar.
Base de la revisión: `92531cd2d` y árbol de trabajo con cambios locales, incluidos
WiFi, instalador y registros de aplicaciones. Esta revisión describe ese árbol;
no acredita una release publicada ni una actualización probada en el ROG —todo lo
de U5 está acreditado en QEMU, no en placa: eso es U8—.

## 1. Resultado buscado

Actualizar la instalación NVMe del ROG por WiFi, desde soso, conservando modelos,
datos, credenciales y configuración. Tras reiniciar debe arrancar la versión
nueva completa o recuperarse la anterior completa: **kernel y programas juntos**.

En una instalación nueva **no existirá `SOSOLOG.TXT` en la ESP**. Los registros
persistentes estarán en `/var/log` dentro de sosofs. La ESP seguirá siendo
necesaria para UEFI y recuperación de actualizaciones. El USB live conservará
su `SOSOLOG.TXT` para diagnosticar máquinas donde todavía no monta sosofs.

La petición se concreta como un plan de implementación. Los comandos y rutas
que se presentan como propuestos aún no constituyen funcionalidad disponible.

## 2. Qué hay hoy y qué falta

| Área | Evidencia en el árbol | Consecuencia |
|---|---|---|
| Cliente | [`soso-update`](../user/soso-update/src/main.rs): `estado`, `comprobar`, `aplicar`, `revertir`, `--local` | Se puede aprovechar la CLI y la descarga diferencial por hashes. |
| Orden de escritura | `cmd_aplicar` sobrescribe ficheros activos, después prepara kernel y finalmente escribe `/etc/soso-release` | Un error de descarga, ESP o reinicio deja una mezcla; la versión de disco puede anunciar un kernel aún no arrancado. |
| Recuperación | [`actualiza.rs`](../boot-shim/src/actualiza.rs), [`kernel_meta.rs`](../crates/soso-update-core/src/kernel_meta.rs) | Hay backup verificable del kernel; no hay reversión del rootfs. |
| Confirmación | [`init`](../user/init/src/main.rs): marca `/tmp/sosh-ready` del PID correcto y proceso vivo | Es una base útil, pero no confirma una transacción conjunta ni que sigan funcionando los drivers necesarios. |
| Descarga | [`net.rs`](../user/soso-update/src/net.rs): HTTPS y Range, acumulando el tramo en RAM | Faltan descarga durable reanudable, límites globales y comprobación de identidad entre peticiones. |
| Canal | `read_config_url`: la URL del fichero prima sobre `--channel`; `dev` construye una ruta con `/download` adicional | Fijar la precedencia y probar la URL generada. Resolver una release una sola vez evita mezclar cambios de `latest`. |
| Paquete | [`release.rs`](../xtask/src/release.rs) y [`PACK_SKIP`](../crates/soso-update-core/src/lib.rs) | Hay exclusiones concretas de configuración, pero no exclusión general de logs/estado mutable ni contrato de compatibilidad de shim y ABI. |
| Instalación | [`soso-install`](../user/coreutils/src/bin/soso-install.rs) clona live y ajusta GPT/GUID; [`install-disk`](../xtask/src/install_disk.rs) escribe la imagen live | El destino hereda los huecos FAT y registros del live. Hay que finalizar el destino como instalación. |
| Log de consola | [`logbuf`](../kernel/src/drivers/logbuf.rs) → [`fatlog`](../kernel/src/drivers/fatlog.rs) | La persistencia depende de un fichero precreado en FAT. `poll` compara longitud: al llenarse el ring deja de detectar nuevos bytes por ese criterio. |
| Log de aplicaciones | [`applog`](../kernel/src/drivers/applog.rs), fd 3, comando `log` | Es un ring separado en RAM; también necesita persistencia. |
| sosofs | [`crates/sosofs`](../crates/sosofs/src/lib.rs), CoW y dos superbloques | Asegura commits del FS; no proporciona por sí solo snapshots retenidos ni una transacción de actualización de múltiples ficheros. |
| Banco actual | [`test_update.rs`](../xtask/src/test_update.rs) | Cubre aplicación local y recuperación de kernel; no demuestra descarga WiFi ni rollback conjunto en NVMe. |

El [diagnóstico del ROG](DIAGNOSTICO-ROG-2026-09-16.md) ya recoge asociación,
WPA2 y DHCP con AX200. Esa evidencia permite preparar este trabajo; aún habrá
que acreditar HTTPS sostenido y la actualización completa en el equipo.

## 3. Diseño propuesto

### 3.1 Actualizar sin sobrescribir el sistema en ejecución

Mantener el esquema de particiones actual. Preparar cada operación en
`/var/lib/soso-update/<id>/`, con manifiesto fijado, ficheros nuevos, copias de
los anteriores y journal. El identificador incluirá el hash del manifiesto:
una versión textual no basta para reconocer la misma operación.

1. **Comprobar:** resolver canal a una release concreta y validar formato,
   compatibilidad, rutas, tamaños, espacio libre y capacidad de los huecos ESP.
2. **Descargar:** almacenar por bloques verificables, con progreso durable.
   Una desconexión no cambia `/bin`, `/lib` ni la versión instalada.
3. **Preparar:** verificar todo lo descargado y respaldar cada fichero que vaya
   a cambiar o borrarse. Registrar también los ficheros que antes no existían.
   Incluir la versión anterior y sus hashes. No armar el arranque hasta que
   staging, backup y journal estén confirmados en disco.
4. **Armar:** preparar kernel y registro ESP con el mismo ID. La operación queda
   lista para el próximo reinicio; se puede seguir usando el sistema anterior.
5. **Aplicar en arranque:** el shim prepara el kernel; el kernel monta sosofs y
   ejecuta el aplicador/recuperador **antes de cargar firmware de `/lib` y antes
   de arrancar `/bin/init`**. Completar la operación por fichero de forma
   idempotente. No lanzar usuarios ni drivers consumidores con una mezcla.
6. **Probar:** arrancar init/sosh; comprobar identidad kernel/rootfs, lectura y
   escritura durable de sosofs y ejecución de un programa de prueba. Conservar
   la comprobación de PID y prefault existente. En el ROG comprobar además que
   el driver WiFi y su firmware se inicializan; la falta de AP o de Internet
   no debe provocar por sí sola un rollback automático.
7. **Confirmar o revertir:** confirmar la pareja completa. Si el arranque falla,
   el siguiente arranque restaura kernel y rootfs antes de sus consumidores.
   La reversión manual usa las mismas copias. Mantener una versión anterior
   confirmada; no borrar su backup como efecto inmediato de `OK`.

Este diseño exige código de recuperación en **ambos kernels**, anterior y nuevo.
Una instalación antigua necesita la transición descrita en U6. No se confiará
en que un `init` recién actualizado pueda reparar su propio ELF roto.

El aplicador temprano será pequeño y compartirá la lógica de estados `no_std`
de `soso-update-core`; no introducirá HTTP en el arranque. Antes de armar se
verificará que ningún proceso puede modificar las rutas administradas entre
backup y reinicio. Bloquear escritores concurrentes (incluida Forja) mediante
una exclusión del kernel; una segunda instancia de `soso-update` debe fallar
con un mensaje claro. Los procesos que solo leen siguen usando los ficheros viejos.

### 3.2 Durabilidad entre sosofs y ESP

Estados propuestos: `DESCARGANDO → PREPARADO → ARMADO → APLICANDO → PROBANDO →
CONFIRMADO`, con `REVERTIENDO → REVERTIDO` y error recuperable por fase.
No equivalen automáticamente a los estados del buzón actual.

- El journal de sosofs contiene inventario, hashes, acciones y progreso; el
  registro de arranque de ESP contiene ID, pareja anterior/nueva y decisión.
  Cada registro tendrá versión de formato, secuencia e integridad comprobable.
- Definir en U0 una tabla de reconciliación de **cada pareja de estados**, con
  orden exacto de escrituras y flush. No existe un commit atómico entre FAT y
  sosofs. Usar registros redundantes para no depender de un sector indemne.
- Solo un registro ESP válido puede armar la operación. Antes de publicarlo,
  todo lo necesario para completar o deshacer debe ser durable y releído.
- El punto de confirmación será una decisión durable identificada por ID;
  preparar primero la evidencia en sosofs y después confirmar en ESP. Si se
  corta entre ambos, conservar la posibilidad de volver a la versión anterior.
  `/etc/soso-release` se reconcilia con esa decisión, nunca decide la recuperación.
- Reiniciar durante aplicación o reversión debe continuar/restaurar de forma
  determinista. Una pareja incoherente o un backup corrupto no se acepta como
  arranque normal: diagnóstico local y recuperación desde live.
- La reversión automática ocurre en el siguiente arranque. Un cuelgue puede
  requerir reinicio físico; este plan no presupone un watchdog ya disponible.

### 3.3 Datos y compatibilidad

El manifiesto identificará archivos administrados por el sistema, adiciones,
borrados y defaults. Preservar `/models`, datos de usuario, `/src`, logs y estado
de aplicaciones, además de claves SSH y configuración en `/etc`. Los defaults
nuevos no sobrescriben configuraciones existentes. Solo borrar rutas poseídas
por la versión anterior, nunca hacer una limpieza genérica del rootfs.

El rollback incluye metadatos de versión y migraciones de configuración que
forme parte de la operación. Las migraciones deben ser reversibles o compatibles
con la versión anterior; rechazar las que no permitan esa garantía.

El manifiesto debe declarar arquitectura, perfil de drivers, ABI, formato del
FS y versión mínima del shim/recuperador. Rechazar paquetes sin NVMe/WiFi del
perfil requerido. Actualizar shim/bootloader y formatos incompatibles requerirá
una transición específica, no una sobrescritura ordinaria de ficheros.

Mantener HTTPS con validación y hashes. Fijar identidad de release y validar
rutas también en el cliente, aunque el empaquetador las filtre. La política
de firmas de manifiesto y distribución de claves se decidirá en U0; los hashes
del mismo manifiesto acreditan integridad, no una firma del editor.

### 3.4 Logs nativos

Rutas propuestas: `/var/log/kernel.log`, `/var/log/aplicaciones.log` y
`/var/log/actualizaciones.log`. Cada arranque llevará un ID, versión/build y
tiempo monótono; añadir fecha real cuando el reloj sea utilizable.

- Capturar desde el inicio en RAM y vaciar a sosofs al montarlo, antes del
  bring-up de firmware. Persistir también fd 3, conservando PID y proceso.
- Añadir cursor/secuencia monotónica a los rings: su longitud se satura. Detectar
  pérdidas por sobrescritura y escribir una marca explícita; no duplicar datos.
- Escritor diferido con lotes acotados, aproximadamente cada 2 s. Copiar el ring
  y soltar su lock antes de E/S. No tomar locks del FS/disco desde IRQ ni hacer
  escrituras recursivas al registrar un error del propio escritor.
- Rotación inicial propuesta: archivo activo de hasta 1 MiB y tres rotaciones
  por flujo (máximo aproximado de 12 MiB entre los tres). Conservar el arranque
  anterior; espacio de recuperación OTA reservado al calcular capacidad. Usar
  append por lotes, sin reescribir un historial creciente cada dos segundos.
- En disco lleno, RO o error, conservar consola/ring, limitar reintentos y
  contabilizar pérdidas. El logger no puede impedir arrancar o recuperarse.
- Apagado limpio: solicitar drenaje y flush con plazo acotado. Panic: solo un
  intento seguro si el FS está disponible y sin locks ocupados; nunca forzar
  su desbloqueo para escribir. El corte brusco puede perder la cola no persistida.
- Si el kernel instalado falla **antes de montar sosofs**, esos mensajes solo
  estarán en consola/RAM. Es el límite de eliminar el log FAT; el live seguirá
  siendo el medio de diagnóstico y acceso al historial ya persistido.
- Adaptar `dmesg save` al destino efectivo y mantener `log` para el ring de
  aplicaciones. La lectura por rutas normales debe funcionar desde sosh y SSH.

### 3.5 Instalación y migración del log FAT

Introducir identidad explícita `live`/`installed` en los metadatos de arranque,
legible antes de montar sosofs. No deducir el modo exclusivamente de USB/NVMe
ni de que exista `SOSOLOG.TXT`.

Tras clonar y antes de registrar la entrada de arranque, finalizar **el destino**:
configurar modo instalado, preparar `/var/log` y el estado inicial OTA, limpiar
estados transitorios heredados del live y eliminar `SOSOLOG.TXT` de su ESP.
La eliminación debe actualizar directorio y cadena FAT correctamente; poner
ceros en su contenido no cumple el requisito. El código actual de `espfat`
localiza huecos: hace falta una primitiva de finalización adicional, comprobable
en host, para operar sobre un volumen destino no montado.

Preservar GPT/GUID, modelos, credenciales que deban trasladarse y huecos de
actualización. No copiar una operación OTA pendiente: rechazar la instalación
desde un origen en ese estado. Comprobar coherencia del sosofs de origen antes
de copiarlo: quiesce/flush mientras dura el clon, incluida la nueva persistencia
de logs, o emplear una imagen estable. No clonar bloques que están cambiando.

Aplicar la misma finalización a instalación nativa, `install-disk` y el script
Linux empaquetado. No cambiar en este trabajo el particionado `SOSOINSTALL`.
En instalaciones existentes, desactivar primero `fatlog` por modo instalado;
eliminar su fichero mediante la migración U6 una vez confirmado el logger nuevo.
Los buzones de boot/OTA siguen en ESP. `SOSODRV.TXT` no se elimina como efecto
colateral; trasladar ese informe podrá evaluarse por separado.

### 3.6 Vuelta atrás segura, incluso después de confirmar

**Ampliación del plan, 2026-09-16; entregada en U5a–U5e el 2026-09-17, salvo las
pruebas de la matriz, que son de U7.**
La confirmación del arranque no prueba que todas las aplicaciones funcionen.
Debe poder recuperarse la versión anterior también después de varios arranques,
sin Internet, sin reinstalar y sin depender de la shell de la versión defectuosa.
Las interfaces que describe esta sección **ya existen** (`soso-update revertir`
y la entrada UEFI «soso — recuperar versión anterior»); donde el texto y el
comando no coincidan, manda el comando.

#### Punto de recuperación y conservación

Antes de armar A→B, crear un punto de recuperación de A con ID completo de
transacción, GUID del destino, versiones/builds, manifiesto, kernel, firmware y
copias de los archivos administrados que cambian o desaparecen. Registrar los
archivos añadidos por B para retirarlos al volver. Incluir hashes y longitudes
exactas, metadatos de versión y cambios reversibles de configuración. Verificar
el conjunto releyéndolo tras flush; si falta una pieza, **no armar**.

- Conservar A mientras B sea la versión activa, aunque B esté confirmada y se
  reinicie muchas veces. No caducar el punto por tiempo, limpieza de caché o
  espacio escaso. `estado` mostrará versión recuperable y si se ha verificado.
- Al preparar B→C conservar A hasta confirmar C y acreditar que el nuevo punto
  de B es completo. Durante esa transición pueden coexistir dos puntos. Si C
  falla, volver a B y conservar también el punto de A que B tenía disponible.
- **No reutilizar la única copia del kernel anterior como área de descarga.**
  `SOSOKRN.BIN` hoy alterna kernel nuevo/backup y no basta para esa retención.
  U5 deberá separar staging y copias retenidas accesibles al shim; U6 ampliará
  o migrará los huecos ESP necesarios sin formatear. Comprobar capacidad antes
  de empezar; si no cabe, rechazar la actualización, sin borrar el último punto.
- La limpieza solo retira puntos no referenciados por el sistema activo, una
  transacción pendiente o su recuperación. Conservar sus manifiestos mientras
  haya dependencias; no dejar backups diferenciales huérfanos.
- Reservar capacidad **efectiva** para restaurar con CoW, journal y logs. La
  reserva fija de U0 es un mínimo, no una demostración de que quepa cualquier
  restauración: calcular el peor caso y evitar que otros escritores consuman
  la reserva mientras exista una operación que la necesite.

#### Tres vías de recuperación

| Situación | Vía propuesta | Comportamiento |
|---|---|---|
| B no supera su primer arranque | Automática en el siguiente encendido | Un intento de prueba sin confirmación durable válida inicia la recuperación de A. Una confirmación interrumpida se reconcilia según U0; no se confunde con un fallo. |
| B arranca, pero falla WiFi o una aplicación | `soso-update revertir` desde consola o SSH | Muestra A/B e ID, verifica el punto y registra la petición durable. La restauración se ejecuta en el siguiente arranque; no sustituye los ejecutables bajo procesos vivos. |
| No funciona init/sosh, o se quiere volver antes de cargar B | Opción UEFI «soso — recuperar versión anterior» | Entrada del shim, independiente de init y de la red. Muestra el destino, valida la petición y activa el mismo recuperador. Debe ser accesible también desde el menú del firmware, sin exigir una tecla que el usuario pueda perder durante el arranque. |
| Falla el shim, la ESP o la recuperación interna | Live actualizado en modo mantenimiento | Inspecciona la instalación por GUID y ejecuta recuperación offline sobre su ESP y sosofs, con el destino desmontado y acceso exclusivo. No llama al instalador destructivo. |

La entrada de recuperación tendrá un cargador/recuperador conocido y compatible
con el formato del punto, protegido de la OTA ordinaria. Su actualización exige
una transición separada y conservar el recuperador anterior hasta validar el
nuevo. U5 debe fijar sus artefactos y huecos; U6 los provisionará en instalaciones
antiguas. Un fallo de la propia ESP se atiende desde el live.

Propuesta de UX adicional: `soso-update recuperacion comprobar` verifica sin
escribir y enumera qué puede restaurar; `soso-update estado` distingue versión
en ejecución, versión confirmada, candidata y punto de vuelta atrás. Desde el
live, `soso-update recuperar --disco <id>` identificará modelo, capacidad y GUID,
mostrará el plan y pedirá confirmación antes de escribir. Debe rechazar el disco
del live, un destino montado o una copia perteneciente a otra instalación.

#### Secuencia de restauración y cortes durante la vuelta atrás

1. Resolver el punto exacto y verificar identidad, compatibilidad y todos sus
   hashes antes de la primera modificación. No elegir «la versión más antigua»
   ni interpretar metadatos dañados como un sistema sin transacción.
2. Registrar la intención de revertir en las ranuras redundantes y tomar la
   exclusión de escritores. Toda entrada —automática, manual, UEFI o live— usa
   la misma lógica y diario, con eventos identificados en `/var/log`.
3. Restaurar archivos y configuración según inventario; retirar solo adiciones
   de la operación. Registrar progreso durable y verificar cada resultado.
   Un corte durante esta fase mantiene `revirtiendo`: el próximo arranque
   reanuda sin iniciar servicios ni aplicar de nuevo B.
4. Restaurar y releer el kernel anterior. Si el recuperador sigue ejecutando
   el kernel B, **reiniciar antes de lanzar init de A**. El kernel en memoria
   no cambia al escribir el ELF en la ESP. El orden definitivo entre shim y
   recuperador debe probarse en U5, incluidos los cortes entre ambos medios.
5. Arrancar A y verificar la pareja restaurada, sosofs e init/sosh. Distinguir
   «restauración escrita» de «arranque restaurado validado»; no anunciar éxito
   solo porque se pudo escribir `revertido` en el journal.
6. Registrar motivo, versiones, hashes y resultado. Mantener B marcada como
   fallida para ese ID de manifiesto: no reintentar automáticamente la misma
   candidata. Un reintento explícito vuelve a pasar todas las verificaciones.

La recuperación tendrá reintentos acotados. Si también falla el arranque de A,
quedarse en diagnóstico/recuperación; no alternar indefinidamente A↔B. Un corte
no cuenta como permiso para borrar backups o saltarse la comprobación de hashes.
No habrá una opción de «forzar» que restaure contenido corrupto o ignore el GUID.

#### Datos creados después de actualizar y límites

La vuelta atrás restaura el sistema administrado, no rebobina todo el disco:
conservar modelos, documentos, fuentes, credenciales y logs, incluidos los de B.
Para configuraciones migradas, registrar contenido previo y posterior. Si el
usuario las modifica después, preservar una copia y usar una transformación
reversible comprobada; ante un conflicto no resuelto, detenerse antes de
mutar el sistema y explicarlo en la consola de recuperación.

Una release con cambios de datos o formato de FS incompatibles hacia atrás no
puede ofrecer esta vuelta atrás ordinaria. U3/U5 deben rechazarla hasta disponer
de una migración reversible y un recuperador compatible. No basta restaurar
binarios viejos sobre datos que ya no entienden.

Este mecanismo cubre actualizaciones defectuosas e interrupciones mientras el
disco y al menos una copia íntegra sean legibles. No recupera por sí solo un
NVMe averiado o un sosofs ilegible. El live diagnosticará esos casos sin
formatear; para restaurarlos se necesita una copia externa válida. Un rollback
fallido debe conservar los datos y copias restantes para ese rescate.

## 4. Entregas y dependencias

**U0–U5 cerradas** (2026-09-17). La sección 3.6 amplió el alcance de la vuelta
atrás después, y U5 se desglosó en U5a–U5e, entregadas y acreditadas una a una
(ver el desglose y la sección 6); la **exclusión de escritores**, que no caía en
ninguna de las cinco, se cerró aparte el mismo día. De **U6** están hechos el inventario,
la provisión desde el live y la retirada del log FAT (2026-09-17); falta la
reparación offline sobre el sosofs del destino. U7–U8 pendientes.

| ID | Depende de | Trabajo y archivos principales | Criterio de cierre |
|---|---|---|---|
| U0 ✅ | — | Contrato de transacción, tabla de cortes/reconciliación, identidad live/installed, formatos y compatibilidad; `soso-update-core` | **Cerrada 2026-09-16.** [Contrato](U0-CONTRATO-ACTUALIZACION.md) y 92 pruebas host: máquina de estados, registros rotos, confirmación interrumpida y espacio insuficiente. |
| U1 ✅ | U0 | Persistencia y rotación de los dos rings y eventos OTA; `logbuf`, `applog`, nuevo escritor sosofs, main/scheduler/halt/kshell | **Cerrada 2026-09-16.** `crates/soso-log-core` (13 pruebas host) + `kernel/src/drivers/logfs.rs`; `cargo xtask test-update` acredita que los logs tempranos y los de fd 3 sobreviven al reinicio. |
| U2 ✅ | U1 | Finalización de instalación, modo temprano y eliminación FAT; `soso-install`, `package_live`, `install_disk`, script Linux y utilidades compartidas | **Cerrada 2026-09-16.** `crates/espfat-core` (10 pruebas host) + identidad temprana en el kernel; `cargo xtask test-install` comprueba ESP sin `SOSOLOG.TXT`, modo `installed` y `/var/log` en el NVMe. |
| U3 ✅ | U0 | Contrato de release, perfil, canales, inventario y exclusiones; `release.rs`, manifest/pack y configuración | **Cerrada 2026-09-16.** `canal.rs` + inventario con motivo, 14 fixtures de canal/precedencia/exclusiones; `release` emite el contrato de compatibilidad y aborta si cuela una ruta prohibida. |
| U4 ✅ | U3 | Preflight y descarga durable reanudable; `soso-update`, `net.rs`, `soso-http` | **Cerrada 2026-09-17.** `descarga.rs` (12 pruebas host) + área de preparación en sosofs; `test-update` comprueba la reanudación **cruzando un reinicio**. |
| U5 ✅ | U0, U4 | Backup retenido, exclusión de escritores, aplicación/recuperación antes de firmware/init, shim, entrada UEFI de rescate y confirmación conjunta; desglose U5a–U5e | **Cerrada 2026-09-17** (U5a–U5e + exclusión de escritores): vuelta atrás automática, manual y desde el firmware, también tras confirmar; ningún corte arranca una pareja mezclada ni elimina la última copia válida (§3.6); y entre el respaldo y el reinicio nadie reescribe lo que el punto copió. Dos límites anotados: el arranque de la pareja antigua **bajo su propio kernel** (lo gobierna `SOSOKRN.MET`) y los cortes inyectados E2E, que son de U7. |
| U6 ⏳ | U2, U5 | Transición de instalaciones existentes, release puente, huecos ESP/entrada de recuperación y reparación offline desde live | **Parcial 2026-09-17:** inventario (`soso-update transicion`), provisión desde el live por el shim (huecos, identidad, cargador y entrada de rescate) y retirada del log FAT, acreditados E2E en `test-install`. Falta la restauración **offline de verdad** (escribir en el sosofs del destino desde el live, para cuando su kernel no arranca) y escribir la secuencia de release puente. |
| U7 | U1–U6 | Extender bancos host, QEMU USB→NVMe y OTA con fallos | Matriz de la sección 5 verde, incluida retención A→B→C, fallo de C, reversión tras confirmar y fallo durante la propia recuperación. |
| U8 | U7 | Release candidata y validación en ROG por WiFi | Instalación/actualización y recuperación verificadas en placa; manual y estado reflejan exactamente lo probado. |

U1–U2 dan una primera entrega útil: instalaciones con logs en sosofs. No se
anunciará actualización recuperable completa hasta U5–U8.

Desglose de U5 para ejecutar la ampliación sin alterar los cierres históricos:

| Subentrega | Depende de | Entregable y aceptación |
|---|---|---|
| U5a ✅ | U0 | **Cerrada 2026-09-17.** Registro de arranque en **formato 2** (punto retenido + decisión `rescatar`) que sigue leyendo el formato 1 y rechaza uno más nuevo; `txn/punto.rs` con verificación releyendo, GUID del destino, retención y `reserva_efectiva`; fila `rescatar` en la tabla de cortes. 11 pruebas host nuevas. |
| U5b ✅ | U5a, U4 | **Cerrada 2026-09-17.** `crear_punto` comprueba espacio **antes** de copiar, copia y **relee** todo; cualquier fallo o falta de espacio devuelve error y no se arma. `Retencion` conserva A mientras C no confirme y **no suelta el punto viejo si el nuevo no está verificado**. 12 pruebas host. |
| U5c ✅ | U5b | **Cerrada 2026-09-17.** Aplicador idempotente (11 pruebas cortando en cada paso), recuperador temprano en el kernel y cliente que **arma** en vez de instalar; `init` confirma la pareja. Falta el arranque de la pareja antigua **bajo su kernel** (sigue gobernado por `SOSOKRN.MET`) y los cortes E2E. |
| U5d ✅ | U5c | **Cerrada 2026-09-17.** `soso-update revertir` muestra A→B, **verifica el punto antes de prometer nada**, deja cancelar y registra la petición; el arranque siguiente restaura. Y la **entrada UEFI de rescate**: segunda `Boot####` «soso — recuperar versión anterior», sin init ni red, que sólo **pide** el rescate; restaura el kernel, que sí puede comprobar el punto entero. Vía manual acreditada E2E en `test-update`; el registro de la entrada, en `test-install`. |
| U5e ✅ | U5d | **Cerrada 2026-09-17.** Confirmación de la pareja (U5c), **bloqueo de la candidata fallida**, **limpieza sólo de puntos no referenciados** —nunca por tiempo ni por espacio— y el arranque **restaurado a prueba**: se acredita como cualquier otro, y verlo todavía a prueba al arrancar es diagnóstico, no otra restauración en bucle. Cuarto arranque E2E: `txn: Normal`. |

**U5a–U5e cerradas** el 2026-09-17, más la exclusión de escritores, con dos
límites anotados y no disimulados: la pareja antigua todavía arranca bajo el
kernel que gobierna `SOSOKRN.MET`, y los cortes inyectados E2E dentro de QEMU
son materia de U7. La ampliación exige nueva evidencia; las pruebas históricas
de U0 no certifican estas garantías.

**U6, transición obligatoria:** inventariar formato de ESP, tamaño de huecos,
shim y kernel de recuperación. La release puente debe habilitar el recuperador
sin exigir cambios incompatibles al sistema viejo. Si el OTA antiguo no puede
instalarla con garantías, usar un live actualizado en modo mantenimiento para
preparar backups, actualizar arranque y conservar el rootfs y sosomfs existentes.
Diseñar y probar esa ruta; el `soso-install` destructivo actual no es una
migración. Una vez arrancada y confirmada la base puente, habilitar OTA normal.

## 5. Validación

| Entorno | Casos obligatorios |
|---|---|
| Host, FS simulado | Cortes antes/después de cada commit y flush, escritura corta, ENOSPC, backup corrupto, registros discordantes, rollback interrumpido y GC sin borrar la única copia recuperable. |
| Host, HTTP simulado | DNS/TLS/timeout, desconexión a mitad, Range 206 y Content-Range incorrecto, 200 inesperado, redirect, 404/429/5xx, cambio de release, tamaño excesivo y hash incorrecto. Sin Internet en tests deterministas. |
| Host, paquete | Rutas absolutas/traversal/alias, duplicados, rutas protegidas, formato desconocido, ABI/shim/perfil incompatibles, mismo número con otro build y argumentos CLI inválidos. |
| Host/QEMU, logs | Más de 256 KiB kernel y 64 KiB apps, sin duplicados silenciosos, pérdidas marcadas, rotación, múltiples boots, FS lleno, escritura fallida y lector SSH simultáneo. |
| QEMU OVMF, instalación | Live USB → NVMe; segundo arranque sin USB; ESP sin SOSOLOG; origen intacto; particiones/credenciales/modelos conservados; log nuevo legible tras reiniciar. |
| QEMU OVMF, OTA | A→B correcta; interrupción de descarga; kernel o init/sosh defectuoso; fallo después de parte del rootfs; corte antes/después de confirmación; revertir; después B→C y C→B. |
| QEMU, instalación antigua | Migración con y sin huecos/meta modernos; interrupción de migración; recuperación desde live; nunca reinstalación destructiva como salida automática. |
| ROG NVMe/WiFi | HTTPS sostenido, desconexión/reconexión durante descarga, actualización, arranque sin USB, shell/SSH, WiFi/firmware y logs persistentes; reversión controlada a una versión conocida. |

Casos de aceptación adicionales de vuelta atrás (§3.6), obligatorios para U7/U8:

| Caso | Resultado exigido |
|---|---|
| B confirmada, varios reinicios y luego regresión de una aplicación | Reversión manual a A; kernel, firmware, programas y versión coinciden con el punto, con datos nuevos y logs de B conservados. |
| B deja init/sosh inutilizable o pierde su driver WiFi | Recuperación automática o desde la entrada UEFI, sin SSH, Internet ni ejecutables de B. |
| Preparar C sobre B falla al descargar o llenar ESP/sosofs | B sigue operativa y su punto de A permanece verificable. No se sobrescribe la única copia de kernel de A. |
| C falla al arrancar; después se solicita volver desde B a A | C→B recupera una pareja completa y mantiene utilizable el punto previo B→A. |
| Corte tras cada escritura/flush durante reversión, incluido kernel | Reanuda reversión; jamás init de A con kernel B; una segunda invocación no aplica de nuevo B. |
| Punto truncado, hash erróneo, GUID ajeno o ambas ranuras dañadas | Rechazo con causa y sin sobrescrituras a ciegas; diagnóstico/lectura desde live y copias restantes conservadas. |
| Configuración editada después de la migración | Restauración compatible sin perder la edición, o conflicto explícito antes de modificar el sistema; nunca descarte silencioso. |
| Disco lleno después de armar y antes de revertir | La reserva efectiva permite completar la recuperación y el journal; el logger no consume ese margen. |
| Kernel/ESP de B inarrancable, punto íntegro en el destino | Rescate desde live sobre el GUID seleccionado; otro NVMe presente queda intacto, sin reinstalar ni tocar modelos. |
| También falla A durante la validación del arranque restaurado | Diagnóstico estable y rescate disponible, sin bucle de reinicios ni falso mensaje de éxito. |

Reutilizar y extender comandos existentes:

```sh
cargo test -p soso-update-core --features std --tests
cargo test -p sosofs --features std
cargo test -p soso-http
SOSO_MODELS_SIZE=256M cargo xtask test-install
cargo xtask test-update
cargo xtask test-usb
cargo xtask check
cargo xtask test
```

Añadir los casos del plan a esos bancos; su cobertura actual no basta. Para
U8 registrar build/perfil, manifiesto y hashes, modo y GUID del disco, versiones
antes/después, ID de transacción y logs del arranque nuevo y del rollback.
Exigir tres arranques consecutivos de la candidata y una sesión útil por SSH.
Una prueba con cable o `--local` no acredita OTA por WiFi; QEMU no acredita
comportamiento ante corte eléctrico real del NVMe.

## 6. Seguimiento y siguiente paso

### Ampliación de vuelta atrás — 2026-09-16

- **Alcance:** §3.6, desglose U5a–U5e y pruebas de recuperación adicionales.
  U0–U2 conservan su cierre histórico; U3–U5 cerradas el 2026-09-17; U6 a
  medias desde esa fecha; U7–U8 siguen pendientes.
- **Hallazgo de diseño:** el slot único `SOSOKRN.BIN` alterna staging/backup;
  no acredita conservar la versión anterior al preparar la siguiente OTA.
  Además, restaurar el ELF no sustituye el kernel que sigue ejecutándose.
- **Evidencia de esta entrega:** revisión del plan, contrato U0 y formatos de
  buzón/meta; comprobación de enlaces y diff de documentación. No implementa
  ni valida todavía rollback completo en host, QEMU o placa.
- **Siguiente paso:** continúa U3; antes del aplicador real, ejecutar U5a y
  sincronizar extensión del contrato, lógica y pruebas de reconciliación.

Este documento es el índice y registro de estado del plan U0–U8. Al iniciar
cada entrega, añadir aquí fecha/base, resultado, comandos y evidencia, limitación
y siguiente paso. Mantener vínculos desde el estado del proyecto y la skill
de instalación. Los contratos de recuperación podrán reutilizarse en
[T31](self-improvement/T31-restauracion.md) y
[T43](self-improvement/T43-validacion-actualizacion-nativa.md), sin
dar por cerradas tareas de automejora con este plan.

Actualizar `MANUAL-USUARIO.md` al implementar la UX: comandos, rutas, retención,
recuperación desde live y significado de `estado`/`revertir`. Hasta entonces,
el manual debe seguir describiendo el comportamiento actual.

### U0 — cerrada el 2026-09-16

- **Base:** `92531cd2d` más el árbol de trabajo de esa fecha.
- **Resultado:** contrato en [U0-CONTRATO-ACTUALIZACION.md](U0-CONTRATO-ACTUALIZACION.md)
  e implementación `no_std` en `crates/soso-update-core`: `record.rs` (envoltura
  con formato, secuencia, suma y copias por turnos), `txn/{mod,journal,bootrec,
  reconcile}.rs` (estados, identidad por hash de manifiesto, órdenes de
  escritura, tabla de reconciliación y `preflight` de capacidad), `identity.rs`
  (live/installed) y `compat.rs` (arch, perfil, drivers, ABI, FS, shim/recuperador
  mínimos, leídos del manifiesto).
- **Comandos y evidencia:** `cargo test -p soso-update-core --features std --tests`
  (92 pruebas, verde), `cargo build -p soso-update-core` (no_std) y
  `cargo xtask check` (exit 0). Los bancos nuevos están en
  `crates/soso-update-core/tests/txn_*.rs` e `identidad_compat.rs`;
  `txn_tabla_doc.rs` falla si la tabla del documento deja de ser la que produce
  `reconcile()`.
- **Limitación:** es lógica pura, sin E/S. Nada de esto se ejecuta todavía en el
  arranque: no hay aplicador, ni escritor del diario, ni huecos `SOSOTXN.BIN` y
  `SOSOMODE.TXT` en la ESP —los reservan U2/U5—, ni emisión de las líneas de
  compatibilidad en `xtask release` (U3, que además debe fijar la política de
  firma; los hashes acreditan integridad, no autoría).
- **Siguiente paso: U1.** Persistencia y rotación de los rings de log en sosofs,
  decidiendo el destino con la identidad de la sección 9 del contrato, para
  tener evidencia dentro del sistema durante el resto del trabajo.

### U1 — cerrada el 2026-09-16

- **Base:** `92531cd2d` más el árbol de trabajo de esa fecha.
- **Resultado:** tres flujos en sosofs —`/var/log/kernel.log`,
  `aplicaciones.log` y `actualizaciones.log`— con cabecera por arranque
  (ID, versión, monotónico y fecha real sólo si el RTC es utilizable),
  cursor monotónico, marca explícita de pérdidas, rotación 1 MiB × 3 y
  suspensión con reintento ante errores de FS.
  - Lógica pura y probable en host: [`crates/soso-log-core`](../crates/soso-log-core)
    (`ring.rs` con cursor, `rotacion.rs`, `flujo.rs`, `texto.rs`).
  - Kernel: [`drivers/logfs.rs`](../kernel/src/drivers/logfs.rs) (escritor),
    [`drivers/otalog.rs`](../kernel/src/drivers/otalog.rs) (ring de eventos OTA,
    alimentado por `updslot`), `logbuf`/`applog` migrados al ring con cursor,
    y enganches en `main` (tras montar sosofs, antes del firmware), planificador,
    `halt`, panic (intento único y sólo con el FS libre) y kshell (`logfs`,
    `dmesg save` al destino efectivo).
  - `SYS_FATLOG_FLUSH` (builtin `sosolog`) persiste ahora en **los dos**
    destinos; en una instalación sin ESP ya no es un error.
  - Emisores reales de fd 3: `soso-update` registra el relato de la
    actualización, que antes sólo existía en la consola de quien la lanzaba.
- **Comandos y evidencia:** `cargo test -p soso-log-core --features std`
  (13 pruebas), `cargo xtask check` (exit 0) y `cargo xtask test-update`
  (4 arranques OK). El arranque 2 comprueba, tras reiniciar, que
  `aplicaciones.log` conserva los registros de fd 3 del arranque anterior y que
  `kernel.log` acumula **una cabecera por arranque** con las trazas tempranas
  (`boot: fs`), en vez de reescribirse.
- **Limitación:** en el live se escriben **los dos** destinos (sosofs y
  `SOSOLOG.TXT`), porque la identidad `live`/`installed` de U0 todavía no se
  escribe en la ESP: `soso-install` no crea `SOSOMODE.TXT` hasta U2, y sin él
  la identidad se resuelve como heredada. Apagar `fatlog` por modo instalado y
  retirar `SOSOLOG.TXT` es justamente el trabajo de U2/U6. Tampoco se ha
  probado el disco lleno ni la rotación dentro de QEMU —sólo en banco host—,
  ni un lector SSH concurrente: son casos de la matriz de U7.
- **Siguiente paso: U2.** Finalización de la instalación: escribir la identidad
  `installed` en la ESP del destino, preparar `/var/log` y el estado inicial OTA,
  y reservar los huecos `SOSOTXN.BIN` y `SOSOMODE.TXT` que el contrato U0 exige.

### U2 — cerrada el 2026-09-16

- **Base:** `92531cd2d` más el árbol de trabajo de esa fecha.
- **Resultado:** el destino de una instalación deja de ser «un live con otros
  GUID» y pasa a declararse como instalación.
  - [`crates/espfat-core`](../crates/espfat-core): la lógica FAT sale del kernel
    a un crate no_std sobre un trait de sectores, para poder operar sobre la ESP
    **del disco destino** —que es otro volumen y no está montado— y para poder
    probarla en host (10 pruebas). Incluye la primitiva que faltaba: **borrar**
    de verdad, marcando la entrada de directorio y liberando la cadena en todas
    las copias de la FAT, en ese orden (al revés, un corte deja dos ficheros
    enlazados sobre los mismos datos). El kernel pierde 200 líneas duplicadas.
  - Identidad temprana: `package_live` pre-crea `SOSOMODE.TXT` y `SOSOTXN.BIN`;
    [`drivers/modo.rs`](../kernel/src/drivers/modo.rs) la lee antes de montar
    sosofs y `fatlog` se apaga **sólo** con una identidad explícita, propia e
    `installed`. `live_disk` conserva ahora el GUID de la ESP para detectar un
    clon sin finalizar.
  - [`soso-install`](../user/coreutils/src/bin/soso-install.rs) finaliza el
    destino: escribe la identidad con el GUID **nuevo**, retira `SOSOLOG.TXT` y
    `SOSOBOOT.TXT`, y **rechaza clonar un origen con una OTA a medias** (el
    destino heredaría un buzón cuyo respaldo se quedó en el USB).
  - Clon coherente: `soso-install` **pausa el escritor de logs** mientras copia
    (`SYS_FATLOG_FLUSH` con modo `quiesce`). Es un riesgo que introdujo U1: desde
    entonces el kernel escribe en el rootfs cada dos segundos, y un commit de
    sosofs a mitad del clon deja en el destino un superbloque que apunta a
    bloques aún sin copiar.
  - Misma finalización en `cargo xtask install-disk` (con `espfat-core` sobre el
    dispositivo) y en el `install-soso.sh` empaquetado (con `mount`+`rm` y la
    suma calculada con `sha256sum`); un test ata el formato que escribe el script
    al que produce el crate.
- **Un hallazgo que costó la tarde:** al meter la lectura de identidad en el
  kernel, `cargo xtask test` empezó a fallar con los **cuatro shards muertos y
  el log de serie a cero bytes**. No era el código: es el techo del cargador de
  la **imagen BIOS**. Medido: 30 921 376 B de kernel arrancan, 31 318 224 B no,
  y la misma imagen **UEFI arranca sin problema** con el mismo kernel. Usar
  `soso-update-core` desde `drivers/modo.rs` costó ~400 KB y bastó para cruzarlo.
  Dos consecuencias:
  - `SOSO_FIRMWARE` pasa a **uefi por defecto** en la suite (decisión del
    usuario, 2026-09-16). Es el firmware de la placa real y el kernel va a
    seguir creciendo: el aplicador de U5 va dentro.
  - Se arregló de paso un fallo del arnés que hacía inútil `SOSO_FIRMWARE=uefi`:
    la copia por shard se llamaba siempre `test-<id>-bios.img` y `apply_firmware`
    comparaba con el nombre exacto `soso-uefi.img`, así que lanzaba la imagen
    UEFI **sin OVMF** — otro guest mudo con la misma pinta.
  - El campo `sum=` de los registros durables pasa de SHA-256 a **CRC32C**
    (`crc`, que el kernel ya enlaza vía sosofs). Protege contra escrituras
    cortadas, no contra falsificación, y esa distinción tiene precio en un
    kernel. El `install-soso.sh` de Linux ya no escribe la identidad —bash no
    calcula CRC32C con herramientas estándar—: retira `SOSOLOG.TXT` y deja la
    identidad heredada, con lo que los logs acaban en `/var/log` igualmente.
- **Comandos y evidencia:** `cargo test -p espfat-core --features std` (10),
  `cargo test -p soso-update-core --features std --tests` (14 en identidad y
  compatibilidad), `cargo xtask check`, `cargo xtask test`, `cargo xtask test-update`
  y `SOSO_MODELS_SIZE=256M cargo xtask test-install`, que añade dos
  comprobaciones nuevas sobre la imagen del destino —sin `SOSOLOG.TXT` y con
  modo `installed`— y exige que el arranque nativo escriba `/var/log/kernel.log`.
  Contraste en los logs de serie del propio banco: el live dice «modo: sin
  SOSOMODE.TXT; se trata como live» y conserva `SOSOLOG.TXT`; la instalación dice
  «modo: installed (declarado…)» y «fatlog: instalación declarada».
- **Limitación:** la finalización por `install-disk` y por el script de Linux
  **no está probada E2E**: sólo lo está el instalador nativo, que es el camino
  del criterio de cierre. El live sigue sin declararse `live` de forma explícita
  (su `SOSOMODE.TXT` va vacío y se resuelve como heredado); es equivalente en
  comportamiento, pero la declaración explícita convendría al llegar a U6.
  `SOSOTXN.BIN` queda **reservado y sin usar**: lo consume el aplicador de U5.
  Las instalaciones ya existentes no se migran: eso es U6.
- **Siguiente paso: U3.** Contrato de release: perfil, canales, inventario y
  exclusiones, emitiendo ya las líneas de compatibilidad que U0 definió y
  fijando la política de firma del manifiesto.

### U3 — cerrada el 2026-09-16

- **Base:** `e2c42f400` más el árbol de trabajo de esa fecha.
- **Resultado:** una release ya dice para qué sirve, y el cliente la rechaza
  antes de bajarla si no sirve para esta máquina.
  - **Contrato emitido:** `cargo xtask release` escribe las líneas de
    compatibilidad que U0 definió (`arch`, `perfil`, `drivers`, `abi`, `fs`,
    `min_shim`, `min_recuperador`), con `drv-all` expandido — un manifiesto que
    dijera «all» obligaría al cliente a conocer las meta-features de este
    repositorio. `soso-abi::ABI_VERSION` es ahora el número que se declara.
  - **Comprobación antes de descargar:** `soso-update` construye su `Equipo` y
    rechaza arquitectura, ABI, formato de FS o shim/recuperador incompatibles, y
    una release que **no traiga el driver de su disco de arranque**. Un
    manifiesto sin contrato se rechaza: es una release anterior a este cliente.
  - **Canales** ([`canal.rs`](../crates/soso-update-core/src/canal.rs)): precedencia
    fijada y documentada (`--local` > `--channel` > `url=` > `channel=` > stable).
    Se arreglan dos cosas que estaban mal: el `url=` del fichero anulaba en
    silencio el `--channel` de la orden, y el canal `dev` generaba
    `…/download/dev/download/manifest.txt`, con un `/download` de más. El origen
    se resuelve **una sola vez por comando** —con `latest`, resolverlo en cada
    petición puede mezclar artefactos de dos releases— y `comprobar` dice de
    dónde va a bajar y por qué.
  - **Inventario con motivo** (`por_que_se_excluye`): las exclusiones dejan de
    ser una lista de rutas y pasan a estar clasificadas (log, estado OTA, caché,
    temporal, config local, volumen, fuente). Se añaden las que U1 y U2 crearon
    y nadie había excluido todavía: **`var/log/`** y **`var/lib/soso-update/`**,
    más `var/cache/`, `tmp/` y los sufijos `.log`/`.key`/`_key`. `release`
    **aborta** si alguna ruta prohibida llega al pack: una release publicada con
    la clave SSH de quien empaquetó no se puede despublicar.
  - **Firma:** decidida y escrita en el contrato (U0 §8). Hoy **no se firma**;
    la confianza está en HTTPS contra el origen y en los hashes del manifiesto,
    lo que detecta corrupción pero **no** protege de quien controle el origen.
    Quedan fijados los requisitos de la firma futura (ed25519 separada sobre los
    bytes exactos del manifiesto, clave que sólo cambia una migración,
    verificación antes de escribir nada).
- **Un error de diseño que sólo apareció al usarlo:** la comprobación de drivers
  estaba **invertida** —exigía que la máquina tuviera todo lo que la release
  trae, de modo que cualquier release completa se rechazaba—. Lo destapó
  `test-update`, no el banco host, porque el banco había fijado las dos listas
  con el mismo contenido. Ahora el manifiesto declara lo que la release **trae**,
  el equipo lo que **necesita**, y hay un test de que lo contrario no es error.
- **Comandos y evidencia:** `cargo test -p soso-update-core --features std --tests`
  (107 pruebas, 14 nuevas en `release_canal.rs`), `cargo xtask check`,
  `cargo xtask test` y `cargo xtask test-update` 4/4 con el contrato nuevo.
- **Limitación:** los drivers que el cliente exige se deducen sólo del **medio
  de arranque**; la red por la que se actualiza todavía no entra en la cuenta,
  porque no hay forma de preguntarle al kernel qué drivers lleva compilados.
  No se ha publicado ninguna release real con este contrato: `cargo xtask release`
  está probado en compilación y por el banco de OTA con una release fabricada,
  no contra GitHub.
- **Siguiente paso: U4.** Preflight y descarga durable reanudable: que un corte
  de red y un reinicio reanuden sólo los bloques pendientes verificados, con RAM
  acotada y sin tocar el sistema activo.

### U4 — cerrada el 2026-09-17

- **Base:** `e2c42f400` más el árbol de trabajo.
- **Resultado:** bajar una actualización deja de ser una operación de todo o
  nada que además tocaba el sistema mientras bajaba.
  - **Bajar ya no cambia el sistema activo.** Antes, cada tramo descargado se
    escribía en `/bin` y `/lib` según llegaba: un corte a mitad dejaba media
    versión instalada. Ahora todo cae en
    `/var/lib/soso-update/<id>/etapa/`, verificado por hash, y sólo cuando está
    entero se copia al sistema.
  - **Reanudable de verdad.** El área lleva un registro durable
    ([`descarga.rs`](../crates/soso-update-core/src/descarga.rs)) atado al **hash
    del manifiesto**, no a la versión: si `latest` cambia entre dos intentos, lo
    bajado no se reutiliza, porque los offsets del pack son de otro fichero. Lo
    que ya está y verifica no se vuelve a pedir; un fichero truncado por un
    corte no cuenta como hecho y se baja entero.
  - **RAM acotada.** `net.rs` gana una variante que entrega cada trozo según
    llega (`https_download_span_a`) y el trozo baja de 8 MiB a
    `TROZO_MAX` = 1 MiB. Antes se reservaba el tramo entero: memoria que una
    máquina pequeña no tiene, y un corte al 99 % obligaba a repetirlo todo. Se
    siguen pidiendo **tramos** y no ficheros sueltos, para no hacer una petición
    HTTP por binario; lo que se sostiene en RAM es el fichero en curso.
  - **Comprobación previa real.** Nueva syscall `SYS_FSINFO` (88) con el espacio
    del sosofs raíz, que alimenta el `preflight` de U0: se exige sitio para
    preparar **y** para deshacer, más las reservas de log y recuperación.
    Quedarse sin espacio a mitad es una de las formas típicas de dejar una
    pareja kernel/rootfs incoherente.
- **Comandos y evidencia:** `cargo test -p soso-update-core --features std --tests`
  (119 pruebas, 12 nuevas en `descarga.rs`), `cargo xtask check`,
  `cargo xtask test` y `cargo xtask test-update` 4/4. El arranque 2 del banco
  ensucia el fichero ya instalado y vuelve a aplicar: tiene que **reanudar desde
  el área de preparación que dejó el arranque anterior**, lo que cruza un
  reinicio de verdad.
- **Limitación:** el corte de red no se provoca en el banco —la reanudación se
  prueba a nivel de unidad y, cruzando el reinicio, con `--local`—; los casos de
  la matriz de U7 (DNS, TLS, 206 con `Content-Range` incorrecto, 200 inesperado,
  redirect, 429/5xx) siguen pendientes. El área de preparación no se recoge
  sola: una release abandonada deja sus ficheros hasta que otra la sustituya.
  La identidad entre peticiones se comprueba contra el manifiesto al empezar,
  no en cada petición HTTP.
- **Siguiente paso: U5.** Respaldo, exclusión de escritores y aplicación desde
  la transacción: el aplicador temprano que consume `reconcile()` y el progreso
  del diario, con confirmación conjunta de kernel y rootfs.

### U5a — cerrada el 2026-09-17 (contrato versionado y punto retenido)

- **Extensión versionada, no un cambio a la brava.** El registro de arranque
  pasa a **formato 2** con el punto retenido y la decisión `rescatar`. Un
  registro de formato 1 **se sigue leyendo** —no trae punto, y
  `BootRecord::conoce_puntos()` obliga a tratarlo como «no consta», no como «no
  hay vuelta atrás»—; uno de formato más nuevo se rechaza entero, porque
  aceptarlo a medias es perder una garantía sin enterarse.
- **Punto retenido** ([`txn/punto.rs`](../crates/soso-update-core/src/txn/punto.rs)):
  guarda la versión A **mientras B sea la activa**, aunque B esté confirmada y
  se reinicie muchas veces. Lleva versión/build, GUID del destino —un punto de
  otra instalación no se aplica—, kernel y qué restaurar o **quitar** (lo que
  añadió B no tiene copia: volver es retirarlo). `verificar` lo **relee** entero
  y comprueba tamaños y hashes; darlo por bueno sin releer se descubre el día
  que hace falta.
- **Reserva efectiva** (`punto::reserva_efectiva`): los datos del punto contados
  **dos veces**, porque con copia en escritura conviven el bloque viejo y el
  nuevo, más logs y margen. La reserva fija de U0 era un mínimo, no una
  demostración de que quepa la restauración.
- **Retención:** `punto::recogible` sólo deja recoger lo que nadie referencia;
  durante un A→B→C conviven dos puntos a propósito.
- **Tabla de cortes** ampliada con la fila `rescatar`, que **manda sobre el
  diario**: quien pide rescate desde fuera del sistema actualizado o no puede
  arrancar, o ya confirmó y aun así quiere volver. Sin punto al que volver es
  diagnóstico, no un intento a ciegas. El test que ata la tabla al documento
  obligó a actualizar el contrato en el mismo cambio.
- **Evidencia:** 11 pruebas host nuevas (`tests/punto_retenido.rs`), 141 en el
  crate, y `cargo build -p soso-update-core --no-default-features --features txn`
  (lo que compila el kernel) sigue verde.
- **Limitación:** es contrato y lógica; **nadie crea todavía un punto**. Eso es
  U5b, que además debe conservar A al preparar C. La entrada UEFI de rescate que
  escribiría `rescatar` es U5d.

### U5b — cerrada el 2026-09-17 (creación y conservación de puntos)

- **Crear un punto es una operación con orden.** `crear_punto` comprueba que
  cabe el punto **y su restauración** *antes* de copiar nada —comprobarlo
  después sería tarde: el espacio ya estaría gastado—, copia, y **relee todo**
  para verificarlo. Cualquier fallo devuelve error y **no se arma**: publicar
  el registro de arranque prometiendo una vuelta atrás que no se ha comprobado
  es justo lo que este contrato evita.
- Clasifica sola lo que le pasa a cada ruta: lo que la versión nueva
  **reemplaza** y lo que **retira** llevan copia; lo que **añade** no tiene nada
  que copiar y se anota para quitarlo al volver.
- **Conservación A→B→C** (`Retencion`): mientras C está armada y sin confirmar
  se conservan **dos** puntos a propósito, el de A y el de B. Al confirmar C, el
  de A se suelta… **sólo si el de B está verificado**. Si no, se conservan los
  dos: quedarse sin ninguna copia recuperable por soltar una que no sabíamos si
  servía es precisamente lo que no puede pasar. Si C falla y se revierte, sobra
  el de B y el de A sigue donde estaba.
- **Evidencia:** 12 pruebas host (`tests/punto_crear.rs`) con las averías que
  importan simuladas —sin espacio, copia que falla, copia que **no se relee
  igual**, registro que no queda durable—, 153 en el crate, `cargo xtask check`
  en verde y el juego de features del kernel compilando.
- **Limitación:** `Almacen` no tiene todavía implementación real; quien cree
  puntos de verdad será el cliente al armar (lo que falta de U5c). La reserva
  usa `reserva_efectiva`, pero nadie impide aún que **otros escritores** se
  coman esa reserva mientras existe la operación: eso sigue pendiente.

### U5e — cerrada el 2026-09-17 (bloqueo, limpieza y arranque restaurado)

- **La candidata que falló no se reinstala a ciegas.** Si el registro de
  arranque dice que esa misma versión hubo que deshacerla, `aplicar` se planta y
  pide `--forzar`. Sin esto, repetir el comando vuelve a armar lo mismo y se
  entra en el bucle de aplicar, fallar y deshacer.
- **Limpieza conservadora de puntos.** Al armar se recogen los que no referencia
  ni la versión activa ni la operación nueva. Nunca por tiempo ni por falta de
  espacio, y **si no consta ninguna referencia no se recoge nada**: sin saber
  cuál es el bueno, borrar es peor que ocupar sitio. Durante un A→B→C conviven
  dos a propósito, y hay pruebas de las tres reglas.
- **La versión restaurada también está a prueba.** El rescate publica
  `restaurado-a-prueba`, no `revertido`: hasta que ese arranque llega a
  userland, la vuelta atrás no está acreditada. Verla todavía a prueba al
  arrancar significa que la restaurada **tampoco** arranca, y entonces lo que
  toca es decirlo (`RestauradoNoArranca`) en vez de restaurar otra vez lo que ya
  está puesto. Es la simétrica de `probando`, y es lo que cierra el bucle.
- **Evidencia:** cuarto arranque en `test-update`, sobre la copia ya revertida:
  tiene que encontrar la vuelta atrás **cerrada** y seguir como si nada
  (`txn: Normal`), sin repetir la restauración y sin diagnóstico.

#### Dos fallos que salieron aquí, los dos de verdad

El cuarto arranque no falló por el arnés: enseñó dos cosas que llevaban tiempo
escondidas porque nadie miraba un arranque **después** del que revierte.

- **Carrera por la secuencia del registro de arranque.** `init` acredita la
  pareja medio segundo después de que sosh dé su marca; `soso-update revertir`
  corría antes. Los dos componían su registro desde la misma lectura, calculaban
  la **misma** secuencia y escribían en la **misma** ranura. En la tanda en que
  se vio, la acreditación se perdió —`confirmado` no llegó a existir nunca— y
  con la misma facilidad podía haberse perdido la vuelta atrás recién prometida,
  que es mucho peor. Ahora el cliente **salda la acreditación pendiente** al
  arrancar: que ese programa esté corriendo *es* la prueba de que el arranque
  llegó a userland, que es lo único que init espera. De paso, `estado` deja de
  enseñar un «probando» eterno en una máquina que evidentemente arrancó.
- **El rescate dejaba el diario abierto.** Mandar sobre el diario no es
  ignorarlo: al terminar quedaba la ESP diciendo `revertido` y el diario
  `probando`, una de las combinaciones que la tabla llama imposible, y el
  arranque siguiente se plantaba. Ahora el rescate lo cierra con los eventos de
  siempre —`revertido` si llegó a aplicarse, `descartado` si no— y **antes** de
  publicar la decisión, para que un corte en medio deje el registro en
  `rescatar` y el arranque siguiente repita un rescate idempotente.

### U6 — parcial el 2026-09-17/18 (inventario, provisión, log FAT y rescate desde el live)

Una instalación hecha con un live antiguo **se puede actualizar, pero sin vuelta
atrás**: no tiene los huecos de la ESP que el contrato necesita ni la entrada de
rescate. Lo entregado aquí es saber eso y arreglarlo sin reinstalar.

- **Inventario** ([`migracion.rs`](../crates/soso-update-core/src/migracion.rs),
  `soso-update transicion`): qué hay y qué falta, separando **lo que bloquea**
  de lo que sólo conviene. Sin `SOSOTXN.BIN` no hay dónde escribir la decisión y
  no se puede prometer vuelta atrás; un registro de formato 1, la entrada de
  rescate ausente o el log todavía en FAT se dicen y no impiden nada. Reglas que
  fijan las pruebas: un hueco del **tamaño equivocado cuenta igual que no
  tenerlo** —el kernel los localiza por LBA exigiendo la medida—, un inventario
  vacío **no** se toma por bueno, y a un live no se le exige lo que no le toca
  (no necesita entrada de rescate: él *es* el rescate, y su `SOSOLOG.TXT` es lo
  que se lee cuando la máquina no monta nada).
- **`aplicar` se planta antes de descargar** si la máquina no admite vuelta
  atrás, en vez de bajar el pack, crear el punto y fallar al publicar el
  registro con el sistema ya tocado.
- **Provisión** ([`boot-shim/src/provision.rs`](../boot-shim/src/provision.rs),
  `soso-update transicion --disco N`): **la hace el shim, y tiene que ser él**.
  Los huecos son ficheros FAT pre-creados que el kernel sobrescribe por LBA;
  *crear* entradas de directorio y asignar clusters pide un driver FAT completo,
  que el kernel no tiene y UEFI sí. El shim del live abre la ESP del disco
  instalado por GUID —la misma vía del registro de entrada tras instalar—, crea
  lo que falte con el tamaño exacto, **declara la identidad** `installed`,
  sustituye `bootx64.efi` y registra las dos entradas de arranque. No toca
  rootfs, ni sosomfs, ni configuración: no es un instalador.
  - La sustitución del shim se **relee y compara**; si no coincide, devuelve los
    bytes viejos. Dejar a medias el fichero que arranca la máquina es la única
    avería de aquí sin arreglo desde el propio sistema.
  - Sólo desde el live, y el cliente lo explica si no: arrancando de la máquina
    vieja, la petición la atendería su propio shim, que es justo el que todavía
    no sabe hacerlo.
- **Retirada del log FAT:** con `/var/log` vivo y una identidad instalada
  declarada, el kernel quita `SOSOLOG.TXT` de la ESP. **Después** de que el
  escritor nativo esté en marcha, nunca antes, y nunca en el live. Si no puede,
  lo dice y se reintenta al arrancar: un log de más no rompe nada.
- **Acreditado E2E** (`test-install`, fase nueva): se fabrica una ESP antigua
  —quitándole los huecos, soltando sus cadenas de clusters y devolviéndole el
  `SOSOLOG.TXT`—, se hace la transición desde el live y **después arranca la
  máquina sola**, que es el único momento en que su propio kernel puede retirar
  el log. Los huecos se comprueban por **tamaño exacto**, no por presencia.

**Dos cosas que encontró esa prueba y que no se habrían visto de otro modo:**

- El hueco de identidad quedaba **creado y en blanco**, que es peor que no
  tenerlo: el kernel lo lee como «sin registro» y trata la máquina como un live,
  con su log en la ESP y sin retirar nada. Ahora la transición declara lo que es.
- Con el destino ya instalado y su entrada en la NVRAM, **el firmware arrancaba
  del disco**, así que quien leía la petición era el shim del propio destino y
  no hacía nada. Y como las dos imágenes son clones, sus GUID coinciden y el
  despiste no salta a la vista. Las pruebas fijan ahora el orden de arranque
  (`bootindex`), lo que de paso hace deterministas las dos primeras fases, que
  arrancaban del live por suerte y no por decisión.

- **Recuperación desde el live** (`soso-update recuperar`): abre el disco de la
  otra máquina —su GPT, su ESP por FAT y su sosofs— **sólo en lectura**, lee su
  registro de arranque, encuentra el punto retenido y lo **verifica entero**
  contra los ficheros de ese disco. Sin `--disco` **enumera**: quien llega desde
  un live no tiene por qué saberse los números de sus discos.
  - Con `--pedir` deja escrita la decisión `rescatar` en su ESP y **la
    restauración la hace su propio kernel** al arrancar. Es a propósito: es el
    único que puede tomar la exclusión de escritores y llevar el diario.
    Escribir en el sosofs de otra máquina desde fuera sería justo el atajo que
    este contrato evita.
  - Un hueco a ceros se dice como «nunca ha actualizado», no como avería: un
    registro vacío no es un registro roto.
  - **Acreditado E2E** (`test-update`, fase «ajeno»): se le cuelga al live la
    imagen que dejó la fase de vuelta atrás —con un punto retenido de verdad— y
    se exige que lo encuentre en una ESP que no es la suya, monte su sosofs y
    verifique la copia antes de ofrecer nada.

**Lo que falta de U6:** la restauración **offline de verdad**, escribiendo en el
sosofs del destino desde el live, para cuando el kernel de esa máquina **no
arranca** y por tanto no puede atender un `rescatar`; y escribir la secuencia de
release puente como procedimiento —hoy el orden recomendado es transición
primero, OTA recuperable después—.

### Exclusión de escritores — cerrada el 2026-09-17

Era el trozo de U5 que no caía en ninguna de las cinco subentregas, y sin él la
vuelta atrás tenía una grieta: el punto guarda los ficheros **tal como estaban**
al respaldarlos, así que si otro programa reescribe `/bin/sosh` después, revertir
no devuelve el sistema a un estado que existió —lo machaca con uno anterior,
perdiendo lo que aquel programa hizo—.

- **Qué protege, exactamente**
  ([`es_administrada`](../crates/soso-update-core/src/lib.rs)): las raíces que
  una release posee (`bin/`, `lib/`, `etc/`) **menos** lo que el pack ya excluye.
  Que la configuración local quede fuera es a propósito: `etc/wifi.conf` o
  `etc/llm.conf` no viajan en el pack, nadie los va a pisar, y bloquearlos sólo
  sería molestar. Una prueba ata la definición al pack: proteger algo que el pack
  no trae, o dejar suelto algo que sí, sería la misma grieta por el otro lado.
- **Leer no se toca.** Los procesos que sólo leen siguen usando los ficheros
  viejos, que es lo que se quiere mientras la versión nueva no está.
- **La exclusión sobrevive al proceso.** Al armar pierde dueño y dura hasta el
  reinicio: la ventana que hay que proteger va del respaldo al reinicio, no de un
  `main` a su `return`. El kernel la retoma al aplicar y no la suelta hasta que
  la pareja se **acredita o se deshace**, porque hasta entonces el arranque
  siguiente todavía puede tener que revertir desde esos respaldos.
- **Un dueño muerto no deja la máquina de solo lectura.** Si el proceso que la
  tomó se cae antes de armar, se suelta sola: se comprueba que siga vivo cada vez
  que se consulta.
- **La reserva deja de ser un cálculo sin consecuencias.** Con el punto ya
  creado, el cliente reserva el peor caso de la restauración y los demás
  escritores reciben `ENOSPC` antes de comérsela, en vez de descubrirlo cuando ya
  no hay margen.
- **Mensajes, no números.** Escribir una ruta administrada dice «hay una
  actualización en curso; esa ruta no se toca hasta reiniciar»; una segunda
  instancia del cliente se planta **antes de bajarse doce megas para nada**.
- **La comprobación no puede estar sólo al abrir.** Un proceso que ya tuviera
  `/bin/algo` abierto para escritura —Forja compilando, que es el caso que
  nombra §3.1— publicaría su contenido después, con el punto ya copiado, y la
  garantía se caería sin que nadie se entere. El descriptor recuerda si su ruta
  la administra una release (se decide al abrir, que es cuando se tiene la ruta
  entera) y se vuelve a comprobar **en cada escritura y al publicar**: el camino
  de streaming vuelca a sosofs según llega, así que mirar sólo en `close`
  tampoco bastaba.
- **Acreditado E2E** en `test-update`: con la operación armada, `echo > /bin/…`
  se rechaza, `echo > /var/…` funciona —una exclusión que deje la máquina de solo
  lectura no sirve de nada— y el segundo `aplicar` falla con su mensaje. Cinco
  pruebas host para la definición de ruta administrada.
- **Un fallo de U5c que esto destapó, y que era serio:** init acreditaba la
  pareja sólo al ver en `/tmp/sosh-ready` la marca de **la sosh que él lanzó**.
  Una sesión SSH lanza la suya, que reescribe la marca con su pid, e init se
  quedaba dando vueltas para siempre (`sosh viva sin marca válida`). O sea:
  **entrar por SSH en el primer medio segundo dejaba la actualización sin
  acreditar, y el arranque siguiente la habría deshecho.** Estaba ahí desde
  U5c e era invisible porque `version_efectiva` de `probando` ya devuelve la
  versión nueva; lo que lo sacó fue la exclusión, que sigue puesta mientras la
  pareja no se acredita. Ahora vale la marca de esa instancia **o la de otra
  sosh viva** —que otra shell viva haya dejado su marca es, si acaso, mejor
  prueba de que el sistema arrancó— y la fase espera `txn: pareja confirmada`
  en el serial antes de entrar, que es la única prueba de que init acredita.
- **Lo que no está probado E2E:** el descriptor abierto **antes** de empezar la
  actualización. sosh no deja mantener uno abierto entre órdenes, así que no hay
  forma de guionizarlo; el caso está cubierto por construcción (la bandera del
  descriptor y las dos comprobaciones), no por una prueba. Cubrirlo de verdad
  pide un programa de prueba que abra, espere y escriba, y encaja en la matriz
  de U7.
- **Añade la syscall `SYS_TXN_LOCK` (91) y no sube `ABI_VERSION`**: la política
  escrita en `soso-abi` es subirla al cambiar o retirar una syscall, no al
  añadir una. Subirla aquí habría hecho que **ninguna** máquina instalada
  pudiera aceptar la release, porque la compatibilidad se compara por igualdad
  exacta.

### U5d — cerrada el 2026-09-17 (vía manual y entrada UEFI de rescate)

- **`soso-update revertir` va contra la transacción**, no contra el buzón:
  enseña `actual → versión del punto` y el ID, **verifica la copia releyéndola**,
  pide confirmación (`--yes` la salta) y **no escribe nada si cancelas**. La
  restauración la ejecuta el arranque siguiente, que es la única forma de no
  sustituir ejecutables por debajo de procesos vivos.
- El kernel **restaura el punto** (`Recuperacion::Rescatar`): lo comprueba
  entero antes de escribir nada —restaurar a medias desde un punto roto deja la
  pareja mezclada— y publica `revertido`. No lleva diario de progreso y no le
  hace falta: cada paso es idempotente, así que un corte se arregla repitiéndolo.
- **Acreditado E2E**: fase nueva de `test-update` sobre una copia limpia —armar,
  aplicar, `revertir --yes`, reiniciar— que exige `txn: restaurada la versión`
  en el serial y comprueba que vuelven **el fichero y el número de versión**.
- **Un fallo con la misma raíz que el del diario, encontrado aquí:** con el hash
  del kernel vacío —máquina recién instalada, nadie lo ha anotado— el punto se
  escribía como `kernel=0 ` y al releer se recorta el espacio final, dejando un
  campo que ya no se puede partir. El registro quedaba **ilegible** y `estado`
  decía «no hay vuelta atrás guardada» con el fichero ahí delante. Ahora «no
  consta» se escribe explícito (`- -`) y hay una prueba que lo fija. De paso, el
  mensaje de «no hay vuelta atrás» dice **por qué**: si el registro existe pero
  no se puede interpretar, lo dice en vez de callarse.
- **La entrada UEFI de rescate**, que es la vía para cuando el sistema **no**
  arranca y por tanto no hay dónde escribir una petición normal:
  - El instalador registra una **segunda** `Boot####`, «soso — recuperar
    versión anterior», al mismo cargador y con `rescatar` en su OptionalData.
    Va la **última** del `BootOrder`: tiene que salir en el menú, pero nunca por
    delante del arranque normal. Como las dos apuntan al mismo cargador, que el
    firmware caiga a la siguiente no puede convertirse en una vuelta atrás
    silenciosa: si una no carga, la otra tampoco.
  - El shim (`boot-shim/src/rescate.rs`) sólo **pide** el rescate: lee el
    registro de arranque de la ESP y publica `rescatar` con el punto que ese
    registro ya trae. No restaura nada —no sabe leer sosofs—; lo hace el kernel
    en ese mismo arranque, que sí puede comprobar el punto entero antes de
    tocar un solo fichero. Si la meta identifica copia del kernel, pide también
    su vuelta atrás: la pareja vuelve entera o no vuelve.
  - **La decisión vive en `soso-update-core`** (`txn/rescate.rs`), no en el
    shim, porque en el shim no se puede probar: un registro de formato 1 es «no
    consta» y no «no hay»; sin punto no se promete nada; pedirlo dos veces no
    gasta otra secuencia; y tras restaurar no se vuelve a restaurar. 12 pruebas
    host (`tests/rescate_uefi.rs`).
  - **Acreditación:** `test-install` exige la línea `rescate Boot####` en
    `SOSOBOOT.TXT`, o sea que la entrada tiene que quedar en la NVRAM de verdad,
    no basta con que el código exista.

### U5c — cerrada el 2026-09-17 (aplicador y recuperador)

**Ojo al alcance.** Esta sesión cerró U0–U4 contra el plan tal como estaba, y
mientras tanto el plan **creció**: la sección 3.6 (2026-09-16) añadió puntos de
recuperación retenidos, tres vías de vuelta atrás y una entrada UEFI de rescate,
y U5 se desglosó en U5a–U5e. Lo entregado aquí es **una parte de U5c**, no U5.
U5a, U5b, U5d y U5e siguen enteras pendientes, y sin U5a/U5b esto **no** da la
garantía de la sección 3.6: el respaldo que maneja es el de la operación en
curso, no un punto retenido que sobreviva a la confirmación de B ni que coexista
con el de B durante B→C.

**Lo que hay, probado:**

- **Aplicador idempotente** ([`txn/aplicador.rs`](../crates/soso-update-core/src/txn/aplicador.rs)):
  ejecuta lo que `reconcile` decide, sin E/S propia (el kernel y el cliente
  aportan un `Sistema`), verificando hashes **antes** de escribir tanto lo
  preparado como lo respaldado. `se_puede_deshacer` es la comprobación previa a
  armar: sin respaldo íntegro de todo lo que cambia, no se publica nada.
- **Banco de cortes** (`tests/aplicador.rs`, 11 pruebas): corta en **cada** paso
  de aplicar y de deshacer, reanuda desde el diario durable y exige que el
  sistema quede o entero nuevo o entero anterior.
  - **Encontró un fallo real:** entre escribir un fichero y anotar que se
    escribió hay una ventana. Un corte ahí dejaba el diario diciendo
    «pendiente» sobre un fichero que ya era el nuevo, y deshacer **sólo lo
    marcado** dejaba una **pareja mezclada** — exactamente lo que este contrato
    existe para impedir. Ahora la reversión restaura la operación entera mire lo
    que mire el progreso: es idempotente (reescribir un fichero intacto con su
    propio respaldo no lo cambia) y cuesta E/S, no corrección.
- **Recuperador temprano en el kernel**
  ([`drivers/txnaplica.rs`](../kernel/src/drivers/txnaplica.rs)): lee el registro
  de arranque de `SOSOTXN.BIN` y el diario de sosofs, llama a `reconcile` y
  ejecuta. Corre **después de montar sosofs y antes de cargar firmware de `/lib`
  y de arrancar `/bin/init`**, y con una pareja incoherente **no lanza nada de
  usuario**: se queda en la consola de emergencia. El diario se escribe por
  turnos entre `diario.0` y `diario.1`.
- `soso-update-core` gana la feature `txn`, para que el kernel compile el
  contrato de transacción sin arrastrar el formato de release.

**Añadido el 2026-09-17: el cliente ya crea puntos de verdad.** `soso-update`
implementa `Almacen` sobre las syscalls y, **antes de tocar el sistema**, crea y
verifica el punto de la versión activa. Si no se puede crear entero y releer,
**la actualización no sigue** — la regla de U5b aplicada de verdad, no sólo en
banco. `soso-update estado` dice ahora a qué versión se puede volver y si esa
copia está comprobada, y el banco lo exige: el arranque 1 no vale sin
«vuelta atrás … verificada» y el 2 comprueba que `estado` la ofrece.
Dos huecos nuevos de la ESP (`UPD_WHICH_TXN`, `UPD_WHICH_MODE`) dejan el registro
de transacción y la identidad al alcance de userspace.

De paso, la aserción nueva cazó dos fallos míos: `getdents` devuelve **bytes**,
no entradas —`estado` reventaba con un índice fuera de rango—, y la propia
aserción fijaba una versión que no correspondía: al reaplicar, el punto pasa a
reflejar la versión activa en ese momento, que es justo lo que debe guardar.

**Cerrada el 2026-09-17: el cliente ya arma y aplica el kernel.** `soso-update`
dejó de escribir `/bin` y `/lib`. Ahora: baja a la etapa, crea y verifica el
punto, escribe el **diario**, prepara el kernel y **publica el registro de
arranque**; instala el recuperador del kernel en el arranque siguiente, antes de
cargar firmware y antes de `/bin/init`. `/etc/soso-release` pasa a ser un fichero
administrado más —se aplica y se deshace con el resto—, porque si no, volver
atrás dejaría los binarios viejos anunciando la versión nueva. `init` confirma la
**pareja** con `SYS_TXN_CONFIRM` (89); sin esa confirmación, el arranque
siguiente encuentra el registro en `probando` y deshace la actualización, que es
justo el comportamiento que se quiere.

El banco lo exige, no lo observa: comprueba que la línea
`txn: actualización aplicada` está en el log de serie **antes** de `boot: ethernet`
y de que se lance init. Si alguien devuelve la instalación al cliente, salta.

Dos fallos encontrados aquí, los dos por hacer que el banco mirara:

- El diario se escribía con el hash del kernel **vacío** —`/etc/soso-release` de
  la imagen live no trae línea `kernel=`— y eso lo dejaba **ilegible**: el
  arranque siguiente decía «armada sin diario», el peor diagnóstico posible
  porque no dice qué falló. Ahora `validate` rechaza un hash mal formado, y el
  kernel anterior puede **no constar** (su vuelta atrás la gobierna
  `SOSOKRN.MET`, que sí lo respalda al arrancar) pero no estar a medias.
- Al armar dos veces la misma release, el diario nuevo empezaba con secuencia
  menor que el viejo y `pick` elegía el obsoleto. La secuencia continúa ahora
  desde la que hubiera.

**Lo que queda fuera de U5c (es U5d/U5e):**

- Arrancar la pareja antigua **bajo su propio kernel**: el recuperador restaura
  el rootfs y la versión, pero la vuelta del kernel sigue siendo la del buzón
  `SOSOKRN.MET`, con su propia recuperación ya probada. Unificar las dos es lo
  que falta del «bajo su kernel» de U5c.
- Cortar en cada punto **dentro de QEMU**: los cortes siguen cubiertos sólo en
  banco host (11 pruebas), no E2E.
- `revertir` contra la transacción y la entrada UEFI de rescate: U5d.
- Exclusión de escritores entre respaldo y reinicio: sigue pendiente.

**Y antes de U5c, en realidad, van U5a y U5b:** la extensión versionada del
contrato (punto retenido, estado de rescate, reserva efectiva) y el
creador/verificador de puntos con conservación A→B→C. El aplicador de aquí les
sirve, pero su modelo de respaldo tendrá que ampliarse: hoy es «una operación,
un respaldo», no «puntos retenidos».

**Estado del árbol:** `cargo xtask check` y `cargo xtask test-update` (4/4) en
verde. `cargo xtask test` pasa, pero el paso `voz` necesita reintentos desde
este cambio, y una vez agotó los tres y tumbó la suite.

**No es un fallo de la actualización ni de voz.** Capturado el log de serie en
el momento del fallo: **pánico del kernel** con `page fault at 0x0 rip=0x0
cs=0x8 err=0x10`, es decir un `ret` sobre basura en ring 0. Analizado:

- El `rsp` del fallo cae **fuera de toda pila válida** —en `AP_STACKS`, 480 KiB
  de `.bss` a ceros que con `-smp 1` nadie usa—, así que es un **`rsp`
  corrompido**, no un desbordamiento de `KSTACK`. El log confirma
  `smp: MADT con 1 CPUs`, y había 377 567 frames libres: ni SMP ni falta de RAM.
- Las líneas `mmap-fault … región RO` y `spawn: elf inválido` de justo antes son
  del paso `init test`, que las provoca a propósito.
- Los marcos del rastro son basura de pila, no una cadena de llamadas.

Queda **abierto**: es corrupción de memoria del kernel y arreglarlo a ojo sería
peor que dejarlo documentado; hace falta una sesión con gdb y una reproducción.
Lo que sí se hizo: el pánico ahora dice **en qué pila cayó el `rsp`**
(`arch::gdt::zona_de_pila`), que es la deducción que costó la sesión entera.

**Corrección importante sobre la atribución:** el control que hice con
`git stash -u` se llevó **U2–U5 a la vez**, no sólo U5. Así que no está
demostrado que lo dispare el recuperador de U5; sólo que con el árbol de hoy
`voz` necesita reintentos y sin esas cuatro entregas no los necesitó.
