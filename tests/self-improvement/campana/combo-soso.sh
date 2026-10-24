#!/bin/bash
# A4: la pasada A del combo (Qwen3.6-35B-A3B sobre el banco entero) con el servidor de **soso** en vez de llama.cpp.
# Uso: combo-soso.sh NOMBRE [TAREAS...]   → ejecucion-NOMBRE-a-soso-moe. Misma configuración que combo-moe-denso.sh (arnés v43 por defecto).
R0=/home/jmdiez/Trabajo/demonodojo/soso
C=$R0/tests/self-improvement/campana; W=$R0/target/gemma-work; Mw=$R0/target/mellum-work
N=${1:-soso}; shift
A=$C/ejecucion-$N-a-soso-moe
export ROUNDS=1 MAX_TURNOS=40 TEMP_BASE=0.7 SOSO_IMPROVE_BIN=${SOSO_IMPROVE_BIN:-$W/target-v43/debug/soso-improve} BASE_INICIAL=${BASE_INICIAL:-$W/base0} \
  CARGO_TARGET_DIR=$Mw/campana-target CAMPANA_JSON=${CAMPANA_JSON:-$R0/tests/self-improvement/campaign-17.json}
espera() { until curl -s -m 3 http://127.0.0.1:$1/v1/models -H "Authorization: Bearer clave-host" 2>/dev/null | grep -q qwen; do sleep 5; done; }
echo "[$(date +%T)] servidor de soso: Qwen3.6-35B-A3B"
PUERTO=7460 bash $C/soso-servir.sh >$Mw/soso-servidor-a.log 2>&1 &
SP=$!
espera 7460
MODELO=qwen3.6-35b-a3b PUERTO=7460 MAX_TOKENS=8192 bash $C/correr.sh $A "$@"
kill $SP; wait $SP 2>/dev/null
echo "[$(date +%T)] fin"
