#!/bin/bash
# T35: agente nativo contra el 14B del host. `host-agente.sh N [CASO] [ARBOL]`
R0=/home/jmdiez/Trabajo/demonodojo/soso
D=$R0/target/self-improvement/first-improvement
TA=/tmp/claude-1000/-home-jmdiez-Trabajo-demonodojo-soso/8c25af8c-6d6f-44c0-b5ac-2ae4db78fb05/scratchpad/target-avx
PORT=7441; N=$1; CASO=${2:-R03}; ARB=${3:-arbol-r03}
W=$D/trabajo-agente; O=$R0/tests/self-improvement/opencode/t35-agente-$N; mkdir -p $O
if [ ! -f $D/host-srv.pid ] || ! kill -0 $(cat $D/host-srv.pid) 2>/dev/null; then
  cd $R0
  HILOS=${HILOS:-4} SOSO_LLM_API_KEY=clave-host setsid nohup $TA/release/examples/serve_host --model-dir ${MODELDIR:-target/qwen2.5-coder-14b-model} --profile $D/${PERFIL:-perfil-14b.json} --port $PORT > $D/host-srv.log 2>&1 < /dev/null &
  echo $! > $D/host-srv.pid
  for i in $(seq 1 120); do curl -s -o /dev/null http://127.0.0.1:$PORT/v1/models -H "Authorization: Bearer clave-host" && break; sleep 2; done
fi
rm -rf $W; cp -r $D/$ARB $W
date +%s > $O/inicio.txt; rm -f $O/fin.txt
cd $R0
SOSO_LLM_API_KEY=clave-host cargo run -q -p soso-improve -- agente --repo $W --tarea "Resuelve la tarea descrita en TASK.md y ejecuta sus comprobaciones." \
  --modelo qwen2.5-coder-14b --puerto $PORT --sesion $O/sesion.json --max-turnos 30 --cargo-home $HOME/.cargo > $O/agente.txt 2>&1
echo $? > $O/agente.exit
git -C $W diff -- crates > $O/parche.diff
cargo run -q -p soso-improve -- verificar repo --caso $CASO --arbol $W > $O/verificador.txt 2>&1
echo $? > $O/verificador.exit
date +%s > $O/fin.txt
