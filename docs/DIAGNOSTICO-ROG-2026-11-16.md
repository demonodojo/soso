# Diagnóstico ROG — 16 de noviembre de 2026

## Evidencia y alcance

- ESP: `/dev/sda1` (etiqueta `KERNEL`, montada y desmontada con `udisksctl`).
- Copias locales: `target/usb-diagnostic-2026-11-16/`. `SOSOLOG.TXT` conserva el binario FAT original; `SOSOLOG.clean.txt` elimina el relleno y conserva las líneas no vacías. Se copiaron también `SOSODRV.TXT`, `BOOTMARK.TXT`, `SOSOBOOT.TXT`, `SOSOUPD.TXT`, `SOSOKRN.MET` y `SOSOWIFI.TXT`; las credenciales no se reproducen aquí.
- Kernel: soso 0.3.28, commit `e466e0965`. Flush #25, uptime 4 517 786 ms (75 min), 58 407 bytes útiles, duración 21 ms y máximo 5 006 ms; hubo pausas del sondeo WiFi de hasta 7 772 ms mientras se volcaba el log.
- PCI: NVIDIA `10de:249c` (RTX 3080 Laptop, GA104), Intel AX200 `8086:2723`, Realtek RTL8168H `10ec:8168`.
- Comparación: árbol local `lxdde/linux/`, Linux v6.6.32 (Makefile declara 6.6.32; el checkout no expone un commit Linux independiente). El checkout principal del proyecto está en `e466e0965`. Para GA104 se usó el mismo árbol Linux 6.6.32, que contiene nouveau Ampere `ga102.c`.
- Hostchecks guardados en `target/usb-diagnostic-2026-11-16/hostcheck-iwl.txt` y `hostcheck-gsp.txt`. Son pruebas de parser/ABI y secuencias de host; no validan por sí solas transporte, DMA físico, enlace ni estabilidad del silicio.

## Etapas del arranque

| Etapa | Evidencia | Resultado |
|---|---|---|
| UEFI / shim | `BOOTMARK.TXT`: `soso-shim: UEFI alcanzado`; `DONE Boot0001 soso` | OK |
| Kernel | SOSOLOG L2: `soso 0.3.28 (e466e0965)`; `boot: memtest` hasta `boot: task` (L12–919) | OK |
| USB / live disk | `boot: live-disk` (L277); xHCI enumera USB y dispositivos HID (L78 en adelante) | OK |
| GPU Ampere | `GSP_INIT_DONE ... res=0x0` (L426), `GSP booted (hw, booter_load Ampere + RPC...)` (L737), pool VRAM sí (L780) y `CE readback verificado` (L562) | OK para RPC, pool y CE |
| AX200 | `UCODE_ALIVE_NTFY` (L754), `INIT_COMPLETE_NOTIF` (L756), `alive=true` (L776) | OK |
| Scan WiFi | Dos `SCAN_COMPLETE status=1`, `end=1`, counts 21 y 18 (L812–830) | OK |
| WPA2 | `4-way completado, enlace autorizado` (L856) | OK |
| Red | `net: backend lx-wifi` (L857), lease `192.168.68.132/24` (L922) | DHCP WiFi OK |
| Ethernet | RTL8168H inicializada; `phystatus 0x84 ... enlace DOWN` (L783–790) | Sin enlace físico observado; no atribuido al driver |
| Userspace | `sosh — escribe 'help'` (L927) | OK |
| WM | `/bin/soso-wm` muere por page fault (L932–938) | Falla confirmada |
| SSH / reconexión | Se carga la host key; no hay evidencia de sesión entrante ni de reconexión | Pendiente |

## Hallazgos

### 1. AX200: cadena real de WiFi completada; sin defecto de driver confirmado

SOSOLOG acredita ALIVE, init MVM, dos scans finalizados con BSS, asociación, 4-way y lease por `LxWifi` (L754–859, L922). El `scan_count` positivo y `SCAN_COMPLETE end=1` descartan el caso de RX silencioso o scan vacío. El hostcheck iwlwifi terminó con código 0 y cubrió, entre otros, gen2 RX, HCMD, scan y asociación; no sustituye el resultado en placa.

Comparación concreta: el port usa la cola HCMD 0 (`lxdde/ports/iwlwifi/iwl_internal.h:73`), anuncia RX gen2 antes de iniciar el firmware (`iwl_trans.c:659`) y forma el doorbell como `wr | (qid << 16)` (`iwl_trans.c:1515–1519`). Linux 6.6.32 asigna HCMD mediante `IWL_MVM_DQA_CMD_QUEUE` (`lxdde/linux/drivers/net/wireless/intel/iwlwifi/mvm/fw.c:73–78`), carga el runtime firmware en `iwl_mvm_up` (`.../mvm/fw.c:1499–1524`) y espera notificaciones de final de scan (`.../mvm/scan.c:3249–3261`). El comportamiento observado es compatible con esos pasos: las finalizaciones llegan con UID y `end=1`, y DHCP sale por WiFi. No se encontró una discrepancia causal para corregir en el port.

El intervalo de sondeo WiFi llegó a 7 772 ms y el log antiguo lo atribuía al volcado (`SOSOLOG.clean.txt:921,929,943`). La causa era una pista, no una medición de una sola escritura: el clasificador comparaba el gap con la duración acumulada de un flush que podía abarcar varias vueltas del planificador. El mismo arranque completa scans, autorización y DHCP; no hay evidencia de fallo de RX o firmware. El cambio de FATLOG abajo limita y mide transacciones individuales, pero hace falta otro arranque en placa para confirmar la latencia.

### 2. GA104: bring-up Ampere, RPC, pool y CE pasan; compute real pendiente

La GPU reporta `NV_PMC_BOOT_0=0xb74000a1` (L778), recibe `GSP_INIT_DONE` con `res=0` (L426), arranca por `booter_load Ampere + RPC` (L737), crea pool VRAM (L563, L780) y completa readback CE de 4 KiB (L560–562). El hostcheck GSP terminó con código 0, pero no acredita el silicio.

Comparación: Linux nouveau v6.6.32 selecciona la implementación GSP Ampere con `ga102_gsp_new` (`lxdde/linux/drivers/gpu/drm/nouveau/nvkm/subdev/gsp/ga102.c:25–58`). El port sigue el camino Ampere de booter SEC2 y, tras GSP vivo, RPC/RM/VMM/CE/pool (`lxdde/ports/nouveau/gsp_bringup.c:351–416, 465–518`). El resultado del USB confirma que esta secuencia sí es compatible con el síntoma: no hay fallo GSP que explique el page fault de userspace. No se ve una ejecución de compute CPU/GPU, carga real ni apagado limpio; esos gates quedan pendientes.

### 3. RTL8168H: enlace DOWN sin prueba de defecto de inicialización

El port identifica MAC version 46, ejecuta `rtl_hw_start_8168h_1`, carga el firmware PHY y configura el PHY antes de leer `phystatus` (`kernel/src/drivers/rtl8169.rs:523–591, 887–895, 1473–1474`). Linux v6.6.32 usa `rtl_hw_start_8168h_1` para MAC ver. 46 (`lxdde/linux/drivers/net/ethernet/realtek/r8169_main.c:3288–3318`) y aplica la configuración/firmware PHY 8168H (`.../r8169_phy_config.c:796–815`). El log da `phystatus 0x84`, BMSR `0x7989` y enlace DOWN (L790). Esto demuestra falta de enlace en esa lectura, pero sin el estado del cable/switch no demuestra una discrepancia del port; DHCP se obtuvo por WiFi. No hay cambio de código justificado para Ethernet a partir de este arranque.

### 4. Page fault de soso-wm en `Superficies.set_sid`

Al ejecutar `soso-wm`, el proceso 3 intenta leer VA `0x706d6f632d6f` (SOSOLOG L932–938). El RIP `0x427601` del ELF que estaba en el USB corresponde a `sal_u_Superficies__set_sid`; la instrucción lee un elemento del almacenamiento de `slots`. La llamada inicializada desde el compositor está en `user/sal/escritorio/wm.sal:1841–1860`, en `st.surfs.set_sid(...)` (L1855). La inspección posterior del lowering del compilador aisló la causa: el ELF era anterior a cambios en Sal que añaden writeback mediante celdas para mutaciones de campos/locales por referencia mutable. Por tanto, el fallo era compatible con código generado obsoleto y no demuestra corrupción de `slots` en el fuente del WM. Se regeneraron los binarios con Sal fijado al commit `6e3b7e725b5ac3a8a517172da9fc46225b12131d`, que contiene esas correcciones. No se modificó `wm.sal`; no es un fallo de GPU.

## Orden de corrección

1. **Regenerar `soso-wm` con el lowering corregido de Sal** — completado en host. `config/sal-source.lock` fija Sal en `6e3b7e725b5ac3a8a517172da9fc46225b12131d`; `cargo xtask sal-build --cross-only` terminó correctamente y validó los ELF estáticos x86-64. `target/sal/soso-wm` y `rootfs/bin/soso-wm` son idénticos (SHA-256 `158876353204d5a82e98fae74b684026479352bf3715f969c3dbac05cdf1e728`). La suite `python3 tests/sal/escritorio/probar_host.py` pasó. Falta probar en placa arrancando `/bin/soso-wm`, creando/bindeando la superficie y confirmando que permanece activo sin page fault.
2. **Limitar el bloqueo de FATLOG sobre el sondeo WiFi** — completado parcialmente en código. El flush incremental ahora emite una escritura de sector (512 B) por vuelta del planificador; el tamaño de los lotes en los checkpoints síncronos no cambió. Se mide por separado la latencia de cada escritura ESP, y WiFi solo la atribuye como causa si es reciente, evitando confundirla con el tiempo acumulado de un flush incremental. `cargo xtask build` y `SOSO_LXDDE_MODE=nouveau,iwlwifi cargo xtask build` compilaron correctamente; esto valida compilación, no la latencia real de USB. No hay banco host existente que ejercite este camino de E/S del kernel. Pendiente en placa: repetir carga sostenida y comprobar que cada gap de sondeo durante el flush queda por debajo de 500 ms; correlacionar las líneas nuevas de causa/latencia y revisar que la escritura completa del log sigue avanzando.

No hay ítem de corrección para AX200, GSP Ampere o RTL8168H confirmado por este log. No se tocaron drivers ni se hizo un nuevo ciclo de placa.

## Qué no se ha hecho

- Se actualizó el pin de Sal y se regeneraron los binarios de host; no se flasheó el USB ni se validó el arranque en placa.
- No hubo sesión SSH entrante, prueba de reconexión WiFi, compute real de GPU, carga real ni apagado limpio.
- No se confirma que la Ethernet tuviera cable conectado.
