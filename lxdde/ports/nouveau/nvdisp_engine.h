#ifndef NVDISP_ENGINE_H
#define NVDISP_ENGINE_H

/* N3: capa soso sobre nvkm disp; N4: head + patrón (N5: GNVD scanout). */
int nvdisp_engine_init(void);
int nvdisp_engine_nvkm_bios_linked(void);

int nvdisp_ready(void);
int nvdisp_head_bringup(void);

#endif
