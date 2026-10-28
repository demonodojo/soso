# Plan — paquetes de aplicaciones

Gestor incremental de aplicaciones en userspace, **independiente** del OTA
(`soso-update`). El OTA sigue administrando `bin/`, `lib/` y `etc/` del sistema;
las apps instaladas viven en `/var/paquetes/` y no viajan en el `rootfs.pack`.

## Estado

| Fase | Contenido | Estado |
|------|-----------|--------|
| P0 | Contrato `paquete.txt` / `paquete.pack`, índice, `soso-pkg-core`, cliente `soso-pkg` (local + HTTPS), índice de comandos, sosh | Hecho |
| P1 | Repositorio publicado, `cargo xtask pkg-pack`, dependencias entre paquetes | Pendiente |
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

## Verificación

- Host: `cargo test -p soso-pkg-core --features std`
- Guest: fase `soso-pkg` en `cargo xtask test` (fixture en `/share/paquetes-prueba`)

## Relación con otros planes

- [PLAN-ACTUALIZACIONES.md](PLAN-ACTUALIZACIONES.md): no mezclar transacción OTA con paquetes.
- [MANUAL-USUARIO.md](../MANUAL-USUARIO.md): sección `soso-pkg`.
