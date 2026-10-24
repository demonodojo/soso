#!/usr/bin/env python3
"""Genera los fixtures de dequantización: bloques reales de un GGUF + su f32 según libggml (oráculo C).

Uso: generar_fixtures.py MODELO.gguf ORACULO_BIN DESTINO [BLOQUES_POR_MUESTRA=24]
Para cada tipo se toman dos muestras (el primer y el último tensor de ese tipo, desde un desplazamiento fijo) para cubrir escalas y signos variados.
"""
import os, struct, subprocess, sys

TIPOS = {'Q3_K': 11, 'Q5_K': 13, 'IQ2_XS': 17, 'IQ3_XXS': 18, 'IQ3_S': 21, 'IQ2_S': 22, 'IQ4_XS': 23}
BLOQUE = {'Q3_K': (256, 110), 'Q5_K': (256, 176), 'IQ2_XS': (256, 74), 'IQ3_XXS': (256, 98), 'IQ3_S': (256, 110), 'IQ2_S': (256, 82), 'IQ4_XS': (256, 136)}
gguf, oraculo, destino = sys.argv[1], sys.argv[2], sys.argv[3]
nb_muestra = int(sys.argv[4]) if len(sys.argv) > 4 else 24
os.makedirs(destino, exist_ok=True)
f = open(gguf, 'rb')
def r(fmt):
    n = struct.calcsize(fmt); return struct.unpack(fmt, f.read(n))
def s():
    n, = r('<Q'); return f.read(n).decode('utf8', 'replace')
def val(t):
    if t == 8: return s()
    if t == 9:
        et, = r('<I'); n, = r('<Q'); return [val(et) for _ in range(n)]
    return r({0: '<B', 1: '<b', 2: '<H', 3: '<h', 4: '<I', 5: '<i', 6: '<f', 7: '<?', 10: '<Q', 11: '<q', 12: '<d'}[t])[0]
assert f.read(4) == b'GGUF'
ver, = r('<I'); nt, = r('<Q'); nkv, = r('<Q')
kv = {}
for _ in range(nkv):
    k = s(); t, = r('<I'); kv[k] = val(t)
tens = []
for _ in range(nt):
    name = s(); nd, = r('<I'); dims = r('<' + 'Q' * nd); ty, = r('<I'); off, = r('<Q'); tens.append((name, dims, ty, off))
al = int(kv.get('general.alignment', 32))
inicio = (f.tell() + al - 1) // al * al
for nombre, ty in TIPOS.items():
    cand = [t for t in tens if t[2] == ty]
    if not cand:
        print(f'{nombre}: no hay tensores de este tipo en {gguf}'); continue
    ebloque, bbytes = BLOQUE[nombre]
    crudo = b''
    for t in (cand[0], cand[-1]):
        n = 1
        for d in t[1]: n *= d
        total_bloques = n // ebloque
        ini = min(7, max(total_bloques - nb_muestra, 0))
        f.seek(inicio + t[3] + ini * bbytes)
        crudo += f.read(nb_muestra * bbytes)
        print(f'  {nombre}: muestra de {t[0]} (bloques {ini}..{ini + nb_muestra} de {total_bloques})')
    base = os.path.join(destino, nombre)
    open(base + '.bin', 'wb').write(crudo)
    nb = len(crudo) // bbytes
    subprocess.run([oraculo, str(ty), base + '.bin', base + '.f32', str(nb)], check=True)
