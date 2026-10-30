# T87 — Repositorio `opencode-sal` e instalación con `soso-pkg`

**Hito:** SI-6 · **Tipo:** Integración · **Estado:** ver catálogo

**Dependencias:** T84, T86

## Objetivo

Repositorio `demonodojo/opencode-sal`, empaquetado genérico (`cargo xtask pkg-pack --manifest DIR/paquete.txt`), `config/opencode-sal.lock`, fase `opencode` en `cargo xtask test`, sección en `MANUAL-USUARIO.md`.

## Criterio de cierre

Guest: `soso-pkg instalar` del fixture y `opencode --version` exit 0.
