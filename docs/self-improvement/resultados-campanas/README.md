# Resultados de las campañas 6–29 (versión ligera)

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
| `campana-12a-gemma-v3-reintentos-T0` | 12a: Gemma, agente v3, 3 rondas, reintentos a T=0 | 2/10 a la primera, 3/10 con reintentos |
| `campana-12b-gemma-v3-reintentos-T07` | 12b: reintentos a T=0,7 | 1/10 a la primera, 5/10 con reintentos |
| `campana-12c-gemma-v3-guardian-T07` | 12c: guardián de repetición con éxito | 3/10 a la primera, 4/10 con reintentos |
| `campana-13-qwen3coder-30b-v3` | 13: Qwen3-Coder-30B-A3B con flujo de expertos, agente v3, 3 rondas | 2/10 a la primera, 4/10 con reintentos |
| `campana-27-gemma-v10-pruebas` | 27: Gemma, agente v10, prompt de pruebas propias, 60 herramientas, 1 ronda | 1/10 |
| `campana-28-gemma-v10` | 28: Gemma, agente v10, prompt estándar, 60 herramientas, 1 ronda | 3/10 |
| `campana-29-gemma-v11-r3` | 29: Gemma, agente v11, prompt estándar, 60 herramientas, 3 rondas (T=0,7 desde la 2.ª) | 3/10 a la primera, 3/10 con reintentos |
| `campana-30-lfm25-v11` | 30: LFM2.5-24B-A2B por llama.cpp, agente v11, 1 ronda | 2/10 |
| `campana-32-lfm25-v12-turnos120` | 32: repetición de R35, R37 y R39 con 120 turnos efectivos | 0 de 3 |
| `campana-33-gemma-v13-autorevision` | 33: Gemma, v13, el agente se revisa a sí mismo antes de terminar, 1 ronda | 1/10 |
| `campana-35-gemma-v15-revisor` | 35: Gemma, v15, revisor independiente (petición aparte), 80 turnos, 1 ronda | 2/10 |
| `campana-39-gemma-v19-herramientas` … `campana-43-gemma-v29` | 39–43: Gemma, 40 turnos, 1 ronda, herramientas de edición y puertas (sintaxis, pruebas propias, Clippy, poda) | 1, 1, 0, 1, 1 /10 |
| `campana-45-gemma-v31` | 45: Gemma, v31, inyección de caída a los 30 s en R33/R38, sin segundo intento | 1/10 (R40) |
| `campana-46-gemma-v32` | 46: Gemma, v32, aviso de Clippy hasta 3 veces y rechazo de monólogos en comentarios | 2/10 (R39, R40) |
| `campana-47-gemma-v33` | 47: Gemma, v33, redirección si solo hay comentarios/módulo vacío | 2/10 (R39, R40) |
| `campana-48-gemma-v35` | 48: Gemma, v35, temperatura adaptativa ante ediciones sin efecto | 2/10 (R39, R40) |
| `campana-49-claude-sonnet55` | 49: **Claude Sonnet 5.5** (suscripción, `claude -p` vía proxy), mismo arnés (v36), 40 turnos, 1 ronda | **10/10** |
| `campana-50-qwen36-35b-a3b-iq3` | 50: **Qwen3.6-35B-A3B** (UD-IQ3_XXS, llama.cpp CUDA, sin razonamiento, T=0,7), arnés v37, 40 turnos, 1 ronda | **5/10** (R34, R36, R37, R39, R40) |
| `campana-51-qwen36-35b-a3b-iq3-razonamiento` | 51: Qwen3.6-35B-A3B (UD-IQ3_XXS) **con razonamiento** (tope 4096 tokens, T=0,6), arnés v37, 40 turnos, 1 ronda | **5/10** (R31, R33, R37, R39, R40) |
| `campana-53-qwen36-35b-a3b-iq3-v38` | 53: Qwen3.6-35B-A3B (UD-IQ3_XXS, sin razonamiento, T=0,7), arnés v38 (rechaza ediciones fuera de «Qué puedes tocar»), 40 turnos, 1 ronda | **7/10** (R31, R32, R35, R36, R37, R39, R40) |
