#!/bin/bash
# Sirve un modelo Qwen3.5/3.6 (formato .som convertido con convert-gguf) con el servidor del runtime de soso en el host: CPU con AVX2 + CUDA.
# Uso: soso-servir.sh [DIR_MODELO]   (PUERTO=7460 CTX=38912 MAX_SALIDA=8192 HILOS=8 SOSO_CUDA=1)
# Binario compilado con: RUSTFLAGS="-C target-feature=+avx2,+fma" CARGO_TARGET_DIR=target/runtime-avx2 cargo build --release -p soso-llm-api --features std --example serve_host
R0=/home/jmdiez/Trabajo/demonodojo/soso
DIR=${1:-$R0/target/qwen36-work/som-35b-iq3}
PUERTO=${PUERTO:-7460}
PERFIL=$(mktemp /tmp/perfil-soso-XXXXXX.json)
python3 -I - "$DIR" "$PERFIL" "${CTX:-38912}" "${MAX_SALIDA:-8192}" "${FAMILIA:-qwen35}" "${MODELO_ID:-qwen3.6-35b-a3b}" <<'P'
import hashlib, json, os, sys
d, out, ctx, mx, fam, mid = sys.argv[1:]
h = lambda f: hashlib.sha256(open(f, "rb").read()).hexdigest()
json.dump({"id": mid, "directory": d, "family": fam, "weights_sha256": h(d + "/index.som"),
           "tokenizer_sha256": h(d + "/tokenizer.som"), "template_sha256": "", "context_tokens": int(ctx),
           "max_output_tokens": int(mx), "stop_token_ids": [248046, 248044]}, open(out, "w"))
P
cd $R0
# Muestreo por defecto como el de llama.cpp con Qwen3.x (la petición del arnés sólo manda la temperatura).
# Decode residente en GPU (capas enteras en VRAM, el resto de expertos por el bus en el prefill): -25 % por petición; la primera paga ~20 s de preparación.
export SOSO_DECODE_GPU=${SOSO_DECODE_GPU:-1} SOSO_RESERVA_DECODE_MIB=${SOSO_RESERVA_DECODE_MIB:-1200}
export HILOS=${HILOS:-8} SOSO_CUDA=${SOSO_CUDA:-1} SOSO_LLM_API_KEY=clave-host SOSO_TOP_P=${SOSO_TOP_P:-0.8} SOSO_TOP_K=${SOSO_TOP_K:-20} SOSO_RECORTAR_SALIDA=1 SOSO_PERFIL=1
exec $R0/target/runtime-avx2/release/examples/serve_host --model-dir "$DIR" --profile "$PERFIL" --port $PUERTO
