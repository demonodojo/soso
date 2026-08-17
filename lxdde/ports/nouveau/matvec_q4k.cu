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

extern "C" __global__ void matvec_q4k(const unsigned char *w, const float *x,
                                      float *y, int rows, int cols)
{
    int tid = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    int warp_id = tid >> 5;
    int lane = tid & 31;
    int nsub;
    long row_bytes;
    const unsigned char *row;
    float sum = 0.0f;
    int k;

    if (warp_id >= rows) {
        return;
    }
    /* Sub-bloques de 32 elementos que tiene una fila, y bytes que ocupa. */
    nsub = cols >> 5;
    row_bytes = (long)(cols / Q4K_BLOCK_ELEMS) * (long)Q4K_BLOCK_BYTES;
    row = w + (long)warp_id * row_bytes;

    for (k = lane; k < nsub; k += 32) {
        int blk = k >> 3;  /* superbloque */
        int j = k & 7;     /* sub-bloque dentro de él */
        sum += q4k_dot_sub(row + (long)blk * Q4K_BLOCK_BYTES, j, x + (long)k * 32);
    }

    sum = q4k_warp_reduce_sum(sum);
    if (lane == 0) {
        y[warp_id] = sum;
    }
}
