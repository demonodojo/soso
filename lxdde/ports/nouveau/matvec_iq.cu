/* matvec con la matriz en un formato GGML de bloques de 256 (i-quants y K-quants que faltaban) **residente tal cual**.
 * y = W·x para UN vector x (decode). Un warp por fila; los lanes reparten unidades de 32 elementos. `x` va a shared.
 *
 * Aritmética en f32 (no el producto entero × Q8_K de la CPU): en la GPU decodificar es barato y lo que manda es leer
 * los bytes del peso una vez. Resultados iguales a los de la CPU salvo el orden de la suma.
 *
 * Tipos (`dtype` = identificador de GGML): 11 Q3_K (no), 13 Q5_K, 17 IQ2_XS (no), 18 IQ3_XXS, 21 IQ3_S, 22 IQ2_S,
 * 23 IQ4_XS. Las tablas vienen de iq_tablas.cuh (generado desde sosomodel/src/iq_tablas.rs).
 * Se compila con tools/cuda-nvrtc/compilar.py (NVRTC, sin toolkit en el host).
 */
#include "iq_tablas.cuh"

#define IQ_GGML_Q5_K 13
#define IQ_GGML_IQ3_XXS 18
#define IQ_GGML_IQ3_S 21
#define IQ_GGML_IQ2_S 22
#define IQ_GGML_IQ4_XS 23

__device__ __forceinline__ float iq_f16(const unsigned char *p)
{
    /* f16 → f32 a mano: NVRTC no trae cuda_fp16.h. */
    unsigned int h = (unsigned int)(p[0] | (p[1] << 8));
    unsigned int e = (h >> 10) & 31u, f = h & 1023u;
    float v;
    if (e == 0u) {
        v = ldexpf((float)f, -24);
    } else if (e == 31u) {
        v = __uint_as_float(0x7f800000u | (f << 13));
    } else {
        v = __uint_as_float(((e + 112u) << 23) | (f << 13));
    }
    return (h & 0x8000u) ? -v : v;
}

__device__ __forceinline__ float iq_sg(unsigned char signs, int j)
{
    return (signs & KMASK_IQ2XS_D[j]) ? -1.0f : 1.0f;
}

/* Producto de 8 elementos de rejilla (8 bytes sin signo) con signos por bit y x. */
__device__ __forceinline__ float iq_grid8(unsigned long long g, unsigned char signs, const float *x)
{
    float s = 0.0f;
#pragma unroll
    for (int j = 0; j < 8; j++) {
        s += (float)((g >> (8 * j)) & 0xffu) * iq_sg(signs, j) * x[j];
    }
    return s;
}

/* Dos rejillas de 4 bytes (u32) = 8 elementos. */
__device__ __forceinline__ float iq_grid44(unsigned int g1, unsigned int g2, unsigned char signs, const float *x)
{
    float s = 0.0f;
#pragma unroll
    for (int j = 0; j < 4; j++) {
        s += (float)((g1 >> (8 * j)) & 0xffu) * iq_sg(signs, j) * x[j];
        s += (float)((g2 >> (8 * j)) & 0xffu) * iq_sg(signs, j + 4) * x[j + 4];
    }
    return s;
}

__device__ __forceinline__ void iq_scale_min_k4(int j, const unsigned char *q, int *sc, int *mn)
{
    if (j < 4) {
        *sc = q[j] & 63;
        *mn = q[j + 4] & 63;
    } else {
        *sc = (q[j + 4] & 0xF) | ((q[j - 4] >> 6) << 4);
        *mn = (q[j + 4] >> 4) | ((q[j] >> 6) << 4);
    }
}

/* Contribución de la unidad `u` (32 elementos) de una fila: `w` apunta al inicio de la fila, `x` a los 32 floats. */
__device__ __forceinline__ float iq_unidad(int dtype, const unsigned char *w, int u, const float *x)
{
    int sb = u >> 3;
    int ib = u & 7;
    float acc = 0.0f;
    switch (dtype) {
    case IQ_GGML_IQ3_XXS: {
        const unsigned char *b = w + (long)sb * 98;
        float d = iq_f16(b);
        const unsigned char *qs = b + 2;
        const unsigned char *gas = qs + 64;
        const unsigned char *ga = gas + 4 * ib; /* los bloques miden 98 B: sin alineación de 4 */
        unsigned int aux = (unsigned int)ga[0] | ((unsigned int)ga[1] << 8) | ((unsigned int)ga[2] << 16) | ((unsigned int)ga[3] << 24);
        float db = d * (0.5f + (float)(aux >> 28)) * 0.5f;
        for (int l = 0; l < 4; l++) {
            unsigned char sg = KSIGNS_IQ2XS_D[(aux >> (7 * l)) & 127];
            unsigned int g1 = IQ3XXS_GRID_D[qs[ib * 8 + 2 * l]];
            unsigned int g2 = IQ3XXS_GRID_D[qs[ib * 8 + 2 * l + 1]];
            acc += db * iq_grid44(g1, g2, sg, x + 8 * l);
        }
        break;
    }
    case IQ_GGML_IQ2_S: {
        const unsigned char *b = w + (long)sb * 82;
        float d = iq_f16(b);
        const unsigned char *qs = b + 2;
        const unsigned char *qh = b + 66;
        unsigned char s = b[74 + ib];
        float db0 = d * (0.5f + (float)(s & 0xf)) * 0.25f;
        float db1 = d * (0.5f + (float)(s >> 4)) * 0.25f;
        for (int l = 0; l < 4; l++) {
            unsigned long long g = IQ2S_GRID_D[qs[ib * 4 + l] | (((int)qh[ib] << (8 - 2 * l)) & 0x300)];
            acc += (l < 2 ? db0 : db1) * iq_grid8(g, qs[32 + ib * 4 + l], x + 8 * l);
        }
        break;
    }
    case 17: { /* IQ2_XS */
        const unsigned char *b = w + (long)sb * 74;
        float d = iq_f16(b);
        unsigned char sbyte = b[66 + ib];
        float db0 = d * (0.5f + (float)(sbyte & 0xf)) * 0.25f;
        float db1 = d * (0.5f + (float)(sbyte >> 4)) * 0.25f;
        for (int l = 0; l < 4; l++) {
            const unsigned char *qp = b + 2 + 2 * (4 * ib + l);
            unsigned int q = (unsigned int)qp[0] | ((unsigned int)qp[1] << 8);
            acc += (l < 2 ? db0 : db1) * iq_grid8(IQ2XS_GRID_D[q & 511u], KSIGNS_IQ2XS_D[q >> 9], x + 8 * l);
        }
        break;
    }
    case IQ_GGML_IQ3_S: {
        const unsigned char *b = w + (long)sb * 110;
        float d = iq_f16(b);
        const unsigned char *qs = b + 2;
        const unsigned char *qh = b + 66;
        const unsigned char *sgn = b + 74;
        const unsigned char *sc = b + 106;
        float db = d * (float)(1 + 2 * ((sc[ib >> 1] >> (4 * (ib & 1))) & 0xf));
        for (int l = 0; l < 4; l++) {
            int i1 = qs[ib * 8 + 2 * l] | (((int)qh[ib] << (8 - 2 * l)) & 256);
            int i2 = qs[ib * 8 + 2 * l + 1] | (((int)qh[ib] << (7 - 2 * l)) & 256);
            acc += db * iq_grid44(IQ3S_GRID_D[i1], IQ3S_GRID_D[i2], sgn[ib * 4 + l], x + 8 * l);
        }
        break;
    }
    case IQ_GGML_IQ4_XS: {
        const unsigned char *b = w + (long)sb * 136;
        float d = iq_f16(b);
        unsigned int sh = b[2] | (b[3] << 8);
        int ls = ((b[4 + (ib >> 1)] >> (4 * (ib & 1))) & 0xf) | (((sh >> (2 * ib)) & 3) << 4);
        float dl = d * (float)(ls - 32);
        const unsigned char *qs = b + 8 + ib * 16;
        float s = 0.0f;
#pragma unroll
        for (int j = 0; j < 16; j++) {
            s += (float)KVALUES_IQ4NL_D[qs[j] & 0xf] * x[j] + (float)KVALUES_IQ4NL_D[qs[j] >> 4] * x[j + 16];
        }
        acc = dl * s;
        break;
    }
    case IQ_GGML_Q5_K: {
        const unsigned char *b = w + (long)sb * 176;
        float d = iq_f16(b);
        float dmin = iq_f16(b + 2);
        int sc, mn;
        iq_scale_min_k4(ib, b + 4, &sc, &mn);
        const unsigned char *qh = b + 16;
        const unsigned char *qs = b + 48 + 32 * (ib >> 1);
        int alto = ib & 1;
        float sq = 0.0f, sx = 0.0f;
#pragma unroll
        for (int l = 0; l < 32; l++) {
            int q = (alto ? (qs[l] >> 4) : (qs[l] & 0xf)) + ((qh[l] >> ib) & 1) * 16;
            sq += (float)q * x[l];
            sx += x[l];
        }
        acc = d * (float)sc * sq - dmin * (float)mn * sx;
        break;
    }
    default:
        break;
    }
    return acc;
}

__device__ __forceinline__ float iq_warp_sum(float v)
{
    for (int o = 16; o > 0; o >>= 1) {
        v += __shfl_down_sync(0xffffffffu, v, o);
    }
    return v;
}

/* y[r] = W[r]·x. grid = ceil(rows / 8); 256 hilos = 8 warps = 8 filas; shared dinámica = cols floats. */
extern "C" __global__ void iq_matvec(const unsigned char *w, const float *x, float *y, int rows, int cols, int dtype,
                                     long row_bytes)
{
    extern __shared__ float xs[];
    for (int i = (int)threadIdx.x; i < cols; i += (int)blockDim.x) {
        xs[(i >> 5) * 33 + (i & 31)] = x[i]; /* relleno: 33 floats por unidad de 32 (sin conflictos de banco) */
    }
    __syncthreads();
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row = (int)blockIdx.x * 8 + warp;
    if (row >= rows) {
        return;
    }
    const unsigned char *wr = w + (long)row * row_bytes;
    int units = cols >> 5;
    float acc = 0.0f;
    for (int u = lane; u < units; u += 32) {
        acc += iq_unidad(dtype, wr, u, xs + (long)u * 33);
    }
    acc = iq_warp_sum(acc);
    if (lane == 0) {
        y[row] = acc;
    }
}

/* ---------------------------------------------------------------------------------------------------------------------
 * Prefill por bloques: varios tokens por lectura de pesos.
 *
 * `iq_decodificar` deja en `o` los 32 pesos (ya con escala) de la unidad `u` de una fila; el producto con cualquier x es entonces
 * un FMA por elemento, y la decodificación (tablas, signos, escalas) se paga una vez para todos los tokens del tile.
 * ------------------------------------------------------------------------------------------------------------------ */
__device__ __forceinline__ void iq_decodificar(int dtype, const unsigned char *w, int u, float *o)
{
    int sb = u >> 3;
    int ib = u & 7;
    switch (dtype) {
    case IQ_GGML_IQ3_XXS: {
        const unsigned char *b = w + (long)sb * 98;
        float d = iq_f16(b);
        const unsigned char *qs = b + 2;
        const unsigned char *ga = qs + 64 + 4 * ib;
        unsigned int aux = (unsigned int)ga[0] | ((unsigned int)ga[1] << 8) | ((unsigned int)ga[2] << 16) | ((unsigned int)ga[3] << 24);
        float db = d * (0.5f + (float)(aux >> 28)) * 0.5f;
        for (int l = 0; l < 4; l++) {
            unsigned char sg = KSIGNS_IQ2XS_D[(aux >> (7 * l)) & 127];
            unsigned int g1 = IQ3XXS_GRID_D[qs[ib * 8 + 2 * l]];
            unsigned int g2 = IQ3XXS_GRID_D[qs[ib * 8 + 2 * l + 1]];
#pragma unroll
            for (int j = 0; j < 4; j++) {
                o[8 * l + j] = db * (float)((g1 >> (8 * j)) & 0xffu) * iq_sg(sg, j);
                o[8 * l + j + 4] = db * (float)((g2 >> (8 * j)) & 0xffu) * iq_sg(sg, j + 4);
            }
        }
        break;
    }
    case IQ_GGML_IQ2_S: {
        const unsigned char *b = w + (long)sb * 82;
        float d = iq_f16(b);
        const unsigned char *qs = b + 2;
        const unsigned char *qh = b + 66;
        unsigned char s = b[74 + ib];
        float db0 = d * (0.5f + (float)(s & 0xf)) * 0.25f;
        float db1 = d * (0.5f + (float)(s >> 4)) * 0.25f;
        for (int l = 0; l < 4; l++) {
            unsigned long long g = IQ2S_GRID_D[qs[ib * 4 + l] | (((int)qh[ib] << (8 - 2 * l)) & 0x300)];
            unsigned char sg = qs[32 + ib * 4 + l];
            float db = l < 2 ? db0 : db1;
#pragma unroll
            for (int j = 0; j < 8; j++) {
                o[8 * l + j] = db * (float)((g >> (8 * j)) & 0xffu) * iq_sg(sg, j);
            }
        }
        break;
    }
    case IQ_GGML_IQ3_S: {
        const unsigned char *b = w + (long)sb * 110;
        float d = iq_f16(b);
        const unsigned char *qs = b + 2;
        const unsigned char *qh = b + 66;
        const unsigned char *sgn = b + 74;
        const unsigned char *sc = b + 106;
        float db = d * (float)(1 + 2 * ((sc[ib >> 1] >> (4 * (ib & 1))) & 0xf));
        for (int l = 0; l < 4; l++) {
            int i1 = qs[ib * 8 + 2 * l] | (((int)qh[ib] << (8 - 2 * l)) & 256);
            int i2 = qs[ib * 8 + 2 * l + 1] | (((int)qh[ib] << (7 - 2 * l)) & 256);
            unsigned int g1 = IQ3S_GRID_D[i1], g2 = IQ3S_GRID_D[i2];
            unsigned char sg = sgn[ib * 4 + l];
#pragma unroll
            for (int j = 0; j < 4; j++) {
                o[8 * l + j] = db * (float)((g1 >> (8 * j)) & 0xffu) * iq_sg(sg, j);
                o[8 * l + j + 4] = db * (float)((g2 >> (8 * j)) & 0xffu) * iq_sg(sg, j + 4);
            }
        }
        break;
    }
    case 17: { /* IQ2_XS: d f16, qs u16[32], scales[8] */
        const unsigned char *b = w + (long)sb * 74;
        float d = iq_f16(b);
        unsigned char s = b[66 + ib];
        float db0 = d * (0.5f + (float)(s & 0xf)) * 0.25f;
        float db1 = d * (0.5f + (float)(s >> 4)) * 0.25f;
        for (int l = 0; l < 4; l++) {
            const unsigned char *qp = b + 2 + 2 * (4 * ib + l);
            unsigned int q = (unsigned int)qp[0] | ((unsigned int)qp[1] << 8);
            unsigned long long g = IQ2XS_GRID_D[q & 511u];
            unsigned char sg = KSIGNS_IQ2XS_D[q >> 9];
            float db = l < 2 ? db0 : db1;
#pragma unroll
            for (int j = 0; j < 8; j++) {
                o[8 * l + j] = db * (float)((g >> (8 * j)) & 0xffu) * iq_sg(sg, j);
            }
        }
        break;
    }
    case IQ_GGML_IQ4_XS: {
        const unsigned char *b = w + (long)sb * 136;
        float d = iq_f16(b);
        unsigned int sh = b[2] | (b[3] << 8);
        int ls = ((b[4 + (ib >> 1)] >> (4 * (ib & 1))) & 0xf) | (((sh >> (2 * ib)) & 3) << 4);
        float dl = d * (float)(ls - 32);
        const unsigned char *qs = b + 8 + ib * 16;
#pragma unroll
        for (int j = 0; j < 16; j++) {
            o[j] = dl * (float)KVALUES_IQ4NL_D[qs[j] & 0xf];
            o[j + 16] = dl * (float)KVALUES_IQ4NL_D[qs[j] >> 4];
        }
        break;
    }
    case IQ_GGML_Q5_K: {
        const unsigned char *b = w + (long)sb * 176;
        float d = iq_f16(b);
        float dmin = iq_f16(b + 2);
        int sc, mn;
        iq_scale_min_k4(ib, b + 4, &sc, &mn);
        const unsigned char *qh = b + 16;
        const unsigned char *qs = b + 48 + 32 * (ib >> 1);
        int alto = ib & 1;
        float ds = d * (float)sc, dm = dmin * (float)mn;
#pragma unroll
        for (int l = 0; l < 32; l++) {
            int q = (alto ? (qs[l] >> 4) : (qs[l] & 0xf)) + ((qh[l] >> ib) & 1) * 16;
            o[l] = ds * (float)q - dm;
        }
        break;
    }
    case 1002: { /* Q4_K (superbloque de 256, 144 B); la unidad `u` es el sub-bloque `ib` de 32 elementos */
        const unsigned char *b = w + (long)sb * 144;
        float d = iq_f16(b);
        float dmin = iq_f16(b + 2);
        int sc, mn;
        iq_scale_min_k4(ib, b + 4, &sc, &mn);
        const unsigned char *qs = b + 16 + 32 * (ib >> 1);
        int alto = ib & 1;
        float ds = d * (float)sc, dm = dmin * (float)mn;
#pragma unroll
        for (int l = 0; l < 32; l++) {
            int q = alto ? (qs[l] >> 4) : (qs[l] & 0xf);
            o[l] = ds * (float)q - dm;
        }
        break;
    }
    case 1001: { /* F32 crudo: la unidad `u` son los 32 floats [u * 32, u * 32 + 32) de la fila */
        const float *f = (const float *)w + (long)u * 32;
#pragma unroll
        for (int l = 0; l < 32; l++) {
            o[l] = f[l];
        }
        break;
    }
    case 1000: { /* Q8_0 de sosomodel: escala f32 + 32 i8 por bloque (36 B); la unidad `u` es el bloque `u` de la fila */
        const unsigned char *b = w + (long)u * 36;
        float d = __uint_as_float((unsigned int)b[0] | ((unsigned int)b[1] << 8) | ((unsigned int)b[2] << 16) | ((unsigned int)b[3] << 24));
#pragma unroll
        for (int l = 0; l < 32; l++) {
            o[l] = d * (float)(signed char)b[4 + l];
        }
        break;
    }
    default:
#pragma unroll
        for (int l = 0; l < 32; l++) {
            o[l] = 0.0f;
        }
        break;
    }
}

#define IQ_TILE 16
#ifndef INFINITY
#define INFINITY __int_as_float(0x7f800000)
#endif

/* Gate y up de un experto para `cnt` tokens y SwiGLU: act[(abase + t) * ffn + fila] = silu(gate·x_t) · (up·x_t).
 * `xs` son los `n` vectores del bloque (h floats cada uno); `tok[t]` dice qué vector es el token `t` del experto.
 * grid = (ceil(ffn / 8), ceil(cnt / IQ_TILE)); 256 hilos = 8 warps = 8 filas; cada warp calcula IQ_TILE tokens a la vez. */
extern "C" __global__ void iq_gateup_n(const unsigned char *wg, const unsigned char *wu, const float *xs, const int *tok, float *act,
                                       int ffn, int h, int cnt, int abase, int dg, int du, long rbg, long rbu)
{
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int fila = (int)blockIdx.x * 8 + warp;
    if (fila >= ffn) {
        return;
    }
    int t0 = (int)blockIdx.y * IQ_TILE;
    int nt = cnt - t0 < IQ_TILE ? cnt - t0 : IQ_TILE;
    const unsigned char *rg = wg + (long)fila * rbg;
    const unsigned char *ru = wu + (long)fila * rbu;
    float ag[IQ_TILE], au[IQ_TILE];
#pragma unroll
    for (int t = 0; t < IQ_TILE; t++) {
        ag[t] = 0.0f;
        au[t] = 0.0f;
    }
    int units = h >> 5;
    for (int u = lane; u < units; u += 32) {
        float vg[32], vu[32];
        iq_decodificar(dg, rg, u, vg);
        iq_decodificar(du, ru, u, vu);
#pragma unroll
        for (int t = 0; t < IQ_TILE; t++) {
            if (t < nt) {
                const float4 *xp = (const float4 *)(xs + (long)tok[t0 + t] * h + (long)u * 32);
                float sg = 0.0f, su = 0.0f;
#pragma unroll
                for (int i = 0; i < 8; i++) {
                    float4 x = xp[i];
                    sg += vg[4 * i] * x.x + vg[4 * i + 1] * x.y + vg[4 * i + 2] * x.z + vg[4 * i + 3] * x.w;
                    su += vu[4 * i] * x.x + vu[4 * i + 1] * x.y + vu[4 * i + 2] * x.z + vu[4 * i + 3] * x.w;
                }
                ag[t] += sg;
                au[t] += su;
            }
        }
    }
#pragma unroll
    for (int t = 0; t < IQ_TILE; t++) {
        float g = iq_warp_sum(ag[t]);
        float up = iq_warp_sum(au[t]);
        if (lane == 0 && t < nt) {
            act[(long)(abase + t0 + t) * ffn + fila] = (g / (1.0f + expf(-g))) * up;
        }
    }
}

/* y[(abase + t) * rows + fila] = W[fila] · act[(abase + t) * cols] para los `cnt` tokens del experto. grid = (ceil(rows / 8), ceil(cnt / IQ_TILE)). */
__device__ __forceinline__ void iq_down_dev(const unsigned char *w, const float *act, float *y, int rows, int cols, int cnt, int abase, int dtype,
                                          long rb)
{
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int fila = (int)blockIdx.x * 8 + warp;
    if (fila >= rows) {
        return;
    }
    int t0 = (int)blockIdx.y * IQ_TILE;
    int nt = cnt - t0 < IQ_TILE ? cnt - t0 : IQ_TILE;
    const unsigned char *r = w + (long)fila * rb;
    float a[IQ_TILE];
#pragma unroll
    for (int t = 0; t < IQ_TILE; t++) {
        a[t] = 0.0f;
    }
    int units = cols >> 5;
    for (int u = lane; u < units; u += 32) {
        float v[32];
        iq_decodificar(dtype, r, u, v);
#pragma unroll
        for (int t = 0; t < IQ_TILE; t++) {
            if (t < nt) {
                const float4 *xp = (const float4 *)(act + (long)(abase + t0 + t) * cols + (long)u * 32);
                float s = 0.0f;
#pragma unroll
                for (int i = 0; i < 8; i++) {
                    float4 x = xp[i];
                    s += v[4 * i] * x.x + v[4 * i + 1] * x.y + v[4 * i + 2] * x.z + v[4 * i + 3] * x.w;
                }
                a[t] += s;
            }
        }
    }
#pragma unroll
    for (int t = 0; t < IQ_TILE; t++) {
        float s = iq_warp_sum(a[t]);
        if (lane == 0 && t < nt) {
            y[(long)(abase + t0 + t) * rows + fila] = s;
        }
    }
}

extern "C" __global__ void iq_down_n(const unsigned char *w, const float *act, float *y, int rows, int cols, int cnt, int abase, int dtype,
                                     long rb)
{
    iq_down_dev(w, act, y, rows, cols, cnt, abase, dtype, rb);
}

/* Matriz densa por bloque: y[t * rows + fila] = W[fila] · xs[t * cols] para t < n (sin lista de tokens). Mismo reparto que `iq_down_n`. */
extern "C" __global__ void iq_matmul_n(const unsigned char *w, const float *xs, float *y, int rows, int cols, int n, int dtype, long rb)
{
    iq_down_dev(w, xs, y, rows, cols, n, 0, dtype, rb);
}

/* ---------------------------------------------------------------------------------------------------------------------
 * Atención del prefill por bloques en la GPU (KV f16 espejado en la tarjeta).
 *
 * Un bloque CUDA por (cabeza KV, grupo de tokens): las `group` cabezas de consulta que comparten la cabeza KV y `tb = 16 / group` tokens
 * consecutivos (hasta 16 consultas) recorren el contexto por tiles de 16 posiciones con softmax en línea; cada hilo es una dimensión de
 * salida (`hd` <= 256). K va en f32 y V en f16 en memoria compartida. El token `i` del bloque ve las posiciones `0 .. seq_antes + i`.
 * ------------------------------------------------------------------------------------------------------------------ */
#define AP_TILE 16

__device__ __forceinline__ float ap_f16(unsigned short h)
{
    float f;
    asm("cvt.f32.f16 %0, %1;" : "=f"(f) : "h"(h));
    return f;
}

extern "C" __global__ void attn_prefill_f16(const float *q, const unsigned short *K, const unsigned short *V, float *out, int n,
                                            int seq_antes, int heads, int kv_heads, int hd, int group)
{
    __shared__ float Qs[16][256];
    __shared__ float Ks[AP_TILE][256];
    __shared__ unsigned short Vs[AP_TILE][256];
    __shared__ float P[16][AP_TILE];
    __shared__ float mm[16], ll[16], alfa[16];
    int tid = (int)threadIdx.x;
    int kvh = (int)blockIdx.x;
    int tb = 16 / group;
    int tok0 = (int)blockIdx.y * tb;
    int ntok = n - tok0 < tb ? n - tok0 : tb;
    int qn = ntok * group;
    int kv_dim = kv_heads * hd;
    int q_dim = heads * hd;
    float inv = rsqrtf((float)hd);
    for (int qi = 0; qi < 16; qi++) {
        float v = 0.0f;
        if (qi < qn && tid < hd) {
            int tl = qi / group, g = qi % group;
            v = q[(long)(tok0 + tl) * q_dim + (long)(kvh * group + g) * hd + tid] * inv;
        }
        Qs[qi][tid] = v;
    }
    if (tid < 16) {
        mm[tid] = -INFINITY;
        ll[tid] = 0.0f;
    }
    float acc[16];
#pragma unroll
    for (int i = 0; i < 16; i++) {
        acc[i] = 0.0f;
    }
    int max_pos = seq_antes + tok0 + ntok;
    __syncthreads();
    for (int t0 = 0; t0 < max_pos; t0 += AP_TILE) {
        for (int r = 0; r < AP_TILE; r++) {
            int pos = t0 + r;
            float kk = 0.0f;
            unsigned short vv = 0;
            if (pos < max_pos && tid < hd) {
                kk = ap_f16(K[(long)pos * kv_dim + (long)kvh * hd + tid]);
                vv = V[(long)pos * kv_dim + (long)kvh * hd + tid];
            }
            Ks[r][tid] = kk;
            Vs[r][tid] = vv;
        }
        __syncthreads();
        {
            int qi = tid >> 4, p = tid & 15;
            float s = -INFINITY;
            if (qi < qn) {
                int tl = qi / group;
                int limite = seq_antes + tok0 + tl + 1;
                if (t0 + p < limite) {
                    float a = 0.0f;
                    for (int d = 0; d < hd; d++) {
                        a += Qs[qi][d] * Ks[p][d];
                    }
                    s = a;
                }
            }
            P[qi][p] = s;
        }
        __syncthreads();
        {
            int qi = (tid >> 5) * 2 + ((tid & 31) >> 4);
            int p = tid & 15;
            float s = P[qi][p];
            float mx = s;
            for (int o = 8; o > 0; o >>= 1) {
                mx = fmaxf(mx, __shfl_xor_sync(0xffffffffu, mx, o, 16));
            }
            float mold = mm[qi];
            float mnew = fmaxf(mold, mx);
            float al = 1.0f, pe = 0.0f;
            if (mnew != -INFINITY) {
                al = (mold == -INFINITY) ? 0.0f : expf(mold - mnew);
                pe = (s == -INFINITY) ? 0.0f : expf(s - mnew);
            }
            float sum = pe;
            for (int o = 8; o > 0; o >>= 1) {
                sum += __shfl_xor_sync(0xffffffffu, sum, o, 16);
            }
            P[qi][p] = pe;
            if (p == 0) {
                ll[qi] = ll[qi] * al + sum;
                mm[qi] = mnew;
                alfa[qi] = al;
            }
        }
        __syncthreads();
        if (tid < hd) {
#pragma unroll
            for (int qi = 0; qi < 16; qi++) {
                acc[qi] *= alfa[qi];
            }
            for (int p = 0; p < AP_TILE; p++) {
                float v = ap_f16(Vs[p][tid]);
#pragma unroll
                for (int qi = 0; qi < 16; qi++) {
                    acc[qi] += P[qi][p] * v;
                }
            }
        }
        __syncthreads();
    }
    if (tid < hd) {
        for (int qi = 0; qi < 16; qi++) {
            if (qi < qn && ll[qi] > 0.0f) {
                int tl = qi / group, g = qi % group;
                out[(long)(tok0 + tl) * q_dim + (long)(kvh * group + g) * hd + tid] = acc[qi] / ll[qi];
            }
        }
    }
}

/* ---------------------------------------------------------------------------------------------------------------------
 * Todos los expertos de un bloque en UN lanzamiento: el descriptor de cada experto (punteros, tipos, token base y cuenta) está en un
 * array de u64 (`IQ_DESC` por experto). Con 256 expertos por capa, un lanzamiento por experto era latencia pura (cada kernel tiene pocos
 * bloques y tarda lo mismo que uno grande). grid = (ceil(filas / 8), tiles máximos, expertos); los bloques sin trabajo salen enseguida.
 * ------------------------------------------------------------------------------------------------------------------ */
#define IQ_DESC 12 /* wg, wu, wd, abase, cnt, dg, du, dd, rbg, rbu, rbd, tokbase */

extern "C" __global__ void iq_gateup_multi(const unsigned long long *desc, const float *xs, const int *tok, float *act, int ffn, int h)
{
    const unsigned long long *d = desc + (long)blockIdx.z * IQ_DESC;
    int cnt = (int)d[4];
    int t0 = (int)blockIdx.y * IQ_TILE;
    if (t0 >= cnt) {
        return;
    }
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int fila = (int)blockIdx.x * 8 + warp;
    if (fila >= ffn) {
        return;
    }
    int abase = (int)d[3];
    int nt = cnt - t0 < IQ_TILE ? cnt - t0 : IQ_TILE;
    const int *tk = tok + (long)d[11];
    const unsigned char *rg = (const unsigned char *)d[0] + (long)fila * (long)d[8];
    const unsigned char *ru = (const unsigned char *)d[1] + (long)fila * (long)d[9];
    int dg = (int)d[5], du = (int)d[6];
    float ag[IQ_TILE], au[IQ_TILE];
#pragma unroll
    for (int t = 0; t < IQ_TILE; t++) {
        ag[t] = 0.0f;
        au[t] = 0.0f;
    }
    int units = h >> 5;
    for (int u = lane; u < units; u += 32) {
        float vg[32], vu[32];
        iq_decodificar(dg, rg, u, vg);
        iq_decodificar(du, ru, u, vu);
#pragma unroll
        for (int t = 0; t < IQ_TILE; t++) {
            if (t < nt) {
                const float4 *xp = (const float4 *)(xs + (long)tk[t0 + t] * h + (long)u * 32);
                float sg = 0.0f, su = 0.0f;
#pragma unroll
                for (int i = 0; i < 8; i++) {
                    float4 x = xp[i];
                    sg += vg[4 * i] * x.x + vg[4 * i + 1] * x.y + vg[4 * i + 2] * x.z + vg[4 * i + 3] * x.w;
                    su += vu[4 * i] * x.x + vu[4 * i + 1] * x.y + vu[4 * i + 2] * x.z + vu[4 * i + 3] * x.w;
                }
                ag[t] += sg;
                au[t] += su;
            }
        }
    }
#pragma unroll
    for (int t = 0; t < IQ_TILE; t++) {
        float g = iq_warp_sum(ag[t]);
        float up = iq_warp_sum(au[t]);
        if (lane == 0 && t < nt) {
            act[(long)(abase + t0 + t) * ffn + fila] = (g / (1.0f + expf(-g))) * up;
        }
    }
}

extern "C" __global__ void iq_down_multi(const unsigned long long *desc, const float *act, float *y, int rows, int cols)
{
    const unsigned long long *d = desc + (long)blockIdx.z * IQ_DESC;
    int cnt = (int)d[4];
    int t0 = (int)blockIdx.y * IQ_TILE;
    if (t0 >= cnt) {
        return;
    }
    iq_down_dev((const unsigned char *)d[2], act, y, rows, cols, cnt, (int)d[3], (int)d[7], (long)d[10]);
}

/* ---------------------------------------------------------------------------------------------------------------------
 * Productos por bloque con tensor cores (mma.sync m16n8k16, f16 → f32).
 *
 * La versión con FMA en f32 gastaba casi todo el tiempo en el producto (cada hilo recorría 16 tokens por unidad de pesos); aquí cada warp
 * descodifica 16 filas × 256 columnas de pesos a f16 en memoria compartida y el producto por los 16 tokens lo hacen los tensor cores. Los
 * pesos ya están cuantizados a ~3 bits y las activaciones a f16 pierden menos que cualquier formato Q8; se acumula en f32.
 *
 * Bloque de 128 hilos = 4 warps = 64 filas × 16 tokens; grid.x = ceil(filas / 64), grid.y = tiles de 16 tokens.
 * ------------------------------------------------------------------------------------------------------------------ */
#define MM_KC 128
#define MM_LD 136 /* halves por fila en shared (128 + 8 de relleno: sin conflictos de banco en los fragmentos) */
#define MM_NTT 4  /* tiles de 16 tokens por bloque: los pesos decodificados se reutilizan para MM_TOK tokens */
#define MM_TOK (16 * MM_NTT)

__device__ __forceinline__ unsigned short mm_h(float f)
{
    unsigned short h;
    asm("cvt.rn.f16.f32 %0, %1;" : "=h"(h) : "f"(f));
    return h;
}

__device__ __forceinline__ void mm_mma(float *c, unsigned a0, unsigned a1, unsigned a2, unsigned a3, unsigned b0, unsigned b1)
{
    asm volatile("mma.sync.aligned.m16n8k16.row.col.f32.f16.f16.f32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};"
                 : "+f"(c[0]), "+f"(c[1]), "+f"(c[2]), "+f"(c[3])
                 : "r"(a0), "r"(a1), "r"(a2), "r"(a3), "r"(b0), "r"(b1));
}

/* acc[tt][j][..] += W[filas del warp] · X[tokens del tile tt] (tt < MM_NTT tiles de 16 tokens, j = 0, 1: los dos grupos de 8). `tok == null`: los tokens
 * son consecutivos desde `tokoff`. Cada warp decodifica sus 16 filas × MM_KC columnas UNA vez y las usa con los MM_TOK tokens del bloque. */
template <bool XH>
__device__ void iq_mma_tile(int dtype, const unsigned char *w, long rb, int rows, int row_w, int K, const void *x, int xstride, const int *tok,
                            int tokoff, int t0, int nt, float acc[MM_NTT][2][4], unsigned short *Xs, unsigned short *Ws)
{
    int tid = (int)threadIdx.x;
    int lane = tid & 31;
    int g = lane >> 2, c4 = (lane & 3) * 2;
    for (int kc = 0; kc < K; kc += MM_KC) {
        /* tile de X: MM_TOK tokens × MM_KC columnas. Con X en f16 las cargas se lanzan antes de descodificar los pesos (su latencia queda tapada por la
         * descodificación) y se guardan en memoria compartida después; el `__syncthreads` del final de la vuelta anterior garantiza que `Xs` está libre. */
        uint4 xr[(MM_TOK * MM_KC) / (128 * 8)];
        if (XH) {
#pragma unroll
            for (int i = 0; i < (MM_TOK * MM_KC) / (128 * 8); i++) {
                int e = (tid + i * 128) * 8;
                int t = e / MM_KC, k = e % MM_KC;
                xr[i] = make_uint4(0, 0, 0, 0);
                if (t < nt && kc + k < K) {
                    long tr = tok ? (long)tok[t0 + t] : (long)(tokoff + t0 + t);
                    xr[i] = *(const uint4 *)((const unsigned short *)x + tr * xstride + kc + k);
                }
            }
        } else {
            for (int e = tid * 2; e < MM_TOK * MM_KC; e += 128 * 2) {
                int t = e / MM_KC, k = e % MM_KC;
                float v0 = 0.0f, v1 = 0.0f;
                if (t < nt && kc + k < K) {
                    long tr = tok ? (long)tok[t0 + t] : (long)(tokoff + t0 + t);
                    float2 v = *(const float2 *)((const float *)x + tr * xstride + kc + k);
                    v0 = v.x;
                    v1 = v.y;
                }
                *(unsigned *)(Xs + t * MM_LD + k) = (unsigned)mm_h(v0) | ((unsigned)mm_h(v1) << 16);
            }
            __syncthreads();
        }
        /* pesos del warp: 16 filas × 4 unidades de 32 → 2 unidades por lane */
        for (int i = 0; i < 2; i++) {
            int u = lane + 32 * i;
            int r = u >> 2, un = u & 3;
            float v[32];
            if (row_w + r < rows && kc + un * 32 < K) {
                iq_decodificar(dtype, w + (long)(row_w + r) * rb, (kc >> 5) + un, v);
            } else {
#pragma unroll
                for (int l = 0; l < 32; l++) {
                    v[l] = 0.0f;
                }
            }
#pragma unroll
            for (int l = 0; l < 32; l += 2) {
                *(unsigned *)(Ws + r * MM_LD + un * 32 + l) = (unsigned)mm_h(v[l]) | ((unsigned)mm_h(v[l + 1]) << 16);
            }
        }
        if (XH) {
#pragma unroll
            for (int i = 0; i < (MM_TOK * MM_KC) / (128 * 8); i++) {
                int e = (tid + i * 128) * 8;
                *(uint4 *)(Xs + (e / MM_KC) * MM_LD + e % MM_KC) = xr[i];
            }
            __syncthreads();
        }
        __syncwarp();
#pragma unroll
        for (int ks = 0; ks < MM_KC / 16; ks++) {
            int k0 = ks * 16;
            unsigned a0 = *(const unsigned *)(Ws + g * MM_LD + k0 + c4);
            unsigned a1 = *(const unsigned *)(Ws + (g + 8) * MM_LD + k0 + c4);
            unsigned a2 = *(const unsigned *)(Ws + g * MM_LD + k0 + 8 + c4);
            unsigned a3 = *(const unsigned *)(Ws + (g + 8) * MM_LD + k0 + 8 + c4);
#pragma unroll
            for (int tt = 0; tt < MM_NTT; tt++) {
                if (tt * 16 < nt) { /* uniforme en el bloque */
#pragma unroll
                    for (int j = 0; j < 2; j++) {
                        unsigned b0 = *(const unsigned *)(Xs + (tt * 16 + j * 8 + g) * MM_LD + k0 + c4);
                        unsigned b1 = *(const unsigned *)(Xs + (tt * 16 + j * 8 + g) * MM_LD + k0 + 8 + c4);
                        mm_mma(acc[tt][j], a0, a1, a2, a3, b0, b1);
                    }
                }
            }
        }
        __syncthreads();
    }
}

/* Escribe `acc` (filas del warp × 16 tokens) en y[(abase + t0 + token) * ldy + fila]. */
template <bool OH>
__device__ __forceinline__ void iq_mma_escribir(float acc[2][4], void *y, int ldy, int rows, int row_w, int abase, int t0, int nt, const int *dest)
{
    int lane = (int)threadIdx.x & 31;
    int g = lane >> 2, c4 = (lane & 3) * 2;
#pragma unroll
    for (int j = 0; j < 2; j++) {
#pragma unroll
        for (int q = 0; q < 4; q++) {
            int fila = row_w + g + (q >> 1) * 8;
            int t = j * 8 + c4 + (q & 1);
            if (fila < rows && t < nt) {
                long fila_y = dest ? (long)dest[abase + t0 + t] : (long)(abase + t0 + t);
                if (OH) {
                    ((unsigned short *)y)[fila_y * ldy + fila] = mm_h(acc[j][q]);
                } else {
                    ((float *)y)[fila_y * ldy + fila] = acc[j][q];
                }
            }
        }
    }
}

template <bool H>
__device__ __forceinline__ void gateup_cuerpo(const unsigned long long *desc, const void *xs, const int *tok, void *act, int ffn, int h)
{
    __shared__ __align__(16) unsigned short Xs[MM_TOK * MM_LD];
    __shared__ __align__(16) unsigned short Wsh[4][16 * MM_LD];
    const unsigned long long *d = desc + (long)blockIdx.z * IQ_DESC;
    int cnt = (int)d[4];
    int t0 = (int)blockIdx.y * MM_TOK;
    if (t0 >= cnt) {
        return; /* uniforme en el bloque */
    }
    int warp = (int)threadIdx.x >> 5;
    int row_w = (int)blockIdx.x * 64 + warp * 16;
    int nt = cnt - t0 < MM_TOK ? cnt - t0 : MM_TOK;
    float ag[MM_NTT][2][4], au[MM_NTT][2][4];
#pragma unroll
    for (int tt = 0; tt < MM_NTT; tt++) {
#pragma unroll
        for (int j = 0; j < 2; j++) {
#pragma unroll
            for (int q = 0; q < 4; q++) {
                ag[tt][j][q] = 0.0f;
                au[tt][j][q] = 0.0f;
            }
        }
    }
    const int *tk = tok + (long)d[11];
    iq_mma_tile<H>((int)d[5], (const unsigned char *)d[0], (long)d[8], ffn, row_w, h, xs, h, tk, 0, t0, nt, ag, Xs, Wsh[warp]);
    iq_mma_tile<H>((int)d[6], (const unsigned char *)d[1], (long)d[9], ffn, row_w, h, xs, h, tk, 0, t0, nt, au, Xs, Wsh[warp]);
#pragma unroll
    for (int tt = 0; tt < MM_NTT; tt++) {
#pragma unroll
        for (int j = 0; j < 2; j++) {
#pragma unroll
            for (int q = 0; q < 4; q++) {
                float gv = ag[tt][j][q];
                ag[tt][j][q] = (gv / (1.0f + expf(-gv))) * au[tt][j][q];
            }
        }
        if (tt * 16 < nt) {
            iq_mma_escribir<H>(ag[tt], act, ffn, ffn, row_w, (int)d[3], t0 + tt * 16, nt - tt * 16, (const int *)0);
        }
    }
}

template <bool H>
__device__ __forceinline__ void down_cuerpo(const unsigned long long *desc, const void *act, float *y, int rows, int cols, const int *dest)
{
    __shared__ __align__(16) unsigned short Xs[MM_TOK * MM_LD];
    __shared__ __align__(16) unsigned short Wsh[4][16 * MM_LD];
    const unsigned long long *d = desc + (long)blockIdx.z * IQ_DESC;
    int cnt = (int)d[4];
    int t0 = (int)blockIdx.y * MM_TOK;
    if (t0 >= cnt) {
        return;
    }
    int warp = (int)threadIdx.x >> 5;
    int row_w = (int)blockIdx.x * 64 + warp * 16;
    int nt = cnt - t0 < MM_TOK ? cnt - t0 : MM_TOK;
    float a[MM_NTT][2][4];
#pragma unroll
    for (int tt = 0; tt < MM_NTT; tt++) {
#pragma unroll
        for (int j = 0; j < 2; j++) {
#pragma unroll
            for (int q = 0; q < 4; q++) {
                a[tt][j][q] = 0.0f;
            }
        }
    }
    iq_mma_tile<H>((int)d[7], (const unsigned char *)d[2], (long)d[10], rows, row_w, cols, act, cols, (const int *)0, (int)d[3], t0, nt, a, Xs, Wsh[warp]);
    #pragma unroll
    for (int tt = 0; tt < MM_NTT; tt++) {
        if (tt * 16 < nt) {
            iq_mma_escribir<false>(a[tt], y, rows, rows, row_w, (int)d[3], t0 + tt * 16, nt - tt * 16, dest);
        }
    }
}

/* Matriz densa por bloque: y[t * rows + fila] = W[fila] · xs[t * cols], t < n. grid = (ceil(rows / 64), ceil(n / 16)). */
template <bool H>
__device__ __forceinline__ void matmul_cuerpo(const unsigned char *w, const void *xs, float *y, int rows, int cols, int n, int dtype, long rb)
{
    __shared__ __align__(16) unsigned short Xs[MM_TOK * MM_LD];
    __shared__ __align__(16) unsigned short Wsh[4][16 * MM_LD];
    int warp = (int)threadIdx.x >> 5;
    int row_w = (int)blockIdx.x * 64 + warp * 16;
    int t0 = (int)blockIdx.y * MM_TOK;
    int nt = n - t0 < MM_TOK ? n - t0 : MM_TOK;
    float a[MM_NTT][2][4];
#pragma unroll
    for (int tt = 0; tt < MM_NTT; tt++) {
#pragma unroll
        for (int j = 0; j < 2; j++) {
#pragma unroll
            for (int q = 0; q < 4; q++) {
                a[tt][j][q] = 0.0f;
            }
        }
    }
    iq_mma_tile<H>(dtype, w, rb, rows, row_w, cols, xs, cols, (const int *)0, 0, t0, nt, a, Xs, Wsh[warp]);
    #pragma unroll
    for (int tt = 0; tt < MM_NTT; tt++) {
        if (tt * 16 < nt) {
            iq_mma_escribir<false>(a[tt], y, rows, rows, row_w, 0, t0 + tt * 16, nt - tt * 16, (const int *)0);
        }
    }
}

extern "C" __global__ void __launch_bounds__(128) iq_gateup_mma(const unsigned long long *desc, const float *xs, const int *tok, float *act, int ffn, int h)
{
    gateup_cuerpo<false>(desc, xs, tok, act, ffn, h);
}
/* Variante con X y la salida (act) en f16: la entrada viene de `cvt_f16` y `act` la lee `iq_down_mma_h`. */
extern "C" __global__ void __launch_bounds__(128) iq_gateup_mma_h(const unsigned long long *desc, const unsigned short *xs, const int *tok, unsigned short *act, int ffn, int h)
{
    gateup_cuerpo<true>(desc, xs, tok, act, ffn, h);
}
extern "C" __global__ void __launch_bounds__(128) iq_down_mma(const unsigned long long *desc, const float *act, float *y, int rows, int cols, const int *dest)
{
    down_cuerpo<false>(desc, act, y, rows, cols, dest);
}
extern "C" __global__ void __launch_bounds__(128) iq_down_mma_h(const unsigned long long *desc, const unsigned short *act, float *y, int rows, int cols, const int *dest)
{
    down_cuerpo<true>(desc, act, y, rows, cols, dest);
}
extern "C" __global__ void __launch_bounds__(128) iq_matmul_mma(const unsigned char *w, const float *xs, float *y, int rows, int cols, int n, int dtype, long rb)
{
    matmul_cuerpo<false>(w, xs, y, rows, cols, n, dtype, rb);
}
extern "C" __global__ void __launch_bounds__(128) iq_matmul_mma_h(const unsigned char *w, const unsigned short *xs, float *y, int rows, int cols, int n, int dtype, long rb)
{
    matmul_cuerpo<true>(w, xs, y, rows, cols, n, dtype, rb);
}
/* f32 → f16, de dos en dos: grid = ceil(n / 2 / 256) */
extern "C" __global__ void cvt_f16(const float *x, unsigned short *xh, long n)
{
    long i = ((long)blockIdx.x * blockDim.x + threadIdx.x) * 2;
    if (i + 1 < n) {
        float2 v = *(const float2 *)(x + i);
        *(unsigned *)(xh + i) = (unsigned)mm_h(v.x) | ((unsigned)mm_h(v.y) << 16);
    } else if (i < n) {
        xh[i] = mm_h(x[i]);
    }
}

/* ---------------------------------------------------------------------------------------------------------------------
 * Producto por bloque con tensor cores enteros (mma.sync m16n8k32 s8) para pesos Q8_0 de sosomodel (escala f32 + 32 i8 por bloque de 36 B).
 * Las activaciones llegan cuantizadas a i8 por grupos de 32 (`dec_quant_x` sobre todo el bloque): xq[t * cols + k], xd[t * cols / 32 + k / 32].
 * Sin descodificar: los pesos entran tal cual a los fragmentos; cada mma cubre un bloque de 32 y su resultado se escala con d_w[fila] · d_x[token].
 * Bloque de 128 hilos = 4 warps = 64 filas × 64 tokens; cada vuelta consume 128 columnas (4 bloques = 144 B por fila, alineados a 16).
 * ------------------------------------------------------------------------------------------------------------------ */
#define QM_ROWS 64
#define QM_TOK 64
#define QM_LDW 176 /* bytes por fila de pesos en shared: 144 + relleno (sin conflictos de banco en los fragmentos) */
#define QM_LDX 144 /* bytes por token en shared: 128 + 16 */

__device__ __forceinline__ void mma_s8(int *c, unsigned a0, unsigned a1, unsigned a2, unsigned a3, unsigned b0, unsigned b1)
{
    asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};"
                 : "+r"(c[0]), "+r"(c[1]), "+r"(c[2]), "+r"(c[3])
                 : "r"(a0), "r"(a1), "r"(a2), "r"(a3), "r"(b0), "r"(b1));
}

/* y[t * rows + fila] = W[fila] · x[t]; grid = (ceil(rows / 64), ceil(n / 64)); cols múltiplo de 128 */
extern "C" __global__ void __launch_bounds__(128) q8_mmq(const unsigned char *w, const signed char *xq, const float *xd, float *y, int rows, int cols, int n, long rb)
{
    __shared__ __align__(16) unsigned char Ws[QM_ROWS * QM_LDW];
    __shared__ __align__(16) signed char Xs[QM_TOK * QM_LDX];
    __shared__ __align__(16) float Ds[QM_TOK * 4];
    int tid = (int)threadIdx.x;
    int warp = tid >> 5, lane = tid & 31;
    int g = lane >> 2, c4 = lane & 3;
    int row0 = (int)blockIdx.x * QM_ROWS, t0 = (int)blockIdx.y * QM_TOK;
    float acc[8][4];
#pragma unroll
    for (int j = 0; j < 8; j++) {
#pragma unroll
        for (int q = 0; q < 4; q++) {
            acc[j][q] = 0.0f;
        }
    }
    for (int g128 = 0; g128 < cols / 128; g128++) {
        for (int e = tid; e < QM_ROWS * 9; e += 128) {
            int r = e / 9, i = e % 9;
            uint4 v = make_uint4(0, 0, 0, 0);
            if (row0 + r < rows) {
                v = __ldg((const uint4 *)(w + (long)(row0 + r) * rb + (long)g128 * 144) + i);
            }
            *(uint4 *)(Ws + r * QM_LDW + i * 16) = v;
        }
        for (int e = tid; e < QM_TOK * 8; e += 128) {
            int t = e >> 3, i = e & 7;
            uint4 v = make_uint4(0, 0, 0, 0);
            if (t0 + t < n) {
                v = __ldg((const uint4 *)(xq + (long)(t0 + t) * cols + g128 * 128) + i);
            }
            *(uint4 *)(Xs + t * QM_LDX + i * 16) = v;
        }
        if (tid < QM_TOK) {
            float4 v = make_float4(0.f, 0.f, 0.f, 0.f);
            if (t0 + tid < n) {
                v = __ldg((const float4 *)(xd + (long)(t0 + tid) * (cols / 32) + g128 * 4));
            }
            *(float4 *)(Ds + tid * 4) = v;
        }
        __syncthreads();
#pragma unroll
        for (int b = 0; b < 4; b++) {
            const unsigned *wr0 = (const unsigned *)(Ws + (warp * 16 + g) * QM_LDW) + 9 * b;
            const unsigned *wr1 = (const unsigned *)(Ws + (warp * 16 + g + 8) * QM_LDW) + 9 * b;
            unsigned a0 = wr0[1 + c4], a1 = wr1[1 + c4], a2 = wr0[5 + c4], a3 = wr1[5 + c4];
            float d0 = __uint_as_float(wr0[0]), d1 = __uint_as_float(wr1[0]);
#pragma unroll
            for (int j = 0; j < 8; j++) {
                const unsigned *xr = (const unsigned *)(Xs + (j * 8 + g) * QM_LDX) + 8 * b;
                int c[4] = {0, 0, 0, 0};
                mma_s8(c, a0, a1, a2, a3, xr[c4], xr[4 + c4]);
                float dxa = Ds[(j * 8 + 2 * c4) * 4 + b], dxb = Ds[(j * 8 + 2 * c4 + 1) * 4 + b];
                acc[j][0] += (float)c[0] * (d0 * dxa);
                acc[j][1] += (float)c[1] * (d0 * dxb);
                acc[j][2] += (float)c[2] * (d1 * dxa);
                acc[j][3] += (float)c[3] * (d1 * dxb);
            }
        }
        __syncthreads();
    }
#pragma unroll
    for (int j = 0; j < 8; j++) {
#pragma unroll
        for (int q = 0; q < 4; q++) {
            int fila = row0 + warp * 16 + g + (q >> 1) * 8;
            int t = t0 + j * 8 + 2 * c4 + (q & 1);
            if (fila < rows && t < n) {
                y[(long)t * rows + fila] = acc[j][q];
            }
        }
    }
}

/* out[t * h + i] = Σ_j wd[t * top_k + j] · y[(t * top_k + j) * h + i] sobre las asignaciones resueltas en la GPU (peso 0 = no se toca).
 * grid = (ceil(h / 256), n). El orden de las sumas es el del ranking. */
extern "C" __global__ void iq_combinar(const float *y, const float *wd, float *out, int top_k, int h, int acum)
{
    int i = (int)blockIdx.x * 256 + (int)threadIdx.x;
    if (i >= h) {
        return;
    }
    int t = (int)blockIdx.y;
    float a = 0.0f;
    for (int j = 0; j < top_k; j++) {
        float w = wd[t * top_k + j];
        if (w != 0.0f) {
            a += w * y[((long)t * top_k + j) * h + i];
        }
    }
    out[(long)t * h + i] = acum ? out[(long)t * h + i] + a : a;
}

/* ---------------------------------------------------------------------------------------------------------------------
 * Gated DeltaNet (Qwen3.5/3.6) de un bloque de tokens en la GPU.
 *   gdn_conv_silu: conv causal de kernel `ker` + SiLU de todos los tokens (hilo por (token, canal)); `hist` = estado + entradas del bloque.
 *   gdn_rec:       recurrencia por cabeza v (un bloque CUDA por cabeza, 256 hilos): el estado S[128][128] vive en registros (cada hilo, una
 *                  columna y la mitad de las filas), los tokens se recorren en orden. Incluye la norma L2 de q/k, la escala, la norma RMS
 *                  con peso y la puerta SiLU(z). Sólo d = 128.
 * ------------------------------------------------------------------------------------------------------------------ */
extern "C" __global__ void gdn_conv_silu(const float *hist, const float *cw, float *up, int n, int qkv_dim, int ker)
{
    int ch = (int)blockIdx.x * 256 + (int)threadIdx.x;
    int t = (int)blockIdx.y;
    if (ch >= qkv_dim) {
        return;
    }
    float acc = 0.0f;
    for (int k = 0; k < ker; k++) {
        acc += cw[(long)ch * ker + k] * hist[(long)(t + k) * qkv_dim + ch];
    }
    up[(long)t * qkv_dim + ch] = acc / (1.0f + expf(-acc));
}

__device__ __forceinline__ float gdn_suma128(float v, float *red)
{
    /* suma sobre los hilos 0..127 (4 warps); el resultado lo ven todos. Los hilos >= 128 aportan 0. */
    int tid = (int)threadIdx.x;
    for (int o = 16; o > 0; o >>= 1) {
        v += __shfl_xor_sync(0xffffffffu, v, o);
    }
    if ((tid & 31) == 0 && tid < 128) {
        red[tid >> 5] = v;
    }
    __syncthreads();
    float r = red[0] + red[1] + red[2] + red[3];
    __syncthreads();
    return r;
}

extern "C" __global__ void __launch_bounds__(128) gdn_rec(const float *up, const float *z, const float *decay, const float *beta, const float *nw,
                                                          float *state, float *out, int n, int n_k, int n_v, float eps)
{
    /* Una cabeza V por bloque, un hilo por columna: el estado entero de la columna (128 filas) vive en registros, así que los productos escalares sobre filas no cruzan hilos.
     * q y k crudos del token siguiente se cargan (y se suman sus cuadrados) mientras se calcula el actual: st[etapa][0..127] = q, [128..255] = k, [256] = Σq², [257] = Σk². */
    __shared__ __align__(16) float st[2][260];
    __shared__ float red[4];
    int tid = (int)threadIdx.x, lane = tid & 31;
    int vh = (int)blockIdx.x;
    int kh = vh % n_k;
    int qk_dim = n_k * 128, qkv_dim = 2 * qk_dim + n_v * 128, v_dim = n_v * 128;
    float *S = state + (long)vh * 128 * 128;
    float s[128];
#pragma unroll
    for (int r = 0; r < 128; r++) {
        s[r] = S[(long)r * 128 + tid];
    }
    const float escala = rsqrtf(128.0f);
    const float nwc = nw[tid];
    float4 nx = make_float4(0.f, 0.f, 0.f, 0.f);
    if (tid < 64) {
        const float *b0 = up + (tid < 32 ? kh * 128 : qk_dim + kh * 128);
        nx = *(const float4 *)(b0 + 4 * (tid & 31));
        float p = nx.x * nx.x + nx.y * nx.y + nx.z * nx.z + nx.w * nx.w;
#pragma unroll
        for (int o = 16; o > 0; o >>= 1) {
            p += __shfl_xor_sync(0xffffffffu, p, o);
        }
        *(float4 *)(&st[0][4 * tid]) = nx;
        if (lane == 0) {
            st[0][256 + (tid >> 5)] = p;
        }
    }
    float vn = up[2 * qk_dim + vh * 128 + tid], gn = decay[vh], bn = beta[vh];
    for (int t = 0; t < n; t++) {
        __syncthreads();
        const int cur = t & 1;
        const bool hay = t + 1 < n;
        if (hay && tid < 64) {
            const float *b1 = up + (long)(t + 1) * qkv_dim + (tid < 32 ? kh * 128 : qk_dim + kh * 128);
            nx = *(const float4 *)(b1 + 4 * (tid & 31));
        }
        const float vv = vn;
        const float g = gn, bt = bn;
        if (hay) {
            vn = up[(long)(t + 1) * qkv_dim + 2 * qk_dim + vh * 128 + tid];
            gn = decay[(t + 1) * n_v + vh];
            bn = beta[(t + 1) * n_v + vh];
        }
        const float zz = z[(long)t * v_dim + vh * 128 + tid];
        const float4 *qq = (const float4 *)st[cur], *kk = (const float4 *)(st[cur] + 128);
        const float invq = 1.0f / sqrtf(st[cur][256] + eps), invk = 1.0f / sqrtf(st[cur][257] + eps);
        float kv0 = 0.f, kv1 = 0.f, kv2 = 0.f, kv3 = 0.f;
#pragma unroll
        for (int j = 0; j < 32; j++) {
            float4 k4 = kk[j];
            kv0 += s[4 * j] * k4.x;
            kv1 += s[4 * j + 1] * k4.y;
            kv2 += s[4 * j + 2] * k4.z;
            kv3 += s[4 * j + 3] * k4.w;
            if ((j & 7) == 7) {
                asm volatile("" ::: "memory");
            }
        }
        float kv = ((kv0 + kv1) + (kv2 + kv3)) * invk;
        float kd = (vv - g * kv) * bt * invk;
        float o0 = 0.f, o1 = 0.f, o2 = 0.f, o3 = 0.f;
#pragma unroll
        for (int j = 0; j < 32; j++) {
            float4 k4 = kk[j], q4 = qq[j];
            float n0 = g * s[4 * j] + k4.x * kd, n1 = g * s[4 * j + 1] + k4.y * kd, n2 = g * s[4 * j + 2] + k4.z * kd, n3 = g * s[4 * j + 3] + k4.w * kd;
            s[4 * j] = n0;
            s[4 * j + 1] = n1;
            s[4 * j + 2] = n2;
            s[4 * j + 3] = n3;
            o0 += n0 * q4.x;
            o1 += n1 * q4.y;
            o2 += n2 * q4.z;
            o3 += n3 * q4.w;
            if ((j & 7) == 7) {
                asm volatile("" ::: "memory");
            }
        }
        float val = ((o0 + o1) + (o2 + o3)) * invq * escala;
        float ssv = val * val;
#pragma unroll
        for (int o = 16; o > 0; o >>= 1) {
            ssv += __shfl_xor_sync(0xffffffffu, ssv, o);
        }
        if (lane == 0) {
            red[tid >> 5] = ssv;
        }
        if (hay && tid < 64) {
            float p = nx.x * nx.x + nx.y * nx.y + nx.z * nx.z + nx.w * nx.w;
#pragma unroll
            for (int o = 16; o > 0; o >>= 1) {
                p += __shfl_xor_sync(0xffffffffu, p, o);
            }
            *(float4 *)(&st[cur ^ 1][4 * tid]) = nx;
            if (lane == 0) {
                st[cur ^ 1][256 + (tid >> 5)] = p;
            }
        }
        __syncthreads();
        float ss = (red[0] + red[1]) + (red[2] + red[3]);
        float sil = zz / (1.0f + expf(-zz));
        out[(long)t * v_dim + vh * 128 + tid] = val * rsqrtf(ss / 128.0f + eps) * nwc * sil;
    }
#pragma unroll
    for (int r = 0; r < 128; r++) {
        S[(long)r * 128 + tid] = s[r];
    }
}

/* Variante de gdn_conv_silu con las entradas en dos orígenes: `cs` (estado de la conv, `ker - 1` filas) y `qkv` (las del bloque). */
extern "C" __global__ void gdn_conv_silu2(const float *cs, const float *qkv, const float *cw, float *up, int n, int qkv_dim, int ker)
{
    int ch = (int)blockIdx.x * 256 + (int)threadIdx.x;
    int t = (int)blockIdx.y;
    if (ch >= qkv_dim) {
        return;
    }
    int prev = ker - 1;
    float acc = 0.0f;
    for (int k = 0; k < ker; k++) {
        int r = t + k;
        float x = r < prev ? cs[(long)r * qkv_dim + ch] : qkv[(long)(r - prev) * qkv_dim + ch];
        acc += cw[(long)ch * ker + k] * x;
    }
    up[(long)t * qkv_dim + ch] = acc / (1.0f + expf(-acc));
}

/* decay[t,i] = exp(softplus(alpha[t,i] + dt[i]) * a[i]);  beta[t,i] = sigmoid(beta_raw[t,i]). Un hilo por (token, cabeza). */
extern "C" __global__ void gdn_params(const float *alpha, const float *braw, const float *dt, const float *a, float *decay, float *beta, int n, int n_v)
{
    int i = (int)blockIdx.x * 256 + (int)threadIdx.x;
    if (i >= n * n_v) {
        return;
    }
    int h = i % n_v;
    float x = alpha[i] + dt[h];
    float sp = x > 20.0f ? x : log1pf(expf(x));
    decay[i] = expf(sp * a[h]);
    beta[i] = 1.0f / (1.0f + expf(-braw[i]));
}

/* Copia las `filas` últimas filas de `src` (n filas de `ancho`) a `dst` (estado de la conv para el bloque siguiente). */
extern "C" __global__ void gdn_cola(const float *qkv, float *dst, int n, int ancho, int filas)
{
    int i = (int)blockIdx.x * 256 + (int)threadIdx.x;
    if (i >= filas * ancho) {
        return;
    }
    dst[i] = qkv[(long)(n - filas) * ancho + i];
}

/* ---------------------------------------------------------------------------------------------------------------------
 * Decodificación de UN token con todo el modelo en la GPU (grafo residente). Los productos matriz×vector usan `iq_matmul_mma` con n = 1.
 * Las posiciones viven en memoria del dispositivo (`dyn[0] = pos`, `dyn[1] = seq = pos + 1`): así los lanzamientos no cambian de un token al
 * siguiente y se pueden capturar en un grafo CUDA.
 * ------------------------------------------------------------------------------------------------------------------ */
__device__ __forceinline__ float dec_bsum256(float v, float *red)
{
    /* suma sobre los 256 hilos del bloque; todos ven el resultado. `red` tiene 8 floats. */
    int tid = (int)threadIdx.x;
    for (int o = 16; o > 0; o >>= 1) {
        v += __shfl_xor_sync(0xffffffffu, v, o);
    }
    if ((tid & 31) == 0) {
        red[tid >> 5] = v;
    }
    __syncthreads();
    float r = 0.0f;
#pragma unroll
    for (int i = 0; i < 8; i++) {
        r += red[i];
    }
    __syncthreads();
    return r;
}

/* y = x · rsqrt(mean(x²) + eps) · w. Un bloque de 256 hilos. */
extern "C" __global__ void dec_rmsnorm(const float *x, const float *w, float *y, int n, float eps)
{
    __shared__ float red[8];
    float ss = 0.0f;
    for (int i = (int)threadIdx.x; i < n; i += 256) {
        ss += x[i] * x[i];
    }
    ss = dec_bsum256(ss, red);
    float sc = rsqrtf(ss / (float)n + eps);
    for (int i = (int)threadIdx.x; i < n; i += 256) {
        y[i] = x[i] * sc * w[i];
    }
}

extern "C" __global__ void dec_add(float *a, const float *b, int n)
{
    int i = (int)blockIdx.x * 256 + (int)threadIdx.x;
    if (i < n) {
        a[i] += b[i];
    }
}

extern "C" __global__ void dec_swiglu(float *up, const float *gate, int n)
{
    int i = (int)blockIdx.x * 256 + (int)threadIdx.x;
    if (i < n) {
        float g = gate[i];
        up[i] = (g / (1.0f + expf(-g))) * up[i];
    }
}

/* Conv causal de un paso + SiLU (estado `cs` de `ker - 1` filas, se desplaza en el sitio). Un hilo por canal. */
extern "C" __global__ void dec_gdn_conv(float *cs, const float *qkv, const float *cw, float *up, int qkv_dim, int ker)
{
    int ch = (int)blockIdx.x * 256 + (int)threadIdx.x;
    if (ch >= qkv_dim) {
        return;
    }
    int prev = ker - 1;
    float acc = 0.0f;
    for (int k = 0; k < prev; k++) {
        acc += cw[(long)ch * ker + k] * cs[(long)k * qkv_dim + ch];
    }
    float x = qkv[ch];
    acc += cw[(long)ch * ker + prev] * x;
    up[ch] = acc / (1.0f + expf(-acc));
    for (int r = 0; r + 1 < prev; r++) {
        cs[(long)r * qkv_dim + ch] = cs[(long)(r + 1) * qkv_dim + ch];
    }
    if (prev > 0) {
        cs[(long)(prev - 1) * qkv_dim + ch] = x;
    }
}

/* Atención con puerta, preparación: parte q/puerta, norma por cabeza, RoPE parcial (parejas (2i, 2i+1) de los primeros `rope_n`), y escribe K/V
 * (f16) en la fila `pos` del KV del dispositivo y en el búfer de salida al host. Un bloque por cabeza (heads de consulta + heads KV), 256 hilos. */
extern "C" __global__ void dec_attn_prep(const float *qf, const float *qnw, const float *knw, const float *kk, const float *vv, float *q_out,
                                         float *gate_out, unsigned short *Kc, unsigned short *Vc, unsigned short *kst, unsigned short *vst,
                                         const int *dyn, int heads, int kv_heads, int hd, int rope_n, float theta, float eps)
{
    __shared__ float red[8];
    __shared__ float xs[256];
    int b = (int)blockIdx.x;
    int tid = (int)threadIdx.x;
    int pos = dyn[0];
    int kv_dim = kv_heads * hd;
    int es_q = b < heads;
    float v = 0.0f, gt = 0.0f;
    if (tid < hd) {
        if (es_q) {
            v = qf[(long)b * 2 * hd + tid];
            gt = qf[(long)b * 2 * hd + hd + tid];
        } else {
            v = kk[(long)(b - heads) * hd + tid];
        }
    }
    float ss = dec_bsum256(v * v, red);
    float sc = rsqrtf(ss / (float)hd + eps);
    float xn = v * sc * (tid < hd ? (es_q ? qnw[tid] : knw[tid]) : 0.0f);
    xs[tid] = xn;
    __syncthreads();
    float out = xn;
    if (tid < rope_n) {
        int i = tid >> 1;
        float freq = powf(theta, -2.0f * (float)i / (float)rope_n);
        float ang = (float)pos * freq;
        float sn, cs;
        sincosf(ang, &sn, &cs);
        float a = xs[2 * i], bb = xs[2 * i + 1];
        out = (tid & 1) ? (a * sn + bb * cs) : (a * cs - bb * sn);
    }
    if (tid < hd) {
        if (es_q) {
            q_out[(long)b * hd + tid] = out;
            gate_out[(long)b * hd + tid] = gt;
        } else {
            int kh = b - heads;
            unsigned short kh16 = mm_h(out);
            Kc[(long)pos * kv_dim + kh * hd + tid] = kh16;
            kst[kh * hd + tid] = kh16;
            unsigned short vh16 = mm_h(vv[(long)kh * hd + tid]);
            Vc[(long)pos * kv_dim + kh * hd + tid] = vh16;
            vst[kh * hd + tid] = vh16;
        }
    }
}

/* Preparación de la atención de un BLOQUE de `n` tokens (misma cuenta que dec_attn_prep): grid = (heads + kv_heads, n). Cada bloque normaliza y
 * rota una cabeza de consulta o KV del token `blockIdx.y` (posición `pos0 + t`), separa la puerta, y escribe K y V (f16) en la fila `pos0 + t` del
 * KV del dispositivo. */
extern "C" __global__ void att_prep_bloque(const float *qf, const float *qnw, const float *knw, const float *kk, const float *vv, float *q_out,
                                           float *gate_out, unsigned short *Kc, unsigned short *Vc, int pos0, int heads, int kv_heads, int hd,
                                           int rope_n, float theta, float eps)
{
    __shared__ float red[8];
    __shared__ float xs[256];
    int b = (int)blockIdx.x;
    int t = (int)blockIdx.y;
    int tid = (int)threadIdx.x;
    int pos = pos0 + t;
    int kv_dim = kv_heads * hd;
    int q_full = 2 * heads * hd;
    int es_q = b < heads;
    float v = 0.0f, gt = 0.0f;
    if (tid < hd) {
        if (es_q) {
            v = qf[(long)t * q_full + (long)b * 2 * hd + tid];
            gt = qf[(long)t * q_full + (long)b * 2 * hd + hd + tid];
        } else {
            v = kk[(long)t * kv_dim + (long)(b - heads) * hd + tid];
        }
    }
    float ss = dec_bsum256(v * v, red);
    float sc = rsqrtf(ss / (float)hd + eps);
    float xn = v * sc * (tid < hd ? (es_q ? qnw[tid] : knw[tid]) : 0.0f);
    xs[tid] = xn;
    __syncthreads();
    float out = xn;
    if (tid < rope_n) {
        int i = tid >> 1;
        float freq = powf(theta, -2.0f * (float)i / (float)rope_n);
        float ang = (float)pos * freq;
        float sn, cs;
        sincosf(ang, &sn, &cs);
        float a = xs[2 * i], bb = xs[2 * i + 1];
        out = (tid & 1) ? (a * sn + bb * cs) : (a * cs - bb * sn);
    }
    if (tid < hd) {
        if (es_q) {
            q_out[((long)t * heads + b) * hd + tid] = out;
            gate_out[((long)t * heads + b) * hd + tid] = gt;
        } else {
            int kh = b - heads;
            Kc[(long)pos * kv_dim + kh * hd + tid] = mm_h(out);
            Vc[(long)pos * kv_dim + kh * hd + tid] = mm_h(vv[(long)t * kv_dim + (long)kh * hd + tid]);
        }
    }
}

/* Atención de prefill con tensor cores (mma m16n8k16 f16): grid = (kv_heads, ceil(n / (64 / group))), 128 hilos (4 warps). Las 64 filas de un bloque
 * son (token, cabeza de la consulta) de una cabeza KV: cada warp lleva 16 filas. Q (ya escalada) va en f16 a memoria compartida; K y V se cargan por
 * tiles de 32 posiciones; softmax en línea en registros y P·V con P como fragmento A. `group` debe dividir 64 y `HD` ser 128 o 256. Memoria
 * compartida dinámica: (64 + 2 * 32) * (HD + 8) * 2 bytes. */
#define AM_ROWS 64
#define AM_TK 32

__device__ __forceinline__ unsigned am_pack(float lo, float hi)
{
    return (unsigned)mm_h(lo) | ((unsigned)mm_h(hi) << 16);
}

template <int HD>
__device__ __forceinline__ void attn_mma_cuerpo(const float *q, const unsigned short *K, const unsigned short *V, float *out, int n, int seq_antes,
                                                int heads, int kv_heads, int group)
{
    extern __shared__ unsigned short am_sm[];
    const int LD = HD + 8;
    unsigned short *Qs = am_sm;
    unsigned short *Ks = Qs + AM_ROWS * LD;
    unsigned short *Vs = Ks + AM_TK * LD;
    int tid = (int)threadIdx.x, warp = tid >> 5, lane = tid & 31;
    int kvh = (int)blockIdx.x;
    int tb = AM_ROWS / group;
    int tok0 = (int)blockIdx.y * tb;
    int kv_dim = kv_heads * HD, q_dim = heads * HD;
    float inv = rsqrtf((float)HD);
    for (int idx = tid; idx < AM_ROWS * (HD / 2); idx += 128) {
        int r = idx / (HD / 2), d2 = idx % (HD / 2);
        int tok = tok0 + r / group, g = r % group;
        float v0 = 0.0f, v1 = 0.0f;
        if (tok < n) {
            float2 v = *(const float2 *)&q[(long)tok * q_dim + (long)(kvh * group + g) * HD + 2 * d2];
            v0 = v.x * inv;
            v1 = v.y * inv;
        }
        *(unsigned *)&Qs[r * LD + 2 * d2] = am_pack(v0, v1);
    }
    int ntok = n - tok0 < tb ? n - tok0 : tb;
    int max_pos = seq_antes + tok0 + ntok;
    int fila0 = warp * 16 + (lane >> 2), fila1 = fila0 + 8;
    int lim0 = min(seq_antes + tok0 + fila0 / group + 1, max_pos), lim1 = min(seq_antes + tok0 + fila1 / group + 1, max_pos);
    float m0 = -INFINITY, m1 = -INFINITY, l0 = 0.0f, l1 = 0.0f;
    float o[HD / 8][4];
#pragma unroll
    for (int j = 0; j < HD / 8; j++) {
        o[j][0] = o[j][1] = o[j][2] = o[j][3] = 0.0f;
    }
    __syncthreads();
    for (int t0 = 0; t0 < max_pos; t0 += AM_TK) {
        for (int idx = tid; idx < AM_TK * (HD / 8); idx += 128) {
            int key = idx / (HD / 8), c = idx % (HD / 8);
            int pos = t0 + key;
            uint4 kk = make_uint4(0, 0, 0, 0), vv = make_uint4(0, 0, 0, 0);
            if (pos < max_pos) {
                kk = *(const uint4 *)&K[(long)pos * kv_dim + (long)kvh * HD + c * 8];
                vv = *(const uint4 *)&V[(long)pos * kv_dim + (long)kvh * HD + c * 8];
            }
            *(uint4 *)&Ks[key * LD + c * 8] = kk;
            *(uint4 *)&Vs[key * LD + c * 8] = vv;
        }
        __syncthreads();
        float s[AM_TK / 8][4];
#pragma unroll
        for (int j = 0; j < AM_TK / 8; j++) {
            s[j][0] = s[j][1] = s[j][2] = s[j][3] = 0.0f;
        }
#pragma unroll 4
        for (int ks = 0; ks < HD / 16; ks++) {
            int kc = ks * 16 + (lane & 3) * 2;
            unsigned a0 = *(unsigned *)&Qs[fila0 * LD + kc], a1 = *(unsigned *)&Qs[fila1 * LD + kc];
            unsigned a2 = *(unsigned *)&Qs[fila0 * LD + kc + 8], a3 = *(unsigned *)&Qs[fila1 * LD + kc + 8];
#pragma unroll
            for (int j = 0; j < AM_TK / 8; j++) {
                int kr = (j * 8 + (lane >> 2)) * LD + kc;
                mm_mma(s[j], a0, a1, a2, a3, *(unsigned *)&Ks[kr], *(unsigned *)&Ks[kr + 8]);
            }
        }
        float mx0 = -INFINITY, mx1 = -INFINITY;
#pragma unroll
        for (int j = 0; j < AM_TK / 8; j++) {
            int p = t0 + j * 8 + (lane & 3) * 2;
            if (p >= lim0) s[j][0] = -INFINITY;
            if (p + 1 >= lim0) s[j][1] = -INFINITY;
            if (p >= lim1) s[j][2] = -INFINITY;
            if (p + 1 >= lim1) s[j][3] = -INFINITY;
            mx0 = fmaxf(mx0, fmaxf(s[j][0], s[j][1]));
            mx1 = fmaxf(mx1, fmaxf(s[j][2], s[j][3]));
        }
        mx0 = fmaxf(mx0, __shfl_xor_sync(0xffffffffu, mx0, 1));
        mx0 = fmaxf(mx0, __shfl_xor_sync(0xffffffffu, mx0, 2));
        mx1 = fmaxf(mx1, __shfl_xor_sync(0xffffffffu, mx1, 1));
        mx1 = fmaxf(mx1, __shfl_xor_sync(0xffffffffu, mx1, 2));
        float mn0 = fmaxf(m0, mx0), mn1 = fmaxf(m1, mx1);
        float al0 = mn0 == -INFINITY ? 1.0f : (m0 == -INFINITY ? 0.0f : expf(m0 - mn0));
        float al1 = mn1 == -INFINITY ? 1.0f : (m1 == -INFINITY ? 0.0f : expf(m1 - mn1));
        m0 = mn0;
        m1 = mn1;
        l0 *= al0;
        l1 *= al1;
#pragma unroll
        for (int j = 0; j < AM_TK / 8; j++) {
            s[j][0] = s[j][0] == -INFINITY ? 0.0f : expf(s[j][0] - mn0);
            s[j][1] = s[j][1] == -INFINITY ? 0.0f : expf(s[j][1] - mn0);
            s[j][2] = s[j][2] == -INFINITY ? 0.0f : expf(s[j][2] - mn1);
            s[j][3] = s[j][3] == -INFINITY ? 0.0f : expf(s[j][3] - mn1);
            l0 += s[j][0] + s[j][1];
            l1 += s[j][2] + s[j][3];
        }
#pragma unroll
        for (int j = 0; j < HD / 8; j++) {
            o[j][0] *= al0;
            o[j][1] *= al0;
            o[j][2] *= al1;
            o[j][3] *= al1;
        }
#pragma unroll
        for (int kp = 0; kp < AM_TK / 16; kp++) {
            unsigned a0 = am_pack(s[2 * kp][0], s[2 * kp][1]), a1 = am_pack(s[2 * kp][2], s[2 * kp][3]);
            unsigned a2 = am_pack(s[2 * kp + 1][0], s[2 * kp + 1][1]), a3 = am_pack(s[2 * kp + 1][2], s[2 * kp + 1][3]);
            int kr = kp * 16 + (lane & 3) * 2;
#pragma unroll 8
            for (int jd = 0; jd < HD / 8; jd++) {
                int d = jd * 8 + (lane >> 2);
                unsigned b0 = (unsigned)Vs[kr * LD + d] | ((unsigned)Vs[(kr + 1) * LD + d] << 16);
                unsigned b1 = (unsigned)Vs[(kr + 8) * LD + d] | ((unsigned)Vs[(kr + 9) * LD + d] << 16);
                mm_mma(o[jd], a0, a1, a2, a3, b0, b1);
            }
        }
        __syncthreads();
    }
    l0 += __shfl_xor_sync(0xffffffffu, l0, 1);
    l0 += __shfl_xor_sync(0xffffffffu, l0, 2);
    l1 += __shfl_xor_sync(0xffffffffu, l1, 1);
    l1 += __shfl_xor_sync(0xffffffffu, l1, 2);
    float i0 = l0 > 0.0f ? 1.0f / l0 : 0.0f, i1 = l1 > 0.0f ? 1.0f / l1 : 0.0f;
    int tk0 = tok0 + fila0 / group, tk1 = tok0 + fila1 / group;
    int g0 = fila0 % group, g1 = fila1 % group;
#pragma unroll
    for (int jd = 0; jd < HD / 8; jd++) {
        int d = jd * 8 + (lane & 3) * 2;
        if (tk0 < n)
            *(float2 *)&out[(long)tk0 * q_dim + (long)(kvh * group + g0) * HD + d] = make_float2(o[jd][0] * i0, o[jd][1] * i0);
        if (tk1 < n)
            *(float2 *)&out[(long)tk1 * q_dim + (long)(kvh * group + g1) * HD + d] = make_float2(o[jd][2] * i1, o[jd][3] * i1);
    }
}

extern "C" __global__ void __launch_bounds__(128) attn_prefill_mma256(const float *q, const unsigned short *K, const unsigned short *V, float *out, int n,
                                                                       int seq_antes, int heads, int kv_heads, int group)
{
    attn_mma_cuerpo<256>(q, K, V, out, n, seq_antes, heads, kv_heads, group);
}

extern "C" __global__ void __launch_bounds__(128) attn_prefill_mma128(const float *q, const unsigned short *K, const unsigned short *V, float *out, int n,
                                                                       int seq_antes, int heads, int kv_heads, int group)
{
    attn_mma_cuerpo<128>(q, K, V, out, n, seq_antes, heads, kv_heads, group);
}

/* Base de la suma de un bloque MoE: out[i] = compartido[i] * sigmoid(pg[i / h]) (el experto compartido con su puerta por token). */
extern "C" __global__ void moe_comp_base(float *out, const float *sd, const float *pg, int total, int h)
{
    int i = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    if (i < total)
        out[i] = sd[i] * (1.0f / (1.0f + expf(-pg[i / h])));
}

/* attn[i] *= sigmoid(gate[i]) */
extern "C" __global__ void att_puerta(float *attn, const float *gate, int total)
{
    int i = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    if (i < total)
        attn[i] *= 1.0f / (1.0f + expf(-gate[i]));
}

/* Atención de decodificación partida por trozos de contexto: grid = (kv_heads, trozos máximos). Cada bloque calcula, para las `group` cabezas de
 * consulta de su cabeza KV, la suma ponderada SIN normalizar y (máximo, suma) del trozo `[s * chunk, min((s + 1) * chunk, seq))`. */
extern "C" __global__ void dec_attn_split(const float *q, const unsigned short *K, const unsigned short *V, float *po, float *pml, const int *dyn,
                                          int heads, int kv_heads, int hd, int group, int chunk, int max_split)
{
    __shared__ float Qs[16][256];
    __shared__ float Ks[AP_TILE][256];
    __shared__ unsigned short Vs[AP_TILE][256];
    __shared__ float P[16][AP_TILE];
    __shared__ float mm[16], ll[16], alfa[16];
    int tid = (int)threadIdx.x;
    int kvh = (int)blockIdx.x;
    int sp = (int)blockIdx.y;
    int seq = dyn[1];
    int lo = sp * chunk;
    if (lo >= seq) {
        return;
    }
    int hi = lo + chunk < seq ? lo + chunk : seq;
    int kv_dim = kv_heads * hd;
    float inv = rsqrtf((float)hd);
    for (int qi = 0; qi < 16; qi++) {
        float v = 0.0f;
        if (qi < group && tid < hd) {
            v = q[(long)(kvh * group + qi) * hd + tid] * inv;
        }
        Qs[qi][tid] = v;
    }
    if (tid < 16) {
        mm[tid] = -INFINITY;
        ll[tid] = 0.0f;
    }
    float acc[16];
#pragma unroll
    for (int i = 0; i < 16; i++) {
        acc[i] = 0.0f;
    }
    __syncthreads();
    for (int t0 = lo; t0 < hi; t0 += AP_TILE) {
        for (int r = 0; r < AP_TILE; r++) {
            int pos = t0 + r;
            float kk = 0.0f;
            unsigned short vv = 0;
            if (pos < hi && tid < hd) {
                kk = ap_f16(K[(long)pos * kv_dim + (long)kvh * hd + tid]);
                vv = V[(long)pos * kv_dim + (long)kvh * hd + tid];
            }
            Ks[r][tid] = kk;
            Vs[r][tid] = vv;
        }
        __syncthreads();
        {
            int qi = tid >> 4, p = tid & 15;
            float s = -INFINITY;
            if (qi < group && t0 + p < hi) {
                float a = 0.0f;
                for (int d = 0; d < hd; d++) {
                    a += Qs[qi][d] * Ks[p][d];
                }
                s = a;
            }
            P[qi][p] = s;
        }
        __syncthreads();
        {
            int qi = (tid >> 5) * 2 + ((tid & 31) >> 4);
            int p = tid & 15;
            float s = P[qi][p];
            float mx = s;
            for (int o = 8; o > 0; o >>= 1) {
                mx = fmaxf(mx, __shfl_xor_sync(0xffffffffu, mx, o, 16));
            }
            float mold = mm[qi];
            float mnew = fmaxf(mold, mx);
            float al = 1.0f, pe = 0.0f;
            if (mnew != -INFINITY) {
                al = (mold == -INFINITY) ? 0.0f : expf(mold - mnew);
                pe = (s == -INFINITY) ? 0.0f : expf(s - mnew);
            }
            float sum = pe;
            for (int o = 8; o > 0; o >>= 1) {
                sum += __shfl_xor_sync(0xffffffffu, sum, o, 16);
            }
            P[qi][p] = pe;
            if (p == 0) {
                ll[qi] = ll[qi] * al + sum;
                mm[qi] = mnew;
                alfa[qi] = al;
            }
        }
        __syncthreads();
        if (tid < hd) {
#pragma unroll
            for (int qi = 0; qi < 16; qi++) {
                acc[qi] *= alfa[qi];
            }
            for (int p = 0; p < AP_TILE; p++) {
                float v = ap_f16(Vs[p][tid]);
#pragma unroll
                for (int qi = 0; qi < 16; qi++) {
                    acc[qi] += P[qi][p] * v;
                }
            }
        }
        __syncthreads();
    }
    if (tid < hd) {
        for (int qi = 0; qi < group; qi++) {
            po[(((long)kvh * max_split + sp) * 16 + qi) * 256 + tid] = acc[qi];
        }
    }
    if (tid < group) {
        pml[(((long)kvh * max_split + sp) * 16 + tid) * 2] = mm[tid];
        pml[(((long)kvh * max_split + sp) * 16 + tid) * 2 + 1] = ll[tid];
    }
}

/* Une los trozos y aplica la puerta: att[head * hd + d] = (Σ_s e^(m_s − M) o_s / Σ_s e^(m_s − M) l_s) · σ(gate). grid = heads (de consulta). */
extern "C" __global__ void dec_attn_combine(const float *po, const float *pml, const float *gate, float *att, const int *dyn, int kv_heads, int group,
                                            int hd, int chunk, int max_split)
{
    int head = (int)blockIdx.x;
    int tid = (int)threadIdx.x;
    int kvh = head / group, qi = head % group;
    int seq = dyn[1];
    int ns = (seq + chunk - 1) / chunk;
    float M = -INFINITY;
    for (int s = 0; s < ns; s++) {
        float l = pml[(((long)kvh * max_split + s) * 16 + qi) * 2 + 1];
        if (l > 0.0f) {
            M = fmaxf(M, pml[(((long)kvh * max_split + s) * 16 + qi) * 2]);
        }
    }
    float o = 0.0f, L = 0.0f;
    for (int s = 0; s < ns; s++) {
        float l = pml[(((long)kvh * max_split + s) * 16 + qi) * 2 + 1];
        if (l <= 0.0f) {
            continue;
        }
        float w = expf(pml[(((long)kvh * max_split + s) * 16 + qi) * 2] - M);
        L += w * l;
        if (tid < hd) {
            o += w * po[(((long)kvh * max_split + s) * 16 + qi) * 256 + tid];
        }
    }
    if (tid < hd) {
        float g = gate[(long)head * hd + tid];
        att[(long)head * hd + tid] = (L > 0.0f ? o / L : 0.0f) * (1.0f / (1.0f + expf(-g)));
    }
}

/* Router MoE: softmax sobre todos los logits, los `top_k` mayores (empate: el índice menor) renormalizados. Un bloque de 256 hilos. */
extern "C" __global__ void dec_router_topk(const float *lg, int *idx, float *wt, int n_exp, int top_k)
{
    lg += (long)blockIdx.x * n_exp; /* grid.x = tokens (1 en decode) */
    idx += (long)blockIdx.x * top_k;
    wt += (long)blockIdx.x * top_k;
    __shared__ float red[8];
    __shared__ float pr[1024];
    __shared__ float bv[256];
    __shared__ int bi[256];
    int tid = (int)threadIdx.x;
    float mx = -INFINITY;
    for (int i = tid; i < n_exp; i += 256) {
        mx = fmaxf(mx, lg[i]);
    }
    for (int o = 16; o > 0; o >>= 1) {
        mx = fmaxf(mx, __shfl_xor_sync(0xffffffffu, mx, o));
    }
    if ((tid & 31) == 0) {
        red[tid >> 5] = mx;
    }
    __syncthreads();
    mx = red[0];
    for (int i = 1; i < 8; i++) {
        mx = fmaxf(mx, red[i]);
    }
    __syncthreads();
    float sum = 0.0f;
    for (int i = tid; i < n_exp; i += 256) {
        float e = expf(lg[i] - mx);
        pr[i] = e;
        sum += e;
    }
    sum = dec_bsum256(sum, red);
    for (int i = tid; i < n_exp; i += 256) {
        pr[i] /= sum;
    }
    __syncthreads();
    float tot = 0.0f;
    for (int k = 0; k < top_k; k++) {
        float best = -1.0f;
        int bidx = 0x7fffffff;
        for (int i = tid; i < n_exp; i += 256) {
            if (pr[i] > best) {
                best = pr[i];
                bidx = i;
            }
        }
        bv[tid] = best;
        bi[tid] = bidx;
        __syncthreads();
        for (int s2 = 128; s2 > 0; s2 >>= 1) {
            if (tid < s2) {
                if (bv[tid + s2] > bv[tid] || (bv[tid + s2] == bv[tid] && bi[tid + s2] < bi[tid])) {
                    bv[tid] = bv[tid + s2];
                    bi[tid] = bi[tid + s2];
                }
            }
            __syncthreads();
        }
        if (tid == 0) {
            idx[k] = bi[0];
            wt[k] = bv[0];
            pr[bi[0]] = -2.0f;
        }
        tot += 0.0f;
        __syncthreads();
    }
    if (tid == 0) {
        float s = 0.0f;
        for (int k = 0; k < top_k; k++) {
            s += wt[k];
        }
        if (s > 0.0f) {
            for (int k = 0; k < top_k; k++) {
                wt[k] /= s;
            }
        }
    }
}

/* sg = σ(pg · xn): puerta escalar del experto compartido. Un bloque. */
extern "C" __global__ void dec_pgate(const float *pg, const float *xn, float *sg, int n)
{
    __shared__ float red[8];
    float a = 0.0f;
    for (int i = (int)threadIdx.x; i < n; i += 256) {
        a += pg[i] * xn[i];
    }
    a = dec_bsum256(a, red);
    if (threadIdx.x == 0) {
        sg[0] = 1.0f / (1.0f + expf(-a));
    }
}

/* hid[i] += Σ_j w[j] · ys[j * h + i] + sg · ysh[i] (expertos en el orden del ranking y luego el compartido). */
extern "C" __global__ void dec_moe_combine(float *hid, const float *ys, const float *wt, const float *ysh, const float *sg, int top_k, int h)
{
    int i = (int)blockIdx.x * 256 + (int)threadIdx.x;
    if (i >= h) {
        return;
    }
    float a = 0.0f;
    for (int j = 0; j < top_k; j++) {
        a += ys[(long)j * h + i] * wt[j];
    }
    if (ysh) {
        a += ysh[i] * sg[0];
    }
    hid[i] += a;
}

/* hid[i] += cpu[i] + sg · ysh[i]: expertos calculados en CPU (capas cuyos expertos no caben en la tarjeta) más el compartido. */
extern "C" __global__ void dec_moe_combine_cpu(float *hid, const float *cpu, const float *ysh, const float *sg, int h)
{
    int i = (int)blockIdx.x * 256 + (int)threadIdx.x;
    if (i >= h) {
        return;
    }
    float a = cpu[i];
    if (ysh) {
        a += ysh[i] * sg[0];
    }
    hid[i] += a;
}

/* matvec de decode (n = 1) para cualquier código de `iq_decodificar`: una fila por warp, `x` en memoria compartida (cols floats).
 * grid = ceil(rows / 8); 256 hilos. Con pocas filas reparte mejor que los tiles de tensor cores (que dan 1 bloque por 64 filas). */
extern "C" __global__ void __launch_bounds__(256) dec_matvec(const unsigned char *w, const float *x, float *y, int rows, int cols, int code, long rb)
{
    extern __shared__ float xs[];
    for (int i = (int)threadIdx.x; i < cols; i += 256) {
        xs[(i >> 5) * 33 + (i & 31)] = x[i];
    }
    __syncthreads();
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row0 = ((int)blockIdx.x * 8 + warp) * 2;
    if (row0 >= rows) {
        return;
    }
    int two = row0 + 1 < rows;
    const unsigned char *w0 = w + (long)row0 * rb;
    const unsigned char *w1 = w0 + rb;
    int units = cols >> 5;
    float a0 = 0.0f, a1 = 0.0f;
    for (int u = lane; u < units; u += 32) {
        float o[32];
        const float *xp = xs + u * 33;
        iq_decodificar(code, w0, u, o);
#pragma unroll
        for (int l = 0; l < 32; l++) {
            a0 += o[l] * xp[l];
        }
        if (two) {
            iq_decodificar(code, w1, u, o);
#pragma unroll
            for (int l = 0; l < 32; l++) {
                a1 += o[l] * xp[l];
            }
        }
    }
    a0 = iq_warp_sum(a0);
    a1 = iq_warp_sum(a1);
    if (lane == 0) {
        y[row0] = a0;
        if (two) {
            y[row0 + 1] = a1;
        }
    }
}

/* Versiones «una fila por warp» de los expertos de decode (n = 1): más bloques que los tiles de tensor cores y sin relleno a 8 columnas. */
extern "C" __global__ void __launch_bounds__(256, 4) dec_moe_gateup_w(const unsigned long long *tg, const unsigned long long *tu, const int *idx, const float *xn,
                                                                  float *act, int ffn, int h, int dg, int du, long rbg, long rbu)
{
    extern __shared__ float xs[];
    for (int i = (int)threadIdx.x; i < h; i += 256) {
        xs[(i >> 5) * 33 + (i & 31)] = xn[i];
    }
    __syncthreads();
    int slot = (int)blockIdx.z;
    int e = idx[slot];
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row = (int)blockIdx.x * 8 + warp;
    if (row >= ffn) {
        return;
    }
    const unsigned char *wg = (const unsigned char *)tg[e] + (long)row * rbg;
    const unsigned char *wu = (const unsigned char *)tu[e] + (long)row * rbu;
    int units = h >> 5;
    float ag = 0.0f, au = 0.0f;
    for (int u = lane; u < units; u += 32) {
        float o[32];
        const float *xp = xs + u * 33;
        iq_decodificar(dg, wg, u, o);
#pragma unroll
        for (int l = 0; l < 32; l++) {
            ag += o[l] * xp[l];
        }
        iq_decodificar(du, wu, u, o);
#pragma unroll
        for (int l = 0; l < 32; l++) {
            au += o[l] * xp[l];
        }
    }
    ag = iq_warp_sum(ag);
    au = iq_warp_sum(au);
    if (lane == 0) {
        act[(long)slot * ffn + row] = (ag / (1.0f + expf(-ag))) * au;
    }
}

extern "C" __global__ void __launch_bounds__(256, 4) dec_moe_down_w(const unsigned long long *td, const int *idx, const float *act, float *ys, int rows, int cols,
                                                                int dd, long rbd)
{
    extern __shared__ float xs[];
    int slot = (int)blockIdx.z;
    for (int i = (int)threadIdx.x; i < cols; i += 256) {
        xs[(i >> 5) * 33 + (i & 31)] = act[(long)slot * cols + i];
    }
    __syncthreads();
    int e = idx[slot];
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row = (int)blockIdx.x * 8 + warp;
    if (row >= rows) {
        return;
    }
    const unsigned char *wr = (const unsigned char *)td[e] + (long)row * rbd;
    int units = cols >> 5;
    float acc = 0.0f;
    for (int u = lane; u < units; u += 32) {
        float o[32];
        iq_decodificar(dd, wr, u, o);
        const float *xp = xs + u * 33;
#pragma unroll
        for (int l = 0; l < 32; l++) {
            acc += o[l] * xp[l];
        }
    }
    acc = iq_warp_sum(acc);
    if (lane == 0) {
        ys[(long)slot * rows + row] = acc;
    }
}

/* Y[t * rows + r] = Σ_k W[r * K + k] · X[t * K + k] en f32 (router de los MoE: la elección de expertos no puede perder precisión).
 * grid = (ceil(rows / 16), ceil(n / 16)); bloque 16 × 16: tx = fila de W, ty = token. */
extern "C" __global__ void __launch_bounds__(256) sgemm_nt(const float *W, const float *X, float *Y, int rows, int K, int n)
{
    __shared__ float Ws[16][33];
    __shared__ float Xs[16][33];
    int tx = (int)threadIdx.x, ty = (int)threadIdx.y;
    int r0 = (int)blockIdx.x * 16, t0 = (int)blockIdx.y * 16;
    float acc = 0.0f;
    for (int k0 = 0; k0 < K; k0 += 32) {
        for (int q = 0; q < 2; q++) {
            int k = k0 + tx + 16 * q;
            Ws[ty][tx + 16 * q] = (r0 + ty < rows && k < K) ? W[(long)(r0 + ty) * K + k] : 0.0f;
            Xs[ty][tx + 16 * q] = (t0 + ty < n && k < K) ? X[(long)(t0 + ty) * K + k] : 0.0f;
        }
        __syncthreads();
#pragma unroll
        for (int kk = 0; kk < 32; kk++) {
            acc += Ws[tx][kk] * Xs[ty][kk];
        }
        __syncthreads();
    }
    if (r0 + tx < rows && t0 + ty < n) {
        Y[(long)(t0 + ty) * rows + r0 + tx] = acc;
    }
}

/* Y[t][r] = Σ_k W[r][k] X[t][k] en f32 (router). Tile 64 filas × 32 tokens, 128 hilos con micro-tile de 4 tokens × 4 filas en registros, K en pasos de 16 (K % 16 == 0). */
extern "C" __global__ void __launch_bounds__(128) sgemm_nt64(const float *W, const float *X, float *Y, int rows, int K, int n)
{
    __shared__ __align__(16) float Ws[16][68];
    __shared__ __align__(16) float Xs[16][36];
    int tid = (int)threadIdx.x, tx = tid & 15, ty = tid >> 4;
    int r0 = (int)blockIdx.x * 64, t0 = (int)blockIdx.y * 32;
    float acc[4][4];
#pragma unroll
    for (int i = 0; i < 4; i++) {
#pragma unroll
        for (int j = 0; j < 4; j++) {
            acc[i][j] = 0.0f;
        }
    }
    for (int k0 = 0; k0 < K; k0 += 16) {
#pragma unroll
        for (int q = 0; q < 2; q++) {
            int idx = tid + 128 * q, m = idx >> 2, kq = idx & 3;
            float4 v = make_float4(0.f, 0.f, 0.f, 0.f);
            if (r0 + m < rows) {
                v = __ldg((const float4 *)(W + (long)(r0 + m) * K + k0 + 4 * kq));
            }
            Ws[4 * kq][m] = v.x;
            Ws[4 * kq + 1][m] = v.y;
            Ws[4 * kq + 2][m] = v.z;
            Ws[4 * kq + 3][m] = v.w;
        }
        {
            int m = tid >> 2, kq = tid & 3;
            float4 v = make_float4(0.f, 0.f, 0.f, 0.f);
            if (t0 + m < n) {
                v = __ldg((const float4 *)(X + (long)(t0 + m) * K + k0 + 4 * kq));
            }
            Xs[4 * kq][m] = v.x;
            Xs[4 * kq + 1][m] = v.y;
            Xs[4 * kq + 2][m] = v.z;
            Xs[4 * kq + 3][m] = v.w;
        }
        __syncthreads();
#pragma unroll
        for (int kk = 0; kk < 16; kk++) {
            float4 a = *(const float4 *)&Ws[kk][tx * 4];
            float4 b = *(const float4 *)&Xs[kk][ty * 4];
            float av[4] = {a.x, a.y, a.z, a.w}, bv[4] = {b.x, b.y, b.z, b.w};
#pragma unroll
            for (int i = 0; i < 4; i++) {
#pragma unroll
                for (int j = 0; j < 4; j++) {
                    acc[i][j] += av[j] * bv[i];
                }
            }
        }
        __syncthreads();
    }
#pragma unroll
    for (int i = 0; i < 4; i++) {
        int t = t0 + ty * 4 + i;
#pragma unroll
        for (int j = 0; j < 4; j++) {
            int r = r0 + tx * 4 + j;
            if (t < n && r < rows) {
                Y[(long)t * rows + r] = acc[i][j];
            }
        }
    }
}

/* ---------------------------------------------------------------------------------------------------------------------
 * Decode con producto entero para Q8_0 (como el mmvq de llama.cpp): la activación se cuantiza a int8 por bloques de 32 (escala = máx|x| / 127)
 * y cada `__dp4a` hace 4 productos de int8. Las cargas son de 16 B: 4 bloques de Q8_0 de sosomodel (36 B: escala f32 + 32 i8) = 144 B = 9 × 16 B
 * alineados. Sólo para cols múltiplo de 128 y potencia de dos de grupos (cols / 128).
 * ------------------------------------------------------------------------------------------------------------------ */
/* xq[i] = round(x[i] / d), xd[b] = d con d = máx|x| / 127 del bloque b (32 valores). Un warp por bloque; grid = ceil(n / 32 / 8). */
extern "C" __global__ void dec_quant_x(const float *x, signed char *xq, float *xd, int n)
{
    int warp = ((int)blockIdx.x * 256 + (int)threadIdx.x) >> 5;
    int lane = (int)threadIdx.x & 31;
    if (warp * 32 >= n) {
        return;
    }
    float v = x[warp * 32 + lane];
    float a = fabsf(v);
    for (int o = 16; o > 0; o >>= 1) {
        a = fmaxf(a, __shfl_xor_sync(0xffffffffu, a, o));
    }
    float d = a / 127.0f;
    float id = a > 0.0f ? 127.0f / a : 0.0f;
    xq[warp * 32 + lane] = (signed char)__float2int_rn(v * id);
    if (lane == 0) {
        xd[warp] = d;
    }
}

/* y[r] = W[r] · x. grid = ceil(rows / (8 * rpw)), 256 hilos = 8 warps; rpw = filas por warp = max(1, 32 / gpr) con gpr = cols / 128 grupos por fila. */
extern "C" __global__ void __launch_bounds__(256) dec_matvec_q8(const unsigned char *w, const signed char *xq, const float *xd, float *y, int rows, int cols, long rb)
{
    int gpr = cols >> 7;
    int seg = gpr < 32 ? gpr : 32;       /* lanes que cooperan en una fila */
    int rpw = 32 / seg;
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row = (((int)blockIdx.x * 8 + warp) * rpw) + lane / seg;
    int g0 = lane % seg;
    float acc = 0.0f;
    if (row < rows) {
        const unsigned char *wr = w + (long)row * rb;
        for (int g = g0; g < gpr; g += seg) {
            const uint4 *wp = (const uint4 *)(wr + (long)g * 144);
            const uint4 *xp = (const uint4 *)(xq + (long)g * 128);
            uint4 wv[9];
            uint4 xv[8];
#pragma unroll
            for (int i = 0; i < 9; i++) {
                wv[i] = __ldg(wp + i);
            }
#pragma unroll
            for (int i = 0; i < 8; i++) {
                xv[i] = __ldg(xp + i);
            }
            const unsigned int *ww = (const unsigned int *)wv;
            const unsigned int *xw = (const unsigned int *)xv;
            float4 dx = __ldg((const float4 *)(xd + g * 4));
            float dxs[4] = {dx.x, dx.y, dx.z, dx.w};
#pragma unroll
            for (int b = 0; b < 4; b++) {
                int s = 0;
#pragma unroll
                for (int j = 0; j < 8; j++) {
                    s = __dp4a((int)ww[9 * b + 1 + j], (int)xw[8 * b + j], s);
                }
                acc += __uint_as_float(ww[9 * b]) * dxs[b] * (float)s;
            }
        }
    }
    for (int o = seg >> 1; o > 0; o >>= 1) {
        acc += __shfl_xor_sync(0xffffffffu, acc, o);
    }
    if (row < rows && g0 == 0) {
        y[row] = acc;
    }
}

/* y[r] = W[r] · x para W f32 (router, beta/alpha de GDN): una fila por warp con cargas float4 coalescidas. cols múltiplo de 4; grid = ceil(rows / 8). */
extern "C" __global__ void __launch_bounds__(256) dec_matvec_f32(const float *w, const float *x, float *y, int rows, int cols)
{
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row = (int)blockIdx.x * 8 + warp;
    if (row >= rows) {
        return;
    }
    const float4 *wr = (const float4 *)(w + (long)row * cols);
    const float4 *xv = (const float4 *)x;
    float acc = 0.0f;
    for (int i = lane; i < (cols >> 2); i += 32) {
        float4 a = __ldg(wr + i), b = xv[i];
        acc += a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w;
    }
    for (int o = 16; o > 0; o >>= 1) {
        acc += __shfl_xor_sync(0xffffffffu, acc, o);
    }
    if (lane == 0) {
        y[row] = acc;
    }
}

/* ---------------------------------------------------------------------------------------------------------------------
 * Expertos de decode con producto entero (como el mmvq de llama.cpp): cada lane procesa un sub-bloque de 8 pesos (`item`) y los multiplica con
 * 8 valores int8 de la activación cuantizada (`__dp4a`). Los items de una fila son independientes: el compilador los desenrolla y las cargas de
 * pesos se solapan (antes cada lane decodificaba 32 pesos a f32 con una cadena de cargas dependientes).
 * ------------------------------------------------------------------------------------------------------------------ */
/* 4 bits de signo → máscara de bytes 0x00 / 0xFF (byte i = bit i). */
__device__ __forceinline__ unsigned iq_mascara4(unsigned bits)
{
    return (((bits & 0xFu) * 0x00204081u) & 0x01010101u) * 0xFFu;
}

/* IQ2_S: item (u = sub-bloque de 32, l = grupo de 8). `b` = superbloque de 82 B; `ib = u & 7`. */
__device__ __forceinline__ float iq2s_item(const unsigned char *b, int ib, int l, int2 xv, float xdu)
{
    float d = iq_f16(b);
    const unsigned char *qs = b + 2;
    const unsigned char *qh = b + 66;
    unsigned s = b[74 + ib];
    float sc = (float)(l < 2 ? (s & 0xf) : (s >> 4));
    unsigned long long g = IQ2S_GRID_D[qs[ib * 4 + l] | (((int)qh[ib] << (8 - 2 * l)) & 0x300)];
    unsigned sg = qs[32 + ib * 4 + l];
    unsigned m0 = iq_mascara4(sg), m1 = iq_mascara4(sg >> 4);
    int g0 = __vsub4((int)((unsigned)g ^ m0), (int)m0);
    int g1 = __vsub4((int)((unsigned)(g >> 32) ^ m1), (int)m1);
    int sum = __dp4a(g1, xv.y, __dp4a(g0, xv.x, 0));
    return d * (0.5f + sc) * 0.25f * xdu * (float)sum;
}

/* IQ3_XXS: item (u, l). `b` = superbloque de 98 B; `ib = u & 7`. */
__device__ __forceinline__ float iq3xxs_item(const unsigned char *b, int ib, int l, int2 xv, float xdu)
{
    float d = iq_f16(b);
    const unsigned char *qs = b + 2;
    const unsigned char *ga = qs + 64 + 4 * ib; /* sin alinear (bloques de 98 B): se lee por bytes */
    unsigned aux = (unsigned)ga[0] | ((unsigned)ga[1] << 8) | ((unsigned)ga[2] << 16) | ((unsigned)ga[3] << 24);
    float db = d * (0.5f + (float)(aux >> 28)) * 0.5f;
    unsigned sg = KSIGNS_IQ2XS_D[(aux >> (7 * l)) & 127];
    unsigned g1 = IQ3XXS_GRID_D[qs[ib * 8 + 2 * l]];
    unsigned g2 = IQ3XXS_GRID_D[qs[ib * 8 + 2 * l + 1]];
    unsigned m0 = iq_mascara4(sg), m1 = iq_mascara4(sg >> 4);
    int w0 = __vsub4((int)(g1 ^ m0), (int)m0);
    int w1 = __vsub4((int)(g2 ^ m1), (int)m1);
    int sum = __dp4a(w1, xv.y, __dp4a(w0, xv.x, 0));
    return db * xdu * (float)sum;
}

/* 8 nibbles (4 bytes) → 2 × 4 int8 de la tabla `kvalues_iq4nl` (.x: nibbles bajos, .y: altos), con `__byte_perm` (igual que llama.cpp). */
__device__ __forceinline__ int2 iq4_tabla16(unsigned q4)
{
    const unsigned t0 = 0xbfad9881u, t1 = 0xf6eaddcfu, t2 = 0x26190d01u, t3 = 0x71594535u;
    unsigned tmp[2];
    const unsigned sel = 0x32103210u | ((q4 & 0x88888888u) >> 1);
#pragma unroll
    for (unsigned i = 0; i < 2; i++) {
        unsigned sh = 16 * i;
        unsigned lo = __byte_perm(t0, t1, q4 >> sh);
        unsigned hi = __byte_perm(t2, t3, q4 >> sh);
        tmp[i] = __byte_perm(lo, hi, sel >> sh);
    }
    return make_int2((int)__byte_perm(tmp[0], tmp[1], 0x6420), (int)__byte_perm(tmp[0], tmp[1], 0x7531));
}

/* IQ4_XS: item (u, w): 4 bytes de qs (8 pesos: 4 bajos + 4 altos). `b` = superbloque de 136 B; `xq32` = activación del sub-bloque (32 int8). */
__device__ __forceinline__ float iq4xs_item(const unsigned char *b, int ib, int w, const signed char *xq32, float xdu)
{
    float d = iq_f16(b);
    unsigned sh = (unsigned)b[2] | ((unsigned)b[3] << 8);
    unsigned sl = b[4 + (ib >> 1)];
    int ls = (int)((sl >> (4 * (ib & 1))) & 0xf) | (int)(((sh >> (2 * ib)) & 3u) << 4);
    float dl = d * (float)(ls - 32);
    unsigned q4 = *(const unsigned *)(b + 8 + 16 * ib + 4 * w);
    int2 v = iq4_tabla16(q4);
    int xa = *(const int *)(xq32 + 4 * w);
    int xb = *(const int *)(xq32 + 16 + 4 * w);
    int sum = __dp4a(v.y, xb, __dp4a(v.x, xa, 0));
    return dl * xdu * (float)sum;
}

/* gate/up + SwiGLU de los `top_k` expertos (gate y up IQ2_S). grid = (ceil(ffn / 8), 1, top_k); 256 hilos = 8 warps = 8 filas. xq/xd: activación cuantizada. */
extern "C" __global__ void __launch_bounds__(256) dec_moe_gateup_s(const unsigned long long *tg, const unsigned long long *tu, const int *idx,
                                                                   const signed char *xq, const float *xd, float *act, int ffn, int h, long rbg, long rbu)
{
    int slot = (int)blockIdx.z;
    int e = idx[slot];
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row = (int)blockIdx.x * 8 + warp;
    if (row >= ffn) {
        return;
    }
    const unsigned char *wg = (const unsigned char *)tg[e] + (long)row * rbg;
    const unsigned char *wu = (const unsigned char *)tu[e] + (long)row * rbu;
    int nitems = h >> 3;
    float ag = 0.0f, au = 0.0f;
#pragma unroll 4
    for (int t = lane; t < nitems; t += 32) {
        int u = t >> 2, l = t & 3;
        int sb = u >> 3, ib = u & 7;
        int2 xv = *(const int2 *)(xq + (long)t * 8);
        float xdu = xd[u];
        ag += iq2s_item(wg + (long)sb * 82, ib, l, xv, xdu);
        au += iq2s_item(wu + (long)sb * 82, ib, l, xv, xdu);
    }
    ag = iq_warp_sum(ag);
    au = iq_warp_sum(au);
    if (lane == 0) {
        act[(long)slot * ffn + row] = (ag / (1.0f + expf(-ag))) * au;
    }
}

/* down de los `top_k` expertos (IQ3_XXS o IQ4_XS), con cols = 512: cada warp calcula 4 filas (256 items = 8 por lane). grid = (ceil(rows / 32), 1, top_k). */
extern "C" __global__ void __launch_bounds__(256) dec_moe_down_s(const unsigned long long *td, const int *idx, const signed char *aq, const float *ad, float *ys,
                                                                 int rows, int cols, int dd, long rbd)
{
    int slot = (int)blockIdx.z;
    int e = idx[slot];
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row0 = ((int)blockIdx.x * 8 + warp) * 4;
    const unsigned char *w = (const unsigned char *)td[e];
    const signed char *xqs = aq + (long)slot * cols;
    const float *xds = ad + (long)slot * (cols >> 5);
    float acc[4] = {0.0f, 0.0f, 0.0f, 0.0f};
    int nitems = cols >> 3; /* 64 */
#pragma unroll
    for (int k = 0; k < 8; k++) {
        int t = lane + 32 * k;
        int r = t >> 6; /* = k >> 1 */
        int ti = t & 63;
        int row = row0 + r;
        if (row < rows) {
            int u = ti >> 2, q = ti & 3;
            int sb = u >> 3, ib = u & 7;
            const unsigned char *b = w + (long)row * rbd + (long)sb * (dd == IQ_GGML_IQ3_XXS ? 98 : 136);
            float xdu = xds[u];
            float v;
            if (dd == IQ_GGML_IQ3_XXS) {
                int2 xv = *(const int2 *)(xqs + (long)ti * 8);
                v = iq3xxs_item(b, ib, q, xv, xdu);
            } else {
                v = iq4xs_item(b, ib, q, xqs + (long)u * 32, xdu);
            }
            acc[r] += v;
        }
    }
#pragma unroll
    for (int r = 0; r < 4; r++) {
        float a = iq_warp_sum(acc[r]);
        if (lane == 0 && row0 + r < rows) {
            ys[(long)slot * rows + row0 + r] = a;
        }
    }
}

/* ---------------------------------------------------------------------------------------------------------------------
 * Expertos por bloque con tensor cores enteros (como el mmq de llama.cpp): los pesos IQ2_S / IQ3_XXS / IQ4_XS se descodifican a i8 (una consulta de
 * rejilla + signo, sin pasar por f32/f16) y se multiplican con las activaciones cuantizadas a i8 por grupos de 32 (`dec_quant_x`).
 * Una vuelta = un superbloque de 256 columnas. IQ3_XXS e IQ4_XS tienen una escala por 32 pesos (mma k32); IQ2_S una por 16 (dos mma k16).
 * Bloque de 128 hilos = 4 warps × 16 filas, 64 tokens; grid = (ceil(filas / 64), tiles de 64 tokens, expertos), igual que `iq_gateup_mma`.
 * ------------------------------------------------------------------------------------------------------------------ */
#ifndef MQ_NT
#define MQ_NT 32 /* tokens por tile del mmq de expertos */
#endif
#define MQ_NJ (MQ_NT / 8)
#define MQ_LD 272 /* bytes por fila / por token en shared: 256 + 16 (sin conflictos de banco en los fragmentos) */

__device__ __forceinline__ void mma_s8_k16(int *c, unsigned a0, unsigned a1, unsigned b0)
{
    asm volatile("mma.sync.aligned.m16n8k16.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5}, {%6}, {%0,%1,%2,%3};"
                 : "+r"(c[0]), "+r"(c[1]), "+r"(c[2]), "+r"(c[3])
                 : "r"(a0), "r"(a1), "r"(b0));
}

/* Bytes crudos del superbloque `sb` que descodifica este lane (fila = lane / 2, sub-bloques (lane & 1) * 4 + 0..3), en registros. Se cargan con anticipación (antes del mma
 * del superbloque anterior) para que la latencia de memoria quede tapada. r: IQ2_S qs[4] sg[4] qh sc | IQ3_XXS qs[8] aux[4] | IQ4_XS sl q4[16]; sh y d aparte. */
template <int FMT>
struct MqCrudo {
    unsigned r[FMT == 22 ? 10 : (FMT == 18 ? 12 : 17)];
    unsigned d, sh;
};

__device__ __forceinline__ unsigned mq_u16(const unsigned char *p)
{
    return (unsigned)__ldg((const unsigned short *)p);
}
__device__ __forceinline__ unsigned mq_u32a2(const unsigned char *p)
{
    return mq_u16(p) | (mq_u16(p + 2) << 16);
}

template <int FMT>
__device__ __forceinline__ void mq_cargar(const unsigned char *w, long rb, int rows, int row_w, int sb, MqCrudo<FMT> &c)
{
    int lane = (int)threadIdx.x & 31;
    int r = lane >> 1, ib0 = (lane & 1) * 4;
    bool ok = row_w + r < rows;
    const unsigned char *b = w + (long)(row_w + r) * rb + (long)sb * (FMT == 22 ? 82 : FMT == 18 ? 98 : 136);
    if (!ok) {
#pragma unroll
        for (int i = 0; i < (int)(sizeof(c.r) / 4); i++) {
            c.r[i] = 0;
        }
        c.d = 0;
        c.sh = 0;
        return;
    }
    c.d = mq_u16(b);
    if (FMT == 22) {
#pragma unroll
        for (int i = 0; i < 4; i++) {
            c.r[i] = mq_u32a2(b + 2 + ib0 * 4 + 4 * i);
            c.r[4 + i] = mq_u32a2(b + 34 + ib0 * 4 + 4 * i);
        }
        c.r[8] = mq_u32a2(b + 66 + ib0);
        c.r[9] = mq_u32a2(b + 74 + ib0);
    } else if (FMT == 18) {
#pragma unroll
        for (int i = 0; i < 8; i++) {
            c.r[i] = mq_u32a2(b + 2 + ib0 * 8 + 4 * i);
        }
#pragma unroll
        for (int i = 0; i < 4; i++) {
            c.r[8 + i] = mq_u32a2(b + 66 + 4 * (ib0 + i));
        }
    } else {
        c.sh = mq_u16(b + 2);
        c.r[0] = mq_u16(b + 4 + (ib0 >> 1));
#pragma unroll
        for (int i = 0; i < 16; i++) {
            c.r[1 + i] = __ldg((const unsigned *)(b + 8 + 16 * ib0) + i);
        }
    }
}

/* Descodifica a Wsw (i8 por fila, MQ_LD) y Sc (escala float por grupo) desde los bytes crudos. */
template <int FMT>
__device__ __forceinline__ void mq_decodificar(const MqCrudo<FMT> &c, unsigned char *Wsw, float *Sc)
{
    int lane = (int)threadIdx.x & 31;
    int r = lane >> 1;
    unsigned char *fila = Wsw + r * MQ_LD;
    unsigned char d2[2] = {(unsigned char)c.d, (unsigned char)(c.d >> 8)};
    float d = iq_f16(d2);
#pragma unroll
    for (int i = 0; i < 4; i++) {
        int ib = (lane & 1) * 4 + i;
        if (FMT == 22) {
            unsigned s = (c.r[9] >> (8 * i)) & 0xffu, qh = (c.r[8] >> (8 * i)) & 0xffu;
            Sc[r * 16 + 2 * ib] = d * (0.5f + (float)(s & 0xf)) * 0.25f;
            Sc[r * 16 + 2 * ib + 1] = d * (0.5f + (float)(s >> 4)) * 0.25f;
#pragma unroll
            for (int l = 0; l < 4; l++) {
                unsigned long long g = IQ2S_GRID_D[((c.r[i] >> (8 * l)) & 0xffu) | ((qh << (8 - 2 * l)) & 0x300)];
                unsigned sg = (c.r[4 + i] >> (8 * l)) & 0xffu;
                unsigned m0 = iq_mascara4(sg), m1 = iq_mascara4(sg >> 4);
                int2 v;
                v.x = __vsub4((int)((unsigned)g ^ m0), (int)m0);
                v.y = __vsub4((int)((unsigned)(g >> 32) ^ m1), (int)m1);
                *(int2 *)(fila + ib * 32 + l * 8) = v;
            }
        } else if (FMT == 18) {
            unsigned aux = c.r[8 + i];
            Sc[r * 16 + ib] = d * (0.5f + (float)(aux >> 28)) * 0.5f;
#pragma unroll
            for (int l = 0; l < 4; l++) {
                unsigned sg = KSIGNS_IQ2XS_D[(aux >> (7 * l)) & 127];
                unsigned q = c.r[2 * i + (l >> 1)] >> (16 * (l & 1));
                unsigned g1 = IQ3XXS_GRID_D[q & 0xffu];
                unsigned g2 = IQ3XXS_GRID_D[(q >> 8) & 0xffu];
                unsigned m0 = iq_mascara4(sg), m1 = iq_mascara4(sg >> 4);
                int2 v;
                v.x = __vsub4((int)(g1 ^ m0), (int)m0);
                v.y = __vsub4((int)(g2 ^ m1), (int)m1);
                *(int2 *)(fila + ib * 32 + l * 8) = v;
            }
        } else {
            unsigned sl = (c.r[0] >> (8 * (i >> 1))) & 0xffu;
            int ls = (int)((sl >> (4 * (i & 1))) & 0xf) | (int)(((c.sh >> (2 * ib)) & 3u) << 4);
            Sc[r * 16 + ib] = d * (float)(ls - 32);
#pragma unroll
            for (int w4 = 0; w4 < 4; w4++) {
                int2 v = iq4_tabla16(c.r[1 + 4 * i + w4]);
                *(int *)(fila + ib * 32 + 4 * w4) = v.x;
                *(int *)(fila + ib * 32 + 16 + 4 * w4) = v.y;
            }
        }
    }
}

/* Disposición de una etapa de smem del mmq de expertos: Ws[4 warps][16 * MQ_LD] | Sc[4][256] float | Xs[MQ_NT * MQ_LD] | Ds[MQ_NT * 8] float. Dos etapas (doble búfer). */
#define MQ_ST_WS (4 * 16 * MQ_LD)
#define MQ_ST_SC (4 * 256 * 4)
#define MQ_ST_XS (MQ_NT * MQ_LD)
#define MQ_ST_DS (MQ_NT * 8 * 4)
#define MQ_STAGE (MQ_ST_WS + MQ_ST_SC + MQ_ST_XS + MQ_ST_DS)
#ifndef MQ_ETAPAS
#define MQ_ETAPAS 1 /* 2 = doble búfer de smem (62 KB: sólo cabe 1 bloque por SM en la 5070 Ti laptop y pierde ocupación) */
#endif
#define MQ_SMEM (MQ_ETAPAS * MQ_STAGE)

/* Carga a registros el tile de X (i8 y escalas) del superbloque `sb`: xr[k] = fila t = (tid + 128k) / 16, trozo i = e & 15. */
__device__ __forceinline__ void mq_x_cargar(const signed char *xq, const float *xd, const int *tok, int tokoff, int t0, int nt, int K, int sb, uint4 xr[MQ_NJ], float4 &dv)
{
    int tid = (int)threadIdx.x;
#pragma unroll
    for (int k = 0; k < MQ_NJ; k++) {
        int e = tid + 128 * k;
        int t = e >> 4, i = e & 15;
        uint4 v = make_uint4(0, 0, 0, 0);
        if (t < nt) {
            long tr = tok ? (long)tok[t0 + t] : (long)(tokoff + t0 + t);
            v = __ldg((const uint4 *)(xq + tr * K + sb * 256) + i);
        }
        xr[k] = v;
    }
    int t = tid >> 1, hf = tid & 1;
    dv = make_float4(0.f, 0.f, 0.f, 0.f);
    if (t < nt) {
        long tr = tok ? (long)tok[t0 + t] : (long)(tokoff + t0 + t);
        dv = __ldg((const float4 *)(xd + tr * (K / 32) + sb * 8) + hf);
    }
}

__device__ __forceinline__ void mq_x_guardar(signed char *Xs, float *Ds, const uint4 xr[MQ_NJ], const float4 &dv)
{
    int tid = (int)threadIdx.x;
#pragma unroll
    for (int k = 0; k < MQ_NJ; k++) {
        int e = tid + 128 * k;
        *(uint4 *)(Xs + (e >> 4) * MQ_LD + (e & 15) * 16) = xr[k];
    }
    int t = tid >> 1;
    if (t < MQ_NT) {
        *(float4 *)(Ds + t * 8 + (tid & 1) * 4) = dv;
    }
}

/* acc[j][q] += W[filas del warp] · X[tokens j*8..] sobre K columnas. `sm`: smem dinámica de MQ_SMEM bytes, doble búfer: la etapa siguiente (X, bytes crudos de pesos y su
 * descodificación) se prepara mientras el mma consume la actual. tok == null: tokens consecutivos desde tokoff + t0. Termina con __syncthreads. */
template <int FMT>
__device__ void mq_tile(const unsigned char *w, long rb, int rows, int row_w, int K, const signed char *xq, const float *xd, const int *tok, int tokoff, int t0, int nt,
                        float acc[MQ_NJ][4], unsigned char *sm)
{
    int tid = (int)threadIdx.x;
    int lane = tid & 31, g = lane >> 2, c4 = lane & 3, warp = tid >> 5;
    const int nsb = K / 256;
    MqCrudo<FMT> cr;
    uint4 xr[MQ_NJ];
    float4 dv;
    mq_cargar<FMT>(w, rb, rows, row_w, 0, cr);
    mq_x_cargar(xq, xd, tok, tokoff, t0, nt, K, 0, xr, dv);
    {
        unsigned char *st = sm;
        mq_decodificar<FMT>(cr, st + warp * 16 * MQ_LD, (float *)(st + MQ_ST_WS) + warp * 256);
        mq_x_guardar((signed char *)(st + MQ_ST_WS + MQ_ST_SC), (float *)(st + MQ_ST_WS + MQ_ST_SC + MQ_ST_XS), xr, dv);
    }
    if (nsb > 1) {
        mq_cargar<FMT>(w, rb, rows, row_w, 1, cr);
    }
    __syncthreads();
    for (int sb = 0; sb < nsb; sb++) {
        unsigned char *st = sm + (MQ_ETAPAS == 2 ? (sb & 1) * MQ_STAGE : 0), *sn = sm + (MQ_ETAPAS == 2 ? ((sb & 1) ^ 1) * MQ_STAGE : 0);
        bool hay = sb + 1 < nsb;
        if (hay) {
            mq_x_cargar(xq, xd, tok, tokoff, t0, nt, K, sb + 1, xr, dv);
            if (MQ_ETAPAS == 2) {
                mq_decodificar<FMT>(cr, sn + warp * 16 * MQ_LD, (float *)(sn + MQ_ST_WS) + warp * 256);
                if (sb + 2 < nsb) {
                    mq_cargar<FMT>(w, rb, rows, row_w, sb + 2, cr);
                }
            }
        }
        const unsigned char *Wsw = st + warp * 16 * MQ_LD;
        const float *Sc = (const float *)(st + MQ_ST_WS) + warp * 256;
        const signed char *Xs = (const signed char *)(st + MQ_ST_WS + MQ_ST_SC);
        const float *Ds = (const float *)(st + MQ_ST_WS + MQ_ST_SC + MQ_ST_XS);
        __syncwarp();
#pragma unroll
        for (int b = 0; b < 8; b++) {
            const unsigned char *r0 = Wsw + g * MQ_LD + b * 32 + 4 * c4;
            const unsigned char *r1 = Wsw + (g + 8) * MQ_LD + b * 32 + 4 * c4;
            if (FMT == 22) {
#pragma unroll
                for (int hf = 0; hf < 2; hf++) {
                    unsigned a0 = *(const unsigned *)(r0 + hf * 16), a1 = *(const unsigned *)(r1 + hf * 16);
                    float d0 = Sc[g * 16 + 2 * b + hf], d1 = Sc[(g + 8) * 16 + 2 * b + hf];
#pragma unroll
                    for (int j = 0; j < MQ_NJ; j++) {
                        if (j * 8 < nt) {
                            int c[4] = {0, 0, 0, 0};
                            mma_s8_k16(c, a0, a1, *(const unsigned *)(Xs + (j * 8 + g) * MQ_LD + b * 32 + hf * 16 + 4 * c4));
                            float dxa = Ds[(j * 8 + 2 * c4) * 8 + b], dxb = Ds[(j * 8 + 2 * c4 + 1) * 8 + b];
                            acc[j][0] += (float)c[0] * (d0 * dxa);
                            acc[j][1] += (float)c[1] * (d0 * dxb);
                            acc[j][2] += (float)c[2] * (d1 * dxa);
                            acc[j][3] += (float)c[3] * (d1 * dxb);
                        }
                    }
                }
            } else {
                unsigned a0 = *(const unsigned *)r0, a1 = *(const unsigned *)r1, a2 = *(const unsigned *)(r0 + 16), a3 = *(const unsigned *)(r1 + 16);
                float d0 = Sc[g * 16 + b], d1 = Sc[(g + 8) * 16 + b];
#pragma unroll
                for (int j = 0; j < MQ_NJ; j++) {
                    if (j * 8 < nt) {
                        const unsigned char *xr8 = (const unsigned char *)(Xs + (j * 8 + g) * MQ_LD + b * 32 + 4 * c4);
                        int c[4] = {0, 0, 0, 0};
                        mma_s8(c, a0, a1, a2, a3, *(const unsigned *)xr8, *(const unsigned *)(xr8 + 16));
                        float dxa = Ds[(j * 8 + 2 * c4) * 8 + b], dxb = Ds[(j * 8 + 2 * c4 + 1) * 8 + b];
                        acc[j][0] += (float)c[0] * (d0 * dxa);
                        acc[j][1] += (float)c[1] * (d0 * dxb);
                        acc[j][2] += (float)c[2] * (d1 * dxa);
                        acc[j][3] += (float)c[3] * (d1 * dxb);
                    }
                }
            }
        }
        if (MQ_ETAPAS == 1) {
            __syncthreads();
            if (hay) {
                mq_decodificar<FMT>(cr, sn + warp * 16 * MQ_LD, (float *)(sn + MQ_ST_WS) + warp * 256);
                if (sb + 2 < nsb) {
                    mq_cargar<FMT>(w, rb, rows, row_w, sb + 2, cr);
                }
            }
        }
        if (hay) {
            mq_x_guardar((signed char *)(sn + MQ_ST_WS + MQ_ST_SC), (float *)(sn + MQ_ST_WS + MQ_ST_SC + MQ_ST_XS), xr, dv);
        }
        __syncthreads();
    }
}

/* gate/up + SwiGLU de los expertos del descriptor (gate y up IQ2_S); `act` en f32 ([abase + t] * ffn + fila), que `dec_quant_x` cuantiza para el down. */
extern "C" __global__ void __launch_bounds__(128) iq_gateup_mmq(const unsigned long long *desc, const signed char *xq, const float *xd, const int *tok, float *act, int ffn, int h)
{
    extern __shared__ __align__(16) unsigned char mq_sm[];
    const unsigned long long *d = desc + (long)blockIdx.z * IQ_DESC;
    int cnt = (int)d[4];
    int t0 = (int)blockIdx.y * MQ_NT;
    if (t0 >= cnt) {
        return;
    }
    int warp = (int)threadIdx.x >> 5, lane = (int)threadIdx.x & 31, g = lane >> 2, c4 = lane & 3;
    int row_w = (int)blockIdx.x * 64 + warp * 16;
    int nt = cnt - t0 < MQ_NT ? cnt - t0 : MQ_NT;
    float ag[MQ_NJ][4], au[MQ_NJ][4];
#pragma unroll
    for (int j = 0; j < MQ_NJ; j++) {
#pragma unroll
        for (int q = 0; q < 4; q++) {
            ag[j][q] = 0.0f;
            au[j][q] = 0.0f;
        }
    }
    const int *tk = tok + (long)d[11];
    mq_tile<22>((const unsigned char *)d[0], (long)d[8], ffn, row_w, h, xq, xd, tk, 0, t0, nt, ag, mq_sm);
    mq_tile<22>((const unsigned char *)d[1], (long)d[9], ffn, row_w, h, xq, xd, tk, 0, t0, nt, au, mq_sm);
#pragma unroll
    for (int j = 0; j < MQ_NJ; j++) {
#pragma unroll
        for (int q = 0; q < 4; q++) {
            int fila = row_w + g + (q >> 1) * 8;
            int t = j * 8 + 2 * c4 + (q & 1);
            if (fila < ffn && t < nt) {
                float gv = ag[j][q];
                act[((long)d[3] + t0 + t) * ffn + fila] = (gv / (1.0f + expf(-gv))) * au[j][q];
            }
        }
    }
}

/* down de los expertos del descriptor: y[dest[abase + t]] [fila] = W[fila] · act[abase + t]; act cuantizada (actq/actd). El formato (IQ3_XXS o IQ4_XS) va en d[7]. */
extern "C" __global__ void __launch_bounds__(128) iq_down_mmq(const unsigned long long *desc, const signed char *aq, const float *ad, float *y, int rows, int cols, const int *dest)
{
    extern __shared__ __align__(16) unsigned char mq_sm[];
    const unsigned long long *d = desc + (long)blockIdx.z * IQ_DESC;
    int cnt = (int)d[4];
    int t0 = (int)blockIdx.y * MQ_NT;
    if (t0 >= cnt) {
        return;
    }
    int warp = (int)threadIdx.x >> 5, lane = (int)threadIdx.x & 31, g = lane >> 2, c4 = lane & 3;
    int row_w = (int)blockIdx.x * 64 + warp * 16;
    int nt = cnt - t0 < MQ_NT ? cnt - t0 : MQ_NT;
    float a[MQ_NJ][4];
#pragma unroll
    for (int j = 0; j < MQ_NJ; j++) {
#pragma unroll
        for (int q = 0; q < 4; q++) {
            a[j][q] = 0.0f;
        }
    }
    if ((int)d[7] == 18) {
        mq_tile<18>((const unsigned char *)d[2], (long)d[10], rows, row_w, cols, aq, ad, (const int *)0, (int)d[3], t0, nt, a, mq_sm);
    } else {
        mq_tile<23>((const unsigned char *)d[2], (long)d[10], rows, row_w, cols, aq, ad, (const int *)0, (int)d[3], t0, nt, a, mq_sm);
    }
#pragma unroll
    for (int j = 0; j < MQ_NJ; j++) {
#pragma unroll
        for (int q = 0; q < 4; q++) {
            int fila = row_w + g + (q >> 1) * 8;
            int t = j * 8 + 2 * c4 + (q & 1);
            if (fila < rows && t < nt) {
                y[(long)dest[(int)d[3] + t0 + t] * rows + fila] = a[j][q];
            }
        }
    }
}

/* Hidden residente del prefill: [r += y;] xn = rmsnorm(r) * w por token (un bloque de 256 hilos por fila de h). Mismo orden de operaciones que `rmsnorm` de la CPU (x * escala * w). */
extern "C" __global__ void __launch_bounds__(256) res_suma_rms(float *r, const float *y, const float *w, float *xn, int h, float eps, int sumar)
{
    __shared__ float red[8];
    long t = (long)blockIdx.x;
    float *rr = r + t * h;
    float ss = 0.0f;
    for (int i = (int)threadIdx.x; i < h; i += 256) {
        float v = rr[i];
        if (sumar) {
            v += y[t * h + i];
            rr[i] = v;
        }
        ss += v * v;
    }
    for (int o = 16; o > 0; o >>= 1) {
        ss += __shfl_xor_sync(0xffffffffu, ss, o);
    }
    if ((threadIdx.x & 31) == 0) {
        red[threadIdx.x >> 5] = ss;
    }
    __syncthreads();
    float tot = 0.0f;
#pragma unroll
    for (int i = 0; i < 8; i++) {
        tot += red[i];
    }
    float escala = 1.0f / sqrtf(tot / (float)h + eps);
    for (int i = (int)threadIdx.x; i < h; i += 256) {
        xn[t * h + i] = rr[i] * escala * w[i];
    }
}
