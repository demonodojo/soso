#!/usr/bin/env python3
"""Valida el kernel de GPU de la escena de primitivas (G1) contra el backend CPU de Sal.

1. Compila tools/gpu-prim/oraculo.sal (usa escena.sal) y lo ejecuta: primitivas + píxeles CPU.
2. Compila gprim.cu a PTX con NVRTC (tools/cuda-nvrtc/compilar.py).
3. Ejecuta el PTX en la GPU del host (API de controlador CUDA) y compara píxel a píxel.

Uso: python3 tools/gpu-prim/validar.py     (requiere GPU NVIDIA y libnvrtc de pip)
"""
import ctypes, pathlib, subprocess, sys, os

REPO = pathlib.Path(__file__).resolve().parents[2]
W, H = 128, 96


def oraculo():
    sal = REPO.parent / "sal" / "target" / "release" / "sal"
    env = dict(os.environ, SAL_SDK=str(REPO / "target" / "sal" / "sdk"))
    exe = pathlib.Path("/tmp/gprim-oraculo")
    r = subprocess.run([str(sal), "build", "--release", str(REPO / "tools/gpu-prim/oraculo.sal"), "-o", str(exe)],
                       env=env, capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stderr or r.stdout)
    datos = subprocess.run([str(exe)], capture_output=True, check=True).stdout
    npix = W * H * 4
    extra = 96 + 36 * 12  # fuente sintética + celdas de la rejilla
    nprim = (len(datos) - npix - extra) // 64
    ini = nprim * 64
    return datos[:ini], datos[ini:ini + npix], nprim, datos[ini + npix:ini + npix + 96], datos[ini + npix + 96:]


def ptx():
    out = pathlib.Path("/tmp/gprim.ptx")
    subprocess.run([sys.executable, str(REPO / "tools/cuda-nvrtc/compilar.py"),
                    str(REPO / "tools/gpu-prim/gprim.cu"), str(out)], check=True)
    return out.read_bytes() + b"\0"


def gpu(prims, nprim, ptx_bytes, psf, celdas):
    cu = ctypes.CDLL("libcuda.so.1")
    def ok(r, que):
        if r != 0:
            sys.exit(f"{que}: error CUDA {r}")
    ok(cu.cuInit(0), "cuInit")
    dev = ctypes.c_int(); ok(cu.cuDeviceGet(ctypes.byref(dev), 0), "cuDeviceGet")
    ctx = ctypes.c_void_p(); ok(cu.cuCtxCreate_v2(ctypes.byref(ctx), 0, dev), "cuCtxCreate")
    mod = ctypes.c_void_p(); ok(cu.cuModuleLoadData(ctypes.byref(mod), ptx_bytes), "cuModuleLoadData")
    fn = ctypes.c_void_p(); ok(cu.cuModuleGetFunction(ctypes.byref(fn), mod, b"gprim"), "cuModuleGetFunction")
    d_rt = ctypes.c_uint64(); ok(cu.cuMemAlloc_v2(ctypes.byref(d_rt), W * H * 4), "cuMemAlloc rt")
    ok(cu.cuMemsetD8_v2(d_rt, 0, W * H * 4), "memset")
    d_pr = ctypes.c_uint64(); ok(cu.cuMemAlloc_v2(ctypes.byref(d_pr), len(prims)), "cuMemAlloc prims")
    ok(cu.cuMemcpyHtoD_v2(d_pr, prims, len(prims)), "HtoD")
    d_ce = ctypes.c_uint64(); ok(cu.cuMemAlloc_v2(ctypes.byref(d_ce), len(celdas)), "cuMemAlloc celdas")
    ok(cu.cuMemcpyHtoD_v2(d_ce, celdas, len(celdas)), "HtoD celdas")
    d_ps = ctypes.c_uint64(); ok(cu.cuMemAlloc_v2(ctypes.byref(d_ps), len(psf)), "cuMemAlloc psf")
    ok(cu.cuMemcpyHtoD_v2(d_ps, psf, len(psf)), "HtoD psf")
    args = [ctypes.c_uint64(d_rt.value), ctypes.c_int(W), ctypes.c_int(W), ctypes.c_int(H), ctypes.c_int(1),
            ctypes.c_uint64(d_pr.value), ctypes.c_int(nprim),
            ctypes.c_uint64(d_ce.value), ctypes.c_uint64(d_ps.value)]
    ptrs = (ctypes.c_void_p * len(args))(*[ctypes.cast(ctypes.pointer(a), ctypes.c_void_p) for a in args])
    ok(cu.cuLaunchKernel(fn, (W + 15) // 16, (H + 15) // 16, 1, 16, 16, 1, 0, None, ptrs, None), "launch")
    ok(cu.cuCtxSynchronize(), "sync")
    buf = ctypes.create_string_buffer(W * H * 4)
    ok(cu.cuMemcpyDtoH_v2(buf, d_rt, W * H * 4), "DtoH")
    return buf.raw


def main():
    prims, cpu, n, psf, celdas = oraculo()
    g = gpu(prims, n, ptx(), psf, celdas)
    malos = [(i % W, i // W) for i in range(W * H) if cpu[i * 4:i * 4 + 4] != g[i * 4:i * 4 + 4]]
    print(f"{n} primitivas, {W}x{H}: {len(malos)} píxeles distintos entre CPU (Sal) y GPU")
    for x, y in malos[:8]:
        o = (y * W + x) * 4
        print(f"  ({x},{y}) cpu={cpu[o:o+4].hex()} gpu={g[o:o+4].hex()}")
    return 1 if malos else 0


if __name__ == "__main__":
    raise SystemExit(main())
