#!/usr/bin/env python3
"""Compara PLD con/sin borrador en hostrun, sin ejecutar dos modelos a la vez."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import time


def sha256(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def disponible():
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) * 1024
    except OSError:
        pass
    return None


def cpu_host():
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or None


def ejecutar(args, activo):
    env = os.environ.copy()
    env.pop("SOSO_NO_DRAFT", None)
    env.pop("SOSO_DEBUG", None)
    if not activo:
        env["SOSO_NO_DRAFT"] = "1"
    cmd = [str(args.runner), str(args.modelo), args.prompt, str(args.tokens),
           str(args.temperatura), str(args.top_p), str(args.semilla)]
    p = subprocess.run(cmd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                       text=True, timeout=args.timeout, check=True)
    registros = [line.removeprefix("medicion: ") for line in p.stderr.splitlines()
                 if line.startswith("medicion: ")]
    if len(registros) != 1:
        raise ValueError("hostrun no produjo exactamente una medición")
    m = json.loads(registros[0])
    if m["draft_enabled"] != activo or len(m["generated_ids"]) != m["generated_tokens"]:
        raise ValueError("medición incoherente")
    return m


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("modelo", type=Path, help="directorio SOM convertido")
    p.add_argument("--runner", type=Path, default=Path("target/release/examples/hostrun"))
    p.add_argument("--prompt", default="hola hola hola hola hola hola")
    p.add_argument("--tokens", type=int, default=32)
    p.add_argument("--temperatura", type=float, default=0.0)
    p.add_argument("--top-p", type=float, default=0.9)
    p.add_argument("--semilla", type=int, default=42)
    p.add_argument("--rondas", type=int, default=5)
    p.add_argument("--calentamiento", type=int, default=1)
    p.add_argument("--timeout", type=int, default=300)
    p.add_argument("--permitir-presion-memoria", action="store_true")
    p.add_argument("--salida", type=Path, required=True)
    a = p.parse_args()
    if a.rondas < 1 or a.calentamiento < 0 or a.tokens < 1 or a.timeout < 1:
        p.error("rondas, tokens y timeout deben ser positivos; calentamiento no negativo")
    a.runner = a.runner.resolve()
    a.modelo = a.modelo.resolve()
    shards = sorted((a.modelo / "shards").rglob("*"))
    shards = [s for s in shards if s.is_file()]
    if not shards or not a.runner.is_file():
        p.error("faltan el ejecutable hostrun o los shards del modelo")
    tamanio = sum(s.stat().st_size for s in shards)
    memoria = disponible()
    if memoria is not None and tamanio > memoria * 0.7 and not a.permitir_presion_memoria:
        p.error(f"shards {tamanio} bytes, memoria disponible {memoria}: riesgo de presión de memoria; "
                "hostrun copia los shards. Usar --permitir-presion-memoria solo si procede")
    metadata = {name: sha256(a.modelo / name) for name in ("manifest.som", "index.som")}
    tokenizer = a.modelo / "tokenizer.som"
    metadata["tokenizer.som"] = sha256(tokenizer) if tokenizer.is_file() else None
    # No se leen todos los pesos antes de medir: eso alteraría la caché y puede tardar minutos.
    inventario = [{"path": str(s.relative_to(a.modelo)), "bytes": s.stat().st_size,
                   "mtime_ns": s.stat().st_mtime_ns} for s in shards]
    resultados = {"con_borrador": [], "sin_borrador": []}
    referencia = None
    for vuelta in range(a.calentamiento + a.rondas):
        for activo in ([False, True] if vuelta % 2 == 0 else [True, False]):
            medicion = ejecutar(a, activo)
            ids = medicion["generated_ids"]
            if referencia is None:
                referencia = ids
            elif ids != referencia:
                raise ValueError(f"secuencias diferentes en vuelta {vuelta}, borrador={activo}")
            if vuelta >= a.calentamiento:
                resultados["con_borrador" if activo else "sin_borrador"].append(medicion)
    resumen = {}
    for nombre, mediciones in resultados.items():
        resumen[nombre] = {
            campo: statistics.median(v) if v else None
            for campo in ("prefill_s", "ttft_s", "decode_tokens_s", "total_s", "decode_forwards")
            for v in [[m[campo] for m in mediciones if m[campo] is not None]]
        }
    salida = {
        "version": 1, "fecha_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "host": platform.platform(), "cpu": cpu_host(),
        "modelo": str(a.modelo), "runner_sha256": sha256(a.runner), "archivos_sha256": metadata,
        "inventario_shards": inventario, "memoria_disponible_bytes": memoria,
        "parametros": {"prompt": a.prompt, "max_new": a.tokens, "temperature": a.temperatura,
                       "top_p": a.top_p, "seed": a.semilla, "rondas": a.rondas,
                       "calentamiento": a.calentamiento},
        "entorno": {k: os.environ[k] for k in ("SOSO_PLANNER", "SOSO_FREE_MB", "SOSO_MEM_PRESET",
                    "SOSO_TRUNK_FRAC", "SOSO_SYNC_STAGING", "SOSO_CHAT", "SOSO_PLANTILLA", "SOSO_KV_FP4") if k in os.environ},
        "resultados": resultados, "medianas": resumen,
    }
    a.salida.parent.mkdir(parents=True, exist_ok=True)
    a.salida.write_text(json.dumps(salida, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(resumen, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as e:
        print(f"bench-inferencia: {e}", file=sys.stderr)
        sys.exit(1)
