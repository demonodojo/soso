# Plan: IRQs pendientes frente a Linux

Incorporar las interrupciones que Linux usa y soso aún sondea. Modelo: top-half
mínimo (ack + flag), soft path con FPU preservada; nunca `wifi::poll` /
`hid_i2c::poll` / HDA desde hardirq.

Fuera de alcance: IRQ async de nouveau/GSP, completion IRQ de
virtio-blk/USB BOT/NVMe, RTC/HPET, SCI genérico ACPI.

## Estado

| Fase | Contenido | Estado | Evidencia |
|------|-----------|--------|-----------|
| I1 | MSI-X iwlwifi + `lx_iwlwifi_irq_top` + soft path | hecho | `SOSO_LXDDE_MODE=iwlwifi cargo xtask build --drivers live-usb` OK |
| I2 | Handlers NMI y #MC en IDT | hecho | build kernel OK |
| I3 | Interrupt/GpioInt HID-I2C + amd_gpio mínimo | hecho | build kernel OK |
| I4 | Wake IRQ HDA | hecho | build kernel OK |

## Modelo

Dispositivo → HardIRQ (ack + marcar pendiente) → si desde ring3: SoftPath →
driver poll; si interrumpe kernel: scheduler/tick recoge el flag.

## Criterios

- I1: SOSOLOG `wifi: MSI-X vector=…`; contador de IRQs wifi > 0 tras asociación (placa).
- I2: entradas IDT presentes (mismo diagnóstico que #XM).
- I3: log `hid-i2c: irq …` o `amd-gpio: pin=…` (placa AMD).
- I4: captura HDA con INTCTL/MSI armados (QEMU hda-duplex).

Regresión: `cargo xtask check` y `cargo xtask test` en verde; I1 no llama a la
pila wifi desde hardirq.
