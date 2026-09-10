#!/usr/bin/env python3
"""Emite el juego de SASS de una arquitectura: `sass_<tag>.c` + manifiesto.

Lee los cubin ya compilados y saca de ellos el `.text` y los metadatos que
declara el propio cubin (registros, base y tamaño del banco de parámetros,
offsets de cada parámetro). Nada de constantes escritas a mano: si el `.cu`
cambia, esto cambia con él.

Cada arquitectura produce su propio fichero y su propia tabla
(`gsp_sass_set_<tag>`), así que compilar para otra arquitectura ya no
sobrescribe el juego anterior — que era el problema de R5: un único juego de
blobs, sin decir de qué arquitectura eran.
"""
import json
import os
import struct
import sys

# EIATTR que interesan (ver nvdisasm / cuobjdump).
EIATTR_REGCOUNT = 0x2F
EIATTR_PARAM_CBANK = 0x0A
EIATTR_KPARAM_INFO = 0x17


def secciones(data):
    shoff, = struct.unpack_from("<Q", data, 0x28)
    shentsize, shnum, shstrndx = struct.unpack_from("<HHH", data, 0x3A)
    shstr, = struct.unpack_from("<Q", data, shoff + shstrndx * shentsize + 0x18)
    out = {}
    for i in range(shnum):
        o = shoff + i * shentsize
        nameoff, = struct.unpack_from("<I", data, o)
        tipo, = struct.unpack_from("<I", data, o + 4)
        name = data[shstr + nameoff:data.find(b"\0", shstr + nameoff)].decode()
        off, size = struct.unpack_from("<QQ", data, o + 0x18)
        out[name] = (off, size, tipo)
    return out


def atributos(data, secs, seccion):
    off, size, _ = secs[seccion]
    b, i, out = data[off:off + size], 0, []
    while i + 2 <= len(b):
        fmt, attr = b[i], b[i + 1]
        i += 2
        if fmt == 1:                      # EIFMT_NVAL
            val = b""
        elif fmt == 2:                    # EIFMT_BVAL
            val, i = b[i:i + 1], i + 1
        elif fmt == 3:                    # EIFMT_HVAL
            val, i = b[i:i + 2], i + 2
        elif fmt == 4:                    # EIFMT_SVAL
            ln, = struct.unpack_from("<H", b, i)
            i += 2
            val, i = b[i:i + ln], i + ln
        else:
            break
        out.append((attr, val))
        # Entradas alineadas a 4 B: tras un BVAL (3 B) viene un byte de relleno.
        i = (i + 3) & ~3
    return out


def texto_kernel(data, secs):
    """La sección `.text.<kernel>` más grande: el código máquina del kernel."""
    mejor = None
    for name, (off, size, tipo) in secs.items():
        if tipo == 1 and size > 64 and name.startswith(".text"):
            if mejor is None or size > mejor[1]:
                mejor = (off, size, name)
    if mejor is None:
        sys.exit("cubin sin .text*")
    return mejor


def lee_cubin(path):
    data = open(path, "rb").read()
    if data[:4] != b"\x7fELF":
        sys.exit(f"{path}: no es un cubin ELF")
    secs = secciones(data)
    off, size, _sec = texto_kernel(data, secs)
    sass = data[off:off + size]

    info = next(n for n in secs if n.startswith(".nv.info."))
    kernel = info[len(".nv.info."):]
    regcount = next(
        struct.unpack_from("<I", v, 4)[0]
        for a, v in atributos(data, secs, ".nv.info")
        if a == EIATTR_REGCOUNT
    )
    pbase, psize = next(
        struct.unpack_from("<HH", v, 4)
        for a, v in atributos(data, secs, info)
        if a == EIATTR_PARAM_CBANK
    )
    cbank = secs[f".nv.constant0.{kernel}"][1]
    params = sorted(
        (struct.unpack_from("<H", v, 4)[0], struct.unpack_from("<H", v, 6)[0])
        for a, v in atributos(data, secs, info)
        if a == EIATTR_KPARAM_INFO
    )
    if [o for o, _ in params] != list(range(len(params))):
        sys.exit(f"{path}: ordinales de parámetro no consecutivos: {params}")
    return {
        "kernel": kernel,
        "sass": sass,
        "regcount": regcount,
        "param_base": pbase,
        "param_size": psize,
        "cbank_size": cbank,
        "param_off": [off for _, off in params],
    }


def bytes_c(blob):
    lineas = []
    for i in range(0, len(blob), 12):
        trozo = blob[i:i + 12]
        lineas.append("    " + " ".join(f"0x{b:02x}," for b in trozo))
    return "\n".join(lineas)


def main():
    if len(sys.argv) < 6:
        sys.exit("uso: l6-g4f-emit-sass.py <cubin_dir> <arch> <tag> <familia> "
                 "<salida.c> <kernel:prefijo>…")
    cubin_dir, arch, tag, familia, salida = sys.argv[1:6]
    kernels = [k.split(":", 1) for k in sys.argv[6:]]

    partes = []
    entradas = []
    manifiesto = []
    for name, prefijo in kernels:
        cubin = os.path.join(cubin_dir, f"{name}.cubin")
        k = lee_cubin(cubin)
        n = len(k["sass"])
        if n % 16 != 0:
            sys.exit(f"{name}: {n} B no es múltiplo de 16 (instrucción de {arch})")
        partes.append(
            f"/* {name} ({k['kernel']}) — {n} B, {n // 16} instrucciones */\n"
            f"static const unsigned char {prefijo}_sass_{tag}[] = {{\n"
            f"{bytes_c(k['sass'])}\n}};\n"
            f"static const unsigned {prefijo}_param_off_{tag}[] = {{ "
            + ", ".join(str(o) for o in k["param_off"])
            + " };\n"
        )
        entradas.append(
            f"    {{\n"
            f"        .name = \"{name}\",\n"
            f"        .sass = {prefijo}_sass_{tag},\n"
            f"        .sass_len = (unsigned)sizeof({prefijo}_sass_{tag}),\n"
            f"        .regcount = {k['regcount']},\n"
            f"        .param_base = {k['param_base']},\n"
            f"        .param_size = {k['param_size']},\n"
            f"        .cbank_size = {k['cbank_size']},\n"
            f"        .param_off = {prefijo}_param_off_{tag},\n"
            f"        .param_count = {len(k['param_off'])},\n"
            f"    }},\n"
        )
        manifiesto.append({
            "kernel": name,
            "simbolo": k["kernel"],
            "arch": arch,
            "bytes": n,
            "regcount": k["regcount"],
            "param_base": k["param_base"],
            "param_size": k["param_size"],
            "cbank_size": k["cbank_size"],
            "param_off": k["param_off"],
        })

    with open(salida, "w", encoding="utf-8") as f:
        f.write(
            f"/* Generado por scripts/l6-g4f-build-sass.sh para {arch} — NO EDITAR.\n"
            f" *\n"
            f" * Un juego por arquitectura: el driver elige el que corresponde a la\n"
            f" * familia detectada y rechaza lanzar si no hay ninguno compatible.\n"
            f" */\n"
            f"#include \"gsp_sass.h\"\n\n"
        )
        f.write("\n".join(partes))
        f.write(f"\nstatic const struct gsp_sass_variant vars_{tag}[] = {{\n")
        f.write("".join(entradas))
        f.write("};\n\n")
        f.write(
            f"const struct gsp_sass_set gsp_sass_set_{tag} = {{\n"
            f"    .arch = \"{arch}\",\n"
            f"    .family = {familia},\n"
            f"    .vars = vars_{tag},\n"
            f"    .count = (unsigned)(sizeof(vars_{tag}) / sizeof(vars_{tag}[0])),\n"
            f"}};\n"
        )

    man = os.path.splitext(salida)[0] + ".json"
    with open(man, "w", encoding="utf-8") as f:
        json.dump({"arch": arch, "tag": tag, "familia": familia,
                   "kernels": manifiesto}, f, ensure_ascii=False, indent=2)
        f.write("\n")
    total = sum(k["bytes"] for k in manifiesto)
    print(f"OK: {salida} ({len(manifiesto)} kernels, {total} B de SASS {arch})")
    print(f"OK: {man}")


if __name__ == "__main__":
    main()
