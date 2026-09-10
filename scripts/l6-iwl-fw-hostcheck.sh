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
   -I"$root/tools/iwl-hostcheck" -I"$out" -I"$src" \
   -o "$out/hostcheck" \
   "$root/tools/iwl-hostcheck/main.c" \
   "$root/tools/iwl-hostcheck/cmd_wait_test.c" \
   "$root/tools/iwl-hostcheck/mvm_init_test.c" \
   "$src/iwl_mvm_init.c"

cc -O1 -Wall -Wextra -Wno-unused-parameter -Wno-unused-function \
   -Wno-incompatible-pointer-types -Wno-address-of-packed-member \
   -I"$root/tools/iwl-hostcheck" -I"$out" -I"$src" \
   -o "$out/hcmd_wide" \
   "$root/tools/iwl-hostcheck/hcmd_wide_test.c" \
   "$src/iwl_trans.c"
echo "=== iwl HCMD wide hostcheck ==="
"$out/hcmd_wide"

cc -O1 -Wall -Wextra -Wno-unused-parameter -Wno-unused-function \
   -Wno-incompatible-pointer-types -Wno-address-of-packed-member \
   -I"$root/tools/iwl-hostcheck" -I"$out" -I"$src" \
   -o "$out/scan_abi" \
   "$root/tools/iwl-hostcheck/scan_abi_test.c" \
   "$src/iwl_mvm.c" \
   "$src/iwl_mvm_nvm.c"
echo "=== iwl scan ABI / MAC hostcheck ==="
"$out/scan_abi"

cc -O1 -Wall -Wextra -Wno-unused-parameter -Wno-unused-function \
   -Wno-incompatible-pointer-types -Wno-address-of-packed-member \
   -I"$root/tools/iwl-hostcheck" -I"$out" -I"$src" \
   -o "$out/hcmd_contract" \
   "$root/tools/iwl-hostcheck/hcmd_contract_test.c" \
   "$src/iwl_trans.c"
echo "=== iwl HCMD contract hostcheck ==="
"$out/hcmd_contract"

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
