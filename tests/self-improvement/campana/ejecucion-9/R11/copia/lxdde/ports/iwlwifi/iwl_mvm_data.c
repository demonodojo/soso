/* Conversión 802.11 ↔ Ethernet del camino de datos.
 *
 * Aislado del transporte a propósito: son funciones puras sobre buffers, así
 * que el banco del host (tools/iwl-hostcheck/rx_datapath_test.c) las ejecuta
 * con tramas reales sin montar ni un anillo DMA.
 *
 * Referencia: net/mac80211/rx.c `ieee80211_data_to_8023_exthdr()` y
 * net/mac80211/tx.c `ieee80211_build_hdr()` de Linux 6.6.
 */

#include "lx_emul.h"
#include "iwl_internal.h"
#include "iwl_ax211.h"

extern void *memcpy(void *dst, const void *src, unsigned long n);
extern void *memset(void *dst, int c, unsigned long n);
extern int memcmp(const void *a, const void *b, unsigned long n);

#define FC_TYPE_MASK        0x000cu
#define FC_TYPE_DATA        0x0008u
#define FC_SUBTYPE_MASK     0x00f0u
#define FC_SUBTYPE_NODATA   0x0040u /* Null / QoS-Null: sin cuerpo */
#define FC_SUBTYPE_QOS      0x0080u
#define FC_TODS             0x0100u
#define FC_FROMDS           0x0200u
#define FC_PROTECTED        0x4000u
#define FC_ORDER            0x8000u

#define IEEE80211_HDR_LEN   24
#define IEEE80211_QOS_LEN   2
#define IEEE80211_HT_CTL_LEN 4
#define IEEE80211_CCMP_HDR_LEN 8
#define LLC_SNAP_LEN        8
#define ETH_HDR_LEN         14

static const uint8_t rfc1042_hdr[6] = { 0xaa, 0xaa, 0x03, 0x00, 0x00, 0x00 };
static const uint8_t bridge_tunnel_hdr[6] = { 0xaa, 0xaa, 0x03, 0x00, 0x00, 0xf8 };

static uint16_t fc_of(const uint8_t *f)
{
    return (uint16_t)f[0] | ((uint16_t)f[1] << 8);
}

static int mac_is_group(const uint8_t *a)
{
    return (a[0] & 1) != 0;
}

/* Longitud real de la cabecera 802.11, con QoS y HT Control. */
int iwl_80211_hdrlen(uint16_t fc)
{
    int len = IEEE80211_HDR_LEN;

    if ((fc & (FC_TODS | FC_FROMDS)) == (FC_TODS | FC_FROMDS))
        len += 6; /* addr4 */
    if ((fc & FC_TYPE_MASK) == FC_TYPE_DATA) {
        if (fc & FC_SUBTYPE_QOS)
            len += IEEE80211_QOS_LEN;
        if (fc & FC_ORDER)
            len += IEEE80211_HT_CTL_LEN;
    }
    return len;
}

/* Comprueba lo que el firmware dice del descifrado de una MPDU protegida.
 * 0 = utilizable; distinto de 0 = descartar. */
int iwl_rx_crypto_ok(uint32_t status)
{
    if ((status & IWL_RX_MPDU_STATUS_SEC_MASK) != IWL_RX_MPDU_STATUS_SEC_CCM)
        return -1;
    if (!(status & IWL_RX_MPDU_STATUS_DECRYPTED))
        return -1;
    if (!(status & IWL_RX_MPDU_STATUS_MIC_OK))
        return -1;
    return 0;
}

/* MPDU 802.11 de datos → trama Ethernet.
 *
 * `status` es el DW5 del descriptor de recepción. Devuelve los bytes escritos
 * en `out`, o un valor negativo con el motivo del descarte:
 *   -1 no es una trama de datos entregable
 *   -2 direcciones que no son de este BSS o no van a esta estación
 *   -3 problema de cifrado (sin proteger con claves puestas, o mal descifrada)
 *   -4 encapsulado LLC/SNAP ausente o truncado
 *   -5 no cabe en `out`
 */
int iwl_mvm_rx_to_eth(const struct iwl_ax211_priv *iwl, const uint8_t *frame, int flen,
                      uint32_t status, uint8_t *out, int outmax)
{
    uint16_t fc;
    int hdrlen;
    int off;
    const uint8_t *da;
    const uint8_t *sa;
    const uint8_t *bssid;
    const uint8_t *snap;
    int payload;

    if (!iwl || !frame || !out || flen < IEEE80211_HDR_LEN)
        return -1;
    fc = fc_of(frame);
    if ((fc & FC_TYPE_MASK) != FC_TYPE_DATA)
        return -1;
    if (fc & FC_SUBTYPE_NODATA)
        return -1;
    /* Estación en modo infraestructura: sólo llegan tramas del AP (FromDS).
     * Las de 4 direcciones y las ToDS son de otra topología. */
    if ((fc & (FC_TODS | FC_FROMDS)) != FC_FROMDS)
        return -1;

    hdrlen = iwl_80211_hdrlen(fc);
    if (flen < hdrlen)
        return -1;
    da = frame + 4;
    bssid = frame + 10;
    sa = frame + 16;
    if (memcmp(bssid, iwl->bssid, 6) != 0)
        return -2;
    if (!mac_is_group(da) && memcmp(da, iwl->mac, 6) != 0)
        return -2;

    off = hdrlen;
    if (fc & FC_PROTECTED) {
        if (iwl_rx_crypto_ok(status) != 0)
            return -3;
        /* El firmware quita el MIC pero deja la cabecera CCMP en la trama. */
        off += IEEE80211_CCMP_HDR_LEN;
    } else if (iwl->keys_installed) {
        /* Con claves instaladas, una trama de datos en claro no es del AP. */
        return -3;
    }
    if (flen < off + LLC_SNAP_LEN)
        return -4;
    snap = frame + off;
    if (memcmp(snap, rfc1042_hdr, 6) != 0 && memcmp(snap, bridge_tunnel_hdr, 6) != 0)
        return -4;
    off += LLC_SNAP_LEN;
    payload = flen - off;
    if (payload < 0)
        return -4;
    if (outmax < ETH_HDR_LEN + payload)
        return -5;

    memcpy(out, da, 6);
    memcpy(out + 6, sa, 6);
    out[12] = snap[6];
    out[13] = snap[7];
    if (payload)
        memcpy(out + ETH_HDR_LEN, frame + off, (size_t)payload);
    return ETH_HDR_LEN + payload;
}

/* Trama Ethernet → MPDU 802.11 de datos ToDS.
 *
 * addr1 = BSSID, addr2 = esta estación, addr3 = **destino Ethernet real**.
 * Fijar addr3 al BSSID perdía el destino y el AP no podía reenviar.
 * El cuerpo va encapsulado en LLC/SNAP: copiar la trama Ethernet entera detrás
 * de la cabecera 802.11 mete los 14 bytes de 802.3 dentro de la carga.
 *
 * Devuelve los bytes escritos o un valor negativo.
 */
int iwl_mvm_eth_to_80211(const struct iwl_ax211_priv *iwl, const uint8_t *eth, int len,
                         uint8_t *out, int outmax)
{
    int payload;
    int total;

    if (!iwl || !eth || !out || len < ETH_HDR_LEN)
        return -1;
    payload = len - ETH_HDR_LEN;
    total = IEEE80211_HDR_LEN + LLC_SNAP_LEN + payload;
    if (total > outmax)
        return -5;

    memset(out, 0, IEEE80211_HDR_LEN);
    out[0] = 0x08;           /* type = datos, subtipo = datos */
    out[1] = 0x01;           /* ToDS: de la estación al AP */
    memcpy(out + 4, iwl->bssid, 6);
    memcpy(out + 10, iwl->mac, 6);
    memcpy(out + 16, eth, 6);
    /* Control de secuencia a cero: lo rellena el firmware. */
    memcpy(out + IEEE80211_HDR_LEN, rfc1042_hdr, 6);
    out[IEEE80211_HDR_LEN + 6] = eth[12];
    out[IEEE80211_HDR_LEN + 7] = eth[13];
    if (payload)
        memcpy(out + IEEE80211_HDR_LEN + LLC_SNAP_LEN, eth + ETH_HDR_LEN,
               (size_t)payload);
    return total;
}
