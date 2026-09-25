# ROG RTX 3080 Laptop: diagnóstico (24 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`, vfat) con `udisksctl`. Copia:
`target/usb-diagnostic-2026-09-24/` (sin PSK en informe). ESP desmontada al terminar.

| Campo | Valor |
|---|---|
| Kernel (banner SOSOLOG) | **soso 0.3.5 (da5d019aa)** |
| SOSOHASH.TXT (hueco ESP) | 0.3.1 / `07e6ef747-dirty` (19 sep) — kernel actualizado in situ |
| Flush / uptime | **#39**, **920 s** |
| Hardware | `10de:249c` + `8086:2723` + `10ec:8168` |
| Instalación nativa | `SOSOBOOT.TXT`: Boot0001 soso, rescate Boot0003 |

Árboles Linux (solo lectura):

| Árbol | Referencia |
|---|---|
| `lxdde/linux/` | 6.6.32 — `vmmgf100.c` (`gf100_vmm_join_`) |
| `lxdde/reference/linux-master-nouveau/` | `fc02acf` — `r535/bar.c` (`r535_bar_bar1_init`) |

No se validó en esta sesión: matvec en GPU, SSH, halt, reconexión WiFi.

## Tabla de etapas

| Etapa | Evidencia SOSOLOG | Resultado |
|---|---|---|
| Shim / firmware | `bootmark.txt`: UEFI alcanzado, `bootsoso.efi` | **OK** |
| Userspace | `sosh —` pid=2, `boot: task` | **OK** |
| fatlog | flush **#39** @ 920 s | OK |
| GPU GSP | `GSP_INIT_DONE`, `GSP booted`, `pool VRAM=sí` (16070 MiB) | **OK** |
| GPU CE | `CE readback verificado (G4e GO)`, golden GR + compute cls=0xc7c0 | **OK** |
| WiFi ALIVE | `UCODE_ALIVE_NTFY`, `alive=true`, `INIT_COMPLETE_NOTIF` | **OK** |
| WiFi MVM | `MVM up mínimo listo`, `SCAN_CFG v5 ok` | **OK** |
| WiFi scan | `SCAN_COMPLETE count=25` / `count=21` | **OK** |
| WiFi assoc | 4-way, `asociado a 'Rutilo' aid=4` | **OK** |
| DHCP | `net: backend lx-wifi`, `192.168.68.132/24` | **OK** |
| Ethernet | rtl8169 UP, `enlace DOWN 10M half` (sin cable) | Parcial |
| OTA comprobar | HTTPS github + release-assets, «ya estás en la última versión» | **OK** |
| OTA aplicar | «usa --forzar» / errno 11 / DNS errno 2 (reintentos manuales) | Parcial (usuario) |

## Hallazgos

### 1. BAR1 PDB de instancia ≠ `bar1PdeBase` — **descartado**

- **Síntoma:** líneas repetidas `PDB=0x31b233f89e94b000` vs `bar1PdeBase=0x3f3c2a000`, PD3 inválido al caminar BAR1.
- **soso:** `lxdde/ports/nouveau/gsp_bar1.c:253-268` (`gsp_bar1_inst_probe`); `gsp_bringup.c:832-857` (`ce_on_verified`) no escribe el bloque.
- **Linux:** `lxdde/reference/.../r535/bar.c:128-151` — `r535_bar_bar1_init` envuelve `rm_bar1_pdb`, no programa `0xb80f40` ni parchea inst+0x200 (`gf100_vmm_join_` en `lxdde/linux/.../vmmgf100.c:342`).
- **Causalidad:** **no confirmada** para este arranque: CE selftest OK, pool VRAM OK, compute listo. Aviso diagnóstico alineado con upstream GSP.

### 2. rtl8169 enlace DOWN — **descartado**

- **Síntoma:** `phystatus 0x84` → DOWN antes de `boot: red`.
- **soso:** driver arrancó; lease DHCP vino por **lx-wifi**.
- **Linux:** comportamiento esperado sin cable/enlace.
- **Causalidad:** **descartada** (entorno, no bug).

### 3. `soso-update` exit 1 — **descartado como driver**

- **Síntoma:** tras `comprobar` OK, `aplicar` sin `--forzar`; luego `tcp_connect` errno 11 y DNS errno 2.
- **soso:** `user/soso-update/src/net.rs:79` (mensaje «conexión rechazada»).
- **Linux:** N/A (userspace/red).
- **Causalidad:** **hipótesis** red transitoria / comando duplicado; no hay `#PF` ni panic en SOSOLOG.

## Orden de corrección

**Ningún cambio de driver obligatorio** en este arranque.

1. *(Opcional, no bloqueante)* Reducir ruido BAR1 en serie cuando CE+pool ya están OK — solo logging en `gsp_bar1.c` / `gsp_bringup.c`, sin escribir inst BAR1 (mantener paridad `r535_bar_bar1_init`).
2. *(Validación)* Placa: `ask` / matvec residente, 3 arranques + SSH, `halt` — para cerrar etapas `compute_cpu_gpu` y `apagado_limpio` en matriz.
3. *(Registro)* Matriz `ga107-igpu` y `ax200-wifi` actualizada; este informe.

## Qué no se ha hecho

- No se modificó código de nouveau, iwlwifi ni rtl8169.
- No se reflasheó el USB.
- No se incrementó `arranques_consecutivos_ok` (un arranque a sosh no es aceptación).
