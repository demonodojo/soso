/*
 * R6: validación de MCC_UPDATE y política de canales de scan.
 *
 * Cubre respuesta truncada, tamaño inconsistente, status sin perfil, layouts
 * v3/v4/v8, perfil válido con cero canales, ausencia de NVM y restricciones
 * pasivas (incluida la codificación de banda de v17).
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

void lx_printk(const char *fmt, ...) { (void)fmt; }
void lx_mdelay(unsigned int ms) { (void)ms; }

int iwl_trans_send_cmd(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                       const void *payload, uint16_t pay_len)
{
    (void)iwl;
    (void)group;
    (void)id;
    (void)payload;
    (void)pay_len;
    return 0;
}

int iwl_trans_send_cmd_wait(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t id,
                            const void *payload, uint16_t pay_len, int wait_ms)
{
    (void)group;
    (void)id;
    (void)payload;
    (void)pay_len;
    (void)wait_ms;
    iwl->cmd_status = 1;
    return 0;
}

void iwl_trans_poll(struct iwl_ax211_priv *iwl) { (void)iwl; }

int iwl_mvm_up_minimal(struct iwl_ax211_priv *iwl)
{
    iwl->mvm_up_done = 1;
    return 0;
}

int iwl_mvm_assoc_prepare(struct iwl_ax211_priv *iwl, const char *ssid,
                          const uint8_t *bssid)
{
    (void)iwl;
    (void)ssid;
    (void)bssid;
    return -1;
}

int iwl_fw_cmd_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    unsigned i;

    for (i = 0; i < iwl->cmd_ver_count; i++) {
        if (iwl->cmd_ver[i].group == group && iwl->cmd_ver[i].cmd == cmd)
            return iwl->cmd_ver[i].version;
    }
    return 0;
}

int iwl_fw_notif_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    unsigned i;

    for (i = 0; i < iwl->cmd_ver_count; i++) {
        if (iwl->cmd_ver[i].group == group && iwl->cmd_ver[i].cmd == cmd)
            return iwl->cmd_ver[i].notif_version;
    }
    return 0;
}

uint8_t iwl_mvm_scan_rx_ant(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 1;
}

void iwl_ax211_add_bss(const struct iwl_ax211_bss *bss) { (void)bss; }

/* Offsets del scan v17 (misma referencia que scan_abi_test.c). */
#define REF_GENERAL_FLAGS 8u
#define REF_CHAN_COUNT    45u
#define REF_CHAN0         48u
#define REF_CHAN_STRIDE   8u

static uint32_t chan_flags_at(const uint8_t *buf, unsigned idx)
{
    const uint8_t *p = buf + REF_CHAN0 + idx * REF_CHAN_STRIDE;

    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}

static uint8_t chan_num_at(const uint8_t *buf, unsigned idx)
{
    return buf[REF_CHAN0 + idx * REF_CHAN_STRIDE + 4];
}

static uint8_t chan_byte5_at(const uint8_t *buf, unsigned idx)
{
    return buf[REF_CHAN0 + idx * REF_CHAN_STRIDE + 5];
}

static void set_scan_ver(struct iwl_ax211_priv *iwl, uint8_t ver)
{
    iwl->cmd_ver_count = 1;
    iwl->cmd_ver[0].group = LONG_GROUP;
    iwl->cmd_ver[0].cmd = SCAN_REQ_UMAC;
    iwl->cmd_ver[0].version = ver;
}

/* Construye una respuesta MCC con la cabecera de la versión indicada. */
static unsigned build_mcc(uint8_t *out, int notif_ver, uint32_t status,
                          uint16_t mcc, uint32_t n_channels,
                          const uint32_t *channels, uint32_t n_declarado)
{
    unsigned hdr;

    memset(out, 0, 1024);
    if (notif_ver >= 8)
        hdr = (unsigned)sizeof(struct iwl_mcc_update_resp_v8);
    else if (notif_ver >= 4)
        hdr = (unsigned)sizeof(struct iwl_mcc_update_resp_v4);
    else
        hdr = (unsigned)sizeof(struct iwl_mcc_update_resp_v3);

    memcpy(out, &status, 4);
    memcpy(out + 4, &mcc, 2);
    if (notif_ver >= 8) {
        struct iwl_mcc_update_resp_v8 *v8 = (struct iwl_mcc_update_resp_v8 *)(void *)out;
        v8->n_channels = n_declarado;
    } else if (notif_ver >= 4) {
        struct iwl_mcc_update_resp_v4 *v4 = (struct iwl_mcc_update_resp_v4 *)(void *)out;
        v4->n_channels = n_declarado;
    } else {
        struct iwl_mcc_update_resp_v3 *v3 = (struct iwl_mcc_update_resp_v3 *)(void *)out;
        v3->n_channels = n_declarado;
    }
    if (channels && n_channels)
        memcpy(out + hdr, channels, n_channels * 4u);
    return hdr + n_channels * 4u;
}

static int check_mcc_ok_v8(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t resp[1024];
    uint32_t chans[4] = {
        NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE,
        0,
        NVM_CHANNEL_VALID,
        NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE | NVM_CHANNEL_RADAR,
    };
    unsigned len;

    memset(&iwl, 0, sizeof(iwl));
    len = build_mcc(resp, 8, MCC_RESP_NEW_CHAN_PROFILE, 0x4553, 4, chans, 4);
    if (iwl_mvm_apply_mcc_resp(&iwl, 8, resp, len) != 0) {
        fprintf(stderr, "MCC v8 válida rechazada\n");
        return -1;
    }
    if (iwl.nvm_n_channels != 4 || iwl.chan_src != IWL_CHAN_SRC_MCC ||
        !iwl.lar_regdom_set || !iwl.mcc_done) {
        fprintf(stderr, "MCC v8 no aplicó el perfil (n=%u src=%u lar=%u)\n",
                (unsigned)iwl.nvm_n_channels, (unsigned)iwl.chan_src,
                (unsigned)iwl.lar_regdom_set);
        return -1;
    }
    if (memcmp(iwl.nvm_chan_flags, chans, sizeof(chans)) != 0) {
        fprintf(stderr, "MCC v8 no copió la lista de canales\n");
        return -1;
    }
    if (iwl.mcc_applied != 0x4553 || iwl.mcc_status != MCC_RESP_NEW_CHAN_PROFILE) {
        fprintf(stderr, "MCC v8 mcc=0x%04x status=%u\n",
                (unsigned)iwl.mcc_applied, (unsigned)iwl.mcc_status);
        return -1;
    }
    /* mcc 0x0000 → dominio mundial "00" (W/A de Linux). */
    memset(&iwl, 0, sizeof(iwl));
    len = build_mcc(resp, 8, MCC_RESP_SAME_CHAN_PROFILE, 0, 4, chans, 4);
    if (iwl_mvm_apply_mcc_resp(&iwl, 8, resp, len) != 0 ||
        iwl.mcc_applied != 0x3030) {
        fprintf(stderr, "mcc 0 no se tradujo a \"00\" (0x%04x)\n",
                (unsigned)iwl.mcc_applied);
        return -1;
    }
    puts("OK: MCC v8 válida aplica lista de canales y regdominio");
    return 0;
}

static int check_mcc_rechazos(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t resp[1024];
    uint32_t chans[4] = { NVM_CHANNEL_VALID, NVM_CHANNEL_VALID, 0, 0 };
    unsigned len;

    /* 1) Truncada por debajo de la cabecera. */
    memset(&iwl, 0, sizeof(iwl));
    len = build_mcc(resp, 8, MCC_RESP_NEW_CHAN_PROFILE, 0x4553, 4, chans, 4);
    if (iwl_mvm_apply_mcc_resp(&iwl, 8, resp,
                               (unsigned)sizeof(struct iwl_mcc_update_resp_v8) - 1u) == 0) {
        fprintf(stderr, "MCC más corta que la cabecera aceptada\n");
        return -1;
    }
    if (iwl.lar_regdom_set || iwl.mcc_done || iwl.nvm_n_channels) {
        fprintf(stderr, "MCC truncada dejó estado aplicado\n");
        return -1;
    }

    /* 2) n_channels declarado ≠ bytes recibidos (ni de más ni de menos). */
    memset(&iwl, 0, sizeof(iwl));
    if (iwl_mvm_apply_mcc_resp(&iwl, 8, resp, len - 4u) == 0) {
        fprintf(stderr, "MCC con un canal de menos aceptada\n");
        return -1;
    }
    if (iwl_mvm_apply_mcc_resp(&iwl, 8, resp, len + 4u) == 0) {
        fprintf(stderr, "MCC con bytes de sobra aceptada\n");
        return -1;
    }

    /* 3) Más canales de los que caben. */
    memset(&iwl, 0, sizeof(iwl));
    len = build_mcc(resp, 8, MCC_RESP_NEW_CHAN_PROFILE, 0x4553, 4, chans,
                    IWL_NUM_CHANNELS + 1u);
    if (iwl_mvm_apply_mcc_resp(&iwl, 8, resp, len) == 0) {
        fprintf(stderr, "MCC con %u canales aceptada\n", IWL_NUM_CHANNELS + 1u);
        return -1;
    }

    /* 4) Status sin perfil aplicable. */
    {
        const uint32_t malos[] = { MCC_RESP_INVALID, MCC_RESP_NVM_DISABLED, 9 };
        unsigned i;

        for (i = 0; i < sizeof(malos) / sizeof(malos[0]); i++) {
            memset(&iwl, 0, sizeof(iwl));
            len = build_mcc(resp, 8, malos[i], 0x4553, 4, chans, 4);
            if (iwl_mvm_apply_mcc_resp(&iwl, 8, resp, len) == 0) {
                fprintf(stderr, "status %u aplicó perfil\n", (unsigned)malos[i]);
                return -1;
            }
            if (iwl.lar_regdom_set) {
                fprintf(stderr, "status %u marcó regdominio\n", (unsigned)malos[i]);
                return -1;
            }
        }
    }

    /* 5) Cabecera de otra versión: los tamaños no cuadran. */
    memset(&iwl, 0, sizeof(iwl));
    len = build_mcc(resp, 3, MCC_RESP_NEW_CHAN_PROFILE, 0x4553, 4, chans, 4);
    if (iwl_mvm_apply_mcc_resp(&iwl, 8, resp, len) == 0) {
        fprintf(stderr, "payload v3 interpretado como v8\n");
        return -1;
    }
    if (iwl_mvm_apply_mcc_resp(&iwl, 3, resp, len) != 0) {
        fprintf(stderr, "payload v3 rechazado con notif_ver=3\n");
        return -1;
    }
    memset(&iwl, 0, sizeof(iwl));
    len = build_mcc(resp, 4, MCC_RESP_NEW_CHAN_PROFILE, 0x4553, 4, chans, 4);
    if (iwl_mvm_apply_mcc_resp(&iwl, 4, resp, len) != 0 ||
        iwl.nvm_n_channels != 4) {
        fprintf(stderr, "payload v4 no aplicado con notif_ver=4\n");
        return -1;
    }
    puts("OK: MCC truncada, descuadrada, excesiva o sin perfil no se aplica");
    return 0;
}

/* Perfil válido con cero canales usables ≠ NVM ausente. */
static int check_cero_canales(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t resp[1024];
    uint8_t buf[4096];
    uint32_t chans[3] = { 0, 0, 0 };
    uint8_t ch[SCAN_MAX_NUM_CHANS_V3];
    uint8_t band[SCAN_MAX_NUM_CHANS_V3];
    uint8_t passive[SCAN_MAX_NUM_CHANS_V3];
    int origen = -1;
    unsigned len;
    unsigned n;

    memset(&iwl, 0, sizeof(iwl));
    set_scan_ver(&iwl, 17);
    len = build_mcc(resp, 8, MCC_RESP_NEW_CHAN_PROFILE, 0x4553, 3, chans, 3);
    if (iwl_mvm_apply_mcc_resp(&iwl, 8, resp, len) != 0) {
        fprintf(stderr, "MCC sin canales válidos debería aplicarse\n");
        return -1;
    }
    n = iwl_mvm_collect_scan_channels(&iwl, ch, band, passive,
                                      SCAN_MAX_NUM_CHANS_V3, &origen);
    if (n != 0 || origen != IWL_CHAN_SRC_EMPTY) {
        fprintf(stderr, "perfil vacío: n=%u origen=%d (esperaba 0/EMPTY)\n", n, origen);
        return -1;
    }
    if (iwl_mvm_build_scan_req(&iwl, buf, sizeof(buf)) != 0) {
        fprintf(stderr, "perfil vacío no debe construir SCAN_REQ\n");
        return -1;
    }

    /* NVM ausente: lista mínima, pero solo pasiva. */
    memset(&iwl, 0, sizeof(iwl));
    set_scan_ver(&iwl, 17);
    origen = -1;
    n = iwl_mvm_collect_scan_channels(&iwl, ch, band, passive,
                                      SCAN_MAX_NUM_CHANS_V3, &origen);
    if (n == 0 || origen != IWL_CHAN_SRC_FALLBACK) {
        fprintf(stderr, "sin NVM: n=%u origen=%d (esperaba fallback)\n", n, origen);
        return -1;
    }
    {
        unsigned i;

        for (i = 0; i < n; i++) {
            if (!passive[i]) {
                fprintf(stderr, "fallback con canal activo (%u)\n", ch[i]);
                return -1;
            }
        }
    }
    if (iwl_mvm_build_scan_req(&iwl, buf, sizeof(buf)) == 0) {
        fprintf(stderr, "fallback pasivo debería construir SCAN_REQ\n");
        return -1;
    }
    {
        uint16_t gflags = (uint16_t)buf[REF_GENERAL_FLAGS] |
                          ((uint16_t)buf[REF_GENERAL_FLAGS + 1] << 8);

        if (!(gflags & IWL_UMAC_SCAN_GEN_FLAGS_V2_FORCE_PASSIVE)) {
            fprintf(stderr, "scan sin regdominio no declaró FORCE_PASSIVE (0x%x)\n",
                    gflags);
            return -1;
        }
        if (!iwl.scan_passive_only) {
            fprintf(stderr, "scan_passive_only no marcado\n");
            return -1;
        }
    }
    /* LAR habilitado sin regdominio: el perfil está, pero solo escucha. */
    memset(&iwl, 0, sizeof(iwl));
    set_scan_ver(&iwl, 17);
    iwl.lar_enabled = 1;
    iwl.nvm_n_channels = 3;
    iwl.nvm_chan_flags[0] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
    iwl.nvm_chan_flags[1] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
    iwl.nvm_chan_flags[2] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
    origen = -1;
    n = iwl_mvm_collect_scan_channels(&iwl, ch, band, passive,
                                      SCAN_MAX_NUM_CHANS_V3, &origen);
    if (n != 3 || origen != IWL_CHAN_SRC_NVM) {
        fprintf(stderr, "LAR sin regdominio: n=%u origen=%d\n", n, origen);
        return -1;
    }
    {
        unsigned i;

        for (i = 0; i < n; i++) {
            if (!passive[i]) {
                fprintf(stderr, "LAR sin regdominio dejó activo el canal %u\n",
                        ch[i]);
                return -1;
            }
        }
    }
    /* Con el regdominio aplicado, los mismos canales vuelven a ser activos. */
    iwl.lar_regdom_set = 1;
    n = iwl_mvm_collect_scan_channels(&iwl, ch, band, passive,
                                      SCAN_MAX_NUM_CHANS_V3, &origen);
    if (n != 3 || passive[0] || passive[1] || passive[2]) {
        fprintf(stderr, "con regdominio aplicado el scan debe ser activo\n");
        return -1;
    }
    puts("OK: cero canales válidos, NVM ausente, LAR sin regdominio y scan "
         "pasivo se distinguen");
    return 0;
}

/* Restricciones pasivas y codificación de banda por versión. */
static int check_pasivos_y_banda(void)
{
    struct iwl_ax211_priv iwl;
    uint8_t buf[4096];
    uint32_t f;

    memset(&iwl, 0, sizeof(iwl));
    set_scan_ver(&iwl, 17);
    iwl.nvm_n_channels = 16;
    /* índice 0 → ch 1 (2,4 GHz activo); 14 → ch 36 (5 GHz, solo interior);
     * 15 → ch 40 (5 GHz activo). */
    iwl.nvm_chan_flags[0] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
    iwl.nvm_chan_flags[14] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE |
                             NVM_CHANNEL_INDOOR_ONLY;
    iwl.nvm_chan_flags[15] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
    if (iwl_mvm_build_scan_req(&iwl, buf, sizeof(buf)) == 0) {
        fprintf(stderr, "perfil con 3 canales no construyó SCAN_REQ\n");
        return -1;
    }
    if (buf[REF_CHAN_COUNT] != 3) {
        fprintf(stderr, "count=%u (esperaba 3)\n", buf[REF_CHAN_COUNT]);
        return -1;
    }
    if (chan_num_at(buf, 0) != 1 || chan_num_at(buf, 1) != 36 ||
        chan_num_at(buf, 2) != 40) {
        fprintf(stderr, "canales %u/%u/%u (esperaba 1/36/40)\n",
                chan_num_at(buf, 0), chan_num_at(buf, 1), chan_num_at(buf, 2));
        return -1;
    }
    /* v17: banda en flags[31:30]; el byte 5 es psd_20 y debe quedar a cero. */
    f = chan_flags_at(buf, 0);
    if ((f >> IWL_CHAN_CFG_FLAGS_BAND_POS) != 0 ||
        (f & IWL_UHB_CHAN_CFG_FLAG_FORCE_PASSIVE)) {
        fprintf(stderr, "ch1 v17 flags=0x%08x (banda 0, activo)\n", f);
        return -1;
    }
    f = chan_flags_at(buf, 1);
    if ((f >> IWL_CHAN_CFG_FLAGS_BAND_POS) != 1 ||
        !(f & IWL_UHB_CHAN_CFG_FLAG_FORCE_PASSIVE)) {
        fprintf(stderr, "ch36 v17 flags=0x%08x (banda 1, pasivo por interior)\n", f);
        return -1;
    }
    f = chan_flags_at(buf, 2);
    if ((f >> IWL_CHAN_CFG_FLAGS_BAND_POS) != 1 ||
        (f & IWL_UHB_CHAN_CFG_FLAG_FORCE_PASSIVE)) {
        fprintf(stderr, "ch40 v17 flags=0x%08x (banda 1, activo)\n", f);
        return -1;
    }
    if (chan_byte5_at(buf, 0) != 0 || chan_byte5_at(buf, 1) != 0) {
        fprintf(stderr, "v17 escribió el byte de banda (psd_20)\n");
        return -1;
    }
    if (iwl.scan_passive_only) {
        fprintf(stderr, "lista con canales activos marcada como pasiva\n");
        return -1;
    }

    /* v15: la banda va en v2.band y flags no lleva bits de banda. */
    memset(&iwl, 0, sizeof(iwl));
    set_scan_ver(&iwl, 15);
    iwl.nvm_n_channels = 16;
    iwl.nvm_chan_flags[0] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
    iwl.nvm_chan_flags[15] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE |
                             NVM_CHANNEL_RADAR;
    if (iwl_mvm_build_scan_req(&iwl, buf, sizeof(buf)) == 0)
        return -1;
    if (chan_byte5_at(buf, 0) != 0 || chan_byte5_at(buf, 1) != 1) {
        fprintf(stderr, "v15 banda en v2.band = %u/%u (esperaba 0/1)\n",
                chan_byte5_at(buf, 0), chan_byte5_at(buf, 1));
        return -1;
    }
    f = chan_flags_at(buf, 1);
    if ((f >> IWL_CHAN_CFG_FLAGS_BAND_POS) != 0) {
        fprintf(stderr, "v15 metió la banda en flags (0x%08x)\n", f);
        return -1;
    }
    if (!(f & IWL_UHB_CHAN_CFG_FLAG_FORCE_PASSIVE)) {
        fprintf(stderr, "canal con radar no marcado pasivo en v15\n");
        return -1;
    }
    if (chan_flags_at(buf, 0) & 1u) {
        fprintf(stderr, "bit 0 (SSID directo) usado como «pasivo»\n");
        return -1;
    }

    /* v6 comparte la misma política: sin lista fija propia. */
    memset(&iwl, 0, sizeof(iwl));
    set_scan_ver(&iwl, 6);
    iwl.nvm_n_channels = 3;
    iwl.nvm_chan_flags[0] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
    iwl.nvm_chan_flags[1] = 0;
    iwl.nvm_chan_flags[2] = NVM_CHANNEL_VALID;
    if (iwl_mvm_build_scan_req(&iwl, buf, sizeof(buf)) == 0)
        return -1;
    if (buf[41] != 2) {
        fprintf(stderr, "v6 count=%u (esperaba 2 del perfil NVM)\n", buf[41]);
        return -1;
    }
    {
        const uint8_t *c0 = buf + IWL_SCAN_REQ_UMAC_SIZE_V6;
        const uint8_t *c1 = c0 + sizeof(struct iwl_scan_channel_cfg_umac);

        if (c0[4] != 1 || c1[4] != 3) {
            fprintf(stderr, "v6 canales %u/%u (esperaba 1/3)\n", c0[4], c1[4]);
            return -1;
        }
        if (!(c1[3] & 0x04u)) {
            fprintf(stderr, "v6 canal sin ACTIVE no marcado pasivo\n");
            return -1;
        }
    }
    /* Con el perfil NVM entero (51 entradas de tabla) la petición v6 sigue
     * cabiendo en un slot de comando: antes la lista era fija de 21 canales y
     * nadie comprobaba el tamaño con la lista larga. */
    memset(&iwl, 0, sizeof(iwl));
    set_scan_ver(&iwl, 6);
    iwl.nvm_n_channels = IWL_NUM_CHANNELS;
    {
        unsigned i;

        for (i = 0; i < IWL_NUM_CHANNELS; i++) {
            iwl.nvm_chan_flags[i] = NVM_CHANNEL_VALID | NVM_CHANNEL_ACTIVE;
        }
    }
    {
        uint16_t pay = iwl_mvm_build_scan_req(&iwl, buf, sizeof(buf));

        if (pay == 0 || pay + 8u > IWL_CMD_SLOT_SIZE) {
            fprintf(stderr, "v6 con el perfil completo: pay=%u (slot %u)\n",
                    pay, IWL_CMD_SLOT_SIZE);
            return -1;
        }
        if (buf[41] == 0 || buf[41] > SCAN_MAX_NUM_CHANS_V3) {
            fprintf(stderr, "v6 count=%u fuera de rango\n", buf[41]);
            return -1;
        }
    }
    puts("OK: activo/pasivo y banda por versión (v6/v15/v17) unificados, y v6 "
         "con el perfil completo cabe en un slot");
    return 0;
}

int main(void)
{
    if (check_mcc_ok_v8() != 0)
        return 1;
    if (check_mcc_rechazos() != 0)
        return 1;
    if (check_cero_canales() != 0)
        return 1;
    if (check_pasivos_y_banda() != 0)
        return 1;
    return 0;
}
