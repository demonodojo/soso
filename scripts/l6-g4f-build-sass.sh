#!/usr/bin/env bash
# G4f: compila saxpy.cu → saxpy.sass.bin (+ embed C) para sm_120 (GB205).
# No requiere soltar la GPU. Usa nvcc del host si está en PATH; si no, Docker.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CU="$ROOT/lxdde/ports/nouveau/saxpy.cu"
OUT_BIN="$ROOT/lxdde/ports/nouveau/saxpy.sass.bin"
OUT_EMB="$ROOT/lxdde/ports/nouveau/saxpy_sass_embed.c"
TMP="$ROOT/target/g4f-sass-build"
ARCH="${SOSO_SASS_ARCH:-sm_120}"
IMAGE="${SOSO_CUDA_DOCKER:-nvidia/cuda:12.8.0-devel-ubuntu24.04}"

mkdir -p "$TMP"

run_nvcc() {
    local nvcc="$1"
    "$nvcc" -O2 -arch="$ARCH" --cubin "$CU" -o "$TMP/saxpy.cubin"
    "$nvcc" -bin="$nvcc" -ptx "$CU" -o "$TMP/saxpy.ptx" 2>/dev/null || true
}

extract_sass() {
    if command -v cuobjdump >/dev/null 2>&1; then
        cuobjdump --dump-sass "$TMP/saxpy.cubin" > "$TMP/saxpy.sass.txt" || true
    fi
    local sec=""
    if command -v readelf >/dev/null 2>&1; then
        sec=$(readelf -S "$TMP/saxpy.cubin" 2>/dev/null | awk '/\.text/ {print $2; exit}' | tr -d '[]')
    fi
    if [[ -n "$sec" ]] && command -v objcopy >/dev/null 2>&1; then
        objcopy -O binary --only-section="$sec" "$TMP/saxpy.cubin" "$OUT_BIN" 2>/dev/null && return 0
    fi
    python3 - "$TMP/saxpy.cubin" "$OUT_BIN" <<'PY'
import struct, sys
path, out = sys.argv[1], sys.argv[2]
data = open(path, "rb").read()
if data[:4] != b"\x7fELF":
    sys.exit("no ELF cubin")
e_shoff = struct.unpack_from("<Q", data, 0x28)[0]
e_shentsize = struct.unpack_from("<H", data, 0x3A)[0]
e_shnum = struct.unpack_from("<H", data, 0x3C)[0]
e_shstrndx = struct.unpack_from("<H", data, 0x3E)[0]
shstr_off = struct.unpack_from("<Q", data, e_shoff + e_shstrndx * e_shentsize + 0x18)[0]
best = None
for i in range(e_shnum):
    off = e_shoff + i * e_shentsize
    sh_name = struct.unpack_from("<I", data, off)[0]
    sh_type = struct.unpack_from("<I", data, off + 4)[0]
    sh_offset = struct.unpack_from("<Q", data, off + 0x18)[0]
    sh_size = struct.unpack_from("<Q", data, off + 0x20)[0]
    name_end = data.find(b"\x00", shstr_off + sh_name)
    name = data[shstr_off + sh_name:name_end].decode("ascii", "ignore")
    if sh_type == 1 and sh_size > 64 and name.startswith(".text"):
        if best is None or sh_size > best[0]:
            best = (sh_size, sh_offset, name)
if not best:
    sys.exit("no .text* in cubin")
size, off, name = best
open(out, "wb").write(data[off:off + size])
print(f"extracted {size} bytes from {name}")
PY
}

if command -v nvcc >/dev/null 2>&1; then
    echo "=== G4f SASS build (host nvcc) arch=$ARCH ==="
    run_nvcc nvcc
elif command -v docker >/dev/null 2>&1; then
    echo "=== G4f SASS build (Docker $IMAGE) arch=$ARCH ==="
    docker run --rm -v "$ROOT:/w" -w /w "$IMAGE" bash -lc \
        "apt-get update -qq && apt-get install -y -qq binutils >/dev/null && \
         nvcc -O2 -arch=$ARCH --cubin lxdde/ports/nouveau/saxpy.cu -o target/g4f-sass-build/saxpy.cubin"
else
    echo "FAIL: necesitas nvcc en PATH o Docker ($IMAGE)" >&2
    exit 1
fi

extract_sass
SZ=$(wc -c < "$OUT_BIN" | tr -d ' ')
if [[ "$SZ" -lt 16 ]]; then
    echo "FAIL: saxpy.sass.bin demasiado pequeño ($SZ B)" >&2
    exit 1
fi

{
    echo "/* Generado por scripts/l6-g4f-build-sass.sh — NO EDITAR */"
    echo "const unsigned char gsp_saxpy_sass[] = {"
    xxd -i < "$OUT_BIN" | sed 's/unsigned char/const unsigned char/' | head -n -1
    echo "};"
    echo "const unsigned gsp_saxpy_sass_len = sizeof(gsp_saxpy_sass);"
} > "$OUT_EMB"

echo "OK: $OUT_BIN ($SZ bytes)"
echo "OK: $OUT_EMB"
if [[ -f "$TMP/saxpy.sass.txt" ]]; then
    echo "--- cuobjdump --dump-sass (primeras líneas) ---"
    head -20 "$TMP/saxpy.sass.txt"
fi
