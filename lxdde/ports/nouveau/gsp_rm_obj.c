/* G4c: objetos de RM. Ver gsp_rm_obj.h. */
#include "gsp_rm_obj.h"

/* Definidos en el shim (lxdde/shim/src/shims.c). */
void *memcpy(void *dst, const void *src, unsigned long n);
void *memset(void *dst, int c, unsigned long n);

/* Los objetos de G4c piden como mucho 120 B de parámetros y el canal de G4e 368,
 * pero `GET_DEVICE_INFO_TABLE` son 3212 B de una sola pieza —la tabla viene
 * paginada de 32 entradas y no se puede pedir más corta—, así que el tope sube a
 * 4 KiB. Cabe de sobra en la cola: los mensajes se parten en páginas y el
 * `msgCount` que negocia el GSP son 63 (log del bring-up), o sea 252 KiB.
 *
 * Los búferes van al heap y no a la pila: con wrapper + params y su gemelo de
 * respuesta son ~8 KiB por llamada, y esto corre en una fibra del bring-up. */
#define RM_PARAMS_MAX 4096u

/* Tiempo de espera de una llamada a RM. Generoso: la primera reserva de cliente
 * es lo primero que hace GSP-RM tras arrancar y puede tardar. */
#define RM_TIMEOUT_MS 2000u

/* ---- Catálogo de clases del chip -------------------------------------------
 *
 * Qué clases acepta esta GPU no se deduce: se pregunta. Hasta ahora el port
 * llevaba `AMPERE_CHANNEL_GPFIFO_A` y `AMPERE_DMA_COPY_A` a pelo sobre una
 * GB205, con un comentario que afirmaba que "GB205 comparte la ruta Ampere" —
 * afirmación que nadie había comprobado y que `rm/gb20x.c` de nouveau
 * desmiente: para este chip son las variantes **B de Blackwell**.
 *
 * El catálogo se pide una vez, tras los objetos de RM, y vale para el canal, el
 * CE y el compute de G4f. Si la consulta falla no se aborta nada: se avisa y se
 * usa la primera candidata, que es lo mismo que se hacía antes pero dicho. */
static uint32_t rm_class[NV0080_CTRL_GPU_CLASSLIST_MAX_SIZE];
static unsigned rm_class_nr;
static int rm_class_known;

void gsp_rm_classes_forget(void)
{
    rm_class_nr = 0;
    rm_class_known = 0;
}

int gsp_rm_classes_probe(struct gsp_rm *rm)
{
    NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS *p;
    uint32_t status = 0;
    unsigned i;

    if (!rm || !rm->ready) {
        return -1;
    }
    /* 404 B no caben en la pila de una fibra del bring-up. */
    p = lx_kzalloc(sizeof(*p), GFP_KERNEL);
    if (!p) {
        lx_printk("nouveau-lx: sin memoria para el catálogo de clases\n");
        return -1;
    }
    /* Aquí hubo un `p->numClasses = MAX_SIZE` como hipótesis de por qué RM
     * rechazaba el control: descartada. El campo está anotado `__OUT__` y la
     * causa era otra —el array medía 200 entradas en vez de las 100 de 570.144,
     * o sea 804 B donde RM esperaba 404. */
    if (gsp_rm_control(rm, rm->device, NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2,
                       p, (uint32_t)sizeof(*p), &status) != 0) {
        /* El nombre del status va DENTRO del mensaje: este control salió
         * rechazado con 0x1f en HW y el log sólo daba el hex, así que hubo que
         * ir a buscar la tabla a mano para saber que era INVALID_ARGUMENT. */
        lx_printk("nouveau-lx: GET_CLASSLIST_V2 rechazado (%s, 0x%x) — "
                  "se cae a la tabla por familia\n",
                  nv_status_name(status), status);
        lx_kfree(p);
        return -1;
    }
    if (p->numClasses == 0 || p->numClasses > NV0080_CTRL_GPU_CLASSLIST_MAX_SIZE) {
        lx_printk("nouveau-lx: catálogo con %u clases (fuera de rango)\n",
                  p->numClasses);
        lx_kfree(p);
        return -1;
    }
    rm_class_nr = p->numClasses;
    for (i = 0; i < rm_class_nr; i++) {
        rm_class[i] = p->classList[i];
    }
    rm_class_known = 1;
    lx_kfree(p);

    lx_printk("nouveau-lx: catálogo de clases: %u\n", rm_class_nr);
    /* La lista entera, de ocho en ocho. Es la respuesta a "¿qué clase de canal,
     * de CE y de compute tiene este chip?" y se paga una sola vez. */
    for (i = 0; i < rm_class_nr; i += 8u) {
        unsigned n = rm_class_nr - i;

        if (n > 8u) {
            n = 8u;
        }
        lx_printk("nouveau-lx:   [%3u] %04x %04x %04x %04x %04x %04x %04x %04x\n",
                  i,
                  rm_class[i], n > 1 ? rm_class[i + 1] : 0,
                  n > 2 ? rm_class[i + 2] : 0, n > 3 ? rm_class[i + 3] : 0,
                  n > 4 ? rm_class[i + 4] : 0, n > 5 ? rm_class[i + 5] : 0,
                  n > 6 ? rm_class[i + 6] : 0, n > 7 ? rm_class[i + 7] : 0);
    }
    return 0;
}

int gsp_rm_class_supported(uint32_t cls)
{
    unsigned i;

    if (!rm_class_known) {
        return -1;      /* no se sabe, que no es lo mismo que "no" */
    }
    for (i = 0; i < rm_class_nr; i++) {
        if (rm_class[i] == cls) {
            return 1;
        }
    }
    return 0;
}

uint32_t gsp_rm_class_pick(const char *what, const uint32_t *cand, unsigned n)
{
    unsigned i;

    if (!cand || !n) {
        return 0;
    }
    if (!rm_class_known) {
        /* "A ciegas" era injusto con el dato y encima despistó: `cand[0]` no es
         * una adivinanza, es lo que `rm/gb20x.c` de nouveau prescribe para este
         * chip, y en hardware RM aceptó las tres primeras candidatas del canal y
         * del CE. Cuando el compute falló, el log decía "a ciegas" y mandó a
         * buscar una clase mala que estaba bien. Sin catálogo esto es la elección
         * de upstream, y así se cuenta. */
        lx_printk("nouveau-lx: %s sin catálogo — 0x%04x, la de upstream para "
                  "este chip\n", what, cand[0]);
        return cand[0];
    }
    for (i = 0; i < n; i++) {
        if (gsp_rm_class_supported(cand[i]) == 1) {
            lx_printk("nouveau-lx: %s → clase 0x%04x (del catálogo)\n",
                      what, cand[i]);
            return cand[i];
        }
    }
    /* Ninguna candidata está en el catálogo: el dato importa más que el intento.
     * Se sigue con la primera para que el log muestre qué contesta RM, pero
     * queda dicho que la clase que vamos a pedir NO la reconoce el chip. */
    lx_printk("nouveau-lx: %s — NINGUNA de las %u candidatas está en el "
              "catálogo; se manda 0x%04x igual para ver qué dice RM\n",
              what, n, cand[0]);
    return cand[0];
}

/* ---- Motores del chip y topología del FIFO ----------------------------------
 *
 * Mismo principio que el catálogo de clases: el `engineType` del canal no se
 * deduce, se pregunta. Ver el bloque de `NV2080_CTRL_CMD_GPU_GET_ENGINES_V2` en
 * nvrm_r570.h para el por qué y las referencias.
 *
 * Ninguno de los dos controles es obligatorio para arrancar: si fallan se dice y
 * se sigue. Lo que devuelven es diagnóstico, no configuración — todavía. */

/* El nombre viene de la tarjeta, así que se trata como dato hostil: se corta a
 * su tamaño, se termina en NUL y lo no imprimible se sustituye. Un array sin NUL
 * metido en un %s es una lectura fuera de límites dentro del printk. */
static void engine_name_safe(const char *src, char *dst, unsigned n)
{
    unsigned i;

    for (i = 0; i + 1u < n; i++) {
        unsigned char c = (unsigned char)src[i];

        if (c == 0u) {
            break;
        }
        dst[i] = (c >= 0x20u && c < 0x7fu) ? (char)c : '.';
    }
    dst[i] = '\0';
}

static int engines_list_probe(struct gsp_rm *rm)
{
    NV2080_CTRL_GPU_GET_ENGINES_V2_PARAMS *e;
    uint32_t status = 0;
    unsigned i;
    int has_copy0 = 0;

    e = lx_kzalloc(sizeof(*e), GFP_KERNEL);
    if (!e) {
        lx_printk("nouveau-lx: sin memoria para la lista de motores\n");
        return -1;
    }
    if (gsp_rm_control(rm, rm->subdevice, NV2080_CTRL_CMD_GPU_GET_ENGINES_V2,
                       e, (uint32_t)sizeof(*e), &status) != 0) {
        lx_printk("nouveau-lx: GET_ENGINES_V2 rechazado (%s, 0x%x)\n",
                  nv_status_name(status), status);
        lx_kfree(e);
        return -1;
    }
    /* Un `engineCount` imposible no es "el chip no tiene motores": es la struct
     * desplazada, y entonces la lista de debajo tampoco vale nada. */
    if (e->engineCount == 0u || e->engineCount > NV2080_GPU_MAX_ENGINES_LIST_SIZE) {
        lx_printk("nouveau-lx: GET_ENGINES_V2 dice %u motores (fuera de 1..%u) — "
                  "la transcripción de los params está mal, no el chip\n",
                  e->engineCount, NV2080_GPU_MAX_ENGINES_LIST_SIZE);
        lx_kfree(e);
        return -1;
    }
    lx_printk("nouveau-lx: motores según RM: %u\n", e->engineCount);
    for (i = 0; i < e->engineCount; i += 8u) {
        unsigned n = e->engineCount - i;

        if (n > 8u) {
            n = 8u;
        }
        lx_printk("nouveau-lx:   [%2u] %3u %3u %3u %3u %3u %3u %3u %3u\n", i,
                  e->engineList[i], n > 1 ? e->engineList[i + 1] : 0,
                  n > 2 ? e->engineList[i + 2] : 0, n > 3 ? e->engineList[i + 3] : 0,
                  n > 4 ? e->engineList[i + 4] : 0, n > 5 ? e->engineList[i + 5] : 0,
                  n > 6 ? e->engineList[i + 6] : 0, n > 7 ? e->engineList[i + 7] : 0);
    }
    for (i = 0; i < e->engineCount; i++) {
        if (e->engineList[i] == NV2080_ENGINE_TYPE_COPY0) {
            has_copy0 = 1;
        }
    }
    /* La pregunta concreta que trajo aquí: el canal pide COPY0 y hasta ahora eso
     * era aritmética sobre una tabla, no un hecho sobre esta tarjeta. */
    lx_printk("nouveau-lx: COPY0 (%u) %s en la lista — es el engineType que pide "
              "el canal\n", NV2080_ENGINE_TYPE_COPY0,
              has_copy0 ? "SÍ está" : "NO está");
    lx_kfree(e);
    return 0;
}

/* Lo que hace falta guardarse de la tabla del FIFO: por motor, dónde están sus
 * registros. Sin esto, mirar el estado del canal en el silicio es imposible y el
 * único síntoma de un submit que no arranca es un semáforo a cero.
 *
 * Los índices de `engineData` no están en un enum nuestro, así que estos tres
 * salen de la tabla del propio chip (2026-07-28): data[2] es el engineType —GR0=1,
 * CE0=9, CE1=10, que casan con `NV2080_ENGINE_TYPE_*`—, data[11] el pri base de la
 * runlist —0x00d00000 para GR0/CE0 y 0x00d00400 para CE1, exactamente los valores
 * que nouveau saca de PTOP como `tdev->runlist`— y data[14] el de la channel RAM.
 * Y no se quedan en suposición: quien los usa lee `chcfg` en runlist+0x004 y
 * comprueba que `chcfg & 0xfffffff0` es data[14]. Si no cuadra, lo dice. */
#define FIFO_DEVINFO_ENGINE_TYPE   2u
#define FIFO_DEVINFO_RUNLIST_PRI  11u
#define FIFO_DEVINFO_CHRAM_PRI    14u
#define FIFO_ENGN_MAX             24u

static struct {
    uint32_t engine;
    uint32_t runl_pri;
    uint32_t chram_pri;
} g_fifo_engn[FIFO_ENGN_MAX];
static unsigned g_fifo_engn_cnt;

int gsp_rm_engine_fifo_regs(uint32_t engine, uint32_t *runl_pri,
                            uint32_t *chram_pri)
{
    unsigned i;

    for (i = 0; i < g_fifo_engn_cnt; i++) {
        if (g_fifo_engn[i].engine != engine) {
            continue;
        }
        /* Un pri base a cero o fuera de los 16 MiB de BAR0 mapeados no se
         * devuelve: sería un offset inventado y lo leído, basura con pinta de
         * dato. */
        if (g_fifo_engn[i].runl_pri == 0u ||
            g_fifo_engn[i].runl_pri >= 0x1000000u) {
            return -1;
        }
        if (runl_pri) {
            *runl_pri = g_fifo_engn[i].runl_pri;
        }
        if (chram_pri) {
            *chram_pri = g_fifo_engn[i].chram_pri;
        }
        return 0;
    }
    return -1;
}

static int engines_fifo_table_probe(struct gsp_rm *rm)
{
    NV2080_CTRL_FIFO_GET_DEVICE_INFO_TABLE_PARAMS *t;
    uint32_t base = 0;
    unsigned page;
    int ret = -1;

    g_fifo_engn_cnt = 0;
    t = lx_kzalloc(sizeof(*t), GFP_KERNEL);
    if (!t) {
        lx_printk("nouveau-lx: sin memoria para la tabla de dispositivos del FIFO\n");
        return -1;
    }
    /* Ocho páginas de 32 son las 256 entradas del MAX_DEVICES de upstream: el
     * tope está aquí para que un `bMore` que nunca baje no cuelgue el bring-up,
     * no porque se espere llegar. */
    for (page = 0; page < 8u; page++) {
        uint32_t status = 0;
        unsigned i;
        int more;

        memset(t, 0, sizeof(*t));
        t->baseIndex = base;
        if (gsp_rm_control(rm, rm->subdevice,
                           NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE,
                           t, (uint32_t)sizeof(*t), &status) != 0) {
            lx_printk("nouveau-lx: GET_DEVICE_INFO_TABLE(base=%u) rechazado "
                      "(%s, 0x%x)\n", base, nv_status_name(status), status);
            goto out;
        }
        if (t->numEntries > NV2080_CTRL_FIFO_DEVICE_INFO_MAX_ENTRIES) {
            lx_printk("nouveau-lx: GET_DEVICE_INFO_TABLE dice %u entradas "
                      "(máximo %u) — params desplazados\n",
                      t->numEntries, NV2080_CTRL_FIFO_DEVICE_INFO_MAX_ENTRIES);
            goto out;
        }
        more = t->bMore ? 1 : 0;
        lx_printk("nouveau-lx: FIFO: %u entradas desde %u%s\n", t->numEntries,
                  base, more ? " (y quedan más)" : "");
        for (i = 0; i < t->numEntries; i++) {
            const NV2080_CTRL_FIFO_DEVICE_ENTRY *d = &t->entries[i];
            char name[NV2080_CTRL_FIFO_DEVICE_INFO_MAX_NAME_LEN + 1u];
            unsigned j;

            engine_name_safe(d->engineName, name, sizeof(name));
            if (g_fifo_engn_cnt < FIFO_ENGN_MAX) {
                g_fifo_engn[g_fifo_engn_cnt].engine =
                    d->engineData[FIFO_DEVINFO_ENGINE_TYPE];
                g_fifo_engn[g_fifo_engn_cnt].runl_pri =
                    d->engineData[FIFO_DEVINFO_RUNLIST_PRI];
                g_fifo_engn[g_fifo_engn_cnt].chram_pri =
                    d->engineData[FIFO_DEVINFO_CHRAM_PRI];
                g_fifo_engn_cnt++;
            }
            /* `numPbdmas` acotado antes de indexar: el array son 2 y el número
             * lo pone RM.
             *
             * Nada de "%-16s" para cuadrar columnas: el vsnprintf del shim se
             * come el flag '-' sin aplicarlo y en "%s" ignora la anchura, así que
             * saldría igual de descuadrado pero mintiendo. El nombre va entre
             * comillas, que separa igual de bien y no depende del formateador. */
            lx_printk("nouveau-lx:   %3u '%s' pbdma=%u [%u %u] fault=[%u %u]\n",
                      base + i, name, d->numPbdmas,
                      d->numPbdmas > 0u ? d->pbdmaIds[0] : 0u,
                      d->numPbdmas > 1u ? d->pbdmaIds[1] : 0u,
                      d->numPbdmas > 0u ? d->pbdmaFaultIds[0] : 0u,
                      d->numPbdmas > 1u ? d->pbdmaFaultIds[1] : 0u);
            /* Los 16 words crudos, en dos líneas de ocho. Sin nombres porque el
             * enum de índices no está en ctrl2080fifo.h y bautizarlos a ojo es
             * cómo se acaba pidiendo un canal sobre un motor que no existe. */
            for (j = 0; j < NV2080_CTRL_FIFO_DEVICE_INFO_DATA_TYPES; j += 8u) {
                lx_printk("nouveau-lx:       data[%2u] %08x %08x %08x %08x "
                          "%08x %08x %08x %08x\n", j,
                          d->engineData[j], d->engineData[j + 1],
                          d->engineData[j + 2], d->engineData[j + 3],
                          d->engineData[j + 4], d->engineData[j + 5],
                          d->engineData[j + 6], d->engineData[j + 7]);
            }
        }
        if (!more || t->numEntries == 0u) {
            ret = 0;
            goto out;
        }
        base += t->numEntries;
    }
    lx_printk("nouveau-lx: FIFO: la tabla sigue diciendo 'más' tras 8 páginas; "
              "se corta aquí\n");
    ret = 0;
out:
    lx_kfree(t);
    return ret;
}

int gsp_rm_engines_probe(struct gsp_rm *rm)
{
    int a, b;

    if (!rm || !rm->ready) {
        return -1;
    }
    a = engines_list_probe(rm);
    b = engines_fifo_table_probe(rm);
    return (a == 0 && b == 0) ? 0 : -1;
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

    /* Estos dos `return -1` eran MUDOS y costaron un ciclo de hardware entero
     * (2026-07-28): un control rechazado sin una sola línea que dijera por qué
     * obliga a razonar a ciegas sobre si el RPC se mandó o no. Si falla algo aquí
     * se dice, que es la norma en el resto del port. */
    if (!rm || !rm->q || !rm->rpc || params_size > RM_PARAMS_MAX) {
        lx_printk("nouveau-lx: RM_ALLOC cls=0x%x sin poder mandarse "
                  "(rm=%d q=%d rpc=%d params=%u/%u)\n",
                  cls, rm ? 1 : 0, (rm && rm->q) ? 1 : 0, (rm && rm->rpc) ? 1 : 0,
                  params_size, RM_PARAMS_MAX);
        return -1;
    }
    buf = lx_kzalloc(cap, GFP_KERNEL);
    reply = lx_kzalloc(cap, GFP_KERNEL);
    if (!buf || !reply) {
        lx_printk("nouveau-lx: RM_ALLOC cls=0x%x sin memoria (2 x %lu B)\n",
                  cls, cap);
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
                  cls, handle, nv_status_name(rhdr->status), rhdr->status);
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

int gsp_rm_control_timeout(struct gsp_rm *rm, uint32_t object, uint32_t cmd,
                           void *params, uint32_t params_size, uint32_t *rm_status,
                           unsigned timeout_ms)
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

    /* Igual que en gsp_rm_alloc: nada de `return -1` a oscuras. */
    if (!rm || !rm->q || !rm->rpc || params_size > RM_PARAMS_MAX) {
        lx_printk("nouveau-lx: RM_CONTROL cmd=0x%08x sin poder mandarse "
                  "(rm=%d q=%d rpc=%d params=%u/%u)\n",
                  cmd, rm ? 1 : 0, (rm && rm->q) ? 1 : 0, (rm && rm->rpc) ? 1 : 0,
                  params_size, RM_PARAMS_MAX);
        return -1;
    }
    buf = lx_kzalloc(cap, GFP_KERNEL);
    reply = lx_kzalloc(cap, GFP_KERNEL);
    if (!buf || !reply) {
        lx_printk("nouveau-lx: RM_CONTROL cmd=0x%08x sin memoria (2 x %lu B)\n",
                  cmd, cap);
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
                      reply, (uint32_t)cap, &got, &transport, timeout_ms) != 0) {
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
                  cmd, object, nv_status_name(rhdr->status), rhdr->status);
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

int gsp_rm_control(struct gsp_rm *rm, uint32_t object, uint32_t cmd,
                   void *params, uint32_t params_size, uint32_t *rm_status)
{
    return gsp_rm_control_timeout(rm, object, cmd, params, params_size, rm_status,
                                  RM_TIMEOUT_MS);
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

/* ¿Es texto imprimible? El nombre de la GPU es la prueba de que los offsets del
 * struct caen donde creemos: si están desplazados, aquí sale basura. */
static int looks_like_text(const unsigned char *s, unsigned n)
{
    unsigned i;

    if (!n || s[0] < 0x20u || s[0] > 0x7eu) {
        return 0;
    }
    for (i = 0; i < n && s[i]; i++) {
        if (s[i] < 0x20u || s[i] > 0x7eu) {
            return 0;
        }
    }
    return 1;
}

int gsp_static_info_get(struct gsp_rm *rm, uint64_t vram_expected,
                        struct gsp_static_info *out)
{
    GspStaticConfigInfo *info;
    uint32_t got = 0;
    uint32_t transport = 0;
    unsigned i;
    int ret = -1;

    if (!rm || !rm->q || !rm->rpc || !out) {
        return -1;
    }
    memset(out, 0, sizeof(*out));

    /* NO es una petición pelada, y creer que lo era costó G4d.
     *
     * Upstream la manda con `nvkm_gsp_rpc_rd(gsp, fn, sizeof(*rpc))`, y ese
     * tamaño **viaja en la petición**: `r535_gsp_rpc_get` pone
     * `rpc->length = sizeof(cabecera) + payload_size`. O sea que el mensaje
     * saliente mide 32 + 1656 = 1688 B, con el payload sin inicializar — RM no
     * lo lee, lo usa de hueco donde escribir la respuesta.
     *
     * Mandándolo pelado (length=32) GSP-RM contestaba `rpc_result=0xff100002`
     * = `NV_VGPU_MSG_RESULT_RPC_INVALID_MESSAGE_FORMAT`: el mensaje no le cabía
     * la respuesta y lo rechazó el transporte, sin llegar a RM. Por eso la
     * respuesta venía con `len=32` y sin payload.
     *
     * El búfer va a cero (`lx_kzalloc`) y se usa de ida y de vuelta: la copia
     * de salida se hace y se libera dentro de `gsp_cmdq_send` antes de que la
     * respuesta se escriba encima. */
    info = lx_kzalloc(sizeof(*info), GFP_KERNEL);
    if (!info) {
        return -1;
    }
    if (gsp_cmdq_call(rm->q, rm->rpc, NV_VGPU_MSG_FUNCTION_GET_GSP_STATIC_INFO,
                      info, (uint32_t)sizeof(*info),
                      info, (uint32_t)sizeof(*info), &got, &transport,
                      RM_TIMEOUT_MS) != 0) {
        lx_printk("nouveau-lx: GET_GSP_STATIC_INFO sin respuesta (transporte=0x%x)\n",
                  transport);
        goto out;
    }
    if (got < sizeof(*info)) {
        lx_printk("nouveau-lx: GSP static info corta: %u B, esperaba %u\n",
                  got, (unsigned)sizeof(*info));
        goto out;
    }

    out->fb_length = info->fb_length;
    out->bar1_pde_base = info->bar1PdeBase;
    out->bar2_pde_base = info->bar2PdeBase;
    out->bar1_size = info->sriovCaps.bar1Size;
    out->internal_client = info->hInternalClient;
    out->internal_device = info->hInternalDevice;
    out->internal_subdevice = info->hInternalSubdevice;
    out->l2_cache_size = info->l2_cache_size;
    for (i = 0; i < sizeof(out->name) - 1; i++) {
        out->name[i] = (char)info->gpuNameString[i];
    }
    out->name[sizeof(out->name) - 1] = 0;

    /* Regiones utilizables: ni reservadas ni protegidas (`r535_gsp_get_static_info_fb`). */
    for (i = 0; i < info->fbRegionInfoParams.numFBRegions &&
                i < NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO_MAX_ENTRIES; i++) {
        const NV2080_CTRL_CMD_FB_GET_FB_REGION_FB_REGION_INFO *r =
            &info->fbRegionInfoParams.fbRegion[i];

        lx_printk("nouveau-lx: región FB %u: 0x%llx-0x%llx rsvd=0x%llx prot=%u\n",
                  i, (unsigned long long)r->base, (unsigned long long)r->limit,
                  (unsigned long long)r->reserved, (unsigned)r->bProtected);

        if (!r->reserved && !r->bProtected && r->limit >= r->base &&
            out->region_nr < GSP_FB_REGION_MAX) {
            out->region[out->region_nr].base = r->base;
            out->region[out->region_nr].size = (r->limit + 1u) - r->base;
            out->usable_bytes += out->region[out->region_nr].size;
            out->region_nr++;
        }
    }

    /* El contraste que hace esto falsable: la VRAM ya la sabemos por registro.
     * Si `fb_length` no cuadra, el struct está desplazado y todo lo que salga de
     * él —incluidas las bases de las PDE— es basura con buena pinta. */
    if (vram_expected && info->fb_length != vram_expected) {
        lx_printk("nouveau-lx: OJO — fb_length=0x%llx no cuadra con la VRAM "
                  "conocida 0x%llx: el layout de GspStaticConfigInfo está mal\n",
                  (unsigned long long)info->fb_length,
                  (unsigned long long)vram_expected);
        goto out;
    }
    if (!looks_like_text(info->gpuNameString, sizeof(info->gpuNameString))) {
        lx_printk("nouveau-lx: OJO — gpuNameString no es texto: offsets desplazados\n");
        goto out;
    }

    out->ready = 1;
    ret = 0;
    lx_printk("nouveau-lx: GSP static info: '%s' VRAM=%llu MiB utilizable=%llu MiB "
              "en %u región(es) L2=%u KiB\n",
              out->name, (unsigned long long)(out->fb_length >> 20),
              (unsigned long long)(out->usable_bytes >> 20), out->region_nr,
              out->l2_cache_size >> 10);
    lx_printk("nouveau-lx: RM interno cli=0x%08x dev=0x%08x sub=0x%08x "
              "bar1Pde=0x%llx bar2Pde=0x%llx\n",
              out->internal_client, out->internal_device, out->internal_subdevice,
              (unsigned long long)out->bar1_pde_base,
              (unsigned long long)out->bar2_pde_base);
out:
    lx_kfree(info);
    return ret;
}

int gsp_rm_client_new(struct gsp_cmdq *q, struct gsp_rpc *rpc, struct gsp_rm *rm,
                      unsigned client_id)
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
    rm->client = NVKM_RM_CLIENT(client_id);
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

int gsp_rm_init(struct gsp_cmdq *q, struct gsp_rpc *rpc, struct gsp_rm *rm)
{
    return gsp_rm_client_new(q, rpc, rm, 0);
}
