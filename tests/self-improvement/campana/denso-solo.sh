#!/bin/bash
# Qwen3.6-27B denso sobre las tareas indicadas, con 5400 s por tarea. Uso: denso-solo.sh NOMBRE R33 R37 ...
R0=/home/jmdiez/Trabajo/demonodojo/soso
C=$R0/tests/self-improvement/campana; W=$R0/target/gemma-work; Mw=$R0/target/mellum-work
N=$1; shift
export ROUNDS=1 MAX_TURNOS=40 TEMP_BASE=0.7 SOSO_IMPROVE_BIN=$W/target-v41/debug/soso-improve BASE_INICIAL=$W/base0 \
  CARGO_TARGET_DIR=$Mw/campana-target CAMPANA_JSON=$R0/tests/self-improvement/campaign-16.json
NGL=57 CTX=38912 bash $C/qwen27-servir.sh >$Mw/denso-solo-servidor.log 2>&1 &
SP=$!
until curl -s -m 3 http://127.0.0.1:7461/health 2>/dev/null | grep -q ok; do sleep 5; done
MODELO=qwen3.6-27b PUERTO=7461 MAX_TOKENS=6144 bash $C/correr.sh $C/ejecucion-$N "$@"
kill $SP; wait $SP 2>/dev/null
echo "[$(date +%T)] TERMINADO"
