#!/bin/bash
# Banco ciego 5: (1) Claude sobre las tareas que quedaron sin medir por el límite de sesión; (2) combo MoE -> denso sobre las diez.
R0=/home/jmdiez/Trabajo/demonodojo/soso
C=$R0/tests/self-improvement/campana
export CAMPANA_JSON=$R0/tests/self-improvement/campaign-17.json SOSO_IMPROVE_BIN=$R0/target/gemma-work/target-v41/debug/soso-improve
echo "[$(date +%T)] CLAUDE: R46 R47 R48 R49 R50"
SALIDA=$C/ejecucion-58-claude-banco5 MAX_LLAMADAS=400 bash $C/claude-piloto.sh R46 R47 R48 R49 R50
echo "[$(date +%T)] COMBO LOCAL sobre las diez"
bash $C/combo-moe-denso.sh 59-banco5
echo "[$(date +%T)] BANCO5 COMPLETO"
