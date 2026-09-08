# Estado actual de soso

Fuentes de verdad (no duplicar números en otros sitios sin citarlas):

| Dato | Fuente |
|------|--------|
| Versión | [`VERSION`](../VERSION) → `/etc/soso-release` y banner del kernel |
| Soporte probado por equipo | [`docs/hw-matrix.json`](hw-matrix.json) + [`docs/HW-MATRIX.md`](HW-MATRIX.md) |
| Plan de entregas A1–A9 | [`PLAN_ASTRA.md`](../PLAN_ASTRA.md) |
| Ciclo B (B1–B6) | [`PLAN_ASTRA.md`](../PLAN_ASTRA.md) — ver estado por entrega |

Versión en árbol: **0.2.2** (septiembre 2026).

## Ciclo B (sept 2026)

| Entrega | Estado | Notas |
|---------|--------|-------|
| **B1** Redimensionado recuperable | Casi cerrado | Host + recovery QEMU OK (`cargo xtask test-resize`). **Pendiente:** grow QEMU en test-resize (TCG lento; usar KVM). USB/NVMe sin flush durable. |
| **B2** Memoria, ELF, argv | Casi cerrado | Kernel + `init test` (mprotect/mremap/argv/env/ELF). **Pendiente:** validar en QEMU (`cargo xtask test`). |
| **B3–B6** | Pendiente | Forja, CI, hw-matrix, caché GPT. |

## Comprobaciones automáticas

```sh
cargo xtask check          # host + builds + hostchecks iwl/GSP + hw-matrix parser
cargo xtask test           # integración QEMU (4 shards; señal fiable tras A1)
cargo xtask test-resize    # B1: host cuts + recovery QEMU (grow QEMU: pendiente sin KVM)
cargo xtask test-update    # OTA E2E + recuperación simulada + manifiesto inválido
```

Una suite en rojo **no** se considera aceptable: investigar y corregir antes de
declarar release.

## Soporte por chip (resumen)

| Componente | Evidencia en placa | Pendiente |
|--------------|-------------------|-----------|
| GPU GB205 (`10de:2f18`) | G1–G5 GO documentado en README/skills | Revalidar tras cambios FWSEC/falcon |
| GPU GA107 (Ampere) | Código FWSEC-FRTS | Matriz A8 sin `ok` en etapas GPU |
| WiFi AX211 gen3 | Parser host + VFIO script | ALIVE/assoc/DHCP en placa real |
| WiFi AX200 gen2 | Parser host | Context-info gen2 en placa |
| QEMU test shards | `cargo xtask test` | Sustituto de placa, no certifica WiFi/GPU real |

Detalle por etapa: `cargo xtask hw-matrix show`.

## OTA — límites publicados

- **Kernel:** recuperación verificable con `SOSOKRN.MET` + backup en
  `SOSOKRN.BIN` (cortes en fase applying/backup). Imágenes antiguas sin
  `SOSOKRN.MET`: revert legacy limitado; reflashear para meta durable.
- **Rootfs:** actualización parcial por fichero; **no** hay rollback automático
  de binarios anteriores (progreso en `/etc/actualiza.estado` para reintentar).
- **Confirmación:** init solo marca `OK` tras rootfs accesible y arranque de
  `/bin/sosh`.

Manual de usuario: [`MANUAL-USUARIO.md`](../MANUAL-USUARIO.md) (sección soso-update).
