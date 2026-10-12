/* G5/G6: matvec f32 (y = W·x) para GB205 (sm_120). Compilar con
 * scripts/l6-g4f-build-sass.sh, igual que saxpy.cu.
 *
 * G6: un warp por fila — los 32 hilos reparten las columnas y reducen con
 * shuffle. Misma firma que la versión lenta de depuración (un hilo/fila).
 *
 * `rows` en el kernel es la altura de la TANDA en G5 o la matriz entera en G6
 * residente. `w` e `y` apuntan al inicio de la región activa. */
extern "C" __device__ float warp_reduce_sum(float v)
{
    unsigned mask = 0xffffffffu;
    v += __shfl_down_sync(mask, v, 16);
    v += __shfl_down_sync(mask, v, 8);
    v += __shfl_down_sync(mask, v, 4);
    v += __shfl_down_sync(mask, v, 2);
    v += __shfl_down_sync(mask, v, 1);
    return v;
}

extern "C" __global__ void matvec_f32(const float *w, const float *x, float *y,
                                      int rows, int cols)
{
    int tid = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    int warp_id = tid >> 5;
    int lane = tid & 31;

    if (warp_id >= rows) {
        return;
    }

    const float *row = w + (long)warp_id * (long)cols;
    float sum = 0.0f;
    int c;

    /* Un float4 es un LDG.E.128 y exige 16 B. Con `c = lane` el hilo 1 leía en
     * row+4 B: la GA104 lo paró con GR_EXCEPTION en una VA acabada en 0x…d04
     * (ROG, 2026-10-12), y además cada columna se sumaba hasta cuatro veces. */
    if ((cols & 3) == 0) {
        for (c = lane * 4; c < cols; c += 128) {
            float4 w4 = *(const float4 *)(row + c);
            float4 x4 = *(const float4 *)(x + c);
            sum += w4.x * x4.x + w4.y * x4.y + w4.z * x4.z + w4.w * x4.w;
        }
    } else {
        for (c = lane; c < cols; c += 32) {
            sum += row[c] * x[c];
        }
    }

    sum = warp_reduce_sum(sum);
    if (lane == 0) {
        y[warp_id] = sum;
    }
}
