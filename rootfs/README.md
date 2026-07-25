# rootfs — sosofs source tree

This directory is the **source tree** for soso's root filesystem. At build time,
`mkfs-soso` embeds it into a sosofs disk image (`target/soso-data.img`), which
QEMU attaches as virtio-blk0 (or NVMe, depending on configuration).

Userspace binaries, SSH keys, and optional NVIDIA firmware are layered on top
before or during that step. At runtime the guest mounts this image as `/`.

## Build pipeline

`cargo xtask build` / `run` / `test` do the following:

1. **Userspace** — compile `user/` (release) and copy ELFs into `rootfs/bin/`
   (`init`, `sosh`, coreutils, `soso-llm`, …).
2. **Firmware (optional)** — if gb205 GSP blobs are missing, xtask tries
   `./scripts/l6-pack-firmware.sh` automatically.
3. **mkfs-soso** — walk `rootfs/` and write `target/soso-data.img`:
   - default **64 MiB** when no gb205 firmware is present;
   - **128 MiB** when `lib/firmware/nvidia/gb205/gsp/bootloader-570.144.bin`
     exists.
4. **SSH keys** — `mkfs-soso` injects into `rootfs/etc/` before packing:
   - `ssh_host_key` — 32-byte ed25519 seed (created once, kept stable across
     rebuilds);
   - `authorized_key` — client public key from `~/.ssh/id_ed25519.pub` or
     `target/soso_test_key`.

Force-regenerate the data disk without a full rebuild:

```sh
cargo xtask mkfs
```

## Layout

| Path | In git | Role |
|------|--------|------|
| `bin/` | no (`.gitignore`) | Userspace ELFs copied by xtask |
| `etc/motd` | yes | Login banner |
| `etc/ssh_host_key` | no | Persistent SSH host key seed |
| `etc/authorized_key` | no | Client key allowed for SSH |
| `lib/firmware/nvidia/` | partial | GSP firmware for L6 GPU bring-up (optional) |
| `hola.txt` | yes | Sample file on the root filesystem |

**Note:** LLM models live on a **separate** sosomfs disk (`target/soso-models.img`,
virtio-blk1) under `/models/`. Anything under `rootfs/models/` in the repo is
not used at runtime and only increases the data image size if left in place.

## NVIDIA firmware (L6 / GSP)

For native GPU work (nouveau/nvkm port), pack firmware from the host's
`linux-firmware` tree:

```sh
./scripts/l6-pack-firmware.sh          # gb205 (+ ga102 reference set)
./scripts/l6-pack-firmware.sh --repack # clean copy
```

Expected layout for the primary target (GB205):

```
lib/firmware/nvidia/gb205/gsp/
  bootloader-570.144.bin
  fmc-570.144.bin
  gsp-570.144.bin
```

Blobs are stored as decompressed `.bin` files (soso has no zstd in the kernel).
Ampere (`ga102/`) files are kept for reference hardware and G3 host checks.

**License:** NVIDIA proprietary — see [`THIRD_PARTY.md`](../THIRD_PARTY.md).

## Editing

- Add static files here (configs, samples, extra firmware paths).
- Rebuild userspace with `cargo xtask build` or run `cargo build --release`
  in `user/` and copy binaries into `bin/` manually.
- After changes, run `cargo xtask mkfs` or any xtask command that rebuilds
  disks so QEMU picks up the new image.

Further build and SSH details: [`MANUAL-USUARIO.md`](../MANUAL-USUARIO.md) and
the `soso-dev` skill in `.claude/skills/soso-dev/`.
