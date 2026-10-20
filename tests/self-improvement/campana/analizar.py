#!/usr/bin/env python3
"""Análisis posterior a una campaña: por qué se rechaza cada tarea.

Uso:  analizar.py DIR_CAMPAÑA [--banco tests/self-improvement/campana4] [--base target/gemma-work/base0]
                  [--bin target/gemma-work/target-v21/debug/soso-improve] [--sin-verificar]

Para cada tarea no aceptada reúne, en `DIR_CAMPAÑA/ANALISIS.md`:
  - el resultado y los datos del intento (segundos, herramientas, tokens);
  - el diff de lo que el agente cambió respecto a la base (sólo los archivos que podía tocar);
  - qué pruebas RESERVADAS fallan y con qué mensaje (se ejecuta `soso-improve verificar` sobre su copia);
  - cómo fueron sus ediciones (aplicadas, rechazadas, bloqueadas) y si escribió pruebas propias;
  - una clasificación orientativa de la causa.

Las pruebas reservadas sólo se leen aquí, para analizar; nunca llegan al agente.
"""
import argparse, collections, glob, json, os, re, subprocess, sys

RAIZ = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))


def leer_json(p):
    try:
        return json.load(open(p))
    except Exception:
        return None


def archivos_tocables(task_md):
    """Rutas de la sección «Qué puedes tocar» de TASK.md."""
    try:
        t = open(task_md).read()
    except Exception:
        return []
    m = re.search(r"## Qué puedes tocar\s*(.*?)(?:\n## |\Z)", t, re.S)
    if not m:
        return []
    return re.findall(r"^- `([^`]+)`", m.group(1), re.M)


def diff(base, copia, rutas):
    sal = []
    for r in rutas:
        a, b = os.path.join(base, r), os.path.join(copia, r)
        p = subprocess.run(["diff", "-u", "--label", f"a/{r}", "--label", f"b/{r}", a, b], capture_output=True, text=True)
        if p.stdout:
            sal.append(p.stdout)
    return "".join(sal)


def verificar(binario, banco, caso, copia):
    env = dict(os.environ)
    # Directorio de compilación PROPIO: compartir el de la campaña (que sigue compilando a la vez) daba resultados contaminados
    # (R36 de la campaña 39 salía con dos pruebas fallando en lugar de una).
    env["CARGO_TARGET_DIR"] = os.path.join(RAIZ, "target/analisis-target")
    try:
        p = subprocess.run([binario, "verificar", "repo", "--caso", caso, "--banco", banco, "--arbol", copia], capture_output=True, text=True, timeout=1500, env=env)
    except subprocess.TimeoutExpired:
        return None, "timeout"
    t = p.stdout + p.stderr
    fallan = re.findall(r"^(\S+) --- FAILED", t, re.M) + re.findall(r"^test (\S+) \.\.\. FAILED", t, re.M)
    paniques = re.findall(r"panicked at ([^\n]+):\n([^\n]+)", t)
    errores = re.findall(r"^error(?:\[E\d+\])?: ([^\n]+)", t, re.M)
    return {"exit": p.returncode, "fallan": sorted(set(fallan)), "panics": paniques[:4], "errores": errores[:4], "salida": t}, None


def ediciones(estado_dir):
    cat = collections.Counter()
    pruebas_propias = 0
    for f in glob.glob(os.path.join(estado_dir, "*conversacion.json")):
        d = leer_json(f) or []
        msgs = d if isinstance(d, list) else d.get("mensajes", d.get("messages", []))
        calls = {}
        for m in msgs:
            for tc in m.get("tool_calls", []) or []:
                calls[tc["id"]] = tc["function"]
                if "#[test]" in tc["function"].get("arguments", ""):
                    pruebas_propias += 1
            if m.get("role") == "tool":
                fn = calls.get(m.get("tool_call_id"))
                if not fn or fn["name"] not in ("reemplazar_texto", "reemplazar_lineas", "reemplazar_funcion", "insertar"):
                    continue
                t = m.get("content", "")
                if t.startswith("Hecho"):
                    k = "aplicada"
                elif "SINTAXIS" in t:
                    k = "rechazo sintaxis"
                elif "desequilibrados" in t:
                    k = "rechazo llaves"
                elif "no cambia nada" in t or "YA está aplicado" in t:
                    k = "sin efecto"
                elif "No he ejecutado" in t:
                    k = "bloqueada"
                else:
                    k = "otro error"
                cat[k] += 1
    return cat, pruebas_propias


def clasificar(reg, ver, dif, cat):
    if reg["resultado"] == "bloqueada" or not dif.strip():
        return "NO CAMBIÓ NADA: el agente terminó sin editar (o sus ediciones se revirtieron)"
    if reg["resultado"] == "error" and (ver is None or ver[0] is None):
        return "INTENTO NO TERMINADO: agotó turnos, herramientas o falló la infraestructura"
    if ver is None or ver[0] is None:
        return "sin verificar"
    v = ver[0]
    previo = "(NO TERMINÓ; estado que dejó en disco) " if reg["resultado"] == "error" else ""
    if v["exit"] == 0:
        return previo + "HABRÍA PASADO la prueba reservada"
    if v["errores"] and not v["fallan"]:
        return previo + "NO COMPILA con las pruebas reservadas: " + "; ".join(v["errores"][:2])
    if v["fallan"]:
        return previo + "CASO LÍMITE NO CUBIERTO: fallan " + ", ".join(v["fallan"][:3])
    return "rechazada por otra causa (revisar la salida)"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dir")
    ap.add_argument("--banco", default=os.path.join(RAIZ, "tests/self-improvement/campana4"))
    ap.add_argument("--base", default=os.path.join(RAIZ, "target/gemma-work/base0"))
    ap.add_argument("--bin", default=os.path.join(RAIZ, "target/gemma-work/target-v21/debug/soso-improve"))
    ap.add_argument("--sin-verificar", action="store_true")
    a = ap.parse_args()
    d = os.path.abspath(a.dir)
    registros = []
    for f in sorted(glob.glob(os.path.join(d, "registros", "R*.json"))):
        r = leer_json(f)
        if r:
            registros.append(r)
    acept = [r for r in registros if r["resultado"] == "aceptada"]
    out = [f"# Análisis de {os.path.basename(d)}\n", f"{len(acept)} aceptadas de {len(registros)}. Las demás, una a una:\n"]
    resumen = []
    for r in registros:
        if r["resultado"] == "aceptada":
            continue
        t = r["id"] + ("" if r.get("ronda", 1) == 1 else f"-r{r['ronda']}")
        w = os.path.join(d, t)
        copia = os.path.join(w, "copia")
        rutas = archivos_tocables(os.path.join(copia, "TASK.md"))
        dif = diff(a.base, copia, rutas) if os.path.isdir(copia) else ""
        cat, propias = ediciones(os.path.join(w, "estado"))
        ver = (None, None)
        if not a.sin_verificar and dif.strip() and r["resultado"] in ("rechazada", "error") and os.path.isdir(copia):
            # Cargo decide si recompilar por la FECHA de los fuentes, no por su contenido: un archivo editado hace horas es «más viejo» que
            # el artefacto que otra tarea compiló hace un minuto con la base, y se reutilizaría el código equivocado (R36 de la campaña 39
            # salía con dos pruebas fallando en lugar de una). Se actualiza la fecha de lo que el agente cambió.
            for ruta in rutas:
                try:
                    os.utime(os.path.join(copia, ruta))
                except OSError:
                    pass
            ver = verificar(a.bin, a.banco, r["id"], copia)
        causa = clasificar(r, ver, dif, cat)
        resumen.append((t, r["resultado"], causa))
        out.append(f"\n## {t} — {r['resultado']} ({r['segundos']} s, {r.get('herramientas')} herramientas)\n")
        out.append(f"**Causa (orientativa):** {causa}\n")
        out.append(f"**Ediciones:** {dict(cat)}; pruebas propias escritas: {propias}\n")
        if ver[0]:
            v = ver[0]
            if v["panics"]:
                out.append("**Panics de las pruebas reservadas:**\n" + "".join(f"- `{p[0]}`: {p[1]}\n" for p in v["panics"]))
            if v["fallan"]:
                out.append("**Pruebas reservadas que fallan:** " + ", ".join(v["fallan"]) + "\n")
        if dif.strip():
            lineas = dif.splitlines()
            out.append("\n```diff\n" + "\n".join(lineas[:90]) + ("\n… (diff recortado)" if len(lineas) > 90 else "") + "\n```\n")
    out.insert(2, "| tarea | resultado | causa |\n|---|---|---|\n" + "\n".join(f"| {t} | {s} | {c[:110]} |" for t, s, c in resumen) + "\n")
    destino = os.path.join(d, "ANALISIS.md")
    open(destino, "w").write("\n".join(out))
    print(destino)
    for t, s, c in resumen:
        print(f"{t:8} {s:10} {c[:140]}")


if __name__ == "__main__":
    main()
