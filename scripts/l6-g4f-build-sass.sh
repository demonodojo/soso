#!/usr/bin/env bash
# G4f/G5: compila los kernels .cu → <name>.sass.bin (+ embed C) para sm_120
# (GB205). No requiere soltar la GPU: `ptxas` compila sin tarjeta, solo hace
# falta CUDA ≥ 12.8 para sm_120. Usa nvcc del host si está en PATH; si no, Docker.
#
# Cada kernel sale con su propio blob y su propio juego de metadatos
# (`gsp_<name>_*`): dónde espera sus parámetros y cuántos registros usa. Un
# kernel nuevo se añade a KERNELS y nada más — cuando esto era un script de un
# solo kernel, los nombres estaban incrustados en 40 líneas.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/lxdde/ports/nouveau"
TMP="$ROOT/target/g4f-sass-build"
ARCH="${SOSO_SASS_ARCH:-sm_120}"
IMAGE="${SOSO_CUDA_DOCKER:-nvidia/cuda:12.8.0-devel-ubuntu24.04}"

# name:prefijo de símbolo. El fichero es <name>.cu y el kernel de dentro puede
# llamarse de otra forma (el nombre real se lee del cubin, no de aquí).
KERNELS=("saxpy:saxpy" "matvec:matvec" "matvec_q4k:matvec_q4k" "matvec_q80:matvec_q80" "matmul:matmul" "softmax_rows:softmax_rows" "layernorm_rows:layernorm_rows")

mkdir -p "$TMP"

compile_all_host() {
    local k name
    for k in "${KERNELS[@]}"; do
        name="${k%%:*}"
        nvcc -O2 -arch="$ARCH" --cubin "$SRC/$name.cu" -o "$TMP/$name.cubin"
    done
}

compile_all_docker() {
    local cmds="" k name
    for k in "${KERNELS[@]}"; do
        name="${k%%:*}"
        cmds+="nvcc -O2 -arch=$ARCH --cubin lxdde/ports/nouveau/$name.cu -o target/g4f-sass-build/$name.cubin && "
    done
    docker run --rm -v "$ROOT:/w" -w /w "$IMAGE" bash -lc "${cmds}true"
}

# El .text del cubin: primero con objcopy si hay binutils, y si no a mano. El
# camino a mano no es un lujo — en Docker la imagen de CUDA no trae binutils.
extract_sass() {
    local cubin="$1" out="$2" sec=""

    if command -v cuobjdump >/dev/null 2>&1; then
        cuobjdump --dump-sass "$cubin" > "${cubin%.cubin}.sass.txt" 2>/dev/null || true
    fi
    if command -v readelf >/dev/null 2>&1; then
        sec=$(readelf -S "$cubin" 2>/dev/null | awk '/\.text/ {print $2; exit}' | tr -d '[]')
    fi
    if [[ -n "$sec" ]] && command -v objcopy >/dev/null 2>&1; then
        objcopy -O binary --only-section="$sec" "$cubin" "$out" 2>/dev/null && return 0
    fi
    python3 - "$cubin" "$out" <<'PY'
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
    echo "=== SASS build (host nvcc) arch=$ARCH ==="
    compile_all_host
elif command -v docker >/dev/null 2>&1; then
    echo "=== SASS build (Docker $IMAGE) arch=$ARCH ==="
    compile_all_docker
else
    echo "FAIL: necesitas nvcc en PATH o Docker ($IMAGE)" >&2
    exit 1
fi

for k in "${KERNELS[@]}"; do
    name="${k%%:*}"
    prefix="${k##*:}"
    cubin="$TMP/$name.cubin"
    out_bin="$SRC/$name.sass.bin"
    out_emb="$SRC/${name}_sass_embed.c"

    extract_sass "$cubin" "$out_bin"
    sz=$(wc -c < "$out_bin" | tr -d ' ')
    if [[ "$sz" -lt 16 ]]; then
        echo "FAIL: $name.sass.bin demasiado pequeño ($sz B)" >&2
        exit 1
    fi

    {
        echo "/* Generado por scripts/l6-g4f-build-sass.sh — NO EDITAR */"
        echo "const unsigned char gsp_${prefix}_sass[] = {"
        # `xxd -i < fichero` emite SOLO las líneas de bytes (sin llaves): aquí no
        # se recorta nada. Un `head -n -1` se comía la última línea = los 8 bytes
        # finales, media instrucción de 16 B.
        xxd -i < "$out_bin"
        echo "};"
        echo "const unsigned gsp_${prefix}_sass_len = sizeof(gsp_${prefix}_sass);"
    } > "$out_emb"

    # Se cuenta AQUÍ, antes de añadir los metadatos: sus comentarios llevan
    # valores en hex y falsearían el recuento de bytes del array.
    emb_bytes=$(grep -o '0x[0-9a-fA-F][0-9a-fA-F]' "$out_emb" | wc -l | tr -d ' ')

    # Los metadatos salen del cubin, no de una constante escrita a mano: dónde
    # espera el kernel sus parámetros (EIATTR_PARAM_CBANK) y cuántos registros
    # usa (EIATTR_REGCOUNT). Si el .cu cambia y esto se queda fijo, el
    # lanzamiento lee basura sin dar un solo error.
    python3 - "$cubin" "$prefix" >> "$out_emb" <<'PY'
import struct, sys

data = open(sys.argv[1], "rb").read()
prefix = sys.argv[2]
shoff, = struct.unpack_from("<Q", data, 0x28)
shentsize, shnum, shstrndx = struct.unpack_from("<HHH", data, 0x3A)
shstr, = struct.unpack_from("<Q", data, shoff + shstrndx * shentsize + 0x18)

secs = {}
for i in range(shnum):
    o = shoff + i * shentsize
    nameoff, = struct.unpack_from("<I", data, o)
    name = data[shstr + nameoff:data.find(b"\0", shstr + nameoff)].decode()
    off, size = struct.unpack_from("<QQ", data, o + 0x18)
    secs[name] = (off, size)

def attrs(section):
    off, size = secs[section]
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
        # Las entradas van alineadas a 4 B: tras un BVAL (3 B) viene un byte de
        # relleno. Sin saltarlo el recorrido se para justo antes de PARAM_CBANK.
        i = (i + 3) & ~3
    return out

kernel = next(n for n in secs if n.startswith(".nv.info."))
name = kernel[len(".nv.info."):]

regcount = next(struct.unpack_from("<I", v, 4)[0] for a, v in attrs(".nv.info") if a == 0x2f)
pbase, psize = next(struct.unpack_from("<HH", v, 4) for a, v in attrs(kernel) if a == 0x0a)
cbank = secs[f".nv.constant0.{name}"][1]

# EIATTR_KPARAM_INFO: ordinal en +4, offset en +6. Se ordenan por ordinal.
params = sorted((struct.unpack_from("<H", v, 4)[0], struct.unpack_from("<H", v, 6)[0])
                for a, v in attrs(kernel) if a == 0x17)
if [o for o, _ in params] != list(range(len(params))):
    sys.exit(f"ordinales de parámetro no consecutivos: {params}")

print(f"/* de {name}: EIATTR_REGCOUNT / PARAM_CBANK / KPARAM_INFO del cubin */")
print(f"const unsigned gsp_{prefix}_regcount = {regcount};")
print(f"const unsigned gsp_{prefix}_param_base = {pbase}; /* 0x{pbase:x} en cbank0 */")
print(f"const unsigned gsp_{prefix}_param_size = {psize};")
print(f"const unsigned gsp_{prefix}_cbank_size = {cbank};")
print("const unsigned gsp_%s_param_off[%d] = { %s };"
      % (prefix, len(params), ", ".join(str(off) for _, off in params)))
# El número de parámetros lo dice el cubin, y el port lo comprueba contra lo que
# su header declara: un .cu con un parámetro más y un header sin tocar es
# exactamente el fallo que no da ningún error, solo un kernel leyendo basura.
print(f"const unsigned gsp_{prefix}_param_count = {len(params)};")
PY

    if [[ "$emb_bytes" != "$sz" ]]; then
        echo "FAIL: el embed de $name tiene $emb_bytes B y el blob $sz B" >&2
        exit 1
    fi
    if (( sz % 16 != 0 )); then
        echo "FAIL: $name — $sz B no es múltiplo de 16 (instrucción SASS de $ARCH)" >&2
        exit 1
    fi

    echo "OK: $out_bin ($sz bytes, $((sz / 16)) instrucciones)"
    echo "OK: $out_emb ($emb_bytes bytes embebidos)"
    if [[ -f "$TMP/$name.sass.txt" ]]; then
        echo "--- $name: cuobjdump --dump-sass (primeras líneas) ---"
        head -12 "$TMP/$name.sass.txt"
    fi
done
