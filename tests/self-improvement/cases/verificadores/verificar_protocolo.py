#!/usr/bin/env python3
"""Verificador de los casos de protocolo del banco (ficha T02).

Un caso de protocolo se juzga sobre un **documento de respuesta**: lo que el
lanzador grabó al mandar la petición visible contra el servidor. Así este
verificador no necesita ni servidor ni modelo, y las mismas aserciones valen
para el ejemplo host (T13) y para el guest (T16).

Formato del documento (JSON):

    {"http_status": 200, "cuerpo": { ... }}                  respuesta completa
    {"http_status": 200, "trozos_b64": ["ZGF0YT...", ...]}   SSE tal y como llegó

En el caso SSE los trozos son los bytes recibidos, en base64 y en orden: el
corte entre trozos puede caer **en medio de un carácter**, que es justo lo que
hay que reensamblar antes de decodificar. El verificador concatena, decodifica
UTF-8 y solo entonces interpreta los eventos.

    verificar_protocolo.py --caso Q01 --respuesta respuesta.json [--json informe.json]

Salida: 0 si cumple, 2 si alguna aserción falla, 1 si el caso o el documento no
se pueden usar.
"""
import argparse
import base64
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import banco  # noqa: E402

OK, FALLO, ERROR = 0, 2, 1


class Documento:
    """Respuesta grabada, ya reensamblada y con los accesos que usan las aserciones."""

    def __init__(self, crudo):
        self.estado = crudo.get("http_status")
        self.cuerpo = crudo.get("cuerpo")
        self.sse_texto = None
        self.eventos = []
        self.errores_formato = []
        if "trozos_b64" in crudo:
            self._reensamblar(crudo["trozos_b64"])

    def _reensamblar(self, trozos):
        datos = b"".join(base64.b64decode(t) for t in trozos)
        try:
            self.sse_texto = datos.decode("utf-8")
        except UnicodeDecodeError as e:
            self.errores_formato.append(f"el flujo SSE no es UTF-8 completo: {e}")
            self.sse_texto = datos.decode("utf-8", "replace")
        for bloque in self.sse_texto.split("\n\n"):
            for linea in bloque.splitlines():
                if not linea.startswith("data:"):
                    continue
                carga = linea[len("data:"):].strip()
                if carga == "[DONE]":
                    self.eventos.append("[DONE]")
                    continue
                try:
                    self.eventos.append(json.loads(carga))
                except json.JSONDecodeError as e:
                    self.errores_formato.append(f"evento SSE ilegible: {e}")

    # --- lecturas comunes ---------------------------------------------------

    def mensaje(self):
        """Mensaje del asistente, venga de una respuesta completa o de deltas."""
        if self.cuerpo is not None:
            opciones = self.cuerpo.get("choices") or []
            return opciones[0].get("message") if opciones else None
        contenido, llamadas, nombre = "", {}, {}
        for ev in self.eventos:
            if ev == "[DONE]":
                continue
            for op in ev.get("choices") or []:
                delta = op.get("delta") or {}
                if isinstance(delta.get("content"), str):
                    contenido += delta["content"]
                for tc in delta.get("tool_calls") or []:
                    i = tc.get("index", 0)
                    fn = tc.get("function") or {}
                    if fn.get("name"):
                        nombre[i] = fn["name"]
                    llamadas[i] = llamadas.get(i, "") + (fn.get("arguments") or "")
        mensaje = {"role": "assistant", "content": contenido if contenido else None}
        if llamadas:
            mensaje["tool_calls"] = [
                {"id": f"delta-{i}", "type": "function",
                 "function": {"name": nombre.get(i, ""), "arguments": llamadas[i]}}
                for i in sorted(llamadas)
            ]
        return mensaje

    def finish_reason(self):
        if self.cuerpo is not None:
            opciones = self.cuerpo.get("choices") or []
            return opciones[0].get("finish_reason") if opciones else None
        for ev in reversed(self.eventos):
            if ev == "[DONE]":
                continue
            for op in ev.get("choices") or []:
                if op.get("finish_reason"):
                    return op["finish_reason"]
        return None

    def usage(self):
        if self.cuerpo is not None:
            return self.cuerpo.get("usage")
        for ev in reversed(self.eventos):
            if ev != "[DONE]" and ev.get("usage"):
                return ev["usage"]
        return None

    def error(self):
        return (self.cuerpo or {}).get("error") if isinstance(self.cuerpo, dict) else None


# --- aserciones -------------------------------------------------------------


def _texto(doc):
    m = doc.mensaje() or {}
    return m.get("content") or ""


def _llamadas(doc):
    return (doc.mensaje() or {}).get("tool_calls") or []


def a_estado_http(doc, a):
    if doc.estado != a["valor"]:
        return f"estado HTTP {doc.estado}, se esperaba {a['valor']}"


def a_contenido_no_vacio(doc, a):
    if not _texto(doc).strip():
        return "el asistente no devolvió texto"


def a_contenido_igual(doc, a):
    obtenido = _texto(doc)
    if obtenido != a["valor"]:
        return f"contenido {obtenido!r}, se esperaba {a['valor']!r}"


def a_contenido_contiene(doc, a):
    obtenido = _texto(doc)
    aguja, pajar = a["valor"], obtenido
    if a.get("ignorar_mayusculas", True):
        aguja, pajar = aguja.lower(), pajar.lower()
    if aguja not in pajar:
        return f"el contenido no incluye {a['valor']!r}: {obtenido!r}"


def a_sin_llamadas(doc, a):
    if _llamadas(doc):
        return f"se esperaba texto y hay {len(_llamadas(doc))} llamada(s)"


def a_llamada_unica(doc, a):
    llamadas = _llamadas(doc)
    if len(llamadas) != 1:
        return f"se esperaba exactamente una llamada y hay {len(llamadas)}"
    fn = llamadas[0].get("function") or {}
    if fn.get("name") != a["nombre"]:
        return f"llamada a {fn.get('name')!r}, se esperaba {a['nombre']!r}"
    crudo = fn.get("arguments")
    if not isinstance(crudo, str):
        return "los argumentos deben venir serializados como cadena JSON"
    try:
        argumentos = json.loads(crudo)
    except json.JSONDecodeError as e:
        return f"los argumentos no son JSON completo ({e}): {crudo!r}"
    if not isinstance(argumentos, dict):
        return "los argumentos deben ser un objeto JSON"
    faltan = [k for k in a.get("claves_argumentos", []) if k not in argumentos]
    if faltan:
        return f"a los argumentos les faltan claves {faltan}: {argumentos!r}"


def a_finish_reason(doc, a):
    obtenido = doc.finish_reason()
    if obtenido != a["valor"]:
        return f"finish_reason {obtenido!r}, se esperaba {a['valor']!r}"


def a_usage_coherente(doc, a):
    u = doc.usage()
    if not isinstance(u, dict):
        return "la respuesta no trae usage"
    faltan = [k for k in ("prompt_tokens", "completion_tokens", "total_tokens") if k not in u]
    if faltan:
        return f"usage incompleto, faltan {faltan}"
    if any(not isinstance(u[k], int) or isinstance(u[k], bool) or u[k] < 0 for k in u
           if k in ("prompt_tokens", "completion_tokens", "total_tokens")):
        return f"usage con valores no enteros o negativos: {u}"
    if u["total_tokens"] != u["prompt_tokens"] + u["completion_tokens"]:
        return f"usage incoherente: {u['total_tokens']} != " \
               f"{u['prompt_tokens']} + {u['completion_tokens']}"


def a_error_presente(doc, a):
    err = doc.error()
    if not isinstance(err, dict):
        return "se esperaba un objeto error en el cuerpo"
    if not err.get("message"):
        return "el error no trae message"
    if "tipo_error" in a and err.get("type") != a["tipo_error"]:
        return f"error.type {err.get('type')!r}, se esperaba {a['tipo_error']!r}"
    if "codigo" in a and err.get("code") != a["codigo"]:
        return f"error.code {err.get('code')!r}, se esperaba {a['codigo']!r}"


def a_sin_texto_de_asistente(doc, a):
    if isinstance(doc.cuerpo, dict) and doc.cuerpo.get("choices"):
        return "un error no debe traer texto del asistente"
    if doc.eventos:
        return "un error antes de las cabeceras no debe abrir un flujo SSE"


def a_flujo_utf8_completo(doc, a):
    if doc.sse_texto is None:
        return "el caso exige un documento con trozos_b64"
    if doc.errores_formato:
        return "; ".join(doc.errores_formato)
    if "�" in _texto(doc):
        return "el texto reensamblado tiene caracteres de reemplazo (U+FFFD)"


ASERCIONES = {
    "estado_http": a_estado_http,
    "contenido_no_vacio": a_contenido_no_vacio,
    "contenido_igual": a_contenido_igual,
    "contenido_contiene": a_contenido_contiene,
    "sin_llamadas": a_sin_llamadas,
    "llamada_unica": a_llamada_unica,
    "finish_reason": a_finish_reason,
    "usage_coherente": a_usage_coherente,
    "error_presente": a_error_presente,
    "sin_texto_de_asistente": a_sin_texto_de_asistente,
    "flujo_utf8_completo": a_flujo_utf8_completo,
}


def verificar(caso_id, respuesta, reservado=None):
    caso = banco.caso_por_id(caso_id)
    if caso["clase"] != "protocolo":
        raise banco.ErrorBanco(f"{caso_id} no es un caso de protocolo")
    raiz = banco.raiz_reservada(reservado)
    with open(os.path.join(raiz, caso_id, "esperado.json"), encoding="utf-8") as f:
        esperado = json.load(f)

    informe = {"caso": caso_id, "respuesta": os.path.abspath(respuesta), "aserciones": []}
    if not os.path.isfile(respuesta):
        informe["estado"] = "error"
        informe["motivo"] = f"no existe el documento {respuesta}"
        return ERROR, informe
    try:
        with open(respuesta, encoding="utf-8") as f:
            crudo = json.load(f)
    except (json.JSONDecodeError, UnicodeDecodeError) as e:
        informe["estado"] = "error"
        informe["motivo"] = f"documento ilegible: {e}"
        return ERROR, informe

    doc = Documento(crudo)
    for a in esperado["aserciones"]:
        comprobador = ASERCIONES.get(a["tipo"])
        if comprobador is None:
            informe["estado"] = "error"
            informe["motivo"] = f"aserción desconocida: {a['tipo']}"
            return ERROR, informe
        motivo = comprobador(doc, a)
        informe["aserciones"].append({"tipo": a["tipo"], "paso": motivo is None,
                                      "motivo": motivo})

    fallados = [a for a in informe["aserciones"] if not a["paso"]]
    informe["estado"] = "ok" if not fallados else "fallo"
    informe["pasadas"] = len(informe["aserciones"]) - len(fallados)
    informe["total"] = len(informe["aserciones"])
    return (OK if not fallados else FALLO), informe


def main(argv=None):
    p = argparse.ArgumentParser(prog="verificar_protocolo.py")
    p.add_argument("--caso", required=True)
    p.add_argument("--respuesta", required=True)
    p.add_argument("--reservado", default=None)
    p.add_argument("--json", default=None)
    args = p.parse_args(argv)
    try:
        codigo, informe = verificar(args.caso, args.respuesta, args.reservado)
    except banco.ErrorBanco as e:
        print(f"error: {e}", file=sys.stderr)
        return ERROR
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump(informe, f, indent=2, ensure_ascii=False)
            f.write("\n")
    if informe["estado"] == "error":
        print(f"{args.caso}: ERROR — {informe['motivo']}", file=sys.stderr)
        return codigo
    print(f"{args.caso}: {informe['estado']} ({informe['pasadas']}/{informe['total']} aserciones)")
    for a in informe["aserciones"]:
        if not a["paso"]:
            print(f"  {a['tipo']}: {a['motivo']}", file=sys.stderr)
    return codigo


if __name__ == "__main__":
    sys.exit(main())
