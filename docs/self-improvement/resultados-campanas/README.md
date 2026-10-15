# Resultados de las campañas 6–11 (versión ligera)

La evidencia completa de estas tandas (conversaciones, eventos e informes por tarea) vive **sólo en local**, en
`tests/self-improvement/campana/ejecucion-9/` … `ejecucion-13-*/`, que están en el `.gitignore`. Aquí se versionan los
**resúmenes, los registros por tarea, las comprobaciones de la base final y las incidencias**, que son lo que citan
`docs/self-improvement/campana.md` y las fichas. Los bancos (`tests/self-improvement/campana3/`, `campana4/`) y los
`campaign-N.json` sí se versionan: bastan para repetir las campañas.

| carpeta | campaña | resultado |
|---|---|---|
| `campana-6-gemma-soso` | 6: R11–R20, Gemma servido por soso (CPU) | 5/10 |
| `campana-7-ciega` | 7: banco ciego R21–R30 | 5/10 |
| `campana-7-parcial-cpu`, `campana-7-repeticion-R26` | intentos auxiliares de la 7 | ver `campana.md` |
| `campana-8-agente-v1` | 8: R31–R40, agente anterior | 2/10 |
| `campana-9-agente-v2` | 9: R31–R40, agente mejorado | 2/10 |
| `campana-11-mellum-v1` | 11a: R31–R40, Mellum 2.1 12B en GPU, agente v1 | 1/10 |
| `campana-11-mellum-v2-sin-ajuste` | 11b: igual con agente v2 (el recorte no se activó) | 1/10 |
| `campana-11-mellum-v2-ctx` | 11c: agente v2 con `chars_por_token = 1.5` | 0/10 |
