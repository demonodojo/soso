---
name: soso-release
description: >-
  Genera y publica una versión nueva de soso: sube VERSION, empaqueta kernel
  y rootfs, empuja el commit y crea la GitHub Release. Usar cuando pidan
  publicar, generar o cortar una release, una versión nueva, un bump de
  VERSION, o /publicar-release.
---

# soso — Publicar versión

Un solo camino: `cargo xtask release`. No escribas `VERSION` a mano, no llames
a `gh release create` aparte y no armes el pack tú.

La compilación es larga. Lánzala una vez y espera.

## Cuál

| Petición | Comando |
|---|---|
| Nueva versión, sin matiz | `cargo xtask release --bump --publish` (patch) |
| Minor o major | `--bump minor` o `--bump major` |
| Publicar el número que ya está en `VERSION` | `cargo xtask release --publish` |

Desde la raíz del repo.

## Qué hace

1. `--bump` escribe `VERSION`, commitea **solo** ese fichero (mensaje = el
   número) y se para si la etiqueta ya existe.
2. Con `--publish`, el árbol tiene que estar limpio salvo `VERSION`. Si hay
   otra cosa sucia, aborta.
3. Compila el perfil live y escribe `target/release-soso/v<VERSION>/`
   (`kernel-x86_64`, `rootfs.pack`, `manifest.txt`). No publica si el
   manifiesto no pasa la validación del cliente.
4. `--publish` hace `git push origin HEAD` (avance rápido, sin `--force`) y
   `gh release create --target` de ese commit. El push manda la rama entera,
   no solo `VERSION`: los commits locales que ya estuvieran hechos salen con
   ella. El trabajo sin commitear no entra en la release.

## Si aborta

- Cambios ajenos a `VERSION`: para y lista las rutas. No hagas stash, no los
  commitees y no los metas en el commit de versión.
- `git push` rechazado: para. No fuerces. El commit de versión puede quedar
  solo en local; no vuelvas a bumpear.
- `gh auth status` falla: hace falta `gh auth login`. No reintentes.
- El empaquetado falla después del commit de versión: no publiques y no hagas
  otro bump. Si el push llegó y `gh` no, dilo: el commit ya está en origin.

No flashees el USB. No hagas `git add` de nada más.

## Al terminar

```sh
gh release view --json url,tagName,targetCommitish --jq '"\(.tagName) \(.targetCommitish) \(.url)"'
```

El `targetCommitish` es el commit que acaba de subir `VERSION`.
