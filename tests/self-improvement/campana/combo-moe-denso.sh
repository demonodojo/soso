#!/bin/bash
# Combinación: pasada A = banco entero con Qwen3.6-35B-A3B (MoE, rápido); pasada B = las tareas NO aceptadas en A, con Qwen3.6-27B denso (lento).
# Uso: combo-moe-denso.sh NOMBRE   → ejecucion-NOMBRE-a-moe y ejecucion-NOMBRE-b-denso en tests/self-improvement/campana
R0=/home/jmdiez/Trabajo/demonodojo/soso
C=$R0/tests/self-improvement/campana; W=$R0/target/gemma-work; Mw=$R0/target/mellum-work
N=${1:-combo}
A=$C/ejecucion-$N-a-moe; B=$C/ejecucion-$N-b-denso
export ROUNDS=1 MAX_TURNOS=40 TEMP_BASE=0.7 SOSO_IMPROVE_BIN=${SOSO_IMPROVE_BIN:-$W/target-v41/debug/soso-improve} BASE_INICIAL=${BASE_INICIAL:-$W/base0} \
  CARGO_TARGET_DIR=$Mw/campana-target CAMPANA_JSON=${CAMPANA_JSON:-$R0/tests/self-improvement/campaign-15.json}
espera() { until curl -s -m 3 http://127.0.0.1:$1/health 2>/dev/null | grep -q ok; do sleep 5; done; }
echo "[$(date +%T)] PASADA A: Qwen3.6-35B-A3B"
bash $C/qwen36-servir.sh >$Mw/combo-servidor-a.log 2>&1 &
SP=$!
espera 7460
MODELO=qwen3.6-35b-a3b PUERTO=7460 MAX_TOKENS=8192 bash $C/correr.sh $A
kill $SP; wait $SP 2>/dev/null
RECH=$(python3 -I - $A <<'P'
import json,glob,sys
r=[json.load(open(f)) for f in glob.glob(sys.argv[1]+'/registros/R*.json')]
print(' '.join(sorted(x['id'] for x in r if x['resultado']!='aceptada')))
P
)
echo "[$(date +%T)] fin de A; no aceptadas: ${RECH:-ninguna}"
if [ -n "$RECH" ]; then
  echo "[$(date +%T)] PASADA B: Qwen3.6-27B denso sobre: $RECH"
  NGL=57 CTX=38912 bash $C/qwen27-servir.sh >$Mw/combo-servidor-b.log 2>&1 &
  SP=$!
  espera 7461
  MODELO=qwen3.6-27b PUERTO=7461 MAX_TOKENS=6144 bash $C/correr.sh $B $RECH
  kill $SP; wait $SP 2>/dev/null
fi
echo "[$(date +%T)] COMBO TERMINADO"
