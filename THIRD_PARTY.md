# Componentes de terceros

El código first-party de soso está bajo GPL-2.0-only (ver [`COPYING`](COPYING)).
Los siguientes componentes mantienen sus propias licencias.

## crates/xhci-nostd

- **Licencia:** MIT OR Apache-2.0
- **Autor:** Matt Gates
- **Texto:** [`crates/xhci-nostd/LICENSE-MIT`](crates/xhci-nostd/LICENSE-MIT),
  [`crates/xhci-nostd/LICENSE-APACHE`](crates/xhci-nostd/LICENSE-APACHE)
- **Nota:** Compatible con GPL-2.0 al enlazar desde el kernel.

## lxdde/linux/

- **Licencia:** GPL-2.0 (upstream Linux)
- **Origen:** kernel Linux 6.6.x, descargado por `cargo xtask lx-build` (gitignored)
- **Nota:** No forma parte de la licencia del código propio de soso; el binario
  del kernel que enlaza código de Linux queda bajo GPLv2.

## Firmware NVIDIA (`rootfs/lib/firmware/nvidia/`)

- **Licencia:** NVIDIA proprietary (ver `LICENCE.nvidia` en linux-firmware)
- **Origen:** empaquetado opcional con `scripts/l6-pack-firmware.sh`
- **Nota:** No está bajo GPL. Si se redistribuye, debe incluirse la licencia
  NVIDIA correspondiente.
