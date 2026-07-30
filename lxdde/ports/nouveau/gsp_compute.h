/* G4f/G5: lanzamiento compute (QMD en sysmem + SEND_PCAS). Ver gsp_compute.h. */
#ifndef GSP_COMPUTE_H
#define GSP_COMPUTE_H

#include "gsp_ce.h"
#include "gsp_chan.h"

/* Derivadas de `GSP_VA_BASE` (gsp_vmm.h). Estaban escritas enteras con la base
 * repetida, y cuando la base tuvo que bajar de 1 TiB a 512 GiB por el límite de
 * 40 bits del GPFIFO, tres de estas cinco se habrían quedado apuntando a la
 * antigua sin que nada se quejara. */
#define G4F_SASS_VA   (GSP_VA_BASE + 0x1000ull)      /* VRAM: blob SASS de saxpy */
#define G5_SASS_VA    (GSP_VA_BASE + 0x2000ull)      /* VRAM: blob SASS de matvec */
/* QMD v05 (384 B) dentro de `cp->data`; SEND_PCAS_A exige alineación >>8. */
#define G4F_QMD_OFF   0x1000u

/* Sysmem coherente propia del compute, detrás de los búferes del canal. La CPU
 * escribe aquí directamente: sin BAR1 no hay ventana a la VRAM, así que el
 * constant bank y los vectores viven en sysmem y se leen por la VA space. */
#define G4F_DATA_VA       (GSP_VA_BASE + 0x30000000ull)
#define G4F_DATA_SIZE     8192u
#define G4F_CBANK_OFF     0u        /* alineado a 64 B: lo exige ADDR_SHIFTED6 */
#define G4F_X_OFF         0x400u
#define G4F_Y_OFF         0x800u
#define G4F_SEM_OFF       0xc00u
#define G4F_MAX_N         256u      /* 1 KiB por vector */

#define G4F_CTA_THREADS   256u
#define G6_ROWS_PER_CTA   (G4F_CTA_THREADS / 32u)
#define G4F_SEM_PAYLOAD   0x5a5a0001u
#define G4F_WAIT_MS       2000u
/* Sondeos a pelo antes de empezar a dormir de milisegundo en milisegundo. */
#define G4F_SPIN_TRIES    20000u

/* G5: staging de matvec. Su propia reserva y su propia ventana de VAs (2 MiB por
 * delante de la de G4f, alineada a 2 MiB para que quepa en una sola tabla hoja).
 *
 * Una matriz de verdad no cabe aquí y no se pretende: las filas van por TANDAS
 * de `gsp_compute_mv_rows_per_tile(cols)` y el kernel se lanza una vez por tanda.
 * Este búfer es la ventana por la que pasa la matriz, no la matriz. */
#define G5_MV_VA          (GSP_VA_BASE + 0x30200000ull)
#define G5_MV_SIZE        0x80000u   /* 512 KiB = 128 páginas contiguas */
#define G5_MV_W_OFF       0x00000u
#define G5_MV_W_BYTES     0x50000u   /* 320 KiB de filas */
#define G5_MV_X_OFF       0x50000u
/* 128 KiB → 32768 columnas. El tamaño de `x` es el que fija la MATRIZ MÁS ANCHA
 * que puede pasar por aquí, y ahí no vale quedarse corto: con 16 KiB el tope
 * eran 4096 columnas y el FFN de un 70B (28672) se habría ido siempre a CPU sin
 * más explicación que un `on_gpu=0`. Las filas se trocean; las columnas no —el
 * kernel escribe `y[r] = sum` de una fila entera, no acumula—, así que este
 * número es un límite duro y no una ventana. */
#define G5_MV_X_BYTES     0x20000u
#define G5_MV_Y_OFF       0x70000u
#define G5_MV_Y_BYTES     0x01000u   /* 4 KiB → 1024 filas por tanda */
#define G5_MAX_COLS       (G5_MV_X_BYTES / 4u)
#define G5_MAX_TILE_ROWS  (G5_MV_Y_BYTES / 4u)

/* G6: staging sysmem para matvec residente (x + y completos, sin trocear W). */
#define G6_RES_VA         (GSP_VA_BASE + 0x30300000ull)
#define G6_RES_SIZE       0x40000u   /* 256 KiB */
#define G6_RES_X_OFF      0x00000u
#define G6_RES_X_BYTES    0x20000u   /* 128 KiB → G5_MAX_COLS columnas */
#define G6_RES_Y_OFF      0x20000u
#define G6_RES_Y_BYTES    0x20000u   /* 128 KiB → 32768 filas */
#define G6_MAX_ROWS       (G6_RES_Y_BYTES / 4u)

/* Lo que el cubin dice de un kernel, junto. Lo rellena `gsp_compute_init` desde
 * los símbolos generados; nadie lo escribe a mano. */
struct gsp_kernel {
    const char *name;
    const unsigned char *sass;
    unsigned sass_len;
    unsigned regcount;      /* EIATTR_REGCOUNT */
    unsigned param_base;    /* EIATTR_PARAM_CBANK: offset en cbank0 */
    unsigned param_size;    /* EIATTR_CBANK_PARAM_SIZE */
    unsigned cbank_size;    /* tamaño de .nv.constant0 */
    const unsigned *param_off;
    unsigned param_count;
    uint64_t sass_va;       /* dónde se stagea en VRAM */
};

struct gsp_compute {
    struct gsp_rm *rm;
    /* Canal de GR0: un objeto de compute NO se puede colgar del canal del CE
     * (ver NV2080_ENGINE_TYPE_GR0 en nvrm_r570.h). */
    struct gsp_chan *chan;
    uint32_t handle;
    uint32_t cls;      /* la que aceptó RM; la elige el catálogo del chip */
    struct gsp_kernel saxpy;
    struct gsp_kernel matvec;
    struct gsp_dma_buf data;   /* cbank0 + x + y + semáforo, en sysmem */
    uint64_t data_va;
    struct gsp_dma_buf mv;     /* G5: tanda de filas + x + y, en sysmem */
    uint64_t mv_va;
    struct gsp_dma_buf res;    /* G6: x + y completos para matvec residente */
    uint64_t res_va;
    int mapped;
    int mv_mapped;
    int res_mapped;
    int ready;
};

/* Blob SASS y los metadatos que el cubin declara sobre él (saxpy: a, x, y, n). */
extern const unsigned char gsp_saxpy_sass[];
extern const unsigned gsp_saxpy_sass_len;
extern const unsigned gsp_saxpy_regcount;
extern const unsigned gsp_saxpy_param_base;
extern const unsigned gsp_saxpy_param_size;
extern const unsigned gsp_saxpy_cbank_size;
extern const unsigned gsp_saxpy_param_off[4];
extern const unsigned gsp_saxpy_param_count;

/* Ídem para matvec (w, x, y, rows, cols). El `param_count` generado se compara
 * con estos 5: un .cu con un parámetro más y este header sin tocar es un
 * lanzamiento que lee basura y no da ningún error. */
extern const unsigned char gsp_matvec_sass[];
extern const unsigned gsp_matvec_sass_len;
extern const unsigned gsp_matvec_regcount;
extern const unsigned gsp_matvec_param_base;
extern const unsigned gsp_matvec_param_size;
extern const unsigned gsp_matvec_cbank_size;
extern const unsigned gsp_matvec_param_off[5];
extern const unsigned gsp_matvec_param_count;

int gsp_compute_init(struct gsp_rm *rm, struct gsp_chan *chan,
                     struct gsp_compute *cp);

/* Copia el SASS de `k` a VRAM vía CE y espera al semáforo. */
int gsp_compute_stage_sass(struct gsp_compute *cp, struct gsp_ce *ce,
                           const struct gsp_kernel *k,
                           uint64_t scratch_va, void *scratch_cpu);

/* Codifica SET_OBJECT + WFI + SEND_PCAS en el pushbuffer; el QMD va a sysmem. */
int gsp_compute_encode_qmd(struct gsp_compute *cp, const GspQmdV05 *qmd,
                           unsigned *pb_off, unsigned *pb_len);

/* Rellena el QMD para `k`: programa, malla, registros y constant bank. */
void gsp_compute_fill_qmd(struct gsp_compute *cp, const struct gsp_kernel *k,
                          GspQmdV05 *qmd, unsigned grid_x);

/* Escribe los parámetros de saxpy en el constant bank. */
void gsp_compute_set_params(struct gsp_compute *cp, float a, uint64_t x_va,
                            uint64_t y_va, unsigned n);

/* Ídem para matvec. `rows` es la altura de la TANDA, no de la matriz. */
void gsp_compute_set_mv_params(struct gsp_compute *cp, uint64_t w_va,
                               uint64_t x_va, uint64_t y_va, unsigned rows,
                               unsigned cols);

/* Filas que caben en una tanda con `cols` columnas; 0 si `cols` no cabe. Es una
 * función pura a propósito: es la aritmética que decide qué se copia dónde, y sin
 * poder comprobarla sin GPU el único síntoma de un error sería un readback
 * silenciosamente incompleto. */
unsigned gsp_compute_mv_rows_per_tile(unsigned cols);

/* Copia la tanda [row0, row0+rows) de `w` y el vector `x` al staging, y pone el
 * semáforo a cero. Separado del lanzamiento para poder comprobar los offsets. */
void gsp_compute_mv_stage(struct gsp_compute *cp, const float *w, const float *x,
                          unsigned rows, unsigned cols, unsigned row0);

/* Devuelve la tanda desde el staging a `y[row0..row0+rows)`. */
void gsp_compute_mv_read(struct gsp_compute *cp, float *y, unsigned rows,
                         unsigned row0);

/* Lanza saxpy de verdad; devuelve 0 solo con el semáforo del QMD señalizado y
 * el resultado releído de memoria que escribió la GPU. */
int gsp_compute_saxpy(struct gsp_compute *cp, struct gsp_ce *ce,
                      float a, const float *x, float *y, unsigned n,
                      uint64_t scratch_va, void *scratch_cpu);

/* G5: y = W·x en la GPU, por tandas de filas. Devuelve 0 solo si TODAS las
 * tandas señalizaron: un 0 con una tanda a medias sería un vector con ceros
 * dentro y nadie lo notaría hasta ver salir texto raro del modelo. */
int gsp_compute_matvec_f32(struct gsp_compute *cp, struct gsp_ce *ce,
                           const float *w, unsigned rows, unsigned cols,
                           const float *x, float *y,
                           uint64_t scratch_va, void *scratch_cpu);

/* G6: y = W·x con W ya residente en VRAM (VA del dispositivo). Un solo QMD. */
int gsp_compute_matvec_resident(struct gsp_compute *cp, struct gsp_ce *ce,
                                uint64_t w_va, unsigned rows, unsigned cols,
                                const float *x, float *y,
                                uint64_t scratch_va, void *scratch_cpu);

void gsp_compute_fini(struct gsp_compute *cp);

#endif
