# Plan — paquetes de aplicaciones

Gestor incremental de aplicaciones en userspace, **independiente** del OTA
(`soso-update`). Los paquetes normales viven en `/var/paquetes/`; los que
declaran `destino=sistema` pueden publicar ejecutables en `/bin` y archivos del
SDK de Sal en `/lib/sal` sin pasar por una release OTA. Se guardan los archivos
previos para poder restaurarlos al desinstalar.

## Estado

| Fase | Contenido | Estado |
|------|-----------|--------|
| P0 | Contrato `paquete.txt` / `paquete.pack`, índice, `soso-pkg-core`, cliente `soso-pkg` (local + HTTPS), índice de comandos, sosh | Hecho |
| P1 | Repositorio publicado, `cargo xtask pkg-pack`, paquetes `soso-ed`, `soso-pkg`, `soso-update`, `soso-wm` y `sal` con destino sistema | En curso: índice público raw de GitHub y paquetes de sistema publicados en `rootfs/share/paquetes/`; `soso-update` se actualiza por este gestor y se excluye de los packs OTA; `soso-web` y `opencode-sal` siguen empaquetables; **varios orígenes** (`soso-pkg` 0.4.0) y `sal` publicado desde su propio repositorio |
| P2 | Servicios/demonios, `.desktop` / escritorio | Pendiente |
| P3 | Descarga por rangos (packs > 64 MiB) | Pendiente |

## Contrato (P0)

Manifiestos `SOSOPKG v1` (sin dependencias) y `SOSOPKG v2` (con `dep=<nombre> <restricción SemVer>`), índice `SOSOPKG indice v1`, rutas bajo `bin/`,
`share/` y `lib/sal/`, tope 64 MiB. `destino=sistema` instala `bin/*` en `/bin`
y `lib/sal/*` en `/lib/sal`,
además de guardarlos en el árbol privado del paquete. Los destinos de sistema
quedan limitados a esas dos áreas. El instalador resuelve dependencias del
índice antes del paquete raíz, comprueba restricciones exactas, comparadores,
`^`, `~` y conjunciones, y evita borrar o degradar dependencias de paquetes
instalados. Los gestores anteriores rechazan manifiestos v2 al no reconocer su
cabecera.

Rutas en disco:

- `/var/paquetes/<nombre>/` — árbol instalado + copia de `paquete.txt`
- `/var/paquetes/comandos` — mapa `nombre → ruta` para sosh
- `/etc/paquetes.conf` — `url=` base del repositorio HTTPS de soso y una línea
  `origen=` por cada repositorio extra (ver «Varios orígenes»)

## Varios orígenes

`soso-pkg` 0.4.0 junta los índices de varios repositorios, en orden de
prioridad: primero `url=` (el de soso) y luego cada `origen=`. Sin ninguna
línea `origen=` usa los de por defecto (`ORIGENES_POR_DEFECTO` en
`soso-pkg-core`: el de Sal); `origen=ninguno` deja sólo el principal. Gana la
versión más alta de cualquier origen y, si empatan, el primero. Las
dependencias se resuelven en el conjunto: `sal` (repositorio de Sal) pide
`soso-pkg` (repositorio de soso). Un origen que no responde se avisa (`sigo sin
el origen …`) y se sigue con los demás. `--origen DIR` se puede repetir.

Compatibilidad: la clave es nueva (`origen=`) para que un `soso-pkg` < 0.4.0,
que sólo entiende `url=`, siga leyendo el repositorio de soso; para llegar a
`sal` primero hay que actualizar `soso-pkg`.
- Paquetes de sistema: copia de seguridad en `.respaldo/` dentro del paquete;
  `soso-pkg` la restaura al desinstalar.

## Comandos de usuario

```text
soso-pkg listar
soso-pkg buscar [texto] [--origen DIR]
soso-pkg instalar <nombre|DIR> [--forzar] [--origen DIR]
soso-pkg actualizar [nombre] [--forzar]
soso-pkg borrar <nombre>
```

## Paquete `soso-web` (referencia P1)

- Host: `cargo xtask pkg-pack soso-web` → `rootfs/share/paquetes/soso-web/1.0.0/`
  (`bin/soso-web`, `share/fonts/DejaVuSans.ttf`, `share/ejemplo/web-prueba.html`).
- Imagen de prueba: `build_user` instala bajo `/var/paquetes/soso-web/` y registra
  `soso-web` en `/var/paquetes/comandos` (sosh y escritorio Alt+W).
- El OTA **no** incluye `/bin/soso-web`; actualizar el navegador con
  `soso-pkg instalar` / `actualizar` sin release completa del sistema.
- **Origen público actual:** `https://raw.githubusercontent.com/demonodojo/soso/main/rootfs/share/paquetes`
  (índice y packs publicados junto al monorepo).

## Paquete `salvi` (aplicación Sal externa)

- Host: `cargo xtask pkg-pack salvi` ejecuta `../salvi/build.sh soso` (necesita el
  SDK de `cargo xtask sal-build --cross-only`) y publica
  `rootfs/share/paquetes/salvi/<versión>/` y su fila en `indice.txt`.
- Sin `destino=sistema` y fuera de la imagen: el usuario lo instala con
  `soso-pkg instalar salvi`; queda en `/var/paquetes/salvi/` y sosh lo resuelve
  por `/var/paquetes/comandos`.
- Prueba en QEMU: `python3 tests/soso_qemu.py` en `salvi` (rootfs propio,
  `soso-pkg instalar salvi --origen /share/paquetes`).

## Paquetes con ejecutables del sistema

- `soso-ed`, `soso-pkg`, `soso-update`, `soso-wm` y `sal` declaran `destino=sistema`; el gestor
  conserva sus archivos en `/var/paquetes/<nombre>/` y copia los originales en
  `.respaldo/` para restaurarlos al desinstalar.
- El gestor nuevo activa automáticamente en `/bin` los paquetes de sistema que
  instaló un gestor anterior bajo `/var/paquetes`.
- `cargo xtask pkg-pack soso-ed|soso-pkg|soso-update|soso-wm` genera su
  manifiesto y pack. La compilación genera los paquetes disponibles y actualiza
  `indice.txt`.
- El ejecutable base `soso-update` se conserva para instalaciones nuevas y el
  USB live, pero `cargo xtask release` lo excluye de `rootfs.pack`: una copia
  instalada como paquete se puede actualizar sin que una OTA la sobrescriba.
  El empaquetado no mueve la transacción al gestor: el cliente sigue armando la
  pareja kernel+rootfs y el arranque confirma o revierte el cambio.

## Paquete `sal` (repositorio de Sal)

- Lo publica el repositorio de Sal en `paquetes/` (índice y packs servidos por
  `https://raw.githubusercontent.com/demonodojo/sal/main/paquetes`), no soso.
  Agrupa compilador nativo, backend LLVM (`sal-llc`) y SDK bajo la versión del
  `Cargo.toml` de Sal; al actualizarlo se renuevan `/bin/sal` y `/lib/sal/`.
  Declara `dep=soso-pkg >=0.3.0` (destinos `lib/sal/`). `lib/sal/origen.txt`
  dice de qué versión, commit y repositorio salió.
- En Sal: `scripts/paquete-soso.sh [etiqueta]` (por defecto `v<versión>`) crea
  un worktree limpio de la etiqueta **junto a soso** (hay versiones que dependen
  de `../soso/crates/sosomodel`) y llama a la receta de soso; luego se commitea
  y empuja `paquetes/`. Una versión publicada no se rehace (`FORZAR=1`).
- La receta es de soso, porque el SDK y la toolchain nativa viven aquí:
  `cargo xtask sal-paquete --fuente <checkout de Sal> --out <dir>` exige el
  checkout sin cambios sin commitear, compila con `scripts/sal-sdk.sh` y
  `scripts/sal-native.sh` en `target/sal-paquete/<versión>` (`SOSO_SAL_OUT`, sin
  pisar el SDK con el que se construye soso) y regenera el `indice.txt` de
  `<dir>`. Necesita la toolchain nativa de soso (docs/SELF-HOSTING.md).
- La imagen de soso sigue llevando `/bin/sal` y `/lib/sal` de
  `cargo xtask sal-build` (pin `config/sal-source.lock`); el paquete es para
  actualizarlo sin una OTA.

## Verificación

- Host: `cargo test -p soso-pkg-core --features std`
- Host: `cargo xtask pkg-pack soso-web` (tras compilar userspace)
- Guest: fase `soso-pkg` en `cargo xtask test` (fixture en `/share/paquetes-prueba`);
  `soso-pkg: varios orígenes` instala `demo` de `origen-b` con su dependencia
  `demo-base` de `origen-a`, tras un origen inexistente;
  fases `soso-web` / `soso-wm` usan rutas bajo `/var/paquetes/soso-web/`

## Relación con otros planes

- [PLAN-ACTUALIZACIONES.md](PLAN-ACTUALIZACIONES.md): no mezclar transacción OTA con paquetes.
- [MANUAL-USUARIO.md](../MANUAL-USUARIO.md): sección `soso-pkg`.
