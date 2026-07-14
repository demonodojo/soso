# soso skills (source of truth)

Skills live here for **Claude Code** (`.claude/skills/`). Cursor discovers them via symlink in `.cursor/skills`.

| Skill | Purpose |
|-------|---------|
| `soso-dev` | Build, run, test and debug soso in QEMU |
| `soso-architecture` | Kernel, sosofs, userspace and coding constraints |
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
