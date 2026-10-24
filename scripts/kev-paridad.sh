#!/usr/bin/env bash
# Arranca serve_host con un modelo .som Kev y ejecuta el test de paridad opcional.
#   KEV_MODEL_DIR=target/kev-4b-model KEV_REF_URL=http://127.0.0.1:8080 ./scripts/kev-paridad.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODEL_DIR="${KEV_MODEL_DIR:-$ROOT/target/kev-4b-model}"
REF="${KEV_REF_URL:?define KEV_REF_URL}"
PORT="${KEV_PORT:-8009}"
export SOSO_LLM_API_KEY="${SOSO_LLM_API_KEY:-dev-secret}"
PERFIL="$(mktemp)"
W=$(sha256sum "$MODEL_DIR/index.som" | awk '{print $1}')
T=$(sha256sum "$MODEL_DIR/tokenizer.som" | awk '{print $1}')
cat >"$PERFIL" <<EOF
{"id":"kev","directory":"$MODEL_DIR","family":"qwen35","weights_sha256":"$W","tokenizer_sha256":"$T","template_sha256":"","context_tokens":8192,"max_output_tokens":0,"stop_token_ids":[]}
EOF
cargo build -q -p soso-llm-api --features std --example serve_host
"$ROOT/target/debug/examples/serve_host" --model-dir "$MODEL_DIR" --profile "$PERFIL" --port "$PORT" &
PID=$!
trap 'kill $PID 2>/dev/null || true' EXIT
for _ in $(seq 1 30); do
  if curl -sf "http://127.0.0.1:$PORT/" >/dev/null 2>&1; then break; fi
  sleep 1
done
export KEV_REF_URL="$REF"
cargo test -p soso-llm-api --features std systemone_paridad_opcional_kev_ref -- --nocapture
