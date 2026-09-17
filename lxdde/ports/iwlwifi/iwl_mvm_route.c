/* Elección MAC legacy (0x28) vs MLD (MAC_CONFIG/LINK) según CMD_VERSIONS del ucode. */
#include "iwl_internal.h"
#include "iwl_ax211.h"

int iwl_mvm_uses_mld_mac(const struct iwl_ax211_priv *iwl)
{
    if (!iwl)
        return 0;
    if (iwl_fw_cmd_ver((struct iwl_ax211_priv *)iwl, LEGACY_GROUP,
                       MAC_CONTEXT_CMD) > 0)
        return 0;
    return iwl_fw_cmd_ver((struct iwl_ax211_priv *)iwl, MAC_CONF_GROUP,
                          MAC_CONFIG_CMD) > 0;
}

int iwl_mvm_fw_has_binding_cmd(const struct iwl_ax211_priv *iwl)
{
    if (!iwl)
        return 0;
    return iwl_fw_cmd_ver((struct iwl_ax211_priv *)iwl, LEGACY_GROUP,
                          BINDING_CONTEXT_CMD) > 0;
}
