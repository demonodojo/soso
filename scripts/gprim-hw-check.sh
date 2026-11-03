#!/usr/bin/env bash
# Comprobación de GPRIM en una máquina con GPU NVIDIA (ver docs/GPRIM-PRUEBA-HW.md).
# Paso 1 (host, libcuda): el kernel gprim.cu contra el oráculo Sal.
# Paso 2: regenera los blobs SASS y avisa si difieren de los versionados.
set -euo pipefail
cd "$(dirname "$0")/.."
echo "== 1/2 kernel gprim contra el oráculo Sal =="
python3 tools/gpu-prim/validar.py
echo "== 2/2 SASS regenerado =="
python3 tools/gpu-prim/cubin.py
git status --short lxdde/ports/nouveau/sass_gprim_sm*.c lxdde/ports/nouveau/sass_gprim_sm*.json
echo "OK: si git no lista cambios, los blobs versionados están al día."
echo "Siguiente (ver docs/GPRIM-PRUEBA-HW.md): en el guest, con el escritorio parado, ejecuta gprim-prueba."
echo "Antes, arranca soso con lxdde y busca en el log de serie:"
echo "  nouveau-lx: compute — gprim (escena 2D) disponible"
