/* Softmax fila a fila: x[row*cols .. (row+1)*cols]. */
extern "C" __device__ float warp_reduce_max(float v)
{
    unsigned mask = 0xffffffffu;
    v = fmaxf(v, __shfl_down_sync(mask, v, 16));
    v = fmaxf(v, __shfl_down_sync(mask, v, 8));
    v = fmaxf(v, __shfl_down_sync(mask, v, 4));
    v = fmaxf(v, __shfl_down_sync(mask, v, 2));
    v = fmaxf(v, __shfl_down_sync(mask, v, 1));
    return v;
}

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

extern "C" __global__ void softmax_rows_f32(float *x, int rows, int cols)
{
    int row = (int)blockIdx.x;
    if (row >= rows) {
        return;
    }
    float *r = x + (long)row * (long)cols;
    int lane = threadIdx.x & 31;
    int warp_id = threadIdx.x >> 5;
    if (warp_id > 0) {
        return;
    }

    float maxv = -1e30f;
    int c;
    for (c = lane; c < cols; c += 32) {
        maxv = fmaxf(maxv, r[c]);
    }
    maxv = warp_reduce_max(maxv);

    float sum = 0.0f;
    for (c = lane; c < cols; c += 32) {
        float e = __expf(r[c] - maxv);
        r[c] = e;
        sum += e;
    }
    sum = warp_reduce_sum(sum);

    if (sum > 0.0f) {
        float inv = 1.0f / sum;
        for (c = lane; c < cols; c += 32) {
            r[c] *= inv;
        }
    }
}
