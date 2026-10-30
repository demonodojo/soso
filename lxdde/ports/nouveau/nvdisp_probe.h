#ifndef NVDISP_PROBE_H
#define NVDISP_PROBE_H

/* N1: VBIOS/DCB sin encender el panel. Devuelve 0 si el ROM parece legible. */
int nvdisp_probe(void);

/* 1 si algún conector devolvió EDID válido (N1: sólo tras DDC real). */
int nvdisp_probe_edid_ok(void);

/* Hostcheck: parser sobre búfer en RAM. */
int nvdisp_probe_selftest(void);

#endif
