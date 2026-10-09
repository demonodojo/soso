#!/usr/bin/env python3
"""Genera los casos de referencia del render de Qwen3-Coder con la plantilla Jinja oficial.

    python3 generar.py <chat_template.jinja> <salida.json>

La plantilla es la de `Qwen/Qwen3-Coder-30B-A3B-Instruct` (hash en `LEEME.md`). Se renderiza
como lo hace Hugging Face: entorno sandbox con `trim_blocks` y `lstrip_blocks`, `tojson`
sin ordenar claves y sin escapar no-ASCII.
"""
import json, sys
from jinja2.sandbox import ImmutableSandboxedEnvironment

def tojson(x, ensure_ascii=False, indent=None, separators=None, sort_keys=False):
    return json.dumps(x, ensure_ascii=ensure_ascii, indent=indent, separators=separators, sort_keys=sort_keys)

env = ImmutableSandboxedEnvironment(trim_blocks=True, lstrip_blocks=True, extensions=["jinja2.ext.loopcontrols"])
env.filters["tojson"] = tojson
plantilla = env.from_string(open(sys.argv[1], encoding="utf-8").read())

READ = {"type": "function", "function": {"name": "read", "description": "  Lee un archivo.\n",
    "parameters": {"type": "object", "properties": {
        "filePath": {"type": "string", "description": "Ruta absoluta"},
        "limit": {"type": "integer", "description": "Líneas", "minimum": 1}},
        "additionalProperties": False, "required": ["filePath"]}}}
BASH = {"type": "function", "function": {"name": "bash", "description": "Ejecuta un comando",
    "parameters": {"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}}}

def llamada(nombre, args):
    return {"id": "call_1", "type": "function", "function": {"name": nombre, "arguments": args}}

casos = {
  "sin_herramientas_con_sistema": {"messages": [
      {"role": "system", "content": "Eres útil."}, {"role": "user", "content": "hola"}], "tools": []},
  "sin_herramientas_sin_sistema": {"messages": [{"role": "user", "content": "hola"}], "tools": []},
  "herramientas_sin_sistema": {"messages": [{"role": "user", "content": "lee TASK.md"}], "tools": [READ, BASH]},
  "herramientas_con_sistema": {"messages": [
      {"role": "system", "content": "Agente de soso."}, {"role": "user", "content": "lee TASK.md"}], "tools": [READ]},
  "ciclo_llamada_resultado_respuesta": {"messages": [
      {"role": "system", "content": "Agente."},
      {"role": "user", "content": "lee TASK.md"},
      {"role": "assistant", "content": "", "tool_calls": [llamada("read", {"filePath": "TASK.md"})]},
      {"role": "tool", "tool_call_id": "call_1", "content": "contenido del archivo"},
      {"role": "assistant", "content": "Listo, lo leí."},
      {"role": "user", "content": "gracias"}], "tools": [READ]},
  "asistente_con_texto_y_llamada": {"messages": [
      {"role": "user", "content": "x"},
      {"role": "assistant", "content": "  Voy a leerlo.\n", "tool_calls": [llamada("read", {"filePath": "a.rs", "limit": 20})]},
      {"role": "tool", "tool_call_id": "call_1", "content": "ok"}], "tools": [READ]},
  "argumentos_no_cadena": {"messages": [
      {"role": "user", "content": "x"},
      {"role": "assistant", "content": "", "tool_calls": [llamada("bash", {"command": "ls", "lista": [1, "a"], "n": 3, "obj": {"k": 1}, "ok": True})]},
      {"role": "tool", "tool_call_id": "call_1", "content": "a\nb"}], "tools": [BASH]},
  "dos_resultados_seguidos": {"messages": [
      {"role": "user", "content": "x"},
      {"role": "assistant", "content": "", "tool_calls": [llamada("read", {"filePath": "a"}), llamada("read", {"filePath": "b"})]},
      {"role": "tool", "tool_call_id": "call_1", "content": "A"},
      {"role": "tool", "tool_call_id": "call_2", "content": "B"}], "tools": [READ]},
}
salida = []
for nombre, c in casos.items():
    texto = plantilla.render(messages=c["messages"], tools=c["tools"], add_generation_prompt=True)
    salida.append({"nombre": nombre, "entrada": c, "esperado": texto})
json.dump(salida, open(sys.argv[2], "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print(len(salida), "casos")
