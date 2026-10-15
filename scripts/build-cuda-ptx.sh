#!/usr/bin/env bash
# G9: compila a PTX los kernels CUDA que usa el backend de GPU del host (`soso-llm-api/src/cuda.rs`).
# No hace falta GPU ni nvcc en el host: usa la imagen de Docker. PTX (y no SASS) para que el driver lo
# compile para la tarjeta que haya (sm_86, sm_120, …); el SASS por arquitectura es para el camino nativo.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IMAGE="${SOSO_CUDA_DOCKER:-nvidia/cuda:12.8.0-devel-ubuntu24.04}"
OUT="crates/soso-llm-api/cuda"
KERNELS=(matvec_q4_0 gemma_capa)
cmds=""
for k in "${KERNELS[@]}"; do
  cmds+="nvcc -O2 -arch=compute_80 -ptx lxdde/ports/nouveau/$k.cu -o $OUT/$k.ptx && "
done
cmds+="true"
cd "$ROOT"
if command -v nvcc >/dev/null 2>&1; then
  bash -c "$cmds"
else
  docker run --rm --user "$(id -u):$(id -g)" -v "$ROOT:/w" -w /w "$IMAGE" bash -c "$cmds"
fi
ls -la $OUT
