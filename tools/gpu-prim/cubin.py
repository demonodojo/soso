#!/usr/bin/env python3
"""Compila lxdde/ports/nouveau/gprim.cu a cubin (SASS) para sm_86 y sm_120 con NVRTC y emite
`sass_gprim_<tag>.c` con scripts/l6-g4f-emit-sass.py. Sin nvcc ni Docker (libnvrtc de pip).

Hay un juego propio (`gsp_sass_set_gprim_*`) para no tocar los blobs ya validados en silicio
de los demás kernels: `scripts/l6-g4f-build-sass.sh` sigue siendo el que los regenera.

Uso: python3 tools/gpu-prim/cubin.py          (NVRTC_LIB=/ruta/libnvrtc.so.12 para fijar la biblioteca)
"""
import ctypes, pathlib, subprocess, sys, os
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "cuda-nvrtc"))

REPO = pathlib.Path(__file__).resolve().parents[2]
SRC = REPO / "lxdde/ports/nouveau/gprim.cu"
ARCHS = [("sm_86", "gprim_sm86", "GSP_FAM_AMPERE"), ("sm_120", "gprim_sm120", "GSP_FAM_BLACKWELL")]


def buscar_nvrtc():
    import glob
    if os.environ.get("NVRTC_LIB"):
        return os.environ["NVRTC_LIB"]
    pats = [os.path.expanduser("~/.local/share/mise/installs/python/*/lib/python*/site-packages/nvidia/cuda_nvrtc/lib/libnvrtc.so.12"),
            "/home/*/Trabajo/*/*/.venv/lib/python*/site-packages/nvidia/cuda_nvrtc/lib/libnvrtc.so.12",
            "/usr/lib/x86_64-linux-gnu/libnvrtc.so.12", "/usr/local/cuda*/lib64/libnvrtc.so.12"]
    # Hace falta NVRTC >= 12.8 para sm_120 (Blackwell): se elige la versión más alta.
    mejor = None
    for p in pats:
        for c in sorted(glob.glob(p)):
            l = ctypes.CDLL(c)
            a, b = ctypes.c_int(), ctypes.c_int()
            l.nvrtcVersion(ctypes.byref(a), ctypes.byref(b))
            if mejor is None or (a.value, b.value) > mejor[0]:
                mejor = ((a.value, b.value), c)
    if mejor is None:
        sys.exit("no encuentro libnvrtc.so.12 (define NVRTC_LIB)")
    return mejor[1]


def cubin(lib, arch, texto):
    prog = ctypes.c_void_p()
    assert lib.nvrtcCreateProgram(ctypes.byref(prog), texto, b"gprim.cu", 0, None, None) == 0
    opts = [f"--gpu-architecture={arch}".encode(), b"-default-device", b"--std=c++14"]
    arr = (ctypes.c_char_p * len(opts))(*opts)
    r = lib.nvrtcCompileProgram(prog, len(opts), arr)
    n = ctypes.c_size_t()
    lib.nvrtcGetProgramLogSize(prog, ctypes.byref(n))
    log = ctypes.create_string_buffer(n.value)
    lib.nvrtcGetProgramLog(prog, log)
    if log.value.strip():
        sys.stderr.write(log.value.decode())
    if r != 0:
        sys.exit(f"nvrtcCompileProgram {arch} falló ({r})")
    lib.nvrtcGetCUBINSize(prog, ctypes.byref(n))
    buf = ctypes.create_string_buffer(n.value)
    assert lib.nvrtcGetCUBIN(prog, buf) == 0
    return buf.raw


def main():
    lib = ctypes.CDLL(buscar_nvrtc())
    texto = SRC.read_bytes()
    for arch, tag, fam in ARCHS:
        out = REPO / "target" / "gprim-sass" / arch
        out.mkdir(parents=True, exist_ok=True)
        (out / "gprim.cubin").write_bytes(cubin(lib, arch, texto))
        subprocess.run([sys.executable, str(REPO / "scripts/l6-g4f-emit-sass.py"), str(out), arch, tag, fam,
                        str(REPO / f"lxdde/ports/nouveau/sass_{tag}.c"), "gprim:gprim"], check=True)


main()
