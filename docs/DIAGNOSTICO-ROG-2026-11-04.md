# Diagnóstico ROG — 2026-11-04

El live arranca bien hasta `boot: task`, pero `/bin/init` muere al instante
con un page fault y la máquina cae en `kernel-shell`. El hardware está sano
(GSP, VRAM, WiFi, DHCP). La causa es un pendrive con **kernel de hoy y rootfs
del 24 de octubre**, que dejó a medias un `flash-usb-live --skip-models`.

## 1. Evidencia y alcance

| Dato | Valor |
|------|-------|
| Pendrive | `/dev/sda`, «General UDisk» `048d:1234`, 500 GiB anunciados, USB 2.0 |
| Logs | copia de la ESP hecha a las 00:11 (`target/usb-diagnostic-2026-11-04/`), flush #20, uptime 121 s |
| Kernel arrancado | `soso 0.3.28 (270b0bef3)` (HEAD, 2026-11-03 23:40) |
| Rootfs montado | sosofs generación 206, 245 760 bloques (960 MiB); modelos `qwen27-iq3` |
| `SOSOHASH.TXT` | `version=0.3.22`, `build=bbbf7820d-dirty` (2026-10-24), rootfs 960 MiB, modelos 14 GiB |
| PCI | `10de:249c` (GA104), `8086:2723` (AX200), `10ec:8168` (rtl8169) |
| Linux | `lxdde/linux/` 6.6.32, `fs/binfmt_elf.c` |

Después de la copia, la ESP del pendrive dejó de reconocerse. Linux la montó
a las 00:10:56 y escribió 3 sectores al desmontarla, y desde entonces `blkid`
y udisks no ven un FAT en la p1, ni tras `Block.Rescan`. No se ha podido
releer en crudo sin sudo. Hay que reflashear de todas formas.

No se valida: ni la corrección (no hay código tocado) ni un arranque nuevo en placa.

## 2. Etapas

| Etapa | Cita del SOSOLOG | Resultado |
|-------|------------------|-----------|
| shim UEFI | `BOOTMARK`: `bootsoso.efi leído (156672 bytes)` | ok |
| live-disk | `live: GPT backend=Usb root LBA 393250 (960 MiB) models LBA 2490402 (14336 MiB)` | ok |
| fs | `fs: sosofs live (generación 206, 245760 bloques)` | ok, pero rootfs viejo |
| GPU | `GSP_INIT_DONE recibido tras 6 mensaje(s)`; `GSP=rm_compute, pool VRAM=sí` | ok |
| WiFi | `firmware ALIVE (UCODE_ALIVE_NTFY)`; `asociado a 'Rutilo'`; `4-way completado` | ok |
| DHCP | `net: backend lx-wifi` → `net: dhcp 192.168.68.132/24` | ok |
| init | `mmap-fault: pid=1 va=0x412c86 wr=0 ejecución en página no ejecutable` | **fallo** |
| userspace | `task: [1] /bin/init matado: page fault` → `kernel-shell lista` | sin sosh |

## 3. Hallazgos

### H1 — `--skip-models` deja el pendrive con kernel nuevo y rootfs viejo (confirmado)

- A las 23:54 se lanzó `flash-usb-live /dev/sda --yes --skip-models`.
- `write_incremental` ([xtask/src/flash_usb_live.rs:270-307](../xtask/src/flash_usb_live.rs#L270-L307))
  escribe primero el kernel en la ESP (`update_esp_from_uefi`) y luego llama a
  `dd_to_partition` para la p2.
- `dd_to_partition` ([xtask/src/package_live.rs:1051](../xtask/src/package_live.rs#L1051))
  rechaza el rootfs porque no cabe: `target/soso-data.img` ocupa 2,8 GiB
  (`soso-live.bytes` = 3 356 491 776) y la p2 del pendrive, 960 MiB. El proceso
  sale con `exit(1)` sin deshacer el kernel ya escrito.
- Prueba en el log: banner 0.3.28 con sosofs de 960 MiB; `SOSOHASH` sigue en 0.3.22.
- El flasheo completo de las 00:06 (`SOSO_LIVE_MODEL=qwen36-35b`) no llegó a
  escribir el pendrive: la p3 sigue con `qwen27-iq3`, mide 14 GiB sin estirar y
  la GPT de respaldo no está reparada (`Alternate GPT is invalid`). Coincidió
  con un `cargo xtask test-install` (00:06:17) que reempaqueta el mismo
  `target/usb-live/` y recompiló el binario de xtask mientras tanto
  (`xtask (deleted)` en el registro de sudo). Esto último es hipótesis: no
  está la salida del flasheo.

### H2 — la carga perezosa de ELF pone NX en la página compartida `.text`/`.rodata` (confirmado)

- El commit `e1e681470` (2026-10-30) añade `ALIGN(4096)` antes de `.rodata` en
  [user/link.ld](../user/link.ld) y la comprobación W^X del kernel. Los binarios
  enlazados antes de esa fecha, como el `init` del rootfs viejo, tienen el final
  de `.text` y el principio de `.rodata` en la misma página. Hoy `init` está alineado:
  `R E 0x400000-0x413b0e`, `R 0x414000…`.
- La carga normal ([kernel/src/task/elf.rs](../kernel/src/task/elf.rs), `load`)
  junta los permisos de cada página (`merge_page_prot`) y avisa con
  `elf: página W+X compartida … (transición W^X)`.
- La carga perezosa (`load_lazy`, [elf.rs:228](../kernel/src/task/elf.rs#L228))
  crea una región por segmento. Al fallar una página,
  [kernel/src/task/mod.rs:963](../kernel/src/task/mod.rs#L963) la mapea con
  `region_prot` de la región que devuelve `find_mmap_region`. Si la página
  compartida se toca antes leyendo `.rodata`, queda `R` + NX. Al ejecutar código
  en ella salta [mod.rs:824](../kernel/src/task/mod.rs#L824) («ejecución en
  página no ejecutable») y `init` muere. Encaja con `va=0x412c86`, cerca del
  final de `.text`.
- Linux 6.6.32 mapea cada `PT_LOAD` con su propio `make_prot`
  (`lxdde/linux/fs/binfmt_elf.c:1070`) y con `MAP_FIXED` el siguiente pisa la
  página compartida. No junta permisos, así que esos binarios también fallarían
  en Linux. La diferencia no es que soso falle donde Linux no: es que soso
  promete la transición en `load` y no la cumple en `load_lazy`.
- El layout exacto del `init` viejo es inferido: la p2 del pendrive no se
  puede leer sin sudo.

## 4. Orden de corrección

1. **Flasheo incremental atómico** — [xtask/src/flash_usb_live.rs](../xtask/src/flash_usb_live.rs)
   `write_incremental`. Antes de tocar la ESP, comprobar que cada parte cabe
   (sacar de `dd_to_partition` la comprobación de tamaño como
   `check_fits_partition`). Si el rootfs no cabe, abortar sin escribir nada y
   decir «haz un flash completo». Host: test con una p2 pequeña que compruebe que
   la ESP queda intacta tras el error. Placa: `--skip-models` sobre este
   pendrive debe negarse y dejarlo como estaba.
2. **`load_lazy` coherente con `load`** — [kernel/src/task/elf.rs](../kernel/src/task/elf.rs)
   y [kernel/src/task/mod.rs](../kernel/src/task/mod.rs) (`handle_mmap_fault`,
   líneas 695/886/963). Al mapear una página de una región ELF, unir los
   permisos de todas las regiones que la tocan, como hace `merge_page_prot`
   (R ∪ RX = RX; si sale RWX, el mismo aviso de transición). Host: test de
   QEMU con un binario enlazado sin `ALIGN(4096)` (el `link.ld` de antes del
   30-10) que llegue a `main`. Placa: el rootfs viejo arranca con el kernel nuevo.
3. **Empaquetado exclusivo** (opcional, hipótesis H1) — `package_live::run`
   ([xtask/src/package_live.rs:88](../xtask/src/package_live.rs#L88)): cerrojo
   en `target/usb-live/.lock` para que `test-install` y `flash-usb-live` no
   reescriban a la vez `soso-live.img`. Host: dos ejecuciones simultáneas, y la
   segunda espera o falla con un mensaje claro.

Para recuperar el pendrive ya (no depende de lo anterior), con el pendrive
conectado y **sin** `test-install` corriendo:

```bash
cargo xtask flash-usb-live /dev/sda --yes
```

## 5. Qué no se ha hecho

- Punto 1 hecho el mismo día: `write_incremental_parts` comprueba antes de
  escribir (`package_live::check_fits_partition`). Tests de host
  `skip_models_sin_sitio_para_el_rootfs_no_toca_la_esp` y
  `skip_models_con_sitio_escribe_kernel_y_rootfs`. `require_mkfs_vfat` usaba
  `mkfs.vfat -h`, que sale con 1 en dosfstools 4.x: los tests con FAT se
  saltaban en silencio, ahora corren. Puntos 2 y 3 sin hacer; no se ha reflasheado.
- Además, para que no vuelva a faltar sitio: la p2 del live mide
  `max(2 × rootfs, rootfs + 4 GiB)`, recortada si no cabe con el modelo
  (`package_live::live_root_partition_bytes`). El flash completo no escribe la
  holgura (`live_copy_ranges`) y `soso-install` no la clona
  (`sosofs::uso::bloques_en_uso` + `hueco_rootfs`).
- No se ha arrancado la placa con ninguna corrección.
- `docs/hw-matrix.json`: `ga107-igpu` y `ax200-wifi` actualizadas (GPU y WiFi
  ok con fecha de hoy, DHCP ok por lx-wifi, `sesion_sostenida=fail`,
  `arranques_consecutivos_ok=0`).
