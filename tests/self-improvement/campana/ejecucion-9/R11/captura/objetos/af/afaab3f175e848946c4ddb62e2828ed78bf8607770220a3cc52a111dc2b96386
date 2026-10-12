# lxdde — capa DDE para drivers Linux en soso

Emulación estilo Genode **lx_emul**: compila código C de drivers (originales o
portados) y lo enlaza al kernel Rust de soso mediante `liblxdde.a`.

## Requisitos

- `clang` (compilación freestanding `-nostdinc`)
- `ar`, `nm`, `curl` (descarga opcional del tarball Linux LTS)

## Build

```bash
# Compilar todos los ports (spike, testdrv, e1000e)
cargo xtask lx-build all

# Kernel con la capa (modo e1000e para NIC Linux)
SOSO_LXDDE_MODE=e1000e cargo xtask build --features lxdde   # vía kernel/
# O desde xtask (activa lxdde automáticamente con lx-e1000e):
SOSO_QEMU_NIC=lx-e1000e cargo xtask build
```

## Puertos incluidos

| Port | Propósito |
|------|-----------|
| `spike` | D0: C↔Rust (`lx_spike_run` → `lx_printk`) |
| `testdrv` | D2: kmalloc + workqueue + completion + PCI |
| `e1000e` | D3: driver e1000e estilo Linux (`pci_driver` + `net_device`) |
| `nouveau` | G3: probe NVIDIA + GSP bring-up gb205 |

Cada port vive en `lxdde/ports/<nombre>/source.list` (lista de `.c`).

## Bucle para portar un driver nuevo

1. Crear `lxdde/ports/<driver>/source.list` con los `.c` del árbol Linux pinneado
   (`lxdde/linux/`, descargado bajo demanda) o fuentes propias en `lxdde/ports/`.
2. `cargo xtask lx-build <driver>` — el generador de stubs produce
   `lxdde/shim/src/generated_dummies.c` para símbolos faltantes.
3. Arrancar en QEMU; si salta un stub (`lx_emul_trace_and_stop`), implementar el
   shim real en `kernel/src/lxdde/` o `lxdde/shim/src/shims.c`.
4. Repetir hasta que el driver haga probe y funcione.

## Integración en soso

- Feature del kernel: `lxdde` (desactivada por defecto — `cargo xtask test` intacto).
- Modo compile-time: `SOSO_LXDDE_MODE=spike|testdrv|e1000e|nouveau`.
- NIC QEMU: `SOSO_QEMU_NIC=lx-e1000e` (dispositivo `e1000e` + backend lxdde).
- Bomba cooperativa: `lxdde::poll()` desde el timer BSP (junto a `net::poll()`).

## Licencia

El código first-party de soso en `lxdde/` (shims, ports y glue Rust) está bajo
**GPL-2.0-only**, igual que el resto del proyecto. Ver [`COPYING`](../COPYING).

El árbol `lxdde/linux/` (descargado por xtask, no versionado) es upstream Linux
6.6.x bajo GPLv2. Enlazar código derivado de ese kernel obliga a licenciar el
binario del kernel con feature `lxdde` bajo GPLv2.

## Anexo GPU (fase L6 — reabierta)

Port `nouveau` activo para GB205 Blackwell móvil (`10de:2f18`):

```bash
./scripts/l6-pack-firmware.sh          # gb205/gsp → rootfs/lib/firmware/
cargo xtask lx-build nouveau
SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask build
SOSO_QEMU_GPU=vfio:01:00.0 cargo xtask run   # tras IOMMU + bind VFIO
```

Componentes:

| Componente | Ubicación |
|------------|-----------|
| Probe BAR0 | `kernel/src/drivers/nvidia_probe.rs` |
| GSP bring-up | `lxdde/ports/nouveau/gsp_bringup.c` |
| Firmware shim | `kernel/src/lxdde/firmware.rs` |
| mini-DRM/GEM | `kernel/src/lxdde/drm.rs` |
| Compute G4 | `kernel/src/drivers/nvidia_compute.rs` |

Roadmap G1→G5 (autonomía): [`docs/L6-native-autonomy.md`](../docs/L6-native-autonomy.md).
La infraestructura D1–D2 (fibras, timers, PCI, DMA, IRQ, stubs) es base del port nvkm.
