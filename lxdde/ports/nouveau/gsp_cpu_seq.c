/* `r535_gsp_msg_run_cpu_sequencer`: MMIO que GSP-RM pide vía RPC. */
#include "gsp_cpu_seq.h"
#include "gsp_mmio.h"
#include "falcon_lx.h"
#include "lx_emul.h"

#define NV_PGSP_FALCON  0x00110000u

enum gsp_seq_opcode {
    GSP_SEQ_BUF_OPCODE_REG_WRITE = 0,
    GSP_SEQ_BUF_OPCODE_REG_MODIFY,
    GSP_SEQ_BUF_OPCODE_REG_POLL,
    GSP_SEQ_BUF_OPCODE_DELAY_US,
    GSP_SEQ_BUF_OPCODE_REG_STORE,
    GSP_SEQ_BUF_OPCODE_CORE_RESET,
    GSP_SEQ_BUF_OPCODE_CORE_START,
    GSP_SEQ_BUF_OPCODE_CORE_WAIT_FOR_HALT,
    GSP_SEQ_BUF_OPCODE_CORE_RESUME,
};

struct gsp_seq_reg_write {
    uint32_t addr;
    uint32_t val;
};

struct gsp_seq_reg_modify {
    uint32_t addr;
    uint32_t mask;
    uint32_t val;
};

struct gsp_seq_reg_poll {
    uint32_t addr;
    uint32_t mask;
    uint32_t val;
    uint32_t timeout;
    uint32_t error;
};

struct gsp_seq_delay_us {
    uint32_t val;
};

struct gsp_seq_reg_store {
    uint32_t addr;
    uint32_t index;
};

struct gsp_seq_cmd {
    uint32_t op_code;
    union {
        struct gsp_seq_reg_write reg_write;
        struct gsp_seq_reg_modify reg_modify;
        struct gsp_seq_reg_poll reg_poll;
        struct gsp_seq_delay_us delay_us;
        struct gsp_seq_reg_store reg_store;
    } payload;
};

struct gsp_run_cpu_sequencer {
    uint32_t buffer_size_dword;
    uint32_t cmd_index;
    uint32_t reg_save_area[8];
    /* commandBuffer[] */
};

#define GSP_CPU_SEQ_HDR_DWORDS 10u

static unsigned seq_payload_dwords(unsigned op)
{
    switch (op) {
    case GSP_SEQ_BUF_OPCODE_REG_WRITE:
        return (unsigned)(sizeof(struct gsp_seq_reg_write) / 4u);
    case GSP_SEQ_BUF_OPCODE_REG_MODIFY:
        return (unsigned)(sizeof(struct gsp_seq_reg_modify) / 4u);
    case GSP_SEQ_BUF_OPCODE_REG_POLL:
        return (unsigned)(sizeof(struct gsp_seq_reg_poll) / 4u);
    case GSP_SEQ_BUF_OPCODE_DELAY_US:
        return (unsigned)(sizeof(struct gsp_seq_delay_us) / 4u);
    case GSP_SEQ_BUF_OPCODE_REG_STORE:
        return (unsigned)(sizeof(struct gsp_seq_reg_store) / 4u);
    default:
        return 0;
    }
}

static void seq_reg_poll(uint32_t addr, uint32_t mask, uint32_t val, uint32_t usec)
{
    unsigned t;

    if (!usec) {
        usec = 4000000u;
    }
    t = usec / 1000u;
    if (t < 1u) {
        t = 1u;
    }
    while (t--) {
        if ((gsp_mmio_rd32(addr) & mask) == val) {
            return;
        }
        lx_mdelay(1);
    }
    lx_printk("nouveau-lx: cpu_seq poll timeout addr=0x%x mask=0x%x val=0x%x\n",
              addr, mask, val);
}

int gsp_cpu_seq_run(const void *payload, uint32_t len)
{
    const struct gsp_run_cpu_sequencer *seq = payload;
    const uint32_t *cmd_buf;
    uint32_t reg_save[8];
    unsigned ptr;
    unsigned cmds = 0;

    if (!payload || len < sizeof(*seq)) {
        return -1;
    }

    cmd_buf = (const uint32_t *)(const void *)((const unsigned char *)seq +
                                             sizeof(*seq));
    for (unsigned i = 0; i < 8u; i++) {
        reg_save[i] = seq->reg_save_area[i];
    }

    lx_printk("nouveau-lx: cpu_seq buffer=%u index=%u\n",
              (unsigned)seq->buffer_size_dword, (unsigned)seq->cmd_index);

    for (ptr = 0; ptr < seq->cmd_index; ) {
        const struct gsp_seq_cmd *cmd;
        unsigned pay;
        unsigned hdr = GSP_CPU_SEQ_HDR_DWORDS + ptr;

        if ((hdr + 1u) * 4u > len) {
            lx_printk("nouveau-lx: cpu_seq truncado en cmd %u\n", cmds);
            return -1;
        }
        cmd = (const struct gsp_seq_cmd *)&cmd_buf[ptr];
        pay = seq_payload_dwords(cmd->op_code);
        if ((hdr + 1u + pay) * 4u > len) {
            lx_printk("nouveau-lx: cpu_seq payload truncado en cmd %u\n", cmds);
            return -1;
        }
        ptr += 1u + pay;
        cmds++;

        switch (cmd->op_code) {
        case GSP_SEQ_BUF_OPCODE_REG_WRITE:
            gsp_mmio_wr32(cmd->payload.reg_write.addr, cmd->payload.reg_write.val);
            break;
        case GSP_SEQ_BUF_OPCODE_REG_MODIFY: {
            uint32_t v = gsp_mmio_rd32(cmd->payload.reg_modify.addr);

            v = (v & ~cmd->payload.reg_modify.mask) |
                (cmd->payload.reg_modify.val & cmd->payload.reg_modify.mask);
            gsp_mmio_wr32(cmd->payload.reg_modify.addr, v);
            break;
        }
        case GSP_SEQ_BUF_OPCODE_REG_POLL:
            seq_reg_poll(cmd->payload.reg_poll.addr, cmd->payload.reg_poll.mask,
                         cmd->payload.reg_poll.val, cmd->payload.reg_poll.timeout);
            break;
        case GSP_SEQ_BUF_OPCODE_DELAY_US:
            if (cmd->payload.delay_us.val >= 1000u) {
                lx_mdelay(cmd->payload.delay_us.val / 1000u);
            } else if (cmd->payload.delay_us.val) {
                lx_udelay(cmd->payload.delay_us.val);
            }
            break;
        case GSP_SEQ_BUF_OPCODE_REG_STORE:
            if (cmd->payload.reg_store.index < 8u) {
                reg_save[cmd->payload.reg_store.index] =
                    gsp_mmio_rd32(cmd->payload.reg_store.addr);
            }
            break;
        case GSP_SEQ_BUF_OPCODE_CORE_RESET:
            (void)falcon_lx_gsp_reset_riscv(LX_FLCN_GSP_BASE);
            gsp_mmio_wr32(NV_PGSP_FALCON + 0x624u, 0x80u);
            gsp_mmio_wr32(NV_PGSP_FALCON + 0x10cu, 0u);
            break;
        case GSP_SEQ_BUF_OPCODE_CORE_START:
            if (gsp_mmio_rd32(NV_PGSP_FALCON + 0x100u) & 0x40u) {
                gsp_mmio_wr32(NV_PGSP_FALCON + 0x130u, 0x2u);
            } else {
                gsp_mmio_wr32(NV_PGSP_FALCON + 0x100u, 0x2u);
            }
            break;
        case GSP_SEQ_BUF_OPCODE_CORE_WAIT_FOR_HALT: {
            unsigned w = 2000u;

            while (w--) {
                if (gsp_mmio_rd32(NV_PGSP_FALCON + 0x100u) & 0x10u) {
                    break;
                }
                lx_mdelay(1);
            }
            break;
        }
        case GSP_SEQ_BUF_OPCODE_CORE_RESUME:
            lx_printk("nouveau-lx: cpu_seq CORE_RESUME omitido (sin SEC2 resume)\n");
            break;
        default:
            lx_printk("nouveau-lx: cpu_seq opcode %u desconocido\n",
                      (unsigned)cmd->op_code);
            return -1;
        }
    }

    lx_printk("nouveau-lx: cpu_seq %u comando(s) ejecutados\n", cmds);
    return 0;
}
