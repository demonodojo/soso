#!/usr/bin/env python3
"""Compila un .cu a PTX con NVRTC (libnvrtc.so.12 del paquete `nvidia-cuda-nvrtc` de pip), sin nvcc ni Docker.

Uso: compilar.py KERNEL.cu SALIDA.ptx [-I DIR]...   (NVRTC_LIB=/ruta/libnvrtc.so.12 para fijar la biblioteca)
NVRTC no trae las cabeceras de CUDA (cuda_fp16.h…): los kernels que se compilen así no deben incluirlas.
"""
import ctypes, glob, os, sys

def buscar():
    if os.environ.get("NVRTC_LIB"):
        return os.environ["NVRTC_LIB"]
    pats = [os.path.expanduser("~/.local/share/mise/installs/python/*/lib/python*/site-packages/nvidia/cuda_nvrtc/lib/libnvrtc.so.12"),
            "/usr/lib/x86_64-linux-gnu/libnvrtc.so.12", "/usr/local/cuda*/lib64/libnvrtc.so.12"]
    for p in pats:
        r = sorted(glob.glob(p))
        if r:
            return r[-1]
    sys.exit("no encuentro libnvrtc.so.12 (define NVRTC_LIB)")

def main():
    a = sys.argv[1:]
    if len(a) < 2:
        sys.exit(__doc__)
    src, out, incs = a[0], a[1], []
    i = 2
    while i < len(a):
        if a[i] == "-I":
            incs.append(a[i + 1]); i += 2
        else:
            i += 1
    lib = ctypes.CDLL(buscar())
    prog = ctypes.c_void_p()
    texto = open(src).read().encode()
    # NVRTC no resuelve #include de ficheros: se inlinean las inclusiones locales.
    import re
    def inlinear(t, base):
        def sub(m):
            for d in [base] + incs:
                p = os.path.join(d, m.group(1))
                if os.path.exists(p):
                    return inlinear(open(p).read(), os.path.dirname(p))
            sys.exit("no encuentro " + m.group(1))
        return re.sub(r'#include "([^"]+)"', sub, t)
    texto = inlinear(texto.decode(), os.path.dirname(os.path.abspath(src))).encode()
    r = lib.nvrtcCreateProgram(ctypes.byref(prog), texto, os.path.basename(src).encode(), 0, None, None)
    assert r == 0, f"nvrtcCreateProgram {r}"
    opts = [b"--gpu-architecture=compute_80", b"-default-device", b"--std=c++14"]
    arr = (ctypes.c_char_p * len(opts))(*opts)
    r = lib.nvrtcCompileProgram(prog, len(opts), arr)
    n = ctypes.c_size_t()
    lib.nvrtcGetProgramLogSize(prog, ctypes.byref(n))
    log = ctypes.create_string_buffer(n.value)
    lib.nvrtcGetProgramLog(prog, log)
    if log.value.strip():
        sys.stderr.write(log.value.decode())
    if r != 0:
        sys.exit(f"nvrtcCompileProgram falló ({r})")
    lib.nvrtcGetPTXSize(prog, ctypes.byref(n))
    ptx = ctypes.create_string_buffer(n.value)
    lib.nvrtcGetPTX(prog, ptx)
    open(out, "wb").write(ptx.value + b"\0" if False else ptx.value)
    print(f"{out}: {len(ptx.value)} bytes")

main()
