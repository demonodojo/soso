#!/usr/bin/env python3
"""Compila un .cu a PTX o cubin con NVRTC, sin nvcc ni GPU.

Uso: compilar.py KERNEL.cu SALIDA [-I DIR]... [--arch compute_80|sm_86] [--cubin]
NVRTC_LIB fija la biblioteca. Para includes CUDA se necesita un directorio de cabeceras con -I.
"""
import argparse, ctypes, glob, os, sys

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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("src")
    parser.add_argument("out")
    parser.add_argument("-I", dest="incs", action="append", default=[])
    parser.add_argument("--arch", default="compute_80")
    parser.add_argument("--cubin", action="store_true")
    args = parser.parse_args()
    src, out, incs = args.src, args.out, args.incs
    if args.cubin and not args.arch.startswith("sm_"):
        parser.error("--cubin requiere --arch sm_NN")
    if not args.cubin and not args.arch.startswith("compute_"):
        parser.error("PTX requiere --arch compute_NN")
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
    opts = [f"--gpu-architecture={args.arch}".encode(), b"-default-device", b"--std=c++14"]
    opts.extend(f"-I{d}".encode() for d in incs)
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
    get_size = lib.nvrtcGetCUBINSize if args.cubin else lib.nvrtcGetPTXSize
    get_data = lib.nvrtcGetCUBIN if args.cubin else lib.nvrtcGetPTX
    r = get_size(prog, ctypes.byref(n))
    if r != 0:
        sys.exit(f"nvrtcGetSize falló ({r})")
    buf = ctypes.create_string_buffer(n.value)
    r = get_data(prog, buf)
    if r != 0:
        sys.exit(f"nvrtcGetData falló ({r})")
    data = buf.raw if args.cubin else buf.value
    with open(out, "wb") as f:
        f.write(data)
    lib.nvrtcDestroyProgram(ctypes.byref(prog))
    print(f"{out}: {len(data)} bytes ({args.arch})")

main()
