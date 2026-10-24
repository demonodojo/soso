#!/usr/bin/env bash
# Vigía desde el host: ping + SSH cada 5 s (cruzar con SOSOLOG en placa).
# Uso: ./scripts/l6-rog-red-watch.sh 192.168.68.132 [directorio_salida]
set -euo pipefail

ip="${1:?IP del ROG}"
out="${2:-$(cd "$(dirname "$0")/.." && pwd)/target/rog-red-$(date +%Y-%m-%d)}"
mkdir -p "$out"
log="$out/vigia-host.log"

echo "Vigía ROG → $ip (log $log). Ctrl-C para parar."
while true; do
  ts="$(date -Iseconds)"
  ping_ok=0
  ssh_ok=0
  if ping -c1 -W1 "$ip" >/dev/null 2>&1; then ping_ok=1; fi
  if ssh -o BatchMode=yes -o ConnectTimeout=3 -o StrictHostKeyChecking=no \
      "root@$ip" true 2>/dev/null; then ssh_ok=1; fi
  printf '%s ping=%s ssh=%s\n' "$ts" "$ping_ok" "$ssh_ok" | tee -a "$log"
  sleep 5
done
