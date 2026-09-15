#!/usr/bin/env python3
"""Verificador de los casos de repo del banco (ficha T02).

Deja caer la prueba de aceptación reservada en el árbol indicado, ejecuta el
comando de aceptación del caso y **devuelve el árbol como estaba**: la prueba
se borra y, si se aplicó el parche de referencia, se revierte. El candidato
nunca ve ni la prueba ni el parche: los dos viven en la raíz reservada, que
puede estar fuera del checkout.

    verificar_repo.py --caso R01 --arbol /ruta/al/arbol
    verificar_repo.py --caso R01 --arbol /ruta/al/arbol --con-referencia

`--con-referencia` aplica la solución reservada antes de medir. Sirve para
comprobar que la aceptación distingue una solución correcta de la base, no para
resolver el caso.

Salida: 0 si la aceptación pasa, 2 si falla, 1 si el caso o el árbol no se
pueden usar. El árbol se restaura pase lo que pase.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import banco  # noqa: E402

OK, FALLO, ERROR = 0, 2, 1


def git(arbol, argumentos, check=True):
    """git atado a este árbol: `--git-dir` evita que suba al repo de encima."""
    arbol = os.path.abspath(arbol)
    argv = ["git", "-C", arbol, "--git-dir", os.path.join(arbol, ".git"),
            "--work-tree", arbol, "--no-optional-locks"] + argumentos
    p = subprocess.run(argv, capture_output=True, check=False)
    if check and p.returncode != 0:
        raise banco.ErrorBanco(
            f"falló {' '.join(argumentos)}: {p.stderr.decode('utf-8', 'replace').strip()}")
    return p


def base_del_arbol(arbol):
    p = git(arbol, ["rev-parse", "HEAD"], check=False)
    return p.stdout.decode().strip() if p.returncode == 0 else None


def verificar(caso_id, arbol, reservado=None, con_referencia=False, timeout=None):
    caso = banco.caso_por_id(caso_id)
    if caso["clase"] != "repo":
        raise banco.ErrorBanco(f"{caso_id} no es un caso de repo")
    arbol = os.path.abspath(arbol)
    if not os.path.isdir(os.path.join(arbol, ".git")):
        raise banco.ErrorBanco(f"{arbol} no es un árbol git")
    raiz = banco.raiz_reservada(reservado)
    aceptacion = caso["aceptacion"]
    origen_prueba = os.path.join(raiz, caso_id, os.path.basename(aceptacion["prueba_reservada"]))
    destino_prueba = os.path.join(arbol, aceptacion["destino_prueba"])

    informe = {
        "caso": caso_id,
        "arbol": arbol,
        "base_declarada": caso["base"]["commit"],
        "base_del_arbol": base_del_arbol(arbol),
        "con_referencia": con_referencia,
        "argv": aceptacion["argv"],
    }
    if os.path.exists(destino_prueba):
        informe["estado"] = "error"
        informe["motivo"] = (f"ya existe {aceptacion['destino_prueba']} en el árbol; "
                             "el verificador no sobrescribe nada")
        return ERROR, informe

    parche = os.path.join(raiz, caso_id, "referencia.patch") if con_referencia else None
    parche_aplicado = False
    try:
        if parche:
            if not os.path.isfile(parche):
                informe["estado"] = "error"
                informe["motivo"] = f"no hay parche de referencia en {parche}"
                return ERROR, informe
            comprobacion = git(arbol, ["apply", "--check", parche], check=False)
            if comprobacion.returncode != 0:
                informe["estado"] = "error"
                informe["motivo"] = ("el parche de referencia no aplica sobre este árbol: "
                                     + comprobacion.stderr.decode("utf-8", "replace").strip())
                return ERROR, informe
            git(arbol, ["apply", parche])
            parche_aplicado = True

        os.makedirs(os.path.dirname(destino_prueba), exist_ok=True)
        shutil.copyfile(origen_prueba, destino_prueba)

        try:
            p = subprocess.run(aceptacion["argv"], cwd=arbol, capture_output=True,
                               timeout=timeout, check=False)
            informe["exit_code"] = p.returncode
            informe["salida"] = (p.stdout + p.stderr).decode("utf-8", "replace")[-8000:]
        except subprocess.TimeoutExpired as e:
            informe["exit_code"] = None
            informe["motivo"] = f"la aceptación pasó de {timeout}s"
            informe["salida"] = ((e.stdout or b"") + (e.stderr or b"")).decode("utf-8", "replace")[-8000:]
    finally:
        # El árbol se restaura aunque la aceptación reviente: un verificador que
        # deja restos convierte la siguiente medida en basura.
        if os.path.exists(destino_prueba):
            os.remove(destino_prueba)
        directorio = os.path.dirname(destino_prueba)
        if os.path.isdir(directorio) and not os.listdir(directorio):
            os.rmdir(directorio)
        if parche_aplicado:
            git(arbol, ["apply", "-R", parche], check=False)

    informe["estado"] = "ok" if informe.get("exit_code") == 0 else "fallo"
    return (OK if informe["estado"] == "ok" else FALLO), informe


def main(argv=None):
    p = argparse.ArgumentParser(prog="verificar_repo.py")
    p.add_argument("--caso", required=True)
    p.add_argument("--arbol", required=True)
    p.add_argument("--reservado", default=None)
    p.add_argument("--con-referencia", action="store_true",
                   help="aplica la solución reservada antes de medir (control)")
    p.add_argument("--timeout", type=float, default=900)
    p.add_argument("--json", default=None)
    args = p.parse_args(argv)
    try:
        codigo, informe = verificar(args.caso, args.arbol, args.reservado,
                                    args.con_referencia, args.timeout)
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
    marca = "con referencia" if informe["con_referencia"] else "tal cual"
    print(f"{args.caso} ({marca}): {informe['estado']} (exit {informe['exit_code']})")
    if informe["estado"] != "ok":
        print(informe["salida"][-2000:], file=sys.stderr)
    if informe["base_del_arbol"] != informe["base_declarada"]:
        print(f"aviso: el árbol está en {informe['base_del_arbol']}, "
              f"el caso declara {informe['base_declarada']}", file=sys.stderr)
    return codigo


if __name__ == "__main__":
    sys.exit(main())
