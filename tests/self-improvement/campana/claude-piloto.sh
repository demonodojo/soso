#!/bin/bash
# Piloto: Claude (suscripción, vía `claude -p`) como modelo del arnés de campañas. NO se lanza solo: ejecútalo a mano cuando se confirme.
# Uso:  claude-piloto.sh [TAREAS...]        (por defecto R40, la más barata)
# Consumo: el proxy anota cada llamada en DIR/consumo.jsonl. Para parar a mitad:  touch /tmp/claude-proxy.stop
# Variables: CLAUDE_MODELO (por defecto claude-sonnet-5-5), MAX_LLAMADAS (por defecto 60 = tope de invocaciones de claude), PUERTO_PROXY (7450).
set -eu
R0=/home/jmdiez/Trabajo/demonodojo/soso
C=$R0/tests/self-improvement/campana
for v in ANTHROPIC_API_KEY ANTHROPIC_AUTH_TOKEN CLAUDE_CODE_USE_BEDROCK CLAUDE_CODE_USE_VERTEX CLAUDE_CODE_USE_FOUNDRY; do
  [ -z "${!v:-}" ] || { echo "ABORTO: $v está definida; claude -p la usaría y facturaría por API. Haz unset $v."; exit 1; }
done
claude auth status 2>/dev/null | grep -q '"authMethod": "claude.ai"' || { echo "ABORTO: claude no está logueado con la suscripción (authMethod claude.ai)."; exit 1; }
TAREAS=${*:-R40}
PUERTO=${PUERTO_PROXY:-7450}
OUT=${SALIDA:-$C/ejecucion-49-claude-piloto}
mkdir -p $OUT
rm -f /tmp/claude-proxy.stop
systemctl --user reset-failed claude-proxy 2>/dev/null || true
systemd-run --user --unit=claude-proxy --working-directory=$R0 -p StandardOutput=file:$OUT/proxy.log -p StandardError=file:$OUT/proxy.log \
  python3 -I $C/claude_proxy.py --puerto $PUERTO --max-llamadas ${MAX_LLAMADAS:-60} --registro $OUT/consumo.jsonl >/dev/null
trap 'systemctl --user stop claude-proxy 2>/dev/null || true' EXIT
until curl -s -m 3 http://127.0.0.1:$PUERTO/v1/models >/dev/null 2>&1; do sleep 1; done
W=$R0/target/gemma-work
export ROUNDS=1 MAX_TURNOS=40 SOSO_IMPROVE_BIN=${SOSO_IMPROVE_BIN:-$W/target-v36/debug/soso-improve} BASE_INICIAL=$W/base0 \
  CARGO_TARGET_DIR=$R0/target/mellum-work/campana-target CAMPANA_JSON=$R0/tests/self-improvement/campaign-14.json \
  MODELO=${CLAUDE_MODELO:-claude-sonnet-5-5} PUERTO=$PUERTO MAX_TOKENS=2048
bash $C/correr.sh $OUT $TAREAS
echo "listo: registros en $OUT/registros · consumo en $OUT/consumo.jsonl"
