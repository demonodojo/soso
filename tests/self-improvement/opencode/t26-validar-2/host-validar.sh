#!/bin/bash
# T26: ciclo completo con el 14B real: capturar → preparar → copiar → ejecutar → validar. `host-validar.sh N`
R0=/home/jmdiez/Trabajo/demonodojo/soso
D=$R0/target/self-improvement/first-improvement
PORT=7441; N=$1; O=$R0/tests/self-improvement/opencode/t26-validar-$N; mkdir -p $O
B=$D/base-t26; CAP=$D/captura-t26; E=$D/estado-t26; W=$D/copia-t26; V=$D/validador-t26; REF=$D/refs-t26
rm -rf $B $CAP $E $W $V $REF; cp -r $D/arbol-r03 $B; rm -rf $B/.git $B/target $B/TASK.md
if ! ss -ltn | grep -q ":$PORT "; then echo 'servidor no esta' > $O/error.txt; exit 1; fi
cd $R0; S="cargo run -q -p soso-improve --"
date +%s > $O/inicio.txt; rm -f $O/fin.txt
$S capturar --repo $B --out $CAP --sin-herramientas > $O/capturar.txt 2>&1
python3 -I - $CAP $O/spec.json $D/arbol-r03/TASK.md <<'P'
import json,sys,hashlib
m=json.load(open(sys.argv[1]+"/baseline.json"))
a="".join(f"{x['ruta']}\0{x['sha256']}\n" for x in m["inventario"])
h=hashlib.sha256(a.encode()).hexdigest()
json.dump({"schema_version":1,"id":"R03","base":h,"problema":open(sys.argv[3]).read(),"rutas_editables":["crates/soso-http/src"],"comprobaciones":[{"argv":["cargo","test","-p","soso-http"],"cwd":".","timeout_s":900}],"limites":{"intentos_max":3,"herramientas_max":30,"segundos_max":7200},"hashes_entrada":{}},open(sys.argv[2],"w"))
P
cat > $O/exigencias.json <<'P'
{"requisitos":[{"nombre":"cargo test -p soso-http","argv":["cargo","test","-p","soso-http"],"cwd":".","timeout_s":900}],"rutas_protegidas":["tests"],"regresion":null}
P
$S tarea preparar --estado $E --spec $O/spec.json --run r$N > $O/preparar.txt 2>&1
$S tarea copiar --estado $E --run r$N --captura $CAP --destino $W > $O/copiar.txt 2>&1
SOSO_LLM_API_KEY=clave-host $S tarea ejecutar --estado $E --run r$N --copia $W --modelo qwen2.5-coder-14b --puerto $PORT --max-turnos 30 --cargo-home $HOME/.cargo > $O/ejecutar.txt 2>&1; echo $? > $O/ejecutar.exit
$S tarea validar --estado $E --run r$N --captura $CAP --copia $W --exigencias $O/exigencias.json --trabajo $V --referencias $REF --campana r03 --publicar-base > $O/validar.txt 2>&1; echo $? > $O/validar.exit
$S tarea informe --estado $E --run r$N > $O/informe.txt 2>&1
cp -r $E $O/estado; cp -r $REF $O/referencias
date +%s > $O/fin.txt
