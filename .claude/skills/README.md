# soso skills

**Un solo directorio real**, `.claude/skills/`. Las otras dos rutas son
**enlaces simbólicos** a él (así están en git, modo `120000`):

| Ruta | Uso | Qué es |
|------|-----|--------|
| `.claude/skills/` | Claude Code / Codex | el directorio de verdad |
| `.cursor/skills/` | Cursor | symlink → `../.claude/skills` |
| `.agents/skills/` | Agentes del repo | symlink → `../.claude/skills` |

No hay nada que sincronizar: editar en `.claude/skills/` y ya está en las tres.
Ojo con escribir «una copia» en `.cursor/` o `.agents/`: sustituiría el enlace
por un directorio de verdad y entonces sí habría tres cosas que mantener.
Comprobarlo con `file .cursor/skills` antes de tocar nada.

| Skill | Purpose |
|-------|---------|
| `soso-dev` | Build, run, `check`, test, debug, `sosolog`, hw-matrix |
| `soso-architecture` | Kernel, sosofs, userspace, syscalls and coding constraints |
| `soso-gpu` | NVIDIA nouveau/GSP (L6, gates G1–G5, VFIO) |
| `soso-wifi` | Intel iwlwifi (AX211/AX200), WPA2, firmware, VFIO |
| `soso-live` | Live USB, ESP slots, boot-shim, install (`soso-install`) |
| `soso-update` | OTA: transacción kernel+rootfs, vuelta atrás, rescate y transición |
| `soso-self-improvement` | Automejora: fichas Txx, SI-0–SI-7, OpenCode, soso-improve |
| `soso-user-manual` | Maintain `MANUAL-USUARIO.md` for end users |

Docs operativos compartidos: [`docs/GUIA-OPERATIVA.md`](../docs/GUIA-OPERATIVA.md),
[`docs/ESTADO.md`](../docs/ESTADO.md), [`docs/HW-MATRIX.md`](../docs/HW-MATRIX.md).

## Rules

- Editar la skill de dominio **en la misma tarea** que el cambio de código.
- Propagar el mismo diff a `.claude/`, `.cursor/` y `.agents/` (no asumir symlink).
- Tras cambios visibles al usuario: actualizar `MANUAL-USUARIO.md` (`soso-user-manual`).
- Tras cambios de tests/comandos: `soso-dev` + tabla de verificación en `soso-architecture`.
- Tras arranque live en placa con SOSOLOG: actualizar `docs/hw-matrix.json` (`parse-logs`);
  no etapas `ok` ni `--boot-ok` sin sosh / `UCODE_ALIVE_NTFY` / GSP RPC — **soso-dev**.
- Flash/install: `cargo xtask flash-usb-live` / `install-disk` (sudo solo para el disco) — nunca `sudo cargo`.
  El agente **no** ejecuta ese sudo: pide contraseña y Cursor no tiene TTY;
  compilas y dejas el comando al usuario (detalle en **soso-live**).
