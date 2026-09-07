# Plan de mejoras a corto plazo de soso

Fecha: 7 de septiembre de 2026. Versión revisada: **0.2.2**.
Base: commit `744945dd4` y árbol de trabajo local, incluidos los cambios todavía
sin commit en ocho archivos de iwlwifi/nouveau. Estos cambios no se han modificado.
Horizonte: **2–4 semanas**, suponiendo una persona dedicada al desarrollo.

## Objetivo y criterio de prioridad

Conseguir una versión que arranque, permita trabajar y se actualice de forma
predecible en QEMU y en los equipos físicos disponibles. La amplitud funcional
ya es considerable: kernel propio, sosofs CoW, inferencia, GPU, WiFi, USB,
instalación y OTA. La mejora inmediata con más valor es reducir fallos de
integración y hacer que las comprobaciones detecten los problemas reales.

Este documento propone trabajo; no declara implementadas las mejoras. Distingue
hallazgos del código, resultados ejecutados y validaciones pendientes. Las
estimaciones son días de trabajo, no garantías de calendario.

| Orden | Entrega | Prioridad | Esfuerzo orientativo |
|---|---|---|---|
| 1 | A1. Pruebas aisladas y hostcheck GPU reparado | P0 | 1–2 días |
| 2 | A2. OTA: errores, versiones y confirmación correctos | P0 | 1–2 días |
| 3 | A3. HTTPS: reloj, redirecciones y liberación de conexiones | P0 | 2–3 días |
| 4 | A4. Una única enumeración PCI con medición de BAR | P1 | 0,5–1 día |
| 5 | A5. Comprobaciones automáticas y builds reproducibles | P1 | 1–2 días |
| 6 | A6. Recuperación OTA tras interrupciones | P1; requisito antes de ampliar su distribución | 4–6 días, revisar tras diseño |
| 7 | A7. Cierre verificable de hilos de inferencia | P1 | 2–4 días |
| 8 | A8. Validación por equipo y métricas de referencia | P1 | 1–2 días de banco, más disponibilidad de hardware |
| 9 | A9. Documentación operativa coherente | P2 | 0,5–1 día, repartido entre entregas |

**Compromiso inicial recomendado:** A1–A5 y documentación asociada. Después,
abordar A6 si la prioridad es instalar/actualizar equipos, o A7 si el uso principal
es `ask` y modelos grandes. Recomiendo A6 primero para máquinas instaladas.
Completar toda la lista podría superar cuatro semanas; A8 se limita a los equipos
disponibles y las optimizaciones de rendimiento quedan condicionadas a sus medidas.

## Base observada y límites del análisis

Se revisaron README, VERSION, el plan de modelos grandes, `board.txt`, las skills
de arquitectura/desarrollo/live/WiFi y los apartados pertinentes de GPU. Se
contrastaron con el código de pruebas, PCI, procesos e hilos, HTTPS, actualizador
y boot-shim. Es una revisión dirigida de los caminos críticos, no una auditoría
exhaustiva de todo el kernel y del código Linux incorporado.

Aspectos que conviene conservar:

- sosofs ya tiene pruebas de cortes de escritura y corrupción; sosomfs dispone
  de pruebas de importación y caché. Hay una base útil para probar OTA por fallos.
- El banco QEMU separa inferencia densa, MoE, syscalls y presión de memoria.
  El shard denso usa SMP para ejercitar workers; no sustituirlo por uno de un core.
- Existen pruebas específicas de USB, instalación y actualización. USB ya inyecta
  teclado mediante el monitor: hay que ampliar esa cobertura, no crearla de cero.
- GPU ya informa de disponibilidad real del pool VRAM y distingue caminos CPU/GPU.
  No sustituir esa evidencia por un simple mensaje de firmware cargado.

### Comprobaciones realizadas durante esta revisión

Los resultados corresponden al árbol local, no a un checkout limpio del commit.

| Comprobación | Resultado |
|---|---|
| Pruebas host de siete paquetes, comando debajo | Resultado pendiente de cierre de la ejecución |
| `./scripts/l6-iwl-fw-hostcheck.sh` | Correcto con los firmwares AX200 `cc-a0-77` y AX211 `so-a0-gf-a0-89`; verifica parser, no asociación WiFi |
| `./scripts/l6-g3-gsp-hostcheck.sh` | Falla al enlazar símbolos `gsp_matmul_*`, `gsp_softmax_rows_*` y `gsp_layernorm_rows_*` |
| `./scripts/l6-fwsec-hostcheck.sh` | Compila el comprobador, pero no ejecuta la validación: falta una VBIOS `.rom` accesible |
| Suite QEMU, USB, instalación, OTA y VFIO | No ejecutadas en esta revisión; no se afirma que estén en verde |

Comando host ejecutado sin descargar dependencias:

```sh
cargo test --offline \
  -p sosofs -p sosomfs -p soso-update-core -p gptdisk \
  -p soso-http -p soso-audio -p gguf2som --features std
```

La skill de desarrollo documenta dos fallos de `ask` en su línea base del
31 de agosto. Son antecedentes, no fallos reproducidos aquí. La colisión de FIFO
descrita en A1 es un hallazgo independiente del código; hay que comprobar cuánto
explica esos fallos antes de atribuirlos todos al arnés.

## A1. Recuperar una señal fiable de las pruebas

**Evidencia.** [xtask/src/test.rs](xtask/src/test.rs), `ssh_guion_inner`, utiliza
siempre `target/.ssh-guion.fifo`, aunque varios shards llaman al helper en
paralelo. Cada llamada elimina y recrea esa ruta. La sincronización de stdin
y el cierre de procesos están duplicados en `ssh_ask_literal`. Además,
`run_usb_scenario` borra variables de entorno globales después de soltar
`USB_ENV_LOCK`, mientras otro escenario puede estar configurando su arranque.

El [hostcheck GSP](scripts/l6-g3-gsp-hostcheck.sh) enlaza cuatro archivos SASS
embebidos, pero [source.list](lxdde/ports/nouveau/source.list) contiene también
`matmul_sass_embed.c`, `softmax_rows_sass_embed.c` y
`layernorm_rows_sass_embed.c`. El fallo de enlace se reprodujo en esta revisión.

**Trabajo.** Aislar FIFO, sockets y archivos temporales por ejecución y sesión;
unificar el helper SSH manteniendo stdin abierto hasta completar el protocolo;
drenar stdout/stderr durante la ejecución; garantizar recogida de SSH, holder y
QEMU al fallar. Pasar la configuración USB al `Command` concreto en vez de mutar
el entorno compartido. Sincronizar los objetos del hostcheck con los kernels SASS
que realmente usa compute, sin sustituirlos por stubs que oculten el problema.

**Aceptación.** Ejecutar sesiones SSH simultáneas contra guests distintos sin
cruzar entrada ni salida; timeout y error de arranque no dejan procesos propios
vivos. Los dos casos de `ask` deben exigir respuesta real, no solo eco del comando.
Para el residente, comprobar que cada pregunta termina y que la reconexión recibe
respuesta sin recarga. Tras el arreglo, tres pasadas QEMU con la configuración
paralela habitual sin reintentos que oculten fallos, una secuencial de comparación,
los cuatro escenarios USB y hostcheck GSP completos.

## A2. No anunciar una actualización que no ha terminado

**Evidencia.** En [user/soso-update/src/main.rs](user/soso-update/src/main.rs),
`cmd_aplicar` imprime un aviso si `apply_kernel` falla, pero continúa escribiendo
la versión y el hash nuevos en `/etc/soso-release`, elimina `actualiza.estado`
y devuelve éxito. `apply_kernel` usa ese hash para decidir si puede omitir futuras
descargas: un fallo puede dejar además una indicación incorrecta de «sin cambios».
El mismo hash se publica con `--sin-kernel`.

En [user/init/src/main.rs](user/init/src/main.rs), `lanzar_shell` llama a
`confirmar_actualizacion` **antes** de intentar `spawn("/bin/sosh")`; se puede
marcar `OK` aunque la shell no arranque. El retorno de la escritura de confirmación
se ignora. La prueba actual [test_update.rs](xtask/src/test_update.rs) cubre
aplicación y comprobación tras reinicio, pero no esa matriz de fallos.

**Trabajo.** Separar versión del rootfs, kernel preparado y kernel confirmado;
preservar el hash real con `--sin-kernel`. Propagar los errores de datos, estado y
buzón; dejar un estado recuperable y devolver error si una fase necesaria falla.
Confirmar después de una comprobación mínima de funcionamiento: rootfs accesible
y shell capaz de arrancar y comunicar que está lista. La ausencia de WiFi/GPU
opcional no debe impedir esa confirmación.

**Aceptación.** Probar kernel ausente/corrupto, slot no disponible, escritura
fallida, `--sin-kernel` y shell que no puede arrancar. Ningún caso publica un hash
no instalado ni borra el estado pendiente indebidamente. Un reintento vuelve a
preparar el kernel que faltaba. `PROBANDO` sin confirmación activa la recuperación
prevista en el siguiente arranque. Esta entrega mejora la semántica; la tolerancia
a cortes en mitad de una escritura se aborda en A6.

## A3. Hacer fiable el transporte HTTPS compartido

**Evidencia.** [crates/soso-http/src/lib.rs](crates/soso-http/src/lib.rs):

- `FixedTimeProvider` devuelve siempre el 1 de enero de 2025. Se valida la cadena
  de certificados, pero contra un instante que puede rechazar certificados
  actuales o aceptar certificados vencidos respecto al reloj real.
- `https_request` reutiliza `auth` al seguir redirecciones, aunque cambie el host.
  Un token de Hugging Face puede enviarse a otro origen indicado por `Location`.
- Después de conectar, varios retornos con `?` preceden a `transport.close(fd)`;
  no hay un guardia de conexión que asegure el cierre en errores TLS/E/S.
- Los tests actuales se centran en cabeceras, parseo y un transporte simulado
  básico; no cubren esos tres comportamientos.

**Trabajo.** Inyectar una fuente de tiempo en el cliente TLS y proporcionar hora
UTC desde el guest —por ejemplo RTC, con validación y error explícito si no hay
hora utilizable— manteniendo la verificación de certificados. Restringir el
Bearer al origen autorizado, retirarlo al cambiar esquema/host/puerto y resolver
correctamente redirecciones relativas. Introducir propiedad de la conexión con
cierre garantizado en todos los caminos. Separar errores de reloj, TLS y red en
los mensajes de las aplicaciones que lo consumen.

**Aceptación.** Tests deterministas con reloj y transporte simulados: certificado
válido, futuro y expirado; redirección al mismo origen y a otro; fallo durante
handshake/lectura/escritura; una única liberación por conexión abierta. Comprobar
después los flujos de `soso-hf`, `soso-web` y `soso-update`. Las pruebas automáticas
no dependerán de servidores externos ni de tokens reales.

## A4. Evitar reprogramar BAR al consultar dispositivos PCI

**Evidencia.** [kernel/src/drivers/pci.rs](kernel/src/drivers/pci.rs), `bar_size`,
escribe `0xffff_ffff` para medir BAR. Ya existe `devices()` con una foto cacheada,
pero seis consumidores siguen llamando a `enumerate()`:

`drivers/{usb_storage,nvme,gpu,nvidia_probe,e1000e}.rs` y `lxdde/pci.rs`, bajo
`kernel/src/`. Aunque se llamen durante el arranque, vuelven a medir dispositivos
que otro controlador puede haber inicializado antes.

**Trabajo.** Migrar esos consumidores a la foto inicial conservando filtros y
propiedad de los datos. Restringir la función de enumeración con efectos de
escritura al módulo de arranque. Revisar el orden de `pci::init()` y los probes.

**Aceptación.** Ninguna llamada externa a la enumeración que mide BAR. Arranque,
lectura/escritura de disco, tráfico y `hwscan` repetido sin nuevos sondeos ni
pérdida de dispositivos, en perfiles QEMU y live. El inventario de dispositivos
y los tamaños de BAR deben coincidir con la base anterior.

## A5. Automatizar la cobertura que hoy exige memoria del desarrollador

**Evidencia.** No se encontró configuración de CI versionada en `.github/` ni
GitLab/Jenkins. `run_host_tests` en [xtask/src/test.rs](xtask/src/test.rs) no incluye
directamente `soso-update-core`, `soso-audio` ni `gguf2som`; que otro paquete dependa
de ellos no ejecuta sus pruebas propias. Kernel, userspace y boot-shim son árboles
separados del workspace raíz, por lo que probar solo la raíz no valida sus builds.

[crates/soso-http/build.rs](crates/soso-http/build.rs) busca el primer
`ring-0.17.*` bajo una ruta fija de `~/.cargo/registry/src`. Eso liga el enlace al
contenido y orden de la caché local, no necesariamente a la versión resuelta.

**Trabajo.** Incorporar primero un comando local de comprobación común y después
un workflow que lo invoque. Dividirlo en host, builds bare-metal y QEMU; reservar
USB/instalación/OTA para una ejecución periódica o candidata a release. Incluir
los paquetes omitidos con sus features correctas y los hostchecks cuyos fixtures
estén disponibles. Fijar dependencias con lockfiles y resolver los ensambladores
de `ring` de forma determinista, respetando `CARGO_HOME`. Mantener el nightly
fijado; actualizarlo no es un objetivo de este ciclo.

**Aceptación.** Una copia nueva puede ejecutar el procedimiento documentado;
la CI usa el mismo comando local, guarda resultado y log por shard y distingue
fallo de código de prerrequisito ausente. Un fallo de OTA/gguf2som/audio hace fallar
la comprobación. Los jobs de hardware quedan explícitamente separados de los
que puede ejecutar cualquier runner. Depende de A1 para una señal QEMU fiable.

## A6. Recuperación OTA verificable, además de checksums

**Evidencia.** En [boot-shim/src/actualiza.rs](boot-shim/src/actualiza.rs),
`aplicar_kernel` reemplaza el único slot con la copia antigua, escribe el kernel
activo y solo después registra `PROBANDO`. Existen puntos intermedios de corte
que el estado actual no describe. `revertir_kernel` estima la longitud antigua
buscando el último byte no nulo, sin tamaño/hash de backup persistidos; el helper
de escritura tampoco trunca el archivo a la longitud nueva.

El rootfs se sustituye fichero a fichero en `apply_span`. El CoW del filesystem
no convierte esa secuencia completa en una transacción de release, y el rollback
del kernel no restaura automáticamente los binarios de userspace anteriores.

**Trabajo.** Diseñar primero el protocolo durable: estados de preparación,
backup, aplicación, prueba y confirmación; tamaño y hash exactos del backup;
orden de flush y recuperación tras cada punto de corte. Elegir un mecanismo
explícito para conservar la generación anterior del rootfs y su compatibilidad
con el kernel, o limitar el alcance admitido hasta tenerlo. Si requiere otro slot
ESP, incluir detección y migración de imágenes 0.2.x; no asumir que existe espacio.

Validar el manifiesto antes de escribir: hashes completos, rutas relativas
normalizadas sin `..`, duplicados, límites de tamaños y sumas comprobadas contra
`pack_size`. `Manifest::parse` admite actualmente campos numéricos y rutas sin
esas comprobaciones globales. Evitar anunciar soporte de ficheros arbitrariamente
grandes mientras `apply_span` los descarga completos a memoria.

**Aceptación.** Banco host con inyección de fallos tras cada operación persistente
y pruebas OVMF de reinicio. Recuperar una pareja compatible de kernel/rootfs;
rechazar backup corrupto; restaurar exactamente un kernel que termine en ceros o
sea más corto que el nuevo. Un manifiesto inválido se rechaza antes de mutar el
sistema. Publicar la matriz de puntos de corte probados y los límites que resten.
Depende de A2 y del arnés A1; estimar de nuevo tras decidir el formato persistente.

## A7. Cerrar y recoger los hilos antes de liberar su estado

**Evidencia.** [kernel/src/task/mod.rs](kernel/src/task/mod.rs), `thread_spawn`,
crea entradas que comparten `AddrSpace`; `exit_current` deja hijos vivos huérfanos.
Eso es útil para procesos independientes como demonios, pero exige distinguirlos
de los hilos que dependen del estado del creador.

[user/soso-llm/src/staging.rs](user/soso-llm/src/staging.rs) tiene `shutdown`,
pero el camino revisado no lo activa al terminar. `disable_worker` cambia flags
sin recoger un worker que ya se hubiera creado. `ThreadPool` sí tiene `Drop`,
pero sus esperas tienen topes; hay que verificar que no se reutiliza el estado
global mientras queden workers. [ask.rs](user/soso-llm/src/ask.rs) mantiene el
staging asíncrono desactivado por un bloqueo SMP documentado.

**Trabajo.** Reproducir terminación normal, cambio de modelo y muerte por señal
con workers activos. Añadir cierre explícito, despertar y recogida verificable
del staging; garantizar la vida del estado compartido y de `SOURCE_PTR` hasta
esa recogida. Definir qué ocurre si un worker falla. Diseñar por separado la
pertenencia de los hilos en el kernel si es necesaria: no matar indiscriminadamente
todos los hijos, pues rompería el arranque independiente de `askd`/`vozd`.

**Aceptación.** Repetir al menos veinte ciclos de carga/generación/cambio/salida,
con 2 y 8 vCPU y presión de memoria; estabilizar número de tareas y memoria tras
el calentamiento. Interrumpir generación y reconectar sin bloqueo. Conservar
staging síncrono en `askd` hasta reproducir y resolver el bloqueo con un test;
no activar la optimización solo porque compile.

## A8. Validar soporte y rendimiento por equipo

**Evidencia.** Las skills registran G1–G5 funcionales en GB205, pero señalan
validaciones pendientes de kernels/VRAM posteriores y de la cadena Ampere GA107.
WiFi distingue AX200 gen2 y AX211 gen3. Los cambios locales actuales afectan
precisamente al arranque gen2 y a FWSEC/falcon/GSP. El parser host de firmware no
demuestra que haya ALIVE, asociación, DHCP o tráfico en esos dispositivos.

**Trabajo.** Crear una matriz versionada: equipo, PCI ID, firmware y hash,
commit más identificación de cambios locales, perfil de drivers, resultado,
fecha y log. Secuencia WiFi: ALIVE real → scan → asociación/WPA2 → DHCP → SSH →
reconexión. Secuencia GPU: GSP/RPC → pool VRAM → CE/readback → resultado numérico
CPU/GPU → carga real → apagado limpio. Registrar como pendiente cada etapa no
ejecutada; limitar los reintentos de firmware y conservar un arranque útil por
CPU/Ethernet cuando falle un componente opcional.

**Aceptación.** Tres arranques consecutivos por equipo probado y una sesión
sostenida con consola, almacenamiento y red. GPU: apagado con liberación de
objetos y DMA, además del cómputo correcto. Las pruebas VFIO quedan para una
sesión de hardware preparada; el análisis de este documento no las ha ejecutado.

Para rendimiento, usar primero modelos sintéticos existentes y un modelo real
pequeño ya disponible, con prompt/seed/contexto y número de tokens fijos. Guardar
mediana de tres ejecuciones, tiempo hasta primer token, decode, E/S, memoria y
contadores de uso real de GPU; separar frío/caliente y CPU/GPU. Usar reloj host o
contadores TSC para E/S, porque el uptime basado en ticks puede subcontar durante
polling. Solo entonces elegir una optimización. No fijar un objetivo de tok/s
para modelos de 70B sin una medida del equipo objetivo.

## A9. Reducir instrucciones contradictorias

**Evidencia.** [README.md](README.md) mezcla instrucciones `sudo cargo` con el
procedimiento de la skill que conserva el entorno de Rust; `board.txt` contiene
rutas de disco concretas, notas de sesiones y texto mal codificado. Las skills
incluyen conclusiones históricas —por ejemplo, aceptar dos fallos conocidos en
la suite— que deberán cambiar al corregir el arnés. El plan de modelos grandes
conserva como diagnóstico inicial limitaciones que ya se resolvieron.

**Trabajo.** Mantener una entrada breve de estado actual y una guía operativa en
español. Archivar notas de sesiones como histórico; sustituir ejemplos de disco
por parámetros que el operador identifique. Explicar límites de soporte por chip
y de recuperación OTA. Editar skills en `.claude/skills`, compartidas por los
enlaces de Codex y Cursor, y actualizar `MANUAL-USUARIO.md` con cada cambio visible.
No reescribir todas las notas históricas ni duplicar el manual en cada skill.

**Aceptación.** Un recorrido documentado de compilar → QEMU → diagnóstico → live
usa comandos coherentes con `xtask`. La versión y el estado de soporte se derivan
de fuentes identificadas. A1 elimina la recomendación de considerar «limpia» una
suite con fallos, cuando exista evidencia de su corrección.

## Secuencia de ejecución propuesta

| Periodo | Resultado esperado |
|---|---|
| Semana 1 | A1 y A2 cerrados; A4; iniciar A3. Las pruebas vuelven a aportar una señal interpretable y OTA deja de ocultar fallos |
| Semana 2 | A3 y A5; manual/skills actualizados; diseño y estimación revisada de A6 |
| Semana 3 | A6 como prioridad para equipos instalados, o A7 si prima inferencia; primeras pruebas A8 |
| Semana 4 | Completar la entrega seleccionada, validar en hardware disponible y dejar la siguiente preparada con pruebas de reproducción |

Antes de empezar cada entrega, registrar el estado de los cambios locales que
afecten a su dominio. No atribuir un fallo a esos cambios sin comparar evidencia.
Cada entrega termina con pruebas apropiadas y documentación; las ejecuciones
repetidas se reservan para carreras, recuperación o hardware que lo requieran.

## Trabajo que aplazaría

- Nuevas familias de modelos, kernels GPU adicionales y más offload sin una base
  de corrección y medidas comparables del camino actual.
- Ampliación general a más GPUs/NIC antes de cerrar la matriz GB205/GA107 y
  AX200/AX211 disponible.
- Refactorización masiva de `task/mod.rs` o `xtask/src/main.rs`: extraer solo las
  piezas necesarias para propiedad de recursos, estados y pruebas de este plan.
- Multiusuario, permisos Unix, IPv6, fork, SFTP o snapshots públicos de sosofs:
  no forman parte del alcance inmediato declarado por el proyecto.

## Condición de cierre del ciclo

Una candidata a release debe tener A1–A5 verificadas, limitaciones de OTA
publicadas y resultados de los escenarios QEMU relevantes archivados. Si se
presenta como recuperable ante cortes, A6 también debe estar cerrada. Solo se
declara soporte probado para las combinaciones de hardware con evidencia A8.
Los pendientes quedan identificados por escenario y prueba, no por un «funciona»
genérico ni por la mera presencia del código.
