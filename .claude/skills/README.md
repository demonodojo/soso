# soso skills

Skills en **tres copias** (mantener alineadas al cambiar comportamiento o docs):

| Ruta | Uso |
|------|-----|
| `.claude/skills/` | Claude Code / Codex |
| `.cursor/skills/` | Cursor |
| `.agents/skills/` | Agentes del repo |

| Skill | Purpose |
|-------|---------|
| `soso-dev` | Build, run, `check`, test, debug, `sosolog`, hw-matrix |
| `soso-architecture` | Kernel, sosofs, userspace, syscalls and coding constraints |
| `soso-gpu` | NVIDIA nouveau/GSP (L6, gates G1–G5, VFIO) |
| `soso-wifi` | Intel iwlwifi (AX211/AX200), WPA2, firmware, VFIO |
| `soso-live` | Live USB, ESP slots, boot-shim, install and OTA (`SOSOKRN.MET`) |
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
- Flash/install: `sudo env "PATH=$PATH" "HOME=$HOME" cargo xtask …` — nunca `sudo cargo`.
  El agente **no** ejecuta ese sudo: pide contraseña y Cursor no tiene TTY;
  compilas y dejas el comando al usuario (detalle en **soso-live**).
