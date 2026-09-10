#!/usr/bin/env bash
# Host check: parser TLV SEC_RT de iwl_fw.c contra firmware real del rootfs.
#
# Cada banco se compila y ejecuta dos veces: normal y con AddressSanitizer +
# UndefinedBehaviorSanitizer (R1). `alignment` queda fuera porque las structs
# de protocolo son `packed` y se acceden por puntero igual que en Linux.
# SOSO_IWL_NO_SAN=1 salta la pasada con sanitizadores (bootstrap sin ASan).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/target/iwl-hostcheck"
src="$root/lxdde/ports/iwlwifi"
fwdir="$root/rootfs/lib/firmware"

mkdir -p "$out"

CFLAGS_COMMON=(-O1 -Wall -Wextra -Wno-unused-parameter -Wno-unused-function
    -Wno-incompatible-pointer-types -Wno-address-of-packed-member
    -I"$root/tools/iwl-hostcheck" -I"$out" -I"$src")
SAN_FLAGS=(-g -fsanitize=address,undefined -fno-sanitize=alignment
    -fno-omit-frame-pointer -fno-sanitize-recover=undefined)

san_enabled() {
    [[ "${SOSO_IWL_NO_SAN:-0}" != 1 ]]
}

# build <nombre> <fuente...>  → deja $out/<nombre> y, si procede, $out/<nombre>-san
build() {
    local name="$1"
    shift
    cc "${CFLAGS_COMMON[@]}" -o "$out/$name" "$@"
    if san_enabled; then
        cc "${CFLAGS_COMMON[@]}" "${SAN_FLAGS[@]}" -o "$out/$name-san" "$@"
    fi
}

# run <nombre> [args...] — ejecuta la variante normal y la sanitizada
run() {
    local name="$1"
    shift
    "$out/$name" "$@"
    if san_enabled; then
        ASAN_OPTIONS="detect_leaks=0" UBSAN_OPTIONS="print_stacktrace=1" \
            "$out/$name-san" "$@" > "$out/$name-san.log" 2>&1 || {
            echo "FALLO sanitizadores en $name:" >&2
            cat "$out/$name-san.log" >&2
            exit 1
        }
    fi
}

grep -v '^#include' "$src/iwl_fw.c" |
    grep -v '^void \*memcpy' | grep -v '^void \*memset' \
    > "$out/iwl_fw_body.inc"

build hostcheck \
    "$root/tools/iwl-hostcheck/main.c" \
    "$root/tools/iwl-hostcheck/cmd_wait_test.c" \
    "$root/tools/iwl-hostcheck/mvm_init_test.c" \
    "$src/iwl_mvm_init.c"

build capa_dqa \
    "$root/tools/iwl-hostcheck/capa_dqa_test.c" \
    "$src/iwl_mvm_up.c" \
    "$src/iwl_mvm_nvm.c"

build hcmd_wide \
    "$root/tools/iwl-hostcheck/hcmd_wide_test.c" \
    "$src/iwl_trans.c"
echo "=== iwl HCMD wide hostcheck ==="
run hcmd_wide

build scan_abi \
    "$root/tools/iwl-hostcheck/scan_abi_test.c" \
    "$src/iwl_mvm.c" \
    "$src/iwl_mvm_nvm.c"
echo "=== iwl scan ABI / MAC hostcheck ==="
run scan_abi

build hcmd_contract \
    "$root/tools/iwl-hostcheck/hcmd_contract_test.c" \
    "$src/iwl_trans.c"
echo "=== iwl HCMD contract hostcheck ==="
run hcmd_contract

build hcmd_queue \
    "$root/tools/iwl-hostcheck/hcmd_queue_test.c" \
    "$src/iwl_trans.c"
echo "=== iwl HCMD queue/recuperación hostcheck ==="
run hcmd_queue

build bss_select \
    "$root/tools/iwl-hostcheck/bss_select_test.c" \
    "$src/iwl_mvm.c" \
    "$src/iwl_mvm_nvm.c"
echo "=== iwl selección de BSS / RSN hostcheck ==="
run bss_select

build mcc_chan \
    "$root/tools/iwl-hostcheck/mcc_chan_test.c" \
    "$src/iwl_mvm.c" \
    "$src/iwl_mvm_nvm.c"
echo "=== iwl MCC / política de canales hostcheck ==="
run mcc_chan

for ucode in \
    "$fwdir/iwlwifi-cc-a0-77.ucode" \
    "$fwdir/iwlwifi-so-a0-gf-a0-89.ucode"; do
    if [[ ! -f "$ucode" ]]; then
        echo "falta $ucode" >&2
        exit 1
    fi
    echo "=== iwl_fw hostcheck: $(basename "$ucode") ==="
    run hostcheck "$ucode"
    if [[ "$(basename "$ucode")" == iwlwifi-cc-a0-77.ucode ]]; then
        echo "=== iwl DQA capa hostcheck: $(basename "$ucode") ==="
        run capa_dqa "$ucode"
    fi
done

if san_enabled; then
    echo "OK: parser SEC_RT lmac/umac presentes; ASan+UBSan sin hallazgos"
else
    echo "OK: parser SEC_RT lmac/umac presentes (sanitizadores omitidos)"
fi
