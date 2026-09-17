/* Host: plan de carga FH 8000 (sin NIC) vs Linux trans.c:1029-1063. */
#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

void lx_printk(const char *fmt, ...) { (void)fmt; }
void lx_mdelay(unsigned int ms) { (void)ms; }
void lx_udelay(unsigned int us) { (void)us; }

void *lx_kmalloc(unsigned long size, unsigned gfp)
{
    (void)gfp;
    return malloc(size);
}

void lx_kfree(const void *p) { free((void *)p); }

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

void iwl_trans_rx_packet(struct iwl_ax211_priv *iwl, const uint8_t *buf, unsigned len)
{
    (void)iwl;
    (void)buf;
    (void)len;
}

void lx_iwlwifi_set_alive(int alive) { (void)alive; }

#include "iwl_fw_body.inc"

static uint32_t le32_at(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}

static int put_tlv(uint8_t *fw, size_t *pos, uint32_t type, const uint32_t *words, unsigned n)
{
    unsigned i;
    uint32_t len = n * 4u;

    memcpy(fw + *pos, &type, 4);
    memcpy(fw + *pos + 4, &len, 4);
    *pos += 8;
    for (i = 0; i < n; i++) {
        memcpy(fw + *pos, &words[i], 4);
        *pos += 4;
    }
    return 0;
}

static int synthetic_plan(void)
{
    uint8_t fw[sizeof(struct iwl_tlv_ucode_header) + 256];
    uint32_t magic = IWL_TLV_UCODE_MAGIC;
    size_t pos;
    struct iwl_ax211_priv iwl;
    struct iwl_8000_load_plan plan;
    struct iwl_8000_fh_prog fh;
    const uint32_t pay = 0x11223344u;

    memset(fw, 0, sizeof(fw));
    memcpy(fw + 4, &magic, 4);
    pos = sizeof(struct iwl_tlv_ucode_header);
    {
        uint32_t w[] = { 0x800000u, pay, pay };
        put_tlv(fw, &pos, IWL_UCODE_TLV_SEC_RT, w, 3);
    }
    {
        uint32_t w[] = { 0x801000u, pay };
        put_tlv(fw, &pos, IWL_UCODE_TLV_SEC_RT, w, 2);
    }
    {
        uint32_t w[] = { IWL_CPU1_CPU2_SEPARATOR };
        put_tlv(fw, &pos, IWL_UCODE_TLV_SEC_RT, w, 1);
    }
    {
        uint32_t w[] = { 0x880000u, pay, pay };
        put_tlv(fw, &pos, IWL_UCODE_TLV_SEC_RT, w, 3);
    }
    {
        uint32_t w[] = { IWL_PAGING_SEPARATOR };
        put_tlv(fw, &pos, IWL_UCODE_TLV_SEC_RT, w, 1);
    }
    {
        uint32_t w[] = { 0x400000u, pay };
        put_tlv(fw, &pos, IWL_UCODE_TLV_SEC_RT, w, 2);
    }

    memset(&iwl, 0, sizeof(iwl));
    if (iwl_fw_parse_tlv(&iwl, fw, pos) != 0) {
        fprintf(stderr, "FALLO: parse sintético 8000\n");
        return -1;
    }
    if (iwl_8000_plan_load(&iwl.fw, &plan) != 0) {
        fprintf(stderr, "FALLO: plan sintético\n");
        return -1;
    }
    if (plan.uses_context_info || plan.cpu1_secs != 2 || plan.cpu2_secs != 1 ||
        plan.n_chunks != 3 || plan.cpu1_final != 0xFFFFu ||
        plan.cpu2_final != 0xFFFFFFFFu) {
        fprintf(stderr, "FALLO: plan sintético cpu1=%u cpu2=%u chunks=%u ctx=%u\n",
                (unsigned)plan.cpu1_secs, (unsigned)plan.cpu2_secs,
                (unsigned)plan.n_chunks, (unsigned)plan.uses_context_info);
        return -1;
    }
    if (plan.chunks[0].dst != 0x800000u || plan.chunks[0].cpu != 1 ||
        plan.chunks[1].dst != 0x801000u || plan.chunks[2].dst != 0x880000u ||
        plan.chunks[2].cpu != 2) {
        fprintf(stderr, "FALLO: destinos/CPU del plan sintético\n");
        return -1;
    }
    if (plan.status[0].load_status != 0x1u || plan.status[1].load_status != 0x3u ||
        plan.status[2].load_status != (1u << 16)) {
        fprintf(stderr, "FALLO: bitmask FH_UCODE_LOAD_STATUS 0x%x 0x%x 0x%x\n",
                plan.status[0].load_status, plan.status[1].load_status,
                plan.status[2].load_status);
        return -1;
    }

    iwl_8000_fh_program(0x800000u, 0x12345000ull, 0x1000u, &fh);
    if (fh.tcsr_cfg_reg != 0x1E20u || fh.sram_reg != 0x19C8u ||
        fh.sram_val != 0x800000u || fh.tfdib0_reg != 0x1948u ||
        fh.tfdib0_val != 0x12345000u || fh.tfdib1_reg != 0x194Cu ||
        fh.tfdib1_val != 0x1000u || fh.buf_sts_reg != 0x1E28u ||
        fh.buf_sts_val != 0x00101003u || fh.tcsr_run != 0x80100000u ||
        fh.tcsr_pause != 0) {
        fprintf(stderr, "FALLO: registros FH "
                "tcsr=0x%x sram=0x%x/0x%x tfdib=0x%x/0x%x+0x%x/0x%x "
                "sts=0x%x/0x%x run=0x%x\n",
                fh.tcsr_cfg_reg, fh.sram_reg, fh.sram_val,
                fh.tfdib0_reg, fh.tfdib0_val, fh.tfdib1_reg, fh.tfdib1_val,
                fh.buf_sts_reg, fh.buf_sts_val, fh.tcsr_run);
        return -1;
    }
    iwl_8000_fh_program(0x40000u, 0x100000000ull, 0x80u, &fh);
    if (fh.tfdib1_val != ((1u << 28) | 0x80u)) {
        fprintf(stderr, "FALLO: dma hi tfdib1=0x%x\n", fh.tfdib1_val);
        return -1;
    }
    if (FH_UCODE_LOAD_STATUS != 0x1AF0u || RELEASE_CPU_RESET != 0x300Cu ||
        RELEASE_CPU_RESET_BIT != (1u << 24) || WFPM_GP2 != 0xA030B4u ||
        UREG_CPU_INIT_RUN == RELEASE_CPU_RESET) {
        fprintf(stderr, "FALLO: constantes 8000 vs gen2\n");
        return -1;
    }
    puts("OK: plan sintético 8000 + programación FH");
    return 0;
}

static int hcmd_constants(void)
{
    if (sizeof(struct iwl_tfd) != IWL_GEN1_TFD_SIZE ||
        IWL_8000_TFD_RING_N * IWL_GEN1_TFD_SIZE != 32768u ||
        IWL_8000_NUM_QUEUES * IWL_SCD_BC_TBL_BYTES == 0u ||
        IWL_8000_RX_N != 256u || IWL_8000_RX_LOG != 8u ||
        FH_MEM_CBBC_QUEUE(0) != 0x19d0u ||
        SCD_SRAM_BASE_ADDR != 0xa02c00u ||
        SCD_DRAM_BASE_ADDR != 0xa02c08u ||
        SCD_TXFACT != 0xa02c10u ||
        CSR_INI_SET_MASK == 0u) {
        fprintf(stderr, "FALLO: constantes HCMD/SCD/RX 8000\n");
        return -1;
    }
    puts("OK: constantes HCMD/SCD/RX 8000 (256 RBD log=8, CSR_INI_SET_MASK)");
    return 0;
}

static int chunk_wait_policy(void)
{
    uint32_t tssr_idle = FH_TSSR_TX_STATUS_REG_MSK_CHNL_IDLE(FH_SRVC_CHNL);

    if (iwl_8000_chunk_done_policy(0u, tssr_idle) != 0) {
        fprintf(stderr, "FALLO: TSSR idle sin FH_TX no debe ser chunk OK\n");
        return -1;
    }
    if (iwl_8000_chunk_done_policy(CSR_INT_BIT_FH_TX, 0u) != 1) {
        fprintf(stderr, "FALLO: FH_TX debe completar chunk\n");
        return -1;
    }
    puts("OK: wait_chunk solo FH_TX (TSSR idle sin INT no cuenta)");
    return 0;
}

static int tx_init_order(void)
{
    uint64_t ring_dma[IWL_8000_NUM_QUEUES];
    struct iwl_8000_tx_init_prog prog;
    unsigned qid;

    for (qid = 0; qid < IWL_8000_NUM_QUEUES; qid++)
        ring_dma[qid] = 0x10000000ull + (uint64_t)qid * 0x20000ull;
    iwl_8000_tx_init_program(0x2000ull, ring_dma, IWL_8000_NUM_QUEUES, &prog);

    if (prog.scd_txfact != 0u || prog.kw_reg != FH_KW_MEM_ADDR_REG ||
        prog.kw_val != 0x200u ||
        prog.gp_ctrl_set != (SCD_GP_CTRL_AUTO_ACTIVE_MODE |
                             SCD_GP_CTRL_ENABLE_31_QUEUES)) {
        fprintf(stderr, "FALLO: tx_init SCD/KW/GP txfact=0x%x kw=0x%x/0x%x gp=0x%x\n",
                prog.scd_txfact, prog.kw_reg, prog.kw_val, prog.gp_ctrl_set);
        return -1;
    }
    for (qid = 0; qid < IWL_8000_NUM_QUEUES; qid++) {
        unsigned slots = iwl_8000_tx_slots(qid, IWL_MVM_DQA_CMD_QUEUE);
        uint32_t expect = (uint32_t)(ring_dma[qid] >> 8);

        if (prog.cbbc[qid] != expect) {
            fprintf(stderr, "FALLO: CBBC q%u=0x%x vs 0x%x\n",
                    qid, prog.cbbc[qid], expect);
            return -1;
        }
        if (qid == IWL_MVM_DQA_CMD_QUEUE) {
            if (slots != IWL_CMD_QUEUE_SIZE) {
                fprintf(stderr, "FALLO: slots cmd=%u\n", slots);
                return -1;
            }
        } else if (slots != IWL_DEFAULT_QUEUE_SIZE) {
            fprintf(stderr, "FALLO: slots q%u=%u\n", qid, slots);
            return -1;
        }
    }
    if (FH_MEM_CBBC_QUEUE(0) != 0x19d0u || SCD_GP_CTRL != 0xa02da8u ||
        PCI_CFG_RETRY_TIMEOUT != 0x41u) {
        fprintf(stderr, "FALLO: constantes tx_init 8000\n");
        return -1;
    }
    puts("OK: tx_init 31 colas (SCD off→KW→CBBC→SCD_GP_CTRL) vs Linux tx.c:546");
    return 0;
}

static int tx_preload_gate(void)
{
    struct iwl_ax211_priv iwl;

    memset(&iwl, 0, sizeof(iwl));
    if (iwl_8000_tx_preload_ready(&iwl) != 0) {
        fprintf(stderr, "FALLO: tx_preload_ready debe ser 0 al inicio\n");
        return -1;
    }
    iwl.tx_preload_ready = 1;
    if (iwl_8000_tx_preload_ready(&iwl) != 1) {
        fprintf(stderr, "FALLO: tx_preload_ready debe ser 1 tras tx_init\n");
        return -1;
    }
    puts("OK: start no carga INIT sin tx_preload_ready");
    return 0;
}

static int nic_config_8265(void)
{
    uint32_t val = iwl_8000_nic_config_value(0x230u, 0x00330018u);
    uint32_t phy = ((uint32_t)0x0u << CSR_HW_IF_CONFIG_REG_POS_PHY_TYPE) |
                   ((uint32_t)0x2u << CSR_HW_IF_CONFIG_REG_POS_PHY_STEP) |
                   ((uint32_t)0x1u << CSR_HW_IF_CONFIG_REG_POS_PHY_DASH);

    if ((val & CSR_HW_IF_CONFIG_REG_MSK_PHY_TYPE) != (phy & CSR_HW_IF_CONFIG_REG_MSK_PHY_TYPE) ||
        (val & CSR_HW_IF_CONFIG_REG_MSK_PHY_STEP) != (phy & CSR_HW_IF_CONFIG_REG_MSK_PHY_STEP) ||
        (val & CSR_HW_IF_CONFIG_REG_MSK_PHY_DASH) != (phy & CSR_HW_IF_CONFIG_REG_MSK_PHY_DASH) ||
        (val & CSR_HW_IF_CONFIG_REG_BIT_RADIO_SI) != 0u) {
        fprintf(stderr, "FALLO: nic_config 8265 val=0x%08x phy=0x%08x\n", val, phy);
        return -1;
    }
    puts("OK: nic_config PHY_SKU 0x00330018 → CSR_HW_IF_CONFIG_REG");
    return 0;
}

static int real_8265(const char *path)
{
    FILE *f;
    long sz;
    uint8_t *fw;
    struct iwl_ax211_priv iwl;
    struct iwl_8000_load_plan plan_init;
    struct iwl_8000_load_plan plan_rt;
    uint32_t first_dest;
    unsigned i;

    f = fopen(path, "rb");
    if (!f) {
        perror(path);
        return -1;
    }
    fseek(f, 0, SEEK_END);
    sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    fw = malloc((size_t)sz);
    if (!fw || fread(fw, 1, (size_t)sz, f) != (size_t)sz) {
        fclose(f);
        return -1;
    }
    fclose(f);
    memset(&iwl, 0, sizeof(iwl));
    if (iwl_fw_parse_tlv(&iwl, fw, (unsigned long)sz) != 0) {
        fprintf(stderr, "FALLO: parse 8265\n");
        return -1;
    }
    if (!iwl.fw.sec_init_n || !iwl.fw.rt_n) {
        fprintf(stderr, "FALLO: 8265 sec_init_n=%d rt_n=%d\n",
                iwl.fw.sec_init_n, iwl.fw.rt_n);
        return -1;
    }
    if (iwl_8000_plan_load_init(&iwl.fw, &plan_init) != 0) {
        fprintf(stderr, "FALLO: plan INIT 8265\n");
        return -1;
    }
    if (iwl_8000_plan_load(&iwl.fw, &plan_rt) != 0) {
        fprintf(stderr, "FALLO: plan RT 8265\n");
        return -1;
    }
    first_dest = le32_at(iwl.fw.rt[0].data);
    if (plan_rt.uses_context_info || !plan_rt.cpu1_secs || !plan_rt.cpu2_secs ||
        plan_rt.n_chunks == 0 || plan_rt.chunks[0].dst != first_dest) {
        fprintf(stderr, "FALLO: 8265 RT ctx=%u cpu1=%u cpu2=%u dst=0x%x vs 0x%x\n",
                (unsigned)plan_rt.uses_context_info, (unsigned)plan_rt.cpu1_secs,
                (unsigned)plan_rt.cpu2_secs, plan_rt.chunks[0].dst, first_dest);
        return -1;
    }
    if (plan_init.uses_context_info || !plan_init.cpu1_secs || !plan_init.cpu2_secs ||
        plan_init.n_chunks == 0) {
        fprintf(stderr, "FALLO: 8265 INIT ctx=%u cpu1=%u cpu2=%u chunks=%u\n",
                (unsigned)plan_init.uses_context_info, (unsigned)plan_init.cpu1_secs,
                (unsigned)plan_init.cpu2_secs, (unsigned)plan_init.n_chunks);
        return -1;
    }
    for (i = 0; i < plan_rt.n_chunks; i++) {
        if (plan_rt.chunks[i].len == 0 || plan_rt.chunks[i].len > FH_MEM_TB_MAX_LENGTH) {
            fprintf(stderr, "FALLO: RT chunk %u len=%u\n", i, plan_rt.chunks[i].len);
            return -1;
        }
        if (plan_rt.chunks[i].cpu != 1 && plan_rt.chunks[i].cpu != 2) {
            fprintf(stderr, "FALLO: RT chunk %u cpu=%u\n", i,
                    (unsigned)plan_rt.chunks[i].cpu);
            return -1;
        }
    }
    printf("OK: 8265 INIT cpu1=%u cpu2=%u chunks=%u; RT cpu1=%u cpu2=%u chunks=%u dst0=0x%x\n",
           (unsigned)plan_init.cpu1_secs, (unsigned)plan_init.cpu2_secs,
           (unsigned)plan_init.n_chunks,
           (unsigned)plan_rt.cpu1_secs, (unsigned)plan_rt.cpu2_secs,
           (unsigned)plan_rt.n_chunks, plan_rt.chunks[0].dst);
    free(fw);
    return 0;
}

int main(int argc, char **argv)
{
    if (synthetic_plan() != 0)
        return 1;
    if (hcmd_constants() != 0)
        return 1;
    if (tx_init_order() != 0)
        return 1;
    if (tx_preload_gate() != 0)
        return 1;
    if (chunk_wait_policy() != 0)
        return 1;
    if (nic_config_8265() != 0)
        return 1;
    if (argc == 2 && real_8265(argv[1]) != 0)
        return 1;
    return 0;
}
