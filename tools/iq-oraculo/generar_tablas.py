#!/usr/bin/env python3
"""Extrae las tablas de cuantización IQ de `ggml-common.h` (llama.cpp, MIT) a `crates/soso-llm-core/src/iq_tablas.rs`.

Uso: generar_tablas.py RUTA/ggml-common.h SALIDA.rs
"""
import re, sys
src = open(sys.argv[1]).read()
TABLAS = [  # (nombre en C, nombre Rust, tipo Rust)
    ('kmask_iq2xs', 'KMASK_IQ2XS', 'u8'), ('ksigns_iq2xs', 'KSIGNS_IQ2XS', 'u8'),
    ('iq2xs_grid', 'IQ2XS_GRID', 'u64'), ('iq2s_grid', 'IQ2S_GRID', 'u64'),
    ('iq3xxs_grid', 'IQ3XXS_GRID', 'u32'), ('iq3s_grid', 'IQ3S_GRID', 'u32'),
    ('kvalues_iq4nl', 'KVALUES_IQ4NL', 'i8'),
]
salida = ['// Generado por tools/iq-oraculo/generar_tablas.py a partir de ggml-common.h (llama.cpp / ggml, licencia MIT). No editar a mano.', '']
for c, rs, ty in TABLAS:
    m = re.search(r'GGML_TABLE_BEGIN\(\w+, ' + c + r', (\d+)\)(.*?)GGML_TABLE_END\(\)', src, re.S)
    assert m, c
    n = int(m.group(1))
    cuerpo = re.sub(r'//[^\n]*', '', m.group(2))
    vals = re.findall(r'-?0x[0-9a-fA-F]+|-?\d+', cuerpo)
    assert len(vals) == n, (c, n, len(vals))
    vals = [v for v in vals]
    ancho = {'u8': 2, 'i8': 0, 'u32': 8, 'u64': 16}[ty]
    def fmt(v):
        n_ = int(v, 0)  # 0x… se lee como hexadecimal y el resto como decimal, según viene en el C
        if ty == 'i8': return str(n_)
        return '0x' + format(n_, '0%dx' % ancho)
    filas = [', '.join(fmt(v) for v in vals[i:i + 8]) for i in range(0, n, 8)]
    salida.append(f'pub static {rs}: [{ty}; {n}] = [')
    salida += ['    ' + f + ',' for f in filas]
    salida += ['];', '']
open(sys.argv[2], 'w').write('\n'.join(salida))
print('tablas escritas:', ', '.join(f'{rs}[{ty};n]' for _, rs, ty in TABLAS))
