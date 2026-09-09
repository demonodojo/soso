---
name: diagnostico-usb
description: >-
  Lee la ESP del USB live de soso (SOSOLOG/SOSODRV), compara fallos con Linux,
  actualiza docs/hw-matrix.json y prepara un plan de corrección.
  Usar con /diagnostico-usb.
---

# Diagnóstico del USB live

Lee el pendrive con el que arrancó soso, extrae los logs de la ESP, **compara
el port con Linux**, clasifica los fallos con evidencia, actualiza
`docs/hw-matrix.json` y entrega un **plan de corrección de código**. El
diagnóstico no está completo si falta la comparación Linux o si la matriz no
cambia. **No implementes el plan** salvo que el usuario lo pida después.
**No flashees.**

Los pasos 1–4 de este fichero son la receta de análisis: **ejecútalos ya**.
No los conviertas en el plan que ve el usuario.

## Resultado (también en modo Plan)

El entregable visible es el **orden de corrección** (un ítem por defecto
confirmado: archivos soso, Linux `archivo:línea`, cambio concreto, validación
host/placa). Ejemplo de calidad: la sección *Orden de corrección* de
`docs/DIAGNOSTICO-ROG-2026-09-09.md`.

**Prohibido** como plan / `CreatePlan` / todos del modo Plan:

- montar la ESP, copiar SOSOLOG, clasificar líneas, comparar con Linux,
  escribir el informe, actualizar la matriz

Eso lo hace este comando en silencio. En modo Plan no pidas aprobación para
analizar; analiza (lectura USB, código, Linux) y el plan que creas **es** el
de corrección. Si el modo es solo lectura, no escribas JSON/markdown todavía:
el plan son los cambios de código; matriz e informe van como últimos ítems
de registro de ese mismo plan, no como receta previa.

No cambies a Agent solo para «planear el análisis».

Antes de actuar, lee estas skills (en este orden):
`.claude/skills/soso-live/SKILL.md`, `.claude/skills/soso-dev/SKILL.md`,
y si el log toca GPU o WiFi también `soso-gpu` y `soso-wifi`.

## Restricciones

- **No** lances `cargo xtask sosolog` ni `sudo mount`: piden TTY/contraseña.
- Monta la ESP (partición **1**, FAT/EFI) con `udisksctl`. Nunca p4 `SOSOINSTALL`.
- Tras copiar, **desmonta** p1. Un `dd` posterior necesita la partición libre.
- No marques etapas `ok` en `docs/hw-matrix.json` sin evidencia en el log
  (`sosh`, `UCODE_ALIVE_NTFY`, `GSP_INIT_DONE` / RPC). `parse-logs` puede
  inventar `dhcp: ok` si mezcla `net: dhcp` con otro `ok`: revisa a mano.
- No ejecutes `flash-usb-live`. Si hace falta reflashear, deja el comando al usuario.

## 1. Localizar y copiar la ESP

```bash
lsblk -o NAME,SIZE,TYPE,FSTYPE,LABEL,PARTTYPENAME,RM
```

Candidato: `RM=1`, partición 1, `vfat` / EFI. Etiqueta típica `kernel`.
Si el usuario pasa `/dev/sdX` o `/dev/sdX1`, úsalo. Si hay varios, elige el
extraíble y dilo. Si no hay USB, para y pide que lo conecte; no inventes logs.
Copias previas en `target/usb-diagnostic-*/` solo si el usuario lo autoriza.

```bash
ESP_DEV=/dev/sdX1   # o el candidato detectado
findmnt -n -o TARGET "$ESP_DEV"
# si vacío:
udisksctl mount -b "$ESP_DEV" --no-user-interaction
ESP=$(findmnt -n -o TARGET "$ESP_DEV")
```

Si `udisksctl` falla, deja el comando al usuario. No reintentes con sudo.

Fecha de hoy (`YYYY-MM-DD`). Copia a `target/usb-diagnostic-YYYY-MM-DD/`:

| Fichero ESP | Obligatorio |
|-------------|-------------|
| `SOSOLOG.TXT` | sí |
| `SOSODRV.TXT` | sí si existe |
| `BOOTMARK.TXT` | sí si existe |
| `SOSOWIFI.TXT` | si existe (no copies el PSK al informe) |
| `SOSOBOOT.TXT`, `SOSOUPD.TXT`, `SOSOKRN.MET` | si existen |

Guarda el binario original (`SOSOLOG.TXT`) y una copia de texto recortando
relleno FAT (`tr -d '\000'`, luego `sed` de NULs/espacios al final). El hueco
de `SOSOLOG.TXT` es 256 KiB: el log real acaba en el primer bloque de ceros.

Lista el resto de la ESP (nombres, tamaños) para ver huecos 8.3 y versión del
kernel empaquetado. Luego:

```bash
udisksctl unmount -b "$ESP_DEV" --no-user-interaction
```

## 2. Lectura mínima del arranque

Del `SOSOLOG` recortado, extrae:

- versión / commit (`soso 0.2.x`, git corto, dirty)
- último `boot:` alcanzado (`memtest` … `task`)
- `sosh —` (¿llegó a userspace?)
- panic / `EXCEPTION:` / RIP
- `hwscan:` y líneas `drv:` / `RED SIN DRIVER`
- GPU: `NV_PMC_BOOT_0`, firmware GSP, `FWSEC-FRTS`, booter, `GSP=`, `pool VRAM=`
- WiFi: `UCODE_ALIVE_NTFY`, `alive=`, scan (`count=`, `complete=`, `notif=`), assoc, DHCP
- red ethernet: driver (`e1000e` / `rtl8169` / virtio), enlace, lease
- `lx: lxdde: stub trace:` (shim dummy que pisa builtins)
- flush `fatlog` / uptime

`BOOTMARK` vacío → el firmware no ejecutó el USB.
`BOOTMARK` escrito y `SOSOLOG` vacío → el kernel no volcó (mira checkpoints en pantalla).

Del `SOSODRV`: PCI `VVVV:DDDD`, driver `compilado|ausente|desconocido`.
IDs conocidos: GPU `10de:2f18` GB205, `10de:249c` GA107; WiFi `8086:7f70` AX211,
`8086:2723` AX200.

## 3. Clasificar fallos (no teorizar sin línea)

Cada hallazgo necesita: cita del log (línea o texto), fichero de código
sospechoso, y si está **confirmado** (código vs Linux) o es **hipótesis**.

Patrones frecuentes (skills `soso-dev` / `soso-gpu` / `soso-wifi` / `soso-live`):

| Señal en el log | Qué mirar |
|-----------------|-----------|
| se queda en un `boot:` | el paso siguiente en `kernel/src/main.rs`; stub `memcmp` en live-disk |
| `lx: lxdde: stub trace: memcmp` | `lxdde/shim/src/shims.c` + `provided_symbols()` |
| `GSP=fallo` / `pool VRAM=no` | `gsp_bringup.c`, `falcon_lx.c`; Ampere ≠ GB205 |
| `reset falló` Falcon/SEC2 | base PRI (Ampere SEC2 **0x840000**, no PTOP 0x87000) |
| `UCODE_ALIVE_NTFY` pero scan `count=0` | doorbell `qid<<16`, cola HCMD=0, init NVM, SCAN v15/v17, RX MQ |
| `timeout ALIVE` / `alive=false` | gen2 RX restock; firmware/pnvm; **no** es «ALIVE degradado» |
| `wifi: ninguna red` / `scan=fallo` | no interpretar como «no hay BSS» hasta validar TX/RX |
| `net: no se encontró ningún virtio-net` en bucle | `try_attach` 1/s; no reenumerar PCI |
| `RED SIN DRIVER` | `registry.rs` IDs o port nuevo; `fit-drivers` |
| panic / `#PF` / `#GP` / `#XM` | RIP: user `0x4xxxxx`, kernel `0x100000xxxxx` |
| teclado muerto tras 1 tecla | IRQ1 + `lock()` (`HOSTS`/`PROCS`); QEMU SSH no lo ve |
| OTA / `SOSOUPD` / `PROBANDO` | skill `soso-live`, `SOSOKRN.MET` |

## 3b. Comparar con Linux (obligatorio)

Cada hallazgo de driver (GPU, WiFi, ethernet, USB/xHCI si aplica) debe
contrastarse con el árbol Linux local **antes** de proponer el plan. Un
hostcheck verde no sustituye esta lectura. No teorizar sin abrir ambos lados.

Árboles (solo lectura; no los copies al informe entero):

| Árbol | Cuándo |
|-------|--------|
| `lxdde/linux/` (6.6.32, lo baja `cargo xtask lx-build`) | iwlwifi, e1000e, r8169, xHCI, Ampere GA10x/GA107 en nvkm 6.6 |
| `lxdde/reference/linux-master-nouveau/` | GPU GB205 / r570 / `gb202.c`; Ampere si 6.6 no basta |
| `lxdde/reference/open-gpu-kernel-modules-570.144/` | offsets PRI/FIFO/CE/FSP GB205 (no uses 6.6 para Blackwell) |
| `lxdde/reference/linux-6.15-nouveau/` | ACR Ampere / r535 si el master no aclara |

Si falta `lxdde/linux/`, `cargo xtask lx-build iwlwifi` o `nouveau` lo extrae.
Si falta `lxdde/reference/`, regenera con `lxdde/reference/README.md` (red).
Anota commit/tag del árbol usado (como en `docs/DIAGNOSTICO-ROG-2026-09-09.md`).

Rutas típicas a abrir según el fallo:

- WiFi: `drivers/net/wireless/intel/iwlwifi/{queue/tx.c,pcie/tx-gen2.c,mvm/fw.c,mvm/scan.c,mvm/rxmq.c,mvm/ops.c}`
- GPU Ampere: `drivers/gpu/drm/nouveau/nvkm/{engine/sec2/ga102.c,subdev/gsp/ga102.c,falcon/fw.c}`
- GPU GB205: `nvkm/subdev/gsp/rm/r570/`, OGKM `published/blackwell/gb202/`
- Ethernet: `drivers/net/ethernet/realtek/r8169*.c`, `intel/e1000e/`

Por cada discrepancia, en el informe:

1. Qué hace soso (`archivo:línea` del port).
2. Qué hace Linux (`archivo:línea` del árbol elegido).
3. Si es **compatible con el síntoma del SOSOLOG** (confirmado) o solo una
   diferencia no causal (descartada).
4. Valores concretos (registros, tamaños, IDs de cola, offsets DMA), no
   «Linux lo hace distinto».

No cites web si el fichero está en el checkout. Si un paso de Linux no está
portado, dilo como etapa ausente (no como bug de una línea suelta).

Host (baratos, siempre; no validan silicio ni esta comparación):

```bash
./scripts/l6-iwl-fw-hostcheck.sh
./scripts/l6-g3-gsp-hostcheck.sh
```

Copia su salida junto a los logs. Anota qué **no** cubren (transporte, DMA
Falcon, scan real).

## 4. Matriz hardware (obligatorio)

Entregable: `docs/hw-matrix.json` editado en esta misma ejecución. El agente
**sí** puede escribir ese JSON (no pide sudo). Sin este paso el comando falla.

Mapa PCI del `SOSODRV` → `--id` (una pasada `parse-logs` por cada id presente):

| PCI | Entrada |
|-----|---------|
| `10de:2f18` | `gb205-dgpu` |
| `10de:249c` | `ga107-igpu` |
| `8086:7f70`, `51f0`, `54f0` | `ax211-wifi` |
| `8086:2723` | `ax200-wifi` |

Si el PCI no tiene entrada: créala antes con
`cargo xtask hw-matrix collect --id <id> --equipo "…" --pci vvvv:dddd --perfil live-usb`
(PCI concreto hex, sin `????`). No toques `qemu-test` ni entradas de chips
ausentes en este hwscan.

```bash
DIR=target/usb-diagnostic-YYYY-MM-DD
# usa las copias recortadas (sin relleno FAT)
cargo xtask hw-matrix parse-logs --id <id> --sosolog "$DIR/SOSOLOG.txt" --sosodrv "$DIR/SOSODRV.txt"
cargo xtask hw-matrix show
```

Tras `parse-logs`, **revisa y corrige a mano** cada entrada tocada:

- `fail` si el log lo demuestra (`GSP=fallo`, `pool VRAM=no`, `timeout ALIVE`,
  `alive=false`, scan `count=0` / `wifi: ninguna red` / `scan=fallo`).
- `ok` solo con evidencia explícita: `sosh —`, `UCODE_ALIVE_NTFY` (no
  «ALIVE degradado»), `GSP_INIT_DONE` / RPC. No conviertas un `pendiente` en
  `ok` porque el parser vio la palabra `ok` en otra línea.
- Falso positivo conocido: `dhcp: ok` si aparece `net: dhcp` mezclado con
  otro `ok` o con ethernet DOWN. DHCP WiFi exige lease en el backend
  `LxWifi`, no rtl8169/e1000e.
- Rellena `notas` con el diagnóstico de esta ejecución (commit del kernel
  del USB, flush/uptime, causa concreta, enlace al `docs/DIAGNOSTICO-*.md`).
- Añade las rutas de las copias a `logs`. Actualiza `git` si el SOSOLOG trae
  versión/commit distintos.
- **No** `record-boot`, **no** `--boot-ok`, **no** incrementes
  `arranques_consecutivos_ok`. Un arranque a sosh no es aceptación.

## 5. Informe (agente) y plan de corrección (lo que ve el usuario)

En modo Agent, escribe `docs/DIAGNOSTICO-<EQUIPO>-YYYY-MM-DD.md` (equipo
corto: ROG, GB205, …). Español. Estructura del fichero (no es el plan):

1. **Evidencia y alcance** — dispositivo ESP, kernel/commit, PCI, árboles Linux
   usados (ruta + tag/commit), qué no se valida.
2. **Tabla de etapas** — etapa, cita del SOSOLOG, resultado.
3. **Hallazgos** — uno por defecto: síntoma, soso `archivo:línea`, Linux
   `archivo:línea`, confirmado vs hipótesis.
4. **Orden de corrección** — el mismo contenido que el plan visible.
5. **Qué no se ha hecho** — ni drivers tocados ni ciclo de placa de la corrección.

**Plan visible** (chat y, en modo Plan, `CreatePlan`): solo el orden de
corrección. Un todo por cambio de código, bloqueantes primero. Título del
plan = corrección (p. ej. «Corregir GSP Ampere y scan AX200»), nunca
«Analizar SOSOLOG» ni «Diagnosticar USB». Cada ítem: archivos a tocar,
discrepancia Linux, criterio de hecho en host y en placa.

Mal: `1. Montar /dev/sda1  2. Leer SOSOLOG  3. Comparar con Linux  4. Proponer fixes`.
Bien: `1. Override SEC2 Ampere a 0x840000  2. DMA del payload HS v2  3. Doorbell HCMD qid<<16`.

Al usuario: resumen corto de evidencia + entradas de matriz (`id` + etapas)
si ya se escribieron + el plan de corrección. Pregunta si implementa el
primer ítem. No pegues el PSK. No subas `target/` al git.
