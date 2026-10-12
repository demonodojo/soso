/* Atención decode de UN head: KV en f16 residente en VRAM, salida f32.
 *
 * Softmax online (flash-style) escalar en el lane 0 del warp; seq <= ATTN_MAX_SEQ.
 * Es la referencia funcional del port a GPU de `attention_decode_kv`; el reparto
 * por lanes y los tensor cores llegan en una iteración posterior. */
#define ATTN_MAX_SEQ 512

extern "C" __device__ float attn_f16_to_f32(unsigned short bits)
{
    unsigned int sign = (unsigned int)((bits >> 15) & 1u);
    unsigned int exp = (unsigned int)((bits >> 10) & 0x1fu);
    unsigned int frac = (unsigned int)(bits & 0x3ffu);
    union {
        unsigned int u;
        float f;
    } c;
    if (exp == 0u) {
        if (frac == 0u) {
            c.u = sign << 31;
            return c.f;
        }
        float v = ((float)frac / 1024.0f) * 6.103515625e-5f;
        return sign != 0u ? -v : v;
    }
    if (exp == 31u) {
        c.u = (sign << 31) | 0x7f800000u;
        return c.f;
    }
    c.u = (sign << 31) | ((exp + 112u) << 23) | (frac << 13);
    return c.f;
}

extern "C" __global__ void attn_decode_f16(const float *q,
                                           const unsigned short *k_f16,
                                           const unsigned short *v_f16,
                                           float *out, int head_dim,
                                           int kv_dim, int kv_head, int seq)
{
    /* Un solo hilo (lane 0) hace la fila entera: correcto y simple. */
    if (threadIdx.x != 0 || blockIdx.x != 0) {
        return;
    }
    if (seq <= 0 || head_dim <= 0 || seq > ATTN_MAX_SEQ) {
        return;
    }

    float inv = rsqrtf((float)head_dim);
    float scores[ATTN_MAX_SEQ];
    float m0 = -1.0e30f;
    int t;
    int d;

    for (t = 0; t < seq; t++) {
        float dot = 0.0f;
        for (d = 0; d < head_dim; d++) {
            int k_off = t * kv_dim + kv_head * head_dim + d;
            dot += q[d] * attn_f16_to_f32(k_f16[k_off]);
        }
        scores[t] = dot * inv;
        if (scores[t] > m0) {
            m0 = scores[t];
        }
    }

    float sum = 0.0f;
    for (t = 0; t < seq; t++) {
        scores[t] = expf(scores[t] - m0);
        sum += scores[t];
    }
    float inv_sum = sum > 0.0f ? 1.0f / sum : 0.0f;

    for (d = 0; d < head_dim; d++) {
        float o = 0.0f;
        for (t = 0; t < seq; t++) {
            int v_off = t * kv_dim + kv_head * head_dim + d;
            o += scores[t] * attn_f16_to_f32(v_f16[v_off]);
        }
        out[d] = o * inv_sum;
    }
}
