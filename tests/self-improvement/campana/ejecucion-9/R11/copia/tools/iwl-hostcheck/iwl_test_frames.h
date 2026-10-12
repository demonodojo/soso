/* Constructores de tramas para los bancos de host de iwlwifi.
 *
 * Compartido entre `rx_datapath_test.c` (contratos de formato) y
 * `ring_soak_test.c` (anillos bajo carga): las dos necesitan fabricar MPDUs de
 * datos y paquetes `REPLY_RX_MPDU_CMD` con el descriptor que le toca a cada
 * generación, y tener dos copias era garantizar que se separaran.
 */
#ifndef IWL_TEST_FRAMES_H
#define IWL_TEST_FRAMES_H

#include <stdint.h>
#include <string.h>

#include "iwl_internal.h"

/* MPDU 802.11 de datos FromDS con LLC/SNAP. Devuelve la longitud.
 * `f` debe admitir 40 + `plen` bytes. */
static inline unsigned iwl_test_data_mpdu(uint8_t *f, uint16_t ethertype,
                                          const uint8_t *da, const uint8_t *bssid,
                                          const uint8_t *sa, int protegida,
                                          const uint8_t *payload, unsigned plen)
{
    unsigned off = 24;

    memset(f, 0, 24);
    f[0] = 0x08;
    f[1] = 0x02; /* FromDS */
    if (protegida)
        f[1] |= 0x40;
    memcpy(f + 4, da, 6);
    memcpy(f + 10, bssid, 6);
    memcpy(f + 16, sa, 6);
    if (protegida) {
        memset(f + off, 0, 8); /* cabecera CCMP; el FW ya descifró */
        off += 8;
    }
    f[off++] = 0xaa;
    f[off++] = 0xaa;
    f[off++] = 0x03;
    f[off++] = 0x00;
    f[off++] = 0x00;
    f[off++] = 0x00;
    f[off++] = (uint8_t)(ethertype >> 8);
    f[off++] = (uint8_t)(ethertype & 0xff);
    if (plen) {
        memcpy(f + off, payload, plen);
        off += plen;
    }
    return off;
}

/* Envuelve una MPDU en un paquete `REPLY_RX_MPDU_CMD` con el descriptor de la
 * generación pedida. Devuelve los bytes escritos (cabecera de 8 B incluida). */
static inline unsigned iwl_test_rx_packet(uint8_t *pkt, int gen3, const uint8_t *frame,
                                          unsigned flen, uint32_t status)
{
    unsigned desc = gen3 ? IWL_RX_DESC_SIZE_V3 : IWL_RX_DESC_SIZE_V1;
    unsigned pay = desc + flen;
    struct iwl_rx_mpdu_desc *d;

    memset(pkt, 0, 8 + pay);
    pkt[0] = (uint8_t)((pay + 4) & 0xff);
    pkt[1] = (uint8_t)(((pay + 4) >> 8) & 0x3f);
    pkt[4] = REPLY_RX_MPDU_CMD;
    pkt[5] = LEGACY_GROUP;
    d = (struct iwl_rx_mpdu_desc *)(pkt + 8);
    d->mpdu_len = (uint16_t)flen;
    d->status = status;
    memcpy(pkt + 8 + desc, frame, flen);
    return 8 + pay;
}

/* Respuesta TX_CMD del firmware para el TFD `idx` de la cola `qid`.
 * El `agg_tx_status` va en un offset distinto por generación (36 en gen2, 40
 * en gen3), así que el cuerpo mide 48 B para que quepan los dos. */
static inline unsigned iwl_test_tx_resp(uint8_t *pkt, int gen3, uint16_t qid,
                                        unsigned idx, uint8_t status)
{
    unsigned pay = 48;
    unsigned off = gen3 ? IWL_MVM_TX_RESP_STATUS_OFF : IWL_MVM_TX_RESP_V3_STATUS_OFF;
    uint16_t seq = (uint16_t)(QUEUE_TO_SEQ(qid) | INDEX_TO_SEQ(idx));

    memset(pkt, 0, 8 + pay);
    pkt[0] = (uint8_t)((pay + 4) & 0xff);
    pkt[1] = (uint8_t)(((pay + 4) >> 8) & 0x3f);
    pkt[4] = TX_CMD;
    pkt[5] = LEGACY_GROUP;
    pkt[6] = (uint8_t)(seq & 0xff);
    pkt[7] = (uint8_t)(seq >> 8);
    pkt[8] = 1; /* frame_count */
    pkt[8 + off] = status;
    return 8 + pay;
}

/* Palabras del stub de MMIO. Tiene que cubrir el registro de mayor offset que
 * toque el transporte: `RFH_Q0_FRBDCB_WIDX_TRG` está en 0x1C80, o sea la
 * palabra 0x720. Un stub de 0x500 se desborda en cada drenaje RX, y el destrozo
 * cae en el global de al lado sin que se note hasta que ASan lo pilla. */
#define IWL_TEST_MMIO_WORDS 0x2000

/* xorshift32: un PRNG reproducible, para que un fallo del soak se repita con
 * la misma semilla en vez de desaparecer. */
static inline uint32_t iwl_test_rand(uint32_t *s)
{
    uint32_t x = *s;

    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    return x;
}

#endif
