#!/bin/bash
# T35: agente nativo (host) contra la inferencia del guest, R03. Lo lanza `cargo xtask test-llm-api --orden`.
set -u
R0=/home/jmdiez/Trabajo/demonodojo/soso
D=$R0/target/self-improvement/first-improvement
O=$R0/tests/self-improvement/opencode/t35-agente-guest; W=$D/trabajo-agente-guest; mkdir -p $O
rm -rf $W; cp -r $D/arbol-r03 $W
date +%s > $O/inicio.txt; rm -f $O/fin.txt
cd $R0
cargo run -q -p soso-improve -- agente --repo $W --tarea "Resuelve la tarea descrita en TASK.md y ejecuta sus comprobaciones." \
  --modelo $SOSO_LLM_MODEL --puerto $SOSO_LLM_PORT --token $SOSO_LLM_API_KEY --sesion $O/sesion.json --max-turnos 30 --cargo-home $HOME/.cargo > $O/agente.txt 2>&1
echo $? > $O/agente.exit
git -C $W diff -- crates > $O/parche.diff
cargo run -q -p soso-improve -- verificar repo --caso R03 --arbol $W > $O/verificador.txt 2>&1
echo $? > $O/verificador.exit
date +%s > $O/fin.txt
exit 0
