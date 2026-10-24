/* H1: objeto 3D (subcanal 0) + camino CLEAR_SURFACE.
 *
 * Los IDs de clase viven en clce97.h / clcd97.h (solo el número); los métodos
 * transcritos de clc097.h (Ampere), que coinciden con clc797 en CLEAR_SURFACE
 * (0x19d0) y el bloque SET_COLOR_TARGET_* usado en gb20x. */
#ifndef GSP_3D_H
#define GSP_3D_H

#include "gsp_chan.h"
#include "gsp_rm_obj.h"

/* Subcanal 3D en el canal GPFIFO (cla06fsubch.h). */
#define GSP_3D_SUBCHANNEL 0u

#define NVKM_RM_THREED0 0x3d000001u

/* Métodos 3D (NVC097_* en open-gpu-kernel-modules clc097.h). */
#define GSP_3D_SET_OBJECT                    0x00000000u
#define GSP_3D_SET_COLOR_TARGET_A(j)         (0x0800u + (unsigned)(j) * 64u)
#define GSP_3D_SET_COLOR_TARGET_B(j)         (0x0804u + (unsigned)(j) * 64u)
#define GSP_3D_SET_COLOR_TARGET_WIDTH(j)     (0x0808u + (unsigned)(j) * 64u)
#define GSP_3D_SET_COLOR_TARGET_HEIGHT(j)    (0x080cu + (unsigned)(j) * 64u)
#define GSP_3D_SET_COLOR_TARGET_FORMAT(j)    (0x0810u + (unsigned)(j) * 64u)
#define GSP_3D_SET_COLOR_TARGET_MEMORY(j)    (0x0814u + (unsigned)(j) * 64u)
#define GSP_3D_SET_COLOR_TARGET_THIRD_DIM(j) (0x0818u + (unsigned)(j) * 64u)
#define GSP_3D_SET_COLOR_TARGET_ARRAY_PITCH(j) (0x081cu + (unsigned)(j) * 64u)
#define GSP_3D_SET_COLOR_CLEAR_VALUE(i)      (0x0d80u + (unsigned)(i) * 4u)
#define GSP_3D_SET_SURFACE_CLIP_HORIZONTAL   0x0ff4u
#define GSP_3D_SET_SURFACE_CLIP_VERTICAL     0x0ff8u
#define GSP_3D_SET_CT_SELECT                 0x121cu
#define GSP_3D_CLEAR_SURFACE                 0x19d0u
#define GSP_3D_SET_REPORT_SEMAPHORE_A        0x1b00u
#define GSP_3D_SET_REPORT_SEMAPHORE_B        0x1b04u
#define GSP_3D_SET_REPORT_SEMAPHORE_C        0x1b08u
#define GSP_3D_SET_REPORT_SEMAPHORE_D        0x1b0cu

#define GSP_3D_FMT_A8B8G8R8                  0x000000d5u
#define GSP_3D_CT_MEM_LAYOUT_PITCH           (1u << 12)
#define GSP_3D_CLEAR_RGBA                    (0x3cu) /* R+G+B+A enable */
#define GSP_3D_CT_SELECT_ONE                 0x1u    /* count=1, target0=0 */
#define GSP_3D_REPORT_RELEASE_WRITES         0x10u   /* RELEASE | writes complete */

#define GSP_3D_WAIT_MS 5000u

struct gsp_3d {
    struct gsp_rm *rm;
    struct gsp_chan *chan;
    uint32_t handle;
    uint32_t cls;
    uint32_t seq;
    int ready;
};

int gsp_3d_init(struct gsp_rm *rm, struct gsp_chan *chan, struct gsp_3d *td);
void gsp_3d_fini(struct gsp_3d *td);

int gsp_3d_clear(struct gsp_3d *td, uint64_t rt_va, uint32_t width,
                 uint32_t height, uint32_t pitch, uint32_t rgba);

/* Hostcheck: codifica el pushbuffer de clear y comprueba la cabecera SET_OBJECT. */
int gsp_3d_selftest_encode(struct gsp_chan *c, uint32_t threed_cls,
                           uint64_t rt_va, uint32_t w, uint32_t h,
                           uint32_t pitch, uint32_t rgba, uint64_t sem_va,
                           uint32_t sem_payload);

#endif
