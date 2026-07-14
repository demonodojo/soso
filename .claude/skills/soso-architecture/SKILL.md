---
name: soso-architecture
description: >-
  soso OS architecture — kernel layout, sosofs CoW filesystem, 14 syscalls,
  userspace ABI, networking, SSH stack and coding constraints. Use when
  modifying kernel/, crates/, user/, xtask/, adding features, syscalls,
  drivers, or understanding how components interact.
---

# soso — Architecture

Learning OS in Rust. **10/10 phases complete.** Bare-metal x86_64 on QEMU q35.

## Workspace layout

```
soso/
├── kernel/           # no_std kernel (x86_64-unknown-none, outside root workspace)
├── xtask/            # build image, mkfs, QEMU launcher, integration tests
├── crates/
│   ├── sosofs/       # CoW FS (no_std + "std" feature for host tests)
│   ├── soso-abi/     # syscall numbers, Stat, Dirent, errno
│   └── block-dev/    # BlockDevice trait (virtio-blk, host File)
├── tools/
│   ├── mkfs-soso/    # host: rootfs dir → sosofs image + SSH key injection
│   └── ssh-proto/    # host prototype (phase 9), isolated from workspace
├── user/             # userspace workspace (libsoso, init, sosh, coreutils)
└── rootfs/           # source tree embedded into disk by mkfs-soso
```

## Kernel (monolithic, single-core)

- Ring 0: drivers, FS, network, SSH server — kernel never preempted
- Ring 3: ELF processes, round-robin preemptive scheduler
- **spawn, not fork** — one PML4 per process, static ELFs at `0x400000`
- Entry: `syscall`/`sysret` (MSRs STAR/LSTAR)

### 14 syscalls

`exit, read, write, open, close, seek, stat, getdents, mkdir, unlink, spawn, wait, sbrk, sleep_ms`

No pipes, signals, mmap, or Unix permissions.

### Key subsystems

| Module | Role |
|--------|------|
| `arch/` | GDT/TSS, IDT, PIC+PIT 100 Hz, paging |
| `drivers/` | serial, virtio-blk, virtio-net (PCI ECAM) |
| `fs/` | sosofs mounted on virtio-blk |
| `net/` | smoltcp, static IP 10.0.2.15, polled from scheduler |
| `ssh/` | sunset SSH-2 server, one session, launches `/bin/sosh` |
| `kshell.rs` | Emergency kernel-shell on serial (`soso>`) |
| `task/` | Processes, scheduler, syscall dispatch |

## sosofs (v1)

- 4 KiB blocks, little-endian, full CoW B+ tree (no journal)
- Dual superblocks A/B — atomic commit via generation increment
- CRC32C checksums on all nodes and file extents
- **Host-first development**: `cargo test -p sosofs --features std` with crash-injection before kernel integration
- Commits on write `close()` and every ~2 s

## Userspace

| Binary | Role |
|--------|------|
| `/bin/init` | PID 1: spawns sosh, relaunches on crash; `init test` = syscall regression suite |
| `/bin/sosh` | User shell — no pipes/redirections/variables |
| `/bin/{ls,cat,echo,mkdir,rm,hexdump,halt}` | Coreutils |

`libsoso`: crt0, syscall wrappers, mini-libstd (256 KiB heap arena).

## Network & SSH

- smoltcp TCP/IPv4 only; NIC polled (no IRQ-driven RX)
- sunset: curve25519 + ed25519 + chacha20-poly1305
- Auth: ed25519 public key only (`/etc/authorized_key`, 32 raw bytes)
- Host key: `/etc/ssh_host_key` (32-byte seed, persistent across mkfs)

## Coding constraints

1. **Minimize scope** — smallest correct diff; match existing style
2. **Kernel is `no_std`** — userspace uses `libsoso`, not std
3. **sosofs changes**: test on host first (`BlockDevice` over `File`)
4. **Do not break** `cargo xtask test` — it is the E2E gate
5. **QEMU fixed**: `-machine q35`, `-cpu max`, virtio PCI (not mmio)
6. **Spanish** for user-facing strings and docs in this repo

## Verification paths

| Layer | Command |
|-------|---------|
| sosofs unit + crash | `cargo test -p sosofs --features std` |
| Syscall regression | `/bin/init test` in QEMU |
| Full system | `cargo xtask test` |

## Out of scope (by design)

Multi-user, permissions, pipes, SFTP, DHCP/IPv6, fork, signals, snapshots/compression in sosofs.
