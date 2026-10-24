#!/usr/bin/env python3
"""Fixtures del render de Qwen3.5/3.6: las mismas entradas que `../qwen3coder/casos.json`, pasadas por `/apply-template` de un
llama-server con el GGUF de Qwen3.6 (plantilla de Unsloth). Uso: generar.py [http://127.0.0.1:7497]. Las claves de esquemas y de
argumentos se ordenan alfabéticamente (el render de soso no conserva el orden de inserción)."""
import json, sys, urllib.request, os

URL = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:7497"

def ordenar(v):
    if isinstance(v, dict):
        return {k: ordenar(v[k]) for k in sorted(v)}
    if isinstance(v, list):
        return [ordenar(x) for x in v]
    return v

def preparar(entrada):
    msgs = []
    for m in entrada["messages"]:
        m = dict(m)
        for c in m.get("tool_calls", []) or []:
            args = c["function"]["arguments"]
            if isinstance(args, str):
                args = json.loads(args)
            c["function"]["arguments"] = json.dumps(ordenar(args), ensure_ascii=False)
        msgs.append(m)
    tools = []
    for t in entrada.get("tools", []):
        f = t["function"]
        g = {"name": f["name"]}
        if "description" in f:
            g["description"] = f["description"]
        g["parameters"] = ordenar(f["parameters"])
        tools.append({"type": "function", "function": g})
    return msgs, tools

def aplicar(msgs, tools, pensar):
    cuerpo = {"messages": msgs, "chat_template_kwargs": {"enable_thinking": pensar}}
    if tools:
        cuerpo["tools"] = tools
    r = urllib.request.Request(URL + "/apply-template", json.dumps(cuerpo).encode(), {"content-type": "application/json"})
    return json.load(urllib.request.urlopen(r))["prompt"]

base = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "qwen3coder", "casos.json")
salida = []
for caso in json.load(open(base)):
    msgs, tools = preparar(caso["entrada"])
    entrada = {"messages": msgs, "tools": tools}
    salida.append({
        "nombre": caso["nombre"],
        "entrada": entrada,
        "esperado": aplicar(msgs, tools, False),
        "esperado_pensar": aplicar(msgs, tools, True),
    })
json.dump(salida, open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "casos.json"), "w"), indent=1, ensure_ascii=False)
print(len(salida), "casos")
