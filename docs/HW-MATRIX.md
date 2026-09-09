# Matriz de validación por equipo (A8)

Fuente versionada: [`hw-matrix.json`](hw-matrix.json).

Cada entrada registra equipo, PCI ID, hashes de firmware, commit git, perfil de
drivers, etapas WiFi/GPU (ok / fail / pendiente), arranques consecutivos y
métricas de bench. Los PCI deben ser concretos (`vvvv:dddd` hex); `collect --pci`
rechaza huecos tipo `10de:????`. GA107: `10de:249c` (ROG 3050 Mobile).

## Comandos

```sh
cargo xtask hw-matrix init          # crea la matriz inicial
cargo xtask hw-matrix show          # resumen en consola
cargo xtask hw-matrix host-check    # hostchecks iwl + GSP (no sustituye placa)

# Tras probar en live USB o placa:
cargo xtask hw-matrix collect --id gb205-box --equipo "..." --pci 10de:2f18 --perfil live-usb
cargo xtask hw-matrix parse-logs --id gb205-box --sosolog /tmp/SOSOLOG.txt --sosodrv /tmp/SOSODRV.txt
cargo xtask hw-matrix record-boot --id gb205-box    # incrementa contador (0 si --fail)
cargo xtask hw-matrix record-bench --id gb205-box --tok-s 1.23 --frio 0.9 --caliente 1.1 --backend gpu

# Atajo con USB montado:
./scripts/l6-a8-collect.sh --id gb205-box --equipo "RTX 5070 Ti" --pci 10de:2f18
```

## Secuencias

**WiFi:** ALIVE real (`UCODE_ALIVE_NTFY`) → scan → WPA2 → DHCP → SSH → reconexión.

**GPU:** GSP/RPC → pool VRAM → CE readback GO → matvec CPU/GPU → carga `soso-llm` → apagado (`unload` + `dma=off`).

**Aceptación:** 3 arranques consecutivos (`record-boot`), sesión sostenida, etapas críticas en `ok`.
VFIO en placa: `scripts/l6-g1-vfio-test.sh`, WiFi: `scripts/l6-wifi-vfio-test.sh`.

**Bench:** mediana de 3 ejecuciones con prompt/seed/max fijos (`cargo xtask bench-llm` + `record-bench`).
