#!/usr/bin/env python3
"""Mono del salvapantallas de soso-wm: pixel art de 32x32, 10 fotogramas.

0..3 andar, 4 quieto, 5 parpadeo, 6 y 7 saludo, 8 y 9 martillazo. Las siluetas salen de discos,
elipses y trazos de píxel; el contorno y el sombreado se añaden después y la
cara se escribe a mano (CARA). Uso:

    python3 scripts/mono-pixel.py previa.png          # vista previa x8
    python3 scripts/mono-pixel.py previa.png --sal    # cuerpo de mono_datos

El resultado va a `mono_datos` en user/sal/escritorio/mono.sal (la paleta,
a `mono_color`).
"""
import sys
from PIL import Image

PAL = {
    '.': None,
    'k': (44, 26, 18),     # contorno
    'D': (96, 54, 30),     # pelo sombra
    'B': (140, 84, 44),    # pelo
    'L': (178, 116, 66),   # pelo luz
    'S': (246, 208, 158),  # piel
    's': (214, 166, 116),  # piel sombra
    'p': (230, 140, 124),  # rosa (oreja, mejilla)
    'W': (255, 255, 255),
    'G': (168, 176, 190),  # martillo
    'g': (104, 110, 124),  # martillo sombra
    'h': (196, 138, 76),   # mango
    'o': (0, 0, 0),        # sombra del suelo (con alfa)
}


G = 32


class L:
    def __init__(s):
        s.c = [['.'] * G for _ in range(G)]
        s.p = [[0] * G for _ in range(G)]
    def put(s, x, y, ch, parte=1):
        if 0 <= x < G and 0 <= y < G:
            s.c[y][x] = ch; s.p[y][x] = parte

def disco(l, cx, cy, r, ch, parte=1):
    for y in range(G):
        for x in range(G):
            if (x - cx) ** 2 + (y - cy) ** 2 <= r * r + r * 0.6:
                l.put(x, y, ch, parte)

def elip(l, cx, cy, rx, ry, ch, parte=1):
    for y in range(G):
        for x in range(G):
            if ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2 <= 1.0:
                l.put(x, y, ch, parte)

def linea(l, pts, ch, grosor, parte):
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        n = max(abs(x1 - x0), abs(y1 - y0), 1)
        for i in range(n + 1):
            x = round(x0 + (x1 - x0) * i / n); y = round(y0 + (y1 - y0) * i / n)
            for dx in range(grosor):
                for dy in range(grosor):
                    l.put(x + dx, y + dy, ch, parte)

CARA = [  # columnas 10..23, filas 5..15 ('.' no toca)
    "...SS...SS....",
    "..SSSS.SSSS...",
    "..SWWkSSWWkS..",
    "..SWWkSSWWkS..",
    "..SSSSSSSSSSS.",
    ".SSSSSkSkSSSS.",
    ".pSSSSSSSSSSp.",
    ".SSkSSSSSSkSS.",
    ".SSSkkkkkkSSS.",
    "..SSSSSSSSSS..",
    "....ssssss....",
]
PARPADEO = {7: "..SSSSSSSSSS..", 8: "..kkkSSSkkkS.."}

# Piernas, brazos y cola por fotograma: 0..3 andar, 4 quieto, 5 parpadeo, 6-7 saludo.
def pose(f):
    piernas = {  # (atrás, delante): lista de puntos cadera→tobillo
        0: ([(14, 25), (14, 28)], [(17, 25), (17, 28)]),
        1: ([(14, 25), (12, 28)], [(17, 25), (19, 28)]),
        2: ([(14, 25), (14, 28)], [(17, 25), (17, 28)]),
        3: ([(14, 25), (16, 28)], [(17, 25), (15, 28)]),
    }.get(f, ([(14, 25), (14, 28)], [(17, 25), (17, 28)]))
    brazos = {  # (atrás, delante)
        0: ([(10, 18), (10, 22)], [(21, 18), (21, 22)]),
        1: ([(10, 18), (12, 22)], [(21, 18), (20, 22)]),
        2: ([(10, 18), (10, 22)], [(21, 18), (21, 22)]),
        3: ([(10, 18), (8, 21)], [(21, 18), (24, 21)]),
        6: ([(10, 18), (10, 22)], [(21, 18), (25, 16), (27, 13)]),
        7: ([(10, 18), (10, 22)], [(21, 18), (24, 15), (25, 12)]),
        8: ([(10, 18), (9, 21)], [(21, 18), (25, 15), (26, 11)]),
        9: ([(10, 18), (12, 21)], [(21, 18), (24, 20), (26, 23)]),
    }.get(f, ([(10, 18), (10, 22)], [(21, 18), (21, 22)]))
    cola = {0: 0, 1: 1, 2: 0, 3: -1}.get(f, 0)
    bote = -1 if f in (1, 3) else 0
    return piernas, brazos, cola, bote

def fotograma(f):
    l = L()
    piernas, brazos, cola, bote = pose(f)
    # cola: sale de la grupa, baja, sube por la izquierda y se enrosca
    t = [(11, 23), (8, 24), (6, 23), (5, 21), (5, 18), (6, 16 + cola), (8, 15 + cola), (9, 16 + cola), (9, 18 + cola)]
    linea(l, t, 'B', 1, 1)
    # lado de atrás
    pa, pd = piernas
    linea(l, pa, 'D', 2, 2)
    l.put(pa[-1][0] - 1, 29, 's', 2); l.put(pa[-1][0], 29, 's', 2); l.put(pa[-1][0] + 1, 29, 's', 2); l.put(pa[-1][0] + 2, 29, 's', 2)
    ba, bd = brazos
    linea(l, ba, 'D', 2, 2)
    hx, hy = ba[-1]; l.put(hx, hy + 2, 's', 2); l.put(hx + 1, hy + 2, 's', 2)
    # cuerpo
    elip(l, 15.5, 21 + bote, 5.6, 5.2, 'B', 3)
    elip(l, 16.5, 22 + bote, 3.1, 3.6, 'S', 3)
    linea(l, pd, 'B', 2, 4)
    for dx in range(-1, 3): l.put(pd[-1][0] + dx + 1, 29, 'S', 4)
    # cabeza (y orejas)
    hy0 = bote
    for ox in (8, 25):
        disco(l, ox, 10 + hy0, 2.3, 'B', 5)
    disco(l, 16.5, 9 + hy0, 7.3, 'B', 5)
    l.put(16, 1 + hy0, 'B', 5); l.put(17, 0 + hy0, 'B', 5)
    for ox in (8, 25):
        l.put(ox, 10 + hy0, 'p', 5); l.put(ox, 11 + hy0, 'p', 5)
    # brazo de delante (encima de todo)
    linea(l, bd, 'B', 2, 6)
    hx, hy = bd[-1]
    if f in (6, 7, 8):
        for dx in (0, 1, 2):
            for dy in (-2, -1):
                l.put(hx + dx, hy + dy, 'S', 6)
    elif f == 9:
        for dx in (0, 1, 2):
            for dy in (0, 1):
                l.put(hx + dx, hy + dy, 'S', 6)
    else:
        for dx in (0, 1, 2):
            for dy in (2, 3):
                l.put(hx + dx, hy + dy, 'S', 6)
    # martillo: arriba, listo para golpear (8); contra el suelo (9)
    if f == 8:
        linea(l, [(27, 8), (27, 4)], 'h', 1, 7)
        for x in range(24, 31):
            for y in range(1, 4):
                l.put(x, y, 'g' if y == 3 else 'G', 7)
    if f == 9:
        linea(l, [(27, 25), (27, 26)], 'h', 1, 7)
        for x in range(24, 31):
            for y in range(27, 30):
                l.put(x, y, 'g' if y == 29 else 'G', 7)
    return l, bote

# Píxel del dibujo donde pega el martillo (fotograma 9).
IMPACTO = (27, 30)

def acabar(l, f, bote):
    c, p = l.c, l.p
    # contorno exterior
    n = [r[:] for r in c]
    for y in range(G):
        for x in range(G):
            if c[y][x] == '.':
                for nx, ny in ((x+1,y),(x-1,y),(x,y+1),(x,y-1)):
                    if 0 <= nx < G and 0 <= ny < G and c[ny][nx] != '.':
                        n[y][x] = 'k'; break
    c = n
    # línea interior que separa el brazo de delante de lo de detrás
    n = [r[:] for r in c]
    for y in range(G):
        for x in range(G):
            if p[y][x] in (3, 5) and c[y][x] in 'BS':
                for nx, ny in ((x+1,y),(x-1,y),(x,y+1),(x,y-1)):
                    if 0 <= nx < G and 0 <= ny < G and p[ny][nx] == 6 and c[ny][nx] != 'k':
                        n[y][x] = 'D' if c[y][x] == 'B' else 's'; break
    c = n
    # luz arriba-izquierda, sombra abajo-derecha en el pelo
    n = [r[:] for r in c]
    for y in range(G):
        for x in range(G):
            if c[y][x] == 'B':
                if (y + 1 < G and c[y+1][x] == 'k') or (x + 1 < G and c[y][x+1] == 'k'):
                    n[y][x] = 'D'
                elif (c[y-1][x] == 'k' or c[y][x-1] == 'k') and p[y][x] in (5,):
                    n[y][x] = 'L'
    c = n
    # cara
    for j, fila in enumerate(CARA):
        if f in (5, 9) and j + 5 in PARPADEO:
            fila = PARPADEO[j + 5]
        for i, ch in enumerate(fila):
            if ch != '.':
                y = 5 + j + bote
                if 0 <= y < G:
                    c[y][10 + i] = ch
    # chispas del golpe
    if f == 9:
        for x, y in ((22, 26), (31, 25), (22, 30), (29, 24)):
            if c[y][x] == '.':
                c[y][x] = 'W'
    # sombra del suelo
    for x in range(9, 23):
        if c[31][x] == '.' and c[30][x] in '.k':
            c[31][x] = 'o'
    l.c = c

def generar():
    fr = []
    for f in range(10):
        l, b = fotograma(f)
        acabar(l, f, b)
        fr.append([''.join(r) for r in l.c])
    return fr

def sal(fr):
    """Cuerpo de mono_datos para user/sal/escritorio/mono.sal."""
    out = []
    for k, rows in enumerate(fr):
        kw = 'if' if k == 0 else ('elsif' if k < len(fr) - 1 else 'else')
        cond = f' f == {k}' if kw != 'else' else ''
        out.append(f'    {kw}{cond}\n        d"{"".join(rows)}"')
    return '\n'.join(out)


def previa(fr, ruta, k=8):
    im = Image.new('RGB', (G * k * len(fr) + (len(fr) - 1) * 8, G * k), (70, 90, 120))
    for i, rows in enumerate(fr):
        for y, r in enumerate(rows):
            for x, ch in enumerate(r):
                col = PAL[ch]
                if col is None: continue
                if ch == 'o': col = (49, 63, 84)
                for yy in range(k):
                    for xx in range(k):
                        im.putpixel((i * (G * k + 8) + x * k + xx, y * k + yy), col)
    im.save(ruta)


if __name__ == '__main__':
    fr = generar()
    previa(fr, sys.argv[1])
    if '--sal' in sys.argv:
        print(sal(fr))
    else:
        for r in fr[4]:
            print(r)
