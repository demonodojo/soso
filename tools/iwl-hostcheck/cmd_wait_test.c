/* Prueba host: send_cmd_wait exige cmd_status (mock sin hardware). */
#include <stdio.h>
#include <stdint.h>

#include "iwl_internal.h"
#include "iwl_ax211.h"

extern struct iwl_ax211_priv g_test;
extern int g_poll_calls;
extern int g_fast_ack;

int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms);

int cmd_wait_hostcheck(void)
{
    g_test.alive = 1;
    g_test.mtr_cpu = (void *)1;
    g_test.mcr_cpu = (void *)1;

    g_fast_ack = 0;
    g_poll_calls = 0;
    g_test.cmd_status = 0;
    if (iwl_trans_send_cmd_wait(&g_test, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, 0, 0, 1) == 0) {
        fprintf(stderr, "send_cmd_wait no debe tener éxito sin ack\n");
        return -1;
    }

    g_poll_calls = 0;
    g_test.cmd_status = 0;
    if (iwl_trans_send_cmd_wait(&g_test, SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD, 0, 0, 10) != 0) {
        fprintf(stderr, "send_cmd_wait debe tener éxito tras ack simulado\n");
        return -1;
    }

    puts("OK: send_cmd_wait ack/timeout");
    return 0;
}
