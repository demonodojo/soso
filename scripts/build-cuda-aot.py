#!/usr/bin/env python3
"""Compila los tres kernels host CUDA a cubin con NVRTC, sin GPU.

Ejemplo: python3 scripts/build-cuda-aot.py --arch sm_86 --include /ruta/cuda/include
Se pueden repetir --include; se requieren cuda_fp16.h y crt/mma.h compatibles.
"""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
KERNELS = ("matvec_q4_0", "matvec_iq", "gemma_capa")
FUENTES = ("matvec_q4_0.cu", "matvec_iq.cu", "gemma_capa.cu", "q4k_decode.h", "iq_tablas.cuh")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--arch", required=True)
    p.add_argument("--include", action="append", default=[])
    p.add_argument("--out", type=Path)
    a = p.parse_args()
    if not a.arch.startswith("sm_") or not a.arch[3:].isdigit():
        p.error("--arch debe tener la forma sm_NN")
    out = a.out or ROOT / "target" / "cuda-aot" / a.arch
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".aot-", dir=out) as tmp:
        fuentes = Path(tmp) / "fuentes"
        fuentes.mkdir()
        for nombre in FUENTES:
            shutil.copyfile(ROOT / "lxdde/ports/nouveau" / nombre, fuentes / nombre)
        for kernel in KERNELS:
            cmd = [sys.executable, str(ROOT / "tools/cuda-nvrtc/compilar.py"),
                   str(fuentes / f"{kernel}.cu"),
                   str(Path(tmp) / f"{kernel}.cubin"), "--arch", a.arch, "--cubin"]
            for include in a.include:
                cmd.extend(["-I", include])
            subprocess.run(cmd, check=True)
        for kernel in KERNELS:
            os.replace(Path(tmp) / f"{kernel}.cubin", out / f"{kernel}.cubin")
        lineas = [f"arch {a.arch}"]
        lineas.extend(f"source {f} {sha(fuentes / f)}" for f in FUENTES)
        lineas.extend(f"cubin {k}.cubin {sha(out / (k + '.cubin'))}" for k in KERNELS)
        (Path(tmp) / "manifest.txt").write_text("\n".join(lineas) + "\n")
        os.replace(Path(tmp) / "manifest.txt", out / "manifest.txt")
    print(f"cubins en {out}; para incorporarlos: SOSO_CUDA_AOT_DIR={out.resolve()} SOSO_CUDA_AOT_SM={a.arch} cargo build -p soso-llm-api --features std --example serve_host")


if __name__ == "__main__":
    main()
