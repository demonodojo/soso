/* MVM: scan, asociación, TX/RX 802.3. */
#include "iwl_internal.h"
#include "iwl_ax211.h"
#include "lx_emul.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);
extern int memcmp(const void *a, const void *b, unsigned long n);
extern char *strncpy(char *dst, const char *src, unsigned long n);
extern int strncmp(const char *a, const char *b, unsigned long n);

static const struct {
    uint8_t ch;
    uint8_t band;
} scan_channels[] = {
    {1, PHY_BAND_24}, {2, PHY_BAND_24}, {3, PHY_BAND_24}, {4, PHY_BAND_24},
    {5, PHY_BAND_24}, {6, PHY_BAND_24}, {7, PHY_BAND_24},
    {8, PHY_BAND_24}, {9, PHY_BAND_24}, {10, PHY_BAND_24}, {11, PHY_BAND_24},
    {12, PHY_BAND_24}, {13, PHY_BAND_24},
    {36, PHY_BAND_5}, {40, PHY_BAND_5}, {44, PHY_BAND_5}, {48, PHY_BAND_5},
    {149, PHY_BAND_5}, {153, PHY_BAND_5}, {157, PHY_BAND_5}, {161, PHY_BAND_5},
};

static uint8_t iwl_mvm_phy_band_from_channel(uint8_t num)
{
    if (num >= 36 && num <= 196)
        return PHY_BAND_5;
    return PHY_BAND_24;
}

/* iwl-nvm-parse.c — orden de índices NVM → número de canal. */
static const uint8_t iwl_nvm_channels_legacy[] = {
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
    36, 40, 44, 48, 52, 56, 60, 64,
    100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144,
    149, 153, 157, 161, 165,
};

static const uint8_t iwl_ext_nvm_channels[] = {
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
    36, 40, 44, 48, 52, 56, 60, 64, 68, 72, 76, 80, 84, 88, 92,
    96, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144,
    149, 153, 157, 161, 165, 169, 173, 177, 181,
};

static const uint8_t iwl_uhb_nvm_channels[] = {
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
    36, 40, 44, 48, 52, 56, 60, 64, 68, 72, 76, 80, 84, 88, 92,
    96, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144,
    149, 153, 157, 161, 165, 169, 173, 177, 181,
    1, 5, 9, 13, 17, 21, 25, 29, 33, 37, 41, 45, 49, 53, 57, 61, 65, 69,
    73, 77, 81, 85, 89, 93, 97, 101, 105, 109, 113, 117, 121, 125, 129,
    133, 137, 141, 145, 149, 153, 157, 161, 165, 169, 173, 177, 181, 185,
    189, 193, 197, 201, 205, 209, 213, 217, 221, 225, 229, 233,
};

const uint8_t *iwl_mvm_nvm_chan_table(unsigned nvm_n, unsigned *table_n)
{
    if (table_n)
        *table_n = IWL_NVM_NUM_CHANNELS;
    if (nvm_n >= IWL_NVM_NUM_CHANNELS_UHB) {
        if (table_n)
            *table_n = IWL_NVM_NUM_CHANNELS_UHB;
        return iwl_uhb_nvm_channels;
    }
    if (nvm_n >= IWL_NVM_NUM_CHANNELS_EXT) {
        if (table_n)
            *table_n = IWL_NVM_NUM_CHANNELS_EXT;
        return iwl_ext_nvm_channels;
    }
    return iwl_nvm_channels_legacy;
}

uint8_t iwl_mvm_phy_band_from_channel_idx(unsigned ch_idx, unsigned nvm_n)
{
    unsigned table_n = 0;

    (void)iwl_mvm_nvm_chan_table(nvm_n, &table_n);
    if (table_n >= IWL_NVM_NUM_CHANNELS_UHB &&
        ch_idx >= NUM_2GHZ_CHANNELS + NUM_5GHZ_CHANNELS)
        return PHY_BAND_6;
    if (ch_idx >= NUM_2GHZ_CHANNELS)
        return PHY_BAND_5;
    return PHY_BAND_24;
}

static uint32_t iwl_read32(struct iwl_ax211_priv *iwl, uint32_t off)
{
    return iwl->mmio[off / 4];
}

/* Linux mvm.h + fw/img.c: ADD_STA se busca con groupid=0 → LONG_GROUP. */
static int iwl_mvm_has_new_station_api(struct iwl_ax211_priv *iwl)
{
    return iwl_fw_cmd_ver(iwl, LEGACY_GROUP, ADD_STA) >= 12;
}

static void iwl_mvm_scan_cfg_bcast(struct iwl_ax211_priv *iwl, uint8_t scan_cfg_ver,
                                   uint8_t *bcast_sta_id)
{
    if (!iwl_mvm_has_new_station_api(iwl)) {
        /* Sin aux STA en el port mínimo: 0xff como antes (no aplica a AX200). */
        *bcast_sta_id = 0xff;
    } else if (scan_cfg_ver < 5) {
        /* Deprecado en SCAN_CFG v5: 0xff si API nueva y versión antigua. */
        *bcast_sta_id = 0xff;
    }
    /* SCAN_CFG ≥ 5 + ADD_STA ≥ 12: memset deja bcast_sta_id=0 (Linux scan.c:1253). */
}

static void iwl_mvm_scan_umac_fill_general_v11(
    struct iwl_ax211_priv *iwl, struct iwl_scan_general_params_v11 *gp,
    uint8_t scan_ver, uint8_t flags2)
{
    gp->scan_start_mac_or_link_id = iwl->scan_mac_id;
    gp->adwell_default_social_chn = IWL_SCAN_ADWELL_DEFAULT_N_APS_SOCIAL;
    gp->adwell_default_2g = IWL_SCAN_ADWELL_DEFAULT_LB_N_APS;
    gp->adwell_default_5g = IWL_SCAN_ADWELL_DEFAULT_HB_N_APS;
    gp->adwell_max_budget = iwl_cpu_to_le16(IWL_SCAN_ADWELL_MAX_BUDGET_FULL_SCAN);
    gp->scan_priority = iwl_cpu_to_le32(IWL_SCAN_PRIORITY_EXT_6);
    gp->max_out_of_time[SCAN_LB_LMAC_IDX] = 0;
    gp->max_out_of_time[SCAN_HB_LMAC_IDX] = 0;
    gp->suspend_time[SCAN_LB_LMAC_IDX] = 0;
    gp->suspend_time[SCAN_HB_LMAC_IDX] = 0;
    gp->active_dwell[SCAN_LB_LMAC_IDX] = IWL_SCAN_DWELL_ACTIVE;
    gp->active_dwell[SCAN_HB_LMAC_IDX] = IWL_SCAN_DWELL_ACTIVE;
    gp->passive_dwell[SCAN_LB_LMAC_IDX] = IWL_SCAN_DWELL_PASSIVE;
    gp->passive_dwell[SCAN_HB_LMAC_IDX] = IWL_SCAN_DWELL_PASSIVE;
    if (scan_ver >= 15)
        gp->flags2 = flags2;
}

static int iwl_hw_rf_kill(struct iwl_ax211_priv *iwl)
{
    uint32_t gp = iwl_read32(iwl, CSR_GP_CNTRL);
    if (!(gp & CSR_GP_CNTRL_REG_FLAG_HW_RF_KILL_SW)) {
        lx_printk("iwl_mvm: RF-kill activo GP_CNTRL=0x%08x\n", gp);
        return -1;
    }
    return 0;
}

int iwl_mvm_send_scan_cfg(struct iwl_ax211_priv *iwl)
{
    uint8_t ver = (uint8_t)iwl_fw_cmd_ver(iwl, LONG_GROUP, SCAN_CFG_CMD);
    uint16_t pay_len;

    if (iwl->scan_cfg_sent)
        return 0;

    {
        uint8_t add_sta_ver = (uint8_t)iwl_fw_cmd_ver(iwl, LEGACY_GROUP, ADD_STA);
        uint8_t bcast = 0;

        if (ver >= 5) {
            /* Linux scan.c:1238 — API reducida, struct iwl_scan_config (12 B). */
            struct iwl_scan_config cfg;

            memset(&cfg, 0, sizeof(cfg));
            iwl_mvm_scan_cfg_bcast(iwl, ver, &cfg.bcast_sta_id);
            bcast = cfg.bcast_sta_id;
            cfg.tx_chains = iwl_cpu_to_le32(iwl_mvm_valid_tx_ant(iwl));
            cfg.rx_chains = iwl_cpu_to_le32(iwl_mvm_scan_rx_ant(iwl));
            pay_len = (uint16_t)sizeof(cfg);
            if (iwl_trans_send_cmd_wait(iwl, LONG_GROUP, SCAN_CFG_CMD, &cfg, pay_len,
                                        IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
                lx_printk("iwl_mvm: SCAN_CFG_CMD v%u falló\n", ver);
                return -1;
            }
        } else if (ver >= 2) {
            struct iwl_scan_config_v2 cfg;

            memset(&cfg, 0, sizeof(cfg));
            iwl_mvm_scan_cfg_bcast(iwl, ver, &cfg.bcast_sta_id);
            bcast = cfg.bcast_sta_id;
            cfg.tx_chains = iwl_cpu_to_le32(iwl_mvm_valid_tx_ant(iwl));
            cfg.rx_chains = iwl_cpu_to_le32(iwl_mvm_scan_rx_ant(iwl));
            pay_len = (uint16_t)sizeof(cfg);
            if (iwl_trans_send_cmd_wait(iwl, LONG_GROUP, SCAN_CFG_CMD, &cfg, pay_len,
                                        IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
                lx_printk("iwl_mvm: SCAN_CFG_CMD v%u falló\n", ver);
                return -1;
            }
        } else {
            struct iwl_scan_config cfg;

            memset(&cfg, 0, sizeof(cfg));
            iwl_mvm_scan_cfg_bcast(iwl, ver, &cfg.bcast_sta_id);
            bcast = cfg.bcast_sta_id;
            cfg.tx_chains = iwl_cpu_to_le32(iwl_mvm_valid_tx_ant(iwl));
            cfg.rx_chains = iwl_cpu_to_le32(iwl_mvm_scan_rx_ant(iwl));
            pay_len = (uint16_t)sizeof(cfg);
            if (iwl_trans_send_cmd_wait(iwl, LONG_GROUP, SCAN_CFG_CMD, &cfg, pay_len,
                                        IWL_MVM_HCMD_TIMEOUT_MS) != 0) {
                lx_printk("iwl_mvm: SCAN_CFG_CMD falló\n");
                return -1;
            }
        }

        iwl->scan_cfg_sent = 1;
        lx_printk("iwl_mvm: SCAN_CFG v%u ok bcast=%u add_sta_ver=%u tx=0x%x rx=0x%x\n",
                  ver, (unsigned)bcast, (unsigned)add_sta_ver,
                  (unsigned)iwl_mvm_valid_tx_ant(iwl),
                  (unsigned)iwl_mvm_scan_rx_ant(iwl));
        return 0;
    }
}

/* R6: única selección de canales para todas las versiones de SCAN_REQ_UMAC.
 *
 * Tres situaciones distintas que antes se confundían en «lista fija activa»:
 *   - sin perfil (NVM no leído)      → lista mínima conocida, solo pasiva
 *   - perfil válido con cero canales → nada que escanear (no scan activo)
 *   - perfil válido (NVM o MCC)      → filtrado, respetando activo/pasivo
 */
unsigned iwl_mvm_collect_scan_channels(struct iwl_ax211_priv *iwl,
                                       uint8_t *ch, uint8_t *band,
                                       uint8_t *passive, unsigned max,
                                       int *origen)
{
    unsigned n = 0;
    unsigned i;
    unsigned nvm_n = iwl->nvm_n_channels;
    unsigned table_n;
    const uint8_t *nvm_chan = iwl_mvm_nvm_chan_table(nvm_n, &table_n);
    int src = IWL_CHAN_SRC_NONE;

    if (origen)
        *origen = IWL_CHAN_SRC_NONE;
    if (!ch || !band || !passive || max == 0)
        return 0;

    /* LAR habilitado y regdominio sin aplicar: solo escucha. Linux rechaza el
     * scan entero (mvm/scan.c); aquí se degrada a pasivo, que está permitido
     * en cualquier dominio, en vez de dejar el WiFi sin nada. */
    unsigned solo_pasivo = (iwl->lar_enabled && !iwl->lar_regdom_set) ? 1u : 0u;

    if (iwl->chan_src == IWL_CHAN_SRC_MCC || iwl->chan_src == IWL_CHAN_SRC_NVM ||
        nvm_n > 0) {
        /* Perfil presente: puede quedarse en cero canales usables, y eso
         * es un resultado legítimo, no un motivo para inventar una lista. */
        src = (iwl->chan_src == IWL_CHAN_SRC_MCC) ? IWL_CHAN_SRC_MCC
                                                  : IWL_CHAN_SRC_NVM;
        if (nvm_n > table_n)
            nvm_n = table_n;
        for (i = 0; i < nvm_n && n < max; i++) {
            uint32_t flags = iwl->nvm_chan_flags[i];
            uint8_t num;

            if (!(flags & NVM_CHANNEL_VALID))
                continue;
            num = nvm_chan[i];
            ch[n] = num;
            band[n] = iwl_mvm_phy_band_from_channel_idx(i, nvm_n);
            /* Sin ACTIVE, con radar o solo interior: nada de probe request. */
            passive[n] = (solo_pasivo ||
                          !(flags & NVM_CHANNEL_ACTIVE) ||
                          (flags & NVM_CHANNEL_RADAR) ||
                          (flags & NVM_CHANNEL_INDOOR_ONLY))
                             ? 1
                             : 0;
            n++;
        }
        if (n == 0)
            src = IWL_CHAN_SRC_EMPTY;
        if (origen)
            *origen = src;
        return n;
    }

    /* Sin perfil regulatorio: solo escucha. Un scan activo sin regdominio no
     * está permitido (Linux directamente lo rechaza, ver mvm/scan.c). */
    {
        unsigned fallback = sizeof(scan_channels) / sizeof(scan_channels[0]);

        if (fallback > max)
            fallback = max;
        for (i = 0; i < fallback; i++) {
            ch[i] = scan_channels[i].ch;
            band[i] = scan_channels[i].band;
            passive[i] = 1;
        }
        n = fallback;
    }
    if (origen)
        *origen = IWL_CHAN_SRC_FALLBACK;
    return n;
}

int iwl_mvm_scan_umac_supported(uint8_t ver)
{
    return ver == 6 || ver == 14 || ver == 15 || ver == 16 || ver == 17;
}

/* Linux `iwl_mvm_scan_umac_flags_v2` (mvm/scan.c): descubrimiento sin SSIDs
 * directos → FORCE_PASSIVE aunque el NVM marque canales ACTIVE; ADAPTIVE_DWELL
 * siempre (IWL_MVM_ADWELL_ENABLE). */
static uint16_t iwl_mvm_scan_umac_flags_v2(struct iwl_ax211_priv *iwl,
                                           unsigned n_direct_ssids)
{
    uint16_t flags = (uint16_t)(IWL_UMAC_SCAN_GEN_FLAGS_V2_PASS_ALL |
                                IWL_UMAC_SCAN_GEN_FLAGS_V2_NTFY_ITER_COMPLETE |
                                IWL_UMAC_SCAN_GEN_FLAGS_V2_ADAPTIVE_DWELL);

    (void)iwl;
    if (n_direct_ssids == 0)
        flags |= IWL_UMAC_SCAN_GEN_FLAGS_V2_FORCE_PASSIVE;
    return flags;
}

static uint16_t iwl_build_scan_req_v17(struct iwl_ax211_priv *iwl, uint8_t *buf,
                                       unsigned cap, uint8_t scan_ver)
{
    struct iwl_scan_req_umac_v17 *req;
    unsigned nch;
    unsigned i;
    unsigned n_passive = 0;
    int origen = IWL_CHAN_SRC_NONE;
    uint8_t ch[SCAN_MAX_NUM_CHANS_V3];
    uint8_t band[SCAN_MAX_NUM_CHANS_V3];
    uint8_t passive[SCAN_MAX_NUM_CHANS_V3];

    if (cap < sizeof(*req))
        return 0;

    nch = iwl_mvm_collect_scan_channels(iwl, ch, band, passive,
                                        SCAN_MAX_NUM_CHANS_V3, &origen);
    if (nch == 0) {
        lx_printk("iwl_mvm: sin canales escaneables (origen=%d)\n", origen);
        return 0;
    }
    for (i = 0; i < nch; i++)
        n_passive += passive[i] ? 1u : 0u;

    memset(buf, 0, sizeof(*req));
    req = (struct iwl_scan_req_umac_v17 *)buf;
    iwl->scan_uid++;
    if (iwl->scan_uid == 0)
        iwl->scan_uid = 1;
    req->uid = iwl_cpu_to_le32(iwl->scan_uid);
    req->ooc_priority = iwl_cpu_to_le32(IWL_SCAN_PRIORITY_EXT_6);
    {
        uint16_t gflags = iwl_mvm_scan_umac_flags_v2(iwl, 0);

        req->scan_params.general_params.flags = gflags;
        iwl->scan_passive_only =
            (gflags & IWL_UMAC_SCAN_GEN_FLAGS_V2_FORCE_PASSIVE) ? 1u : 0u;
        (void)n_passive;
    }
    iwl_mvm_scan_umac_fill_general_v11(iwl, &req->scan_params.general_params, scan_ver, 0);
    req->scan_params.channel_params.flags = IWL_SCAN_CHANNEL_FLAG_ENABLE_CHAN_ORDER;
    req->scan_params.channel_params.n_aps_override[0] =
        IWL_SCAN_ADWELL_N_APS_GO_FRIENDLY;
    req->scan_params.channel_params.n_aps_override[1] =
        IWL_SCAN_ADWELL_N_APS_SOCIAL_CHS;
    req->scan_params.channel_params.count = (uint8_t)nch;
    for (i = 0; i < nch; i++) {
        struct iwl_scan_channel_cfg_umac *cfg =
            &req->scan_params.channel_params.channel_config[i];
        uint32_t flags = 0;

        cfg->v2.channel_num = ch[i];
        cfg->v2.iter_count = 1;
        /* v17 mueve la banda a flags[31:30] y reutiliza el byte de `band`
         * como psd_20 (`iwl_mvm_umac_scan_cfg_channels_v7`). */
        if (scan_ver >= 17)
            flags |= (uint32_t)band[i] << IWL_CHAN_CFG_FLAGS_BAND_POS;
        else
            cfg->v2.band = band[i];
        /* Pasivo global vía FORCE_PASSIVE en general_params; bit 26 solo 6 GHz
         * (Linux iwl_mvm_umac_scan_cfg_channels_v7, no v7_6g). */
        cfg->flags = flags;
        (void)passive;
    }
    req->scan_params.periodic_params.schedule[0].iter_count = 1;
    /* schedule[1] queda en cero: scan regular = un plan (Linux fill_scan_sched). */
    iwl_mvm_fill_probe_req(iwl, &req->scan_params.probe_params);
    return (uint16_t)sizeof(*req);
}

uint16_t iwl_mvm_build_scan_req(struct iwl_ax211_priv *iwl, uint8_t *buf, unsigned cap)
{
    uint8_t scan_ver = (uint8_t)iwl_fw_cmd_ver(iwl, LONG_GROUP, SCAN_REQ_UMAC);

    if (!iwl_mvm_scan_umac_supported(scan_ver))
        return 0;
    if (scan_ver >= 14)
        return iwl_build_scan_req_v17(iwl, buf, cap, scan_ver);

    /* v6 usa la misma política de canales: sin lista fija propia. */
    uint8_t chs[SCAN_MAX_NUM_CHANS_V3];
    uint8_t bands[SCAN_MAX_NUM_CHANS_V3];
    uint8_t passives[SCAN_MAX_NUM_CHANS_V3];
    int origen = IWL_CHAN_SRC_NONE;
    unsigned nch = iwl_mvm_collect_scan_channels(iwl, chs, bands, passives,
                                                 SCAN_MAX_NUM_CHANS_V3, &origen);
    unsigned n_passive = 0;
    unsigned pay;
    uint8_t *data;
    struct iwl_scan_req_umac_tail_v1 *tail;
    unsigned i;

    if (nch == 0) {
        lx_printk("iwl_mvm: v6 sin canales escaneables (origen=%d)\n", origen);
        return 0;
    }
    pay = IWL_SCAN_REQ_UMAC_SIZE_V6 + nch * sizeof(struct iwl_scan_channel_cfg_umac) +
          sizeof(struct iwl_scan_req_umac_tail_v1);
    if (pay > cap)
        return 0;
    for (i = 0; i < nch; i++)
        n_passive += passives[i] ? 1u : 0u;

    memset(buf, 0, pay);
    {
        uint16_t gflags = (uint16_t)(IWL_UMAC_SCAN_GEN_FLAGS_PASS_ALL |
                                     IWL_UMAC_SCAN_GEN_FLAGS_ITER_COMPLETE);

        if (n_passive == nch)
            gflags |= IWL_UMAC_SCAN_GEN_FLAGS_PASSIVE;
        *(uint16_t *)(buf + 12) = gflags;
        iwl->scan_passive_only = (n_passive == nch) ? 1u : 0u;
    }
    buf[17] = 30;
    buf[18] = 30;
    buf[19] = 10;
    buf[41] = (uint8_t)nch;

    data = buf + IWL_SCAN_REQ_UMAC_SIZE_V6;
    for (i = 0; i < nch; i++) {
        struct iwl_scan_channel_cfg_umac *cfg =
            (struct iwl_scan_channel_cfg_umac *)(data + i * sizeof(*cfg));
        cfg->v2.channel_num = chs[i];
        cfg->v2.band = bands[i];
        cfg->v2.iter_count = 1;
        cfg->v2.iter_interval = 0;
        if (passives[i])
            cfg->flags = IWL_UHB_CHAN_CFG_FLAG_FORCE_PASSIVE;
    }

    tail = (struct iwl_scan_req_umac_tail_v1 *)(data + nch * sizeof(struct iwl_scan_channel_cfg_umac));
    tail->schedule[0].interval = 0;
    tail->schedule[0].iter_count = 1;
    tail->delay = 0;

    (void)iwl;
    return (uint16_t)pay;
}

static int frame_is_beacon_or_probe_resp(const uint8_t *frame, int len)
{
    if (len < 2)
        return 0;
    uint16_t fc = (uint16_t)frame[0] | ((uint16_t)frame[1] << 8);
    uint8_t type = (uint8_t)((fc >> 2) & 3u);
    uint8_t subtype = (uint8_t)((fc >> 4) & 0xfu);
    return type == 0 && (subtype == 8 || subtype == 5);
}

/* R7: qué dice el beacon/probe response de este BSS.
 *
 * Antes todo lo que se veía se marcaba `open = 1`, así que una red WPA2 se
 * presentaba como abierta y `connect_open` se lanzaba contra ella. La
 * clasificación sale de dos sitios, como en `mac80211`:
 *   - el bit Privacy de las capacidades (offset 34 del cuerpo de gestión),
 *   - el RSN IE (id 48), del que además se leen AKM y cifrados.
 *
 * `iwl_mvm_parse_bss` se expone para poder comprobarla con tramas reales en el
 * banco del host, sin transporte ni GPU.
 */
int iwl_mvm_parse_bss(const uint8_t *frame, int len, struct iwl_ax211_bss *out)
{
    uint16_t caps;
    int pos;
    int ssid_len = -1;

    if (!frame || !out || len < 24 + 12) {
        return -1;
    }
    if (!frame_is_beacon_or_probe_resp(frame, len)) {
        return -1;
    }
    memset(out, 0, sizeof(*out));
    memcpy(out->bssid, frame + 16, 6);
    /* timestamp(8) + beacon interval(2) + capability(2) = 12 B tras la MAC. */
    caps = (uint16_t)frame[34] | ((uint16_t)frame[35] << 8);
    out->open = (caps & WLAN_CAPABILITY_PRIVACY) ? 0u : 1u;

    pos = 24 + 12;
    while (pos + 2 <= len) {
        uint8_t id = frame[pos];
        uint8_t elen = frame[pos + 1];
        const uint8_t *body = &frame[pos + 2];

        if (pos + 2 + (int)elen > len) {
            /* IE truncado: lo que ya se leyó vale, lo que falta no se inventa. */
            break;
        }
        if (id == WLAN_EID_SSID && elen > 0 && elen <= IWL_AX211_SSID_MAX) {
            memcpy(out->ssid, body, elen);
            out->ssid[elen] = '\0';
            ssid_len = (int)elen;
        } else if (id == WLAN_EID_DS_PARAMS && elen >= 1) {
            out->channel = body[0];
            out->band24 = 1u;
        } else if (id == WLAN_EID_RSN && elen >= 8) {
            /* version(2) group cipher(4) pairwise count(2) … */
            uint16_t ver = (uint16_t)body[0] | ((uint16_t)body[1] << 8);
            unsigned off = 2;
            unsigned n;
            unsigned i;
            int group_ccmp;
            int pair_ccmp = 0;
            int akm_psk = 0;

            if (ver != 1u) {
                pos += 2 + elen;
                continue;
            }
            out->rsn = 1u;
            out->open = 0u;
            group_ccmp = (body[off + 3] == WLAN_CIPHER_CCMP128) ? 1 : 0;
            off += 4;
            if (off + 2 > elen) {
                pos += 2 + elen;
                continue;
            }
            n = (unsigned)body[off] | ((unsigned)body[off + 1] << 8);
            off += 2;
            for (i = 0; i < n; i++) {
                if (off + 4 > elen) {
                    break;
                }
                if (body[off + 3] == WLAN_CIPHER_CCMP128) {
                    pair_ccmp = 1;
                }
                off += 4;
            }
            if (off + 2 <= elen) {
                n = (unsigned)body[off] | ((unsigned)body[off + 1] << 8);
                off += 2;
                for (i = 0; i < n; i++) {
                    if (off + 4 > elen) {
                        break;
                    }
                    if (body[off + 3] == WLAN_AKM_PSK ||
                        body[off + 3] == WLAN_AKM_PSK_SHA256) {
                        akm_psk = 1;
                    }
                    off += 4;
                }
            }
            out->akm_psk = akm_psk ? 1u : 0u;
            out->ccmp = (group_ccmp && pair_ccmp) ? 1u : 0u;
        }
        pos += 2 + elen;
    }
    return ssid_len;
}

void iwl_mvm_rx_scan_frame(struct iwl_ax211_priv *iwl, const uint8_t *frame, int len)
{
    struct iwl_ax211_bss bss;
    int slen;
    int i;

    if (!iwl->scan_active || len <= 0) {
        return;
    }
    slen = iwl_mvm_parse_bss(frame, len, &bss);
    if (slen <= 0) {
        return;
    }
    for (i = 0; i < iwl->scan_count; i++) {
        if (!memcmp(iwl->scan[i].bssid, bss.bssid, 6) &&
            !strncmp(iwl->scan[i].ssid, bss.ssid, IWL_AX211_SSID_MAX)) {
            return;
        }
    }
    bss.rssi = iwl->last_rx_rssi ? iwl->last_rx_rssi : -70;
    /* El canal del DS Params manda; si no venía, el del RX. */
    if (!bss.channel) {
        bss.channel = iwl->last_rx_channel;
    }
    iwl_ax211_add_bss(&bss);
}

void iwl_mvm_on_scan_complete(struct iwl_ax211_priv *iwl, uint32_t uid, uint8_t status)
{
    if (!iwl->scan_active)
        return;
    if (uid != iwl->scan_uid)
        return;
    if (status == IWL_SCAN_OFFLOAD_COMPLETED)
        iwl->scan_end = IWL_SCAN_END_NORMAL;
    else if (status == IWL_SCAN_OFFLOAD_ABORTED)
        iwl->scan_end = IWL_SCAN_END_ABORTED;
    else
        iwl->scan_end = IWL_SCAN_END_ABORTED;
    iwl->scan_complete = 1;
    iwl->scan_active = 0;
}

int iwl_mvm_scan(struct iwl_ax211_priv *iwl)
{
    uint8_t req[IWL_CMD_SLOT_SIZE];
    uint16_t pay;
    int last_count = 0;

    if (!iwl->radio_ready) {
        lx_printk("iwl_mvm: scan sin INIT_COMPLETE\n");
        return -1;
    }
    if (!iwl->mvm_up_done && iwl_mvm_up_minimal(iwl) != 0) {
        lx_printk("iwl_mvm: scan sin up MVM\n");
        return -1;
    }
    if (iwl_hw_rf_kill(iwl) != 0)
        return -1;

    if (iwl->scan_active) {
        iwl->scan_end = IWL_SCAN_END_ABORTED;
        iwl->scan_active = 0;
        lx_printk("iwl_mvm: scan previo abortado uid=0x%x\n",
                  (unsigned)iwl->scan_uid);
    }

    iwl->scan_count = 0;
    iwl->scan_complete = 0;
    iwl->scan_end = IWL_SCAN_END_NONE;
    iwl->scan_active = 1;

    if (iwl_mvm_send_scan_cfg(iwl) != 0) {
        iwl->scan_active = 0;
        return -1;
    }

    pay = iwl_mvm_build_scan_req(iwl, req, sizeof(req));
    if (pay == 0) {
        lx_printk("iwl_mvm: SCAN_REQ versión no soportada, sin canales o no cabe\n");
        iwl->scan_active = 0;
        return IWL_SCAN_RC_NO_CHANNELS;
    }

    /* LAR sin regdominio aplicado: Linux rechaza el scan (mvm/scan.c). Aquí
     * se deja continuar solo si es pasivo, que sí está permitido en cualquier
     * dominio; así el diagnóstico distingue «sin MCC» de «sin radio». */
    if (iwl->lar_enabled && !iwl->lar_regdom_set && !iwl->scan_passive_only) {
        lx_printk("iwl_mvm: LAR sin regdominio aplicado; scan activo no permitido\n");
        iwl->scan_active = 0;
        return IWL_SCAN_RC_NO_REGDOM;
    }

    {
        uint8_t scan_ver = (uint8_t)iwl_fw_cmd_ver(iwl, LONG_GROUP, SCAN_REQ_UMAC);
        uint8_t sch[SCAN_MAX_NUM_CHANS_V3];
        uint8_t sb[SCAN_MAX_NUM_CHANS_V3];
        uint8_t sp[SCAN_MAX_NUM_CHANS_V3];
        int sorig = IWL_CHAN_SRC_NONE;
        unsigned snch = iwl_mvm_collect_scan_channels(iwl, sch, sb, sp,
                                                      SCAN_MAX_NUM_CHANS_V3, &sorig);

        lx_printk("iwl_mvm: SCAN_REQ_UMAC v%u %u B uid=0x%x origen=%u pasivo=%u nch=%u ch=%u..%u\n",
                  scan_ver, pay, (unsigned)iwl->scan_uid, (unsigned)iwl->chan_src,
                  (unsigned)iwl->scan_passive_only, snch,
                  snch ? (unsigned)sch[0] : 0u,
                  snch ? (unsigned)sch[snch - 1] : 0u);
    }

    if (iwl_trans_send_cmd_wait(iwl, LONG_GROUP, SCAN_REQ_UMAC, req, pay, 1000) != 0) {
        lx_printk("iwl_mvm: SCAN_REQ_UMAC rechazado\n");
        iwl->scan_active = 0;
        iwl->scan_end = IWL_SCAN_END_NONE;
        return -1;
    }

    for (int i = 0; i < 400; i++) {
        iwl_trans_poll(iwl);
        if (iwl->scan_count != last_count) {
            lx_printk("iwl_mvm: scan_count=%d\n", iwl->scan_count);
            last_count = iwl->scan_count;
        }
        if (iwl->scan_end != IWL_SCAN_END_NONE)
            break;
        lx_mdelay(20);
    }

    if (iwl->scan_end == IWL_SCAN_END_NONE) {
        iwl->scan_end = IWL_SCAN_END_TIMEOUT;
        iwl->scan_active = 0;
    }

    lx_printk("iwl_mvm: scan fin count=%d end=%d uid=0x%x GP=0x%08x\n",
              iwl->scan_count, iwl->scan_end, (unsigned)iwl->scan_uid,
              iwl_read32(iwl, CSR_GP_CNTRL));

    if (iwl->scan_end == IWL_SCAN_END_NORMAL)
        return 0;
    if (iwl->scan_end == IWL_SCAN_END_ABORTED)
        return IWL_SCAN_RC_ABORTED;
    return IWL_SCAN_RC_TIMEOUT;
}

static int iwl_mvm_assoc(struct iwl_ax211_priv *iwl, const char *ssid, const uint8_t *bssid)
{
    if (!bssid)
        return -1;
    return iwl_mvm_assoc_prepare(iwl, ssid, bssid);
}

/* El BSS con ese SSID exacto y mejor señal. NULL si no está: conectarse «al
 * primero que haya» es asociarse a una red que nadie pidió (R7). */
const struct iwl_ax211_bss *iwl_mvm_pick_bss(struct iwl_ax211_priv *iwl,
                                             const char *ssid)
{
    const struct iwl_ax211_bss *pick = 0;
    int i;

    if (!iwl || !ssid || !ssid[0]) {
        return 0;
    }
    for (i = 0; i < iwl->scan_count; i++) {
        const struct iwl_ax211_bss *b = &iwl->scan[i];

        if (strncmp(b->ssid, ssid, IWL_AX211_SSID_MAX) != 0) {
            continue;
        }
        /* Mismo SSID en varios BSS (repetidores, 2,4 y 5 GHz): el de más
         * señal, no el primero que aparezca en el array. */
        if (!pick || b->rssi > pick->rssi) {
            pick = b;
        }
    }
    return pick;
}

int iwl_mvm_connect_open(struct iwl_ax211_priv *iwl, const char *ssid)
{
    const struct iwl_ax211_bss *pick;

    if (!iwl->alive || !ssid) {
        return -1;
    }
    pick = iwl_mvm_pick_bss(iwl, ssid);
    if (!pick) {
        lx_printk("iwl_mvm: '%s' no está entre los %d BSS del scan\n",
                  ssid ? ssid : "", iwl->scan_count);
        return -1;
    }
    if (!pick->open) {
        lx_printk("iwl_mvm: '%s' está protegida (rsn=%u akm_psk=%u ccmp=%u); "
                  "usa la conexión WPA2\n", ssid, pick->rsn, pick->akm_psk,
                  pick->ccmp);
        return -1;
    }
    iwl->channel = pick->channel;
    return iwl_mvm_assoc(iwl, ssid, pick->bssid);
}

int iwl_mvm_connect_wpa2(struct iwl_ax211_priv *iwl, const char *ssid,
                         const uint8_t psk[32])
{
    const struct iwl_ax211_bss *pick;

    if (!iwl->alive || !ssid || !psk) {
        return -1;
    }
    pick = iwl_mvm_pick_bss(iwl, ssid);
    if (!pick) {
        lx_printk("iwl_mvm: '%s' no está entre los %d BSS del scan\n", ssid,
                  iwl->scan_count);
        return -1;
    }
    /* R8 pendiente: aquí falta el 4-way con el supplicant. Lo que sí se puede
     * decidir ya es si esta red admite WPA2-PSK con CCMP; si no, no hay nada
     * que intentar y decirlo es mejor que caer a la ruta abierta. */
    if (!pick->rsn || !pick->akm_psk || !pick->ccmp) {
        lx_printk("iwl_mvm: '%s' no ofrece WPA2-PSK/CCMP (rsn=%u akm_psk=%u "
                  "ccmp=%u)\n", ssid, pick->rsn, pick->akm_psk, pick->ccmp);
        return -1;
    }
    iwl->channel = pick->channel;
    lx_printk("iwl_mvm: '%s' ofrece WPA2-PSK/CCMP en el canal %u; falta el "
              "4-way (R8)\n", ssid, pick->channel);
    return iwl_mvm_assoc(iwl, ssid, pick->bssid);
}

int iwl_mvm_tx_8023(struct iwl_ax211_priv *iwl, const uint8_t *buf, int len)
{
    if (!iwl->associated || !buf || len <= 0)
        return -1;
    return iwl_trans_send_cmd(iwl, DATA_PATH_GROUP, TX_CMD, buf, (uint16_t)len);
}

int iwl_mvm_rx_8023(struct iwl_ax211_priv *iwl, uint8_t *buf, int buflen)
{
    if (!buf || buflen <= 0 || iwl->rxq_head == iwl->rxq_tail)
        return 0;
    int idx = iwl->rxq_tail % 8;
    int len = (int)iwl->rxq[idx][0] | ((int)iwl->rxq[idx][1] << 8);
    if (len <= 0 || len > 2040)
        return 0;
    if (len > buflen)
        len = buflen;
    memcpy(buf, &iwl->rxq[idx][2], (size_t)len);
    iwl->rxq_tail = (iwl->rxq_tail + 1) % 8;
    return len;
}
