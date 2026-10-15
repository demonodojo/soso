/* G9: matvec / matmul con la matriz Q4_0 (GGML) residente, para la GPU del host.
 *
 * Formato de entrada **reempaquetado al subir** (el de soso son bloques de 18 bytes, sin alineación):
 *   d  : u16[rows * nb]   escala f16 de cada bloque
 *   qs : uint4[rows * nb] los 16 bytes de nibbles de cada bloque, alineados a 16
 * donde nb = cols / 32. Elemento j (0..15) = nibble bajo del byte j; elemento j+16 = nibble alto.
 * Valor = d * (nibble - 8). Así cada lane lee 16 bytes alineados y las lecturas de un warp son contiguas.
 *
 * Un warp por fila de salida; los lanes se reparten los bloques de la fila y se reduce con shuffle.
 * `x` son `n` vectores de `cols` floats seguidos; `y` son `n` vectores de `rows` floats (y[t*rows + fila]).
 * `cols` múltiplo de 32 (lo comprueba el que lanza). `n` <= NT.
 *
 * Dos instancias de la misma plantilla: `matvec_q4_0r` (NT=1, decodificación) y `matmul_q4_0r8` (NT=8,
 * prefill: la fila de pesos se decodifica una vez y se aplica a los 8 tokens).
 */
#include <cuda_fp16.h>

__device__ __forceinline__ float q40_warp_sum(float v)
{
    unsigned mask = 0xffffffffu;
    v += __shfl_down_sync(mask, v, 16);
    v += __shfl_down_sync(mask, v, 8);
    v += __shfl_down_sync(mask, v, 4);
    v += __shfl_down_sync(mask, v, 2);
    v += __shfl_down_sync(mask, v, 1);
    return v;
}

/* CTA de 8 warps = 8 filas de salida. La dimensión K se recorre por trozos de 32 bloques (1024 columnas): el CTA
 * carga en shared el trozo de `x` de los `n` tokens UNA vez y los 8 warps lo comparten (sin esto cada warp
 * releía todo `x` de la caché por fila: 15360 filas × 123 KB). Cada bloque de `x` ocupa 36 floats en shared
 * (32 + 4 de relleno): los lanes leen bloques distintos con zancada de 36 palabras, que en fases de 8 lanes
 * (float4) cae en bancos distintos. */
#define Q40_WARPS 8
#define Q40_PAD 36

template <int NT>
__device__ __forceinline__ void q40_body(const unsigned short *d, const uint4 *qs, const float *x, float *y,
                                         int rows, int cols, int n)
{
    __shared__ __align__(16) float xs[NT][32 * Q40_PAD];
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row = (int)blockIdx.x * Q40_WARPS + warp;
    int nb = cols >> 5;
    const unsigned short *dr = d + (long)row * (long)nb;
    const uint4 *qr = qs + (long)row * (long)nb;
    float acc[NT];
#pragma unroll
    for (int t = 0; t < NT; t++) {
        acc[t] = 0.0f;
    }
    for (int c0 = 0; c0 < nb; c0 += 32) {
        int nblk = nb - c0 < 32 ? nb - c0 : 32;
        __syncthreads();
        for (int i = (int)threadIdx.x; i < NT * nblk * 8; i += (int)blockDim.x) {
            int t = i / (nblk * 8);
            int r = i - t * nblk * 8;
            int blk = r >> 3;
            int q = r & 7;
            if (t < n) {
                float4 v = ((const float4 *)(x + (long)t * (long)cols + (long)(c0 + blk) * 32))[q];
                *(float4 *)&xs[t][blk * Q40_PAD + q * 4] = v;
            }
        }
        __syncthreads();
        if (row < rows && lane < nblk) {
            int b = c0 + lane;
            float sc = __half2float(__ushort_as_half(dr[b]));
            uint4 q = qr[b];
            unsigned w[4] = {q.x, q.y, q.z, q.w};
            float v[32];
#pragma unroll
            for (int i = 0; i < 4; i++) {
#pragma unroll
                for (int k = 0; k < 4; k++) {
                    unsigned byte = (w[i] >> (8 * k)) & 0xffu;
                    v[i * 4 + k] = (float)((int)(byte & 0xfu) - 8);
                    v[16 + i * 4 + k] = (float)((int)(byte >> 4) - 8);
                }
            }
#pragma unroll
            for (int t = 0; t < NT; t++) {
                if (t < n) {
                    const float4 *xp = (const float4 *)&xs[t][lane * Q40_PAD];
                    float s = 0.0f;
#pragma unroll
                    for (int j = 0; j < 4; j++) {
                        float4 lo = xp[j];
                        float4 hi = xp[4 + j];
                        s += v[4 * j] * lo.x + v[4 * j + 1] * lo.y + v[4 * j + 2] * lo.z + v[4 * j + 3] * lo.w;
                        s += v[16 + 4 * j] * hi.x + v[16 + 4 * j + 1] * hi.y + v[16 + 4 * j + 2] * hi.z +
                             v[16 + 4 * j + 3] * hi.w;
                    }
                    acc[t] += sc * s;
                }
            }
        }
    }
#pragma unroll
    for (int t = 0; t < NT; t++) {
        if (t < n) {
            float r = q40_warp_sum(acc[t]);
            if (lane == 0 && row < rows) {
                y[(long)t * (long)rows + row] = r;
            }
        }
    }
}

extern "C" __global__ void matvec_q4_0r(const unsigned short *d, const uint4 *qs, const float *x, float *y,
                                        int rows, int cols)
{
    q40_body<1>(d, qs, x, y, rows, cols, 1);
}

/* Variante por lotes (prefill): cada warp procesa R filas a la vez y **reutiliza** cada trozo de `x` leído de shared
 * para las R filas. Con una fila por warp, las lecturas de shared de `x` (una LDS.128 por cada 4 FMA) saturaban
 * el pipe de shared tres veces antes de llenar el de FMA; con R = 4 queda por debajo. Más registros por hilo
 * (acumuladores R x NT) a cambio de menos lecturas. Las filas más allá de `rows` no escriben. */
#define Q40R_ROWS 4

template <int NT>
__device__ __forceinline__ void q40_body_filas(const unsigned short *d, const uint4 *qs, const float *x, float *y,
                                               int rows, int cols, int n)
{
    __shared__ __align__(16) float xs[NT][32 * Q40_PAD];
    int warp = (int)threadIdx.x >> 5;
    int lane = (int)threadIdx.x & 31;
    int row0 = ((int)blockIdx.x * Q40_WARPS + warp) * Q40R_ROWS;
    int nb = cols >> 5;
    float acc[Q40R_ROWS][NT];
#pragma unroll
    for (int r = 0; r < Q40R_ROWS; r++) {
#pragma unroll
        for (int t = 0; t < NT; t++) {
            acc[r][t] = 0.0f;
        }
    }
    for (int c0 = 0; c0 < nb; c0 += 32) {
        int nblk = nb - c0 < 32 ? nb - c0 : 32;
        __syncthreads();
        for (int i = (int)threadIdx.x; i < NT * nblk * 8; i += (int)blockDim.x) {
            int t = i / (nblk * 8);
            int rr = i - t * nblk * 8;
            int blk = rr >> 3;
            int q = rr & 7;
            if (t < n) {
                float4 v = ((const float4 *)(x + (long)t * (long)cols + (long)(c0 + blk) * 32))[q];
                *(float4 *)&xs[t][blk * Q40_PAD + q * 4] = v;
            }
        }
        __syncthreads();
        if (lane < nblk) {
            int b = c0 + lane;
            unsigned w[Q40R_ROWS][4];
            float sc[Q40R_ROWS];
#pragma unroll
            for (int r = 0; r < Q40R_ROWS; r++) {
                if (row0 + r < rows) {
                    long o = (long)(row0 + r) * (long)nb + b;
                    sc[r] = __half2float(__ushort_as_half(d[o]));
                    uint4 q = qs[o];
                    w[r][0] = q.x;
                    w[r][1] = q.y;
                    w[r][2] = q.z;
                    w[r][3] = q.w;
                } else {
                    sc[r] = 0.0f;
                    w[r][0] = w[r][1] = w[r][2] = w[r][3] = 0x88888888u; /* nibble 8 -> valor 0 */
                }
            }
            float s[Q40R_ROWS][NT];
#pragma unroll
            for (int r = 0; r < Q40R_ROWS; r++) {
#pragma unroll
                for (int t = 0; t < NT; t++) {
                    s[r][t] = 0.0f;
                }
            }
#pragma unroll
            for (int j = 0; j < 4; j++) {
                float vlo[Q40R_ROWS][4];
                float vhi[Q40R_ROWS][4];
#pragma unroll
                for (int r = 0; r < Q40R_ROWS; r++) {
#pragma unroll
                    for (int k = 0; k < 4; k++) {
                        unsigned byte = (w[r][j] >> (8 * k)) & 0xffu;
                        vlo[r][k] = (float)((int)(byte & 0xfu) - 8);
                        vhi[r][k] = (float)((int)(byte >> 4) - 8);
                    }
                }
#pragma unroll
                for (int t = 0; t < NT; t++) {
                    if (t < n) {
                        const float4 *xp = (const float4 *)&xs[t][lane * Q40_PAD];
                        float4 lo = xp[j];
                        float4 hi = xp[4 + j];
#pragma unroll
                        for (int r = 0; r < Q40R_ROWS; r++) {
                            s[r][t] += vlo[r][0] * lo.x + vlo[r][1] * lo.y + vlo[r][2] * lo.z + vlo[r][3] * lo.w;
                            s[r][t] += vhi[r][0] * hi.x + vhi[r][1] * hi.y + vhi[r][2] * hi.z + vhi[r][3] * hi.w;
                        }
                    }
                }
            }
#pragma unroll
            for (int r = 0; r < Q40R_ROWS; r++) {
#pragma unroll
                for (int t = 0; t < NT; t++) {
                    acc[r][t] += sc[r] * s[r][t];
                }
            }
        }
    }
#pragma unroll
    for (int r = 0; r < Q40R_ROWS; r++) {
#pragma unroll
        for (int t = 0; t < NT; t++) {
            if (t < n) {
                float v = q40_warp_sum(acc[r][t]);
                if (lane == 0 && row0 + r < rows) {
                    y[(long)t * (long)rows + row0 + r] = v;
                }
            }
        }
    }
}

extern "C" __global__ void matmul_q4_0r8(const unsigned short *d, const uint4 *qs, const float *x, float *y,
                                         int rows, int cols, int n)
{
    q40_body_filas<8>(d, qs, x, y, rows, cols, n);
}
