#!/usr/bin/env python3
"""Genera lxdde/ports/nouveau/iq_tablas.cuh (tablas de los i-quants para CUDA) a partir de crates/sosomodel/src/iq_tablas.rs."""
import re, sys
src = open(sys.argv[1]).read()
out = ["// Generado por tools/cuda-nvrtc/generar_tablas_cuh.py desde iq_tablas.rs (ggml-common.h, MIT). No editar.\n"]
cty = {"u8": "unsigned char", "i8": "signed char", "u32": "unsigned int", "u64": "unsigned long long"}
for m in re.finditer(r"pub static (\w+): \[(\w+); (\d+)\] = \[(.*?)\];", src, re.S):
    nombre, ty, n, cuerpo = m.groups()
    vals = [v.strip() for v in cuerpo.replace("\n", " ").split(",") if v.strip()]
    assert len(vals) == int(n), (nombre, len(vals), n)
    suf = "ULL" if ty == "u64" else ("u" if ty == "u32" else "")
    out.append(f"__device__ const {cty[ty]} {nombre}_D[{n}] = {{\n" + ",".join(v + suf for v in vals) + "\n};\n")
open(sys.argv[2], "w").write("".join(out))
