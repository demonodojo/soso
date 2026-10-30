# Plan — paquetes de aplicaciones

Gestor incremental de aplicaciones en userspace, **independiente** del OTA
(`soso-update`). El OTA sigue administrando `bin/`, `lib/` y `etc/` del sistema;
las apps instaladas viven en `/var/paquetes/` y no viajan en el `rootfs.pack`.

## Estado

| Fase | Contenido | Estado |
|------|-----------|--------|
| P0 | Contrato `paquete.txt` / `paquete.pack`, índice, `soso-pkg-core`, cliente `soso-pkg` (local + HTTPS), índice de comandos, sosh | Hecho |
| P1 | Repositorio publicado, `cargo xtask pkg-pack`, paquete referencia `soso-web`, dependencias entre paquetes | En curso (`pkg-pack`, `soso-web` fuera del OTA; **`opencode-sal`** empaquetable vía `cargo xtask pkg-pack opencode-sal`) |
| P2 | Servicios/demonios, `.desktop` / escritorio | Pendiente |
| P3 | Descarga por rangos (packs > 64 MiB) | Pendiente |

## Contrato (P0)

Ver el plan de implementación acordado en el repositorio (manifiesto `SOSOPKG v1`,
índice `SOSOPKG indice v1`, rutas bajo `bin/` y `share/`, tope 64 MiB).

Rutas en disco:

- `/var/paquetes/<nombre>/` — árbol instalado + copia de `paquete.txt`
- `/var/paquetes/comandos` — mapa `nombre → ruta` para sosh
- `/etc/paquetes.conf` — `url=` base del repositorio HTTPS

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
- **Repo Git separado** para el navegador: opcional; el monorepo sigue siendo la
  fuente hasta publicar un índice HTTPS estable.

## Verificación

- Host: `cargo test -p soso-pkg-core --features std`
- Host: `cargo xtask pkg-pack soso-web` (tras compilar userspace)
- Guest: fase `soso-pkg` en `cargo xtask test` (fixture en `/share/paquetes-prueba`);
  fases `soso-web` / `soso-wm` usan rutas bajo `/var/paquetes/soso-web/`

## Relación con otros planes

- [PLAN-ACTUALIZACIONES.md](PLAN-ACTUALIZACIONES.md): no mezclar transacción OTA con paquetes.
- [MANUAL-USUARIO.md](../MANUAL-USUARIO.md): sección `soso-pkg`.
