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
| **B1** Redimensionado recuperable | Cerrado en QEMU | Grow/recovery virtio OK. USB flush + `test-usb` 4/4. Flush fallido no confirma slide (host). Corte de alimentación en placa pendiente. |
| **B2** Memoria, ELF, argv | Cerrado en init | Validado `init` (KVM). `llm-dense`/`reclaim`/`llm-moe` (tiny-moe, latent-moe, q4k) OK el 2026-09-09. |
| **B3** Forja remota | Cerrado | Token fuera de loopback; POST y GET con Bearer. Demo guest `hola-std`. |
| **B4** CI | Cerrado | OTA: sosh prefaulta PT_LOAD. `qemu-sys` (init) y `qemu-shards` (4 shards TCG) en cada PR/push. `e2e-live` en cron/`workflow_dispatch`. |
| **B5** Matriz hardware | Parser + PCI | Merge no pisa `ok`. Hashes FW del árbol. GA107 `10de:249c`. Huecos `????` rechazados. **Pendiente:** revalidar placa. |
| **B6** Caché GPT | Medido host+guest | Live p3 8 MiB: 35.9 → 1283 MiB/s. Guest `tiny` 40/30 ms. Guest `bench` 161 MiB: 8 tok 580 ms frío / 860 ms caliente; ~202 MiB de pesos por run (CPU, no GPU). |

## Fase D — Steam Deck OLED (sept 2026)

Plan: [`docs/PLAN-STEAMDECK.md`](PLAN-STEAMDECK.md). Objetivo: arrancar en una
Steam Deck OLED (Galileo) con consola, entrada, disco y SSH por WiFi nativo.
La GPU RDNA2 queda fuera.

| Entrega | Estado | Notas |
|---------|--------|-------|
| **D0** Arranque defensivo | Cerrado salvo placa | `arch/iommu.rs`: IVRS + apagado de AMD-Vi si el firmware lo deja activo. Verificado con `-device amd-iommu` en QEMU y 8 tests host en `soso-hw::ivrs`. |
| **D1** Consola rotada | Cerrado salvo placa | Panel 800×1280 vertical: `rot=270` automática (la que Linux aplica al quirk de la Deck). 10 tests host + captura de pantalla real (`cargo xtask fb-shot`). |
| **D2** Entrada USB | Cerrado lo que no exige placa | `SET_PROTOCOL` sólo en interfaces boot; report descriptors e inventario USB volcados a `SOSODRV.TXT`. El parseo del descriptor espera a ver el del mando. |
| **D3** Disco | Sin trabajo pendiente conocido | NVMe ya soporta LBA de 512 y 4096. |
| **D4/W1** MHI de ath11k | Escrito, verde contra modelo | `lxdde/ports/ath11k`: reset, READY, contextos, descarga BHIe y M0. 38 comprobaciones en `scripts/l6-ath11k-hostcheck.sh`. |
| **D4/W2** QMI | Formato del cable hecho | Cabecera, TLV y mensajes IND_REGISTER / HOST_CAP / CAP / WLAN_MODE con bytes verificados. Falta el transporte por IPCR y la secuencia. |
| **D4/W3–W5** | Pendientes | HTC/WMI, anillos de datos, scan y WPA2. |

Firmware: `./scripts/l6-pack-ath11k-fw.sh` copia WCN6855 hw2.1 a
`rootfs/lib/firmware/` (unos 17 MiB con la variante `nfa765`).

## Comprobaciones automáticas

```sh
cargo xtask check          # host + builds + hostchecks iwl/GSP/ath11k + hw-matrix
cargo xtask test           # integración QEMU (4 shards; señal fiable tras A1)
cargo xtask test-resize    # B1: host cuts + recovery QEMU (grow QEMU: pendiente sin KVM)
cargo xtask test-update    # OTA E2E + recuperación simulada + manifiesto inválido
cargo xtask fb-shot        # captura la pantalla del guest (rotación en placa ajena)
```

Una suite en rojo **no** se considera aceptable: investigar y corregir antes de
declarar release.

## Soporte por chip (resumen)

| Componente | Evidencia en placa | Pendiente |
|--------------|-------------------|-----------|
| GPU GB205 (`10de:2f18`) | G1–G5 GO documentado en README/skills | Revalidar tras cambios FWSEC/falcon |
| GPU GA107 (Ampere) | Código FWSEC-FRTS; PCI `10de:249c` | Matriz A8 sin `ok` en etapas GPU |
| WiFi AX211 gen3 | Parser host + VFIO script | ALIVE/assoc/DHCP en placa real |
| WiFi AX200 gen2 | Parser host | Context-info gen2 en placa |
| WiFi WCN6855 (Deck OLED) | Hostcheck MHI+QMI contra modelo | Todo lo que exige silicio: W1 en placa y W2–W5 enteros |
| Steam Deck OLED | Ninguna todavía | Primer arranque: volcado PCI, IOMMU, descriptores USB y foto de la pantalla |
| QEMU test shards | `cargo xtask test` | Sustituto de placa, no certifica WiFi/GPU real |

Detalle por etapa: `cargo xtask hw-matrix show`.

## OTA — límites publicados

- **Kernel:** recuperación verificable con `SOSOKRN.MET` + backup en
  `SOSOKRN.BIN` (cortes en fase applying/backup). Imágenes antiguas sin
  `SOSOKRN.MET`: revert legacy limitado; reflashear para meta durable.
- **Rootfs:** actualización parcial por fichero; **no** hay rollback automático
  de binarios anteriores (progreso en `/etc/actualiza.estado` para reintentar).
- **Confirmación:** init solo marca `OK` tras rootfs accesible, `/tmp/sosh-ready`
  (sosh prefaultó su ELF y llegó a `main`) y `kill(pid, 0)` (~400 ms más
  de sondeo; el hijo sigue vivo).

Manual de usuario: [`MANUAL-USUARIO.md`](../MANUAL-USUARIO.md) (sección soso-update).
