/* R5: juegos de SASS identificados por arquitectura y familias de GPU.
 *
 * Autónomo a propósito (solo tipos de C): lo incluyen los ficheros generados
 * `sass_<tag>.c`, el port y el banco de pruebas del host, sin arrastrar la capa
 * lx ni las cabeceras de RM.
 *
 * Lo que hay detrás: aceptar una clase de compute en RM (por ejemplo `0xc7c0`
 * en Ampere) no demuestra que el descriptor QMD ni el código máquina del kernel
 * sirvan para ese chip. Cada familia tiene su versión de QMD y su arquitectura
 * de SASS, y las dos se comprueban antes de enviar nada.
 */
#ifndef GSP_SASS_H
#define GSP_SASS_H

#define GSP_FAM_DESCONOCIDA 0u
#define GSP_FAM_AMPERE      1u
#define GSP_FAM_ADA         2u
#define GSP_FAM_HOPPER      3u
#define GSP_FAM_BLACKWELL   4u

/* Un kernel compilado para una arquitectura, con lo que el cubin declara de
 * él. Nadie escribe estos números a mano: los emite el generador. */
struct gsp_sass_variant {
    const char *name;
    const unsigned char *sass;
    unsigned sass_len;
    unsigned regcount;      /* EIATTR_REGCOUNT */
    unsigned param_base;    /* EIATTR_PARAM_CBANK: offset en cbank0 */
    unsigned param_size;    /* EIATTR_CBANK_PARAM_SIZE */
    unsigned cbank_size;    /* tamaño de .nv.constant0 */
    const unsigned *param_off;
    unsigned param_count;
};

/* Todos los kernels compilados para una misma arquitectura. */
struct gsp_sass_set {
    const char *arch;       /* "sm_86", "sm_120"… */
    unsigned family;        /* GSP_FAM_* al que sirve */
    const struct gsp_sass_variant *vars;
    unsigned count;
};

/* Capacidades de una familia: qué clase de compute, qué QMD y qué SASS.
 *
 * `qmd_version` 0 significa «no sé escribir el descriptor de esta familia»:
 * se puede llegar a tener el objeto de compute y aun así no poder lanzar,
 * y eso hay que decirlo antes de enviar el QMD, no después. */
struct gsp_family_caps {
    unsigned family;
    const char *nombre;
    unsigned qmd_version;   /* 1 → QMDV01_07 (Ampere/Turing), 5 → QMDV05 */
    unsigned qmd_bytes;
    const char *sass_arch;
};

#endif
