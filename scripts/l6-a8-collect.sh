#!/usr/bin/env bash
# Recoge metadatos A8 en la matriz hw-matrix.json (host + logs ESP si hay USB).
#
# Uso:
#   ./scripts/l6-a8-collect.sh --id mi-placa --equipo "..." [--pci 10de:2f18] [--perfil live-usb]
#   ./scripts/l6-a8-collect.sh --id mi-placa --boot-ok
#   ./scripts/l6-a8-collect.sh --id mi-placa --boot-fail
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

ID=""
EQUIPO=""
PCI=""
PERFIL=""
BOOT=""
TMPDIR="${TMPDIR:-/tmp}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --id) ID="$2"; shift 2 ;;
    --equipo) EQUIPO="$2"; shift 2 ;;
    --pci) PCI="$2"; shift 2 ;;
    --perfil) PERFIL="$2"; shift 2 ;;
    --boot-ok) BOOT=ok; shift ;;
    --boot-fail) BOOT=fail; shift ;;
    *) echo "Argumento desconocido: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$ID" ]]; then
  ID="$(hostname -s 2>/dev/null || echo host)"
fi

if [[ ! -f docs/hw-matrix.json ]]; then
  cargo xtask hw-matrix init
fi

COLLECT=(cargo xtask hw-matrix collect --id "$ID")
[[ -n "$EQUIPO" ]] && COLLECT+=(--equipo "$EQUIPO")
[[ -n "$PCI" ]] && COLLECT+=(--pci "$PCI")
[[ -n "$PERFIL" ]] && COLLECT+=(--perfil "$PERFIL")
"${COLLECT[@]}"

if [[ "$BOOT" == ok ]]; then
  cargo xtask hw-matrix record-boot --id "$ID"
elif [[ "$BOOT" == fail ]]; then
  cargo xtask hw-matrix record-boot --id "$ID" --fail
fi

LOG="$TMPDIR/soso-a8-$$"
mkdir -p "$LOG"
if cargo xtask sosolog --help >/dev/null 2>&1; then
  cargo xtask sosolog >"$LOG/SOSOLOG.txt" 2>/dev/null || true
  cargo xtask sosolog --drv >"$LOG/SOSODRV.txt" 2>/dev/null || true
fi

PARSE=(cargo xtask hw-matrix parse-logs --id "$ID")
[[ -f "$LOG/SOSOLOG.txt" ]] && PARSE+=(--sosolog "$LOG/SOSOLOG.txt")
[[ -f "$LOG/SOSODRV.txt" ]] && PARSE+=(--sosodrv "$LOG/SOSODRV.txt")
if [[ ${#PARSE[@]} -gt 3 ]]; then
  "${PARSE[@]}" || true
fi

cargo xtask hw-matrix show | sed -n "/\\[$ID\\]/,/^$/p"
echo "Logs temporales: $LOG"
