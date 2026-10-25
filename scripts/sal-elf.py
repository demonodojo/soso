#!/usr/bin/env python3
"""Valida los ELF del port antes de empaquetarlos, sin ejecutarlos en Linux."""
import pathlib
import struct
import sys

for name in sys.argv[1:]:
    data = pathlib.Path(name).read_bytes()
    assert data[:6] == b"\x7fELF\x02\x01", f"{name}: no es ELF64 little endian"
    kind, arch = struct.unpack_from("<HH", data, 16)
    entry, phoff, shoff = struct.unpack_from("<QQQ", data, 24)
    phsize, phnum, shsize, shnum = struct.unpack_from("<HHHH", data, 54)
    assert (kind, arch) == (2, 62), f"{name}: se requiere EXEC x86-64"
    executable = False
    for i in range(phnum):
        typ, flags, off, va, _, filesz, memsz, _ = struct.unpack_from("<IIQQQQQQ", data, phoff + i * phsize)
        assert typ not in (2, 3), f"{name}: dependencia del cargador dinámico"
        if typ == 1:
            assert va >= 0x400000 and filesz <= memsz and off + filesz <= len(data), f"{name}: segmento inválido"
            executable |= bool(flags & 1) and va <= entry < va + memsz
    assert executable, f"{name}: entrada fuera de segmento ejecutable"
    for i in range(shnum):
        _, typ, _, _, off, size, _, _, _, entsize = struct.unpack_from("<IIQQQQIIQQ", data, shoff + i * shsize)
        if typ == 2:
            for pos in range(off + entsize, off + size, entsize):
                _, info, _, section, _, _ = struct.unpack_from("<IBBHQQ", data, pos)
                assert section != 0 or info >> 4 != 1, f"{name}: símbolo global sin resolver"
    print(f"ELF OK: {name}, entrada={entry:#x}, estático x86-64")
