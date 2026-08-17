/* G7: matvec con la matriz Q8_0 residente sin expandir, para GB205 (sm_120).
 *
 * Es el hermano simple de `matvec_q4k.cu` y va primero a propósito: un bloque Q8_0
 * es una escala f32 y 32 int8, sin escalas de 6 bits ni nibbles cruzados, así que si
 * el silicio falla con éste el problema es del lanzamiento (QMD, parámetros, VAs) y
 * no de la decodificación. Con Q4_K los dos se confunden.
 *
 * Un warp por fila, los lanes reparten bloques de 32 elementos, reducción con
 * shuffle. `cols` múltiplo de 32, comprobado por el driver.
 */
#include "q4k_decode.h"

extern "C" __device__ float q80_warp_reduce_sum(float v)
{
    unsigned mask = 0xffffffffu;
    v += __shfl_down_sync(mask, v, 16);
    v += __shfl_down_sync(mask, v, 8);
    v += __shfl_down_sync(mask, v, 4);
    v += __shfl_down_sync(mask, v, 2);
    v += __shfl_down_sync(mask, v, 1);
    return v;
}

extern "C" __global__ void matvec_q80(const unsigned char *w, const float *x,
                                      float *y, int rows, int cols)
{
    int tid = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    int warp_id = tid >> 5;
    int lane = tid & 31;
    int nb;
    const unsigned char *row;
    float sum = 0.0f;
    int b;

    if (warp_id >= rows) {
        return;
    }
    nb = cols / Q80_BLOCK_ELEMS;
    row = w + (long)warp_id * (long)nb * (long)Q80_BLOCK_BYTES;

    for (b = lane; b < nb; b += 32) {
        sum += q80_dot_block(row + (long)b * Q80_BLOCK_BYTES,
                             x + (long)b * Q80_BLOCK_ELEMS);
    }

    sum = q80_warp_reduce_sum(sum);
    if (lane == 0) {
        y[warp_id] = sum;
    }
}
