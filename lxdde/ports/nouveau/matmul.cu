/* G6: matmul batched f32 — Y[n×rows] = X[n×cols] · W[rows×cols]^T.
 * Un warp por fila de salida; blockIdx.y = batch. */
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

extern "C" __global__ void matmul_xwt_f32(const float *w, const float *x, float *y,
                                          int rows, int cols, int n)
{
    int batch = (int)blockIdx.y;
    if (batch >= n) {
        return;
    }
    int tid = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    int warp_id = tid >> 5;
    int lane = tid & 31;
    if (warp_id >= rows) {
        return;
    }

    const float *row_w = w + (long)warp_id * (long)cols;
    const float *row_x = x + (long)batch * (long)cols;
    float sum = 0.0f;
    int c;

    /* Mismo bucle que matvec.cu: float4 sólo con bloques de 4 columnas
     * alineados (lane*4), o el LDG.E.128 cae desalineado. */
    if ((cols & 3) == 0) {
        for (c = lane * 4; c < cols; c += 128) {
            float4 w4 = *(const float4 *)(row_w + c);
            float4 x4 = *(const float4 *)(row_x + c);
            sum += w4.x * x4.x + w4.y * x4.y + w4.z * x4.z + w4.w * x4.w;
        }
    } else {
        for (c = lane; c < cols; c += 32) {
            sum += row_w[c] * row_x[c];
        }
    }

    sum = warp_reduce_sum(sum);
    if (lane == 0) {
        y[(long)batch * (long)rows + warp_id] = sum;
    }
}
