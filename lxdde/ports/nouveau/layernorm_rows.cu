/* LayerNorm fila a fila con peso y bias. */
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

extern "C" __global__ void layernorm_rows_f32(float *x, const float *weight,
                                              const float *bias, int rows,
                                              int cols, float eps)
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

    float mean = 0.0f;
    int c;
    for (c = lane; c < cols; c += 32) {
        mean += r[c];
    }
    mean = warp_reduce_sum(mean) / (float)cols;

    float var = 0.0f;
    for (c = lane; c < cols; c += 32) {
        float d = r[c] - mean;
        var += d * d;
    }
    var = warp_reduce_sum(var) / (float)cols;
    float scale = rsqrtf(var + eps);

    for (c = lane; c < cols; c += 32) {
        r[c] = (r[c] - mean) * scale * weight[c] + bias[c];
    }
}
