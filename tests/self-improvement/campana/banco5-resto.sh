#!/bin/bash
# Retoma el banco ciego 5 tras el OOM de R49: (A2) MoE sobre R43 (error de infraestructura), R49 y R50; (B) denso sobre todo lo no aceptado en A y A2;
# (C) Claude sobre R46–R50 (si el cupo de la suscripción lo permite). Arnés v42 (sólo arreglos de infraestructura) y tope de memoria de datos.
R0=/home/jmdiez/Trabajo/demonodojo/soso
C=$R0/tests/self-improvement/campana; W=$R0/target/gemma-work; Mw=$R0/target/mellum-work
export ROUNDS=1 MAX_TURNOS=40 TEMP_BASE=0.7 SOSO_IMPROVE_BIN=$W/target-v42/debug/soso-improve BASE_INICIAL=$W/base0 \
  CARGO_TARGET_DIR=$Mw/campana-target CAMPANA_JSON=$R0/tests/self-improvement/campaign-17.json
A1=$C/ejecucion-59-banco5-a-moe; A2=$C/ejecucion-60-banco5-a2-moe; B=$C/ejecucion-60-banco5-b-denso
espera() { until curl -s -m 3 http://127.0.0.1:$1/health 2>/dev/null | grep -q ok; do sleep 5; done; }
echo "[$(date +%T)] PASADA A2: Qwen3.6-35B-A3B sobre R43 R49 R50"
bash $C/qwen36-servir.sh >$Mw/banco5-resto-servidor-a.log 2>&1 &
SP=$!; espera 7460
MODELO=qwen3.6-35b-a3b PUERTO=7460 MAX_TOKENS=8192 bash $C/correr.sh $A2 R43 R49 R50
kill $SP; wait $SP 2>/dev/null
RECH=$(python3 -I - $A1 $A2 <<'P'
import json,glob,sys
estado={}
for d in sys.argv[1:]:
    for f in glob.glob(d+'/registros/R*.json'):
        r=json.load(open(f)); estado[r['id']]=r['resultado']   # la pasada posterior (A2) pisa a la anterior
print(' '.join(sorted(t for t,v in estado.items() if v!='aceptada')))
P
)
echo "[$(date +%T)] fin de A2; no aceptadas tras A y A2: ${RECH:-ninguna}"
if [ -n "$RECH" ]; then
  echo "[$(date +%T)] PASADA B: Qwen3.6-27B denso sobre: $RECH"
  NGL=57 CTX=38912 bash $C/qwen27-servir.sh >$Mw/banco5-resto-servidor-b.log 2>&1 &
  SP=$!; espera 7461
  MODELO=qwen3.6-27b PUERTO=7461 MAX_TOKENS=6144 bash $C/correr.sh $B $RECH
  kill $SP; wait $SP 2>/dev/null
fi
echo "[$(date +%T)] CLAUDE R46–R50 (si hay cupo)"
SALIDA=$C/ejecucion-61-claude-banco5-b MAX_LLAMADAS=400 bash $C/claude-piloto.sh R46 R47 R48 R49 R50
echo "[$(date +%T)] BANCO5 RESTO TERMINADO"
