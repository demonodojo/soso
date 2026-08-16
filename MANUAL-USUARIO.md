# Manual de usuario — soso

**soso** es un sistema operativo minimalista de aprendizaje: kernel propio en Rust,
filesystem copy-on-write con checksums (sosofs) y acceso remoto por SSH real.
Es **monousuario**: una sola sesión SSH a la vez, sin permisos ni cuentas múltiples.

Este manual describe cómo arrancar el sistema, conectarte y usar la shell y los
comandos disponibles.

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

Desde el directorio del proyecto:

```sh
cargo xtask run
```

Este comando compila el kernel, genera la imagen del disco con el contenido de
`rootfs/`, y lanza QEMU. La consola del sistema aparece en la misma terminal.

Comandos relacionados:

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

### 2. SSH (acceso remoto cifrado)

Con soso en marcha, abre **otra terminal** en el anfitrión:

```sh
ssh -tt -i target/soso_test_key -p 2222 soso@localhost
```

- **Puerto:** 2222 (redirigido al puerto 22 interno de soso).
- **Usuario:** `soso` (el nombre es convencional; la autenticación es solo por clave).
- **Cifrado:** SSH-2 con curve25519, ed25519 y chacha20-poly1305.

Al conectar verás el mensaje del día (`/etc/motd`) y luego la misma shell **sosh**
que en la consola serie. Puedes desconectar con Ctrl-C o `exit` y volver a conectar;
solo hay **una sesión SSH a la vez**.

**Clave de acceso:** al construir la imagen, se inyecta una clave pública ed25519 en
`/etc/authorized_key` del disco:

- Si existe `~/.ssh/id_ed25519.pub`, se usa esa clave.
- Si no, se genera un par de prueba en `target/soso_test_key` (y su `.pub`).

Para usar tu propia clave SSH, asegúrate de tener `~/.ssh/id_ed25519.pub` antes de
compilar, o regenera la imagen con `cargo xtask mkfs`.

> **Nota:** la primera conexión puede avisar de una host key desconocida. La host key
> del servidor se guarda en `/etc/ssh_host_key` dentro del disco y persiste entre
> reconstrucciones de la imagen si no borras el disco de datos.

### 2b. WiFi (Intel AX211, arranque en hardware real)

En placa con tarjeta **Intel Wi-Fi 6E AX211** (PCI `8086:7f70`), soso puede usar WiFi
en lugar de Ethernet cableada si compilas con la capa **lxdde/iwlwifi**:

```sh
cargo xtask lx-build iwlwifi
SOSO_LXDDE=1 SOSO_LXDDE_MODE=iwlwifi cargo xtask build
# USB live en la máquina objetivo:
SOSO_LXDDE=1 SOSO_LXDDE_MODE=iwlwifi cargo xtask flash-usb-live /dev/sdX --yes
```

**Configuración** en `/etc/wifi.conf` (claves `ssid=` y opcionalmente `psk=`):

```ini
ssid=MiRed
psk=MiClaveWPA2
```

El firmware va en `/lib/firmware/iwlwifi-so-a0-gf-a0-89.ucode` y `.pnvm` (incluidos
en `rootfs/lib/firmware/`).

**Consola de emergencia (kernel-shell):**

```text
wifi scan          # listar redes
wifi status        # estado del driver
wifi connect Red   # red abierta
wifi connect Red clave  # WPA2-PSK
```

**Prueba con VFIO** (passthrough del dispositivo WiFi a QEMU):

```sh
sudo ./scripts/l6-wifi-vfio-test.sh
```

La red WiFi tiene prioridad sobre virtio-net **solo si no hay Ethernet cableada**.
DHCP y SSH funcionan igual que con virtio/e1000e.

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

Ejemplos:

```sh
help
pwd
cd /tmp
cd ..                       # sube al directorio padre
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

### ask — preguntarle al modelo

```sh
ask ¿por qué el cielo es azul?
ask                                # modo interactivo
```

Escribe la pregunta detrás y ya está: **el texto llega al modelo tal cual se
escribió**, con comillas, tildes, `|`, `>` o lo que lleve. La respuesta sale por
el terminal según se genera, sin una sola línea de diagnóstico (para eso está
`soso-llm run`).

Sin texto, `ask` abre su propio prompt y el modelo se carga **una sola vez** para
toda la sesión, así que a partir de la segunda pregunta la respuesta empieza
mucho antes:

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
soso-install list          # discos y particiones de cada uno
soso-install 3 --yes       # instalar en el disco con ese id
soso-install status        # estado de la entrada de arranque UEFI
```

Solo tiene sentido arrancando desde el pendrive live. Ver
[Instalar soso en un disco](#instalar-soso-en-un-disco-dual-boot-uefi).

---

## Estructura del disco

Tras el arranque, el filesystem **sosofs** expone al menos:

```
/
├── bin/          # Programas (init, sosh, ls, cat, soso-llm, …)
├── etc/
│   ├── motd              # Mensaje de bienvenida
│   ├── authorized_key    # Clave pública ed25519 autorizada (32 bytes)
│   ├── llm.conf          # Modelo y límites que usa `ask` (ver ask-modelo)
│   └── ssh_host_key      # Semilla de la host key del servidor SSH
├── models/       # Modelos LLM (disco sosomfs, solo lectura)
│   ├── tiny/             # Modelo sintético denso (4 capas)
│   ├── tiny-moe/         # Modelo MoE sintético (4 expertos, top-2)
│   ├── tiny-mla/         # Modelo MLA sintético (1 capa, KV latente)
│   └── tiny-latent-moe/  # Modelo LatentMoE sintético (1 capa, vocab 64)
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
| `halt` | Apagar |

La kernel-shell también incluye comandos de bajo nivel para depuración (`hwscan`,
`blk`, `blkread`, `blkwrite`, `pf`, `panic`). Están pensados para desarrollo,
no para uso habitual.

### Autodescubrimiento de drivers (`hwscan`)

El comando **`hwscan`** enumera dispositivos PCI y muestra qué driver los
atendería y si está compilado en el kernel actual. Formato de cada línea:

```
drv: bb:dd.f VVVV:DDDD nombre-driver compilado|ausente
```

En arranque **live**, si falta algún driver para el hardware detectado, el
informe se imprime también por serie y se guarda en **`SOSODRV.TXT`** en la ESP
(junto a `SOSOLOG.TXT`). En el PC de desarrollo:

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
cargo xtask sosolog | less
```

Regenera la imagen live tras actualizar el kernel:
`cargo xtask package-usb-live` (incluye el fichero pre-creado en la ESP).

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

soso es un sistema de aprendizaje con un alcance deliberadamente reducido:

| Área | Limitación |
|---|---|
| Usuarios | Monousuario; una sesión SSH simultánea |
| Shell | Sin variables ni historial; cwd, pipes y redirecciones |
| Procesos | `spawn`, no `fork`; scheduler round-robin preemptivo |
| Red | DHCP automático al arrancar; fallback a `10.0.2.15` en QEMU; sin IPv6 |
| SSH | Sin SFTP, port forwarding ni múltiples sesiones |
| Ficheros | Sin permisos Unix, hardlinks ni snapshots |
| Comandos | Conjunto mínimo de coreutils |

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
- Tras desconectar con Ctrl-C puedes reconectar; si falla, espera un segundo o reinicia QEMU.

### La consola no responde

- QEMU usa `-serial mon:stdio`. Escribe en la misma terminal donde lanzaste `run`.
- Para salir: `Ctrl-A X` (no `Ctrl-C`).

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

El offload GPU (sin `--cpu`) descuantiza al subir pesos **F32, Q8_0, Q4_K y MXFP4** a VRAM. Con `--gpu-soft` ejercitas esa fontanería sin silicio NVIDIA.

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
token) con cache LRU de expertos calientes entre tokens. También aplica ventana
**StreamingLLM** / **H2O** en el KV (sink + tokens de mayor atención + recientes)
y, con contextos largos, atención sparse por bloques (**Quest-lite**). Al
arrancar y al terminar verás:

```text
soso-llm: planificador — presupuesto pesos … KiB, modelo … KiB, capas CPU/GPU/remoto …
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

El prompt admite varias palabras (hasta el siguiente flag); sosh no
interpreta comillas.

### GPU NVIDIA nativa (cuando hay dGPU en QEMU)

Si arrancas soso con la GPU pasada por VFIO (`SOSO_QEMU_GPU=vfio:…`), el bring-up
GSP puede dejar el compute listo (`rm_compute`). Entonces `soso-llm run` usa la
GPU sola, sin flags extra: los matvec salen con `on_gpu=1` y el resumen final
cuenta trabajo en silicio. En la RTX 5070 Ti Mobile (GB205) esto ya funciona para
modelos pequeños (`tiny` con `--max 4`). Si el GSP no llega a compute, `soso-llm`
sigue en CPU sin que tengas que hacer nada.

### `--gpu-soft`: el camino de la GPU sin GPU

Con `--gpu-soft`, soso enciende un dispositivo de cómputo de mentira que calcula
en la CPU del kernel, y `soso-llm` manda los matvec por las mismas syscalls que
usaría con una GPU de verdad. **No acelera nada** —es más lento que el backend de
CPU normal, porque los datos van y vienen por syscalls— y sirve para dos cosas:

- comprobar en cualquier máquina que la fontanería del offload funciona (reservar
  búferes, subir pesos, lanzar, leer el resultado);
- ver el resumen que imprime al final: matvec lanzados, **subidas de pesos** y
  matrices residentes. Si las subidas fueran tantas como los matvec, los pesos se
  estarían resubiendo en cada token.

```sh
soso-llm run tiny --prompt test --gpu-soft --max 4
```

```
soso-llm: dispositivo de cómputo «soft (CPU del kernel, pruebas)» (fase ), VRAM libre 268435456 bytes
soso-llm: generado (6 tokens, 5450 ms, 1.10 tok/s)
soso-llm: dispositivo «soft (CPU del kernel, pruebas)» — 144 matvec, 24 subidas de pesos, 24 matrices residentes, último on_gpu=0
soso-llm: el silicio no calculó nada — el GSP se quedó en la fase «»
```

`on_gpu=0` dice la verdad: **lo calculó la CPU**. Ese bit sólo vale 1 cuando el
resultado viene del silicio de una GPU. El dispositivo se apaga al terminar el
comando.

La **fase** sale vacía aquí porque el dispositivo de software no tiene bring-up que
recorrer. Con una GPU NVIDIA de verdad dice hasta dónde llegó (`booted`, `rm_ce`,
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

soso solo sabe escribir en discos NVMe (`raw_disk::writable`) y el kernel
rechaza cualquier escritura sobre el disco desde el que arrancó. El disco de
Linux no se toca: ni su tabla de particiones, ni su ESP, ni GRUB.

### Opción A — desde soso live, sin Linux (recomendada)

```sh
# 1. Arranca desde el pendrive live y mira qué hay en cada disco:
soso-install list
# id  nombre   sectores      tamano  contenido
#  0  usb          843776     412 MiB  soso ro boot
#       p1  ESP             26 MiB  boot
#       p2  linux          128 MiB
#       p3  linux          257 MiB
#  2  nvme0    1953525168  953869 MiB  OTRO
#       p1  ESP            512 MiB
#       p2  linux         900000 MiB
#       p3  swap           16384 MiB
#  3  nvme1       8388608    4096 MiB  vacio

# 2. Instala en el disco vacío (aquí el id 3):
soso-install 3 --yes

# 3. Reinicia SIN quitar el pendrive: el shim UEFI registra la entrada
#    de arranque «soso» en la NVRAM de la placa.

# 4. Apaga, quita el USB y arranca: «soso» está en el menú de la placa (F12),
#    y puedes dejarlo como predeterminado en la BIOS.
```

Qué hace `soso-install`:

1. **Comprueba el destino.** Rechaza el disco de arranque, cualquier disco que
   no sea NVMe y todo disco con particiones de otro sistema (swap, LVM,
   Windows, raíces Linux con GUID propio); las lista antes de negarse. Para
   sobrescribirlo de todos modos hace falta `--force` **y** teclear el nombre
   del disco.
2. **Clona** el pendrive entero sobre el destino.
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
sudo cargo xtask install-disk /dev/nvme1n1 --yes
```

Añade además `/etc/grub.d/41_soso` (chainload a `BOOTX64.EFI`) y ejecuta
`update-grub`. Con `--no-grub` deja el snippet en `target/install-disk/41_soso`.

### Opción C — USB live con instalador (sin cargo en el equipo destino)

```sh
# En la máquina de desarrollo (TinyLlama por defecto; fetch-hf solo la primera vez):
cargo xtask fetch-hf TinyLlama/TinyLlama-1.1B-Chat-v1.0   # si falta target/tinyllama-model
cargo xtask package-usb-live
sudo cargo xtask flash-usb-live /dev/sdX --yes   # live + partición SOSOINSTALL

# En placa: ask  o  soso-llm run tinyllama --prompt "hola" --max 32

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
# Arrancar
cargo xtask run

# En otra terminal: SSH
ssh -i target/soso_test_key -p 2222 soso@localhost

# Dentro de sosh
pwd
cd /tmp
echo hola > nota.txt
cat nota.txt
ls /
ls /models
ask ¿cuánto es 2 > 1?       # el texto va literal al modelo
ask                         # modo interactivo
ask-modelo                  # ver/cambiar el modelo que usa ask
soso-llm run tiny --prompt hola
soso-llm run tiny-moe --prompt @bos --max 4
cat /etc/motd
mkdir prueba
halt

# Salir de QEMU
# Ctrl-A X
```
