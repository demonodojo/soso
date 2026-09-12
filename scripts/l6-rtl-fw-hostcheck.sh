#!/usr/bin/env bash
# Host check: parser rtl8168h-2.fw (magic/chksum/fw_start) contra rootfs.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/target/rtl-hostcheck"
fw="$root/rootfs/lib/firmware/rtl_nic/rtl8168h-2.fw"

mkdir -p "$out"

if [[ ! -f "$fw" ]]; then
  echo "falta $fw — ./scripts/l6-pack-rtl-fw.sh" >&2
  exit 1
fi

cc -O1 -Wall -Wextra -o "$out/phy_fw_test" "$root/tools/rtl-hostcheck/phy_fw_test.c"
echo "=== rtl PHY firmware hostcheck ==="
"$out/phy_fw_test" "$fw"

src="$root/kernel/src/drivers/rtl8169.rs"
for needle in 0x808a 0x0811 0x0a42 0x0bcf 0x0bcd; do
  if ! grep -q "$needle" "$src"; then
    echo "FAIL: $src no tiene $needle (cola Linux rtl8168h_2_hw_phy_config)" >&2
    exit 1
  fi
done
if grep -q '0x9600' "$src"; then
  echo "FAIL: tabla EPHY de 6 regs todavía en $src" >&2
  exit 1
fi
if grep -q 'if load_phy_firmware(nic)' "$src"; then
  echo "FAIL: return temprano tras firmware PHY" >&2
  exit 1
fi
echo "OK: cola rtl8168h_2_hw_phy_config en rtl8169.rs (sin fallback EPHY)"
