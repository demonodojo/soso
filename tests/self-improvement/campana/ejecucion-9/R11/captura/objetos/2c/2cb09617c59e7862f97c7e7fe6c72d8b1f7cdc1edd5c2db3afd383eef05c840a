#!/usr/bin/env bash
# G5: verifica matvec end-to-end (fallback CPU on_gpu=0) y mide overhead base.
# Sin VFIO: valida tests host + soso-llm CPU en QEMU.
# Con VFIO: SOSO_QEMU_GPU=vfio:BB:DD.F ejerce syscalls GPU (on_gpu=0 hasta G4 GO).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SERIAL="${TMPDIR:-/tmp}/soso-g5-matvec-serial.log"
KEY="$ROOT/target/soso_test_key"
IMG="$ROOT/target/soso-bios.img"
MAX="${SOSO_G5_MAX:-8}"
TIMEOUT="${SOSO_G5_TIMEOUT:-180}"
QEMU_MEM="${SOSO_QEMU_MEM:-2G}"
QEMU_SMP="${SOSO_QEMU_SMP:-1}"

echo "=== L6 G5 — matvec smoke ==="

echo "--- host: matvec soso-llm-core ---"
t0=$(date +%s.%N)
cargo test -q -p soso-llm-core --lib quant:: 2>&1
t1=$(date +%s.%N)
host_test_s=$(python3 - "$t0" "$t1" <<'PY'
import sys
print(f"{float(sys.argv[2]) - float(sys.argv[1]):.3f}")
PY
)
echo "OK: cargo test matvec (${host_test_s}s)"

echo "--- host: GSP hostcheck (G4f QMD) ---"
./scripts/l6-g3-gsp-hostcheck.sh >/dev/null
echo "OK: hostcheck"

echo "--- build LXDDE nouveau ---"
SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask build >/dev/null
echo "OK: cargo xtask build"

if ss -ltn 2>/dev/null | grep -q ':2222 '; then
    echo "FAIL: puerto 2222 ocupado — cierra QEMU previo" >&2
    exit 1
fi

launch_qemu() {
    local data="$ROOT/target/soso-data.img"
    local models="$ROOT/target/soso-models.img"
    rm -f "$SERIAL"
    qemu-system-x86_64 -machine q35 -cpu max -m "$QEMU_MEM" -smp "$QEMU_SMP" \
        -drive "format=raw,file=$IMG" \
        -drive "if=none,id=nvme0,format=raw,file=$data" \
        -device "nvme,drive=nvme0,serial=soso-root" \
        -drive "if=none,id=nvme1,format=raw,file=$models" \
        -device "nvme,drive=nvme1,serial=soso-models" \
        -netdev "user,id=net0,hostfwd=tcp::2222-:22,hostfwd=tcp::7777-:7" \
        -device "virtio-net-pci,netdev=net0,mac=52:54:00:12:34:15" \
        ${SOSO_QEMU_GPU:+-device "vfio-pci,host=${SOSO_QEMU_GPU#vfio:}"} \
        -serial "file:$SERIAL" -display none \
        -device "isa-debug-exit,iobase=0xf4,iosize=0x04" -no-reboot \
        >/dev/null 2>&1 &
    echo $!
}

echo "--- QEMU: arranque ---"
QEMU_PID=$(launch_qemu)
cleanup() {
    kill "$QEMU_PID" 2>/dev/null || true
    wait "$QEMU_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

deadline=$((SECONDS + 120))
while (( SECONDS < deadline )); do
    if [[ -f "$SERIAL" ]] && grep -q 'sosh —' "$SERIAL" 2>/dev/null; then
        break
    fi
    sleep 0.3
done
if ! grep -q 'sosh —' "$SERIAL" 2>/dev/null; then
    echo "FAIL: QEMU no llegó a sosh (ver $SERIAL)" >&2
    tail -30 "$SERIAL" >&2 || true
    exit 1
fi
echo "OK: sosh lista"
sleep 2

echo "--- QEMU: soso-llm tiny (backend CPU o syscalls on_gpu=0) ---"
out_file=$(mktemp)
llm_wait=$(( MAX * 20 + 30 ))
{ sleep 3; echo "soso-llm run tiny --prompt g5smoke --max ${MAX}"; sleep "$llm_wait"; echo exit; } | \
    timeout $((llm_wait + 45)) ssh -tt -i "$KEY" -p 2222 \
        -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
        -o ConnectTimeout=10 soso@localhost >"$out_file" 2>&1 || true
out=$(cat "$out_file")
rm -f "$out_file"
if ! grep -q 'soso-llm: generado' <<<"$out"; then
    echo "FAIL: soso-llm no generó tokens" >&2
    echo "$out" >&2
    exit 1
fi

tok_s=$(grep 'soso-llm: generado' <<<"$out" | tail -1 | sed -n 's/.*, \([0-9.]*\) tok\/s).*/\1/p')
if [[ -z "$tok_s" ]]; then
    echo "FAIL: no parseo tok/s" >&2
    exit 1
fi
echo "OK: soso-llm generó (${tok_s} tok/s, max=${MAX})"

if grep -q 'soso-llm: GPU detectada' <<<"$out"; then
    echo "OK: SysGpu activo (matvec vía syscall; on_gpu=0 hasta GO G4/G5 HW)"
else
    echo "OK: backend CPU (sin GPU en PCI — syscalls GPU no aplican en QEMU sin VFIO)"
fi

if grep -q 'gpu: sin GPU' "$SERIAL" 2>/dev/null; then
    echo "INFO: serial confirma sin NVIDIA en PCI (esperado sin SOSO_QEMU_GPU)"
fi
if grep -q 'nvidia-compute:' "$SERIAL" 2>/dev/null; then
    echo "OK: serial nvidia-compute presente"
fi

echo ""
echo "=== G5 resumen ==="
echo "  host matvec tests:     ${host_test_s}s"
echo "  QEMU soso-llm decode:  ${tok_s} tok/s (max=${MAX})"
echo "  syscall roundtrip:     requiere SOSO_QEMU_GPU=vfio:BB:DD.F (on_gpu=0 honesto)"
echo ""

trap - EXIT INT TERM
cleanup
