/* G9: kernels de la capa de Gemma 4 en la GPU del host (activaciones f32, K/V f16).
 *
 * Cada kernel reproduce una operación de `soso-llm-core/src/gemma.rs` con la misma fórmula; sólo cambia el orden
 * de las sumas. Los datos de `n` tokens van seguidos: x[t * dim + i].
 *
 *   gm_rmsnorm_rows   RMSNorm por filas (con o sin pesos); vale también por cabeza (filas de `head_dim`).
 *   gm_rope           RoPE NeoX con base propia y factores por frecuencia opcionales.
 *   gm_kv_append      f32 -> f16 a las filas `row0 + t` de la caché.
 *   gm_attn_scores    Q·K * escala de todas las posiciones de la ventana de cada (token, cabeza).
 *   gm_attn_softmax   softmax en sitio sobre esa ventana.
 *   gm_attn_pv        P·V repartido en `S` trozos de posiciones (parciales) ...
 *   gm_attn_reduce    ... y su suma.
 *   gm_geglu          up *= gelu(gate) (tanh).
 *   gm_add_scale      y = (a + b) * s.
 *   mo_*              capa MoE (Mellum): router + top-k, expertos Q4_K/Q8_0 elegidos en el dispositivo, combinación.
 *
 * Posiciones: las filas de la caché se numeran 0..total dentro del búfer (las expulsadas ya no están). El token t
 * del bloque ve las filas [max(0, total_t - ventana), total_t) con total_t = seq_antes + t + 1; ventana 0 = todas.
 */
#include <cuda_fp16.h>
#include "q4k_decode.h"

__device__ __forceinline__ float gm_warp_sum(float v)
{
    unsigned mask = 0xffffffffu;
    v += __shfl_down_sync(mask, v, 16);
    v += __shfl_down_sync(mask, v, 8);
    v += __shfl_down_sync(mask, v, 4);
    v += __shfl_down_sync(mask, v, 2);
    v += __shfl_down_sync(mask, v, 1);
    return v;
}

__device__ __forceinline__ float gm_warp_max(float v)
{
    unsigned mask = 0xffffffffu;
    v = fmaxf(v, __shfl_down_sync(mask, v, 16));
    v = fmaxf(v, __shfl_down_sync(mask, v, 8));
    v = fmaxf(v, __shfl_down_sync(mask, v, 4));
    v = fmaxf(v, __shfl_down_sync(mask, v, 2));
    v = fmaxf(v, __shfl_down_sync(mask, v, 1));
    return v;
}

/* Reducción de un bloque (hasta 1024 hilos); el resultado queda en todos los hilos. */
__device__ __forceinline__ float gm_block_sum(float v)
{
    __shared__ float red[32];
    int lane = (int)threadIdx.x & 31;
    int warp = (int)threadIdx.x >> 5;
    v = gm_warp_sum(v);
    __syncthreads();
    if (lane == 0) {
        red[warp] = v;
    }
    __syncthreads();
    int nw = ((int)blockDim.x + 31) >> 5;
    float r = (lane < nw) ? red[lane] : 0.0f;
    r = gm_warp_sum(r);
    return __shfl_sync(0xffffffffu, r, 0);
}

__device__ __forceinline__ float gm_block_max(float v)
{
    __shared__ float redm[32];
    int lane = (int)threadIdx.x & 31;
    int warp = (int)threadIdx.x >> 5;
    v = gm_warp_max(v);
    __syncthreads();
    if (lane == 0) {
        redm[warp] = v;
    }
    __syncthreads();
    int nw = ((int)blockDim.x + 31) >> 5;
    float r = (lane < nw) ? redm[lane] : -3.0e38f;
    r = gm_warp_max(r);
    return __shfl_sync(0xffffffffu, r, 0);
}

/* x[fila][h] -> y[fila][h]; en sitio vale. `w` puede ser NULL (V de Gemma no lleva pesos). */
extern "C" __global__ void gm_rmsnorm_rows(const float *x, const float *w, float *y, int h, float eps)
{
    const float *xr = x + (long)blockIdx.x * (long)h;
    float *yr = y + (long)blockIdx.x * (long)h;
    float s = 0.0f;
    for (int i = (int)threadIdx.x; i < h; i += (int)blockDim.x) {
        float v = xr[i];
        s += v * v;
    }
    s = gm_block_sum(s);
    float scale = 1.0f / sqrtf(s / (float)h + eps);
    for (int i = (int)threadIdx.x; i < h; i += (int)blockDim.x) {
        float v = xr[i] * scale;
        yr[i] = (w != 0) ? v * w[i] : v;
    }
}

/* x[t][heads][hd]; pares (i, i + hd/2); un hilo por par. grid = (heads, n). */
extern "C" __global__ void gm_rope(float *x, int heads, int hd, int pos0, float theta, const float *factors, float mscale, const int *dyn)
{
    if (dyn != 0) {
        pos0 = dyn[0];
    }
    int t = (int)blockIdx.y;
    int head = (int)blockIdx.x;
    int i = (int)threadIdx.x;
    int mitad = hd >> 1;
    if (i >= mitad) {
        return;
    }
    float ln_theta = logf(theta);
    float freq = expf(-ln_theta * (float)(2 * i) / (float)hd);
    if (factors != 0) {
        freq /= factors[i];
    }
    float ang = (float)(pos0 + t) * freq;
    float c = cosf(ang);
    float s = sinf(ang);
    float *v = x + ((long)t * heads + head) * hd;
    float a = v[i];
    float b = v[i + mitad];
    c *= mscale;
    s *= mscale;
    v[i] = a * c - b * s;
    v[i + mitad] = a * s + b * c;
}

extern "C" __global__ void gm_kv_append(const float *k, const float *v, __half *dk, __half *dv, int kv_dim, int row0, int n, const int *dyn)
{
    if (dyn != 0) {
        row0 = dyn[1];
    }
    int t = (int)blockIdx.y;
    int i = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    if (t >= n || i >= kv_dim) {
        return;
    }
    long dst = (long)(row0 + t) * kv_dim + i;
    dk[dst] = __float2half_rn(k[(long)t * kv_dim + i]);
    dv[dst] = __float2half_rn(v[(long)t * kv_dim + i]);
}

/* grid = (ceil(maxtotal / 32), heads, n); 128 hilos = 4 warps, cada warp una posición por vuelta. */
extern "C" __global__ void gm_attn_scores(const float *q, const __half *dk, float *sc, int heads, int kv_heads, int hd,
                                          int kv_dim, int seq_antes, int ventana, float escala, int stride, const int *dyn)
{
    if (dyn != 0) {
        seq_antes = dyn[1];
    }
    int t = (int)blockIdx.z;
    int h = (int)blockIdx.y;
    int total = seq_antes + t + 1;
    int t_ini = (ventana > 0 && total > ventana) ? total - ventana : 0;
    int c0 = (int)blockIdx.x * 32;
    if (c0 >= total) {
        return;
    }
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int kvh = h / (heads / kv_heads);
    const float2 *qp = (const float2 *)(q + ((long)t * heads + h) * hd);
    float *scr = sc + ((long)t * heads + h) * (long)stride;
    int pares = hd >> 1; /* pares de elementos */
    for (int p = c0 + warp; p < c0 + 32 && p < total; p += 4) {
        if (p < t_ini) {
            continue;
        }
        const __half2 *kp = (const __half2 *)(dk + (long)p * kv_dim + (long)kvh * hd);
        float s = 0.0f;
        for (int j = lane; j < pares; j += 32) {
            float2 qv = qp[j];
            float2 kv = __half22float2(kp[j]);
            s += qv.x * kv.x + qv.y * kv.y;
        }
        s = gm_warp_sum(s);
        if (lane == 0) {
            scr[p] = s * escala;
        }
    }
}

/* grid = (heads, n); 256 hilos. */
extern "C" __global__ void gm_attn_softmax(float *sc, int heads, int seq_antes, int ventana, int stride, const int *dyn)
{
    if (dyn != 0) {
        seq_antes = dyn[1];
    }
    int t = (int)blockIdx.y;
    int h = (int)blockIdx.x;
    int total = seq_antes + t + 1;
    int t_ini = (ventana > 0 && total > ventana) ? total - ventana : 0;
    float *scr = sc + ((long)t * heads + h) * (long)stride;
    float m = -3.0e38f;
    for (int p = t_ini + (int)threadIdx.x; p < total; p += (int)blockDim.x) {
        m = fmaxf(m, scr[p]);
    }
    m = gm_block_max(m);
    float suma = 0.0f;
    for (int p = t_ini + (int)threadIdx.x; p < total; p += (int)blockDim.x) {
        float e = expf(scr[p] - m);
        scr[p] = e;
        suma += e;
    }
    suma = gm_block_sum(suma);
    float inv = suma > 0.0f ? 1.0f / suma : 0.0f;
    for (int p = t_ini + (int)threadIdx.x; p < total; p += (int)blockDim.x) {
        scr[p] *= inv;
    }
}

/* grid = (S, heads, n); un hilo por dimensión (blockDim.x = hd). Parcial s: posiciones [t_ini + s*L, ...). */
extern "C" __global__ void gm_attn_pv(const float *sc, const __half *dv, float *parcial, int heads, int kv_heads, int hd,
                                      int kv_dim, int seq_antes, int ventana, int stride, int S, const int *dyn)
{
    if (dyn != 0) {
        seq_antes = dyn[1];
    }
    int s = (int)blockIdx.x;
    int h = (int)blockIdx.y;
    int t = (int)blockIdx.z;
    int d = (int)threadIdx.x;
    int total = seq_antes + t + 1;
    int t_ini = (ventana > 0 && total > ventana) ? total - ventana : 0;
    int len = total - t_ini;
    int por = (len + S - 1) / S;
    int a = t_ini + s * por;
    int b = a + por < total ? a + por : total;
    int kvh = h / (heads / kv_heads);
    const float *scr = sc + ((long)t * heads + h) * (long)stride;
    float acc = 0.0f;
    for (int p = a; p < b; p++) {
        acc += scr[p] * __half2float(dv[(long)p * kv_dim + (long)kvh * hd + d]);
    }
    parcial[(((long)t * heads + h) * S + s) * hd + d] = acc;
}

/* grid = (heads, n); blockDim.x = hd. */
extern "C" __global__ void gm_attn_reduce(const float *parcial, float *out, int heads, int hd, int S)
{
    int h = (int)blockIdx.x;
    int t = (int)blockIdx.y;
    int d = (int)threadIdx.x;
    float acc = 0.0f;
    for (int s = 0; s < S; s++) {
        acc += parcial[(((long)t * heads + h) * S + s) * hd + d];
    }
    out[((long)t * heads + h) * hd + d] = acc;
}

extern "C" __global__ void gm_geglu(float *up, const float *gate, int n)
{
    int i = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    if (i >= n) {
        return;
    }
    float x = gate[i];
    float g = 0.5f * x * (1.0f + tanhf(0.7978845608f * (x + 0.044715f * x * x * x)));
    up[i] *= g;
}

extern "C" __global__ void gm_add_scale(const float *a, const float *b, float *y, int n, float s)
{
    int i = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    if (i >= n) {
        return;
    }
    y[i] = (a[i] + b[i]) * s;
}

/* ------------------------------------------------------------------------------------------------------------
 * Capa MoE estilo Mixtral/Qwen3-MoE (Mellum): los expertos se eligen en el dispositivo. Cada tensor de experto vive
 * en su propio búfer; las tablas `pg/pu/pd` guardan un puntero por experto (gate/up Q4_K, down Q8_0).
 * ---------------------------------------------------------------------------------------------------------- */

/* `xs` va en shared con 4 floats de relleno cada 32: sin él, las lanes de una fase de float4 caen casi todas en los
 * mismos bancos (desplazamientos múltiplos de 16 floats). Índice del elemento i: (i >> 5) * XPAD + (i & 31). */
#define XPAD 36
#define XS_FLOATS (4096 / 32 * XPAD)

/* Producto de una fila Q4_K por `xs` (shared, f32). Un warp por fila; cada lane toma "unidades" de 16 bytes de nibbles
 * (superbloque, par de sub-bloques, mitad): una carga uint4 coalescida da 16 elementos del sub-bloque 2p (nibble bajo)
 * y 16 del 2p+1 (alto). Mismas escalas y mismo orden de decodificación que `q4k_dot_sub`. Devuelve el parcial del
 * lane (hay que reducir con `gm_warp_sum`). `cols` múltiplo de 256. */
__device__ __forceinline__ float q4k_row_dot(const unsigned char *row, const float *xs, int cols, int lane)
{
    float acc = 0.0f;
    int units = (cols >> 8) * 8;
    /* Tres unidades por vuelta: las seis cargas (nibbles y cabecera de 16 B) salen antes de usar ninguna, así hay
     * latencia de memoria solapada (una fila de 2304 columnas son 72 unidades = 2,25 por lane). */
    for (int u0 = lane; u0 < units; u0 += 96) {
        uint4 q[3];
        uint4 hd[3];
#pragma unroll
        for (int j = 0; j < 3; j++) {
            int u = u0 + 32 * j;
            if (u < units) {
                const unsigned char *blk = row + (long)(u >> 3) * Q4K_BLOCK_BYTES;
                int r8 = u & 7;
                q[j] = *(const uint4 *)(blk + 16 + (r8 >> 1) * 32 + (r8 & 1) * 16);
                hd[j] = *(const uint4 *)blk;
            }
        }
#pragma unroll
        for (int j = 0; j < 3; j++) {
            int u = u0 + 32 * j;
            if (u < units) {
                int sb = u >> 3;
                int r8 = u & 7;
                int pair = r8 >> 1;
                int half = r8 & 1;
                union {
                    uint4 v;
                    unsigned char b[16];
                } h;
                h.v = hd[j];
                float d = q4k_f16((unsigned short)(h.b[0] | ((unsigned short)h.b[1] << 8)));
                float dmin = q4k_f16((unsigned short)(h.b[2] | ((unsigned short)h.b[3] << 8)));
                unsigned char sl, ml, sh, mh;
                q4k_scale_min(h.b + 4, 2 * pair, &sl, &ml);
                q4k_scale_min(h.b + 4, 2 * pair + 1, &sh, &mh);
                const float4 *xl = (const float4 *)(xs + (sb * 8 + 2 * pair) * XPAD + half * 16);
                const float4 *xh = (const float4 *)(xs + (sb * 8 + 2 * pair + 1) * XPAD + half * 16);
                unsigned w[4] = {q[j].x, q[j].y, q[j].z, q[j].w};
                float ql = 0.0f, sxl = 0.0f, qh = 0.0f, sxh = 0.0f;
#pragma unroll
                for (int i = 0; i < 4; i++) {
                    float4 a = xl[i];
                    float4 b = xh[i];
                    unsigned v = w[i];
                    ql += (float)(v & 0xfu) * a.x + (float)((v >> 8) & 0xfu) * a.y + (float)((v >> 16) & 0xfu) * a.z + (float)((v >> 24) & 0xfu) * a.w;
                    sxl += a.x + a.y + a.z + a.w;
                    qh += (float)((v >> 4) & 0xfu) * b.x + (float)((v >> 12) & 0xfu) * b.y + (float)((v >> 20) & 0xfu) * b.z + (float)((v >> 28) & 0xfu) * b.w;
                    sxh += b.x + b.y + b.z + b.w;
                }
                acc += (d * (float)sl) * ql - (dmin * (float)ml) * sxl + (d * (float)sh) * qh - (dmin * (float)mh) * sxh;
            }
        }
    }
    return acc;
}
/* Variante de R filas por warp: cada unidad (16 B de nibbles de cada fila) reutiliza los 32 floats de `x` leídos de shared
 * (con una fila por warp, `x` pesaba tanto como los pesos). Las filas son `fila0 + r * stride_filas`. */
template <int R>
__device__ __forceinline__ void q4k_rows_dot(const unsigned char *base, long row_bytes, int stride_filas, int nrows_validas,
                                             const float *xs, int cols, int lane, float *acc)
{
    int units = (cols >> 8) * 8;
#pragma unroll
    for (int r = 0; r < R; r++) {
        acc[r] = 0.0f;
    }
    for (int u = lane; u < units; u += 32) {
        int sb = u >> 3;
        int r8 = u & 7;
        int pair = r8 >> 1;
        int half = r8 & 1;
        uint4 q[R];
        uint4 hd[R];
#pragma unroll
        for (int r = 0; r < R; r++) {
            if (r * stride_filas < nrows_validas) {
                const unsigned char *blk = base + (long)(r * stride_filas) * row_bytes + (long)sb * Q4K_BLOCK_BYTES;
                q[r] = *(const uint4 *)(blk + 16 + pair * 32 + half * 16);
                hd[r] = *(const uint4 *)blk;
            }
        }
        const float4 *xl = (const float4 *)(xs + (sb * 8 + 2 * pair) * XPAD + half * 16);
        const float4 *xh = (const float4 *)(xs + (sb * 8 + 2 * pair + 1) * XPAD + half * 16);
        float4 a[4], b[4];
        float sxl = 0.0f, sxh = 0.0f;
#pragma unroll
        for (int i = 0; i < 4; i++) {
            a[i] = xl[i];
            b[i] = xh[i];
            sxl += a[i].x + a[i].y + a[i].z + a[i].w;
            sxh += b[i].x + b[i].y + b[i].z + b[i].w;
        }
#pragma unroll
        for (int r = 0; r < R; r++) {
            if (r * stride_filas < nrows_validas) {
                union {
                    uint4 v;
                    unsigned char b[16];
                } h;
                h.v = hd[r];
                float d = q4k_f16((unsigned short)(h.b[0] | ((unsigned short)h.b[1] << 8)));
                float dmin = q4k_f16((unsigned short)(h.b[2] | ((unsigned short)h.b[3] << 8)));
                unsigned char sl, ml, sh, mh;
                q4k_scale_min(h.b + 4, 2 * pair, &sl, &ml);
                q4k_scale_min(h.b + 4, 2 * pair + 1, &sh, &mh);
                unsigned w[4] = {q[r].x, q[r].y, q[r].z, q[r].w};
                float ql = 0.0f, qh = 0.0f;
#pragma unroll
                for (int i = 0; i < 4; i++) {
                    unsigned v = w[i];
                    ql += (float)(v & 0xfu) * a[i].x + (float)((v >> 8) & 0xfu) * a[i].y + (float)((v >> 16) & 0xfu) * a[i].z + (float)((v >> 24) & 0xfu) * a[i].w;
                    qh += (float)((v >> 4) & 0xfu) * b[i].x + (float)((v >> 12) & 0xfu) * b[i].y + (float)((v >> 20) & 0xfu) * b[i].z + (float)((v >> 28) & 0xfu) * b[i].w;
                }
                acc[r] += (d * (float)sl) * ql - (dmin * (float)ml) * sxl + (d * (float)sh) * qh - (dmin * (float)mh) * sxh;
            }
        }
    }
}

#define Q4K_R 4

/* NOTA: los 16 bytes de una unidad cubren los elementos [half*16, half*16+16) de DOS sub-bloques (el bajo y el alto),
 * por eso `a` (x del sub-bloque bajo) y `b` (x del alto) se leen a 32 floats de distancia. */

/* Producto de una fila Q8_0 (bloques de 36 bytes: escala f32 + 32 int8) por `x` (f32 en memoria global/shared). */
__device__ __forceinline__ float q80_row_dot(const unsigned char *row, const float *x, int nb, int lane)
{
    float acc = 0.0f;
    for (int b = lane; b < nb; b += 32) {
        const unsigned *w = (const unsigned *)(row + (long)b * 36);
        float sc = __uint_as_float(w[0]);
        const float4 *xp = (const float4 *)(x + (long)b * 32);
        float s = 0.0f;
#pragma unroll
        for (int i = 0; i < 8; i++) {
            unsigned v = w[1 + i];
            float4 a = xp[i];
            s += (float)(signed char)(v & 0xffu) * a.x + (float)(signed char)((v >> 8) & 0xffu) * a.y +
                 (float)(signed char)((v >> 16) & 0xffu) * a.z + (float)(signed char)(v >> 24) * a.w;
        }
        acc += sc * s;
    }
    return acc;
}

/* y[t][r] = W[r]·x[t] para los `n` tokens (un lanzamiento por token: `x` y `y` ya apuntan a su vector). grid = rows/8,
 * 256 hilos; `cols` <= 4096 (x en shared). */
extern "C" __global__ void gm_matvec_q4k(const unsigned char *w, const float *x, float *y, int rows, int cols)
{
    __shared__ __align__(16) float xs[XS_FLOATS];
    for (int i = (int)threadIdx.x; i < cols; i += (int)blockDim.x) {
        xs[(i >> 5) * XPAD + (i & 31)] = x[i];
    }
    __syncthreads();
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row = (int)blockIdx.x * 8 + warp;
    if (row >= rows) {
        return;
    }
    float s = q4k_row_dot(w + (long)row * (long)(cols >> 8) * Q4K_BLOCK_BYTES, xs, cols, lane);
    s = gm_warp_sum(s);
    if (lane == 0) {
        y[row] = s;
    }
}

/* Tres matvec Q4_K con la misma `x` (Q, K y V) en un solo lanzamiento: más CTAs a la vez y una carga de `x` por CTA.
 * grid = (rows0 + rows1 + rows2) / 8 (cada `rows_i` múltiplo de 8); un CTA pertenece a una sola matriz. */
extern "C" __global__ void gm_matvec_q4k_x3(const unsigned char *w0, const unsigned char *w1, const unsigned char *w2,
                                            const float *x, float *y0, float *y1, float *y2, int rows0, int rows1,
                                            int rows2, int cols)
{
    __shared__ __align__(16) float xs[XS_FLOATS];
    for (int i = (int)threadIdx.x; i < cols; i += (int)blockDim.x) {
        xs[(i >> 5) * XPAD + (i & 31)] = x[i];
    }
    __syncthreads();
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    /* Cada warp: 4 filas consecutivas; el CTA cubre 32 filas de UNA matriz (cada rows_i múltiplo de 32). */
    int fila = ((int)blockIdx.x * 8 + warp) * Q4K_R;
    const unsigned char *w;
    float *y;
    int nv;
    if (fila < rows0) {
        w = w0;
        y = y0;
        nv = rows0 - fila;
    } else if (fila < rows0 + rows1) {
        fila -= rows0;
        w = w1;
        y = y1;
        nv = rows1 - fila;
    } else {
        fila -= rows0 + rows1;
        if (fila >= rows2) {
            return;
        }
        w = w2;
        y = y2;
        nv = rows2 - fila;
    }
    long rb = (long)(cols >> 8) * Q4K_BLOCK_BYTES;
    float acc[Q4K_R];
    q4k_rows_dot<Q4K_R>(w + (long)fila * rb, rb, 1, nv, xs, cols, lane, acc);
#pragma unroll
    for (int r = 0; r < Q4K_R; r++) {
        float s = gm_warp_sum(acc[r]);
        if (lane == 0 && r < nv) {
            y[fila + r] = s;
        }
    }
}

extern "C" __global__ void gm_matvec_q80(const unsigned char *w, const float *x, float *y, int rows, int cols)
{
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row = (int)blockIdx.x * 8 + warp;
    if (row >= rows) {
        return;
    }
    float s = q80_row_dot(w + (long)row * (long)(cols >> 5) * 36, x, cols >> 5, lane);
    s = gm_warp_sum(s);
    if (lane == 0) {
        y[row] = s;
    }
}

/* Router en dos pasos: logits (un warp por experto, grid = (ceil(n_exp / 8), n)) y luego softmax + top-k. */
extern "C" __global__ void mo_router_logits(const float *xn, const float *w, float *lg, int h, int n_exp)
{
    int t = (int)blockIdx.y;
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int e = (int)blockIdx.x * 8 + warp;
    if (e >= n_exp) {
        return;
    }
    const float4 *we = (const float4 *)(w + (long)e * h);
    const float4 *x = (const float4 *)(xn + (long)t * h);
    float s = 0.0f;
    for (int i = lane; i < (h >> 2); i += 32) {
        float4 a = we[i];
        float4 b = x[i];
        s += a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w;
    }
    s = gm_warp_sum(s);
    if (lane == 0) {
        lg[(long)t * n_exp + e] = s;
    }
}

extern "C" __global__ void mo_router_topk(const float *lgall, int *idx, float *wgt, int n_exp, int top_k)
{
    __shared__ float lg[256];
    int t = (int)blockIdx.x;
    for (int e = (int)threadIdx.x; e < n_exp; e += (int)blockDim.x) {
        lg[e] = lgall[(long)t * n_exp + e];
    }
    __syncthreads();
    if (threadIdx.x == 0) {
        float m = -3.0e38f;
        for (int e = 0; e < n_exp; e++) {
            m = fmaxf(m, lg[e]);
        }
        float suma = 0.0f;
        for (int e = 0; e < n_exp; e++) {
            lg[e] = expf(lg[e] - m);
            suma += lg[e];
        }
        for (int e = 0; e < n_exp; e++) {
            lg[e] /= suma;
        }
        float tot = 0.0f;
        for (int j = 0; j < top_k; j++) {
            int mejor = -1;
            float mv = -1.0f;
            for (int e = 0; e < n_exp; e++) {
                if (lg[e] > mv) {
                    mv = lg[e];
                    mejor = e;
                }
            }
            idx[t * top_k + j] = mejor;
            wgt[t * top_k + j] = mv;
            tot += mv;
            lg[mejor] = -2.0f;
        }
        if (tot > 0.0f) {
            for (int j = 0; j < top_k; j++) {
                wgt[t * top_k + j] /= tot;
            }
        }
    }
}

/* Router: softmax de los `n_exp` logits y top-k renormalizado, idéntico a `topk_softmax` de la CPU (empates: gana el
 * índice menor). grid = n tokens; 256 hilos = 8 warps, cada warp un experto por vuelta. */
extern "C" __global__ void mo_router(const float *xn, const float *w, int *idx, float *wgt, int h, int n_exp, int top_k)
{
    __shared__ float lg[256];
    int t = (int)blockIdx.x;
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    const float *x = xn + (long)t * h;
    for (int e = warp; e < n_exp; e += 8) {
        const float *we = w + (long)e * h;
        float s = 0.0f;
        for (int i = lane; i < h; i += 32) {
            s += we[i] * x[i];
        }
        s = gm_warp_sum(s);
        if (lane == 0) {
            lg[e] = s;
        }
    }
    __syncthreads();
    if (threadIdx.x == 0) {
        float m = -3.0e38f;
        for (int e = 0; e < n_exp; e++) {
            m = fmaxf(m, lg[e]);
        }
        float suma = 0.0f;
        for (int e = 0; e < n_exp; e++) {
            lg[e] = expf(lg[e] - m);
            suma += lg[e];
        }
        for (int e = 0; e < n_exp; e++) {
            lg[e] /= suma;
        }
        float tot = 0.0f;
        for (int j = 0; j < top_k; j++) {
            int mejor = -1;
            float mv = -1.0f;
            for (int e = 0; e < n_exp; e++) {
                if (lg[e] > mv) {
                    mv = lg[e];
                    mejor = e;
                }
            }
            idx[t * top_k + j] = mejor;
            wgt[t * top_k + j] = mv;
            tot += mv;
            lg[mejor] = -2.0f;
        }
        if (tot > 0.0f) {
            for (int j = 0; j < top_k; j++) {
                wgt[t * top_k + j] /= tot;
            }
        }
    }
}

/* act[slot][r] = silu(gate[r]) * up[r] para el experto elegido de cada slot (slot = token * top_k + j).
 * grid = (ceil(ffn / 8), slots); 256 hilos = 8 warps = 8 filas. `h` <= 4096 (x en shared). */
#define MO_X_MAX 4096
extern "C" __global__ void mo_gateup_q4k(const float *xn, const unsigned long long *pg, const unsigned long long *pu,
                                         const int *idx, float *act, int h, int ffn, int top_k)
{
    __shared__ __align__(16) float xs[XS_FLOATS];
    int slot = (int)blockIdx.y;
    int t = slot / top_k;
    for (int i = (int)threadIdx.x; i < h; i += (int)blockDim.x) {
        xs[(i >> 5) * XPAD + (i & 31)] = xn[(long)t * h + i];
    }
    __syncthreads();
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int fila = ((int)blockIdx.x * 8 + warp) * Q4K_R;
    if (fila >= ffn) {
        return;
    }
    int nv = ffn - fila;
    int e = idx[slot];
    long rb = (long)(h / Q4K_BLOCK_ELEMS) * (long)Q4K_BLOCK_BYTES;
    float g[Q4K_R], u[Q4K_R];
    q4k_rows_dot<Q4K_R>((const unsigned char *)pg[e] + (long)fila * rb, rb, 1, nv, xs, h, lane, g);
    q4k_rows_dot<Q4K_R>((const unsigned char *)pu[e] + (long)fila * rb, rb, 1, nv, xs, h, lane, u);
#pragma unroll
    for (int r = 0; r < Q4K_R; r++) {
        float gs = gm_warp_sum(g[r]);
        float us = gm_warp_sum(u[r]);
        if (lane == 0 && r < nv) {
            act[(long)slot * ffn + fila + r] = (gs / (1.0f + expf(-gs))) * us;
        }
    }
}

/* y[slot][r] = down[e] · act[slot]; down es Q8_0 (bloques de 36 bytes: escala f32 + 32 int8).
 * grid = (ceil(h / 8), slots); 256 hilos. */
#define Q80_R 4
extern "C" __global__ void mo_down_q80(const float *act, const unsigned long long *pd, const int *idx, float *y, int h,
                                       int ffn)
{
    int slot = (int)blockIdx.y;
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int fila = ((int)blockIdx.x * 8 + warp) * Q80_R;
    if (fila >= h) {
        return;
    }
    int e = idx[slot];
    int nb = ffn >> 5;
    const unsigned char *w = (const unsigned char *)pd[e] + (long)fila * (long)nb * 36;
    const float *x = act + (long)slot * ffn;
    float acc[Q80_R];
#pragma unroll
    for (int r = 0; r < Q80_R; r++) {
        acc[r] = 0.0f;
    }
    for (int b = lane; b < nb; b += 32) {
        unsigned wd[Q80_R][9];
#pragma unroll
        for (int r = 0; r < Q80_R; r++) {
            if (fila + r < h) {
                const unsigned *wp = (const unsigned *)(w + (long)r * nb * 36 + (long)b * 36);
#pragma unroll
                for (int i = 0; i < 9; i++) {
                    wd[r][i] = wp[i];
                }
            }
        }
        const float4 *xp = (const float4 *)(x + (long)b * 32);
        float4 xv[8];
#pragma unroll
        for (int i = 0; i < 8; i++) {
            xv[i] = xp[i];
        }
#pragma unroll
        for (int r = 0; r < Q80_R; r++) {
            if (fila + r < h) {
                float s = 0.0f;
#pragma unroll
                for (int i = 0; i < 8; i++) {
                    unsigned v = wd[r][1 + i];
                    s += (float)(signed char)(v & 0xffu) * xv[i].x + (float)(signed char)((v >> 8) & 0xffu) * xv[i].y +
                         (float)(signed char)((v >> 16) & 0xffu) * xv[i].z + (float)(signed char)(v >> 24) * xv[i].w;
                }
                acc[r] += __uint_as_float(wd[r][0]) * s;
            }
        }
    }
#pragma unroll
    for (int r = 0; r < Q80_R; r++) {
        float s = gm_warp_sum(acc[r]);
        if (lane == 0 && fila + r < h) {
            y[(long)slot * h + fila + r] = s;
        }
    }
}

/* hs[t][i] = Σ_j wgt[t][j] * y[t*top_k + j][i] + hs[t][i]  (la suma de los expertos y luego el residual, en el
 * mismo orden que la CPU). grid = (ceil(h / 256), n). */
extern "C" __global__ void mo_combine(float *hs, const float *y, const float *wgt, int h, int top_k)
{
    int t = (int)blockIdx.y;
    int i = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    if (i >= h) {
        return;
    }
    float acc = 0.0f;
    for (int j = 0; j < top_k; j++) {
        acc += y[((long)t * top_k + j) * h + i] * wgt[t * top_k + j];
    }
    hs[(long)t * h + i] = acc + hs[(long)t * h + i];
}

/* ------------------------------------------------------------------------------------------------------------
 * Atención GQA "flash-decoding" (hd = 128, hasta 8 cabezas de consulta por cabeza KV). Cada CTA atiende un trozo de
 * ATT_CH posiciones de UNA cabeza KV y de UN token: lee K y V una sola vez para todas las cabezas del grupo (el camino
 * por cabeza los releía G veces) y no pasa por memoria global con las puntuaciones. Cada trozo deja su máximo, su
 * suma y su acumulador de P·V sin normalizar; `mo_attn_combine` los junta.
 * ---------------------------------------------------------------------------------------------------------- */
#define ATT_CH 256
#define ATT_HD 128
#define ATT_G 8

extern "C" __global__ void mo_attn_split(const float *q, const __half *dk, const __half *dv, float *parcial, float *ml,
                                         int heads, int kvh, int kv_dim, int seq_antes, int ventana, float escala, int S,
                                         const int *dyn)
{
    __shared__ float sc[ATT_G][ATT_CH];
    if (dyn != 0) {
        seq_antes = dyn[1];
    }
    int s = (int)blockIdx.x;
    int g = (int)blockIdx.y;
    int t = (int)blockIdx.z;
    int tid = (int)threadIdx.x;
    int G = heads / kvh;
    int total = seq_antes + t + 1;
    int t_ini = (ventana > 0 && total > ventana) ? total - ventana : 0;
    int a = t_ini + s * ATT_CH;
    int b = a + ATT_CH < total ? a + ATT_CH : total;
    long base_o = (((long)t * kvh + g) * S + s);
    if (a >= total) {
        if (tid < G) {
            ml[(base_o * ATT_G + tid) * 2] = -3.0e38f;
            ml[(base_o * ATT_G + tid) * 2 + 1] = 0.0f;
        }
        return; /* trozo vacío: el combinador lo ignora (máximo -inf) */
    }
    /* Pasada 1: puntuaciones del trozo. Un warp por posición: los 32 hilos leen la fila de K entera (256 B contiguos) y
     * cada uno guarda 4 dimensiones de q para las G cabezas en registros; las G sumas parciales se reducen entre los 32
     * hilos con una mariposa (9 shuffles en lugar de 8 reducciones completas). */
    int warp = tid >> 5;
    int lane = tid & 31;
    float qv[ATT_G][4];
#pragma unroll
    for (int j = 0; j < ATT_G; j++) {
#pragma unroll
        for (int i = 0; i < 4; i++) {
            qv[j][i] = j < G ? q[((long)t * heads + g * G + j) * ATT_HD + lane * 4 + i] * escala : 0.0f;
        }
    }
    int hcab = ((lane & 16) ? 4 : 0) + ((lane & 8) ? 2 : 0) + ((lane & 4) ? 1 : 0); /* cabeza que queda en este hilo */
#pragma unroll 4
    for (int p = a + warp; p < b; p += 8) {
        uint2 raw = *(const uint2 *)(dk + (long)p * kv_dim + (long)g * ATT_HD + lane * 4);
        float2 k01 = __half22float2(*(const __half2 *)&raw.x);
        float2 k23 = __half22float2(*(const __half2 *)&raw.y);
        float v[ATT_G];
#pragma unroll
        for (int j = 0; j < ATT_G; j++) {
            v[j] = k01.x * qv[j][0] + k01.y * qv[j][1] + k23.x * qv[j][2] + k23.y * qv[j][3];
        }
        float w4[4];
#pragma unroll
        for (int i = 0; i < 4; i++) {
            float envio = (lane & 16) ? v[i] : v[i + 4];
            float recibo = __shfl_xor_sync(0xffffffffu, envio, 16);
            w4[i] = ((lane & 16) ? v[i + 4] : v[i]) + recibo;
        }
        float w2[2];
#pragma unroll
        for (int i = 0; i < 2; i++) {
            float envio = (lane & 8) ? w4[i] : w4[i + 2];
            float recibo = __shfl_xor_sync(0xffffffffu, envio, 8);
            w2[i] = ((lane & 8) ? w4[i + 2] : w4[i]) + recibo;
        }
        float envio = (lane & 4) ? w2[0] : w2[1];
        float recibo = __shfl_xor_sync(0xffffffffu, envio, 4);
        float w1 = ((lane & 4) ? w2[1] : w2[0]) + recibo;
        w1 += __shfl_xor_sync(0xffffffffu, w1, 2);
        w1 += __shfl_xor_sync(0xffffffffu, w1, 1);
        if ((lane & 3) == 0 && hcab < G) {
            sc[hcab][p - a] = w1;
        }
    }
    __syncthreads();
    /* Máximo y suma por cabeza (un warp por cabeza). */
    if (warp < G) {
        float m = -3.0e38f;
        for (int i = lane; i < b - a; i += 32) {
            m = fmaxf(m, sc[warp][i]);
        }
        m = gm_warp_max(m);
        m = __shfl_sync(0xffffffffu, m, 0);
        float l = 0.0f;
        for (int i = lane; i < b - a; i += 32) {
            float e = expf(sc[warp][i] - m);
            sc[warp][i] = e;
            l += e;
        }
        l = gm_warp_sum(l);
        if (lane == 0) {
            ml[(base_o * ATT_G + warp) * 2] = m;
            ml[(base_o * ATT_G + warp) * 2 + 1] = l;
        }
    }
    __syncthreads();
    /* Pasada 2: P·V sin normalizar. Hilo (d, mitad): 4 cabezas de una dimensión. */
    int d = tid & (ATT_HD - 1);
    int hh = tid >> 7;
    float o[4] = {0.0f, 0.0f, 0.0f, 0.0f};
#pragma unroll 8
    for (int p = a; p < b; p++) {
        float v = __half2float(dv[(long)p * kv_dim + (long)g * ATT_HD + d]);
#pragma unroll
        for (int j = 0; j < 4; j++) {
            o[j] += sc[hh * 4 + j][p - a] * v;
        }
    }
#pragma unroll
    for (int j = 0; j < 4; j++) {
        if (hh * 4 + j < G) {
            parcial[(base_o * ATT_G + hh * 4 + j) * ATT_HD + d] = o[j];
        }
    }
}

/* out[t][g*G + j][d] = Σ_s e^(m_s - M) · acc_s / Σ_s e^(m_s - M) · l_s. grid = (heads, n); 128 hilos. */
extern "C" __global__ void mo_attn_combine(const float *parcial, const float *ml, float *out, int heads, int kvh, int S)
{
    __shared__ float mm[128];
    __shared__ float ll[128];
    __shared__ float ww[128];
    int h = (int)blockIdx.x;
    int t = (int)blockIdx.y;
    int d = (int)threadIdx.x;
    int G = heads / kvh;
    int g = h / G;
    int j = h % G;
    long base = ((long)t * kvh + g) * S;
    /* Todos los (máximo, suma) de una vez, y el máximo global lo calcula un warp. */
    if (d < S) {
        long o = (base + d) * ATT_G + j;
        mm[d] = ml[o * 2];
        ll[d] = ml[o * 2 + 1];
    }
    __syncthreads();
    if (d < 32) {
        float M = -3.0e38f;
        for (int s = d; s < S; s += 32) {
            M = fmaxf(M, mm[s]);
        }
        M = gm_warp_max(M);
        M = __shfl_sync(0xffffffffu, M, 0);
        float den = 0.0f;
        for (int s = d; s < S; s += 32) {
            float w = mm[s] > -1.0e38f ? expf(mm[s] - M) : 0.0f;
            ww[s] = w;
            den += w * ll[s];
        }
        den = gm_warp_sum(den);
        if (d == 0) {
            ll[0] = den;
        }
    }
    __syncthreads();
    float den = ll[0];
    float acc = 0.0f;
#pragma unroll 8
    for (int s = 0; s < S; s++) {
        long o = (base + s) * ATT_G + j;
        acc += ww[s] * parcial[o * ATT_HD + d];
    }
    out[((long)t * heads + h) * ATT_HD + d] = den > 0.0f ? acc / den : 0.0f;
}

/* ------------------------------------------------------------------------------------------------------------
 * Prefill por bloques de muchos tokens (n >= 4): los pesos se leen una vez para varios tokens.
 *   mo_group          agrupa los slots (token, j) por experto elegido.
 *   mo_gateup_grp     gate/up de un experto para todos los tokens que lo eligieron (TT tokens por lectura de pesos).
 *   mo_down_grp       down de un experto para sus tokens.
 *   gm_matmul_q4k_n   Q4_K x n tokens; gm_matmul_q80_n Q8_0 x n tokens (proyecciones de atención).
 * Layout igual que el resto: x[t*cols + i], y[t*rows + fila].
 * ---------------------------------------------------------------------------------------------------------- */
#define TT 4

/* start[e] .. start[e+1] son las posiciones de ord[] con los slots (t*top_k + j) que eligieron el experto e. 1 CTA. */
extern "C" __global__ void mo_group(const int *idx, int *start, int *ord, int n_slots, int n_exp)
{
    __shared__ int cnt[256];
    int e = (int)threadIdx.x;
    if (e < n_exp) {
        int c = 0;
        for (int s = 0; s < n_slots; s++) {
            c += idx[s] == e;
        }
        cnt[e] = c;
    }
    __syncthreads();
    if (e == 0) {
        int acc = 0;
        for (int i = 0; i < n_exp; i++) {
            start[i] = acc;
            acc += cnt[i];
        }
        start[n_exp] = acc;
    }
    __syncthreads();
    if (e < n_exp) {
        int o = start[e];
        for (int s = 0; s < n_slots; s++) {
            if (idx[s] == e) {
                ord[o++] = s;
            }
        }
    }
}

/* R filas x hasta TT tokens de una matriz Q4_K. `xs`: x de los tokens ya en shared, `xt` floats por token. */
template <int R>
__device__ __forceinline__ void q4k_rows_toks(const unsigned char *base, long rb, int nv, const float *xs, int xt, int ntok,
                                              int cols, int lane, float (*acc)[TT])
{
    int units = (cols >> 8) * 8;
#pragma unroll
    for (int r = 0; r < R; r++) {
#pragma unroll
        for (int k = 0; k < TT; k++) {
            acc[r][k] = 0.0f;
        }
    }
    for (int u = lane; u < units; u += 32) {
        int sb = u >> 3;
        int r8 = u & 7;
        int pair = r8 >> 1;
        int half = r8 & 1;
        uint4 q[R], hd[R];
#pragma unroll
        for (int r = 0; r < R; r++) {
            if (r < nv) {
                const unsigned char *blk = base + (long)r * rb + (long)sb * Q4K_BLOCK_BYTES;
                q[r] = *(const uint4 *)(blk + 16 + pair * 32 + half * 16);
                hd[r] = *(const uint4 *)blk;
            }
        }
        float dd[R], dm[R], sl[R], ml[R], sh[R], mh[R];
#pragma unroll
        for (int r = 0; r < R; r++) {
            if (r < nv) {
                union {
                    uint4 v;
                    unsigned char b[16];
                } h;
                h.v = hd[r];
                dd[r] = q4k_f16((unsigned short)(h.b[0] | ((unsigned short)h.b[1] << 8)));
                dm[r] = q4k_f16((unsigned short)(h.b[2] | ((unsigned short)h.b[3] << 8)));
                unsigned char a1, a2, a3, a4;
                q4k_scale_min(h.b + 4, 2 * pair, &a1, &a2);
                q4k_scale_min(h.b + 4, 2 * pair + 1, &a3, &a4);
                sl[r] = (float)a1;
                ml[r] = (float)a2;
                sh[r] = (float)a3;
                mh[r] = (float)a4;
            }
        }
#pragma unroll
        for (int k = 0; k < TT; k++) {
            if (k < ntok) {
                const float4 *xl = (const float4 *)(xs + k * xt + (sb * 8 + 2 * pair) * XPAD + half * 16);
                const float4 *xh = (const float4 *)(xs + k * xt + (sb * 8 + 2 * pair + 1) * XPAD + half * 16);
                float4 a[4], b[4];
                float sxl = 0.0f, sxh = 0.0f;
#pragma unroll
                for (int i = 0; i < 4; i++) {
                    a[i] = xl[i];
                    b[i] = xh[i];
                    sxl += a[i].x + a[i].y + a[i].z + a[i].w;
                    sxh += b[i].x + b[i].y + b[i].z + b[i].w;
                }
#pragma unroll
                for (int r = 0; r < R; r++) {
                    if (r < nv) {
                        unsigned w[4] = {q[r].x, q[r].y, q[r].z, q[r].w};
                        float ql = 0.0f, qh = 0.0f;
#pragma unroll
                        for (int i = 0; i < 4; i++) {
                            unsigned v = w[i];
                            ql += (float)(v & 0xfu) * a[i].x + (float)((v >> 8) & 0xfu) * a[i].y + (float)((v >> 16) & 0xfu) * a[i].z + (float)((v >> 24) & 0xfu) * a[i].w;
                            qh += (float)((v >> 4) & 0xfu) * b[i].x + (float)((v >> 12) & 0xfu) * b[i].y + (float)((v >> 20) & 0xfu) * b[i].z + (float)((v >> 28) & 0xfu) * b[i].w;
                        }
                        acc[r][k] += (dd[r] * sl[r]) * ql - (dm[r] * ml[r]) * sxl + (dd[r] * sh[r]) * qh - (dm[r] * mh[r]) * sxh;
                    }
                }
            }
        }
    }
}

#define GR 2 /* filas por warp en los kernels agrupados de Q4_K: 16 filas por CTA */

extern "C" __global__ void gm_matmul_q4k_n(const unsigned char *w, const float *x, float *y, int rows, int cols, int n, int tc)
{
    extern __shared__ __align__(16) float xs_dyn[];
    int xt = (cols >> 5) * XPAD;
    int tid = (int)threadIdx.x;
    int warp = tid >> 5;
    int lane = tid & 31;
    int fila = ((int)blockIdx.x * 8 + warp) * GR;
    int nv = rows - fila;
    nv = nv < 0 ? 0 : (nv > GR ? GR : nv);
    long rb = (long)(cols >> 8) * Q4K_BLOCK_BYTES;
    for (int c0 = 0; c0 < n; c0 += tc) {
        int ntok = n - c0 < tc ? n - c0 : tc;
        __syncthreads();
        for (int k = 0; k < ntok; k++) {
            const float4 *src = (const float4 *)(x + (long)(c0 + k) * cols);
            for (int j = tid; j < (cols >> 2); j += 256) {
                *(float4 *)&xs_dyn[k * xt + (j >> 3) * XPAD + (j & 7) * 4] = src[j];
            }
        }
        __syncthreads();
        float acc[GR][TT];
        q4k_rows_toks<GR>(w + (long)fila * rb, rb, nv, xs_dyn, xt, ntok, cols, lane, acc);
#pragma unroll
        for (int r = 0; r < GR; r++) {
#pragma unroll
            for (int k = 0; k < TT; k++) {
                float s = gm_warp_sum(acc[r][k]);
                if (lane == 0 && k < ntok && r < nv) {
                    y[(long)(c0 + k) * rows + fila + r] = s;
                }
            }
        }
    }
}

/* R filas x hasta TT tokens de una matriz Q8_0 (bloques de 36 bytes); xrow[k] = x del token k en memoria global. */
#define Q8R 4
__device__ __forceinline__ void q80_rows_toks(const unsigned char *w, long row_bytes, int nv, const float *const *xrow, int ntok,
                                              int nb, int lane, float (*acc)[TT])
{
#pragma unroll
    for (int r = 0; r < Q8R; r++) {
#pragma unroll
        for (int k = 0; k < TT; k++) {
            acc[r][k] = 0.0f;
        }
    }
    for (int b = lane; b < nb; b += 32) {
        unsigned wd[Q8R][9];
#pragma unroll
        for (int r = 0; r < Q8R; r++) {
            if (r < nv) {
                const unsigned *wp = (const unsigned *)(w + (long)r * row_bytes + (long)b * 36);
#pragma unroll
                for (int i = 0; i < 9; i++) {
                    wd[r][i] = wp[i];
                }
            }
        }
#pragma unroll
        for (int k = 0; k < TT; k++) {
            if (k < ntok) {
                const float4 *xp = (const float4 *)(xrow[k] + (long)b * 32);
                float4 xv[8];
#pragma unroll
                for (int i = 0; i < 8; i++) {
                    xv[i] = xp[i];
                }
#pragma unroll
                for (int r = 0; r < Q8R; r++) {
                    if (r < nv) {
                        float s = 0.0f;
#pragma unroll
                        for (int i = 0; i < 8; i++) {
                            unsigned v = wd[r][1 + i];
                            s += (float)(signed char)(v & 0xffu) * xv[i].x + (float)(signed char)((v >> 8) & 0xffu) * xv[i].y +
                                 (float)(signed char)((v >> 16) & 0xffu) * xv[i].z + (float)(signed char)(v >> 24) * xv[i].w;
                        }
                        acc[r][k] += __uint_as_float(wd[r][0]) * s;
                    }
                }
            }
        }
    }
}

extern "C" __global__ void gm_matmul_q80_n(const unsigned char *w, const float *x, float *y, int rows, int cols, int n)
{
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int fila = ((int)blockIdx.x * 8 + warp) * Q8R;
    if (fila >= rows) {
        return;
    }
    int nv = rows - fila < Q8R ? rows - fila : Q8R;
    int nb = cols >> 5;
    long rb = (long)nb * 36;
    for (int c0 = 0; c0 < n; c0 += TT) {
        int ntok = n - c0 < TT ? n - c0 : TT;
        const float *xrow[TT];
#pragma unroll
        for (int k = 0; k < TT; k++) {
            xrow[k] = x + (long)(c0 + (k < ntok ? k : 0)) * cols;
        }
        float acc[Q8R][TT];
        q80_rows_toks(w + (long)fila * rb, rb, nv, xrow, ntok, nb, lane, acc);
#pragma unroll
        for (int r = 0; r < Q8R; r++) {
#pragma unroll
            for (int k = 0; k < TT; k++) {
                float s = gm_warp_sum(acc[r][k]);
                if (lane == 0 && k < ntok && r < nv) {
                    y[(long)(c0 + k) * rows + fila + r] = s;
                }
            }
        }
    }
}

/* act[slot][fila] = silu(gate) * up para los tokens que eligieron el experto e = blockIdx.y. grid = (ceil(ffn / 16), n_exp);
 * shared dinámica = TT * (h/32*XPAD) floats. */
extern "C" __global__ void mo_gateup_grp(const float *xn, const unsigned long long *pg, const unsigned long long *pu,
                                         const int *start, const int *ord, float *act, int h, int ffn, int top_k)
{
    extern __shared__ __align__(16) float xs_dyn[];
    int e = (int)blockIdx.y;
    int cnt = start[e + 1] - start[e];
    if (cnt == 0) {
        return;
    }
    int xt = (h >> 5) * XPAD;
    int tid = (int)threadIdx.x;
    int warp = tid >> 5;
    int lane = tid & 31;
    int fila = ((int)blockIdx.x * 8 + warp) * GR;
    int nv = ffn - fila;
    nv = nv < 0 ? 0 : (nv > GR ? GR : nv);
    long rb = (long)(h >> 8) * Q4K_BLOCK_BYTES;
    const unsigned char *wg = (const unsigned char *)pg[e] + (long)fila * rb;
    const unsigned char *wu = (const unsigned char *)pu[e] + (long)fila * rb;
    for (int c0 = 0; c0 < cnt; c0 += TT) {
        int ntok = cnt - c0 < TT ? cnt - c0 : TT;
        __syncthreads();
        for (int k = 0; k < ntok; k++) {
            int t = ord[start[e] + c0 + k] / top_k;
            const float4 *src = (const float4 *)(xn + (long)t * h);
            for (int j = tid; j < (h >> 2); j += 256) {
                *(float4 *)&xs_dyn[k * xt + (j >> 3) * XPAD + (j & 7) * 4] = src[j];
            }
        }
        __syncthreads();
        float g[GR][TT], u[GR][TT];
        q4k_rows_toks<GR>(wg, rb, nv, xs_dyn, xt, ntok, h, lane, g);
        q4k_rows_toks<GR>(wu, rb, nv, xs_dyn, xt, ntok, h, lane, u);
#pragma unroll
        for (int r = 0; r < GR; r++) {
#pragma unroll
            for (int k = 0; k < TT; k++) {
                float gs = gm_warp_sum(g[r][k]);
                float us = gm_warp_sum(u[r][k]);
                if (lane == 0 && k < ntok && r < nv) {
                    int slot = ord[start[e] + c0 + k];
                    act[(long)slot * ffn + fila + r] = (gs / (1.0f + expf(-gs))) * us;
                }
            }
        }
    }
}

/* y[slot][fila] = down[e] · act[slot] para los tokens del experto e. grid = (ceil(h / 32), n_exp). */
extern "C" __global__ void mo_down_grp(const float *act, const unsigned long long *pd, const int *start, const int *ord,
                                       float *y, int h, int ffn)
{
    int e = (int)blockIdx.y;
    int cnt = start[e + 1] - start[e];
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int fila = ((int)blockIdx.x * 8 + warp) * Q8R;
    if (cnt == 0 || fila >= h) {
        return;
    }
    int nv = h - fila < Q8R ? h - fila : Q8R;
    int nb = ffn >> 5;
    long rb = (long)nb * 36;
    const unsigned char *w = (const unsigned char *)pd[e] + (long)fila * rb;
    for (int c0 = 0; c0 < cnt; c0 += TT) {
        int ntok = cnt - c0 < TT ? cnt - c0 : TT;
        const float *xrow[TT];
        int slots[TT];
#pragma unroll
        for (int k = 0; k < TT; k++) {
            slots[k] = ord[start[e] + c0 + (k < ntok ? k : 0)];
            xrow[k] = act + (long)slots[k] * ffn;
        }
        float acc[Q8R][TT];
        q80_rows_toks(w, rb, nv, xrow, ntok, nb, lane, acc);
#pragma unroll
        for (int r = 0; r < Q8R; r++) {
#pragma unroll
            for (int k = 0; k < TT; k++) {
                float s = gm_warp_sum(acc[r][k]);
                if (lane == 0 && k < ntok && r < nv) {
                    y[(long)slots[k] * h + fila + r] = s;
                }
            }
        }
    }
}
