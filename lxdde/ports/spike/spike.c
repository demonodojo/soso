/* D0: spike C↔Rust — llama a lx_printk implementado en el kernel. */
#include "lx_emul.h"

void lx_spike_run(void)
{
    lx_printk("lxdde: spike C llamando a Rust\n");
}
