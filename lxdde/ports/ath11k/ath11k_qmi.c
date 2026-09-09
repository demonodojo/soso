/* QMI (WLFW) de ath11k — codificación de los mensajes de arranque.
 *
 * Fase W2 del plan Steam Deck. Aquí sólo está el **formato del cable**: la
 * cabecera QMI, los TLV y los mensajes de la secuencia inicial. El transporte
 * (canal IPCR sobre MHI) y la máquina de estados llegan cuando W1 esté
 * confirmado en placa; separarlo permite comprobar byte a byte lo que se va a
 * enviar sin depender del dispositivo.
 *
 * Referencias:
 *   include/linux/soc/qcom/qmi.h            — `struct qmi_header` (7 bytes)
 *   drivers/soc/qcom/qmi_encdec.c           — TLV y `qmi_response_type_v01`
 *   drivers/net/wireless/ath/ath11k/qmi.c   — mensajes WLFW y sus tipos TLV
 *   drivers/net/wireless/ath/ath11k/qmi.h   — IDs de mensaje
 *
 * Los tipos TLV de cada campo salen de los `qmi_elem_info` del driver, no de
 * la documentación: son el contrato real con el firmware.
 */
#include "lx_emul.h"
#include "ath11k_qmi.h"

/* ---- Escritura ---- */

void ath11k_qmi_init(struct qmi_writer *w, unsigned char *buf, unsigned int cap)
{
    w->buf = buf;
    w->cap = cap;
    w->len = QMI_HDR_LEN; /* la cabecera se rellena al cerrar */
    w->error = 0;
    if (cap < QMI_HDR_LEN)
        w->error = 1;
}

static unsigned char *reservar(struct qmi_writer *w, unsigned int n)
{
    if (w->error || w->len + n > w->cap) {
        w->error = 1;
        return 0;
    }
    unsigned char *p = w->buf + w->len;
    w->len += n;
    return p;
}

/* Un TLV: tipo (1) + longitud (2, little endian) + valor. */
static unsigned char *tlv(struct qmi_writer *w, unsigned char type, unsigned int len)
{
    unsigned char *p = reservar(w, 3 + len);
    if (!p)
        return 0;
    p[0] = type;
    p[1] = (unsigned char)(len & 0xff);
    p[2] = (unsigned char)((len >> 8) & 0xff);
    return p + 3;
}

void ath11k_qmi_put_u8(struct qmi_writer *w, unsigned char type, unsigned char v)
{
    unsigned char *p = tlv(w, type, 1);
    if (p)
        p[0] = v;
}

void ath11k_qmi_put_u32(struct qmi_writer *w, unsigned char type, uint32_t v)
{
    unsigned char *p = tlv(w, type, 4);
    if (!p)
        return;
    p[0] = (unsigned char)(v & 0xff);
    p[1] = (unsigned char)((v >> 8) & 0xff);
    p[2] = (unsigned char)((v >> 16) & 0xff);
    p[3] = (unsigned char)((v >> 24) & 0xff);
}

void ath11k_qmi_put_u64(struct qmi_writer *w, unsigned char type, uint64_t v)
{
    unsigned char *p = tlv(w, type, 8);
    if (!p)
        return;
    for (int i = 0; i < 8; i++)
        p[i] = (unsigned char)((v >> (8 * i)) & 0xff);
}

void ath11k_qmi_put_bytes(struct qmi_writer *w, unsigned char type,
                          const unsigned char *datos, unsigned int n)
{
    unsigned char *p = tlv(w, type, n);
    if (!p)
        return;
    for (unsigned int i = 0; i < n; i++)
        p[i] = datos[i];
}

int ath11k_qmi_finish(struct qmi_writer *w, unsigned char msg_type, uint16_t txn,
                      uint16_t msg_id)
{
    if (w->error)
        return -1;
    unsigned int payload = w->len - QMI_HDR_LEN;
    if (payload > 0xffff)
        return -1;
    w->buf[0] = msg_type;
    w->buf[1] = (unsigned char)(txn & 0xff);
    w->buf[2] = (unsigned char)((txn >> 8) & 0xff);
    w->buf[3] = (unsigned char)(msg_id & 0xff);
    w->buf[4] = (unsigned char)((msg_id >> 8) & 0xff);
    w->buf[5] = (unsigned char)(payload & 0xff);
    w->buf[6] = (unsigned char)((payload >> 8) & 0xff);
    return (int)w->len;
}

/* ---- Lectura ---- */

int ath11k_qmi_parse_hdr(const unsigned char *buf, unsigned int len,
                         struct qmi_msg_hdr *out)
{
    if (len < QMI_HDR_LEN)
        return -1;
    out->type = buf[0];
    out->txn = (uint16_t)(buf[1] | (buf[2] << 8));
    out->msg_id = (uint16_t)(buf[3] | (buf[4] << 8));
    out->msg_len = (uint16_t)(buf[5] | (buf[6] << 8));
    /* Un mensaje que dice medir más de lo recibido es basura: aceptarlo
     * llevaría a leer fuera del búfer al buscar los TLV. */
    if ((unsigned int)out->msg_len + QMI_HDR_LEN > len)
        return -1;
    return 0;
}

const unsigned char *ath11k_qmi_find_tlv(const unsigned char *buf, unsigned int len,
                                         unsigned char type, unsigned int *len_out)
{
    struct qmi_msg_hdr h;
    if (ath11k_qmi_parse_hdr(buf, len, &h))
        return 0;
    unsigned int off = QMI_HDR_LEN;
    unsigned int fin = QMI_HDR_LEN + h.msg_len;
    while (off + 3 <= fin) {
        unsigned char t = buf[off];
        unsigned int l = (unsigned int)(buf[off + 1] | (buf[off + 2] << 8));
        if (off + 3 + l > fin)
            return 0;
        if (t == type) {
            if (len_out)
                *len_out = l;
            return buf + off + 3;
        }
        off += 3 + l;
    }
    return 0;
}

/* El TLV 0x02 de toda respuesta QMI: `qmi_response_type_v01`, dos u16
 * (resultado y error). result != 0 significa fallo, y `error` dice cuál. */
int ath11k_qmi_parse_result(const unsigned char *buf, unsigned int len,
                            uint16_t *result, uint16_t *error)
{
    unsigned int l = 0;
    const unsigned char *v = ath11k_qmi_find_tlv(buf, len, QMI_TLV_RESULT, &l);
    if (!v || l < 4)
        return -1;
    if (result)
        *result = (uint16_t)(v[0] | (v[1] << 8));
    if (error)
        *error = (uint16_t)(v[2] | (v[3] << 8));
    return 0;
}

/* ---- Mensajes de la secuencia de arranque ---- */

/* IND_REGISTER: dice al firmware qué indicaciones queremos recibir.
 * Campos y tipos TLV de `ath11k_qmi_fw_ind_register_send` (qmi.c:1790). */
int ath11k_qmi_build_ind_register(unsigned char *buf, unsigned int cap, uint16_t txn)
{
    struct qmi_writer w;
    ath11k_qmi_init(&w, buf, cap);
    /* En orden ascendente de tipo, como los emite el codificador de Linux
     * recorriendo el `qmi_elem_info`. */
    ath11k_qmi_put_u8(&w, 0x10, 1);                    /* fw_ready_enable */
    ath11k_qmi_put_u32(&w, 0x15, QMI_WLANFW_CLIENT_ID); /* client_id */
    ath11k_qmi_put_u8(&w, 0x16, 1);                    /* request_mem_enable */
    ath11k_qmi_put_u8(&w, 0x17, 1);                    /* fw_mem_ready_enable */
    ath11k_qmi_put_u8(&w, 0x18, 1);                    /* fw_init_done_enable */
    ath11k_qmi_put_u8(&w, 0x1b, 1);                    /* cal_done_enable */
    return ath11k_qmi_finish(&w, QMI_REQUEST, txn, QMI_WLANFW_IND_REGISTER_REQ_V01);
}

/* HOST_CAP: lo que el host puede hacer. Los valores son los que pone
 * `ath11k_qmi_host_cap_send` (qmi.c:1707) para un chip con `m3_fw_support`,
 * `internal_sleep_clock` y `global_reset` — los tres, en WCN6855 hw2.1
 * (core.c:462). */
int ath11k_qmi_build_host_cap(unsigned char *buf, unsigned int cap,
                              uint16_t txn, unsigned char mem_cfg_mode,
                              unsigned char cal_done)
{
    struct qmi_writer w;
    ath11k_qmi_init(&w, buf, cap);
    ath11k_qmi_put_u32(&w, 0x10, 1); /* num_clients */
    /* nm_modem: avisa de que la plataforma no es de Qualcomm, que el reloj de
     * sueño es el interno, y de las capacidades PCIe. */
    ath11k_qmi_put_u8(&w, 0x13,
                      HOST_CSTATE_BIT | SLEEP_CLOCK_SELECT_INTERNAL_BIT |
                          PLATFORM_CAP_PCIE_GLOBAL_RESET |
                          PLATFORM_CAP_PCIE_PME_D3COLD);
    ath11k_qmi_put_u8(&w, 0x14, 1); /* bdf_support */
    ath11k_qmi_put_u8(&w, 0x16, 1); /* m3_support */
    ath11k_qmi_put_u8(&w, 0x17, 1); /* m3_cache_support */
    ath11k_qmi_put_u8(&w, 0x1a, cal_done);
    ath11k_qmi_put_u8(&w, 0x1c, mem_cfg_mode);
    return ath11k_qmi_finish(&w, QMI_REQUEST, txn, QMI_WLANFW_HOST_CAP_REQ_V01);
}

/* CAP: petición sin cuerpo; la respuesta trae la capacidad del target
 * (QMI_WLANFW_CAP_REQ_MSG_V01_MAX_LEN es 0 en qmi.h:300). */
int ath11k_qmi_build_cap_req(unsigned char *buf, unsigned int cap, uint16_t txn)
{
    struct qmi_writer w;
    ath11k_qmi_init(&w, buf, cap);
    return ath11k_qmi_finish(&w, QMI_REQUEST, txn, QMI_WLANFW_CAP_REQ_V01);
}

/* WLAN_MODE: arranca (MISSION) o para (OFF) el firmware. */
int ath11k_qmi_build_wlan_mode(unsigned char *buf, unsigned int cap, uint16_t txn,
                               uint32_t mode, int hw_debug)
{
    struct qmi_writer w;
    ath11k_qmi_init(&w, buf, cap);
    ath11k_qmi_put_u32(&w, 0x01, mode); /* obligatorio */
    if (hw_debug)
        ath11k_qmi_put_u8(&w, 0x10, 1);
    return ath11k_qmi_finish(&w, QMI_REQUEST, txn, QMI_WLANFW_WLAN_MODE_REQ_V01);
}

/* BDF_DOWNLOAD: manda `board-2.bin` en segmentos de hasta 6144 B
 * (`ath11k_qmi_load_bdf_qmi`, qmi.c:2322).
 *
 * El TLV de datos (0x13) es especial: lleva un prefijo de longitud de **dos**
 * bytes antes del contenido, porque su `QMI_DATA_LEN` declara `sizeof(u16)`
 * y el codificador de Linux escribe ese prefijo dentro del TLV
 * (`qmi_encdec.c:340`). Escribirlo sin el prefijo desplaza todo el blob.
 */
int ath11k_qmi_build_bdf_download(unsigned char *buf, unsigned int cap, uint16_t txn,
                                  uint32_t total_size, uint32_t seg_id,
                                  const unsigned char *datos, unsigned int n,
                                  int end, unsigned char bdf_type)
{
    if (n > QMI_WLANFW_MAX_DATA_SIZE_V01)
        return -1;
    struct qmi_writer w;
    ath11k_qmi_init(&w, buf, cap);
    ath11k_qmi_put_u8(&w, 0x01, 1); /* valid */
    ath11k_qmi_put_u32(&w, 0x10, QMI_WLANFW_CAL_TEMP_IDX_0_V01); /* file_id */
    ath11k_qmi_put_u32(&w, 0x11, total_size);
    ath11k_qmi_put_u32(&w, 0x12, seg_id);
    /* TLV 0x13 = [len u16][datos] */
    if (!w.error && w.len + 3 + 2 + n <= w.cap) {
        unsigned char *p = w.buf + w.len;
        p[0] = 0x13;
        unsigned int tlv_len = 2 + n;
        p[1] = (unsigned char)(tlv_len & 0xff);
        p[2] = (unsigned char)((tlv_len >> 8) & 0xff);
        p[3] = (unsigned char)(n & 0xff);
        p[4] = (unsigned char)((n >> 8) & 0xff);
        for (unsigned int i = 0; i < n; i++)
            p[5 + i] = datos[i];
        w.len += 3 + tlv_len;
    } else {
        w.error = 1;
    }
    ath11k_qmi_put_u8(&w, 0x14, end ? 1 : 0);
    ath11k_qmi_put_u8(&w, 0x15, bdf_type);
    return ath11k_qmi_finish(&w, QMI_REQUEST, txn, QMI_WLANFW_BDF_DOWNLOAD_REQ_V01);
}

/* M3_INFO: dónde ha dejado el host el blob m3.bin. Ambos campos son
 * obligatorios (`qmi_wlanfw_m3_info_req_msg_v01_ei`). */
int ath11k_qmi_build_m3_info(unsigned char *buf, unsigned int cap, uint16_t txn,
                             uint64_t addr, uint32_t size)
{
    struct qmi_writer w;
    ath11k_qmi_init(&w, buf, cap);
    ath11k_qmi_put_u64(&w, 0x01, addr);
    ath11k_qmi_put_u32(&w, 0x02, size);
    return ath11k_qmi_finish(&w, QMI_REQUEST, txn, QMI_WLANFW_M3_INFO_REQ_V01);
}
