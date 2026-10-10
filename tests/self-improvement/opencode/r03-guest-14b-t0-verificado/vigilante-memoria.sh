#!/bin/bash
# Seguridad del escritorio del usuario: para la unidad dada ante riesgo REAL de OOM/thrash.
# Un swap bajo por sí solo no es riesgo (puede estar lleno de páginas frías): hace falta además poca memoria disponible.
UNIT=$1; SWAP_MIN_KB=${2:-150000}; AVAIL_MIN_KB=${3:-1500000}; AVAIL_CON_SWAP_KB=${4:-4000000}
while systemctl --user is-active "$UNIT" >/dev/null 2>&1; do
  sw=$(awk '/SwapFree/{print $2}' /proc/meminfo); av=$(awk '/MemAvailable/{print $2}' /proc/meminfo)
  if [ "$av" -lt "$AVAIL_MIN_KB" ] || { [ "$sw" -lt "$SWAP_MIN_KB" ] && [ "$av" -lt "$AVAIL_CON_SWAP_KB" ]; }; then
    echo "$(date +%T) VIGILANTE: swap libre ${sw} kB, disponible ${av} kB -> paro $UNIT" >> /home/jmdiez/Trabajo/demonodojo/soso/target/self-improvement/first-improvement/vigilante-memoria.log
    systemctl --user stop "$UNIT"; exit 0
  fi
  sleep 5
done
