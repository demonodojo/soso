#!/usr/bin/env python3
"""Verificador de los casos de programación del banco (ficha T02).

Compila el archivo candidato con `rustc` y lo ejecuta contra cada vector
reservado en un directorio de trabajo desechable. Un caso pasa solo si **todos**
sus vectores coinciden byte a byte en stdout y en código de salida.

Los vectores viven en la raíz reservada, que por defecto está dentro del banco
pero se mueve fuera del checkout del agente con `--reservado` o
`SOSO_BANCO_RESERVADO`. El candidato nunca los ve: los recibe ya ejecutados en
forma de veredicto.

    verificar_programa.py --caso P01 --candidato solucion.rs [--json informe.json]

Salida: 0 si pasa, 2 si falla algún vector, 1 si el caso o el candidato no se
pueden usar (no compila, falta el archivo, no hay rustc).
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import banco  # noqa: E402  (se carga tras fijar la ruta del banco)

OK, FALLO, ERROR = 0, 2, 1


def compilar(fuente, destino, timeout):
    argv = ["rustc", "--edition", "2021", "-O", "-o", destino, fuente]
    try:
        p = subprocess.run(argv, capture_output=True, timeout=timeout, check=False)
    except FileNotFoundError:
        return None, "rustc no está disponible"
    except subprocess.TimeoutExpired:
        return None, f"la compilación pasó de {timeout}s"
    if p.returncode != 0:
        return None, p.stderr.decode("utf-8", "replace").strip()
    return destino, None


def preparar_trabajo(directorio, vector):
    for rel in vector.get("directorios", []):
        os.makedirs(os.path.join(directorio, rel), exist_ok=True)
    for rel, contenido in vector.get("archivos", {}).items():
        ruta = os.path.join(directorio, rel)
        os.makedirs(os.path.dirname(ruta), exist_ok=True)
        with open(ruta, "w", encoding="utf-8") as f:
            f.write(contenido)


def ejecutar_vector(binario, vector, timeout, trabajo):
    """Ejecuta un vector en su propio directorio limpio."""
    directorio = tempfile.mkdtemp(prefix="vector-", dir=trabajo)
    preparar_trabajo(directorio, vector)
    try:
        p = subprocess.run(
            [binario],
            cwd=directorio,
            input=vector["stdin"].encode(),
            capture_output=True,
            timeout=timeout,
            check=False,
        )
        salida = p.stdout.decode("utf-8", "replace")
        codigo = p.returncode
        motivo = None
    except subprocess.TimeoutExpired:
        salida, codigo, motivo = "", None, f"pasó de {timeout}s"
    esperado = vector["stdout"]
    esperado_exit = vector.get("exit", 0)
    paso = motivo is None and salida == esperado and codigo == esperado_exit
    return {
        "vector": vector["nombre"],
        "paso": paso,
        "esperado": esperado,
        "obtenido": salida,
        "exit_esperado": esperado_exit,
        "exit": codigo,
        "motivo": motivo,
        "stderr": p.stderr.decode("utf-8", "replace")[-2000:] if motivo is None else "",
    }


def verificar(caso_id, candidato, reservado=None, timeout=30):
    caso = banco.caso_por_id(caso_id)
    if caso["clase"] != "programacion":
        raise banco.ErrorBanco(f"{caso_id} no es un caso de programación")
    raiz = banco.raiz_reservada(reservado)
    with open(os.path.join(raiz, caso_id, "vectores.json"), encoding="utf-8") as f:
        vectores = json.load(f)["vectores"]

    informe = {"caso": caso_id, "candidato": os.path.abspath(candidato), "vectores": []}
    if not os.path.isfile(candidato):
        informe["estado"] = "error"
        informe["motivo"] = f"no existe el candidato {candidato}"
        return ERROR, informe

    trabajo = tempfile.mkdtemp(prefix=f"banco-{caso_id}-")
    try:
        fuente = os.path.join(trabajo, "candidato.rs")
        shutil.copyfile(candidato, fuente)
        binario, error = compilar(fuente, os.path.join(trabajo, "candidato"), timeout * 4)
        if binario is None:
            informe["estado"] = "error"
            informe["motivo"] = "no compila"
            informe["compilacion"] = error
            return ERROR, informe
        for vector in vectores:
            informe["vectores"].append(ejecutar_vector(binario, vector, timeout, trabajo))
    finally:
        shutil.rmtree(trabajo, ignore_errors=True)

    fallados = [v for v in informe["vectores"] if not v["paso"]]
    informe["estado"] = "ok" if not fallados else "fallo"
    informe["pasados"] = len(informe["vectores"]) - len(fallados)
    informe["total"] = len(informe["vectores"])
    return (OK if not fallados else FALLO), informe


def main(argv=None):
    p = argparse.ArgumentParser(prog="verificar_programa.py")
    p.add_argument("--caso", required=True)
    p.add_argument("--candidato", required=True)
    p.add_argument("--reservado", default=None,
                   help="raíz del material reservado (por defecto, la del banco "
                        f"o ${banco.ENV_RESERVADO})")
    p.add_argument("--timeout", type=float, default=30)
    p.add_argument("--json", default=None, help="escribe el informe completo aquí")
    args = p.parse_args(argv)
    try:
        codigo, informe = verificar(args.caso, args.candidato, args.reservado, args.timeout)
    except banco.ErrorBanco as e:
        print(f"error: {e}", file=sys.stderr)
        return ERROR
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump(informe, f, indent=2, ensure_ascii=False)
            f.write("\n")
    if informe["estado"] == "error":
        print(f"{args.caso}: ERROR — {informe['motivo']}", file=sys.stderr)
        if informe.get("compilacion"):
            print(informe["compilacion"], file=sys.stderr)
        return codigo
    print(f"{args.caso}: {informe['estado']} ({informe['pasados']}/{informe['total']} vectores)")
    for v in informe["vectores"]:
        if v["paso"]:
            continue
        print(f"  vector «{v['vector']}»: esperado {v['esperado']!r} exit {v['exit_esperado']}; "
              f"obtenido {v['obtenido']!r} exit {v['exit']}"
              + (f" ({v['motivo']})" if v["motivo"] else ""), file=sys.stderr)
    return codigo


if __name__ == "__main__":
    sys.exit(main())
