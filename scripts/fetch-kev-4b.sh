#!/usr/bin/env bash
# GGUF empaquetado con cabeza Kev (`dec.head_*`, metadatos `kev.*`).
# Referencia: https://huggingface.co/espetro/kev-4b-gguf (Q8_0 recomendado, temp 2.4061).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${1:-$ROOT/target/kev-4b-gguf}"
FILE="${KEV_GGUF:-kev-4b-q8_0.gguf}"
mkdir -p "$DEST"
if command -v huggingface-cli >/dev/null 2>&1; then
  huggingface-cli download espetro/kev-4b-gguf "$FILE" --local-dir "$DEST"
elif command -v hf >/dev/null 2>&1; then
  hf download espetro/kev-4b-gguf "$FILE" --local-dir "$DEST"
else
  echo "Instala huggingface-cli o hf para bajar espetro/kev-4b-gguf" >&2
  exit 1
fi
GGUF="$DEST/$FILE"
echo "GGUF en $GGUF"
cargo run -q -p convert-gguf -- --inspect-kev "$GGUF"
echo "Convertir con: cargo xtask convert-gguf $GGUF target/kev-4b-model --name kev-4b"
echo "Decidir en host: cargo run -p soso-llm-api --features std --example decide -- target/kev-4b-model request.json"
