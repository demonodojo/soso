#!/bin/bash
# Sirve Qwen3.6-27B (DENSO, GGUF) con llama.cpp (CUDA). Con IQ3_XXS (12 GB) no cabe entero en 12 GB de VRAM junto al KV: NGL capas en GPU y el resto en CPU.
# Sin razonamiento (modo instruct) y muestreo recomendado por Qwen para ese modo. Uso: qwen27-servir.sh [GGUF] ; NGL=57 CTX=40960 PUERTO=7461
R0=/home/jmdiez/Trabajo/demonodojo/soso
export LD_LIBRARY_PATH=$R0/target/lfm-work/out/bin:${LD_LIBRARY_PATH:-}
GGUF=${1:-$R0/target/qwen36-work/Qwen3.6-27B-UD-IQ3_XXS.gguf}
if [ "${RAZONAR:-0}" = 1 ]; then
  MODO=(--chat-template-kwargs '{"enable_thinking":true}' --reasoning-budget ${REASONING_BUDGET:-4096} --temp 0.6 --top-k 20 --top-p 0.95 --min-p 0 --presence-penalty 0)
else
  MODO=(--chat-template-kwargs '{"enable_thinking":false}' --temp 0.7 --top-k 20 --top-p 0.8 --min-p 0 --presence-penalty 0)
fi
exec $R0/target/lfm-work/out/bin/llama-server -m "$GGUF" --alias qwen3.6-27b --host 127.0.0.1 --port ${PUERTO:-7461} \
  -ngl ${NGL:-57} -c ${CTX:-40960} -fa on -ctk q8_0 -ctv q8_0 -np 1 --jinja "${MODO[@]}"
