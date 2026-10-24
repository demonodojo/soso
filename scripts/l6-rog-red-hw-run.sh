#!/usr/bin/env bash
# Checklist de la tanda en placa (ROG). No sustituye flashear: lo hace el operador.
# Recoge SOSOLOG por SSH si responde.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/target/rog-red-$(date +%Y-%m-%d)"
ip="${ROG_IP:-192.168.68.132}"
mkdir -p "$out"

cat >"$out/README.txt" <<EOF
Tanda red ROG $(date -Iseconds)
IP esperada: $ip

Escenarios (manual en sosh):
  1. 20 min reposo
  2. soso-update aplicar
  3. ask largo CPU
  4. ask GPU
  5. >1 h conectado (rekey GTK)
  6. reiniciar AP (WiFi caída / DHCP)

Host: ./scripts/l6-rog-red-watch.sh $ip $out

Copiar logs:
  udisksctl mount -b /dev/sdX1  → SOSOLOG.TXT
  o: scp root@$ip:/var/log/kernel.log $out/kernel.log
EOF

if ssh -o BatchMode=yes -o ConnectTimeout=5 -o StrictHostKeyChecking=no \
    "root@$ip" "cat /var/log/kernel.log" >"$out/kernel.log" 2>/dev/null; then
  echo "OK: kernel.log en $out"
else
  echo "ROG no accesible por SSH en $ip; sigue README en $out"
fi
