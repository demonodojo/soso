#ifndef GSP_CPU_SEQ_H
#define GSP_CPU_SEQ_H

#include <stdint.h>

/* Contexto para CORE_RESUME (`r535_gsp_msg_run_cpu_sequencer`). */
struct gsp_cpu_seq_ctx {
    uint64_t libos_phys;
    uint32_t app_version;
};

void gsp_cpu_seq_set_ctx(const struct gsp_cpu_seq_ctx *ctx);

/* Ejecuta `GSP_RUN_CPU_SEQUENCER` (`r535_gsp_msg_run_cpu_sequencer`). */
int gsp_cpu_seq_run(const void *payload, uint32_t len);

#endif
