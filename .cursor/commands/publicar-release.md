---
name: publicar-release
description: >-
  Sube VERSION y publica soso en GitHub Releases.
  Usar con /publicar-release. Acepta patch (defecto), minor o major.
---

# Publicar release de soso

Lee `.cursor/skills/soso-update/SKILL.md` y `.cursor/skills/soso-dev/SKILL.md`.

No implementes otra cosa. Desde la raíz del repo ejecuta **un** comando:

```sh
cargo xtask release --bump --publish
```

Si el usuario pide `minor` o `major`, sustituye `--bump` por `--bump minor` o `--bump major`.

`--bump` escribe `VERSION`, commitea **solo** ese fichero y `--publish` crea la GitHub Release (`gh release create`). No hagas `git add` de nada más, no limpies el árbol y no hagas force push.

Si el xtask aborta porque hay cambios ajenos a `VERSION`, para y dilo: el usuario decide si commitearlos o apartarlos. No los metas en el commit de versión.

Al terminar, enseña la versión nueva y la URL:

```sh
gh release view --json url,tagName --jq '"\(.tagName) \(.url)"'
```
