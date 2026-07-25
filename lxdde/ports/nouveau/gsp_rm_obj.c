/* G4c: objetos de RM. Ver gsp_rm_obj.h. */
#include "gsp_rm_obj.h"

/* Definidos en el shim (lxdde/shim/src/shims.c). */
void *memcpy(void *dst, const void *src, unsigned long n);
void *memset(void *dst, int c, unsigned long n);

/* Los objetos de G4c piden como mucho 120 B de parámetros; el canal de G4e no
 * llega a 1 KiB. Los búferes van al heap y no a la pila: con wrapper + params y
 * su gemelo de respuesta son ~2 KiB por llamada, y esto corre en una fibra del
 * bring-up. */
#define RM_PARAMS_MAX 1024u

/* Tiempo de espera de una llamada a RM. Generoso: la primera reserva de cliente
 * es lo primero que hace GSP-RM tras arrancar y puede tardar. */
#define RM_TIMEOUT_MS 2000u

static const char *rm_status_hint(uint32_t st)
{
    switch (st) {
    case 0x00u: return "OK";
    case 0x1fu: return "INVALID_ARGUMENT";
    case 0x2bu: return "INVALID_CLASS";
    case 0x2fu: return "INVALID_OBJECT_PARENT";
    case 0x51u: return "NO_MEMORY";
    case 0x56u: return "NOT_SUPPORTED";
    case 0x59u: return "OPERATING_SYSTEM";
    default:    return "?";
    }
}

int gsp_rm_alloc(struct gsp_rm *rm, uint32_t parent, uint32_t handle, uint32_t cls,
                 const void *params, uint32_t params_size, uint32_t *rm_status)
{
    unsigned long cap = sizeof(rpc_gsp_rm_alloc) + RM_PARAMS_MAX;
    unsigned char *buf;
    unsigned char *reply;
    rpc_gsp_rm_alloc *hdr;
    const rpc_gsp_rm_alloc *rhdr;
    uint32_t total = (uint32_t)sizeof(rpc_gsp_rm_alloc) + params_size;
    uint32_t got = 0;
    uint32_t transport = 0;
    int ret = -1;

    if (!rm || !rm->q || !rm->rpc || params_size > RM_PARAMS_MAX) {
        return -1;
    }
    buf = lx_kzalloc(cap, GFP_KERNEL);
    reply = lx_kzalloc(cap, GFP_KERNEL);
    if (!buf || !reply) {
        lx_kfree(buf);
        lx_kfree(reply);
        return -1;
    }
    hdr = (rpc_gsp_rm_alloc *)buf;
    rhdr = (const rpc_gsp_rm_alloc *)reply;
    hdr->hClient = rm->client;
    hdr->hParent = parent;
    hdr->hObject = handle;
    hdr->hClass = cls;
    hdr->status = 0;
    hdr->paramsSize = params_size;
    if (params && params_size) {
        memcpy(buf + sizeof(*hdr), params, params_size);
    }

    if (gsp_cmdq_call(rm->q, rm->rpc, NV_VGPU_MSG_FUNCTION_GSP_RM_ALLOC, buf, total,
                      reply, (uint32_t)cap, &got, &transport, RM_TIMEOUT_MS) != 0) {
        lx_printk("nouveau-lx: RM_ALLOC cls=0x%x obj=0x%08x sin respuesta "
                  "(transporte=0x%x)\n", cls, handle, transport);
        goto out;
    }
    if (got < sizeof(*rhdr)) {
        lx_printk("nouveau-lx: RM_ALLOC cls=0x%x respuesta corta (%u B)\n", cls, got);
        goto out;
    }
    if (rm_status) {
        *rm_status = rhdr->status;
    }
    if (rhdr->status) {
        lx_printk("nouveau-lx: RM_ALLOC cls=0x%x obj=0x%08x → %s (0x%x)\n",
                  cls, handle, rm_status_hint(rhdr->status), rhdr->status);
        goto out;
    }
    lx_printk("nouveau-lx: RM_ALLOC cls=0x%04x obj=0x%08x padre=0x%08x ok (%u B)\n",
              cls, handle, parent, params_size);
    ret = 0;
out:
    lx_kfree(buf);
    lx_kfree(reply);
    return ret;
}

int gsp_rm_control(struct gsp_rm *rm, uint32_t object, uint32_t cmd,
                   void *params, uint32_t params_size, uint32_t *rm_status)
{
    unsigned long cap = sizeof(rpc_gsp_rm_control) + RM_PARAMS_MAX;
    unsigned char *buf;
    unsigned char *reply;
    rpc_gsp_rm_control *hdr;
    const rpc_gsp_rm_control *rhdr;
    uint32_t total = (uint32_t)sizeof(rpc_gsp_rm_control) + params_size;
    uint32_t got = 0;
    uint32_t transport = 0;
    int ret = -1;

    if (!rm || !rm->q || !rm->rpc || params_size > RM_PARAMS_MAX) {
        return -1;
    }
    buf = lx_kzalloc(cap, GFP_KERNEL);
    reply = lx_kzalloc(cap, GFP_KERNEL);
    if (!buf || !reply) {
        lx_kfree(buf);
        lx_kfree(reply);
        return -1;
    }
    hdr = (rpc_gsp_rm_control *)buf;
    rhdr = (const rpc_gsp_rm_control *)reply;
    hdr->hClient = rm->client;
    hdr->hObject = object;
    hdr->cmd = cmd;
    hdr->status = 0;
    hdr->paramsSize = params_size;
    if (params && params_size) {
        memcpy(buf + sizeof(*hdr), params, params_size);
    }

    if (gsp_cmdq_call(rm->q, rm->rpc, NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL, buf, total,
                      reply, (uint32_t)cap, &got, &transport, RM_TIMEOUT_MS) != 0) {
        lx_printk("nouveau-lx: RM_CONTROL cmd=0x%08x sin respuesta (transporte=0x%x)\n",
                  cmd, transport);
        goto out;
    }
    if (got < sizeof(*rhdr)) {
        lx_printk("nouveau-lx: RM_CONTROL cmd=0x%08x respuesta corta (%u B)\n", cmd, got);
        goto out;
    }
    if (rm_status) {
        *rm_status = rhdr->status;
    }
    if (rhdr->status) {
        lx_printk("nouveau-lx: RM_CONTROL cmd=0x%08x obj=0x%08x → %s (0x%x)\n",
                  cmd, object, rm_status_hint(rhdr->status), rhdr->status);
        goto out;
    }
    /* Devolver lo que RM haya escrito, recortado a lo que quepa. */
    if (params && params_size) {
        uint32_t avail = got - (uint32_t)sizeof(*rhdr);

        memcpy(params, reply + sizeof(*rhdr),
               avail < params_size ? avail : params_size);
    }
    ret = 0;
out:
    lx_kfree(buf);
    lx_kfree(reply);
    return ret;
}

int gsp_rm_free(struct gsp_rm *rm, uint32_t handle)
{
    /* `rpc_free_v03_00` = `NVOS00_PARAMETERS_v03_00`: root, padre, objeto, status. */
    uint32_t params[4];

    if (!rm || !rm->q || !rm->rpc) {
        return -1;
    }
    params[0] = rm->client;   /* hRoot */
    params[1] = 0;            /* hObjectParent — upstream lo manda a cero */
    params[2] = handle;       /* hObjectOld */
    params[3] = 0;            /* status */

    return gsp_cmdq_call(rm->q, rm->rpc, NV_VGPU_MSG_FUNCTION_FREE, params,
                         (uint32_t)sizeof(params), NULL, 0, NULL, NULL,
                         RM_TIMEOUT_MS);
}

int gsp_rm_init(struct gsp_cmdq *q, struct gsp_rpc *rpc, struct gsp_rm *rm)
{
    NV0000_ALLOC_PARAMETERS root;
    NV0080_ALLOC_PARAMETERS dev;
    NV2080_ALLOC_PARAMETERS sub;

    if (!q || !rpc || !rm) {
        return -1;
    }
    memset(rm, 0, sizeof(*rm));
    rm->q = q;
    rm->rpc = rpc;
    rm->client = NVKM_RM_CLIENT(0);
    rm->device = NVKM_RM_DEVICE;
    rm->subdevice = NVKM_RM_SUBDEVICE;

    /* 1) El cliente. Su propio handle va dentro de los parámetros: es el único
     * objeto que se reserva "sobre sí mismo" (padre = él mismo). */
    memset(&root, 0, sizeof(root));
    root.hClient = rm->client;
    root.processID = 0xffffffffu;   /* `args->processID = ~0` en upstream */
    if (gsp_rm_alloc(rm, rm->client, rm->client, NV01_ROOT, &root,
                     (uint32_t)sizeof(root), NULL) != 0) {
        lx_printk("nouveau-lx: no pude reservar el cliente de RM\n");
        return -1;
    }

    /* 2) El device, colgado del cliente. Solo `hClientShare`; el resto a cero
     * deja que RM elija el espacio de direcciones. */
    memset(&dev, 0, sizeof(dev));
    dev.hClientShare = rm->client;
    if (gsp_rm_alloc(rm, rm->client, rm->device, NV01_DEVICE_0, &dev,
                     (uint32_t)sizeof(dev), NULL) != 0) {
        lx_printk("nouveau-lx: no pude reservar el device\n");
        return -1;
    }

    /* 3) El subdevice, colgado del device. Parámetros todos a cero. */
    memset(&sub, 0, sizeof(sub));
    if (gsp_rm_alloc(rm, rm->device, rm->subdevice, NV20_SUBDEVICE_0, &sub,
                     (uint32_t)sizeof(sub), NULL) != 0) {
        lx_printk("nouveau-lx: no pude reservar el subdevice\n");
        gsp_rm_free(rm, rm->device);
        return -1;
    }

    rm->ready = 1;
    lx_printk("nouveau-lx: objetos RM listos cli=0x%08x dev=0x%08x sub=0x%08x\n",
              rm->client, rm->device, rm->subdevice);
    return 0;
}
