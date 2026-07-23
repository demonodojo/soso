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
- Modo compile-time: `SOSO_LXDDE_MODE=spike|testdrv|e1000e`.
- NIC QEMU: `SOSO_QEMU_NIC=lx-e1000e` (dispositivo `e1000e` + backend lxdde).
- Bomba cooperativa: `lxdde::poll()` desde el timer BSP (junto a `net::poll()`).

## Licencia

Enlazar código derivado del kernel Linux (GPLv2) obliga a licenciar el binario
del kernel bajo GPLv2. Los shims propios de soso siguen la licencia del repo.

## Anexo GPU (fase L6)

Sobre esta base, un driver DRM/NVIDIA necesitaría además:

- Subsistema DRM (`drm_device`, `drm_gem`, IOCTL shimeados).
- `request_firmware` leyendo blobs desde sosofs.
- Mucha más superficie lx_emul (IOMMU, dma-buf, prime, …).

La infraestructura D1–D2 (fibras, timers, PCI, DMA, IRQ diferida, stubs) es
prerrequisito directo del spike L6.
