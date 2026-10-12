# soso — OS minimalista en Rust con FS CoW propio y SSH real

## Contexto

Proyecto greenfield en `/home/jmdiez/Trabajo/demonodojo/soso` (directorio vacío). Objetivo: un sistema operativo de aprendizaje, kernel propio bare-metal en Rust sobre QEMU x86_64, monousuario, con:

- **Filesystem propio moderno** (copy-on-write, checksums, commits atómicos) — pieza central del proyecto.
- **SSH real** (protocolo SSH-2 con cifrado) para entrar al sistema.
- Mínimo viable: arrancar → terminal (serie primero, SSH después) → acceso al FS → ejecutar ELFs en userspace.

Proyecto de meses, organizado en fases donde **cada fase deja algo funcionando y verificable en QEMU**.

## Prerrequisitos del entorno (detectado en la máquina)

- El rustc del sistema es 1.75 (2023) y no hay rustup → instalar **rustup** con stable ≥1.95, edition 2024, target `x86_64-unknown-none`.
- No hay `qemu-system-x86_64` → instalar QEMU (`sudo apt install qemu-system-x86`).

## Decisiones de stack (verificadas jul 2026)

| Área | Elección |
|---|---|
| Boot | `bootloader`/`bootloader_api` 0.11.15 (rust-osdev) — imagen BIOS+UEFI 100% cargo |
| CPU | `x86_64` 0.15.5 (GDT/IDT/paging) |
| Interrupciones | PIC 8259 (`pic8259` 0.11) + PIT 100 Hz — nada de APIC/ACPI (monocore, minimalismo) |
| Drivers | `virtio-drivers` 0.13 (rcore-os), transporte PCI/ECAM → QEMU `-machine q35` obligatorio |
| Red | `smoltcp` 0.13.1 (ipv4 + tcp, sin ipv6/dhcp; IP estática 10.0.2.15) |
| SSH | **`sunset` 0.5** (no_std, del autor de Dropbear): servidor SSH-2, curve25519, chacha20-poly1305, ed25519. API event-driven no-async (sin Embassy) |
| Crypto | Las transitivas de sunset (RustCrypto, no_std) — no fijar a mano |
| ELF | `xmas-elf` 0.10 (zero-alloc) |
| Allocator | `talc` 4.x |
| Checksums FS | CRC32C (`crc32fast` sin default features) + `zerocopy` para structs on-disk |

## Arquitectura del kernel

- **Monolítico simple**, todo en ring 0 (drivers, FS, red, SSH). Monocore.
- Cooperativo (bucle de poll) hasta userspace; después **preemptivo round-robin solo para procesos de usuario** (el kernel nunca se desaloja → locking trivial).
- Procesos: **`spawn`, no `fork`** (evita CoW de address spaces). Binarios ELF estáticos a dirección fija, un PML4 por proceso.
- Entrada: instrucciones `syscall`/`sysret` (MSRs STAR/LSTAR).
- **14 syscalls**: `exit, read, write, open, close, seek, stat, getdents, mkdir, unlink, spawn, wait, sbrk, sleep_ms`. Fuera: pipes, signals, mmap, permisos.
- Abstracción **`Tty`** (cola RX + sink TX) con backend serie y backend SSH — la misma shell funciona por ambos.

## Estructura del workspace

```
soso/
├── Cargo.toml                # workspace (Cargo.lock commiteado, versiones exactas)
├── rust-toolchain.toml
├── .cargo/config.toml        # alias cargo xtask ...
├── xtask/                    # host: compila todo, empaqueta imagen, lanza QEMU
│                             #   (q35, virtio-blk-pci, virtio-net-pci, hostfwd 2222→22)
├── kernel/src/
│   ├── arch/  mm/  task/  drivers/  fs/  net/  ssh/  kshell.rs
├── crates/
│   ├── sosofs/               # ★ el FS: no_std + feature "std" → testeable en host
│   ├── soso-abi/             # nºs de syscall, Stat/Dirent/errno (kernel ↔ user)
│   └── block-dev/            # trait BlockDevice (impl: virtio-blk y File de host)
├── tools/mkfs-soso/          # host: crea imagen sosofs desde un dir + host key ed25519
└── user/                     # workspace aparte (linker script propio)
    ├── libsoso/              # crt0 + wrappers syscall + mini-libstd
    ├── init/  sosh/          # PID 1 y la shell
    └── bin/                  # cat, ls, echo, mkdir, rm, hexdump
```

## Diseño de sosofs (v1)

Bloques de 4 KiB, little-endian, direcciones de 64 bits. CoW total, sin journal.

- **Layout**: bloques 0 y 1 = superbloques A/B (lo único que se sobreescribe, alternando); resto área general CoW.
- **Superbloque**: `{ magic "SOSOFS10", crc32c, generation, block_count, tree_root, bitmap_start/blocks, next_inode }`. Al montar se elige el slot válido de mayor `generation`.
- **Un único árbol B+ CoW** (estilo FS-tree de btrfs), clave `(inode, kind, offset)` con items `INODE`, `DIRENT` (offset = hash(nombre)), `EXTENT` (con crc32c de los datos). Cada nodo del árbol lleva `{ crc32c, generation, level, nkeys, bloque_esperado }`.
- **Asignación**: bitmap en RAM, reescrito completo y CoW en cada commit (≤128 KiB a 4 GiB). Dos bitmaps ("actual"/"post-commit"): un bloque liberado en la transacción T no se reutiliza hasta después del commit de T.
- **Commit atómico**: escribir bloques nuevos → flush → superbloque en slot alterno con generation+1 → flush. Un corte en cualquier punto deja la generación anterior íntegra. Commit en `close()` de escritura y cada 2 s.
- **Fuera (fases futuras)**: compresión, snapshots (casi gratis: raíces viejas), hardlinks, permisos.
- **Estrategia clave: host-first.** sosofs se desarrolla y prueba en host (BlockDevice sobre `File`) con tests de propiedad y **crash-injection** (truncar la secuencia de escrituras en cada punto y verificar que siempre monta N o N-1). El kernel consume la crate ya probada.

## Fases (cada una con entregable verificable)

| # | Fase | Entregable verificable | Est. |
|---|---|---|---|
| 0 | Entorno + boot | rustup/QEMU instalados; `cargo xtask run` imprime `soso 0.1` por serie (uart_16550, panic handler, isa-debug-exit) | 1-2 sem |
| 1 | Memoria e interrupciones | GDT+TSS, IDT (double fault con IST), PIC+PIT, frame allocator, paging, heap `talc`; `Vec` funciona; page fault → diagnóstico, no triple fault | 2-3 sem |
| 2 | Kernel-shell por serie | Prompt `soso>` interactivo por `-serial stdio`: help, mem, uptime | 1 sem |
| 3 | PCI + virtio-blk | Enumeración ECAM, trait `Hal`, leer/escribir sectores persistentes desde el prompt | 2 sem |
| 4 | sosofs solo-lectura + mkfs | `mkfs-soso` crea imagen desde dir host; `cat /etc/motd` en el prompt; un byte corrupto → error de checksum | 3-4 sem |
| 5 | sosofs escritura CoW | Crear ficheros desde el prompt; matar QEMU en mitad de escrituras → siempre monta íntegro (crash-injection en host) | 3-4 sem |
| 6 | Userspace | Ring 3, syscall/sysret, 14 syscalls, ELF con xmas-elf, scheduler preemptivo, `soso-abi` + `libsoso`; `/bin/init` imprime desde ring 3 | 3-4 sem |
| 7 | Shell de usuario + coreutils | `sosh` (sin pipes) + ls/cat/echo/mkdir/rm como ELFs reales; kernel-shell queda de emergencia | 2 sem |
| 8 | virtio-net + smoltcp | Echo TCP puerto 7; `nc localhost 7777` responde; ping funciona | 2-3 sem |
| 9 | SSH con sunset | `ssh -p 2222 localhost` abre una `sosh` cifrada (host key ed25519 de `/etc/ssh_host_key`); 1 sesión, sin SFTP/forwarding. **Antes: prototipo host sunset↔smoltcp sobre TUN** | 3-4 sem |
| 10 | Auth + pulido | Auth solo por clave pública (`/etc/authorized_key` inyectada por mkfs desde `~/.ssh/id_ed25519.pub`), motd, halt, `cargo xtask test` (boot, crash-safety, TCP, handshake SSH) | 1-2 sem |

Total: ~7-9 meses a ritmo de proyecto de aprendizaje.

## Riesgos y mitigaciones

1. **Integración sunset↔smoltcp** (poco ejemplo público): prototipar en host sobre TUN antes de tocar el kernel; plan B = mini-executor async en kernel para `sunset-async`.
2. **FS CoW = mayor riesgo de calendario**: host-first + crash-injection; recortes ya hechos (un árbol, bitmap completo, sin snapshots).
3. **virtio PCI exige ECAM**: `-machine q35` fijado en xtask desde fase 0; plan B `-M microvm` con virtio-mmio.
4. **Triple faults opacos en ring 3**: `-d int -no-reboot` y GDB stub documentados en xtask desde fase 1; double fault con IST desde el día 1.
5. **Deriva de versiones 0.x**: Cargo.lock commiteado, versiones exactas, solo actualizar entre fases.

## Verificación

- Cada fase cierra con su entregable ejecutado en QEMU (tabla anterior).
- `sosofs`: `cargo test` en host (unit + crash-injection) antes de integrarse en kernel.
- Al final: `cargo xtask test` automatiza boot, integridad del FS tras kill, echo TCP y handshake SSH scriptado.

## Primer paso de implementación (fase 0)

1. Instalar rustup (stable, `rust-src` no necesario) y QEMU.
2. `Cargo.toml` del workspace + `rust-toolchain.toml` + `.cargo/config.toml`.
3. `xtask` que compila el kernel para `x86_64-unknown-none`, genera la imagen con `bootloader` 0.11 y lanza `qemu-system-x86_64 -machine q35 -serial stdio`.
4. `kernel/src/main.rs` con `bootloader_api::entry_point!`, driver serie y `panic_handler`.
