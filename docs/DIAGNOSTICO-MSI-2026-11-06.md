# Diagnóstico MSI (GB205 + AX211) — 2026-11-06

Arranque desde el pendrive correcto en el **MSI** (no el ROG). El GSP y el pool
de VRAM suben; el **clear 3D** no señaliza y el **CE** queda atascado en la
misma runlist que GR0. El log se corta tras `iwlwifi probe 8086:7f70 ok`, sin
`UCODE_ALIVE_NTFY` ni `sosh`. Este documento resume la evidencia del **run2**
y el orden de corrección aplicado en el árbol frente a lo que falta validar en
placa (sin flashear en esta entrega).

## 1. Evidencia y alcance

| Dato | Valor |
|------|-------|
| Máquina | MSI (placa distinta del ROG del mismo día) |
| Copia de logs | `target/usb-diagnostic-2026-11-06-run2/` (no va a git) |
| SOSOLOG | flush **#10**, ~**76 s**, **76 108** bytes útiles (`fb522bfc…` en hw-matrix) |
| Kernel | `soso 0.3.28 (886e8bb0d-dirty)`; imagen USB empaquetada 2026-11-05 |
| PCI | GPU `10de:2f18` (GB205), WiFi `8086:7f70` (AX211), ethernet `10ec:8125` |
| Referencia Linux | `lxdde/linux` 6.6.32 (iwl `iwl-io.c` / `iwl_finish_nic_init`); nouveau ref. `lxdde/reference/linux-master-nouveau` |
| Clase 3D Blackwell | OGKM 570.144 `clc597.h` (`0xce97`, `BLACKWELL_B`) |
| hw-matrix | `gb205-dgpu`, `ax211-wifi` actualizados con este log; **sin** subir `arranques_consecutivos_ok` ni ALIVE sin evidencia |

Distinto del diagnóstico **ROG** del mismo día ([DIAGNOSTICO-ROG-2026-11-06.md](./DIAGNOSTICO-ROG-2026-11-06.md)): allí hay `sosh`, ALIVE y scan; aquí el arranque no llega a userspace.

## 2. Etapas

| Etapa | Evidencia (SOSOLOG / contexto) | Resultado en USB run2 |
|-------|----------------------------------|------------------------|
| shim / live | Arranque desde ESP `KERNEL` del pendrive | ok |
| GPU GSP | `GSP_INIT_DONE`, pool VRAM (~12227 MiB) | ok |
| CE readback inicial | `G4e GO`, sondas `mueve` | ok (antes del clear) |
| 3D clear | semáforo no llega a 1; GR_FAULT / PBDMA en cadena con GR | **fallo** |
| CE tras clear | semáforo 11≠12, CE atascado | **fallo** |
| WiFi | `iwlwifi probe 8086:7f70 ok`; sin `UCODE_ALIVE_NTFY` visible después | **pendiente** (sin ALIVE probado) |
| `boot: gpu` / `sosh` | no alcanzados en el volcado | no |
| Ethernet RTL8125 | `10ec:8125` presente; en USB antiguo el driver puede no reclamar o el log corta antes de `boot: gpu` | validar en placa tras pasar GPU |

## 3. Hallazgos

### H1 — Directorio GR en inst+0x2a0 (Blackwell)

El RC encaja con **falta de join PDB** en el canal GR: RM escribe el directorio
en **inst+0x200**; Volta+ lee la ranura en **inst+0x2a0** (Linux `gv100_vmm_join`).
`chan_join_gr_pdb` en soso solo corría en Ampere; en **Blackwell** la ranura
+0x2a0 podía quedar vacía → fallo MMU en GR, mutex retenido, **PBDMA_HANG** y el
CE en la misma runlist deja de señalizar.

**En el árbol:** [`lxdde/ports/nouveau/gsp_chan.c`](../lxdde/ports/nouveau/gsp_chan.c) — `chan_join_gr_pdb` también para `NV_FAM_BLACKWELL`, copiando PDB
0x200→0x2a0 y la máscara en +0x298 (mismo criterio que `gv100_vmm_join`).

**SET_REPORT_SEMAPHORE_D:** el árbol mantiene `0x10 | (0xF << 12)` en
[`lxdde/ports/nouveau/gsp_3d.h`](../lxdde/ports/nouveau/gsp_3d.h) (release +
`PIPELINE_LOCATION_ALL`). No sustituye el join del PDB; ambos son necesarios
donde aplique.

### H2 — Poll del MAC del AX211 (25 ms frente a 25 s)

En Linux 6.6.32 `iwl_finish_nic_init` pasa **25000** como **microsegundos** con
intervalo 10 µs. Una implementación que use `lx_mdelay(1)` por iteración
convierte eso en **25 s** mudos tras el probe.

**En el árbol:** [`lxdde/ports/iwlwifi/iwl_poll.h`](../lxdde/ports/iwlwifi/iwl_poll.h), `iwl_trans.c` / `iwl_trans_8000.c` — sondeo en µs y mensaje
`finish_nic_init, esperando MAC_CLOCK_READY`. El firmware USB analizado seguía
el camino lento; el siguiente ciclo de placa con kernel nuevo debe mostrar salida
de esa espera o `MAC no despertó` en ~25 ms wall time.

### H3 — RTL8125 `10ec:8125` (VER_61/63)

El chip es **8125**, no 8168H. En el árbol el driver declara `0x8125` y usa el
camino **`rtl_hw_start_8125`** / init 8125 ([`kernel/src/drivers/rtl8169.rs`](../kernel/src/drivers/rtl8169.rs), [`kernel/src/drivers/rtl_xid.rs`](../kernel/src/drivers/rtl_xid.rs)).

**Pendiente en placa:** cuando el arranque pase de `boot: gpu`, comprobar en log
`rtl_hw_start_8125` / VER_61 o VER_63, enlace y ausencia del camino 8168H para
este PCI ID.

## 4. Orden de corrección (estado)

1. **Join PDB en canal GR para Blackwell** — implementado en `gsp_chan.c`;
   validar en placa: sin `GR_FAULT_DURING_CTXSW` por PDE en MAIN, sin
   `PBDMA_HANG_DURING_HTE` en la cadena del clear, CE sigue en `mueve` tras el
   selftest 3D.
2. **Poll iwl en µs (25 ms tope)** — implementado en ports iwlwifi; validar:
   líneas tras el probe o fallo explícito en tiempo corto.
3. **Semáforo 3D** — `GSP_3D_REPORT_RELEASE_WRITES` con `PIPELINE_LOCATION_ALL`;
   ya en `gsp_3d.h`; revalidar tras (1).
4. **RTL8125** — código listo; validar tras superar la secuencia GPU en el log.

## 5. Qué no se ha hecho

No se ha **reflachado** el USB ni se ha ejecutado un nuevo arranque MSI con el
kernel compilado desde este árbol. Los hostchecks de firmware no sustituyen ese
ciclo.
