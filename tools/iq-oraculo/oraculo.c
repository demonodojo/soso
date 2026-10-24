// Oráculo de dequantización: usa libggml-base (llama.cpp) para convertir bloques cuantizados a f32.
// Uso: oraculo TIPO_GGML entrada.bin salida.f32 N_BLOQUES
// Sirve para generar los fixtures contra los que se validan las dequantizaciones propias en Rust (crates/soso-llm-core/tests/iq_oraculo.rs).
#include <stdio.h>
#include <stdlib.h>
#include "ggml.h"
int main(int argc, char **argv) {
    if (argc != 5) { fprintf(stderr, "uso: oraculo TIPO entrada.bin salida.f32 N_BLOQUES\n"); return 2; }
    enum ggml_type t = (enum ggml_type) atoi(argv[1]);
    const struct ggml_type_traits *tr = ggml_get_type_traits(t);
    if (!tr || !tr->to_float) { fprintf(stderr, "tipo %d sin to_float\n", (int) t); return 3; }
    long nb = atol(argv[4]);
    size_t bytes = (size_t) nb * tr->type_size;
    void *in = malloc(bytes);
    FILE *f = fopen(argv[2], "rb");
    if (!f || fread(in, 1, bytes, f) != bytes) { fprintf(stderr, "entrada corta\n"); return 4; }
    fclose(f);
    long n = nb * (long) tr->blck_size;
    float *out = malloc((size_t) n * sizeof(float));
    tr->to_float(in, out, n);
    f = fopen(argv[3], "wb");
    fwrite(out, sizeof(float), (size_t) n, f);
    fclose(f);
    printf("%s: %ld bloques de %d elementos (%zu bytes) -> %ld f32\n", tr->type_name, nb, (int) tr->blck_size, tr->type_size, n);
    return 0;
}
