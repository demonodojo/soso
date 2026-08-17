/* Decodificación de bloques cuantizados (Q4_K y Q8_0), en C plano.
 *
 * POR QUÉ ES UNA CABECERA Y NO ESTÁ DENTRO DEL .cu: la incluyen DOS cosas, el
 * kernel SASS (`matvec_q4k.cu`, `matvec_q80.cu`) y el hostcheck
 * (`tools/gsp-hostcheck/main.c`). Ésa es la única forma de fijar sin GPU que el
 * lado C decodifica igual que el Rust de `sosomodel::dequant`, que es quien lo
 * hace en la CPU. Si las dos versiones divergen, el síntoma es «la GPU saca otros
 * tokens» y se busca en el silicio, que es donde no está el error.
 *
 * Los dos detalles del layout Q4_K que no son evidentes y que ya se han
 * equivocado antes:
 *
 *  1. Las escalas son de 6 bits repartidas en 12 bytes de forma no obvia
 *     (`get_scale_min_k4` de GGML): los sub-bloques 0..3 usan los bits bajos de
 *     `scales[j]` y `scales[j+4]`; los 4..7 se componen con los nibbles de
 *     `scales[j+4]` y los bits altos de `scales[j-4]`/`scales[j]`.
 *  2. El byte `qs[pair*32 + l]` lleva el elemento `pair*64 + l` en el nibble BAJO
 *     y el `pair*64 + 32 + l` en el ALTO. Invertirlo da un tensor con las mitades
 *     cruzadas y ningún error.
 */
#ifndef Q4K_DECODE_H
#define Q4K_DECODE_H

/* `static inline` a secas es host-only para nvcc, y estas funciones se llaman desde
 * un `__global__`. El hostcheck compila la misma cabecera con clang en C plano,
 * donde `__device__` no existe, así que el calificador va por macro. */
#ifdef __CUDACC__
#define Q4K_FN static __device__ __forceinline__
#else
#define Q4K_FN static inline
#endif

/* Superbloque Q4_K: 256 elementos en 144 bytes.
 *   [0..2)   d      f16
 *   [2..4)   dmin   f16
 *   [4..16)  scales 8 escalas + 8 mins de 6 bits
 *   [16..144) qs    128 nibbles empaquetados
 * valor = (d*sc)*q - (dmin*m) */
#define Q4K_BLOCK_ELEMS 256
#define Q4K_BLOCK_BYTES 144

/* Códigos de dtype, los mismos que `sosomodel::layout::DTYPE_*` (crates/sosomodel).
 * Van por valor y no por nombre porque el ABI de `gpu_submit` los pasa como un byte;
 * si allí cambian, el hostcheck lo caza con `check_mvq_layout`. */
#define GSP_DTYPE_F32   1u
#define GSP_DTYPE_Q8_0  2u
#define GSP_DTYPE_Q4_K  3u
#define GSP_DTYPE_MXFP4 4u

/* Bloque Q8_0: 32 elementos en 36 bytes (escala f32 + 32 int8). */
#define Q80_BLOCK_ELEMS 32
#define Q80_BLOCK_BYTES 36

/* f16 → f32 sin hardware F16C. Réplica exacta de `sosomodel::dequant::f16_to_f32`
 * (incluido el camino de subnormales y el de inf/NaN). En el .cu se compila igual:
 * `__device__` no hace falta porque `static inline` en C++ ya vale para device
 * cuando se llama desde un kernel, y nvcc lo acepta con `--x cu`. */
Q4K_FN float q4k_f16(unsigned short bits)
{
    unsigned int sign = (unsigned int)((bits >> 15) & 1u);
    unsigned int exp = (unsigned int)((bits >> 10) & 0x1fu);
    unsigned int frac = (unsigned int)(bits & 0x3ffu);
    union {
        unsigned int u;
        float f;
    } c;

    if (exp == 0u) {
        if (frac == 0u) {
            c.u = sign << 31;
            return c.f;
        }
        /* 2^-14, la constante que usa el lado Rust. */
        {
            float v = ((float)frac / 1024.0f) * 6.103515625e-5f;
            return sign != 0u ? -v : v;
        }
    }
    if (exp == 31u) {
        c.u = (sign << 31) | 0x7f800000u;
        if (frac != 0u) {
            c.u |= 0x400000u; /* NaN */
        }
        return c.f;
    }
    c.u = (sign << 31) | ((exp + 112u) << 23) | (frac << 13);
    return c.f;
}

/* Escala y min de 6 bits del sub-bloque `j` (0..8) — `get_scale_min_k4`. */
Q4K_FN void q4k_scale_min(const unsigned char *scales, int j,
                                 unsigned char *sc, unsigned char *m)
{
    if (j < 4) {
        *sc = (unsigned char)(scales[j] & 63u);
        *m = (unsigned char)(scales[j + 4] & 63u);
    } else {
        *sc = (unsigned char)((scales[j + 4] & 0x0Fu) | ((scales[j - 4] >> 6) << 4));
        *m = (unsigned char)((scales[j + 4] >> 4) | ((scales[j] >> 6) << 4));
    }
}

/* Producto punto del sub-bloque `j` (0..8, 32 elementos) de un superbloque Q4_K
 * por los 32 valores de `x` que le tocan (`x` apunta ya a `x[j*32]`).
 *
 * Acumula `Σq·x` y `Σx` y aplica `d·sc` / `dmin·m` UNA vez, en vez de reconstruir
 * cada peso: es la misma álgebra que el matvec fusionado de la CPU
 * (`gemm::matvec_q4_k`) y ahorra 32 multiplicaciones por sub-bloque.
 *
 * El sub-bloque, y no el superbloque, es la unidad de reparto del kernel: una fila
 * de un FFN de TinyLlama son 22 superbloques y con eso sólo trabajarían 22 de los
 * 32 lanes de un warp; en sub-bloques son 176 y se reparten enteros. */
Q4K_FN float q4k_dot_sub(const unsigned char *blk, int j, const float *x)
{
    float d = q4k_f16((unsigned short)(blk[0] | ((unsigned short)blk[1] << 8)));
    float dmin = q4k_f16((unsigned short)(blk[2] | ((unsigned short)blk[3] << 8)));
    const unsigned char *qs = blk + 16;
    unsigned char sc, m;
    /* El byte `qs[pair*32 + l]` lleva el elemento `pair*64 + l` en el nibble bajo
     * y el `pair*64 + 32 + l` en el alto: los sub-bloques pares leen el bajo. */
    int pair = j >> 1;
    int alto = j & 1;
    float qx = 0.0f, sx = 0.0f;
    int l;

    q4k_scale_min(blk + 4, j, &sc, &m);
    for (l = 0; l < 32; l++) {
        unsigned char q = qs[pair * 32 + l];
        float v = (float)(alto ? (q >> 4) : (q & 0x0Fu));
        qx += v * x[l];
        sx += x[l];
    }
    return (d * (float)sc) * qx - (dmin * (float)m) * sx;
}

/* Producto punto de UN superbloque Q4_K (256 elementos) por `x`. Definido sobre
 * `q4k_dot_sub` para que no haya dos decodificadores que puedan divergir. */
Q4K_FN float q4k_dot_block(const unsigned char *blk, const float *x)
{
    float acc = 0.0f;
    int j;

    for (j = 0; j < 8; j++) {
        acc += q4k_dot_sub(blk, j, x + j * 32);
    }
    return acc;
}

/* Producto punto de UN bloque Q8_0 (32 elementos) por `x`. */
Q4K_FN float q80_dot_block(const unsigned char *blk, const float *x)
{
    union {
        unsigned int u;
        float f;
    } c;
    float acc = 0.0f;
    int l;

    c.u = (unsigned int)blk[0] | ((unsigned int)blk[1] << 8) |
          ((unsigned int)blk[2] << 16) | ((unsigned int)blk[3] << 24);
    for (l = 0; l < 32; l++) {
        acc += (float)(signed char)blk[4 + l] * x[l];
    }
    return acc * c.f;
}

#endif
