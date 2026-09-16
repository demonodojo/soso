# Actualizaciones de soso instalado y logs en sosofs

Fecha: **2026-09-16**. Estado: **U0 y U1 cerradas; U2–U8 pendientes**.
El contrato está en [U0-CONTRATO-ACTUALIZACION.md](U0-CONTRATO-ACTUALIZACION.md);
todavía no está conectado al arranque, al cliente ni al instalador.
Base de la revisión: `92531cd2d` y árbol de trabajo con cambios locales, incluidos
WiFi, instalador y registros de aplicaciones. Esta revisión describe ese árbol;
no acredita una release publicada ni una actualización probada en el ROG.

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

## 4. Entregas y dependencias

**U0 y U1 cerradas** (2026-09-16). U2–U8 siguen **pendientes**; U2 es el primer
paso ejecutable.

| ID | Depende de | Trabajo y archivos principales | Criterio de cierre |
|---|---|---|---|
| U0 ✅ | — | Contrato de transacción, tabla de cortes/reconciliación, identidad live/installed, formatos y compatibilidad; `soso-update-core` | **Cerrada 2026-09-16.** [Contrato](U0-CONTRATO-ACTUALIZACION.md) y 92 pruebas host: máquina de estados, registros rotos, confirmación interrumpida y espacio insuficiente. |
| U1 ✅ | U0 | Persistencia y rotación de los dos rings y eventos OTA; `logbuf`, `applog`, nuevo escritor sosofs, main/scheduler/halt/kshell | **Cerrada 2026-09-16.** `crates/soso-log-core` (13 pruebas host) + `kernel/src/drivers/logfs.rs`; `cargo xtask test-update` acredita que los logs tempranos y los de fd 3 sobreviven al reinicio. |
| U2 | U1 | Finalización de instalación, modo temprano y eliminación FAT; `soso-install`, `package_live`, `install_disk`, script Linux y utilidades compartidas | Instalación NVMe arranca sin USB, no contiene `SOSOLOG.TXT`, escribe `/var/log`; live conserva su log y el clon es coherente. |
| U3 | U0 | Contrato de release, perfil, canales, inventario y exclusiones; `release.rs`, manifest/pack y configuración | Fixtures verifican canales/precedencia, versiones y preservación; ninguna release contiene logs, claves, cachés o journal OTA. |
| U4 | U3 | Preflight y descarga durable reanudable; `soso-update`, `net.rs`, `soso-http` | Corte de red y reinicio reanudan solo bloques pendientes verificados, con RAM acotada y sin cambiar el sistema activo. |
| U5 | U0, U4 | Backup, exclusión de escritores, aplicación/recuperación antes de firmware/init, shim y confirmación conjunta | Cada corte deja una pareja anterior/nueva completa al arrancar; `revertir` restaura kernel, programas, borrados y versión. |
| U6 | U2, U5 | Transición de instalaciones existentes, release puente y reparación offline desde live | Migrar una copia de instalación antigua sin formatear ni perder modelos/configuración; interrupciones recuperables; eliminar log FAT tras habilitar logs nativos. |
| U7 | U1–U6 | Extender bancos host, QEMU USB→NVMe y OTA con fallos | Matriz de la sección 5 verde, incluida actualización sucesiva A→B→C y reversión C→B. |
| U8 | U7 | Release candidata y validación en ROG por WiFi | Instalación/actualización y recuperación verificadas en placa; manual y estado reflejan exactamente lo probado. |

U1–U2 dan una primera entrega útil: instalaciones con logs en sosofs. No se
anunciará actualización recuperable completa hasta U5–U8.

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
