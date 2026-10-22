#!/usr/bin/env python3
"""Proxy local OpenAI-chat → `claude -p` (suscripción de Claude Code) para correr el arnés de campañas con Claude como modelo.

El arnés habla HTTP plano con 127.0.0.1:PUERTO (POST /v1/chat/completions, mensajes + herramientas → tool_calls). Este proxy recibe esa
petición, la convierte en UNA invocación del CLI oficial `claude -p` (sin herramientas propias, sin memoria/skills/MCP, con nuestro system
prompt) y devuelve la respuesta como una llamada de herramienta del arnés.

Uso:   claude_proxy.py [--puerto 7450] [--max-llamadas 150] [--registro ruta.jsonl] [--parada ruta] [--seco]
Parar: crear el archivo de `--parada` (por defecto /tmp/claude-proxy.stop) o matar el proceso.

Reglas de seguridad:
  - Se niega a arrancar si hay ANTHROPIC_API_KEY / ANTHROPIC_AUTH_TOKEN / proveedor cloud en el entorno: `claude -p` usaría esa clave y
    FACTURARÍA por API en vez de gastar la suscripción. Las variables se eliminan además del entorno del subproceso.
  - No lee ni toca ~/.claude/.credentials.json: el login lo gestiona el binario oficial sin modificar.
  - Escucha sólo en 127.0.0.1 y atiende de una en una (el arnés es secuencial; así el consumo es medible).
  - Cada llamada queda en el registro (duración, tokens) para vigilar el cupo de la ventana de 5 horas.
  - `--seco` imprime lo que enviaría y NO llama a claude.
"""
import argparse, json, os, shutil, subprocess, sys, tempfile, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PELIGROSAS = ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY")
CLAUDE = os.environ.get("CLAUDE_BIN", "claude")

INSTRUCCION = (
    "Eres el modelo de un agente. Se te entrega la conversación hasta ahora y las herramientas disponibles. "
    "Responde SIEMPRE con un único objeto JSON, sin texto antes ni después ni bloques de código:\n"
    '  - para llamar a UNA herramienta: {"tool": "<nombre>", "arguments": { ... }}\n'
    '  - cuando hayas terminado todos los pasos y toque el informe final: {"final": "<texto del informe>"}\n'
    "Nunca inventes el resultado de una herramienta: se te dará en el mensaje siguiente."
)


def renderizar(msgs, tools):
    """(system, prompt): el system va aparte; el resto se vuelca como transcripción legible."""
    system = "\n\n".join(m.get("content") or "" for m in msgs if m.get("role") == "system")
    partes = []
    for m in msgs:
        r = m.get("role")
        if r == "system":
            continue
        if r == "user":
            partes.append("[USUARIO]\n" + (m.get("content") or ""))
        elif r == "assistant":
            txt = (m.get("content") or "").strip()
            if txt:
                partes.append("[TÚ (texto)]\n" + txt)
            for tc in m.get("tool_calls") or []:
                f = tc.get("function", {})
                partes.append(f"[TÚ llamaste a la herramienta] id={tc.get('id')} nombre={f.get('name')} argumentos={f.get('arguments')}")
        elif r == "tool":
            partes.append(f"[RESULTADO de la herramienta] id={m.get('tool_call_id')}\n" + (m.get("content") or ""))
    defs = []
    for t in tools or []:
        f = t.get("function", t)
        defs.append({"nombre": f.get("name"), "descripcion": f.get("description", ""), "parametros": f.get("parameters", {})})
    prompt = (
        INSTRUCCION + "\n\n=== HERRAMIENTAS DISPONIBLES (JSON) ===\n" + json.dumps(defs, ensure_ascii=False)
        + "\n\n=== CONVERSACIÓN ===\n" + "\n\n".join(partes) + "\n\n=== TU TURNO ===\nResponde ahora con el JSON."
    )
    return system, prompt


def esquema(tools):
    nombres = [(t.get("function", t)).get("name") for t in tools or []]
    return {
        "type": "object",
        "properties": {
            "tool": {"type": "string", "enum": nombres} if nombres else {"type": "string"},
            "arguments": {"type": "object"},
            "final": {"type": "string"},
        },
    }


def extraer(texto):
    """JSON del texto de la respuesta (por si no llega `structured_output`): tolera ```json … ```."""
    t = texto.strip()
    if t.startswith("```"):
        t = t.strip("`")
        t = t[t.find("{"):] if "{" in t else t
    i, j = t.find("{"), t.rfind("}")
    if i < 0 or j < i:
        return None
    try:
        return json.loads(t[i:j + 1])
    except Exception:
        return None


def a_openai(obj, uso, contador):
    if obj and obj.get("tool"):
        contador[0] += 1
        args = obj.get("arguments")
        msg = {"role": "assistant", "content": None, "tool_calls": [{
            "id": f"call_{contador[0]}", "type": "function",
            "function": {"name": obj["tool"], "arguments": json.dumps(args if isinstance(args, dict) else {}, ensure_ascii=False)}}]}
    else:
        msg = {"role": "assistant", "content": (obj or {}).get("final") or ""}
    return {"id": "claude-proxy", "object": "chat.completion", "choices": [{"index": 0, "message": msg, "finish_reason": "stop"}], "usage": uso}


class Estado:
    def __init__(self, a):
        self.a = a
        self.cerrojo = threading.Lock()
        self.n = 0
        self.tok_in = 0
        self.tok_out = 0
        self.contador = [0]
        self.dir = tempfile.mkdtemp(prefix="claude-proxy-")  # cwd vacío: nada del repo ni CLAUDE.md a la vista


def llamar_claude(est, modelo, system, prompt, tools):
    a = est.a
    sf = os.path.join(est.dir, "system.txt")
    open(sf, "w").write(system or "Eres un asistente conciso.")
    cmd = [CLAUDE, "-p", "--safe-mode", "--tools", "", "--no-session-persistence", "--output-format", "json",
           "--json-schema", json.dumps(esquema(tools)), "--system-prompt-file", sf]
    if modelo:
        cmd += ["--model", modelo]
    if a.seco:
        sys.stderr.write("[seco] " + " ".join(cmd[:8]) + " … modelo=%s\n[seco] prompt: %d caracteres\n" % (modelo, len(prompt)))
        return {"structured_output": {"final": "(modo seco: no se llamó a claude)"}, "usage": {"input_tokens": len(prompt) // 4, "output_tokens": 8}}, 0.0
    env = {k: v for k, v in os.environ.items() if k not in PELIGROSAS}
    ultimo = ""
    for intento in range(1, a.reintentos + 2):
        t0 = time.time()
        try:
            p = subprocess.run(cmd, input=prompt, capture_output=True, text=True, timeout=a.plazo, env=env, cwd=est.dir)
        except subprocess.TimeoutExpired:
            ultimo = f"claude no contestó en {a.plazo} s"
            sys.stderr.write(f"[reintento {intento}] {ultimo}\n")
            continue
        dt = time.time() - t0
        try:
            r = json.loads(p.stdout)
        except Exception:
            r = None
        # El CLI puede salir con código ≠ 0 y aun así traer una respuesta válida (p. ej. `is_error` con contenido): se aprovecha.
        if r is not None and isinstance(r.get("structured_output"), dict) and not r.get("is_error"):
            return r, dt
        if p.returncode == 0 and r is not None and not r.get("is_error"):
            return r, dt
        motivo = {k: r.get(k) for k in ("subtype", "is_error", "stop_reason", "terminal_reason", "result", "errors", "api_error_status") if r and k in r} if r else {}
        ultimo = f"claude salió con {p.returncode}: {json.dumps(motivo, ensure_ascii=False)[:1500] or (p.stderr or p.stdout)[:600]}"
        sys.stderr.write(f"[reintento {intento}/{a.reintentos + 1}] {ultimo}\n")
        if r is not None and a.registro:
            with open(a.registro + ".errores", "a") as f:
                f.write(json.dumps({"intento": intento, "codigo": p.returncode, "salida": r}, ensure_ascii=False)[:6000] + "\n")
        time.sleep(3)
    raise RuntimeError(ultimo)


def manejador(est):
    a = est.a

    class H(BaseHTTPRequestHandler):
        def log_message(self, *x):
            pass

        def _responder(self, codigo, cuerpo):
            b = json.dumps(cuerpo, ensure_ascii=False).encode()
            self.send_response(codigo)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(b)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(b)

        def do_GET(self):
            if self.path.rstrip("/") in ("/v1/models", "/models"):
                return self._responder(200, {"data": [{"id": "claude-proxy"}]})
            self._responder(404, {"error": "no existe"})

        def do_POST(self):
            if not self.path.endswith("/chat/completions"):
                return self._responder(404, {"error": "sólo /v1/chat/completions"})
            if os.path.exists(a.parada):
                return self._responder(503, {"error": {"message": f"parada pedida ({a.parada} existe)"}})
            n = int(self.headers.get("Content-Length") or 0)
            try:
                req = json.loads(self.rfile.read(n))
            except Exception:
                return self._responder(400, {"error": {"message": "JSON inválido"}})
            with est.cerrojo:
                if est.n >= a.max_llamadas:
                    return self._responder(500, {"error": {"message": f"tope de {a.max_llamadas} llamadas alcanzado (--max-llamadas)"}})
                est.n += 1
                system, prompt = renderizar(req.get("messages", []), req.get("tools"))
                try:
                    r, dt = llamar_claude(est, req.get("model"), system, prompt, req.get("tools"))
                except Exception as e:
                    sys.stderr.write(f"[error] {e}\n")
                    return self._responder(500, {"error": {"message": str(e)}})
                if r.get("is_error"):
                    return self._responder(500, {"error": {"message": "claude devolvió error: " + str(r.get("result", ""))[:300]}})
                u = r.get("usage") or {}
                ent = (u.get("input_tokens") or 0) + (u.get("cache_creation_input_tokens") or 0) + (u.get("cache_read_input_tokens") or 0)
                sal = u.get("output_tokens") or 0
                est.tok_in += ent
                est.tok_out += sal
                obj = r.get("structured_output")
                if not isinstance(obj, dict):
                    obj = extraer(r.get("result") or "")
                if a.registro:
                    with open(a.registro, "a") as f:
                        f.write(json.dumps({"n": est.n, "seg": round(dt, 1), "entrada": ent, "salida": sal, "ok": bool(obj),
                                            "acum_entrada": est.tok_in, "acum_salida": est.tok_out, "coste_usd_estimado": r.get("total_cost_usd"),
                                        "modelo_pedido": req.get("model"), "modelos_usados": sorted((r.get("modelUsage") or {}).keys())}) + "\n")
                sys.stderr.write(f"[{est.n}] {dt:.1f}s entrada={ent} salida={sal} acumulado={est.tok_in}/{est.tok_out}\n")
                self._responder(200, a_openai(obj, {"prompt_tokens": ent, "completion_tokens": sal, "total_tokens": ent + sal}, est.contador))

    return H


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--puerto", type=int, default=7450)
    ap.add_argument("--max-llamadas", type=int, default=150, help="tope de invocaciones de claude (una por turno del agente)")
    ap.add_argument("--plazo", type=int, default=300, help="segundos máximos por invocación")
    ap.add_argument("--reintentos", type=int, default=2, help="reintentos de una invocación fallida antes de devolver error al arnés")
    ap.add_argument("--registro", default="/tmp/claude-proxy.jsonl")
    ap.add_argument("--parada", default="/tmp/claude-proxy.stop")
    ap.add_argument("--seco", action="store_true", help="no llama a claude: sólo muestra lo que enviaría")
    a = ap.parse_args()
    puestas = [k for k in PELIGROSAS if os.environ.get(k)]
    if puestas and not a.seco:
        sys.exit(f"NO arranco: {', '.join(puestas)} está definida y `claude -p` la usaría (facturación por API, no suscripción). Haz `unset` y vuelve a lanzar.")
    if not a.seco and not shutil.which(CLAUDE):
        sys.exit(f"no encuentro `{CLAUDE}` en el PATH")
    est = Estado(a)
    srv = ThreadingHTTPServer(("127.0.0.1", a.puerto), manejador(est))
    sys.stderr.write(f"claude-proxy en 127.0.0.1:{a.puerto} · tope {a.max_llamadas} llamadas · registro {a.registro} · parada {a.parada}{' · MODO SECO' if a.seco else ''}\n")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        shutil.rmtree(est.dir, ignore_errors=True)
        sys.stderr.write(f"fin: {est.n} llamadas, entrada {est.tok_in}, salida {est.tok_out}\n")


if __name__ == "__main__":
    main()
