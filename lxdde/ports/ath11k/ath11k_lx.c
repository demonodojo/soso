/* Puente lx_emul → ath11k (WCN6855 / QCNFA765, Steam Deck OLED).
 *
 * Alcance actual: **W1 del plan** (docs/PLAN-STEAMDECK.md) — probe PCI y
 * transporte MHI hasta mission mode. Todavía no hay QMI (W2), ni HTC/WMI (W3),
 * ni camino de datos (W4): el dispositivo queda encendido y con los contextos
 * publicados, que es el punto desde el que arranca lo siguiente.
 */
#include "lx_emul.h"
#include "ath11k_internal.h"

/* ath11k/core.c:462 — WCN6855 hw2.1, que es el de la Deck OLED. */
#define ATH11K_FW_AMSS "ath11k/WCN6855/hw2.1/amss.bin"

static struct ath11k_base g_ab;
static int probado;
static int encendido;

static const struct lx_pci_device_id ath11k_ids[] = {
    { QCOM_VENDOR_ID, WCN6855_DEVICE_ID, 0, 0, 0, 0, 0 },
    { QCOM_VENDOR_ID, QCA6390_DEVICE_ID, 0, 0, 0, 0, 0 },
    { QCOM_VENDOR_ID, QCN9074_DEVICE_ID, 0, 0, 0, 0, 0 },
    { 0, 0, 0, 0, 0, 0, 0 },
};

static int ath11k_probe(struct lx_pci_dev *pdev, const struct lx_pci_device_id *id)
{
    (void)id;
    if (probado)
        return 0;

    g_ab.pdev = pdev;
    g_ab.device_id = lx_pci_device_id(pdev);
    g_ab.phase = "probe";

    if (lx_pci_enable_device(pdev)) {
        lx_printk("ath11k: no pude habilitar el dispositivo PCI\n");
        return -1;
    }
    lx_pci_set_master(pdev);

    /* BAR0 completo: los doorbells de canal y evento viven más arriba que los
     * registros de control, y `ath11k_mhi_init_mmio` comprueba que caben. */
    g_ab.mmio = lx_pci_iomap(pdev, 0, 0);
    if (!g_ab.mmio) {
        lx_printk("ath11k: sin BAR0 mapeado\n");
        return -1;
    }
    /* Longitud del BAR: si el mapeo no la da, se deja a 0 y las comprobaciones
     * de rango quedan desactivadas (no se puede acotar lo que no se conoce). */
    g_ab.mmio_len = 0;

    lx_printk("ath11k: %04x:%04x probado (BAR0 mapeado)\n",
              QCOM_VENDOR_ID, g_ab.device_id);
    probado = 1;
    return 0;
}

int lx_ath11k_init_module(void)
{
    return lx_pci_register_driver("ath11k", ath11k_ids, ath11k_probe, NULL);
}

int lx_ath11k_probed(void)
{
    return probado;
}

/* Enciende el transporte: carga amss.bin y lleva el chip a mission mode. */
int lx_ath11k_start(void)
{
    if (!probado)
        return -1;
    if (encendido)
        return 0;

    const unsigned char *fw = NULL;
    unsigned long len = 0;
    if (lx_request_firmware(ATH11K_FW_AMSS, &fw, &len) || !fw || !len) {
        lx_printk("ath11k: falta %s en /lib/firmware\n", ATH11K_FW_AMSS);
        g_ab.phase = "sin_firmware";
        return -1;
    }
    lx_printk("ath11k: %s cargado (%lu B)\n", ATH11K_FW_AMSS, len);

    int rc = ath11k_mhi_power_up(&g_ab, fw, len);
    lx_release_firmware(fw);
    if (rc) {
        lx_printk("ath11k: bring-up MHI falló en la fase %s\n", g_ab.phase);
        return rc;
    }
    encendido = 1;
    return 0;
}

/* Fase alcanzada, para el informe de hardware: en una máquina sin puerto serie
 * es lo que dice dónde se quedó el bring-up. */
const char *lx_ath11k_phase(void)
{
    return g_ab.phase ? g_ab.phase : "sin_probar";
}

int lx_ath11k_alive(void)
{
    return encendido && g_ab.state == MHI_STATE_M0;
}

void lx_ath11k_exit_module(void)
{
    lx_printk("ath11k: exit\n");
}
