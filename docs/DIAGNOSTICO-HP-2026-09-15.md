# HP 8265: WiFi no reclamado (familia 8000 ausente)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat, extraíble) con `udisksctl`. Copias en
`target/usb-diagnostic-2026-09-15/` (ESP desmontada). `SOSOWIFI.TXT` vacío (sin
PSK). Este flush **no** es el ROG/AX200 de la mañana
([`DIAGNOSTICO-ROG-2026-09-15.md`](DIAGNOSTICO-ROG-2026-09-15.md)).

| Campo | Valor |
|---|---|
| Kernel USB | **0.2.2 (`dfec53f84-dirty`)** — `SOSOHASH` 15 sep 18:13 |
| Flush | **#24 @ 211763 ms** |
| Firmware UEFI | HP rev 0x11700 |
| Hardware | iGPU `8086:5917` + WiFi **`8086:24fd` (8265)** + `10ec:8168` rtl8169 DOWN |
| Userspace | **`sosh —`** OK (pid=2) |
| WiFi | register OK; **start rc=-1 alive=false**; **RED SIN DRIVER** |
| Ethernet | rtl8169 enlace DOWN → sin DHCP/SSH por cable |

Árboles Linux (solo lectura):

| Árbol | Tag | Uso |
|---|---|---|
| [`lxdde/linux/`](../lxdde/linux/) | **6.6.32** | `pcie/drv.c` (0x24FD), `cfg/8000.c`, `pcie/trans.c` (`iwl_pcie_load_given_ucode_8000`) |

Hostcheck `./scripts/l6-iwl-fw-hostcheck.sh` **OK** (AX200/AX211). No cubre
familia 8000, ni `iwlwifi-8265-36.ucode` (no está en `rootfs/lib/firmware/`),
ni transporte ICT/TFD, ni silicio. `./scripts/l6-g3-gsp-hostcheck.sh` OK; no
aplica a esta placa (sin NVIDIA).

---

## Tabla de etapas (flush #24)

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Boot / sosh | `soso 0.2.2 (dfec53f84-dirty)` → `sosh —` pid=2 | **OK** |
| fatlog | flush **#24** @ 211 s | OK |
| iwlwifi register | `lxdde: iwlwifi register rc=0` | OK (driver registrado) |
| Probe 8265 | sin `iwlwifi: probe id=0x24fd`; `hwscan: RED SIN DRIVER 02:00.0 8086:24fd` | **FAIL** |
| ALIVE | `lxdde: iwlwifi start rc=-1 phase= alive=false`; sin `UCODE_ALIVE_NTFY` | **FAIL** (no es «ALIVE degradado») |
| Scan userspace | `$ wifi scan` (syscall `ENOTSUP` si `!present`) | **FAIL** |
| Assoc / DHCP WiFi | no alcanzado; `SOSOWIFI` vacío | pendiente |
| Ethernet | `rtl8169: … enlace DOWN 10M half`; `net: dhcp…` sin lease | DOWN |

Secuencia:

```
iwlwifi register rc=0
→ pci::init (8086:24fd no coincide con 7f70/51f0/54f0/2723)
→ start_module: !probed → rc=-1 alive=false
→ hwscan RED SIN DRIVER 02:00.0 8086:24fd
→ sosh; wifi scan; halt
```

---

## Hallazgos

### WIFI-1. PCI 8086:24fd no está en la tabla del port (confirmado)

**Síntoma:** `RED SIN DRIVER 02:00.0 8086:24fd`; `start rc=-1 alive=false`;
sin línea `iwlwifi: probe`.

**soso:** [`iwl_ax211.c:124-129`](../lxdde/ports/iwlwifi/iwl_ax211.c) solo
lista `0x7f70`, `0x51f0`, `0x54f0`, `0x2723`.
[`registry.rs:55-61`](../kernel/src/drivers/registry.rs) igual.
[`iwlwifi_lx.c:19-22`](../lxdde/ports/iwlwifi/iwlwifi_lx.c): si `!probed`,
`start_module` vuelve −1 sin log del ID visto.

**Linux 6.6** [`pcie/drv.c:439-465`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/pcie/drv.c):
`IWL_PCI_DEVICE(0x24FD, …, iwl8265_2ac_cfg)` (y 8275). Alias del host
`pci:v00008086d000024FD… iwlwifi`. USB `8087:0a2b` en el mismo arranque es
el Bluetooth del combo 8265 (no es la NIC).

**Compatible con el síntoma:** el firmware no se carga porque el probe no
corre. Confirmado.

### WIFI-2. Añadir el ID como gen3 colgaría o cargaría el ucode equivocado (confirmado, no causal aún)

**soso:** [`iwl_ax211.c:87`](../lxdde/ports/iwlwifi/iwl_ax211.c)
`gen3 = device_id != IWL_PCI_AX200`. Cualquier ID nuevo (incluido 0x24fd)
iría por **gen3**: ucode `iwlwifi-so-a0-gf-a0-*` + context-info
([`iwl_trans.c:986+`](../lxdde/ports/iwlwifi/iwl_trans.c),
`UREG_CPU_INIT_RUN`).

**Linux 6.6** [`cfg/8000.c:79-136`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/cfg/8000.c):
`device_family = IWL_DEVICE_FAMILY_8000`, firmware
`iwlwifi-8265` API 22–36 (`iwlwifi-8265-36.ucode`), 31 colas, TFD 256,
`apmg_not_supported`, NVM_EXT. Carga por
[`trans.c:1410-1413`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/pcie/trans.c)
`iwl_pcie_load_given_ucode_8000` (`RELEASE_CPU_RESET` + secciones CPU1/CPU2),
**no** context-info gen2/gen3. IRQ ICT
([`trans.c:3720`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/pcie/trans.c)).
`CSR_HW_REV` de 8000 usa step en los 4 bits bajos
([`trans.c:3692-3698`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/pcie/trans.c)).

**Compatible:** si se mete el ID sin familia, el siguiente síntoma sería
firmware no encontrado o un arranque gen3 sobre silicio 8000. Hoy no se
llegó ahí. Confirmado como trampa; no es la causa de *este* log.

### WIFI-3. Rootfs sin ucode 8265 (confirmado, etapa ausente)

**soso:** `rootfs/lib/firmware/` tiene `iwlwifi-cc-a0-77.ucode` y
`iwlwifi-so-a0-gf-a0-89.ucode` + pnvm. No hay `iwlwifi-8265-*.ucode`.
[`iwl_load_firmware_files`](../lxdde/ports/iwlwifi/iwl_ax211.c) solo prueba
esas dos familias.

**Linux:** `IWL8265_FW_PRE "iwlwifi-8265"` + API 36
([`cfg/8000.c:34-36`](../lxdde/linux/drivers/net/wireless/intel/iwlwifi/cfg/8000.c)).
El host tiene `/lib/firmware/iwlwifi-8265-36.ucode.zst`.

Etapa ausente, no bug de una línea.

---

## Implementación en tree (15 sep noche)

Port 8265 aplicado en el working tree (pendiente reflash en placa):

| Ítem | Estado host | Archivos |
|---|---|---|
| Probe `0x24fd` + firmware `iwlwifi-8265-36.ucode` | OK hostcheck | `iwl_ax211.c`, `registry.rs`, `rootfs/lib/firmware/` |
| Transporte FH 8000 + ALIVE | OK hostcheck | `iwl_trans_8000.c` |
| HCMD gen1 + SCD/FH post-ALIVE | OK hostcheck | `iwl_trans_8000_fw_alive`, `iwl_trans.c` (doorbell `qid<<8`) |
| MVM init/up tras ALIVE 8000 | OK hostcheck MVM | `iwl_ax211.c`, `iwl_mvm_init.c` (sin PNVM) |

**Siguiente en placa:** reflash kernel y validar SOSOLOG.

```bash
cargo xtask flash-usb-live /dev/sda --yes --only kernel
```

Montar ESP p1 (`udisksctl`) y escribir `SOSOWIFI.TXT`:

```
ssid=<tu_red>
psk=<tu_clave>
```

Criterios SOSOLOG: `probe id=0x24fd familia=8000` → `UCODE_ALIVE_NTFY` →
`fw_alive 8000` → `INIT_COMPLETE_NOTIF` → `wifi scan` con BSS → lease `LxWifi`.

---

## Orden de corrección

### 1. Rechazar familia 8000 con log del PCI (no reclamar 0x24fd como gen3)

Archivos: `lxdde/ports/iwlwifi/iwlwifi_lx.c`, `iwl_ax211.c`.
Si `!probed`, enumerar PCI `8086` clase red y cantar
`iwlwifi: 8086:24fd familia 8000 no portada (IDs: 7f70/51f0/54f0/2723)`.
**No** añadir `0x24fd` a `iwl_ax211_ids` hasta el ítem 2: el `gen3 = id !=
0x2723` lo trataría como AX211.

Discrepancia: Linux `drv.c:439` reclama 0x24FD; soso lo ignora en silencio.

Hecho host: el start sin device imprime el ID. Hecho placa: SOSOLOG con
`8086:24fd … no portada` en vez de `start rc=-1 phase=`.

### 2. Familia + firmware 8265 (bloqueante probe)

Archivos: `iwl_ax211.c` / `iwl_internal.h` (enum familia, no booleano gen2/gen3),
`kernel/src/drivers/registry.rs`, `rootfs/lib/firmware/iwlwifi-8265-36.ucode`,
carga en `iwl_load_firmware_files`.

Discrepancia: Linux `cfg/8000.c:130-136` + `drv.c:439`; soso solo AX200/AX211.

Hecho host: hostcheck parsea TLV de `iwlwifi-8265-36.ucode` (SEC_RT).
Hecho placa: `iwlwifi: probe id=0x24fd` y **no** pide `so-a0-gf-a0`.

### 3. Transporte 8000: carga FH + ICT (bloqueante ALIVE)

Archivos: nuevo camino en `iwl_trans.c` (o `iwl_trans_8000.c`) según
Linux `trans.c:1029-1063` (`iwl_pcie_load_given_ucode_8000`) y
`trans.c:1410-1413`. 31 colas, TFD 256, `RELEASE_CPU_RESET`, sin
context-info / `UREG_CPU_INIT_RUN` de gen2/3. HW_REV step 8000
(`trans.c:3697-3698`).

Hecho host: hostcheck de carga de secciones 8000 (sin NIC).
Hecho placa: **`UCODE_ALIVE_NTFY`** y `alive=true` (no timeout vacío).

### 4. MVM / NVM_EXT 8000 y scan

Archivos: `iwl_mvm_nvm.c`, `iwl_mvm.c` — `nvm_hw_section_num=10`,
DCCM/SMEM de `cfg/8000.c:23-28`, `IWL_NVM_EXT`. Reutilizar scan UMAC
cuando el transporte viva.

Hecho placa: `wifi scan` lista BSS (no `no hay adaptador` / `ninguna red`
por TX/RX muerto).

### 5. Assoc WPA2 + DHCP `LxWifi`

Tras ALIVE+scan, el 4-way ya está en `soso-wpa2` / `wifi_wpa.rs`.
`SOSOWIFI.TXT` sigue vacío: hay que escribir `ssid=` / `psk=` en la ESP
(el agente no pega el PSK).

Hecho placa: `wifi connect` → autorizada → lease en backend `LxWifi` → SSH :22.

---

## Qué no se ha hecho

Reflash en placa ni ciclo WiFi real (USB no conectado al host en esta sesión).
Sin `--boot-ok`. Etapas WiFi en matriz siguen **fail** hasta nuevo SOSOLOG.
`target/` no va al git. GPU Intel de esta placa no se diagnostica aquí.
