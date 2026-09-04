---
name: soso-user-manual
description: >-
  Maintain MANUAL-USUARIO.md with soso end-user documentation. Use when adding
  or changing user-visible commands, SSH access, shell behavior, filesystem
  layout, or when the user asks for user-facing docs or a manual update.
---

# soso — Manual de usuario

Documentación para **usuarios finales** (no desarrolladores). Lenguaje claro en **español**.

## Archivo canónico

- **Ruta:** [`MANUAL-USUARIO.md`](../../../MANUAL-USUARIO.md) (raíz del repo)
- **No duplicar** en README ni en otras skills: enlazar a este fichero.

## Cuándo actualizar (obligatorio)

Actualiza `MANUAL-USUARIO.md` en la **misma tarea** que cambia la funcionalidad visible:

| Cambio en código | Acción en el manual |
|------------------|---------------------|
| Nuevo comando en `/bin` o builtin de sosh | Documentar con sintaxis y ejemplos |
| Cambio de puertos, SSH o auth | Actualizar sección de acceso |
| Nuevo fichero en rootfs visible al usuario | Actualizar estructura del disco |
| Cambio de cwd, rutas relativas o redirecciones | Actualizar sosh y ejemplos |
| Modelos LLM o `soso-llm` | Sección `/models`, planificador/streaming, conversión GGUF |
| Etapa de `/loop` que cambie strings o UX de `soso-llm` | Actualizar en **esa misma etapa** (no aplazar) |
| Solo refactor interno sin cambio de UX | No tocar el manual |
| Cambio solo de kernel-shell de depuración | Solo si afecta comandos que el usuario final usa |

## Workflow

1. **Identificar** qué ve o usa el usuario: sosh, SSH, comandos de `/bin`, mensajes (motd).
2. **Editar** la sección correspondiente del manual; no incluir detalle de implementación (syscalls, structs del kernel, etc.).
3. **Verificar** que los ejemplos de terminal coinciden con el comportamiento real en QEMU.
4. **Revisar** el resumen rápido al final si añades comandos nuevos.

## Estilo

- Segunda persona o imperativo («Ejecuta», «Conecta»).
- Tablas para comandos, puertos y opciones de `cargo xtask`.
- Bloques de código con comandos completos (sin `...`).
- Sin jerga de desarrollo (CoW, ring 3, sunset, etc.) salvo en la introducción breve.
- Mencionar limitaciones conocidas cuando afecten al usuario (monousuario, etc.).

## Comandos documentados hoy

Referencia rápida — ampliar el manual si cambian:

- **Panorama:** tabla «¿Qué incluye soso?» al inicio (sosh, ask, voz, web, install, update, …)
- **`ask`:** modelo residente en `soso-llm askd` (`127.0.0.1:7420`); persiste entre SSH
- **`voz` / `soso-voz`:** dictado vía `vozd` (`127.0.0.1:7421`); F4 push-to-talk; `/etc/voz.conf`
- **`soso-web`:** HTTPS o `--local`; `--grafico` con framebuffer
- **`soso-update`:** `estado`, `comprobar`, `aplicar`, `revertir`; `/etc/actualiza.conf`, `/etc/soso-release`
- **`soso-install`:** `list`, `<id> --yes`, `status`; buzón `SOSOBOOT.TXT`
- **`soso-hf`:** `search`, `list`, `pull` (modelos en `/models/` desde el guest)
- **Arranque:** QEMU `cargo xtask run`; live `flash-usb-live`; salida `Ctrl-A X`
- **Log USB live:** `cargo xtask sosolog` / `--drv` (ESP p1; no `sudo cargo`)
- **SSH:** `ssh -tt -i target/soso_test_key -p 2222 soso@localhost` (QEMU); puerto 22 en placa
- **Shell:** sosh (`help`, `exit`, `cd`, `pwd`, `wifi`, `ask`, `voz`, pipes, redirecciones)
- **Coreutils:** `ls`, `cat`, `echo`, `mkdir`, `rm`, `hexdump`, `halt`
- **LLM:** `soso-llm run …`; modelos en `/models/`; live escala modelo según tamaño del stick
- **Red:** echo TCP `nc localhost 7777`; live: Realtek 8168 o WiFi AX211/AX200

## Skills compartidas

Las skills viven en `.claude/skills/`. Cursor las descubre vía `.cursor/skills`. Editar bajo `.claude/skills/`.
