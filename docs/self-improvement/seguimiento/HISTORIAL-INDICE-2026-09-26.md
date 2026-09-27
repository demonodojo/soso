# Historial del índice anterior a la revisión del 26-sep-2026

Texto preservado del checkout antes de replanificar. Contiene estados y
recomendaciones de distintas fechas; **no es una guía operativa vigente**.
Consultar [README](../README.md) y los seguimientos por tarea.

## Por dónde empezar

**[T01](../T01-base.md), [T02](../T02-banco.md) y [T03](../T03-perfil-modelo.md) están
completadas** (15–16 de septiembre de 2026; resúmenes en
[seguimiento/](../seguimiento/)). T03 encontró que el tokenizer de soso **no
reproduce la segmentación oficial del modelo** y dejó dos fichas previas a T06;
el perfil del modelo está en [modelo.md](../modelo.md). **[T52](../T52-tokenizer-merges.md) y [T53](../T53-tokenizer-bpe.md) están
completadas**: soso segmenta ya **exactamente igual** que el tokenizer oficial
del modelo (5/5 fixtures). **[T04](../T04-dominio-chat.md)**, **[T05](../T05-validacion-chat.md)** y
**[T06](../T06-render-chat.md)**–**[T12](../T12-respuestas-sse.md)** están
completadas (dominio chat + render + parse + informe + cancelación + API JSON + HTTP + respuestas SSE).
**[T13](../T13-servidor-host.md)** (servidor de desarrollo host) está **completada** (2026-09-22;
[seguimiento/T13.md](../seguimiento/T13.md)). **[T15](../T15-sesion-residente.md)** (sesión residente guest) está **completada** (2026-09-22;
[seguimiento/T15.md](../seguimiento/T15.md) … [seguimiento/T19.md](../seguimiento/T19.md)). **SI-2 (T16–T19) tiene ya su evidencia guest**: el 23-sep-2026 `cargo xtask test-llm-api` pasó **12/12 invariantes contra soso en QEMU con los pesos reales** (`guest_ok`, exit 0). Llegar ahí costó cuatro defectos reales, con ficha cada uno: **[T54](../T54-accept-externo.md)** (el `accept` de userspace era loopback puro), **[T55](../T55-accept-sin-plazo.md)** (`tcp_accept(fd,0)` dormía al servidor para siempre), **[T56](../T56-utf8-tool-parser.md)** (el parser partía caracteres UTF-8 y mataba el proceso) y **[T57](../T57-medio-cierre.md)** (el medio cierre del cliente tiraba la respuesta). **[T14](../T14-evaluacion-modelo.md) dio NO-GO**: **8 de 10** casos de protocolo sólidos frente a un umbral de 10, con **0 inestables** (el modelo es reproducible a temperatura 0). La investigación de **[T59](../T59-tool-calls.md)** (24-sep) cerró la atribución: **el no-go es del modelo**. El Qwen2.5-Coder-3B no usa las herramientas en ninguno de los dos casos que el banco ejercita —en Q04 repite la llamada con el resultado delante; en Q07, con `tool_choice: "required"`, no emite ninguna—, mientras que el render es correcto y el servicio detecta la violación. Los otros dos casos ya pasan: uno era un defecto real del servicio (**[T60](../T60-validacion-peticion.md)**, cerrada) y el otro, del propio arnés. Con este modelo el umbral de 10/10 **no se alcanza**, y **todo lo accionable está hecho**: la decisión siguiente es de plan —probar otro modelo con el mismo banco, sin tocarlo durante la comparación, o acotar SI-3 a lo que un agente sin herramientas pueda hacer—. Presupuestos medidos: mediana 60,7 s por tarea, máximo 278 s. **Un no-go no habilita a [T16](../T16-servicio-guest.md) ni a [T22](../T22-primera-mejora.md)**, así que **SI-2 no cierra**: fichas recomendadas ahora, T59 y T60, y repetir la campaña al cerrarlas. Criterio de la evaluación en [evaluacion.md](../evaluacion.md).
**Camino SI-4**: **[T23](../T23-estado-coordinador.md)** (formato de tareas y
estados), **[T63](../T63-generacion-rota-encalla.md)** (el corte que encallaba la
referencia durable) y **[T24](../T24-checkout.md)** (copia de tarea y exportación
de su parche) están **completadas** el 24-sep-2026, las tres con su sonda en
guest. Con eso el coordinador ya sabe llevar el estado de una tarea y preparar
y recoger el trabajo de un candidato **sin Git**.

De T23 salieron las decisiones que después no se pueden cambiar: sólo el
validador escribe «aceptada», lo que no se midió se informa con motivo —nunca
como cero— y en el estado van referencias, no logs. De T24, que lo que siembra
el coordinador no cuenta como cambio del candidato y que un enlace simbólico se
informa en vez de desaparecer del parche.

**Y aquí la cadena se para.** [T25](../T25-ejecutor.md) necesita
[T21](../T21-opencode-contrato.md), que necesita [T20](../T20-opencode-config.md), que
depende del **no-go de [T14](../T14-evaluacion-modelo.md)**; y
[T26](../T26-validador.md)–[T29](../T29-campana.md) van detrás de T25. El hito SI-4 no
avanza más sin la decisión de plan que T14 dejó sobre la mesa: probar otro
modelo con el mismo banco, sin tocarlo durante la comparación, o acotar SI-3 a
lo que un agente sin herramientas pueda hacer.

**[T62](../T62-argv-en-los-programas.md) está cerrada** el 24-sep-2026, y resultó
ser de **cinco** sitios y no de uno: además de `entry!`, empaquetaban la línea
entera como un solo argumento `sosh`, `libsoso::spawn_io` y dos caminos del
kernel. El compilador encontró los 39 consumidores; los cinco productores no,
porque ahí no cambia el tipo, y salieron en QEMU disfrazados de otra cosa
—conexión rechazada, registros YMM, un HTTP 500 de Forja—. Con eso el «argv,
nunca una línea de shell» de C5 es cierto de punta a punta. Y
**[T64](../T64-sosh-comillas.md)** cierra el otro extremo el mismo día: `sosh`
entiende ya `'…'`, `"…"` y `\`, así que una ruta con espacios se puede
**escribir** además de pasar.

**[T32](../T32-opencode-inventario.md) está cerrada** el 24-sep-2026 con la
revisión de OpenCode fijada (v1.18.32, commit `545f51d2…`) y su matriz de
capacidades. Dos hallazgos cambian lo que SI-6 puede planear: el repositorio
**se movió** a `anomalyco/opencode`, y **`opencode run` no es un modo sin TUI**
—importa `@opentui/*` en el módulo, y lo no interactivo es una rama dentro de
él—. El obstáculo de fondo no es la lista de paquetes sino tres cosas: **no hay
`dlopen`** (ningún `.node` es cargable, ni recompilándolo), no hay pty ni
`inotify`, y **portar OpenCode es portar Bun**. Las incógnitas quedan escritas
como siete sondas para [T33](../T33-sondas-abi.md).

**[T33](../T33-sondas-abi.md) está completada** el 24-sep-2026: **siete sondas**
en un crate nuevo (`user/soso-agent-probe/`), ejecutadas en siete pases —uno por
capacidad, como la ficha exige— con la tabla en
[`native/probes.json`](../native/probes.json), que lleva `esperado` y `observado`
por caso también en los que pasan. Hay además un `cargo xtask test-probe` que
arranca la imagen **dos veces** con un `halt` en medio, porque «sobrevive a
cerrar el descriptor» y «sobrevive al apagado» no son lo mismo.

**Lo que sale bien, y no era obvio**: páginas ejecutables **con recompilación**
—el JIT es viable—, señales suficientes para el timeout de una herramienta,
stdout y stderr separados, hilos con `join` que espera, relojes que no mienten,
tuberías que aguantan 64 KiB y avisan del EOF cuando el escritor muere, y TCP
que reconecta.

**Lo que sale mal**, con reproducción y archivos responsables: **el modelo de
ficheros es por descriptor, no por inodo** —dos descriptores no comparten el
fichero y una escritura parcial trunca la cola, así que SQLite no puede
funcionar, y no por falta de `pread`—; y **no hay protecciones de página más
allá de la escritura** —ni `PROT_EXEC` porque todo es ejecutable, ni `PROT_NONE`
así que no hay guarda de pila—, que es **una** decisión de diseño con los dos
signos. De ahí salen dos defectos abiertos, **[T65](../T65-o-excl-no-excluye.md)**
y **[T66](../T66-guarda-de-pila-fingida.md)**, y cinco huecos listos para
**[T34](../T34-tickets-port.md)**, que es la recomendada ahora.

**[T34](../T34-tickets-port.md) está completada** el mismo día, con
[`native/backlog.json`](../native/backlog.json) —11 fichas N-xxx, grafo acíclico—
y las tres primeras redactadas. **La ruta elegida es «sustrato POSIX primero»**:
un programa escrito contra POSIX tiene que encontrarse POSIX, y lo que T33 midió
que no lo es no se arregla en el runtime. Portar Bun queda descartado por cuatro
carencias medidas —sin `dlopen` ningún `.node` es cargable, faltan pty e
`inotify`, y el modelo de ficheros impide SQLite—.

**Y queda una decisión de plan sobre la mesa, como la de T14**: el backlog anota
que la alternativa a portar es **un agente nativo en Rust que reutilice el
protocolo** — que es lo que el plan ya está construyendo (`soso-llm`,
`conversation`, `soso-improve`) y lo que las sondas de T33 dicen que ya
funciona. Elegir entre las dos no es de esta ficha.

**[N-001](../native/N-001.md) tiene su paso 1 hecho**: la ficha exigía **medir
antes de elegir**, y la medida dice que la opción que parecía simple —cada
`write` al VFS— cuesta **~75×** más (64 KiB en una transacción frente a 64, con
`rdtsc` y no con el PIT, que subcuenta durante el polling de disco). Elegida la
**caché por inodo**, que conserva el agrupado y resuelve lo que N-001 persigue.
La intuición escrita en la ficha era la contraria y sólo se vio al medir.

**[N-002](../native/N-002.md) está hecha** el mismo día: `mprotect` acepta
`prot == 0` y **los hilos de soso tienen guarda de pila**. La pieza que lo hizo
barato es que el mmap es perezoso — la guarda no cuesta ni un marco, porque
basta con que el manejador de faltas se niegue a mapearla. El caso que lo
acredita es el que mide que **tocarla mata al proceso**, con un hijo, porque el
que la toca no vuelve a contarlo. De rebote cerró lo que T66 dejó preparado:
`hay_guarda_de_pila()` pasó a `true` sin tocar una línea.

**[T65](../T65-o-excl-no-excluye.md) también está cerrada**: `O_CREAT|O_EXCL`
reserva el nombre **al abrir** en vez de al cerrar, así que la ventana en la que
ganaban los dos ya no existe. Encaja con [T63](../T63-generacion-rota-encalla.md):
aquélla enseñó a soso a seguir tras un corte, y ésta hace que la exclusión sea
real — las dos sostienen el contrato durable de
[T46](../T46-archivos-durables.md), que es la única exclusión mutua que hay.

**[N-003](../native/N-003.md) está hecha**: el spawn acepta directorio de trabajo,
así que un agente puede lanzar dos herramientas a la vez en sitios distintos sin
mover el suyo. La compatibilidad la resolvió un precedente que ya estaba escrito
en el ABI —el comentario de `log_fd`—: el cero tiene que significar la conducta
de antes, y aquí sale gratis. De paso se borró el `CwdGuardado` de T47.

**[N-001](../native/N-001.md) está cerrada**: el modelo de ficheros pasa de ser
**por descriptor** a ser **por inodo**. Con eso los ocho casos de la sonda de
SQLite pasan — dos descriptores se ven, una escritura parcial no trunca la cola
y dos escritores no se pisan—, y el coste vuelve a su sitio (~84×, igual que
antes de empezar), porque el segundo que abre se encuentra el contenido ya
cargado. El riesgo que la ficha declaraba se respetó: **`StreamWrite` no se ha
tocado**, así que el camino de crear ficheros y `/var/models/` siguen igual.

**[N-004](../native/N-004.md) está hecha**, y sólo tenía sentido ahora: con el
modelo por descriptor, un candado sobre un fichero que cada proceso ve por su
cuenta no habría coordinado nada. `flock` consultivo, del fichero entero y
**sólo no bloqueante** — sin `LOCK_NB` devuelve `ENOSYS` en vez de fingir que
esperó, porque quien pide un cerrojo bloqueante y recibe uno que no bloquea
corre sin saberlo. El cerrojo es del proceso: cerrar un fd no lo suelta, morir
sí. Se prueba con **dos procesos**, que es la única forma de demostrar que el
otro no puede cogerlo.

**[N-005](../native/N-005.md) está decidida** — es una ficha de experimento, y su
resultado binario es **enlazado estático**. C escrito para la ocasión,
compilado hacia el target de soso y enlazado en un programa, corre; un ELF que
no sea `ET_EXEC` no arranca, medido con un delta de **un byte** sobre un
control que acababa de arrancar. De propina, `nm -u` dice que el C de `ring`
sólo pide tres símbolos externos (`memcpy`, `memset`, `__stack_chk_fail`) — el
**suelo** de N-006, no su techo, porque `ring` es freestanding. Con eso queda
dicho que `dlopen` no hace falta: los `.node` de OpenCode no son código que
haya que cargar en caliente, son binarios de otro sistema operativo, y
recompilarlos es un problema de libc.

**[N-009](../native/N-009.md) está hecha**, y se eligió **por delante de N-006**
con el motivo escrito: no hay ningún consumidor de C en soso, así que una capa
libc hoy sería elegir funciones adivinando — mientras que buscar en el código
hace falta tanto si el agente se porta como si se reescribe en Rust. Lo que
estaba roto no era que el `grep` de soso fuera limitado: era que **no lo
decía**, y podía mentir de tres formas —«no hay» por «no pude», una expresión
regular buscada tal cual, y una línea no-UTF8 saltada en silencio—. Ahora los
códigos de salida separan 0/1/2, un patrón que sólo tiene sentido como regex se
**rechaza** (y `-F` lo fuerza), y se busca sobre bytes. Con `-n`, `-r`, `-l`,
`-i` y `-m`.

**[N-010](../native/N-010.md) está hecha**, y es la misma lección que N-009 en otro
sitio: `sosh` no es bash, y lo que faltaba no era parecerse más sino **decir en
qué no se parece**. Antes `;`, `&`, `&&`, `*` y `$` se colaban como argumentos
del comando — `echo dos ; echo tres` imprimía «dos ; echo tres» y el segundo
comando **no se ejecutaba**, y `ls *.rs` contestaba «no existe», que suena a un
hecho sobre el disco. Ahora cada uno se rechaza diciendo qué hacer en su lugar,
con entrecomillar como salida de emergencia. La línea base se midió **antes**
de tocar nada, escribiendo el paso con la conducta deseada y ejecutándolo
contra el `sosh` viejo.

Y el rechazo resultó ser el paso intermedio, no el final: **encadenar (`;`,
`&&`, `||`) se implementó el mismo día** y la suite lo comprueba. Declarar el
subconjunto fue lo que hizo visible el hueco — antes nadie veía que media línea
se perdía—, y con eso delante implementarlo fue una decisión informada. Lo mismo con **`2>&1`**, implementado después: resultó ser el más barato de los
cuatro, porque en soso el hijo recibe un array de descriptores y duplicar es
pasar el mismo número dos veces. Siguen rechazados, y ahí el rechazo es el
entregable: `&`, `*` y `$`.

**[N-008](../native/N-008.md) tiene la forma decidida**: si hace falta vigilar
ficheros, tiene que ser **por eventos**. Lo decidió la corrección, no el coste
que había salido a medir: dos escrituras del **mismo tamaño** dentro del mismo
segundo dejan `stat` idéntico —**6 de 6 pares indistinguibles**—, así que
aunque `stat` fuera gratis el sondeo seguiría sin verlas. La medida casi se
queda en anécdota: una ejecución anterior dio «se distinguen» porque las
escrituras cayeron a caballo de un segundo, y por eso el caso pasó a informar
de una **tasa**. La implementación se aplaza —el único consumidor conocido es
`@parcel/watcher`, de la rama de portar—, pero la forma ya está decidida con
datos, que es lo que la ficha pedía. De rebote salió
**[N-012](../native/N-012.md)**: `stat` cuesta ~3 M de ciclos **por componente de
ruta**, y lo paga todo lo que toca rutas.

**[N-012](../native/N-012.md) está cerrada, 20,5×** — un `stat` de cinco
componentes pasó de 17,1 M a **833 k** de ciclos, y el coste **por componente
de ruta**, que es lo que daba título a la ficha, de ~3 M a ~36 k (**83×**) — y
lo que enseñó vale más que la cifra.
Cuatro controles, y **dos me estaban mintiendo**: los tests del host corrían en
`debug` mientras el guest lleva `sosofs` a `opt-level = 3` (comparaba debug
contra release; en release el host cae 12–17×), y los microbancos en release
daban **0 ns** porque el optimizador se los había comido enteros. QEMU va con
**KVM**, así que no había emulación a la que culpar, y el mínimo de 32 vueltas
coincide con la media, así que tampoco era el planificador. Con eso, los
arreglos: `lookup` ya no vuelca el directorio entero (la clave del dirent ya era
`name_hash`), y `read_node` ya no pide 4 KiB de montón por nodo. Pero el trozo
grande **no es mío y no lo toco**: `revisar()` audita el montón del kernel en
cada `alloc` y cada `dealloc` recorriendo 4096 ranuras, y explica el **~75 %**
del coste — medido comentándolo y revirtiendo byte a byte. Es un detector de
desbordamientos con una caza de pánicos en marcha, así que es
**[N-013](../native/N-013.md)**: el número y cuatro opciones, y la decisión de
quien depura. Con eso medido, la pregunta útil deja de ser «¿qué hace lento
este código?» y pasa a ser «**¿cuántas veces pide memoria?**» — una reserva son
**~243 000 ciclos**, y quitar seis de ellas del camino de `stat` valió tanto
como todo lo anterior junto. La más ancha ni siquiera estaba en el sistema de
ficheros: `resolve_user_path`, que paga **toda** syscall con ruta, hacía cinco
reservas y ahora hace una. Lo había descartado como «ficha aparte porque toca
el ABI» — lo que toca el ABI es cambiar el tipo de retorno, no quitar reservas
de dentro. Se cierra con el reparto medido: de los 263 k que cuesta hoy un
`stat("/")`, el mecanismo de llamada es **2 635 ciclos** (el 1 %), el árbol
~17 k, y **~243 k son la única reserva que sobrevive**. Dentro del sistema de
ficheros no queda nada grande; lo que queda es N-013.

**[T36](../T36-forja-trazabilidad.md) está hecha** (2026-09-25): `manifest.txt` deja
de ser una etiqueta y pasa a ser un **recibo versionado** que liga fuentes,
build y artefactos. Tres cosas estaban mal y las tres eran del mismo tipo —
algo que *parecía* comprobar y no comprobaba—: el `build-id` salía de
`DefaultHasher` (que no promete estabilidad entre versiones ni plataformas), el
campo `sources=` contenía el hash **del pack** en vez del de las fuentes, y el
cliente escribía el staging **antes** de mirar el manifiesto. Ahora `sync`
guarda lo que envió y `build` verifica versión, build-id, fuentes y el sha256
de lo descargado **antes** de escribir nada — porque del staging se aplica.
Resumen en [resultado.md](../../../target/self-improvement/tasks/T36/resultado.md).

**[T38](../T38-toolchain-inventario.md) está hecha** (2026-09-25), y aplicar su
propia regla —«ninguna herramienta marcada nativa solo por su nombre o
`--version`»— cambió el estado de tres. `sosoas` se anuncia como ensamblador y
rechaza ensamblador real; el objeto que sí emite tiene la **cabecera ELF
desplazada dos bytes**, y cada síntoma de `readelf` mapea a una escritura
concreta ([T67](../T67-sosoas-elf-desplazado.md)). `wild-soso` declara su binario
como `wild`, así que las dos rutas que lo buscan como `wild-soso` no existen
nunca, y si su directorio entra en el PATH el envoltorio se llamaría a sí mismo
([T68](../T68-wild-soso-nombre.md)); `wild` no estaba instalado —**sí lo está
desde hoy: 0.10.0**, en `~/.cargo/bin`—. Y el sysroot
de `x86_64-unknown-soso` **no se ha construido**. Ninguno de los tres se había
notado porque ninguno participa en el camino que hoy funciona: todo lo que
arranca sale de `rustc` + `rust-lld` sobre los targets de `user/`. Entrega en
[toolchain-lock.json](../native/toolchain-lock.json) y
[toolchain-deps.md](../native/toolchain-deps.md).

**[T67](../T67-sosoas-elf-desplazado.md) y [T68](../T68-wild-soso-nombre.md) están
hechas** el mismo día, y las dos resultaron ser más anchas de lo documentado.
En `sosoas`, los **section headers** iban ocho bytes corridos además de la
cabecera, y `sh_name`/`sh_type` no se escribían nunca: no se había visto porque
`readelf -S` se paraba antes, en el `e_shentsize` roto — un síntoma tapaba al
otro. Hoy `objdump -d` desensambla los bytes que entraron. En `wild-soso`, el
nombre arrastraba **dos rutas más** que tampoco existen: el bootstrap y `xtask`
apuntaban a `tools/*/target/release`, que nunca se crean porque los dos crates
son miembros del workspace, y el bootstrap compilaba sin `--release`.

Los dos arreglos llevan tests que **no dependen de binutils** —la toolchain
nativa tiene que poder comprobarse a sí misma— y en los dos se verificó por
mutación que pueden fallar. El de `wild-soso` ata el nombre del binario al
`"linker"` del target: no vigila el bug, vigila la **deriva** que lo produjo.

**[T39](../T39-bootstrap-libstd.md) encontró que el bootstrap no podía
completarse.** El `sed` que parchea `os/mod.rs` tiene los paréntesis
desbalanceados —`\)` sin `\(` en una expresión básica—, así que **falla en
cualquier entrada**, y con `set -euo pipefail` abortaba `apply-patches.sh`
justo ahí: los cuatro parches siguientes no se ejecutaban nunca en un vendor
limpio. El vendor real lo confirma: `os/mod.rs` no tiene «soso» por ninguna
parte mientras los otros seis parches sí están. Arreglado y comprobado de punta
a punta contra un **vendor falso** hecho con copias de los ficheros reales
—sin tocar el del usuario—: el script completa, los parches aplican, y tres
pasadas dejan los ficheros **byte a byte iguales**, que es la propiedad que la
ficha exige. De paso salió [T69](../T69-apply-patches-no-completaba.md): el script
copia `dl.rs` a una PAL cuyo `mod.rs` no lo declara.

**Y va por los pasos 1–2 de 5**, con la receta ya enchufada en el host:
`soso-improve receta comprobar --vendor … --plantillas …` dice qué parches
faltan **sin tocar el vendor**, y sobre el real señala a la primera los dos de
`os/mod.rs`. Distingue «falta» (código 1) de «ancla rota» (2), porque el
segundo no se arregla repitiendo la preparación. `aplicar` existe y está
probado en el núcleo pero **no se expone todavía**: sin poder compilar libstd
no hay forma de validar que la versión Rust deja el vendor igual que el
script, y dos formas de escribir sin comprobarlas es cómo aparecen dos
verdades. También corre **dentro de soso**: la suite lo acredita apuntando a un
directorio vacío, que da una forma determinista —2 por aplicar, 8 con el ancla
rota— y prueba despacho, capacidad e informe sin fingir un vendor que en el
guest todavía no existe. Lo que la motiva
está comprobado, no supuesto: `sed -i` y `perl -i` **salen 0 cuando el ancla no
aparece**, así que `apply-patches.sh` imprime `OK` habiendo parcheado **cero**
si upstream renombra cualquier cosa — y el fallo sale mucho después como un
error de compilación que no señala a la causa. Ya existe
`soso_improve_core::receta`: declarativa sobre el trait `Archivos` (así podrá
correr dentro de soso), idempotente por una marca **explícita** en vez de por
una subcadena adivinada, y con un ancla ausente como **fallo con nombre**. Ocho
tests.

Los pasos **4–5 siguen sin hacerse**, pero ya no por falta de enlazador: `wild`
0.10.0 está instalado (`cargo install --locked wild-linker`) y enlazó un ELF64
estático que salió con 0. Lo que falta es construir libstd para el target soso
y ejecutar ese programa en el guest.

**[T20](../T20-opencode-config.md) no estaba habilitada**, aunque sus dependencias
figuren `done`: [T14](../T14-evaluacion-modelo.md) cerró con **NO-GO**, y la propia
skill advierte que un informe no-go marcado `done` no habilita a su consumidor.
Un filtro que mira `status` y no el veredicto lo da por listo — el mismo error
de leer la etiqueta en vez del contenido.

**La campaña se repitió el 2026-09-25** con el arnés nuevo
(`cargo xtask test-llm-api --campana`), 10 casos × 3 repeticiones en 55,6
minutos: **8/10, sigue NO-GO**, 0 inestables. Salió exactamente lo que se había
escrito **antes** de correrla, y eso es lo valioso: **T60 queda confirmada a
nivel de sistema** —Q08 y Q10 pasan; estaba cerrada sólo con pruebas
unitarias— y el techo lo pone el modelo, como dijo T59. La evidencia es
literal: en las tres repeticiones Q04 devuelve
`content: "{\"name\": \"obtener_hora\", …}"` con `finish_reason: "stop"`. El
modelo **sí** decide llamar y **sí** compone el JSON correcto; lo emite por el
canal equivocado.

Presupuestos nuevos: mediana **50 058 ms**, máximo **287 100 ms**, timeout
sugerido **574 200 ms**, 22 tokens.

Así que lo que falta **no es una corrección pendiente, es una decisión**:
modelo mayor, parsear la llamada en el servicio y declararlo como capa de
compatibilidad, o un agente nativo que no dependa de `tool_calls` (la rama que
el backlog de T34 ya tiene anotada). Las tres parten de **8/10 medido**.

**El recibo de Forja va por la v2** (2026-09-25) y con eso se cierra el último
límite que T36 se había dejado escrito: `perfil` daba el modo y nada más, y la
ficha remitía a T38 para los comandos. Ahora lleva `comando=` (literal) y
`herramienta=` con lo que responde cada `--version`. **Describen, no
acreditan**: el cliente no tiene contra qué compararlos, así que no los
verifica — decir lo contrario sería otro campo mentiroso, como el `sources=`
que T36 quitó. Un perfil de prueba dice `(ninguno: perfil de prueba)` en vez de
fingir un comando plausible, y la v1 sigue aceptándose porque lo que el cliente
verifica no cambió.

**Lo que queda**: casi todo decisiones, no implementación — las siete de
[DECISIONES.md](../DECISIONES.md), más [T73](../T73-futex-sin-plazo.md), que sí es
trabajo (kernel). El 25-sep se ejecutó por primera vez `cargo xtask rust-build-std`
—el comando de la propia Comprobación de T39, que nunca se había corrido— y salieron
**siete** defectos en cadena, cada uno tapando al siguiente, todos cerrados
([T71](../T71-vendor-sin-cache.md) es uno de ellos, con ficha propia). Ahora `core`, `alloc`,
`compiler_builtins`, `soso-abi` y `soso-rt` **se construyen** para `x86_64-unknown-soso`, y lo que
faltaba era escribir la PAL. El 25-sep se escribió: `sys/{io/error,args,env,random}/soso.rs`, TLS nativo
declarado en el target y el `guard` en la rama de hermit/xous. **`std` bajó de 79 errores a 2**. [T73](../T73-futex-sin-plazo.md) —el plazo que le
faltaba al futex del kernel— se cerró el 26-sep acreditada en QEMU, y con ella los cuatro de
sincronización. **Los 2 que quedan son [T72](../T72-heap-de-libstd.md), que es una decisión (D7):
de dónde sale el montón.** Es lo único entre el plan y una libstd de soso que compila. Eso contesta además la pregunta de
[T69](../T69-apply-patches-no-completaba.md): lo que faltaba no era `dl`, sino `io` y `thread_local`.
En paralelo siguen habilitadas **T18**, **T45** y **T46**. Los cierres
históricos de T01/T02 no acreditan aún su validación nativa completa.
Entregar al modelo una ficha por sesión, las secciones indicadas
del [contrato](../CONTRATO.md), el [contrato nativo](../NATIVO.md) y el contexto de
código que la ficha enumera. No necesita cargar todas las fichas.

Cada subplan define alcance, archivos, decisiones, pasos, pruebas, salida y
condiciones de bloqueo. Los nombres propuestos de módulos y comandos se crean
al implementar la ficha; los enlaces de contexto apuntan al código existente.

También pueden empezar sin otras tareas
**[T08](../T08-resultado-generacion.md)** (resultado de generación) y
**[T18](../T18-puertos-qemu.md)** (puertos QEMU). Trabajar una tarea cada vez por defecto;
varias fichas modifican los mismos manifiestos o módulos de integración.

### Prompt listo para copiar

```text
Implementa docs/self-improvement/T06-render-chat.md.

Lee las secciones del CONTRATO.md que indique la ficha, NATIVO.md y las instrucciones
locales aplicables. Revisa el estado del checkout antes de editar.
Comprueba las dependencias y trabaja solo dentro del alcance de esta ficha.
Usa los símbolos y los fragmentos de código pertinentes; no cargues todos
los planes ni todos los archivos grandes en tu contexto.

Implementa, ejecuta las comprobaciones y conserva sus salidas.
No declares éxito sin evidencia ni sustituyas una prueba de modelo real
por un mock. Si aparece una dependencia ausente, deja una reproducción
y una ficha concreta; no amplíes el alcance sin documentarlo.

Entrega diff, resultado de pruebas, artefactos y dependencias pendientes.
Actualiza solo el estado que puedas acreditar, incluida native_validation
por separado. Una prueba host o un ELF no demuestran ejecución guest.
No empieces la siguiente tarea.
```

Para continuar, sustituir la ruta por la ficha elegida y proporcionar la base,
los artefactos de dependencias y las entradas externas que requiera.

