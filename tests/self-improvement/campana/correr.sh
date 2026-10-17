#!/bin/bash
# T29: campaña de diez tareas, secuencial desde la base aceptada. Uso: correr.sh DIR_SALIDA [TAREAS...]
# TEMP_REINTENTO=T (por defecto 0,7): temperatura de las rondas de reintento (k>1); la ronda 1 va siempre a T=0.
# SISTEMA_AGENTE=pruebas: prompt que exige pruebas propias por caso límite (y un aviso si termina sin ninguna).
# MAX_TURNOS=N (por defecto 30): turnos del agente (con 60 herramientas por tarea hay que subirlo).
# ROUNDS=N (por defecto 1): si una tarea no se acepta, se repite hasta N rondas desde la misma base con un aviso genérico de
# rechazo en el enunciado (sin datos de la prueba reservada). El registro de la ronda k>1 se llama Rxx-rk.json.
# Requiere el servidor del modelo en 127.0.0.1:7441 (SOSO_LLM_API_KEY=clave-host) y /tmp/base-t02 (git archive 6f558f072).
set -u
R0=/home/jmdiez/Trabajo/demonodojo/soso
OUT=$1; shift
BIN=${SOSO_IMPROVE_BIN:-$R0/target/debug/soso-improve}
CAMP=${CAMPANA_JSON:-$R0/tests/self-improvement/campaign.json}
# Las tareas son las que fija el JSON de la campaña (o las que se pasen como argumentos).
TAREAS=${@:-$(python3 -I -c "import json,sys;print(' '.join(t['id'] for t in json.load(open(sys.argv[1]))['tareas']))" $CAMP)}
PORT=${PUERTO:-7441}
MODELO=${MODELO:-qwen2.5-coder-14b}
MAXTOK=${MAX_TOKENS:-2048}
mkdir -p $OUT/registros
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/tmp/campana-target}
export SOSO_LLM_API_KEY=clave-host
cd $R0
BASE=${BASE_INICIAL:-/tmp/base-t02}
date +%s > $OUT/inicio.txt
HINT_PREV=""
for T in $TAREAS; do
HINT_PREV=""
for RONDA in $(seq 1 ${ROUNDS:-1}); do
  SUF=""; [ $RONDA -gt 1 ] && SUF="-r$RONDA"
  W=$OUT/$T$SUF; rm -rf $W; mkdir -p $W
  eval "$(python3 -I - $CAMP $T <<'P'
import json,sys
c=json.load(open(sys.argv[1])); t=[x for x in c["tareas"] if x["id"]==sys.argv[2]][0]
lim=dict(c["limites_por_defecto"]); inj=t.get("inyeccion")
if inj: lim["intentos_max"]=inj["intentos_max"]
print(f'BANCO={t["banco"]}; INTENTOS={lim["intentos_max"]}; HERR={lim["herramientas_max"]}; SEGS={lim["segundos_max"]}; INJ_S={inj["a_los_segundos"] if inj else 0}')
P
)"
  CASO=$R0/$BANCO/repo/$T.json
  T0=$(date +%s)
  # spec desde el caso (el enunciado visible es el problema; la prueba reservada NO entra)
  $BIN capturar --repo $BASE --out $W/captura --sin-herramientas > $W/capturar.txt 2>&1
  python3 -I - $W/captura $CASO $R0/$BANCO $W/spec.json $W/exigencias.json $BIN $INTENTOS $HERR $SEGS $RONDA "$HINT_PREV" <<'P'
import json,sys,hashlib
cap,caso,banco,spec,exig,binp,intentos,herr,segs,ronda,prev=sys.argv[1:]
m=json.load(open(cap+"/baseline.json"))
h=hashlib.sha256("".join(f"{x['ruta']}\0{x['sha256']}\n" for x in m["inventario"]).encode()).hexdigest()
c=json.load(open(caso))
enun=open(banco+"/"+c["entrada"]["enunciado"]).read()
if int(ronda)>1:
    enun+=("\n\n---\nNota del coordinador: un intento anterior de esta misma tarea fue **rechazado por la validación independiente** ("+prev+"). "
           "La validación comprueba TODOS los comportamientos descritos arriba, no sólo el que ves fallar. Relee el enunciado punto por punto, "
           "decide qué comportamiento sigue sin cumplirse y cámbialo en el archivo permitido.\n")
json.dump({"schema_version":1,"id":c["id"],"base":h,"problema":enun,"rutas_editables":c["rutas_editables"],
 "comprobaciones":[{"argv":c["aceptacion"]["argv"],"cwd":".","timeout_s":900}],
 "limites":{"intentos_max":int(intentos),"herramientas_max":int(herr),"segundos_max":int(segs)},"hashes_entrada":{}},open(spec,"w"))
json.dump({"requisitos":[{"nombre":"aceptacion reservada de "+c["id"],"argv":[binp,"verificar","repo","--caso",c["id"],"--banco",banco,"--arbol","{arbol}"],"cwd":".","timeout_s":1500}],
 "rutas_protegidas":["tests/self-improvement"],"regresion":None},open(exig,"w"))
P
  RUN=r-$T$SUF
  $BIN tarea preparar --estado $W/estado --spec $W/spec.json --run $RUN > $W/preparar.txt 2>&1
  $BIN tarea copiar --estado $W/estado --run $RUN --captura $W/captura --destino $W/copia > $W/copiar.txt 2>&1
  ARGS="--estado $W/estado --run $RUN --captura $W/captura --copia $W/copia --exigencias $W/exigencias.json --trabajo $W/validador --referencias $OUT/refs --campana campana-$T --publicar-base --modelo $MODELO --puerto $PORT --max-tokens $MAXTOK --max-turnos ${MAX_TURNOS:-30} --cargo-home $HOME/.cargo"
  # SISTEMA_AGENTE=pruebas: el agente debe escribir pruebas propias para cada caso límite (campaña 14).
  [ "${SISTEMA_AGENTE:-}" = pruebas ] && ARGS="$ARGS --sistema pruebas"
  # Rondas de reintento: temperatura > 0 (a T=0 la misma petición da la misma respuesta). TEMP_REINTENTO=0 la deja en T=0.
  [ $RONDA -gt 1 ] && ARGS="$ARGS --temperatura ${TEMP_REINTENTO:-0.7}"
  INYECTADA=0
  if [ "$INJ_S" != "0" ]; then
    $BIN tarea reanudar --continuar $ARGS > $W/continuar-1.txt 2>&1 &
    PID=$!
    sleep $INJ_S
    if kill -0 $PID 2>/dev/null; then kill -9 $PID; wait $PID 2>/dev/null; INYECTADA=1; echo "inyección: proceso $PID muerto a los ${INJ_S}s" > $W/inyeccion.txt; else wait $PID; echo "la ejecución terminó antes de la inyección" > $W/inyeccion.txt; fi
    $BIN tarea informe --estado $W/estado --run $RUN > $W/informe-tras-kill.txt 2>&1
    $BIN tarea reanudar --continuar $ARGS > $W/continuar-2.txt 2>&1; RC=$?
    cp $W/continuar-2.txt $W/continuar.txt
  else
    $BIN tarea reanudar --continuar $ARGS > $W/continuar.txt 2>&1; RC=$?
  fi
  # Si el caso fue bloqueado por la caída con la copia ya editada, se resuelve adoptándola (queda anotado).
  if grep -q "BLOQUEADA" $W/continuar.txt && grep -q "adoptar_copia" $W/continuar.txt && [ "$INYECTADA" = 1 ]; then
    echo "bloqueo por caída; se adopta la copia" >> $W/inyeccion.txt
    $BIN tarea reanudar --continuar --adoptar-copia $ARGS > $W/continuar-adoptada.txt 2>&1; RC=$?
    cp $W/continuar-adoptada.txt $W/continuar.txt
  fi
  $BIN tarea informe --estado $W/estado --run $RUN > $W/informe.txt 2>&1
  T1=$(date +%s)
  python3 -I - $W $T $RC $((T1-T0)) $INYECTADA $INTENTOS $OUT/registros/$T$SUF.json $RONDA <<'P'
import json,sys,re
w,t,rc,seg,iny,intentos,dest,ronda=sys.argv[1:]
rc=int(rc); seg=int(seg); iny=int(iny)
cont=open(w+"/continuar.txt").read()
inf=open(w+"/informe.txt").read()
estado=re.search(r"estado (\w+)",inf)
estado=estado.group(1) if estado else "?"
herr=tok=0; hok=tok_ok=True; n=0
for m in re.finditer(r"intento (\d+) \S+\s+herramientas (\S+) tokens (\S+)",inf):
    n+=1
    if m.group(2).isdigit(): herr+=int(m.group(2))
    else: hok=False
    if m.group(3).isdigit(): tok+=int(m.group(3))
    else: tok_ok=False
nota=""
if estado=="aceptada": res,fallo="aceptada",None
elif "BLOQUEADA" in cont:
    res="bloqueada"
    nota=re.search(r"BLOQUEADA — (.*)",cont).group(1)[:200] if re.search(r"BLOQUEADA — (.*)",cont) else ""
    fallo="protocolo" if "no cambió nada" in nota else "infraestructura"
elif "Rechazado" in cont or estado=="rechazada": res,fallo="rechazada","programacion"
else:
    res="error"; nota=cont.strip().splitlines()[-1][:200] if cont.strip() else "sin salida"
    fallo="protocolo" if re.search(r"LimiteHerramientas|LimitePasos|Some\(400\)",cont) else "infraestructura"
reg={"id":t,"ronda":int(ronda),"resultado":res,"intentos":max(n,1) if res!="error" or n else 0,"segundos":seg,"nota":nota}
if fallo: reg["fallo"]=fallo
if hok and n: reg["herramientas"]=herr
if tok_ok and n and tok>0: reg["tokens"]=tok
if iny:
    ok = res in ("aceptada","rechazada","bloqueada") and ("ya hecho" in cont or "ejecutado" in cont or res=="bloqueada")
    reg["inyeccion"]={"que":"agente matado (kill -9) y coordinador reiniciado","recuperada":bool(ok)}
json.dump(reg,open(dest,"w"),indent=1)
print(json.dumps(reg))
P
  # Si se aceptó, la base avanza: el árbol del validador (base + paquete) es la nueva base.
  RES=$(python3 -I -c "import json;print(json.load(open('$OUT/registros/$T$SUF.json'))['resultado'])")
  if [ "$RES" = "aceptada" ]; then
    rm -rf $OUT/base-$T; cp -r $W/validador/arbol $OUT/base-$T; BASE=$OUT/base-$T
  fi
  rm -rf $W/validador/arbol 2>/dev/null
  # Se libera la copia de trabajo de las rondas ya juzgadas (ocupa ~25 MB por ronda y el disco va justo).
  [ -n "$SUF" ] || true
  if [ "$RES" = "aceptada" ]; then break; fi
  HINT_PREV="resultado: $RES"
  [ "$RES" = "bloqueada" ] && HINT_PREV="terminó sin modificar ningún archivo"
done
done
date +%s > $OUT/fin.txt
