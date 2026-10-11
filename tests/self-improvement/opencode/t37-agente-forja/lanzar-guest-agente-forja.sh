#!/bin/bash
# Lanza guest-agente-forja.sh con reintentos: el arranque del servicio guest falla de forma intermitente (~2 de cada 3).
cd /home/jmdiez/Trabajo/demonodojo/soso
D=target/self-improvement/first-improvement
for intento in 1 2 3 4 5 6; do
  if (ss -ltn | grep -q ':2222 '); then echo "2222 ocupado (QEMU del usuario): aborto" > $D/lanzar-guest-agente-forja.fin; exit 1; fi
  SOSO_QEMU_MEM=${MEM:-14G} SOSO_QEMU_SMP=4 cargo xtask test-llm-api --model-dir ${MODELDIR:-target/qwen2.5-coder-14b-model} --profile ${PERFIL:-target/self-improvement/tasks/T14/model-lock-14b.json} --agente-forja > $D/xtask-guest-agente-forja.txt 2>&1 < /dev/null
  grep -q "contra el endpoint vivo" $D/xtask-guest-agente-forja.txt && break
  cp $D/xtask-guest-agente-forja.txt $D/xtask-guest-agente-forja-reintento$intento.txt; sleep 10
done
echo FIN > $D/lanzar-guest-agente-forja.fin
