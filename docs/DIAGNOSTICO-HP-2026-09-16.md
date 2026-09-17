# HP 8265: probe OK, timeout ALIVE INIT (carga FH)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat). Copias en
`target/usb-diagnostic-2026-09-16-eve/` (sin PSK). Kernel **0.2.2 (`e2c42f400-dirty`)**,
flush **#21 @ ~129 s**, `sosh` pid=2, teclado vivo, halt limpio.

Avance vs [`DIAGNOSTICO-HP-2026-09-15.md`](DIAGNOSTICO-HP-2026-09-15.md): ya no es
`RED SIN DRIVER`. El probe reclama `8086:24fd`, parsea SEC_INIT y carga FH INIT,
pero el firmware no entrega `UCODE_ALIVE_NTFY`.

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`e2c42f400-dirty`)** |
| Flush | **#21 @ ~129 s** |
| WiFi | **`8086:24fd` (8265)** — familia 8000 |
| Firmware | `iwlwifi-8265-36.ucode` (SEC_INIT 13, SEC_RT 12) |
| Síntoma | `timeout ALIVE INIT` `rb_hw=0x000` `rx_write=255` `WPTR=0xf8` |
| Ethernet | rtl8169 DOWN (sin cable) |

Boot anterior del mismo día (flush **#22 @ ~365 s**, logs
`target/usb-diagnostic-2026-09-16/`): aún sin SEC_INIT, RX 32, `rx_write=24`.

---

## Tabla de etapas (flush #21)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `sosh —` pid=2 | **OK** |
| Probe 8265 | `probe id=0x24fd familia=8000` | **OK** |
| Parser FW | `SEC_INIT 13 SEC_RT 12` | **OK** |
| APM / FH | `APM 8000 ok`; carga INIT cpu1=5 cpu2=4 chunks=11 | parcial |
| RX FH | `rx_write=255` (256 RBD); WPTR=0xf8 | **OK** (host) |
| ALIVE INIT | `timeout ALIVE INIT … INT=0x08000000 rb_hw=0x000 LOAD_ST=0xffffffff` | **FAIL** |
| MVM / scan | `phase=fw_start alive=false`; `wifi scan` sin FW | **FAIL** |

Secuencia observada:

```
probe 24fd → PHY_SKU 0x00330018 → SEC_INIT 13 SEC_RT 12
→ APM 8000 ok → RELEASE_CPU_RESET (en prepare_hw, antes de KW/ICT)
→ carga FH INIT cpu1=5 cpu2=4 chunks=11
→ timeout ALIVE INIT INT=0x08000000 GP=0x08040005 rb_hw=0 rx_write=255 WPTR=0xf8
→ LOAD_ST=0xffffffff → arranque firmware 8000 falló → alive=false
```

---

## Hallazgos (soso vs Linux 6.6.32)

### 1. `wait_chunk` aceptaba TSSR idle sin FH_TX (bloqueante)

**Síntoma:** `INT=0x08000000` (bit 27 = FH_TX) sigue puesto al timeout; los 11 chunks
pueden devolver OK con el SRAM a medias.

**soso (antes):** `iwl_8000_wait_chunk` daba por bueno `FH_TSSR` canal 9 idle sin ACK de
`CSR_INT_BIT_FH_TX`.

**Linux:** solo `CSR_INT_BIT_FH_TX` completa el chunk (`pcie/trans.c:687-711`).

**Corrección:** eliminar atajo TSSR; ACK `CSR_FH_INT_STATUS` + `CSR_INT` como el ISR de Linux.

### 2. ICT activado antes de INIT (bloqueante)

**soso (antes):** `CSR_DRAM_INT_TBL_ENABLE` escrito antes de la carga INIT.

**Linux:** `iwl_pcie_reset_ict` solo en `fw_alive` post-ALIVE RT (`trans.c:1427`).

**Corrección:** diferir ICT hasta tras ALIVE RT.

### 3. Orden RELEASE vs nic_init (bloqueante)

**soso (antes):** `RELEASE_CPU_RESET` en `prepare_hw`, luego alloc KW/ICT/chunk.

**Linux:** `iwl_pcie_nic_init` (RX + KW con `write_direct32`) → `load_given_ucode_8000`
(WFPM_GP2 → RELEASE → carga inmediata).

**Corrección:** `start_hw` (APM, RX, KW) sin RELEASE; `begin_fw_load` justo antes del primer chunk.

### 4. `CSR_INI_SET_MASK` después del kick final (bloqueante)

**soso (antes):** kick `0xFFFF`/`0xFFFFFFFF` antes de la máscara completa.

**Linux:** `iwl_enable_interrupts` antes de escribir `FH_UCODE_LOAD_STATUS` final
(`load_cpu_sections_8000`, `trans.c:820-834`).

**Corrección:** `CSR_INI_SET_MASK` antes del kick por CPU.

### 5. Diagnóstico SecBoot en timeout

**Corrección:** log `SB_CPU_1_STATUS` / `SB_CPU_2_STATUS` en `iwl_8000_log_alive_timeout`.

---

## Validación host (post-corrección)

```text
./scripts/l6-iwl-fw-hostcheck.sh
OK: wait_chunk solo FH_TX (TSSR idle sin INT no cuenta)
OK: 8265 INIT cpu1=5 cpu2=4 chunks=11; RT cpu1=4 cpu2=4 chunks=10
OK: nic_config PHY_SKU 0x00330018 → CSR_HW_IF_CONFIG_REG
OK: constantes HCMD/SCD/RX 8000 (256 RBD log=8, CSR_INI_SET_MASK)
```

Hostcheck no valida silicio. **Pendiente:** reflash kernel y repetir boot en placa
(`cargo xtask flash-usb-live /dev/sda --yes --only kernel` — usuario).

---

## Run7 (post-FH, pre-tx_init) — flush #24 @ ~408 s

Lectura ESP `/dev/sda1` (`KERNEL`). Copias en
`target/usb-diagnostic-2026-09-16-run7/` (desmontada). Kernel **0.2.2
(`e2c42f400-dirty`)**, `sosh` pid=2, 48 scancodes, halt limpio.

Avance vs flush #21: las correcciones FH **sí están en el USB** (`INT` ya no es
`0x08000000`; salen `SB1`/`SB2`; 11 chunks INIT sin timeout FH). El firmware
**sigue sin arrancar**.

| Campo | Valor |
|---|---|
| Flush | **#24 @ ~408 s** |
| Síntoma | `timeout ALIVE INIT INT=0x00000000 GP=0x08040005` |
| SecBoot | `SB1=0x00000003 SB2=0x000029f6` |
| RX FH | `rb_hw=0x000 rx_write=255 WPTR=0xf8 LOAD_ST=0xffffffff` |
| Scan | `wifi scan` → `phase=fw_start alive=false` → `halt` |

Secuencia observada:

```
… carga FH INIT cpu1=5 cpu2=4 chunks=11 (sin timeout FH)
→ timeout ALIVE INIT INT=0x00000000 … SB1=0x00000003 SB2=0x000029f6
→ arranque firmware 8000 falló → phase=fw_start alive=false
```

Descartado: «sosh sale al fallar un comando». `wifi scan` es builtin y no mata
sosh; `phase=hlt`/`apagando` fue tecleo de `halt` encima del mensaje.

### Hallazgos adicionales (soso vs Linux 6.6.32)

#### 6. `tx_init` incompleto antes de INIT (bloqueante)

**soso (run7):** `start_hw` solo programaba KW; las 31 colas SCD/CBBC y
`SCD_GP_CTRL` iban en `alloc_hcmd` **después** del `wait_alive INIT` que nunca
llega. Compatible con SecBoot clavado en `SB1=0x3` y `rb_hw=0`.

**Linux:** `iwl_pcie_nic_init` → `iwl_pcie_tx_init` (31× CBBC, SCD off, KW,
`SCD_GP_CTRL_AUTO_ACTIVE_MODE` + `ENABLE_31_QUEUES`) **antes** de
`load_given_ucode_8000` (`pcie/trans.c:540-574`, `pcie/tx.c:546-604`).

**Corrección (host):** `iwl_8000_tx_init` en `start_hw` antes de `begin_fw_load`.

#### 7. `PCI_CFG_RETRY_TIMEOUT` (0x41) = 0 en probe

**soso (run7):** probe solo `enable_device` + `set_master`.

**Linux:** `pci_write_config_byte(pdev, 0x41, 0)` en probe (`pcie/drv.c:1532`).

**Corrección (host):** `lx_pci_write_config(pdev, 0x41, 0, 1)` en probe.

#### 8. `wait_alive` sin ACK de interrupciones

**soso (run7):** solo drenaba RX/`rb_stts`; `INT=0` al timeout.

**Linux:** ISR ACK `CSR_INT` + `CSR_FH_INT_STATUS` FH_RX/ALIVE/FH_TX
(`pcie/rx.c:1902-2040`).

**Corrección (host):** `iwl_8000_poll_alive_int` en el bucle de espera; log
`LOAD_ST`/`SB1`/`SB2`/`FH_TSSR` tras kick CPU1/CPU2.

### Validación host (nic_init 8000, post-corrección)

```text
./scripts/l6-iwl-fw-hostcheck.sh
OK: tx_init 31 colas (SCD off→KW→CBBC→SCD_GP_CTRL) vs Linux tx.c:546
OK: start no carga INIT sin tx_preload_ready
OK: wait_chunk solo FH_TX (TSSR idle sin INT no cuenta)
OK: 8265 INIT cpu1=5 cpu2=4 chunks=11; RT cpu1=4 cpu2=4 chunks=10
```

**Pendiente placa:** reflash kernel con tx_init+PCI 0x41+wait_alive ISR; buscar
`UCODE_ALIVE_NTFY` o cambio en `SB1`/SecBoot. Sin flash en esta sesión.

---

## Referencias

- Plan: corrección INIT ALIVE 8265 (sep 2026)
- Logs run7: `target/usb-diagnostic-2026-09-16-run7/`
- Logs eve: `target/usb-diagnostic-2026-09-16-eve/`
- Logs mañana: `target/usb-diagnostic-2026-09-16/SOSOLOG.txt`
- Informe previo: [`DIAGNOSTICO-HP-2026-09-15.md`](DIAGNOSTICO-HP-2026-09-15.md)
