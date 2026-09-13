/* G7: matvec con la matriz Q4_K **residente sin expandir** (y = W·x) para GB205
 * (sm_120). Compilar con scripts/l6-g4f-build-sass.sh, igual que matvec.cu.
 *
 * POR QUÉ EXISTE: hasta aquí el offload subía siempre f32 descuantizado, así que un
 * peso Q4_K de TinyLlama (5632×2048) ocupaba 44 MiB en VRAM y 44 MiB de PCIe por
 * los 5,9 MiB que hay en disco. Con este kernel lo que vive en la tarjeta son los
 * bytes del shard tal cual: 8× menos tráfico al subir, 8× menos VRAM (y por tanto
 * 8× más tensores residentes) y 8× menos ancho de banda de VRAM por token, que en
 * decode es el recurso que manda.
 *
 * Un warp por fila; los lanes reparten SUB-bloques de 32 elementos (no
 * superbloques: una fila de FFN son 22 superbloques y sólo trabajarían 22 de los 32
 * lanes) y reducen con shuffle. La aritmética de decodificación está en
 * `q4k_decode.h`, compartida con el hostcheck para poder fijarla sin GPU.
 *
 * `cols` ha de ser múltiplo de 256 — una fila es un número entero de superbloques.
 * Lo comprueba el driver antes de lanzar: aquí no se puede.
 */
#include "q4k_decode.h"

extern "C" __device__ float q4k_warp_reduce_sum(float v)
{
    unsigned mask = 0xffffffffu;
    v += __shfl_down_sync(mask, v, 16);
    v += __shfl_down_sync(mask, v, 8);
    v += __shfl_down_sync(mask, v, 4);
    v += __shfl_down_sync(mask, v, 2);
    v += __shfl_down_sync(mask, v, 1);
    return v;
}

/* Techo de `x` cacheado en shared (16 KiB f32). Cubre hidden=4096 (Mistral): las
 * proyecciones q/k/v/o y gate/up leen `x` de tamaño hidden, así que 6 de los 7
 * matvec por capa lo aprovechan. `down` (cols=ffn) lo supera y relee de global,
 * como antes. Es la idea del `mmvq` de llama.cpp: la activación se lee una vez por
 * CTA y todos los warps (filas) la comparten desde shared en vez de releerla de
 * memoria global una vez por fila. Numéricamente idéntico: mismos floats. */
#define Q4K_X_SHARED_FLOATS 4096

extern "C" __global__ void matvec_q4k(const unsigned char *w, const float *x,
                                      float *y, int rows, int cols)
{
    __shared__ __align__(16) float xs[Q4K_X_SHARED_FLOATS];
    int tid = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    int warp_id = tid >> 5;
    int lane = tid & 31;
    int nsub;
    long row_bytes;
    const unsigned char *row;
    const float *xp = x;
    float sum = 0.0f;
    int k;

    /* `cols` es uniforme en el CTA, así que el barrier lo cruzan todos los hilos o
     * ninguno: no hay divergencia posible sobre `__syncthreads`. El guard de fila
     * va DESPUÉS del barrier — si un warp saliera antes, colgaría al resto. */
    if (cols <= Q4K_X_SHARED_FLOATS) {
        int i;
        for (i = (int)threadIdx.x; i < cols; i += (int)blockDim.x) {
            xs[i] = x[i];
        }
        __syncthreads();
        xp = xs;
    }

    if (warp_id >= rows) {
        return;
    }
    /* Sub-bloques de 32 elementos que tiene una fila, y bytes que ocupa. */
    nsub = cols >> 5;
    row_bytes = (long)(cols / Q4K_BLOCK_ELEMS) * (long)Q4K_BLOCK_BYTES;
    row = w + (long)warp_id * row_bytes;

    /* Fase 4: un sub-bloque por lane; dp4a en q4k_decode cuando el SM lo expone. */
    for (k = lane; k < nsub; k += 32) {
        int blk = k >> 3;  /* superbloque */
        int j = k & 7;     /* sub-bloque dentro de él */
#if __CUDA_ARCH__ >= 610
        sum += q4k_dot_sub_dp4a(row + (long)blk * Q4K_BLOCK_BYTES, j, xp + (long)k * 32);
#else
        sum += q4k_dot_sub(row + (long)blk * Q4K_BLOCK_BYTES, j, xp + (long)k * 32);
#endif
    }

    sum = q4k_warp_reduce_sum(sum);
    if (lane == 0) {
        y[warp_id] = sum;
    }
}
