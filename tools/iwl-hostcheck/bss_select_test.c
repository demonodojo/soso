/*
 * R7 (parte comprobable sin AP): clasificación de un BSS a partir del beacon
 * y selección exacta del que se pide.
 *
 * Lo que se cubre aquí: bit Privacy, RSN IE (versión, cifrados, AKM), IE
 * truncado, SSID ausente del scan, varios BSS con el mismo SSID y rechazo de
 * conectar en abierto a una red protegida. La asociación 802.11 completa
 * necesita un AP y no se simula.
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

/* La asociación real es de R7 con AP; aquí solo interesa a QUÉ se asocia. */
static uint8_t g_assoc_bssid[6];
static char g_assoc_ssid[IWL_AX211_SSID_MAX + 1];
static int g_assoc_llamadas;

int iwl_mvm_assoc_prepare(struct iwl_ax211_priv *iwl, const char *ssid,
                          const uint8_t *bssid)
{
    (void)iwl;
    g_assoc_llamadas++;
    memcpy(g_assoc_bssid, bssid, 6);
    memset(g_assoc_ssid, 0, sizeof(g_assoc_ssid));
    if (ssid) {
        unsigned n = 0;

        while (ssid[n] && n < IWL_AX211_SSID_MAX) {
            g_assoc_ssid[n] = ssid[n];
            n++;
        }
    }
    return 0;
}

int iwl_fw_cmd_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    (void)iwl;
    (void)group;
    (void)cmd;
    return 17;
}

int iwl_fw_notif_ver(struct iwl_ax211_priv *iwl, uint8_t group, uint8_t cmd)
{
    (void)iwl;
    (void)group;
    (void)cmd;
    return 8;
}

uint8_t iwl_mvm_scan_rx_ant(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
    return 1;
}

static struct iwl_ax211_priv *g_iwl_target;

void iwl_ax211_add_bss(const struct iwl_ax211_bss *bss)
{
    if (!g_iwl_target || !bss || g_iwl_target->scan_count >= IWL_AX211_MAX_SCAN) {
        return;
    }
    g_iwl_target->scan[g_iwl_target->scan_count++] = *bss;
}

/* --- Constructor de beacons ------------------------------------------- */

struct beacon {
    uint8_t buf[512];
    unsigned len;
};

static void beacon_init(struct beacon *b, const uint8_t bssid[6], uint16_t caps)
{
    memset(b, 0, sizeof(*b));
    b->buf[0] = 0x80; /* type=0 subtype=8: beacon */
    b->buf[1] = 0x00;
    memset(&b->buf[4], 0xff, 6);    /* DA broadcast */
    memcpy(&b->buf[10], bssid, 6);  /* SA */
    memcpy(&b->buf[16], bssid, 6);  /* BSSID */
    b->buf[34] = (uint8_t)(caps & 0xff);
    b->buf[35] = (uint8_t)(caps >> 8);
    b->len = 24 + 12;
}

static void beacon_ie(struct beacon *b, uint8_t id, const uint8_t *body,
                      unsigned len)
{
    b->buf[b->len++] = id;
    b->buf[b->len++] = (uint8_t)len;
    if (body && len) {
        memcpy(&b->buf[b->len], body, len);
    }
    b->len += len;
}

static void beacon_ssid(struct beacon *b, const char *ssid)
{
    beacon_ie(b, WLAN_EID_SSID, (const uint8_t *)ssid, (unsigned)strlen(ssid));
}

static void beacon_ds(struct beacon *b, uint8_t chan)
{
    beacon_ie(b, WLAN_EID_DS_PARAMS, &chan, 1);
}

/* RSN IE con un cifrado por pares y un AKM. */
static void beacon_rsn(struct beacon *b, uint8_t group_cipher,
                       uint8_t pair_cipher, uint8_t akm, int recortar)
{
    uint8_t ie[20];
    unsigned n = 0;

    ie[n++] = 1; /* version */
    ie[n++] = 0;
    ie[n++] = 0x00; ie[n++] = 0x0f; ie[n++] = 0xac; ie[n++] = group_cipher;
    ie[n++] = 1; ie[n++] = 0;
    ie[n++] = 0x00; ie[n++] = 0x0f; ie[n++] = 0xac; ie[n++] = pair_cipher;
    ie[n++] = 1; ie[n++] = 0;
    ie[n++] = 0x00; ie[n++] = 0x0f; ie[n++] = 0xac; ie[n++] = akm;
    beacon_ie(b, WLAN_EID_RSN, ie, recortar ? 6u : n);
}

static int check_abierta(void)
{
    struct beacon b;
    struct iwl_ax211_bss bss;
    const uint8_t bssid[6] = { 0x02, 0, 0, 0, 0, 1 };
    int slen;

    beacon_init(&b, bssid, 0);
    beacon_ssid(&b, "Abierta");
    beacon_ds(&b, 6);
    slen = iwl_mvm_parse_bss(b.buf, (int)b.len, &bss);
    if (slen != 7 || strcmp(bss.ssid, "Abierta") != 0) {
        fprintf(stderr, "SSID mal parseado (%d, '%s')\n", slen, bss.ssid);
        return -1;
    }
    if (!bss.open || bss.rsn || bss.akm_psk || bss.ccmp) {
        fprintf(stderr, "beacon sin Privacy debería ser abierto\n");
        return -1;
    }
    if (bss.channel != 6 || !bss.band24) {
        fprintf(stderr, "canal del DS Params = %u\n", bss.channel);
        return -1;
    }
    if (memcmp(bss.bssid, bssid, 6) != 0) {
        fprintf(stderr, "BSSID mal copiado\n");
        return -1;
    }
    puts("OK: beacon abierto — SSID, canal y BSSID del propio beacon");
    return 0;
}

static int check_protegidas(void)
{
    struct beacon b;
    struct iwl_ax211_bss bss;
    const uint8_t bssid[6] = { 0x02, 0, 0, 0, 0, 2 };

    /* 1) Privacy sin RSN (WEP o WPA1): protegida, pero no WPA2-PSK. */
    beacon_init(&b, bssid, WLAN_CAPABILITY_PRIVACY);
    beacon_ssid(&b, "Vieja");
    if (iwl_mvm_parse_bss(b.buf, (int)b.len, &bss) <= 0)
        return -1;
    if (bss.open || bss.rsn || bss.akm_psk) {
        fprintf(stderr, "Privacy sin RSN: open=%u rsn=%u akm=%u\n",
                bss.open, bss.rsn, bss.akm_psk);
        return -1;
    }

    /* 2) WPA2-PSK con CCMP en los dos cifrados. */
    beacon_init(&b, bssid, WLAN_CAPABILITY_PRIVACY);
    beacon_ssid(&b, "Casa");
    beacon_ds(&b, 11);
    beacon_rsn(&b, WLAN_CIPHER_CCMP128, WLAN_CIPHER_CCMP128, WLAN_AKM_PSK, 0);
    if (iwl_mvm_parse_bss(b.buf, (int)b.len, &bss) <= 0)
        return -1;
    if (bss.open || !bss.rsn || !bss.akm_psk || !bss.ccmp || bss.channel != 11) {
        fprintf(stderr, "WPA2-PSK/CCMP: open=%u rsn=%u akm=%u ccmp=%u ch=%u\n",
                bss.open, bss.rsn, bss.akm_psk, bss.ccmp, bss.channel);
        return -1;
    }

    /* 3) RSN con AKM 802.1X (5): protegida y sin PSK. */
    beacon_init(&b, bssid, WLAN_CAPABILITY_PRIVACY);
    beacon_ssid(&b, "Empresa");
    beacon_rsn(&b, WLAN_CIPHER_CCMP128, WLAN_CIPHER_CCMP128, 5u, 0);
    if (iwl_mvm_parse_bss(b.buf, (int)b.len, &bss) <= 0)
        return -1;
    if (bss.open || !bss.rsn || bss.akm_psk) {
        fprintf(stderr, "AKM 802.1X marcado como PSK\n");
        return -1;
    }

    /* 4) TKIP por pares: RSN y PSK, pero no CCMP. */
    beacon_init(&b, bssid, WLAN_CAPABILITY_PRIVACY);
    beacon_ssid(&b, "Mixta");
    beacon_rsn(&b, 2u /* TKIP */, 2u, WLAN_AKM_PSK, 0);
    if (iwl_mvm_parse_bss(b.buf, (int)b.len, &bss) <= 0)
        return -1;
    if (!bss.rsn || !bss.akm_psk || bss.ccmp) {
        fprintf(stderr, "TKIP contado como CCMP\n");
        return -1;
    }

    /* 5) RSN truncado: no se inventa lo que falta. */
    beacon_init(&b, bssid, WLAN_CAPABILITY_PRIVACY);
    beacon_ssid(&b, "Corta");
    beacon_rsn(&b, WLAN_CIPHER_CCMP128, WLAN_CIPHER_CCMP128, WLAN_AKM_PSK, 1);
    if (iwl_mvm_parse_bss(b.buf, (int)b.len, &bss) <= 0)
        return -1;
    if (bss.open || bss.akm_psk || bss.ccmp) {
        fprintf(stderr, "RSN truncado: akm=%u ccmp=%u (no deben afirmarse)\n",
                bss.akm_psk, bss.ccmp);
        return -1;
    }

    /* 6) IE que dice medir más de lo que hay: se corta sin leer fuera. */
    beacon_init(&b, bssid, 0);
    beacon_ssid(&b, "Rota");
    b.buf[b.len++] = WLAN_EID_RSN;
    b.buf[b.len++] = 200; /* longitud imposible */
    if (iwl_mvm_parse_bss(b.buf, (int)b.len, &bss) <= 0) {
        fprintf(stderr, "un IE con longitud imposible no debe perder el SSID\n");
        return -1;
    }
    if (bss.rsn) {
        fprintf(stderr, "IE fuera del buffer interpretado como RSN\n");
        return -1;
    }
    /* 7) Trama que no es beacon ni probe response. */
    beacon_init(&b, bssid, 0);
    beacon_ssid(&b, "NoMgmt");
    b.buf[0] = 0x08; /* data frame */
    if (iwl_mvm_parse_bss(b.buf, (int)b.len, &bss) != -1) {
        fprintf(stderr, "una trama de datos no es un BSS\n");
        return -1;
    }
    puts("OK: Privacy/RSN/AKM/cifrado, IE truncado y trama ajena");
    return 0;
}

static int check_seleccion(void)
{
    struct iwl_ax211_priv iwl;
    struct beacon b;
    const uint8_t bssid_lejos[6] = { 0x02, 0, 0, 0, 0, 0x10 };
    const uint8_t bssid_cerca[6] = { 0x02, 0, 0, 0, 0, 0x20 };
    const uint8_t bssid_wpa[6] = { 0x02, 0, 0, 0, 0, 0x30 };

    memset(&iwl, 0, sizeof(iwl));
    iwl.alive = 1;
    iwl.scan_active = 1;
    g_iwl_target = &iwl;
    g_assoc_llamadas = 0;

    /* Mismo SSID en dos BSS: -80 dBm y -40 dBm. */
    iwl.last_rx_rssi = -80;
    beacon_init(&b, bssid_lejos, 0);
    beacon_ssid(&b, "Casa");
    beacon_ds(&b, 1);
    iwl_mvm_rx_scan_frame(&iwl, b.buf, (int)b.len);

    iwl.last_rx_rssi = -40;
    beacon_init(&b, bssid_cerca, 0);
    beacon_ssid(&b, "Casa");
    beacon_ds(&b, 36);
    iwl_mvm_rx_scan_frame(&iwl, b.buf, (int)b.len);

    iwl.last_rx_rssi = -50;
    beacon_init(&b, bssid_wpa, WLAN_CAPABILITY_PRIVACY);
    beacon_ssid(&b, "Cerrada");
    beacon_ds(&b, 6);
    beacon_rsn(&b, WLAN_CIPHER_CCMP128, WLAN_CIPHER_CCMP128, WLAN_AKM_PSK, 0);
    iwl_mvm_rx_scan_frame(&iwl, b.buf, (int)b.len);

    if (iwl.scan_count != 3) {
        fprintf(stderr, "scan_count=%d (esperaba 3)\n", iwl.scan_count);
        return -1;
    }
    /* Duplicado exacto: no entra dos veces. */
    iwl_mvm_rx_scan_frame(&iwl, b.buf, (int)b.len);
    if (iwl.scan_count != 3) {
        fprintf(stderr, "el mismo BSS entró dos veces\n");
        return -1;
    }

    /* El SSID que no está no se sustituye por «el primero». */
    if (iwl_mvm_pick_bss(&iwl, "NoExiste") != 0 ||
        iwl_mvm_connect_open(&iwl, "NoExiste") == 0) {
        fprintf(stderr, "un SSID ausente no debe conectar\n");
        return -1;
    }
    if (g_assoc_llamadas != 0) {
        fprintf(stderr, "se intentó asociar sin BSS (%d)\n", g_assoc_llamadas);
        return -1;
    }
    if (iwl_mvm_connect_open(&iwl, "") == 0) {
        fprintf(stderr, "SSID vacío no debe conectar\n");
        return -1;
    }

    /* Con dos candidatos gana el de más señal, y se fija su canal. */
    if (iwl_mvm_connect_open(&iwl, "Casa") != 0) {
        fprintf(stderr, "no conectó a la red abierta\n");
        return -1;
    }
    if (memcmp(g_assoc_bssid, bssid_cerca, 6) != 0) {
        fprintf(stderr, "se asoció al BSS de peor señal\n");
        return -1;
    }
    if (strcmp(g_assoc_ssid, "Casa") != 0 || iwl.channel != 36) {
        fprintf(stderr, "ssid='%s' canal=%u (esperaba Casa/36)\n",
                g_assoc_ssid, iwl.channel);
        return -1;
    }

    /* En abierto contra una red protegida: no. */
    g_assoc_llamadas = 0;
    if (iwl_mvm_connect_open(&iwl, "Cerrada") == 0 || g_assoc_llamadas != 0) {
        fprintf(stderr, "conexión abierta a una red WPA2 admitida\n");
        return -1;
    }
    /* WPA2 contra una red abierta: tampoco (no hay RSN que negociar). */
    if (iwl_mvm_connect_wpa2(&iwl, "Casa", (const uint8_t *)
                             "0123456789abcdef0123456789abcdef") == 0) {
        fprintf(stderr, "WPA2 contra una red abierta admitido\n");
        return -1;
    }
    /* WPA2 contra la que sí lo ofrece: llega a la asociación (el 4-way es R8). */
    if (iwl_mvm_connect_wpa2(&iwl, "Cerrada", (const uint8_t *)
                             "0123456789abcdef0123456789abcdef") != 0) {
        fprintf(stderr, "WPA2 rechazado en una red WPA2-PSK/CCMP\n");
        return -1;
    }
    if (memcmp(g_assoc_bssid, bssid_wpa, 6) != 0 || iwl.channel != 6) {
        fprintf(stderr, "WPA2 se asoció al BSS equivocado\n");
        return -1;
    }
    puts("OK: SSID exacto, mejor RSSI, canal del beacon y seguridad coherente");
    return 0;
}

int main(void)
{
    if (check_abierta() != 0)
        return 1;
    if (check_protegidas() != 0)
        return 1;
    if (check_seleccion() != 0)
        return 1;
    return 0;
}
