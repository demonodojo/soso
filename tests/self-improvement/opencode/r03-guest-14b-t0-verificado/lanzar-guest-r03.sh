#!/bin/bash
# Lanza guest-r03.sh con reintentos: el arranque del servicio guest falla de forma intermitente (~2 de cada 3).
cd /home/jmdiez/Trabajo/demonodojo/soso
D=target/self-improvement/first-improvement
for intento in 1 2 3 4 5 6; do
  if (ss -ltn | grep -q ':2222 '); then echo "2222 ocupado (QEMU del usuario): aborto" > $D/lanzar-guest-r03.fin; exit 1; fi
  SOSO_QEMU_MEM=${MEM:-8G} SOSO_QEMU_SMP=4 cargo xtask test-llm-api --model-dir ${MODELDIR:-target/qwen2.5-coder-7b-model} --profile ${PERFIL:-target/self-improvement/tasks/T14/model-lock-7b.json} --orden $PWD/$D/guest-r03.sh > $D/xtask-guest-r03.txt 2>&1 < /dev/null
  grep -q "contra el endpoint vivo" $D/xtask-guest-r03.txt && break
  cp $D/xtask-guest-r03.txt $D/xtask-guest-r03-reintento$intento.txt; sleep 10
done
echo FIN > $D/lanzar-guest-r03.fin
