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
ssh -i target/soso_test_key -p 2222 soso@localhost
```

- **Puerto:** 2222 (redirigido al puerto 22 interno de soso).
- **Usuario:** `soso` (el nombre es convencional; la autenticación es solo por clave).
- **Cifrado:** SSH-2 con curve25519, ed25519 y chacha20-poly1305.

Al conectar verás el mensaje del día (`/etc/motd`) y luego la misma shell **sosh**
que en la consola serie.

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
funciona; no es una shell interactiva.

---

## La shell de usuario: sosh

**sosh** es la shell principal de soso. Cada línea que escribes se interpreta como
un comando.

### Prompt

```
$ 
```

### Comportamiento

- **Una línea = un comando.** No hay pipes (`|`), redirecciones (`>`, `<`) ni
  variables de entorno.
- Los comandos sin ruta se buscan en `/bin/`.
- También puedes invocar un ELF por ruta absoluta (por ejemplo `/bin/init test`).
- **Backspace** funciona para corregir la línea.
- Si un comando falla, sosh muestra el código de salida.

### Comandos integrados (builtins)

| Comando | Descripción |
|---|---|
| `help` | Muestra la ayuda |
| `exit` | Cierra la shell (código de salida opcional, por defecto 0) |

Ejemplos:

```sh
help
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

---

## Estructura del disco

Tras el arranque, el filesystem **sosofs** expone al menos:

```
/
├── bin/          # Programas de userspace (init, sosh, ls, cat, …)
├── etc/
│   ├── motd              # Mensaje de bienvenida
│   ├── authorized_key    # Clave pública ed25519 autorizada (32 bytes)
│   └── ssh_host_key      # Semilla de la host key del servidor SSH
└── hola.txt      # Fichero de ejemplo
```

El mensaje de bienvenida (`/etc/motd`) se muestra al conectar por SSH.

Los cambios que hagas con `mkdir`, escritura de ficheros (desde userspace o desde
la kernel-shell) se persisten en el disco virtual entre arranques, gracias al
filesystem copy-on-write con commits atómicos.

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
ssh -i target/soso_test_key -p 2222 soso@localhost
```

---

## Limitaciones conocidas

soso es un sistema de aprendizaje con un alcance deliberadamente reducido:

| Área | Limitación |
|---|---|
| Usuarios | Monousuario; una sesión SSH simultánea |
| Shell | Sin pipes, redirecciones, variables ni historial |
| Procesos | `spawn`, no `fork`; scheduler round-robin preemptivo |
| Red | IP fija `10.0.2.15`; sin DHCP ni IPv6 |
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

### «Connection refused» en el puerto 2222

- soso debe estar en marcha (`cargo xtask run`).
- Espera a ver el mensaje `sosh — escribe 'help' para la ayuda` antes de conectar.

### La consola no responde

- QEMU usa `-serial mon:stdio`. Escribe en la misma terminal donde lanzaste `run`.
- Para salir: `Ctrl-A X` (no `Ctrl-C`).

### «no existe» al acceder a un fichero

- Usa rutas absolutas (`/etc/motd`, no `etc/motd`).
- Lista el contenido con `ls /` para ver qué hay en el disco.

### Verificar el sistema de extremo a extremo

```sh
cargo xtask test
```

Ejecuta pruebas de integridad del FS, arranque, echo TCP, sesión SSH y apagado limpio.

---

## Resumen rápido

```sh
# Arrancar
cargo xtask run

# En otra terminal: SSH
ssh -i target/soso_test_key -p 2222 soso@localhost

# Dentro de sosh
ls /
cat /etc/motd
echo hola
mkdir /tmp/prueba
halt

# Salir de QEMU
# Ctrl-A X
```
