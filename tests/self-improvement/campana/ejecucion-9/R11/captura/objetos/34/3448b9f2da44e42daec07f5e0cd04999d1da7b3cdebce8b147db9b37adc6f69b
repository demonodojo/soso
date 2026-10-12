#!/usr/bin/env bash
# L6-H: arranca llama-server (CUDA) + cuda-proxy para soso.
# Uso: ./scripts/l6-h-start-cuda.sh /path/to/model.gguf
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODEL="${1:-}"
LLAMA_PORT="${LLAMA_PORT:-8080}"
PROXY_PORT="${PROXY_PORT:-11400}"
NGL="${NGL:-99}"
LLAMA_BIN="${LLAMA_BIN:-llama-server}"

if [[ -z "$MODEL" || ! -f "$MODEL" ]]; then
  echo "uso: $0 /path/to/model.gguf" >&2
  echo "  LLAMA_BIN=$LLAMA_BIN  LLAMA_PORT=$LLAMA_PORT  PROXY_PORT=$PROXY_PORT  NGL=$NGL" >&2
  exit 1
fi

if ! command -v "$LLAMA_BIN" >/dev/null 2>&1; then
  echo "FAIL: no encuentro $LLAMA_BIN (compila llama.cpp con CUDA)" >&2
  exit 1
fi

cargo build -p cuda-proxy --release --manifest-path "$ROOT/Cargo.toml"

PROXY_BIN="$ROOT/target/release/cuda-proxy"
LLAMA_LOG="${TMPDIR:-/tmp}/soso-llama-server.log"
PROXY_LOG="${TMPDIR:-/tmp}/soso-cuda-proxy.log"

cleanup() {
  [[ -n "${LLAMA_PID:-}" ]] && kill "$LLAMA_PID" 2>/dev/null || true
  [[ -n "${PROXY_PID:-}" ]] && kill "$PROXY_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "=== L6-H CUDA host ==="
echo "Modelo: $MODEL"
echo "llama-server :$LLAMA_PORT (-ngl $NGL)"
echo "cuda-proxy   :$PROXY_PORT → http://127.0.0.1:$LLAMA_PORT"
echo ""

"$LLAMA_BIN" \
  --host 127.0.0.1 \
  --port "$LLAMA_PORT" \
  -m "$MODEL" \
  -ngl "$NGL" \
  >"$LLAMA_LOG" 2>&1 &
LLAMA_PID=$!

for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:${LLAMA_PORT}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

if ! curl -sf "http://127.0.0.1:${LLAMA_PORT}/health" >/dev/null 2>&1; then
  echo "FAIL: llama-server no responde (ver $LLAMA_LOG)" >&2
  tail -20 "$LLAMA_LOG" >&2 || true
  exit 1
fi

"$PROXY_BIN" \
  --listen "0.0.0.0:${PROXY_PORT}" \
  --llama "http://127.0.0.1:${LLAMA_PORT}" \
  >"$PROXY_LOG" 2>&1 &
PROXY_PID=$!

echo "OK: listo. Desde soso:"
echo "  soso-llm run $(basename "${MODEL%.gguf}") --cuda-host 10.0.2.2:${PROXY_PORT} --prompt \"hola\""
echo ""
echo "Logs: $LLAMA_LOG , $PROXY_LOG"
echo "Ctrl+C para parar."

wait "$PROXY_PID"
