#!/bin/bash
# Sirve Qwen3.6-35B-A3B (GGUF) con llama.cpp (CUDA): atención y expertos de las capas altas en la GPU, expertos de las primeras N capas en RAM.
# Sin razonamiento (modo instruct) y muestreo recomendado por Qwen para ese modo. Uso: qwen36-servir.sh [GGUF] ; NCMOE=14 PUERTO=7460
R0=/home/jmdiez/Trabajo/demonodojo/soso
export LD_LIBRARY_PATH=$R0/target/lfm-work/out/bin:${LD_LIBRARY_PATH:-}
GGUF=${1:-$R0/target/qwen36-work/Qwen3.6-35B-A3B-UD-IQ3_XXS.gguf}
# RAZONAR=1: razonamiento activado (modo thinking), con el muestreo que Qwen recomienda para código preciso y un tope de tokens de razonamiento
# por turno (REASONING_BUDGET, 4096 por defecto). Sin RAZONAR: modo instruct, sin razonamiento.
if [ "${RAZONAR:-0}" = 1 ]; then
  MODO=(--chat-template-kwargs '{"enable_thinking":true}' --reasoning-budget ${REASONING_BUDGET:-4096} --temp 0.6 --top-k 20 --top-p 0.95 --min-p 0 --presence-penalty 0)
else
  MODO=(--chat-template-kwargs '{"enable_thinking":false}' --temp 0.7 --top-k 20 --top-p 0.8 --min-p 0 --presence-penalty 0)
fi
exec $R0/target/lfm-work/out/bin/llama-server -m "$GGUF" --alias qwen3.6-35b-a3b --host 127.0.0.1 --port ${PUERTO:-7460} \
  -ngl 99 --n-cpu-moe ${NCMOE:-14} -c ${CTX:-65536} -fa on -np 1 --jinja "${MODO[@]}"
