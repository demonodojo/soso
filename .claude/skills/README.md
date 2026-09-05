# soso skills (source of truth)

Skills live here for **Claude Code** (`.claude/skills/`). Cursor discovers them via symlink in `.cursor/skills`.

| Skill | Purpose |
|-------|---------|
| `soso-dev` | Build, run, test and debug soso in QEMU |
| `soso-architecture` | Kernel, sosofs, userspace, syscalls and coding constraints |
| `soso-gpu` | NVIDIA nouveau/GSP (L6, gates G1–G6, VFIO) |
| `soso-wifi` | Intel iwlwifi (AX211/AX200), WPA2, firmware, VFIO |
| `soso-live` | Live USB, ESP slots, boot-shim, install and OTA |
| `soso-user-manual` | Maintain `MANUAL-USUARIO.md` for end users |

## Layout

```
.claude/skills/          ← edit skills here (source of truth)
.cursor/skills/          ← symlink → ../.claude/skills
```

## Install symlink (project)

From repo root:

```bash
mkdir -p .cursor
ln -sfn ../.claude/skills .cursor/skills
```

Verify:

```bash
ls -la .cursor/skills
# .cursor/skills -> ../.claude/skills
```

## Rules

- Create and edit skills **only** under `.claude/skills/<name>/SKILL.md`.
- Never put real skill files inside `.cursor/skills/` (it must stay a symlink).
- If the symlink is missing after clone, recreate it with the command above.
- After a `/loop` stage that changes behaviour: update the matching domain skill
  **in the same task** (`soso-architecture` for kernel/FS/LLM, `soso-gpu` /
  `soso-wifi` / `soso-live` for those stacks, `soso-dev` if commands/tests
  change, `soso-user-manual` if the user sees different strings).
