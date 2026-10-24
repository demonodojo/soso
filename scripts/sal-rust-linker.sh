#!/usr/bin/env bash
# La std de soso declara bibliotecas C opcionales; todas deben ser estáticas.
set -euo pipefail
# Wild conserva las cabeceras de sección mapeadas que consulta la std de soso.
libc=$(cc -print-file-name=libc.a)
if [ "${1:-}" = -flavor ] && [ "${2:-}" = gnu ]; then shift 2; fi
args=()
for arg in "$@"; do
    if [ "$arg" = -Bdynamic ]; then args+=(-Bstatic); else args+=("$arg"); fi
done
exec wild -L "$(dirname "$libc")" "${args[@]}" --static
