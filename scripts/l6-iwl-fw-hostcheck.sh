#!/usr/bin/env bash
# Host check: parser TLV SEC_RT de iwl_fw.c contra firmware real del rootfs.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/target/iwl-hostcheck"
src="$root/lxdde/ports/iwlwifi"
fwdir="$root/rootfs/lib/firmware"

mkdir -p "$out"

grep -v '^#include' "$src/iwl_fw.c" |
    grep -v '^void \*memcpy' | grep -v '^void \*memset' \
    > "$out/iwl_fw_body.inc"

cc -O1 -Wall -Wextra -Wno-unused-parameter -Wno-unused-function \
   -Wno-incompatible-pointer-types -Wno-address-of-packed-member \
   -I"$out" -I"$src" \
   -o "$out/hostcheck" \
   "$root/tools/iwl-hostcheck/main.c" \
   "$root/tools/iwl-hostcheck/cmd_wait_test.c"

for ucode in \
    "$fwdir/iwlwifi-cc-a0-77.ucode" \
    "$fwdir/iwlwifi-so-a0-gf-a0-89.ucode"; do
    if [[ ! -f "$ucode" ]]; then
        echo "falta $ucode" >&2
        exit 1
    fi
    echo "=== iwl_fw hostcheck: $(basename "$ucode") ==="
    "$out/hostcheck" "$ucode"
done

echo "OK: parser SEC_RT lmac/umac presentes"
