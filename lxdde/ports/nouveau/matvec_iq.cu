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
        xs[i] = x[i];
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
        acc += iq_unidad(dtype, wr, u, xs + (long)u * 32);
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
    default:
#pragma unroll
        for (int l = 0; l < 32; l++) {
            o[l] = 0.0f;
        }
        break;
    }
}

#define IQ_TILE 16

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
