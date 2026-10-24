// Volcado de tensores intermedios de llama.cpp (cb_eval) para comparar el forward de soso capa a capa.
// Uso: volcar_capas MODELO.gguf id,id,... DIR_SALIDA   → DIR/<nombre>.f32 con la última posición de cada tensor [n_embd…].
#include "llama.h"
#include "ggml-backend.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *dir;
static bool cb(struct ggml_tensor *t, bool ask, void *ud) {
    (void)ud;
    if (ask) return true;
    if (t->type != GGML_TYPE_F32) return true;
    size_t n = ggml_nelements(t);
    float *buf = malloc(n * 4);
    ggml_backend_tensor_get(t, buf, 0, n * 4);
    char p[512];
    snprintf(p, sizeof p, "%s/%s.f32", dir, t->name);
    for (char *c = p + strlen(dir) + 1; *c; c++) if (*c == '/') *c = '_';
    FILE *f = fopen(p, "wb");
    // tensor completo (todas las posiciones); el comparador escoge
    fwrite(buf, 4, n, f);
    fclose(f);
    fprintf(stderr, "%s ne=[%lld,%lld,%lld,%lld]\n", t->name, (long long)t->ne[0], (long long)t->ne[1], (long long)t->ne[2], (long long)t->ne[3]);
    free(buf);
    return true;
}

int main(int argc, char **argv) {
    if (argc < 4) { fprintf(stderr, "uso: volcar_capas modelo.gguf ids dir\n"); return 2; }
    dir = argv[3];
    llama_backend_init();
    struct llama_model_params mp = llama_model_default_params();
    mp.n_gpu_layers = 0;
    struct llama_model *m = llama_model_load_from_file(argv[1], mp);
    if (!m) return 1;
    struct llama_context_params cp = llama_context_default_params();
    cp.n_ctx = 512; cp.n_batch = 512; cp.cb_eval = cb;
    struct llama_context *c = llama_init_from_model(m, cp);
    llama_token ids[256]; int n = 0;
    for (char *s = strtok(argv[2], ","); s; s = strtok(NULL, ",")) ids[n++] = atoi(s);
    struct llama_batch b = llama_batch_get_one(ids, n);
    if (llama_decode(c, b)) { fprintf(stderr, "decode falló\n"); return 1; }
    return 0;
}
