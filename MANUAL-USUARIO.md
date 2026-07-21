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
> arranques si no borras la imagen de datos.

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
cat /etc/motd | hexdump
ls | cat
cat < /etc/motd
```

### Comandos integrados (builtins)

| Comando | Descripción |
|---|---|
| `help` | Muestra la ayuda |
| `cd <dir>` | Cambia el directorio de trabajo |
| `pwd` | Imprime el directorio de trabajo actual |
| `exit` | Cierra la shell (código de salida opcional, por defecto 0) |

Ejemplos:

```sh
help
pwd
cd /tmp
cd ..                       # sube al directorio padre
exit
exit 1
```

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
```

Ejecuta inferencia greedy sobre el modelo en `/models/tiny/`. Ver sección
[Modelos LLM](#modelos-llm-soso-llm) para importar modelos y más detalle.

---

## Estructura del disco

Tras el arranque, el filesystem **sosofs** expone al menos:

```
/
├── bin/          # Programas (init, sosh, ls, cat, soso-llm, …)
├── etc/
│   ├── motd              # Mensaje de bienvenida
│   ├── authorized_key    # Clave pública ed25519 autorizada (32 bytes)
│   └── ssh_host_key      # Semilla de la host key del servidor SSH
├── models/       # Modelos LLM (disco sosomfs, solo lectura)
│   └── tiny/             # Modelo sintético de prueba
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
| `ls [ruta]` | Listar directorio (por defecto `/`) |
| `cat <ruta>` | Mostrar un fichero |
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

La kernel-shell también incluye comandos de bajo nivel para depuración (`blk`,
`blkread`, `blkwrite`, `pf`, `panic`). Están pensados para desarrollo, no para uso
habitual.

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
```

### Modelo de prueba incluido

Al arrancar con `cargo xtask run`, se genera automáticamente el modelo sintético **tiny** (4 capas, hidden 128). Puedes ejecutar inferencia desde **sosh**:

```sh
soso-llm run tiny --prompt hola
```

La salida muestra el texto generado con decode greedy. El modelo tiny usa un
tokenizer byte-level; los modelos importados de GGUF usan su propio
vocabulario (`tokenizer.som`).

### Importar un modelo GGUF

En la máquina anfitriona, convierte un fichero GGUF de arquitectura llama al
layout `.som`. Se soportan tensores **F32, F16, Q8_0 y Q4_K** (los Q4_K_M
descargables funcionan tal cual; sus tensores Q6_K se convierten a Q8_0),
GQA, SwiGLU con `ffn_gate` y el vocabulario del tokenizer. Verificado con
TinyLlama-1.1B-Chat Q4_K_M:

```sh
cargo xtask convert-gguf ruta/al/modelo.gguf target/mi-modelo --name mi-modelo
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

### Memoria, CPU y modelos grandes

QEMU arranca por defecto con 2 GiB y 1 CPU; ambos son configurables:

```sh
SOSO_QEMU_MEM=16G SOSO_QEMU_SMP=4 cargo xtask run
```

Los pesos se leen **sin copia** directamente del mmap del modelo (páginas de
2 MiB bajo demanda) y el KV cache va en f16: el límite de tamaño de modelo es
la RAM que le des a QEMU (ventana de mapeo de ~416 GiB). La imagen de modelos
se dimensiona con `SOSO_MODELS_SIZE` (por defecto 8G) si el modelo no cabe.

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

El prompt admite varias palabras (hasta el siguiente flag); sosh no
interpreta comillas.

Para generar modelos sintéticos de prueba de cualquier tamaño:

```sh
cargo run --release -p mkmodel-soso -- target/big-model \
  --hidden 2048 --ffn 5632 --layers 10 --vocab 32000 --heads 32 --kv-heads 8
```

Cuantizaciones GGUF distintas de F32/F16/Q8_0 (Q4_K…) aún no están
soportadas. Nota: dentro de QEMU sin KVM la velocidad la limita la emulación
TCG, no soso.

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
soso-llm run tiny --prompt hola
cat /etc/motd
mkdir prueba
halt

# Salir de QEMU
# Ctrl-A X
```
