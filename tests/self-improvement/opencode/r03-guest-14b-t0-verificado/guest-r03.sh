#!/bin/bash
# T22 (guest): OpenCode real contra la inferencia del guest, R03 sin pistas de edición, con rondas y verificador.
# Lo lanza `cargo xtask test-llm-api --orden`; recibe SOSO_LLM_API_KEY, SOSO_LLM_PORT y SOSO_LLM_MODEL.
set -u
R0=/home/jmdiez/Trabajo/demonodojo/soso
D=$R0/target/self-improvement/first-improvement
O=$D/intento-guest; W=$D/trabajo-guest; mkdir -p $O; RONDAS=${RONDAS:-4}
rm -rf $W; cp -r $D/arbol-r03-lineas $W
python3 -I - "$R0/opencode.json" "$W/opencode.json" "$SOSO_LLM_PORT" "$SOSO_LLM_MODEL" <<'P'
import json,sys
c=json.load(open(sys.argv[1])); pr=c["provider"]["soso"]
pr["options"]["baseURL"]=f"http://127.0.0.1:{sys.argv[3]}/v1"
pr["models"]["soso-coder"]["id"]=sys.argv[4]
pr["options"]["timeout"]=18000000; pr["options"]["chunkTimeout"]=18000000
json.dump(c,open(sys.argv[2],"w"),indent=2,ensure_ascii=False)
P
mkdir -p $W/.opencode/agents $W/.opencode/tools; cp $R0/.opencode/agents/soso-improve.md $W/.opencode/agents/; cp $R0/.opencode/tools/*.ts $W/.opencode/tools/
K=$R0/target/self-improvement/tasks/T21-cache
EMPUJON="Continúa: haz ahora la siguiente llamada a una herramienta, sin explicar nada. Cuando el cambio esté hecho, ejecuta cargo test -p soso-http y resume el resultado."
date +%s > $O/inicio.txt; rm -f $O/fin.txt $O/rondas.txt
cd $W
for r in $(seq 1 $RONDAS); do
  if [ $r = 1 ]; then ARGS=(); MSG="Resuelve la tarea descrita en TASK.md y ejecuta sus comprobaciones."; else ARGS=(-c); MSG="$EMPUJON"; fi
  echo "$MSG" > $O/mensaje-ronda$r.txt
  env -i PATH=$PATH HOME=$K/home XDG_CONFIG_HOME=$K/cfg XDG_DATA_HOME=$K/data XDG_CACHE_HOME=$K/cache XDG_STATE_HOME=$K/state \
    CARGO_HOME=${CARGO_HOME:-$HOME/.cargo} RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup} SOSO_LLM_API_KEY=$SOSO_LLM_API_KEY \
    timeout ${T22_PLAZO:-14400} opencode run "${ARGS[@]}" --print-logs --model soso/soso-coder --format json "$MSG" \
    > $O/transcript-ronda$r.jsonl 2> $O/opencode-ronda$r.log < /dev/null
  echo "ronda $r: opencode rc=$?" >> $O/rondas.txt
  git -C $W diff -- crates > $O/parche-ronda$r.diff
  (cd $R0 && cargo run -q -p soso-improve -- verificar repo --caso R03 --arbol $W > $O/verificador-ronda$r.txt 2>&1); V=$?
  echo "ronda $r: verificador exit=$V" >> $O/rondas.txt
  [ $V = 0 ] && break
done
cat $O/transcript-ronda*.jsonl > $O/transcript.jsonl 2>/dev/null
git -C $W diff -- crates > $O/parche.diff
echo "guest: $r rondas; verificador exit=$V" > $O/exit.txt
date +%s > $O/fin.txt
exit 0
