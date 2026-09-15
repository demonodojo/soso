#!/usr/bin/env python3
"""Carga y sella el banco de casos (ficha T02).

Un caso es un JSON con su entrada visible, el resultado observable, el argv de
su comprobador, sus límites y la lista de material reservado. Este módulo es la
única puerta de entrada al banco: lo usan las pruebas, los verificadores y —más
adelante— el coordinador.

Tres invariantes que se comprueban aquí y no en cada consumidor:

- **Partición reproducible.** `desarrollo` o `reservado` no se eligen a mano:
  salen de un hash del id con sal fija. Así la partición se puede rehacer sin
  guardar una lista, y elegir a dedo qué caso se reserva deja de ser posible.
- **Entradas selladas.** Cada caso guarda el SHA-256 de sus archivos visibles y
  el banco guarda una huella de todos los casos. Si una campaña empieza con una
  huella y termina con otra, los resultados no son comparables.
- **Reservado fuera del paquete visible.** El material de `reservado/` nunca
  entra en lo que se copia al agente. `paquete_visible()` es lo que el lanzador
  puede copiar, y `raiz_reservada()` puede apuntar fuera del checkout.

Uso:

    python3 banco.py listar
    python3 banco.py validar
    python3 banco.py sellar     # recalcula hashes de entrada y huella
"""
import argparse
import hashlib
import json
import os
import re
import sys

RAIZ = os.path.dirname(os.path.abspath(__file__))
ESQUEMA = os.path.join(os.path.dirname(RAIZ), "case.schema.json")
MANIFIESTO = os.path.join(RAIZ, "banco.json")

CLASES = ("programacion", "protocolo", "repo")
DISTRIBUCION = {"programacion": 10, "protocolo": 10, "repo": 5}

# Regla de partición: fija, publicada y sin lista que mantener.
SAL_PARTICION = "soso-banco-v1"
MODULO_PARTICION = 5
CORTE_RESERVADO = 2

# Variable para mover el material reservado fuera del checkout que ve el agente.
ENV_RESERVADO = "SOSO_BANCO_RESERVADO"


class ErrorBanco(Exception):
    """Banco mal formado: ids repetidos, rutas fuera, esquema incumplido."""


# --- utilidades -------------------------------------------------------------


def sha256_archivo(ruta):
    h = hashlib.sha256()
    with open(ruta, "rb") as f:
        for trozo in iter(lambda: f.read(1 << 20), b""):
            h.update(trozo)
    return h.hexdigest()


def particion_de(caso_id):
    """Partición reproducible a partir del id. No depende del contenido."""
    h = hashlib.sha256(f"{SAL_PARTICION}|{caso_id}".encode()).hexdigest()
    return "reservado" if int(h[:8], 16) % MODULO_PARTICION < CORTE_RESERVADO else "desarrollo"


def ruta_relativa_segura(ruta):
    if ruta.startswith("/") or ".." in ruta.split("/"):
        raise ErrorBanco(f"ruta no relativa o con '..': {ruta!r}")
    return ruta


# --- validación de esquema --------------------------------------------------


def _validar_nodo(valor, esquema, ruta, errores):
    """Subconjunto de JSON Schema suficiente para `case.schema.json`.

    Se implementa aquí para no depender de un paquete externo: las pruebas de
    este repositorio solo usan la biblioteca estándar. Si `jsonschema` está
    instalado, `validar_esquema` lo prefiere.
    """
    tipo = esquema.get("type")
    tipos = {
        "object": dict, "array": list, "string": str,
        "integer": int, "number": (int, float), "boolean": bool,
    }
    if tipo == "integer" and isinstance(valor, bool):
        errores.append(f"{ruta}: se esperaba integer, no boolean")
        return
    if tipo and not isinstance(valor, tipos[tipo]):
        errores.append(f"{ruta}: se esperaba {tipo}, hay {type(valor).__name__}")
        return
    if "enum" in esquema and valor not in esquema["enum"]:
        errores.append(f"{ruta}: {valor!r} no está en {esquema['enum']}")
    if tipo == "string":
        if "pattern" in esquema and not re.search(esquema["pattern"], valor):
            errores.append(f"{ruta}: {valor!r} no casa con {esquema['pattern']}")
        if "minLength" in esquema and len(valor) < esquema["minLength"]:
            errores.append(f"{ruta}: más corto que {esquema['minLength']}")
    if tipo == "integer" or tipo == "number":
        if "minimum" in esquema and valor < esquema["minimum"]:
            errores.append(f"{ruta}: {valor} < {esquema['minimum']}")
        if "maximum" in esquema and valor > esquema["maximum"]:
            errores.append(f"{ruta}: {valor} > {esquema['maximum']}")
    if tipo == "array":
        if "minItems" in esquema and len(valor) < esquema["minItems"]:
            errores.append(f"{ruta}: menos de {esquema['minItems']} elementos")
        if "items" in esquema:
            for i, v in enumerate(valor):
                _validar_nodo(v, esquema["items"], f"{ruta}[{i}]", errores)
    if tipo == "object":
        for req in esquema.get("required", []):
            if req not in valor:
                errores.append(f"{ruta}: falta el campo obligatorio {req!r}")
        props = esquema.get("properties", {})
        if esquema.get("additionalProperties") is False:
            for k in valor:
                if k not in props:
                    errores.append(f"{ruta}: campo no declarado {k!r}")
        for k, v in valor.items():
            if k in props:
                _validar_nodo(v, props[k], f"{ruta}.{k}", errores)


def validar_esquema(caso):
    with open(ESQUEMA, encoding="utf-8") as f:
        esquema = json.load(f)
    try:
        import jsonschema  # opcional: si está, manda su validador
    except ImportError:
        errores = []
        _validar_nodo(caso, esquema, caso.get("id", "?"), errores)
        return errores
    validador = jsonschema.Draft202012Validator(esquema)
    return [f"{caso.get('id','?')}{list(e.path)}: {e.message}"
            for e in validador.iter_errors(caso)]


# --- carga ------------------------------------------------------------------


def raiz_reservada(explicita=None):
    """Dónde vive el material reservado.

    Por defecto, dentro del banco. En una campaña real se mueve fuera del
    checkout que ve el agente y se apunta con `--reservado` o
    `SOSO_BANCO_RESERVADO`; los verificadores no necesitan nada más.
    """
    if explicita:
        return os.path.abspath(explicita)
    if os.environ.get(ENV_RESERVADO):
        return os.path.abspath(os.environ[ENV_RESERVADO])
    return os.path.join(RAIZ, "reservado")


def archivos_de_caso(clase):
    directorio = os.path.join(RAIZ, clase)
    if not os.path.isdir(directorio):
        return []
    return sorted(
        os.path.join(directorio, n) for n in os.listdir(directorio) if n.endswith(".json")
    )


def cargar_casos(validar=True):
    """Devuelve la lista de casos ordenada por id, con `_ruta` añadido."""
    casos, vistos = [], {}
    for clase in CLASES:
        for ruta in archivos_de_caso(clase):
            with open(ruta, encoding="utf-8") as f:
                caso = json.load(f)
            caso["_ruta"] = os.path.relpath(ruta, RAIZ)
            if validar:
                errores = validar_esquema({k: v for k, v in caso.items() if k != "_ruta"})
                if errores:
                    raise ErrorBanco(f"{caso['_ruta']}: " + "; ".join(errores))
            if caso["clase"] != clase:
                raise ErrorBanco(f"{caso['_ruta']}: clase {caso['clase']} en directorio {clase}")
            if caso["id"] in vistos:
                raise ErrorBanco(f"id repetido {caso['id']}: {vistos[caso['id']]} y {caso['_ruta']}")
            vistos[caso["id"]] = caso["_ruta"]
            casos.append(caso)
    casos.sort(key=lambda c: c["id"])
    return casos


def caso_por_id(caso_id, casos=None):
    for c in casos if casos is not None else cargar_casos():
        if c["id"] == caso_id:
            return c
    raise ErrorBanco(f"caso desconocido: {caso_id}")


def paquete_visible(caso):
    """Rutas que el lanzador puede copiar al espacio del agente."""
    rutas = [caso["entrada"]["enunciado"]] + list(caso["entrada"].get("archivos", []))
    for r in rutas:
        ruta_relativa_segura(r)
        if not r.startswith("visible/"):
            raise ErrorBanco(f"{caso['id']}: entrada fuera de visible/: {r}")
    return rutas


def hashes_calculados(caso):
    return {r: sha256_archivo(os.path.join(RAIZ, r)) for r in paquete_visible(caso)}


def huella(casos):
    """Huella del banco entero: identifica la distribución de una campaña."""
    h = hashlib.sha256()
    for c in casos:
        h.update(c["id"].encode())
        h.update(b"\0")
        h.update(sha256_archivo(os.path.join(RAIZ, c["_ruta"])).encode())
        h.update(b"\0")
    return h.hexdigest()


def manifiesto(casos=None):
    casos = casos if casos is not None else cargar_casos()
    conteo = {clase: sum(1 for c in casos if c["clase"] == clase) for clase in CLASES}
    particiones = {
        clase: {
            "desarrollo": sum(1 for c in casos
                              if c["clase"] == clase and c["particion"] == "desarrollo"),
            "reservado": sum(1 for c in casos
                             if c["clase"] == clase and c["particion"] == "reservado"),
        }
        for clase in CLASES
    }
    return {
        "schema_version": 1,
        "distribucion": conteo,
        "distribucion_exigida": DISTRIBUCION,
        "particion": {
            "regla": "sha256('" + SAL_PARTICION + "|' + id)[:8] % "
                     f"{MODULO_PARTICION} < {CORTE_RESERVADO} -> reservado",
            "conteo": particiones,
        },
        "umbrales": {
            "estado": "no fijados",
            "fijar_en": "T14",
            "nota": "T02 no mide nada: fijar un umbral sin ejecución sería inventarlo. "
                    "T14 los escribe aquí con su evidencia y desde entonces son "
                    "inmutables durante la campaña.",
            "por_clase": {clase: None for clase in CLASES},
        },
        "huella": huella(casos),
        "casos": [
            {"id": c["id"], "clase": c["clase"], "particion": c["particion"],
             "titulo": c["titulo"], "archivo": c["_ruta"]}
            for c in casos
        ],
    }


# --- CLI --------------------------------------------------------------------


def comprobar(casos=None):
    """Comprobaciones estructurales; devuelve la lista de problemas."""
    casos = casos if casos is not None else cargar_casos()
    problemas = []
    for clase, esperados in DISTRIBUCION.items():
        hay = sum(1 for c in casos if c["clase"] == clase)
        if hay != esperados:
            problemas.append(f"clase {clase}: {hay} casos, se exigen {esperados}")
    for c in casos:
        if c["particion"] != particion_de(c["id"]):
            problemas.append(
                f"{c['id']}: partición {c['particion']} no sale de la regla "
                f"({particion_de(c['id'])})"
            )
        for r in paquete_visible(c):
            if not os.path.isfile(os.path.join(RAIZ, r)):
                problemas.append(f"{c['id']}: falta la entrada {r}")
        for r in c["reservado"]:
            ruta_relativa_segura(r)
            destino = os.path.join(raiz_reservada(), r[len("reservado/"):])
            if not os.path.isfile(destino):
                problemas.append(f"{c['id']}: falta el material reservado {r}")
        esperados = hashes_calculados(c)
        if c["hashes_entrada"] != esperados:
            problemas.append(f"{c['id']}: hashes_entrada no coincide con el contenido")
        for arg in c["comprobador"]["argv"]:
            if arg.startswith("reservado/"):
                problemas.append(f"{c['id']}: el argv del comprobador expone {arg}")
    if os.path.isfile(MANIFIESTO):
        with open(MANIFIESTO, encoding="utf-8") as f:
            guardado = json.load(f)
        if guardado.get("huella") != huella(casos):
            problemas.append("banco.json: la huella no corresponde a los casos actuales")
    else:
        problemas.append("falta banco.json")
    return problemas


def sellar():
    """Recalcula partición, hashes de entrada y huella.

    La partición también se escribe aquí: es dato derivado del id, no una
    elección. Un cambio deliberado del banco pasa por este sellado.
    """
    casos = cargar_casos()
    for c in casos:
        nuevos = hashes_calculados(c)
        particion = particion_de(c["id"])
        if c["hashes_entrada"] == nuevos and c["particion"] == particion:
            continue
        ruta = os.path.join(RAIZ, c["_ruta"])
        with open(ruta, encoding="utf-8") as f:
            crudo = json.load(f)
        crudo["hashes_entrada"] = nuevos
        crudo["particion"] = particion
        with open(ruta, "w", encoding="utf-8") as f:
            json.dump(crudo, f, indent=2, ensure_ascii=False)
            f.write("\n")
        c["hashes_entrada"] = nuevos
        c["particion"] = particion
    with open(MANIFIESTO, "w", encoding="utf-8") as f:
        json.dump(manifiesto(casos), f, indent=2, ensure_ascii=False)
        f.write("\n")
    return casos


def main(argv=None):
    p = argparse.ArgumentParser(prog="banco.py", description="Banco de casos de automejora")
    sub = p.add_subparsers(dest="comando", required=True)
    sub.add_parser("listar")
    sub.add_parser("validar")
    sub.add_parser("sellar")
    args = p.parse_args(argv)
    try:
        if args.comando == "sellar":
            casos = sellar()
            print(f"sellados {len(casos)} casos; huella {huella(casos)[:16]}")
            return 0
        casos = cargar_casos()
        if args.comando == "listar":
            for c in casos:
                print(f"{c['id']}  {c['clase']:12} {c['particion']:10} {c['titulo']}")
            return 0
        problemas = comprobar(casos)
        for x in problemas:
            print(f"problema: {x}", file=sys.stderr)
        print(f"{len(casos)} casos, {len(problemas)} problema(s)")
        return 1 if problemas else 0
    except ErrorBanco as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
