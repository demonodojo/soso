/* Host: punteros ALIVE y volcado de tablas de error del firmware (Linux dump.c). */
#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <stdarg.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "iwl_test_frames.h"

#ifndef IWL_HOSTCHECK_SRAM_BYTES
#define IWL_HOSTCHECK_SRAM_BYTES (64u * 1024u)
#endif

uint8_t iwl_hostcheck_sram[IWL_HOSTCHECK_SRAM_BYTES];

static uint32_t g_mmio_stub[IWL_TEST_MMIO_WORDS];
static char g_log[4096];
static int g_fails;

static void check(int ok, const char *label)
{
    printf("%s: %s\n", ok ? "PASS" : "FAIL", label);
    if (!ok)
        g_fails++;
}

void lx_printk(const char *fmt, ...)
{
    va_list ap;
    size_t n = strlen(g_log);

    va_start(ap, fmt);
    vsnprintf(g_log + n, sizeof(g_log) - n, fmt, ap);
    va_end(ap);
}

void lx_mdelay(unsigned int ms) { (void)ms; }
void lx_udelay(unsigned int us) { (void)us; }

void *lx_dma_alloc_coherent(void *dev, size_t size, uint64_t *dma, unsigned gfp)
{
    (void)dev;
    (void)gfp;
    void *p = aligned_alloc(4096, (size + 4095u) & ~(size_t)4095u);
    if (p && dma)
        *dma = (uint64_t)(uintptr_t)p;
    return p;
}

void lx_dma_free_coherent(void *dev, size_t size, void *cpu, uint64_t dma)
{
    (void)dev;
    (void)size;
    (void)dma;
    free(cpu);
}

int iwl_fw_cmd_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    (void)iwl;
    (void)group;
    (void)cmd;
    return 0;
}

void iwl_ax211_deliver_rx(const uint8_t *data, int len)
{
    (void)data;
    (void)len;
}

void iwl_ax211_deliver_eapol(const uint8_t *data, int len)
{
    (void)data;
    (void)len;
}

void lx_iwlwifi_set_alive(int alive) { (void)alive; }
void iwl_ax211_add_bss(const struct iwl_ax211_bss *bss) { (void)bss; }

void iwl_mvm_rx_scan_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    (void)iwl;
    (void)frame;
    (void)len;
}

void iwl_mvm_rx_mlme_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    (void)iwl;
    (void)frame;
    (void)len;
}

void iwl_mvm_on_scan_complete(struct iwl_ax211_priv *iwl, uint32_t uid, uint8_t status)
{
    (void)iwl;
    (void)uid;
    (void)status;
}

int iwl_mvm_rx_to_eth(const struct iwl_ax211_priv *iwl, const uint8_t *frame, int flen,
                      uint32_t rx_status, uint8_t *out, int out_cap)
{
    (void)iwl;
    (void)frame;
    (void)flen;
    (void)rx_status;
    (void)out;
    (void)out_cap;
    return -1;
}

void iwl_mvm_rx_missed_beacons(struct iwl_ax211_priv *iwl, const uint8_t *data, int len)
{
    (void)iwl;
    (void)data;
    (void)len;
}

int iwl_fw_upload_sections(struct iwl_ax211_priv *iwl, struct iwl_context_info_dram *dram)
{
    (void)iwl;
    (void)dram;
    return 0;
}

#include "iwl_fw_pnvm_weak.c"
#include "iwl_trans.c"

static void put_le32(uint32_t addr, uint32_t v)
{
    iwl_hostcheck_sram[addr] = (uint8_t)(v & 0xffu);
    iwl_hostcheck_sram[addr + 1] = (uint8_t)((v >> 8) & 0xffu);
    iwl_hostcheck_sram[addr + 2] = (uint8_t)((v >> 16) & 0xffu);
    iwl_hostcheck_sram[addr + 3] = (uint8_t)((v >> 24) & 0xffu);
}

static void test_alive_ptrs(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t alive[64];

    memset(&iwl, 0, sizeof(iwl));
    memset(alive, 0, sizeof(alive));
    alive[4 + 16] = 0x00;
    alive[4 + 17] = 0x01;
    alive[4 + 18] = 0x00;
    alive[4 + 19] = 0x00;
    alive[4 + 44 + 8] = 0x80;
    alive[4 + 44 + 9] = 0x00;
    alive[4 + 44 + 10] = 0x00;
    alive[4 + 44 + 11] = 0x00;
    /* pay = 1×LMAC + UMAC (no 2×LMAC, que desplaza umac_off). */
    iwl_trans_alive_store_dbg_ptrs(&iwl, alive, 4 + 44 + 16);
    check(iwl.dbg_lmac_err_ptr[0] == 0x100u, "punteros ALIVE v5: lmac error table");
    check(iwl.dbg_umac_err_ptr == 0x80u, "punteros ALIVE v5: umac error table");
}

static void test_dump_lmac(void)
{
    struct iwl_ax211_priv iwl;
    const uint32_t lmac_base = 0x2000u;

    memset(&iwl, 0, sizeof(iwl));
    memset(g_log, 0, sizeof(g_log));
    iwl.mmio = g_mmio_stub;
    iwl.dbg_lmac_err_ptr[0] = lmac_base;
    put_le32(lmac_base, 1u);              /* valid */
    put_le32(lmac_base + 4, 0x70u);       /* FW_ASSERT_LMAC_FATAL */
    put_le32(lmac_base + 80, 0xdeadbeefu); /* log_pc */
    put_le32(lmac_base + 144, 42u);       /* u_timestamp / line */
    iwl_trans_dump_fw_errors(&iwl);
    check(strstr(g_log, "error fw lmac0 id=0x70 (FW_ASSERT_LMAC_FATAL)") != NULL,
          "volcado lmac: error_id y nombre");
    check(strstr(g_log, "pc=0xdeadbeef") != NULL, "volcado lmac: pc");
    check(strstr(g_log, "line=0x2a") != NULL, "volcado lmac: line");
}

int main(void)
{
    test_alive_ptrs();
    test_dump_lmac();
    puts(g_fails ? "fw_errlog: FALLOS" : "fw_errlog OK (tabla sintética, no silicio)");
    return g_fails ? 1 : 0;
}
