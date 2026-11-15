#include <stdlib.h>
#include <stddef.h>

__attribute__((weak)) void *lx_kmalloc(size_t size, unsigned flags)
{
    (void)flags;
    return malloc(size);
}

__attribute__((weak)) void lx_kfree(void *ptr)
{
    free(ptr);
}

struct iwl_ax211_priv;
__attribute__((weak)) int iwl_mvm_calibrate_8000(struct iwl_ax211_priv *iwl)
{ (void)iwl; return -1; }
__attribute__((weak)) int iwl_mvm_phy_db_apply_8000(struct iwl_ax211_priv *iwl)
{ (void)iwl; return -1; }
__attribute__((weak)) int iwl_mvm_nvm_read_8000(struct iwl_ax211_priv *iwl)
{ (void)iwl; return -1; }
__attribute__((weak)) unsigned iwl_trans_8000_read_prph(struct iwl_ax211_priv *iwl, unsigned addr)
{ (void)iwl; (void)addr; return ~0u; }
__attribute__((weak)) void iwl_phy_db_clear(struct iwl_ax211_priv *iwl)
{ (void)iwl; }
__attribute__((weak)) int iwl_phy_db_send(struct iwl_ax211_priv *iwl)
{ (void)iwl; return -1; }
__attribute__((weak)) int iwl_phy_db_valid(const struct iwl_ax211_priv *iwl)
{ (void)iwl; return 1; }
