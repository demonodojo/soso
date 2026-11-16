# Plan — paquetes de aplicaciones

Gestor incremental de aplicaciones en userspace, **independiente** del OTA
(`soso-update`). Los paquetes normales viven en `/var/paquetes/`; los que
declaran `destino=sistema` pueden publicar sus ejecutables en `/bin` sin pasar
por una release OTA. Se guarda copia del binario previo para poder desinstalar.

## Estado

| Fase | Contenido | Estado |
|------|-----------|--------|
| P0 | Contrato `paquete.txt` / `paquete.pack`, índice, `soso-pkg-core`, cliente `soso-pkg` (local + HTTPS), índice de comandos, sosh | Hecho |
| P1 | Repositorio publicado, `cargo xtask pkg-pack`, paquetes `soso-ed`, `soso-pkg`, `soso-wm` y `sal` con destino sistema | En curso: índice público raw de GitHub y paquetes de sistema publicados en `rootfs/share/paquetes/`; `soso-web` y `opencode-sal` siguen empaquetables |
| P2 | Servicios/demonios, `.desktop` / escritorio | Pendiente |
| P3 | Descarga por rangos (packs > 64 MiB) | Pendiente |

## Contrato (P0)

Manifiesto `SOSOPKG v1`, índice `SOSOPKG indice v1`, rutas bajo `bin/` y
`share/`, tope 64 MiB. `destino=sistema` (opcional y compatible con gestores
anteriores) instala `bin/*` además de guardarlo en el árbol privado del paquete.

Rutas en disco:

- `/var/paquetes/<nombre>/` — árbol instalado + copia de `paquete.txt`
- `/var/paquetes/comandos` — mapa `nombre → ruta` para sosh
- `/etc/paquetes.conf` — `url=` base del repositorio HTTPS
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

## Paquetes con ejecutables del sistema

- `soso-ed`, `soso-pkg`, `soso-wm` y `sal` declaran `destino=sistema`; el gestor conserva también
  los binarios en `/var/paquetes/<nombre>/bin/` para rollback y compatibilidad.
- El gestor nuevo activa automáticamente en `/bin` los paquetes de sistema que
  instaló un gestor anterior bajo `/var/paquetes`.
- `cargo xtask pkg-pack soso-ed|soso-pkg|soso-wm|sal` genera su manifiesto y
  pack. El paquete `sal` contiene el compilador nativo, y usa el SDK/backend
  que instala la imagen base en `/lib/sal`. La compilación genera los paquetes
  disponibles y actualiza `indice.txt`.

## Verificación

- Host: `cargo test -p soso-pkg-core --features std`
- Host: `cargo xtask pkg-pack soso-web` (tras compilar userspace)
- Guest: fase `soso-pkg` en `cargo xtask test` (fixture en `/share/paquetes-prueba`);
  fases `soso-web` / `soso-wm` usan rutas bajo `/var/paquetes/soso-web/`

## Relación con otros planes

- [PLAN-ACTUALIZACIONES.md](PLAN-ACTUALIZACIONES.md): no mezclar transacción OTA con paquetes.
- [MANUAL-USUARIO.md](../MANUAL-USUARIO.md): sección `soso-pkg`.
