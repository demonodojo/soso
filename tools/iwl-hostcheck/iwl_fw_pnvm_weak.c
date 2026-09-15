/* Enlaces débiles para bancos que incluyen iwl_trans.c sin iwl_fw.c completo. */
#include "iwl_internal.h"
#include "iwl_ax211.h"

__attribute__((weak)) int iwl_fw_has_capa(const struct iwl_ax211_priv *iwl, unsigned capa_bit)
{
    (void)iwl;
    (void)capa_bit;
    return 0;
}

__attribute__((weak)) int iwl_fw_pnvm_select(struct iwl_ax211_priv *iwl, struct iwl_pnvm_image *out)
{
    (void)iwl;
    (void)out;
    return -1;
}

__attribute__((weak)) void iwl_trans_8000_drain(struct iwl_ax211_priv *iwl)
{
    (void)iwl;
}
