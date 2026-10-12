#!/usr/bin/env bash
# L6-H: línea base tok/s CPU (QEMU) vs --cuda-host (llama-server CUDA en host).
# Requiere cuda-proxy en PROXY_PORT (11400) y llama-server en LLAMA_PORT (8080).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROXY_PORT="${PROXY_PORT:-11400}"
LLAMA_PORT="${LLAMA_PORT:-8080}"
CUDA_HOST="${CUDA_HOST:-10.0.2.2:${PROXY_PORT}}"
MODEL="${MODEL:-tinyllama-q4km}"
MAX="${L6H_BENCH_MAX:-32}"
PROMPT="${L6H_BENCH_PROMPT:-hola}"
KEY="$ROOT/target/soso_test_key"
IMG="$ROOT/target/soso-bios.img"
SERIAL="${TMPDIR:-/tmp}/soso-l6h-bench-serial.log"
QEMU_MEM="${SOSO_QEMU_MEM:-2G}"

launch_qemu() {
    rm -f "$SERIAL"
    qemu-system-x86_64 -machine q35 -cpu max -m "$QEMU_MEM" -smp 1 \
        -drive "format=raw,file=$IMG" \
        -drive "if=none,id=nvme0,format=raw,file=$ROOT/target/soso-data.img" \
        -device "nvme,drive=nvme0,serial=soso-root" \
        -drive "if=none,id=nvme1,format=raw,file=$ROOT/target/soso-models.img" \
        -device "nvme,drive=nvme1,serial=soso-models" \
        -netdev "user,id=net0,hostfwd=tcp::2222-:22" \
        -device "virtio-net-pci,netdev=net0,mac=52:54:00:12:34:15" \
        -serial "file:$SERIAL" -display none \
        -device "isa-debug-exit,iobase=0xf4,iosize=0x04" -no-reboot \
        >/dev/null 2>&1 &
    echo $!
}

wait_sosh() {
    local pid=$1
    local deadline=$((SECONDS + 120))
    while (( SECONDS < deadline )); do
        [[ -f "$SERIAL" ]] && grep -q 'sosh —' "$SERIAL" && break
        sleep 0.3
    done
    if ! grep -q 'sosh —' "$SERIAL" 2>/dev/null; then
        kill "$pid" 2>/dev/null || true
        echo "FAIL: QEMU no arrancó (ver $SERIAL)" >&2
        exit 1
    fi
    sleep 2
}

ssh_llm() {
    local cmd=$1
    local wait="${2:-120}"
    local out_file
    out_file=$(mktemp)
    { sleep 3; echo "$cmd"; sleep "$wait"; echo exit; } | \
        timeout $((wait + 30)) ssh -tt -i "$KEY" -p 2222 \
            -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
            soso@localhost >"$out_file" 2>&1 || true
    cat "$out_file"
    rm -f "$out_file"
}

parse_tok_s() {
    grep 'soso-llm: generado' | tail -1 | sed -n 's/.*, \([0-9.]*\) tok\/s).*/\1/p'
}

echo "=== L6-H benchmark — CPU vs CUDA host ==="
echo "Modelo CUDA: $MODEL  max=$MAX  cuda-host=$CUDA_HOST"

SOSO_LXDDE=1 cargo xtask build >/dev/null

if ss -ltn 2>/dev/null | grep -q ':2222 '; then
    echo "FAIL: puerto 2222 ocupado" >&2
    exit 1
fi

echo "--- CPU (QEMU, modelo sintético tiny) ---"
pid=$(launch_qemu)
wait_sosh "$pid"
out=$(ssh_llm "soso-llm run tiny --prompt ${PROMPT} --max ${MAX}" 120)
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
cpu_tok=$(parse_tok_s <<<"$out")
if [[ -z "$cpu_tok" ]]; then
    echo "FAIL: no obtuve tok/s CPU" >&2
    echo "$out" >&2
    exit 1
fi
echo "OK: CPU ${cpu_tok} tok/s"

echo "--- CUDA host (--cuda-host → cuda-proxy) ---"
cuda_tok=""
if curl -sf "http://127.0.0.1:${LLAMA_PORT}/health" >/dev/null 2>&1; then
    echo "OK: llama-server :${LLAMA_PORT}"
else
    echo "WARN: llama-server no responde en :${LLAMA_PORT}"
fi

if curl -sf "http://127.0.0.1:${PROXY_PORT}/" >/dev/null 2>&1; then
    pid=$(launch_qemu)
    wait_sosh "$pid"
    out=$(ssh_llm "soso-llm run ${MODEL} --cuda-host ${CUDA_HOST} --prompt ${PROMPT} --max ${MAX}" 180)
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    cuda_tok=$(parse_tok_s <<<"$out")
    if [[ -n "$cuda_tok" ]]; then
        echo "OK: CUDA host ${cuda_tok} tok/s"
    else
        echo "WARN: cuda-proxy activo pero inferencia falló"
    fi
else
    echo "SKIP: cuda-proxy no en :${PROXY_PORT} — ./scripts/l6-h-start-cuda.sh"
fi

echo ""
echo "=== L6-H resumen (referencia G5) ==="
printf "  %-20s %s tok/s\n" "CPU (QEMU tiny)" "$cpu_tok"
if [[ -n "$cuda_tok" ]]; then
    printf "  %-20s %s tok/s\n" "CUDA host" "$cuda_tok"
    ratio=$(python3 - "$cuda_tok" "$cpu_tok" <<'PY'
import sys
c, u = map(float, sys.argv[1:3])
print(f"{c/u:.2f}" if u > 0 else "n/a")
PY
)
    echo "  speedup CUDA/CPU:    ×${ratio}"
fi
