/* RoPE in-place sobre una fila (head_dim elementos). */
extern "C" __device__ float rope_theta(int pos, int i, float base)
{
    int half = i >> 1;
    float freq = powf(base, -((float)(half * 2) / (float)i));
    float ang = (float)pos * freq;
    return (i & 1) != 0 ? sinf(ang) : cosf(ang);
}

extern "C" __global__ void rope_row_f32(float *x, int dim, int pos, float base)
{
    int pair = (int)threadIdx.x;
    if (pair * 2 + 1 >= dim) {
        return;
    }
    float x0 = x[pair * 2];
    float x1 = x[pair * 2 + 1];
    int half = dim >> 1;
    float freq = powf(base, -((float)pair * 2.0f) / (float)dim);
    float ang = (float)pos * freq;
    float c = cosf(ang);
    float s = sinf(ang);
    x[pair * 2] = x0 * c - x1 * s;
    x[pair * 2 + 1] = x0 * s + x1 * c;
}
