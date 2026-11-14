#!/usr/bin/env python3
"""Compara el rasterizador TTF de Sal (user/sal/escritorio/ttf.sal) con FreeType.

Compila tools/ttf-prueba/volcar.sal, lo ejecuta en modo celda (DejaVu Sans Mono a
celdas de 8x16 y 12x24) y libre (DejaVu Sans a 13 y 20 px) y deja PNG en --out con la
imagen de Sal arriba y la de FreeType (Pillow, con hinting) debajo. Imprime la tinta
total de cada una: deberían diferir poco (el hinting mueve bordes, no la cantidad).

    python3 tools/ttf-prueba/comparar.py [--out DIR]
"""
import argparse, os, pathlib, struct, subprocess, sys
from PIL import Image, ImageDraw, ImageFont

REPO = pathlib.Path(__file__).resolve().parents[2]
MONO = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
SANS = str(REPO / "rootfs/lib/fonts/DejaVuSans.ttf")
LINEAS = ["jmdiez@soso:~$ ls -la /etc", "Ventana: «Terminal» — ñandú, acción",
          "┌──┬──┐ █▓▒░ {}[]() => != gjpqy Il1|O0", "└──┴──┘ AVATAR Tava WAVE ¿¡ áéíóú ÁÉÍÓÚ"]


def compilar(out):
    sal = REPO.parent / "sal" / "target" / "release" / "sal"
    env = dict(os.environ, SAL_SDK=str(REPO / "target/sal/sdk"))
    r = subprocess.run([str(sal), "build", "--release", str(REPO / "tools/ttf-prueba/volcar.sal"), "-o", str(out)],
                       env=env, capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stderr or r.stdout)


def sal(exe, *args):
    d = subprocess.run([str(exe), *map(str, args)], capture_output=True, check=True).stdout
    w, h = struct.unpack_from("<II", d, 0)
    return Image.frombytes("L", (w, h), d[8:8 + w * h])


def freetype_celda(gw, gh):
    # Misma regla de tamaño que ttf_cache_celda: el avance de «M» llena la celda.
    f = ImageFont.truetype(MONO, 100)
    px = 100 * gw / f.getlength("M")
    f = ImageFont.truetype(MONO, round(px))
    img = Image.new("L", (40 * gw, 4 * gh), 0)
    d = ImageDraw.Draw(img)
    asc, desc = f.getmetrics()
    for r, l in enumerate(LINEAS):
        for c, ch in enumerate(l[:40]):
            d.text((c * gw, r * gh + (gh - asc - desc) // 2 + asc), ch, font=f, fill=255, anchor="ls")
    return img


def freetype_libre(px, w):
    f = ImageFont.truetype(SANS, px)
    asc, desc = f.getmetrics()
    img = Image.new("L", (w, 4 * (asc + desc)), 0)
    d = ImageDraw.Draw(img)
    for r, l in enumerate(LINEAS):
        d.text((2, r * (asc + desc) + asc), l, font=f, fill=255, anchor="ls")
    return img


def apilar(a, b):
    w = max(a.width, b.width)
    img = Image.new("L", (w, a.height + b.height + 4), 128)
    img.paste(a, (0, 0))
    img.paste(b, (0, a.height + 4))
    return img


def tinta(img):
    return sum(img.tobytes()) / 255


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(REPO / "target/ttf-prueba"))
    a = ap.parse_args()
    out = pathlib.Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    exe = out / "volcar"
    compilar(exe)
    for gw, gh in [(8, 16), (12, 24)]:
        s = sal(exe, MONO, "celda", gw, gh)
        f = freetype_celda(gw, gh)
        apilar(s, f).save(out / f"celda-{gw}x{gh}.png")
        print(f"celda {gw}x{gh}: tinta sal {tinta(s):.0f}, freetype {tinta(f):.0f}")
    for px in [13, 20]:
        s = sal(exe, SANS, "libre", px, 0)
        f = freetype_libre(px, s.width)
        apilar(s, f).save(out / f"libre-{px}.png")
        print(f"libre {px}px: tinta sal {tinta(s):.0f}, freetype {tinta(f):.0f}")
    print(f"imágenes en {out}")


if __name__ == "__main__":
    main()
