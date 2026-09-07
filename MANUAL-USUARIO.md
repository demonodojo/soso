# Manual de usuario — soso

**soso** es un sistema operativo propio para x86_64: kernel en Rust, disco con
filesystem copy-on-write (sosofs), modelos de lenguaje en un segundo disco
(sosomfs), red real (ethernet, WiFi, SSH), reconocimiento de voz, un navegador
mínimo, instalación en NVMe y actualizaciones por red. Es **monousuario**: una
sola sesión SSH a la vez, sin permisos ni cuentas múltiples.

Puedes usarlo en **QEMU** (desarrollo), en un **pendrive live** en hardware real
o **instalado en un disco NVMe** junto a Linux (dual-boot UEFI).

Este manual describe cómo arrancar el sistema, conectarte y usar la shell y los
comandos disponibles. Para compilar el proyecto desde el código fuente, consulta
el [`README.md`](README.md) (en inglés). Guía operativa de desarrollo:
[`docs/GUIA-OPERATIVA.md`](docs/GUIA-OPERATIVA.md). Estado y límites de soporte:
[`docs/ESTADO.md`](docs/ESTADO.md).

---

## ¿Qué incluye soso?

| Área | Comandos / programas | Para qué sirve |
|------|----------------------|----------------|
| Shell | **sosh** | Consola con pipes, redirecciones, WiFi, `ask`, `voz` |
| LLM | **ask**, **soso-llm** | Preguntar al modelo; inferencia directa o modo interactivo |
| Modelos | **soso-hf**, **ask-modelo** | Descargar GGUF desde Hugging Face; elegir modelo por defecto |
| Voz | **voz**, **soso-voz** | Dictado por micrófono o fichero WAV (Whisper) |
| Web | **soso-web** | Leer páginas HTTPS o HTML local (consola o pantalla gráfica) |
| Instalación | **soso-install** | Copiar el live a un disco NVMe desde soso, sin Linux |
| Actualización | **soso-update** | Bajar e instalar releases nuevas (programas + kernel) |
| Desarrollo | **soso-ed**, **soso-forja** | Editar fuentes en `/src/soso`; bucle remoto o plan local (`docs/SELF-HOSTING.md`) |
| Utilidades | **cp**, **mv**, **grep**, **diff**, **find**, **wc**, **head**, **tail**, **stat** | Coreutils mínimas para editar y depurar en el guest |
| Red | **wifi** (builtin) | Escanear y conectar redes WiFi Intel en placa real |
| Sistema | **halt**, **exit** | Apagar o salir de la shell |

Al arrancar verás una línea como `soso 0.2.2 (6641119fd)` — versión del kernel
y build. La versión del disco está en `/etc/soso-release` (`soso-update estado`
la muestra junto al estado del buzón de actualización).

---

## Requisitos

En la máquina anfitriona (Linux) necesitas:

| Herramienta | Para qué sirve |
|---|---|
| **rustup** | Compilar el kernel y el userspace (nightly fijado en `rust-toolchain.toml`) |
| **qemu-system-x86_64** | Emular la máquina (`sudo apt install qemu-system-x86`) |
| **OpenSSH client** | Conectarte por SSH (`ssh`) |
| **ssh-keygen** | Generar claves ed25519 (normalmente incluido con OpenSSH) |

---

## Arrancar soso

### En QEMU (desarrollo)

Desde el directorio del proyecto en Linux:

```sh
cargo xtask run
```

Este comando compila el kernel, genera la imagen del disco con el contenido de
`rootfs/`, y lanza QEMU. La consola del sistema aparece en la misma terminal.

### En hardware real (pendrive live)

1. Genera y graba el live USB (desde el repo, en Linux):

   ```sh
   cargo xtask flash-usb-live /dev/sdX --yes
   ```

   Sustituye `/dev/sdX` por tu pendrive (¡comprueba bien el dispositivo!).
   El empaquetado incluye el mejor modelo GGUF que quepa en el stick.

2. Arranca la máquina desde el USB (menú UEFI / F12).

3. Espera el prompt `$` de **sosh** en pantalla o conéctate por SSH si hay red
   (DHCP en ethernet Realtek o WiFi Intel; ver [Red en hardware real](#2b-red-en-hardware-real-ethernet-realtek-y-wifi-intel)).

Para **instalar** soso en un NVMe del equipo, sigue la sección
[Instalar soso en un disco](#instalar-soso-en-un-disco-dual-boot-uefi). Para
**actualizar** una instalación existente, [soso-update](#soso-update--actualizar-soso-instalado).

Comandos relacionados (desarrollo en el repo):

| Comando | Descripción |
|---|---|
| `cargo xtask build` | Solo compila; genera `target/soso-bios.img` |
| `cargo xtask run` | Compila y arranca en QEMU |
| `cargo xtask gdb` | Arranca congelado en el boot; conectar con `gdb -ex 'target remote :1234'` |
| `cargo xtask test` | Batería automática de integración (FS, boot, TCP, SSH, apagado) |

**Salir de QEMU:** `Ctrl-A` seguido de `X`.

Al arrancar, el proceso `init` (PID 1) lanza automáticamente la shell de usuario
**sosh**. Verás el prompt `$` y el mensaje de bienvenida.

---

## Formas de acceso

soso ofrece tres interfaces:

### 1. Consola serie (terminal de QEMU)

Es la forma más directa. Al ejecutar `cargo xtask run`, interactúas con sosh en
la misma terminal donde corre QEMU.

En **placa real** (live USB, sin adaptador serie), la consola es el **framebuffer**
de la pantalla: el teclado integrado o USB alimenta la shell directamente.

#### Teclado y UTF-8 en consola física

El mapa de teclado por defecto es **español (ISO-105)**: `ñ`, `ç`, `¡`, `¿`, tildes
con teclas muertas (`´` + vocal → `á`, etc.) y **AltGr** para `@`, `#`, `€`, `|`, `{`, `}`…

La consola gráfica decodifica **UTF-8** y pinta un carácter por celda (incluidos
acentos y `€`). El eco de **sosh** y el REPL de `ask` también aceptan UTF-8 en
entrada.

Si usas un teclado americano o QEMU con layout US, cambia en la kernel-shell
(`soso>`):

```
kbd us
```

Para volver al mapa español:

```
kbd es
```

Por SSH el terminal del anfitrión pinta los caracteres; el mapa del kernel solo
afecta al teclado conectado a la máquina soso.

### 2. SSH (acceso remoto cifrado)

Con soso en marcha, abre **otra terminal** en el anfitrión:

```sh
ssh -tt -i target/soso_test_key -p 2222 soso@localhost
```

- **Puerto:** 2222 (redirigido al puerto 22 interno de soso).
- **Usuario:** `soso` (el nombre es convencional; la autenticación es solo por clave).
- **Cifrado:** SSH-2 con curve25519, ed25519 y chacha20-poly1305.

Al conectar verás el mensaje del día (`/etc/motd`) y luego la misma shell **sosh**
que en la consola serie. Para **cerrar la sesión** usa `exit` o cierra la terminal
del cliente; solo hay **una sesión SSH a la vez**. Con `ssh -tt`, **Ctrl-C** durante
un comando interrumpe ese comando (como en la consola serie), no cierra la sesión.

**Clave de acceso:** al construir la imagen, se inyecta una clave pública ed25519 en
`/etc/authorized_key` del disco:

- Si existe `~/.ssh/id_ed25519.pub`, se usa esa clave.
- Si no, se genera un par de prueba en `target/soso_test_key` (y su `.pub`).

Para usar tu propia clave SSH, asegúrate de tener `~/.ssh/id_ed25519.pub` antes de
compilar, o regenera la imagen con `cargo xtask mkfs`.

> **Nota:** la primera conexión puede avisar de una host key desconocida. La host key
> del servidor se guarda en `/etc/ssh_host_key` dentro del disco y persiste entre
> reconstrucciones de la imagen si no borras el disco de datos.

### 2b. Red en hardware real (ethernet Realtek y WiFi Intel)

El **USB live** incluye ethernet **Realtek RTL8111/8168** (`rtl8169`, el mismo
chip que Linux cubre con `r8169`) y WiFi **Intel AX211** (`8086:7f70`) y
**AX200** (`8086:2723`) vía `iwlwifi`.
Enchufa un cable en el RJ45 o configura WiFi; tras DHCP, SSH escucha en el
**puerto 22** (no 2222 — ese es solo QEMU).

**Configuración WiFi** (elige una):

1. **Desde sosh** (recomendado cuando ya arrancó):

```sh
wifi scan
wifi status
wifi connect MiRed
wifi connect MiRed MiClaveWPA2
```

Tras asociar, soso pide DHCP. SSH queda en el **puerto 22**.

2. **En el pendrive, antes de arrancar:** edita `SOSOWIFI.TXT` en la ESP (partición 1
   FAT). El kernel se conecta solo al boot. No hace falta regenerar la imagen.
3. **En rootfs:** `/etc/wifi.conf` (se empaqueta al flashear).

```ini
ssid=MiRed
psk=MiClaveWPA2
```

Firmware en `/lib/firmware/iwlwifi-so-a0-gf-a0-89.ucode` y `.pnvm` (AX211),
o `iwlwifi-cc-a0-77.ucode` (AX200, sin pnvm).

**SSH en placa** (IP del log `net: dhcp …`):

```sh
ssh -i target/soso_test_key soso@<ip>
```

**Consola de emergencia (kernel-shell):**

```text
wifi scan          # listar redes
wifi status        # estado del driver
wifi connect Red   # red abierta
wifi connect Red clave  # WPA2-PSK
```

**Flashear live:**

```sh
cargo xtask flash-usb-live /dev/sdX --yes
```

**Prueba VFIO** (AX211 passthrough a QEMU):

```sh
sudo ./scripts/l6-wifi-vfio-test.sh
```

WiFi tiene prioridad sobre virtio-net si no hay Ethernet. En WiFi no hay fallback
`10.0.2.x` (solo lease DHCP real).

### 3. Echo TCP (prueba de red)

Servicio de prueba en el puerto 7 interno (7777 en el anfitrión):

```sh
nc localhost 7777
```

Todo lo que escribas se devuelve tal cual (eco). Útil para comprobar que la red
funciona; no es una shell interactiva. La IP se obtiene por DHCP al arrancar; en
QEMU, si no hay servidor DHCP, se usa `10.0.2.15` como respaldo.

---

## La shell de usuario: sosh

**sosh** es la shell principal de soso. Cada línea que escribes se interpreta como
un comando.

### Prompt

```
$ 
```

### Comportamiento

- **Una línea = un comando o un pipeline.** Puedes encadenar comandos con `|`
  y redirigir la entrada o salida con `<`, `>` y `>>`.
- **Directorio de trabajo (cwd):** cada shell tiene un cwd (inicialmente `/`).
  Las rutas sin `/` inicial son relativas al cwd (p. ej. `echo x > f.txt` en
  `/tmp` crea `/tmp/f.txt` tras `cd /tmp`).
- No hay variables de entorno ni historial de comandos.
- Los comandos sin ruta se buscan en `/bin/`.
- También puedes invocar un ELF por ruta absoluta (por ejemplo `/bin/init test`).
- **Backspace** funciona para corregir la línea.
- **Ctrl-C** interrumpe el comando en primer plano (p. ej. un pipeline largo) y
  devuelve el prompt `$`; en la línea vacía muestra `^C` y sigue en sosh.
- **Ctrl-D** en una línea vacía cierra la shell (equivalente a `exit`).
- Si un comando falla, sosh muestra el código de salida.

### Pipes y redirecciones

| Operador | Significado |
|---|---|
| `cmd1 \| cmd2` | La salida de `cmd1` es la entrada de `cmd2` |
| `cmd > fichero` | Redirige la salida a `fichero` (lo crea o trunca) |
| `cmd >> fichero` | Añade la salida al final de `fichero` |
| `cmd < fichero` | Lee la entrada desde `fichero` |

Ejemplos:

```sh
cd /tmp
pwd                         # /tmp
echo hola > saludo.txt      # crea /tmp/saludo.txt
cat saludo.txt
echo linea >> saludo.txt
cd ..
ls /tmp
cat /etc/motd | hexdump -    # el `-` es lo que lee stdin
ls | cat -
cat < /etc/motd
```

**Los pipelines necesitan un `-`.** `cat` y `hexdump` leen stdin cuando se les pasa
`-` como fichero, no cuando se les llama sin argumentos: en la tty de soso nadie
interpreta Ctrl-D, así que un `cat` sin argumentos leyendo la consola se quedaría
colgado para siempre en vez de decir cómo se usa. Dentro de un pipeline sí hay EOF
de verdad (la lectura del pipe devuelve 0 cuando el escritor cierra), pero el
programa no puede distinguir un caso del otro sin preguntarle al kernel, y eso hoy
no se puede. Hasta la versión de 2026-07-28 ningún programa leía stdin, así que los
pipelines de esta sección **no funcionaban** aunque estuvieran documentados.

### Comandos integrados (builtins)

| Comando | Descripción |
|---|---|
| `help` | Muestra la ayuda |
| `cd <dir>` | Cambia el directorio de trabajo |
| `pwd` | Imprime el directorio de trabajo actual |
| `exit` | Cierra la shell (código de salida opcional, por defecto 0) |
| `ask [pregunta]` | Habla con el LLM. Ver [ask](#ask--preguntarle-al-modelo) |
| `voz [ask]` | Dictado por voz: transcribe e inserta en la línea (Enter confirma). Ver [voz](#voz--dictado) |
| `wifi scan` | Lista redes WiFi (Intel AX211/AX200) |
| `wifi status` | Estado del adaptador WiFi |
| `wifi connect <ssid> [psk]` | Asocia a una red (sin `psk` = abierta; con clave = WPA2) |

Ejemplos:

```sh
help
pwd
cd /tmp
cd ..                       # sube al directorio padre
wifi scan
wifi connect MiRed MiClaveWPA2
exit
exit 1
```

**`ask` es la excepción al parseo de arriba.** Se resuelve *antes* de trocear la
línea, así que todo lo que va detrás es texto para el modelo: comillas, tildes,
`|`, `>` y `<` incluidos. El precio es que `ask` no admite pipes ni
redirecciones — que es justamente lo que permite que esos caracteres formen
parte de la pregunta.

### Salir del sistema

```sh
halt
```

Apaga la máquina virtual de forma limpia. También puedes usar `exit` en sosh: si la
shell termina con código 0, `init` se detiene y el kernel pasa a la **kernel-shell**
de emergencia (prompt `soso>`).

---

## Comandos de /bin

Estos programas están disponibles desde sosh (o por ruta absoluta):

### ls — listar directorios

```sh
ls              # lista /
ls /etc         # lista un directorio concreto
ls /etc/motd    # muestra un fichero suelto
```

Salida: tipo (`d` directorio, `-` fichero), tamaño en bytes y nombre.

### cat — mostrar ficheros

```sh
cat /etc/motd
cat hola.txt /etc/motd    # varios ficheros seguidos
```

### echo — imprimir texto

```sh
echo hola mundo
```

### mkdir — crear directorios

```sh
mkdir /tmp
mkdir /tmp/a /tmp/b       # varios a la vez
```

### rm — borrar ficheros o directorios vacíos

```sh
rm /tmp/nota.txt
rm /tmp/vacio
```

No borra directorios con contenido.

### hexdump — volcado hexadecimal

```sh
hexdump /etc/motd
```

Muestra offset, bytes en hex y representación ASCII.

### halt — apagar el sistema

```sh
halt
```

### soso-llm — inferencia de modelos

```sh
soso-llm run tiny --prompt hola
soso-llm run tiny-moe --prompt @bos --max 4
```

Ejecuta inferencia greedy sobre modelos en `/models/<nombre>/`. Por defecto
incluye **tiny** (denso), **tiny-moe** (MoE estilo Mixtral), **tiny-mla** (MLA sintético, 1 capa) y **tiny-latent-moe** (LatentMoE, 1 capa). Ver sección
[Modelos LLM](#modelos-llm-soso-llm) para importar modelos y más detalle.

### voz — dictado

```sh
voz              # una toma de micrófono → texto en la línea; Enter para ejecutar
voz ask          # prefija «ask » al dictado
soso-voz dictar --wav /etc/voz-prueba.wav   # transcribe un WAV (PCM16 mono 16 kHz)
```

El reconocimiento corre en **`soso-voz vozd`** (`127.0.0.1:7421`), igual que `askd`
en `:7420`. El texto **nunca se autoejecuta**: se inserta en la línea de sosh y
confirmas con Enter. Push-to-talk: **F4** en la consola serie.

Config en `/etc/voz.conf`:

| Clave | Significado |
|-------|-------------|
| `modelo` | Nombre en `/models` (p. ej. `whisper-tiny`) |
| `idioma` | Índice de idioma Whisper (3 = español) |
| `vad` | Umbral RMS para fin de frase (micrófono) |
| `gpu` | `auto` (GPU si hay pool VRAM), `on` o `off` |

El modelo ASR por defecto es **`whisper-tiny`** en `/models` (Whisper tiny real,
~150 MiB). Con GPU NVIDIA y pool VRAM activo, `vozd` calienta los pesos en VRAM
al arrancar y usa matvec en GPU para convoluciones, capas y logits; al terminar
cada transcripción imprime estadísticas (`matvec`, subidas, residentes).

Para regenerar el modelo en desarrollo: `cargo xtask fetch-whisper` y empaquetar
con `SOSO_ASR_DIR=target/whisper-tiny-model cargo xtask mkfs`.

Para QEMU con micrófono emulado: `SOSO_QEMU_AUDIO=1 cargo xtask run`.

### soso-web — navegador mínimo

```sh
soso-web https://example.com          # modo lectura (texto en consola)
soso-web --local /etc/web-prueba.html # HTML local (pruebas sin red)
soso-web --grafico https://example.com  # modo gráfico (framebuffer)
```

Tras abrir una página, el prompt `[soso-web]` acepta:

| Comando | Acción |
|---------|--------|
| `<n>` | Seguir el enlace numerado `[n]` |
| `u <url>` | Ir a otra URL (HTTPS) |
| `b` | Atrás |
| `q` | Salir |

Modo lectura: parseo HTML tolerante, sin JavaScript, word-wrap configurable
(`--ancho N`, default 72). Modo gráfico: requiere framebuffer GOP; el kernel
cede la consola de texto mientras pinta la app (`SYS_FB_SET_MODE`). Texto con
DejaVu Sans TTF en `/lib/fonts/DejaVuSans.ttf` (fontdue; `--fuente` para otra ruta).

### ask — preguntarle al modelo

```sh
ask ¿por qué el cielo es azul?
ask                                # modo interactivo
```

Escribe la pregunta detrás y ya está: **el texto llega al modelo tal cual se
escribió**, con comillas, tildes, `|`, `>` o lo que lleve. Tras cargar el modelo
sale `ask: generando...` y, en modelos grandes, un punto por cada **capa**
mientras calcula (Mixtral tiene 32: verás una ristra de puntos antes del
texto). El diagnóstico de velocidad y disco está en `soso-llm run`.

Si arrancas desde el live USB con **Mixtral** y no hay pool de VRAM
(`GPU presente sin pool de VRAM` / `pool VRAM=no`) — GPU Ampere cuyo booter
no llegó a montar el pool, o sin GPU —, `ask` corre en CPU: cada token
puede tardar **minutos**. No está colgado: los puntos siguen saliendo.
Para una respuesta ahora, sin reflashear:

```sh
ask :modelo tiny
ask hola
```

(`tiny` viaja siempre en el live, detrás del modelo grande.) O `:max 8` si
quieres quedarte en Mixtral pero cortar la espera.

Sin texto, `ask` abre su propio prompt y el modelo se carga **una sola vez** para
toda la sesión, así que a partir de la segunda pregunta la respuesta empieza
mucho antes.

El modelo vive en un servicio de máquina (`soso-llm askd` en `127.0.0.1:7420`):
consola, SSH y reconexiones comparten la misma carga. Solo se recarga al cambiar
de modelo (`:modelo` / `ask-modelo`), si el planificador necesita RAM, o al
apagar. Salir de sosh o cortar SSH **no** descarga el modelo.

```
$ ask
ask: modelo tiny, máx 128 tokens
ask: escribe la pregunta; «salir» o Ctrl-D para terminar
?> ¿cuánto es 2 > 1?
...
?> salir
$
```

Dentro del prompt, las órdenes empiezan por `:` para no chocar con el texto libre:

| Orden | Efecto |
|---|---|
| `:modelos` | Lista los modelos y marca el que se está usando |
| `:modelo <nombre>` | Cambia de modelo (lo recarga) |
| `:max <n>` | Cambia el máximo de tokens por respuesta |
| `:eco <texto>` | Devuelve el texto tal cual llegó, sin pasar por el modelo |
| `salir`, `exit`, Ctrl-D | Volver a sosh |

`:eco` sirve fuera del prompt igual (`ask :eco a|b>c "x"`) y es la forma rápida de
comprobar que la shell no ha tocado nada.

`ask` no lleva flags —todo lo que va detrás es la pregunta—, así que su
configuración vive en **`/etc/llm.conf`**:

```
# modelo=tinyllama    (sin fijar: el primero de /models)
max=128
temp=0.7
top_p=0.9
```

**Lo normal es no fijar `modelo`.** `ask` coge entonces el primero de `/models`,
y el empaquetado del live pone ahí el modelo de verdad por delante de los
sintéticos: en el pendrive sale ese, y en las imágenes de prueba sale `tiny`.
Fíjalo solo si tienes varios y quieres elegir, y hazlo con `ask-modelo` en la
máquina donde estés — si lo dejas escrito en la imagen y ese modelo no viaja en
ella, `ask` avisa en cada respuesta y usa otro.

#### La plantilla de chat (y por qué no hay que tocarla)

Un modelo de chat no se entrenó con texto suelto sino con turnos marcados. A
TinyLlama hay que darle esto:

```
<|user|>
hola</s>
<|assistant|>
```

Si se le manda «hola» a secas no ve una conversación: ve un fragmento de texto y
lo continúa, y la respuesta sale con pinta de novela a medias. **Eso ya está
resuelto y no hay que configurar nada**: la plantilla viaja dentro del modelo (la
guardan `convert-gguf` y `soso-hf pull` al convertirlo) y `ask` la aplica sola.

La clave `plantilla=` de `/etc/llm.conf` está para los casos raros:

| Valor | Efecto |
|---|---|
| sin poner | la que trae el modelo — lo normal |
| `crudo` | ninguna; el texto va pelado (para comparar) |
| `<\|user\|>\n{prompt}{eos}\n<\|assistant\|>\n` | ésa, pisando la del modelo |

`{prompt}` es la pregunta y `{eos}` el token de fin **del modelo**: es un token,
no las letras `</s>`, y ahí está el detalle que hace que funcione. Los saltos de
línea se escriben `\n`.

Se ignora en los modelos sintéticos (`tiny`, `tiny-moe`…): su tokenizador es el
byte-level de reserva, donde `<|user|>` no es un token sino nueve bytes de ruido.

Y una expectativa honesta: la plantilla arregla el **formato** —el modelo
contesta como asistente en vez de continuar un texto— no lo que sabe. TinyLlama
son 1.1B cuantizados y en español se le nota; en inglés responde bastante mejor.

### ask-modelo — elegir el modelo de `ask`

```sh
ask-modelo                  # lista los modelos y marca el actual
ask-modelo tiny-moe         # lo fija en /etc/llm.conf
```

Escribe solo la clave `modelo=` y respeta el resto del fichero.

Aviso con los modelos sintéticos: `tiny-moe` tiene un vocabulario de 64 tokens y
el tokenizador de reserva es byte a byte, así que casi cualquier texto se le sale
de rango. `ask` lo dice en vez de fallar sin explicación; usa `tiny` (vocabulario
256) o un modelo importado de verdad.

### soso-install — instalar soso en un disco

```sh
soso-install               # lista discos, su uso, y pide en cuál instalar
soso-install list          # lo mismo, sin preguntar
soso-install nvme1 --yes   # instalar en ese disco (id o nombre)
soso-install status        # estado de la entrada de arranque UEFI
```

Solo tiene sentido arrancando desde el pendrive live. Ver
[Instalar soso en un disco](#instalar-soso-en-un-disco-dual-boot-uefi).

### soso-resize — ampliar el disco de sistema

La imagen live empaqueta sosofs **compacto** (solo contenido + margen). Al
instalar, el espacio sobrante del NVMe va a la partición de **modelos**; el
rootfs sigue pequeño hasta que lo amplíes:

```sh
soso-resize              # cuánto ocupa rootfs/modelos y cuánto se puede mover
soso-resize rootfs +8G   # roba 8 GiB del margen libre de modelos
```

Solo funciona en pendrive live o instalación GPT (no en QEMU virtio sin tabla
GPT: ahí el rootfs ya crece solo al arrancar si `SOSO_ROOTFS_SIZE` es mayor que
el empaquetado).

### soso-update — actualizar soso instalado

Comprueba y aplica releases publicadas en GitHub (`demonodojo/soso`):

```sh
soso-update estado              # medio de arranque, versión rootfs, kernel y buzón ESP
soso-update comprobar           # compara con la última release
soso-update aplicar             # descarga e instala (rootfs + kernel)
soso-update revertir            # restaura el kernel anterior (reinicia después)
```

Opciones útiles: `--forzar` (reinstala aunque la versión no suba), `--sin-kernel`
(solo `/bin`, `/etc`, `/lib`), `--local /ruta` (artefactos locales, sin red).

Configuración en `/etc/actualiza.conf` (`url=https://…/releases/latest/download`).

**Solo se descarga lo que cambia.** `aplicar` compara cada fichero instalado con
el hash del manifiesto y pide por HTTP `Range` únicamente los tramos del pack que
cubren los ficheros distintos, agrupando los que están cerca para no repetir
handshakes TLS. El kernel se salta entero si el que ya tienes es el que pide el
manifiesto. Una actualización que solo toca un par de programas baja unos pocos
MB en vez del pack completo:

```
$ soso-update comprobar
remoto: 0.2.2 (a1b2c3d)
local:  0.2.1
hay actualización disponible
  ~ bin/soso-update (1,3 MB)
rootfs: 1,3 MB en 1 petición (pack completo: 12 MB)
kernel: sin cambios
descarga total: 1,3 MB
```

`comprobar` no descarga nada: solo trae el manifiesto y te dice el tamaño real
de la actualización antes de lanzarla.

Tras `aplicar`, **reinicia** para que el shim UEFI aplique el kernel nuevo.
Si el arranque falla, el shim revierte al kernel anterior en el siguiente
reinicio (backup en `SOSOKRN.BIN`, estado durable en `SOSOKRN.MET` en la ESP).
Las imágenes live **anteriores** a tener esos huecos solo pueden actualizar
rootfs hasta reflashear/reinstalar el live una vez.

**Rootfs:** la actualización es por fichero (solo baja lo que cambia). Si
`aplicar` se interrumpe, el progreso queda en `/etc/actualiza.estado` para
reintentar; **no** hay copia automática de los binarios anteriores del rootfs.

La versión del sistema está en `/etc/soso-release`; el kernel la muestra al
arrancar (`soso 0.2.2 (build)`).

Ver también [Instalar soso en un disco](#instalar-soso-en-un-disco-dual-boot-uefi)
(sección «Actualizaciones de kernel»).

### soso-hf — descargar modelos desde Hugging Face

```sh
soso-hf search tinyllama
soso-hf list org/repo
soso-hf pull org/repo --file modelo.Q4_K_M.gguf --name mi-modelo
```

Descarga el GGUF por HTTPS, lo convierte al formato `.som` e importa el modelo
en `/models/` sin reiniciar. Requiere red y espacio en el disco de modelos.
Detalle en [Descargar desde Hugging Face (guest)](#descargar-desde-hugging-face-guest).

---

## Estructura del disco

Tras el arranque, el filesystem **sosofs** expone al menos:

```
/
├── bin/          # Programas (init, sosh, soso-llm, soso-update, soso-web, …)
├── etc/
│   ├── soso-release    # Versión instalada (version/build/fecha)
│   ├── actualiza.conf  # URL de releases para soso-update
│   ├── llm.conf        # Modelo y límites de `ask` (ver ask-modelo)
│   ├── voz.conf        # Modelo ASR, idioma, VAD
│   ├── wifi.conf       # SSID y clave WiFi (alternativa a SOSOWIFI.TXT en ESP)
│   ├── motd            # Mensaje de bienvenida
│   ├── authorized_key  # Clave pública ed25519 autorizada (32 bytes)
│   └── ssh_host_key    # Semilla de la host key del servidor SSH
├── lib/
│   ├── firmware/       # Firmware WiFi, NVIDIA GSP, …
│   └── fonts/          # TTF para soso-web --grafico
├── models/       # Modelos LLM y ASR (disco sosomfs, solo lectura)
│   ├── tiny/             # Modelo sintético denso (4 capas)
│   ├── tiny-moe/         # Modelo MoE sintético (4 expertos, top-2)
│   ├── whisper-tiny/     # ASR Whisper (live con fetch-whisper)
│   └── …                 # Modelos importados (soso-hf, mkfs, …)
├── var/
│   └── actualiza-prueba/ # Solo en imágenes de prueba (E2E actualización)
└── hola.txt      # Fichero de ejemplo
```

El mensaje de bienvenida (`/etc/motd`) se muestra al conectar por SSH.

Los cambios que hagas con `mkdir`, redirecciones (`>`, `>>`) o escritura desde
userspace se persisten en el disco virtual entre arranques (sosofs copy-on-write).
Los ficheros bajo `/models/` viven en un disco aparte y no se modifican desde sosh.

---

## Kernel-shell de emergencia

Si la shell de usuario no está disponible (por ejemplo, tras un `exit 0` de sosh),
el kernel ofrece una consola de diagnóstico con prompt:

```
soso>
```

Comandos principales:

| Comando | Descripción |
|---|---|
| `help` | Lista todos los comandos |
| `dmesg` | Log de consola paginado (espacio/enter = más, `q` = salir); el FB solo muestra ~40 líneas |
| `dmesg save` | Volcar el log a `SOSOLOG.TXT` en la ESP del USB live (también se hace solo cada ~2 s) |
| `ls [ruta]` | Listar directorio (por defecto `/`) |
| `cat <ruta>` | Mostrar un fichero (`cat -` lee stdin, para pipelines) |
| `stat <ruta>` | Metadatos de un fichero o directorio |
| `write <ruta> <texto>` | Crear o sobrescribir un fichero |
| `mkdir <ruta>` | Crear un directorio |
| `rm <ruta>` | Borrar un fichero o directorio vacío |
| `df` | Espacio libre en el disco |
| `spawn <elf> [args]` | Lanzar un programa ELF manualmente |
| `ps` | Listar procesos |
| `uptime` | Tiempo desde el arranque |
| `mem` | Memoria física libre |
| `kbd` | Estado del teclado; `kbd es` / `kbd us` cambia el mapa |
| `halt` | Apagar |

La kernel-shell también incluye comandos de bajo nivel para depuración (`hwscan`,
`blk`, `blkread`, `blkwrite`, `pf`, `panic`). Están pensados para desarrollo,
no para uso habitual.

### Autodescubrimiento de drivers (`hwscan`)

El comando **`hwscan`** enumera dispositivos PCI y muestra qué driver los
atendería y si está compilado en el kernel actual. Una línea por dispositivo:

```
drv: bb:dd.f VVVV:DDDD nombre-driver compilado|ausente clase cc:ss:pi texto
drv: bb:dd.f VVVV:DDDD sin-driver   desconocido        clase cc:ss:pi texto
```

Los tres estados:

| Estado | Significa |
| --- | --- |
| `compilado` | Hay driver y está en este kernel |
| `ausente` | soso conoce el driver, pero esta imagen no lo lleva |
| `desconocido` | Ningún driver reclama el dispositivo |

Al final del informe, cada controlador de red (clase PCI `02`) que nadie
reclama sale destacado:

```
hwscan: RED SIN DRIVER 00:1f.6 8086:15fc (ethernet)
```

Ese `VVVV:DDDD` es lo que hace falta para decidir si basta con ampliar la lista
de IDs de un driver existente o si hay que portar uno nuevo.

El informe se imprime por serie **en cada arranque**, y en live se guarda además
en **`SOSODRV.TXT`** en la ESP (junto a `SOSOLOG.TXT`). Desde el PC de
desarrollo, con el pendrive puesto:

```sh
cargo xtask sosolog --drv           # informe hwscan del último arranque
cargo xtask sosolog --drv /dev/sdX  # ESP concreta
```

Y para recalcular el perfil de drivers a partir de él:

```sh
cargo xtask fit-drivers /ruta/a/SOSODRV.TXT
# o actualizar solo la ESP del pendrive:
cargo xtask fit-drivers target/SOSODRV.TXT --esp /dev/sdX
```

### Trazas persistentes en USB live (`SOSOLOG.TXT`)

En arranque live, el kernel vuelca automáticamente el log de consola (ring de
~256 KiB) al fichero **`SOSOLOG.TXT`** en la **ESP** (partición 1 FAT del
pendrive). No hace falta teclado: el volcado empieza en cuanto se detecta el
disco live y se repite cada ~2 s si hay trazas nuevas (también en panic).

Tras probar soso en placa, vuelve a Linux y lee el log. La ESP **no se monta sola**
(Linux oculta las particiones EFI); usa la utilidad, que monta, muestra y desmonta.
Pide `sudo` solo para mount/umount: no lances `sudo cargo` (root no tiene rustup).

```sh
cargo xtask sosolog              # auto-detecta el USB live
cargo xtask sosolog /dev/sdX     # disco entero → partición 1
cargo xtask sosolog /dev/sdX1    # ESP concreta
cargo xtask sosolog --drv        # SOSODRV.TXT (informe hwscan) en vez del log
cargo xtask sosolog | less
```

Regenera la imagen live tras actualizar el kernel:
`cargo xtask package-usb-live` (incluye el fichero pre-creado en la ESP).

**Actualización rápida (desarrollo):** si el pendrive ya está flasheado y solo
cambiaste kernel o rootfs, no hace falta reescribir los modelos (p3):

```sh
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --skip-models
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --only kernel
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --only rootfs
```

`--skip-models` actualiza ESP (p1) y rootfs (p2) sin tocar sosomfs. Preserva
`SOSOWIFI.TXT` si ya lo tenías en la ESP.

### Buzón de instalación (`SOSOBOOT.TXT`)

Tercer fichero pre-creado en la ESP del pendrive, de 4 KiB. Es por donde
`soso-install` le pide al shim UEFI que registre la entrada de arranque del
disco recién instalado, y por donde el shim contesta. Se lee desde soso con
`soso-install status`, y desde Linux montando la ESP:

- `INSTALL <guid-de-la-ESP-destino>` — petición pendiente; se atiende en el
  siguiente arranque del USB.
- `DONE Boot0007 soso` — entrada creada y puesta la primera en `BootOrder`.
- `ERROR …` — el shim no pudo (firmware que rechaza `SetVariable`, o la ESP
  destino no aparece). El arranque del USB continúa igual: esto nunca lo
  bloquea.

---

## Suite de pruebas

Para verificar que las 14 syscalls del sistema funcionan desde userspace:

```sh
/bin/init test
```

(o desde sosh: `init test`). Imprime una batería de comprobaciones con resultado
`OK` / `FALLO` por cada prueba.

---

## Autenticación SSH

soso **solo acepta autenticación por clave pública ed25519**. No hay contraseñas.

1. La clave autorizada vive en `/etc/authorized_key` (32 bytes crudos, no texto OpenSSH).
2. Se inyecta al crear la imagen del disco (`mkfs-soso`).
3. Si el fichero no existe, **todas** las conexiones SSH son rechazadas.

Para autorizar tu clave:

1. Ten una clave ed25519 en el anfitrión (`ssh-keygen -t ed25519`).
2. Asegúrate de que `~/.ssh/id_ed25519.pub` exista antes de `cargo xtask run`.
3. O regenera la imagen: `cargo xtask mkfs`.

Con la clave de test generada automáticamente:

```sh
ssh -tt -i target/soso_test_key -p 2222 soso@localhost
```

---

## Limitaciones conocidas

soso es un sistema con alcance deliberadamente reducido en algunas áreas:

| Área | Limitación |
|---|---|
| Usuarios | Monousuario; una sesión SSH simultánea |
| Shell | Sin variables ni historial; cwd, pipes y redirecciones |
| Procesos | `spawn`, no `fork`; scheduler round-robin preemptivo |
| Red | DHCP automático al arrancar; fallback a `10.0.2.15` en QEMU; sin IPv6 |
| SSH | Sin SFTP, port forwarding ni múltiples sesiones |
| Web | Sin JavaScript ni CSS avanzado; modo gráfico básico |
| Ficheros | Sin permisos Unix, hardlinks ni snapshots |
| Comandos | Conjunto acotado de coreutils y utilidades propias |

Un page fault en userspace mata al proceso afectado, no al kernel. Si sosh muere con
error, `init` la relanza automáticamente.

---

## Solución de problemas

### «Permission denied» al conectar por SSH

- Comprueba que usas la clave correcta (`-i target/soso_test_key` o tu `id_ed25519`).
- Regenera la imagen si cambiaste de clave: `cargo xtask mkfs` y vuelve a arrancar.
- Verifica en la consola serie que aparezca `ssh: auth por clave pública ed25519`.

### «Connection refused» o «Connection reset» en el puerto 2222

- soso debe estar en marcha (`cargo xtask run`).
- Espera a ver el mensaje `sosh — escribe 'help' para la ayuda` antes de conectar.
- Si falló un arranque anterior, el puerto puede quedar ocupado: `pkill qemu-system-x86`
  y vuelve a lanzar `cargo xtask run`.
- Tras cerrar la sesión con `exit` puedes reconectar; si falla, espera un segundo o reinicia QEMU.

### La consola no responde

- QEMU usa `-serial mon:stdio`. Escribe en la misma terminal donde lanzaste `run`.
- Para salir: `Ctrl-A X` (no `Ctrl-C`).

### Caracteres raros o teclado «americano» en pantalla

- En placa, el mapa por defecto es **es** (ISO español). Si las teclas no coinciden
  con lo impreso, prueba `kbd us` en la kernel-shell (`soso>`) o `kbd es` si tenías
  el mapa US.
- Si ves dos símbolos basura por cada letra acentuada, recompila el kernel reciente
  (la consola GOP decodifica UTF-8 desde una sola celda por carácter).
- Por SSH, el layout lo gestiona tu terminal; esto solo aplica al teclado físico
  conectado a soso.

### «no existe» al acceder a un fichero

- Comprueba el cwd con `pwd`; las rutas sin `/` son relativas a ese directorio.
- Usa rutas absolutas si dudas (`/etc/motd`).
- Lista el contenido con `ls` o `ls /` para ver qué hay en el disco.

### Redirección `> fichero` no crea el fichero

- Comprueba que el directorio padre exista (`mkdir /tmp` si hace falta).
- El fichero se escribe al terminar el comando hijo (p. ej. cuando `echo` sale).
- Verifica con `cat fichero` o `ls`.

### Verificar el sistema de extremo a extremo

```sh
cargo xtask test
```

Ejecuta pruebas de integridad del FS, arranque, echo TCP, inferencia LLM (`soso-llm`), sesión SSH y apagado limpio.

---

## Modelos LLM (`soso-llm`)

soso incluye un segundo disco virtual (`virtio-blk1`) con el filesystem de modelos **sosomfs**. Los modelos aparecen bajo `/models/<nombre>/` con esta estructura:

```
/models/tiny/manifest.som    # arquitectura (capas, hidden, vocab, GQA, RoPE…)
/models/tiny/index.som       # tabla tensor → shard + offset + shape + dtype
/models/tiny/tokenizer.som   # vocabulario (solo modelos importados de GGUF)
/models/tiny/shards/*.tensor # pesos empaquetados (F32 o Q8_0, con CRC32C)
/models/tiny-moe/            # Modelo MoE sintético (Mixtral-style, 4 expertos top-2)
/models/tiny-mla/            # Modelo MLA sintético (1 capa, atención comprimida)
```

El manifest v3 añade campos MoE: `num_experts`, `num_experts_per_tok`, `moe_ffn_dim`.
Los tensores de expertos siguen la convención `L{i}.E{e}.ffn_{gate,up,down}`; el
router es `L{i}.ffn_gate_inp`. Solo se cargan en RAM los expertos activos por token
(streaming estilo AirLLM).

### Modelo de prueba incluido

Al arrancar con `cargo xtask run`, se generan los modelos sintéticos **tiny** (4 capas,
denso, hidden 128), **tiny-moe** (2 capas, 4 expertos, top-2), **tiny-mla** (1 capa MLA) y **tiny-latent-moe** (1 capa LatentMoE). Puedes ejecutar
inferencia desde **sosh**:

```sh
soso-llm run tiny --prompt hola
soso-llm run tiny-moe --prompt @bos --max 4
soso-llm run tiny-mla --prompt test --max 2
soso-llm run tiny-mla --prompt test --gpu-soft --max 2
soso-llm run tiny-latent-moe --prompt @bos --max 2
```

El offload GPU (sin `--cpu`) admite pesos **F32, Q8_0, Q4_K y MXFP4**. Q4_K y Q8_0
se suben a VRAM **tal como están en disco** y el dispositivo los multiplica sin
expandirlos: 8× menos memoria de vídeo y 8× menos tráfico por el bus que
descuantizarlos antes, así que caben 8× más capas en la tarjeta. En Mixtral los
expertos `Lxx.Eyy.ffn_*` también van a ese pool (los que quepan); el FFN denso
`ffn_gate` igual. MXFP4 sí se descuantiza al subir. Con `--gpu-soft` ejercitas
esa fontanería sin silicio NVIDIA.

La salida muestra el texto generado con decode greedy. El modelo tiny usa un
tokenizer byte-level; los modelos importados de GGUF usan su propio
vocabulario (`tokenizer.som`). Para **tiny-moe** (vocab 64) usa `@bos` como
prompt o tokens con id &lt; 64.

### Modelos MoE (Mixtral-style)

Los modelos con `num_experts > 0` en el manifest (v3) usan un router por capa
(`ffn_gate_inp`) que elige los expertos activos por token (top-k). Solo esos
expertos se cargan en memoria — el resto permanece en disco hasta que el
router los necesite. Esto permite inferir modelos MoE mucho mayores que la
RAM disponible (técnica inspirada en AirLLM).

Ejemplo con Mixtral convertido desde GGUF:

```sh
# En el host
cargo xtask convert-gguf mixtral-8x7b.Q4_K_M.gguf target/mixtral --name mixtral
SOSO_MODELS_DIR=target/mixtral cargo xtask run

# En el guest
soso-llm run mixtral --prompt "Once upon a time" --max 16
```

Los expertos siguen la convención `L{i}.E{e}.ffn_{gate,up,down}` en el índice
del modelo; el convertidor trocea automáticamente los tensores 3D `ffn_*_exps`
del GGUF.

### Modelos Qwen3.8 (atención híbrida)

Qwen3.8-27B (el default del live en pendrives de 32 GB+) mezcla **Gated
DeltaNet** (la mayoría de las capas, estado fijo) con **atención completa
con puerta** cada cuatro capas. `ask` y `soso-llm run qwen3.8-27b` lo usan
igual que el resto: no hay flags extra.

### Descargar desde Hugging Face (host)

En la máquina anfitriona puedes bajar un GGUF del Hub, convertirlo a `.som` y
dejar listo el arranque con QEMU:

```sh
# buscar modelos GGUF en el Hub (por defecto solo repos con tag gguf)
cargo xtask fetch-hf search tinyllama
cargo xtask fetch-hf search llama --limit 10 --all

# listar ficheros .gguf de un repo
cargo xtask fetch-hf list TinyLlama/TinyLlama-1.1B-Chat-v1.0

# elige Q4_K_M automáticamente si existe
cargo xtask fetch-hf TinyLlama/TinyLlama-1.1B-Chat-v1.0

# repos gated: export HF_TOKEN=hf_…
cargo xtask fetch-hf org/repo --file modelo.Q4_K_M.gguf --name mi-modelo --run
```

Cache en `target/hf-cache/`; modelos convertidos en `target/hf-models/<nombre>/`.
Al terminar imprime `SOSO_MODELS_DIR=… SOSO_MODELS_SIZE=… cargo xtask run`.

### Descargar desde Hugging Face (guest)

Con red en QEMU o en soso live puedes instalar un modelo **directamente en
`/models/`** (partición sosomfs). No hace falta reconstruir la imagen de
modelos en el host ni usar `/var/models/` como almacén intermedio.

```sh
# token opcional para repos privados o gated
echo hf_… > /etc/hf_token

soso-hf search tinyllama
soso-hf list TinyLlama/TinyLlama-1.1B-Chat-v1.0
soso-hf pull TinyLlama/TinyLlama-1.1B-Chat-v1.0
soso-hf pull org/repo --file mixtral.Q4_K_M.gguf --name mixtral
soso-llm run tinyllama --prompt hola
```

`soso-hf pull` descarga el GGUF, lo convierte a `.som` e importa el modelo de
forma atómica. Si el Hub admite HTTP Range, la conversión no guarda una copia
completa del GGUF en disco; si no, usa un área temporal en la propia partición
de modelos. Al terminar, el modelo aparece en `/models/<nombre>/` y `soso-llm`
(o `ask`) lo listan sin reiniciar.

`soso-llm` busca modelos en `/models/`; `/var/models/` queda solo como respaldo
legacy si copias ficheros a mano en sosofs.

### Importar un modelo GGUF

En la máquina anfitriona, convierte un fichero GGUF de arquitectura llama al
layout `.som`. Se soportan tensores **F32, F16, Q8_0 y Q4_K** (los Q4_K_M
descargables funcionan tal cual; sus tensores Q6_K se convierten a Q8_0),
GQA, SwiGLU con `ffn_gate`, **MoE Mixtral** (`llama.expert_count > 0`: trocea
`ffn_*_exps` en shards por experto) y el vocabulario del tokenizer. Verificado con
TinyLlama-1.1B-Chat Q4_K_M:

```sh
cargo xtask convert-gguf ruta/al/modelo.gguf target/mi-modelo --name mi-modelo
```

Arquitectura **deepseek2** (MLA): el convertidor acepta GGUF con
`general.architecture = deepseek2`. Smoke en host tras convertir:

```sh
cargo test -p gguf2som --features std convierte_deepseek2_mla_sintetico
cargo run --release -p soso-llm-core --features std --example hostrun -- \
  target/mi-modelo "a" 1
```

Después arranca con la variable `SOSO_MODELS_DIR` apuntando al modelo
convertido (la imagen de modelos se reconstruye automáticamente):

```sh
SOSO_MODELS_DIR=target/mi-modelo cargo xtask run
```

El modelo estará en `/models/mi-modelo/`:

```sh
soso-llm run mi-modelo --prompt "hola"
```

Sin la variable, `cargo xtask run` vuelve a empaquetar el modelo tiny.

### Inferencia distribuida (pipeline por capas)

Puedes repartir un modelo **por capas** entre varias máquinas soso enlazadas por
red. El **head** orquesta la inferencia en topología **en estrella**: se conecta
por TCP a todos los nodos remotos, ejecuta embedding y sus capas locales, y
reenvía activaciones entre etapas. Cada nodo mantiene su KV cache y solo pagina
en RAM los pesos de su rango.

**Limitaciones:** configuración estática (IP/puerto/splits manual), sin
descubrimiento automático. Con protocolo **v3** hay tolerancia a fallos
(timeouts, keepalive, nodos persistentes, head standby).

#### Dos nodos (compat v1)

En el **nodo remoto** (tail):

```sh
soso-llm node tiny --listen 9900 --layers 2:4
# alias: soso-llm worker tiny --listen 9900 --split 2
```

En el **head**:

```sh
soso-llm run tiny --remote 10.0.2.15:9900 --split 2 --prompt test --max 8 --seed 42
```

#### Tres o más nodos (estrella)

Arranca los nodos remotos **de tail a head** (todos en `--listen` antes del
`run`). Ejemplo con tiny (4 capas, reparto 2+1+1):

```sh
# Tail (capas 3..4, logits)
soso-llm node tiny --listen 9902 --layers 3:4

# Intermedio (capas 2..3)
soso-llm node tiny --listen 9901 --layers 2:3

# Head (capas 0..2 local + orquestación)
soso-llm run tiny --pipeline 10.0.2.16:9901,10.0.2.15:9902 --splits 2,3 \
  --prompt test --max 8 --seed 42

# Head de reserva (failover automático)
soso-llm run tiny --pipeline 10.0.2.16:9901,10.0.2.15:9902 --splits 2,3 \
  --standby --prompt test --max 8 --seed 42
```

| Flag | Efecto |
|---|---|
| `--pipeline <ip:puerto>,...` | Nodos remotos en orden de pipeline (solo en `run`) |
| `--splits <n1,n2,...>` | Fronteras de capa; head ejecuta `[0,n1)`, remotos los tramos siguientes |
| `--remote` / `--split` | Atajo de 2 nodos (equivale a `--pipeline` con un solo remoto) |
| `--listen <puerto>` | Puerto TCP del nodo (en `node`/`worker`) |
| `--layers <start>:<end>` | Rango de capas del nodo |
| `--step-timeout-ms <ms>` | Timeout por paso de inferencia (default 120000) |
| `--handshake-timeout-ms <ms>` | Timeout de conexión/handshake (default 60000) |
| `--accept-timeout-ms <ms>` | Timeout de `accept` en nodos (default 60000) |
| `--ping-interval-ms <ms>` | Intervalo de `PING` keepalive (default 5000) |
| `--ping-idle-ms <ms>` | Máx. silencio del peer antes de abortar (default 15000) |
| `--standby` | Head en espera: toma el cluster cuando los nodos quedan libres |
| `--standby-retry-ms <ms>` | Pausa entre intentos de takeover (default 3000) |

#### Tolerancia a fallos (v3)

- **Nodos colgados:** el head aborta la sesión si un remoto no responde dentro
  de `--step-timeout-ms` y envía error al resto.
- **Keepalive activo:** durante esperas idle (handshake, nodo esperando `Step`),
  ambos lados envían `PING` cada `--ping-interval-ms`; si no hay respuesta en
  `--ping-idle-ms`, la sesión se aborta (p. ej. head caído detectado en ~15 s).
- **Nodos persistentes:** tras caída del head, timeout o error de sesión, cada
  `soso-llm node` vuelve a `listen` automáticamente (resetea KV cache).
- **Failover automático del head:** arranca un head de reserva con `--standby`;
  reintenta conectar y orquestar cuando los nodos quedan libres. Varios hosts
  con `--standby` compiten: el primero en completar handshake gana la sesión.
- **Failover manual:** también puedes lanzar de nuevo `soso-llm run --pipeline ...`
  sin `--standby` cuando los nodos ya re-escuchan.

Gates de prueba en QEMU:

```sh
cargo xtask test-distributed-llm      # 2 nodos
cargo xtask test-distributed-llm-3    # 3 nodos en estrella
```

### Memoria, CPU y modelos grandes

QEMU arranca por defecto con 2 GiB y 1 CPU; ambos son configurables:

```sh
SOSO_QEMU_MEM=16G SOSO_QEMU_SMP=4 cargo xtask run
```

Los pesos se leen **sin copia** directamente del mmap del modelo (páginas de
2 MiB bajo demanda). El KV cache usa **f16** por defecto, o **int8** (KIVI-lite)
si el planificador detecta poca RAM. El kernel **reclama** páginas de pesos bajo
presión (marca de agua ~4 MiB): un modelo puede ser más grande que la RAM y
degradar a velocidad de disco en lugar de morir por OOM.
`soso-llm` incluye un **planificador de recursos** que lee la memoria libre
(`SYS_MEMINFO`), reparte capas entre CPU/GPU/nodo remoto según latencia medida
y replanifica cada pocos tokens. Además aplica streaming estilo **LayerKV /
FlexGen** (pocas capas de pesos residentes + prefetch de la siguiente). En
modelos **MoE**, los expertos se cargan bajo demanda (solo los activos por
token) con cache LRU de expertos calientes entre tokens. Si hay VRAM, el
planificador pinnea primero el tronco (atención + router) y deja un pool
compartido para esos expertos — no reserva los 8 de Mixtral por capa, porque
solo se activan 2. También aplica ventana
**StreamingLLM** / **H2O** en el KV (sink + tokens de mayor atención + recientes)
y, con contextos largos, atención sparse por bloques (**Quest-lite**). Al
arrancar y al terminar verás:

```text
soso-llm: planificador — presupuesto pesos … KiB, modelo … KiB, capas CPU/GPU/remoto …
soso-llm: GPU MoE — N expertos caben en VRAM, offload en M capas
soso-llm: streaming — working-set N capas, ventana KV T tokens …, KV f16|int8 H2O=… sparse=…
soso-llm: memoria — libre … KiB, reclaimable … KiB
soso-llm: plan memoria — trunk … KiB (pin N capas, anillo …), expert cache … KiB …
soso-llm: pesos — tronco … KiB, expertos … KiB, siempre-residente … KiB
…
soso-llm: planificador — replanes N, latencia media CPU/GPU/remoto … ms
soso-llm: hot path — matvec … ms/capa, attn … ms/capa
soso-llm: streaming — prefetch …, staging wait … ms, liberaciones shard …, ventanas KV …
soso-llm: tronco — N hits, M misses, … KiB leídos
soso-llm: MoE cache — N resident, M JIT, K fríos
soso-llm: MoE especulativo — N aciertos, M fallos
soso-llm: prompt-lookup — N aceptados en M intentos (n≈…, draft≤…)
```

Presets de memoria (split **tronco antes que caché MoE**, estilo kimi-k3-in-c):

```text
soso-llm run mixtral --prompt hola --mem-tight      # máximo pin de tronco
soso-llm run mixtral --prompt hola --mem-balanced   # reparto 85/15
soso-llm run mixtral --prompt hola --mem-max-pin    # pin agresivo
soso-llm run mixtral --prompt hola --trunk-frac 90 --ring-slots 2
```

En host, `convert-gguf modelo.gguf salida/ --pack-trunk` empaqueta attn+FFN por capa
en `Lxx.trunk.tensor` (menos mmap/faults por token). Simular caché MoE offline:
`SOSO_MOE_TRACE=1 SOSO_PLANNER=1 hostrun …` y `python3 tools/sim-moe-cache.py trace.bin`.

El runtime aplica **double-buffer estilo AirLLM**: `kick` de la capa N+1 mientras
calcula N, y un hilo de staging en userspace solapa el page-fault de shards con
matvec/attn. En MoE, además prefetcha los expertos del token anterior antes del
router (hint especulativo). En greedy, **Prompt Lookup** reutiliza continuaciones del
propio contexto (sin modelo draft) y **autotunea** el n-gramo / longitud de draft
según la tasa de aceptación.
El prefill hace prefetch del embed del siguiente token mientras calcula.

La ventana de mapeo de usuario llega a ~416 GiB; la imagen de modelos se
dimensiona con `SOSO_MODELS_SIZE` (por defecto 8G) si el árbol `.som` no cabe
en el disco de modelos.

`soso-llm run` genera en **streaming** (imprime cada token según sale) y
acepta muestreo además del greedy por defecto:

```sh
soso-llm run tinyllama --prompt Once upon a time --max 32 --temp 0.8 --top-p 0.9 --seed 7
```

| Flag | Efecto |
|---|---|
| `--max <n>` | Tokens nuevos como máximo (16 por defecto) |
| `--temp <t>` | Temperatura; 0 = greedy (por defecto) |
| `--top-p <p>` | Muestreo nucleus (0.9 por defecto) |
| `--seed <s>` | Semilla determinista del muestreo |
| `--gpu-soft` | Dispositivo de cómputo **software** del kernel (ver abajo) |
| `--chat` | Aplica la plantilla de chat del modelo (lo que hace `ask` siempre) |

El prompt admite varias palabras (hasta el siguiente flag); sosh no
interpreta comillas.

**`--chat` es opt-in aquí a propósito.** `soso-llm run` es la herramienta de
diagnóstico, y lo que la hace útil es poder lanzar el mismo prompt con y sin
plantilla sobre el mismo modelo:

```sh
soso-llm run tinyllama --prompt What is the capital of France? --max 24 --chat
```

Sin `--chat` un modelo de chat continúa el texto en vez de responder. `ask` no
tiene el flag porque siempre la aplica.

### GPU NVIDIA nativa (cuando hay dGPU en QEMU)

Si arrancas soso con la GPU pasada por VFIO (`SOSO_QEMU_GPU=vfio:…`), el bring-up
GSP puede dejar el compute listo (`rm_compute`). Entonces `soso-llm run` usa la
GPU sola, sin flags extra: los matvec salen con `on_gpu=1` y el resumen final
cuenta trabajo en silicio. En la RTX 5070 Ti Mobile (GB205) esto ya funciona para
modelos pequeños (`tiny` con `--max 4`). Si el GSP no llega a compute, `soso-llm`
sigue en CPU sin que tengas que hacer nada.

Cuando la tarjeta está pero no hay dónde poner los pesos —el bring-up arrancó el
GSP y no llegó a canal ni motor de copia, que es hoy el caso de las Ampere— lo
dice y se va a CPU sin más:

```
soso-llm: GPU presente sin pool de VRAM (fase=booted o **fallo**) — inferencia en CPU
```

Y el arranque lo canta antes de que preguntes: `gpu: NVIDIA detectada (…, pool
VRAM=no)`. Si en su lugar ves `offload GPU desactivado — <razón>`, ahí sí falló
algo del camino de la GPU con el dispositivo listo, y la razón dice dónde.

### `--gpu-soft`: el camino de la GPU sin GPU

Con `--gpu-soft`, soso enciende un dispositivo de cómputo de mentira que calcula
en la CPU del kernel, y `soso-llm` manda los matvec por las mismas syscalls que
usaría con una GPU de verdad. **No acelera nada** —es más lento que el backend de
CPU normal, porque los datos van y vienen por syscalls— y sirve para dos cosas:

- comprobar en cualquier máquina que la fontanería del offload funciona (reservar
  búferes, subir pesos, lanzar, leer el resultado);
- ver el resumen que imprime al final: matvec lanzados, **subidas de pesos** y
  matrices residentes. Si las subidas fueran tantas como los matvec, los pesos se
  estarían resubiendo en cada token;
- y la línea de **subidas**, que dice cuántas fueron *en crudo* (bloques
  cuantizados sin expandir) y cuántos ciclos se han ido en descuantizar y en la
  propia subida. Si con un modelo Q4_K ninguna va en crudo, se está pagando 8× de
  memoria de vídeo sin necesidad. Con una GPU de verdad aparece además una línea
  con las subidas que fueron por **DMA** (el motor de copia leyendo directamente de
  donde está el modelo, sin que la CPU toque un byte) y las que cayeron al camino
  lento: ahí lo que se quiere ver es cero por rebote.

```sh
soso-llm run tiny --prompt test --gpu-soft --max 4
```

```
soso-llm: dispositivo de cómputo «soft (CPU del kernel, pruebas)» (fase ), VRAM libre 268435456 bytes
soso-llm: generado (6 tokens, 5450 ms, 1.10 tok/s)
soso-llm: dispositivo «soft (CPU del kernel, pruebas)» — 144 matvec, 24 subidas de pesos, 24 matrices residentes, 0 sin sitio (a CPU), último on_gpu=0
soso-llm: subidas — 0 de 24 en crudo (sin expandir a f32), 0 Mciclos descuantizando, 21 Mciclos en gpu_map
soso-llm: el silicio no calculó nada — el GSP se quedó en la fase «»
```

(`0 de 24 en crudo` porque el modelo `tiny` es F32: no hay nada que expandir. Con
`tiny-q4k` o un modelo importado en Q4_K las 24 van en crudo.)

`on_gpu=0` dice la verdad: **lo calculó la CPU**. Ese bit sólo vale 1 cuando el
resultado viene del silicio de una GPU. El dispositivo se apaga al terminar el
comando.

La **fase** sale vacía aquí porque el dispositivo de software no tiene bring-up que
recorrer. Con una GPU NVIDIA de verdad dice hasta dónde llegó (`booted`, `fallo`, `rm_ce`,
`rm_compute`…), que es lo que convierte un `on_gpu=0` en un diagnóstico sin tener que
leer el log de serie.

Para generar modelos sintéticos de prueba de cualquier tamaño:

```sh
cargo run --release -p mkmodel-soso -- target/big-model \
  --hidden 2048 --ffn 5632 --layers 10 --vocab 32000 --heads 32 --kv-heads 8
```

Opciones adicionales de arquitectura y MoE:

| Flag | Valores | Uso |
|------|---------|-----|
| `--moe` | flag | Modelo MoE (router + expertos) |
| `--experts N` | entero | Número de expertos routed |
| `--experts-per-tok K` | entero | Top-K del router |
| `--moe-ffn D` | entero | Dimensión FFN por experto (o espacio latente con `--ffn-kind latent-moe`) |
| `--shared-experts S` | entero | Expertos compartidos `Lxx.Syy.*` por capa |
| `--attn` | `gqa` (default), `mla`, `kda` | Atención GQA, MLA (tensores `attn_q_down/up`, …) o KDA sintético |
| `--ffn-kind` | `dense`, `moe`, `latent-moe` | FFN denso, MoE Mixtral o LatentMoE (`ffn_latent_in/out`, expertos `[latent,latent]`) |
| `--pack-trunk` | flag | Empaqueta attn+FFN por capa en `Lxx.trunk.tensor` (solo F32) |

Ejemplos:

```sh
# MLA sintético (1 capa, smoke tests)
cargo run --release -p mkmodel-soso -- target/tiny-mla --attn mla --name tiny-mla --layers 1 --hidden 128

# LatentMoE (--moe obligatorio)
cargo run --release -p mkmodel-soso -- target/tiny-latent-moe --moe --ffn-kind latent-moe --layers 1

# MoE con expertos compartidos
cargo run --release -p mkmodel-soso -- target/tiny-moe-sh --moe --shared-experts 2
```

Tests host de arquitecturas extendidas (requieren `--features std`):

```sh
cargo test -p soso-llm-core --features std --test arch_ext
```

Con `--quant q8_0` o `--quant q4_k` los tensores 2D salen cuantizados (los `norm`
se quedan en F32, como en un modelo real), que es lo que hace falta para probar el
camino de pesos cuantizados. Ese modo necesita cada tensor entero en RAM, así que es
para modelos de prueba, no para los de decenas de GB:

```sh
cargo run --release -p mkmodel-soso -- /tmp/tiny-q8 --name tiny --quant q8_0
SOSO_MODELS_DIR=/tmp/tiny-q8 cargo xtask mkfs && cargo xtask build
```

**Q4_K exige `hidden` y `ffn` múltiplos de 256** (el superbloque del formato), y el
generador lo rechaza si no lo son en vez de producir un modelo que falla al
inferir:

```sh
cargo run --release -p mkmodel-soso -- /tmp/tiny-q4k --name tiny --quant q4_k \
  --hidden 256 --ffn 512
```

Cuantizaciones GGUF distintas de F32/F16/Q8_0 (Q4_K…) aún no están
soportadas. Nota: dentro de QEMU sin KVM la velocidad la limita la emulación
TCG, no soso.

---

## Instalar soso en un disco (dual-boot UEFI)

Si tienes un **segundo disco NVMe vacío** (o con una instalación previa de soso)
y la máquina arranca en **UEFI**, puedes instalar soso ahí y elegir entre Linux
y soso al encender. Lo normal es hacerlo **desde el propio soso live, sin pasar
por Linux en ningún momento**.

### Requisitos

| Requisito | Detalle |
|-----------|---------|
| Firmware | UEFI (no BIOS/Legacy en esta versión) |
| Disco destino | NVMe entero, vacío o con soso. **Nunca el disco de Linux** |
| Origen | Pendrive live generado con `cargo xtask package-usb-live` |

**Actualizaciones de kernel:** las instalaciones hechas con un live **anterior** a
0.2.0 pueden no tener los huecos `SOSOKRN.BIN` / `SOSOKRN.MET` en la ESP;
reflashea o reinstala una vez para poder usar `soso-update aplicar` con kernel
y recuperación verificable.

soso solo sabe escribir en discos NVMe (`raw_disk::writable`) y el kernel
rechaza cualquier escritura sobre el disco desde el que arrancó. El disco de
Linux no se toca: ni su tabla de particiones, ni su ESP, ni GRUB.

### Opción A — desde soso live, sin Linux (recomendada)

```sh
# 1. Arranca desde el pendrive live. Sin argumentos, el instalador lista
#    los discos, en qué se están usando, y te pide en cuál instalar:
soso-install
# id  nombre      tamaño  uso
#  0  usb         32.0 GiB  pendrive live (origen)  [no se toca]
#       p1  ESP             26 MiB  boot
#       p2  linux          128 MiB
#       p3  linux         8192 MiB
#  2  nvme0      931 GiB    Linux (ESP, linux, swap)  [ocupado — --force para borrar]
#       p1  ESP            512 MiB
#       p2  linux         900000 MiB
#       p3  swap           16384 MiB
#  3  nvme1        4.0 GiB  vacío  [se puede instalar]
# Elige en qué disco instalar soso.
# disco destino (id o nombre, q cancela): nvme1

# También puedes pasar el disco a mano:
soso-install nvme1 --yes
# o el id: soso-install 3 --yes

# 2. Reinicia SIN quitar el pendrive: el shim UEFI registra la entrada
#    de arranque «soso» en la NVRAM de la placa.

# 3. Apaga, quita el USB y arranca: «soso» está en el menú de la placa (F12),
#    y puedes dejarlo como predeterminado en la BIOS.
```

Qué hace `soso-install`:

1. **Comprueba el destino.** Rechaza el disco de arranque, cualquier disco que
   no sea NVMe y todo disco con particiones de otro sistema (swap, LVM,
   Windows, raíces Linux con GUID propio); las lista y dice para qué se usa
   cada uno antes de negarse. Para sobrescribirlo de todos modos hace falta
   `--force` **y** teclear el nombre del disco.
2. **Clona** el live sobre el destino (ESP, rootfs, SOSOINSTALL y los modelos
   empaquetados). Omite la cola vacía de la partición de modelos y la GPT de
   respaldo de la imagen; al arrancar, `relayout` estira p3 y sosomfs crece el
   superbloque. Mientras copia enseña progreso en MiB/s (lecturas USB de hasta
   512 KiB por transferencia).
3. **Repara la GPT** del destino: el clon describe el pendrive, así que se
   recoloca la cabecera de respaldo al final del disco, la partición de modelos
   se estira hasta llenarlo y se reparten GUID nuevos (si no, el disco sería
   indistinguible del USB para el firmware).
4. **Anota la petición de arranque** en `SOSOBOOT.TXT` de la ESP del pendrive.
   El kernel no puede tocar la NVRAM (ya no hay Runtime Services UEFI cuando
   corre soso), así que lo hace el shim en el siguiente arranque del USB.

`soso-install status` enseña el estado de esa petición: `INSTALL <guid>`
mientras está pendiente y `DONE Boot0007 soso` cuando el shim la ha atendido.

### Opción B — desde Linux (cargo)

```sh
lsblk                                        # identifica el disco vacío
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask install-disk /dev/nvme1n1 --yes
```

Añade además `/etc/grub.d/41_soso` (chainload a `BOOTX64.EFI`) y ejecuta
`update-grub`. Con `--no-grub` deja el snippet en `target/install-disk/41_soso`.

### Opción C — USB live con instalador (sin cargo en el equipo destino)

```sh
# En la máquina de desarrollo:
sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes   # mide el stick y empaqueta el mejor modelo que quepa

# Escalera automática (Q4_K_M):
#   8 GB  → tinyllama
#  16 GB  → mistral-7b
#  32 GB+ → qwen3.8-27b
# La primera vez descarga desde Hugging Face (puede tardar horas en modelos grandes).
# Sin descargas: el mayor ya materializado que quepa en el stick:
# sudo env SOSO_LIVE_OFFLINE=1 "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes

# Sin pendrive conectado (qwen3.8-27b) o simular capacidad:
cargo xtask package-usb-live
SOSO_LIVE_CAPACITY=64G cargo xtask package-usb-live

# En placa: ask  o  soso-llm run <modelo> --prompt "hola" --max 32
# (<modelo> = el empaquetado: tinyllama, mistral-7b o qwen3.8-27b)

# Override manual:
# SOSO_MODELS_DIR=target/mi-modelo cargo xtask flash-usb-live /dev/sdX --yes

# Tras el primer flash: actualizar solo kernel/rootfs (sin reescribir modelos):
# sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask flash-usb-live /dev/sdX --yes --skip-models

# Instalar en disco interno desde Linux (USB conectado):
lsblk
sudo /media/$USER/SOSOINSTALL/install-soso.sh /dev/nvme1n1 --yes
```

### Si tu firmware ignora la entrada nueva

Algunas placas rehacen el orden de arranque por su cuenta. La entrada sigue
existiendo: elige el disco en el menú de arranque (F12) o súbela en la BIOS.
Y si prefieres arrancar soso desde el GRUB de Linux, con el USB conectado:

```sh
sudo /media/$USER/SOSOINSTALL/install-soso.sh --grub-only /dev/nvme1n1
```

soso no puede hacer esto por sí mismo: `grub.cfg` vive en la partición ext4 de
Linux y soso no escribe ext4 — que es justo lo que no debe tocar.

### Arranque

Reinicia. Elige **soso** (o Linux) en el menú de la placa. Dentro de soso:
consola serie, pantalla, o SSH (`ssh -i target/soso_test_key soso@<ip>`).

### Desinstalar

Borra la entrada desde la BIOS de la placa (o el disco entero). Si añadiste la
entrada de GRUB:

```sh
sudo rm /etc/grub.d/41_soso
sudo update-grub
```

Linux no se modifica en ningún caso.

---

## Resumen rápido

```sh
# --- QEMU (desarrollo) ---
cargo xtask run
ssh -tt -i target/soso_test_key -p 2222 soso@localhost

# --- Dentro de sosh ---
soso-update estado          # medio de arranque, versión rootfs, kernel, buzón
pwd
cd /tmp
echo hola > nota.txt
cat nota.txt
ls /
ls /models

# LLM
ask ¿cuánto es 2 > 1?       # texto literal al modelo (askd en :7420)
ask                         # modo interactivo
ask-modelo                  # ver/cambiar modelo de ask
soso-llm run tiny --prompt hola

# Modelos desde la red (requiere HTTPS)
soso-hf search llama
soso-hf pull org/repo --name mi-modelo

# Voz y web
voz                         # dictado → línea (Enter confirma)
soso-web --local /etc/web-prueba.html
soso-web https://example.com

# Red (placa real)
wifi scan
wifi connect MiRed MiClaveWPA2

# Instalación / actualización (live o NVMe instalado)
soso-install                  # elige disco; list para solo mirar
soso-update comprobar
soso-update aplicar           # reiniciar después

cat /etc/motd
mkdir prueba
halt

# Salir de QEMU
# Ctrl-A X
```

**Pendrive live:** grabar con `cargo xtask flash-usb-live /dev/sdX --yes` (desde
Linux, en el repo). Log del último arranque: `cargo xtask sosolog`.
