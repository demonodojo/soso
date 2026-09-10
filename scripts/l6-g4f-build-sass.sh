#!/usr/bin/env bash
# G4f/G5: compila los kernels .cu a SASS para **cada arquitectura objetivo** y
# emite un juego identificado por arquitectura (`sass_<tag>.c` + manifiesto).
#
# No requiere soltar la GPU ni tener una: `ptxas` compila sin tarjeta. Usa nvcc
# del host si está en PATH; si no, Docker.
#
# R5: antes esto producía UN único juego de blobs, con `sm_120` por defecto y
# sin decir de qué arquitectura eran: compilar para otra los sobrescribía y el
# driver los lanzaba igual. Ahora cada arquitectura tiene su fichero, su tabla y
# su manifiesto, y el driver elige por familia.
#
#   SOSO_SASS_ARCHS="sm_86:sm86:GSP_FAM_AMPERE sm_120:sm120:GSP_FAM_BLACKWELL"
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/lxdde/ports/nouveau"
TMP="$ROOT/target/g4f-sass-build"
IMAGE="${SOSO_CUDA_DOCKER:-nvidia/cuda:12.8.0-devel-ubuntu24.04}"

# arch:tag:familia — el tag va en los símbolos C, la familia en la tabla.
ARCHS_DEFAULT="sm_86:sm86:GSP_FAM_AMPERE sm_120:sm120:GSP_FAM_BLACKWELL"
read -r -a ARCHS <<< "${SOSO_SASS_ARCHS:-$ARCHS_DEFAULT}"

# name:prefijo de símbolo. El fichero es <name>.cu y el kernel de dentro puede
# llamarse de otra forma (el nombre real se lee del cubin, no de aquí).
KERNELS=("saxpy:saxpy" "matvec:matvec" "matvec_q4k:matvec_q4k" "matvec_q80:matvec_q80" "matmul:matmul" "softmax_rows:softmax_rows" "layernorm_rows:layernorm_rows")

compile_host() {
    local arch="$1" out="$2" k name
    for k in "${KERNELS[@]}"; do
        name="${k%%:*}"
        nvcc -O2 -arch="$arch" --cubin "$SRC/$name.cu" -o "$out/$name.cubin"
    done
}

compile_docker() {
    local arch="$1" out="$2" rel cmds="" k name
    rel="${out#"$ROOT/"}"
    for k in "${KERNELS[@]}"; do
        name="${k%%:*}"
        cmds+="nvcc -O2 -arch=$arch --cubin lxdde/ports/nouveau/$name.cu -o $rel/$name.cubin && "
    done
    # --user: si no, los cubin salen de root y el build siguiente falla al
    # intentar reescribir target/ (clang «Operation not permitted»).
    docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp \
        -v "$ROOT:/w" -w /w "$IMAGE" bash -lc "${cmds}true"
}

for spec in "${ARCHS[@]}"; do
    arch="${spec%%:*}"
    resto="${spec#*:}"
    tag="${resto%%:*}"
    familia="${resto#*:}"
    out="$TMP/$arch"
    mkdir -p "$out"

    if command -v nvcc >/dev/null 2>&1; then
        echo "=== SASS build (host nvcc) arch=$arch ==="
        compile_host "$arch" "$out"
    elif command -v docker >/dev/null 2>&1; then
        echo "=== SASS build (Docker $IMAGE) arch=$arch ==="
        compile_docker "$arch" "$out"
    else
        echo "FAIL: necesitas nvcc en PATH o Docker ($IMAGE)" >&2
        exit 1
    fi

    python3 "$ROOT/scripts/l6-g4f-emit-sass.py" \
        "$out" "$arch" "$tag" "$familia" "$SRC/sass_$tag.c" "${KERNELS[@]}"

    # Los blobs sueltos también quedan identificados por arquitectura, para
    # comparar tamaños o desensamblarlos sin rehacer el build.
    mkdir -p "$SRC/sass/$arch"
    for k in "${KERNELS[@]}"; do
        name="${k%%:*}"
        python3 - "$out/$name.cubin" "$SRC/sass/$arch/$name.sass.bin" <<'PY'
import struct, sys
data = open(sys.argv[1], "rb").read()
shoff, = struct.unpack_from("<Q", data, 0x28)
shentsize, shnum, shstrndx = struct.unpack_from("<HHH", data, 0x3A)
shstr, = struct.unpack_from("<Q", data, shoff + shstrndx * shentsize + 0x18)
best = None
for i in range(shnum):
    o = shoff + i * shentsize
    nameoff, tipo = struct.unpack_from("<II", data, o)
    name = data[shstr + nameoff:data.find(b"\0", shstr + nameoff)].decode()
    off, size = struct.unpack_from("<QQ", data, o + 0x18)
    if tipo == 1 and size > 64 and name.startswith(".text"):
        if best is None or size > best[1]:
            best = (off, size)
if not best:
    sys.exit("cubin sin .text*")
open(sys.argv[2], "wb").write(data[best[0]:best[0] + best[1]])
PY
    done
    if command -v cuobjdump >/dev/null 2>&1; then
        cuobjdump --dump-sass "$out/saxpy.cubin" > "$out/saxpy.sass.txt" 2>/dev/null || true
    fi
done

# Cabecera generada: qué juegos trae este build. El driver la incluye para no
# tener que listar arquitecturas a mano en dos sitios.
{
    echo "/* Generado por scripts/l6-g4f-build-sass.sh — NO EDITAR. */"
    echo "#ifndef SASS_SETS_H"
    echo "#define SASS_SETS_H"
    echo ""
    echo "#include \"gsp_sass.h\""
    echo ""
    for spec in "${ARCHS[@]}"; do
        resto="${spec#*:}"
        tag="${resto%%:*}"
        echo "extern const struct gsp_sass_set gsp_sass_set_$tag;"
    done
    echo ""
    printf '#define GSP_SASS_SETS {'
    for spec in "${ARCHS[@]}"; do
        resto="${spec#*:}"
        tag="${resto%%:*}"
        printf ' &gsp_sass_set_%s,' "$tag"
    done
    printf ' }\n'
    echo "#define GSP_SASS_SET_COUNT ${#ARCHS[@]}u"
    echo ""
    echo "#endif"
} > "$SRC/sass_sets.h"
echo "OK: $SRC/sass_sets.h (${#ARCHS[@]} juegos)"
