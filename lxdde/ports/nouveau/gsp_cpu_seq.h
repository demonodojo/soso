#ifndef GSP_CPU_SEQ_H
#define GSP_CPU_SEQ_H

#include <stdint.h>

/* Ejecuta `GSP_RUN_CPU_SEQUENCER` (`r535_gsp_msg_run_cpu_sequencer`). */
int gsp_cpu_seq_run(const void *payload, uint32_t len);

#endif
