# Por qué se rechaza tanto (análisis de las campañas 45–47 y de todo el histórico del banco R31–R40)

Gemma 4 12B, 40 turnos, 1 ronda. 30 intentos en 45–47 (2 aceptados en 46 y 47, 1 en 45), 22 campañas en la matriz histórica.
Los recuentos de conversación son de la ventana **visible** (el agente pliega los mensajes viejos), así que son cotas inferiores.

## 1. La matriz tarea × campaña

| tarea | aceptadas / intentos | lectura |
|---|---|---|
| R40 | 18 / 19 | una función, un límite numérico: el modelo la resuelve |
| R39 | 2 / 20 | sólo pasa desde que Clippy se repite (46, 47) |
| R36 | 8 / 19 — **0 / 9 desde la campaña 39** | regresión (ver §4) |
| R32 | 2 / 19 | pasó en 28 y 29, nunca después |
| R34 | 1 / 19 | |
| **R31, R33, R35, R37, R38** | **0 / ~19–20 cada una** | nunca han pasado, con ninguna versión del agente |

Cinco de diez tareas del banco son, hoy, inalcanzables para este modelo con este presupuesto. El pass@1 de ~1–2/10 es, casi entero, R40 y R39 (y R36 antes). Subir o bajar uno depende de dos tareas.

## 2. Se falla por poco, y siempre en lo mismo

Pruebas reservadas que fallan (45 / 46 / 47) sobre el total de la tarea: R31 3/3/3 de 5; R34 2/2/3 de 6; R36 **1/1/1 de 5**; R37 2/1/1 de 6; R33 3/4/0 de 10; R38 3/4/4 de 5; R35 –/3/3 de 8.
- **Las mismas pruebas fallan en las tres campañas**: R31 (`cabecera_de_modelo_justa_no_basta`, `corrupcion_aleatoria…`, `cualquier_truncado…`), R34 (`greedy_ignora_los_nan`, `sin_distribucion_se_comporta_como_greedy`), R36 (`las_versiones_desconocidas_se_rechazan`), R38 (`gzip_simple`, `deflate_zlib_y_crudo`, `gzip_con_campos_opcionales`). No es azar: es un hueco concreto y estable del modelo en cada tarea.
- El criterio es binario: R36 (4 de 5) y R37 (5 de 6) cuentan igual que 0 de 5.
- Los enunciados son claros y cada prueba que falla corresponde a una viñeta de «Qué se pide» (R36: «versión 0 o mayor que 6»; R34: «los NaN nunca se eligen»; R37: «puerto vacío»). No es un problema de especificación.

## 3. Mecanismo dominante: bucles a T=0 (lo más accionable)

- Aceptadas: mediana de **0** ediciones fallidas y 18 llamadas. Rechazadas: mediana de **11** ediciones fallidas y 29 llamadas, 1360 s frente a 190 s.
- De las 246 ediciones fallidas visibles en 45–47: **134 `reemplazar_funcion` «sin efecto»** (el `nuevo` es idéntico al código que ya está), 60 bloqueadas por repetición, 16 `reemplazar_texto` sin efecto. Sólo 6 son solo-comentarios, 5 ambiguas, 3 de línea. **El problema ya no es la interfaz de edición** (rechazos de sintaxis o de «no encuentro el texto»: ~0): es que el modelo reenvía lo mismo.
- Peso en el total: «sin efecto» + «bloqueada» = **35 % (45), 36 % (46), 40 % (47)** de los resultados de herramienta (26 % en la 41, 32 % en la 28).
- Caso R34 (campaña 45): la prueba propia `temp_positivo_no_finito_devuelve_greedy` sale en rojo (msg 13) y desde ahí **23 `reemplazar_funcion` seguidos con el mismo texto** de `argmax`/`sample` hasta agotar los turnos. Con la misma entrada y T=0 la respuesta es la misma.
- Indicio de que la temperatura lo rompe (correlacional): en la campaña 29 las rondas 2 y 3 corrían a T=0,7 y los «sin efecto + bloqueadas» bajaron de 22 % (ronda 1, T=0) a 16 % y 4 %.
- Hasta ahora sólo se subía la temperatura ante respuestas mal formadas, no ante este bucle.

## 4. R36: una regresión real, no ruido

Pasaba 8 de 8 en las campañas 8–35 (28: 17 llamadas, 108 s, 221 k tokens: `if version == 0 || version > 6 { return Err(()) }` y el techo de capas, dos ediciones mínimas). Desde la 39: 0 de 9 con 34 llamadas, ~1400 s, ~650 k tokens; el agente escribe una prueba propia enorme (un constructor de `.som` sintético de 100 líneas) y hace **sólo el techo de 1024 capas**, nunca el rechazo de versiones. Probabilidad de 0/9 con la tasa anterior (~40 %): <1 %. Sospechosos, por orden: las pruebas propias obligatorias (consumen el presupuesto en montar fixtures y diluyen el primer punto del enunciado), `reemplazar_funcion` (reescribir 100 líneas en lugar de un `if`), y los bucles de §3. En 39 y 40 (sin pruebas propias, con herramientas nuevas) también falló, así que no es sólo el prompt.

## 5. Dificultad real de las cinco imposibles

R38 pide implementar el miembro gzip (RFC 1952) y arreglar un `inflate` existente que desborda («attempt to subtract with overflow», `decode.rs:36`, código que el agente no tocó); R35, convertir GGUF de tres arquitecturas; R33, parsear WAV con seis casos; R31, endurecer un parser entero. Son cambios de 100–300 líneas con muchos casos a la vez; con 40 turnos y 12 B, no.

## 6. Lo que NO explica los rechazos (descartado con datos)

Turnos insuficientes: en 32 (120 turnos) y 38 (120 turnos) no mejora, y los intentos largos son los de los bucles. Interfaz de edición, sintaxis y compilación: ya casi no aparecen. Recuperación tras caída: ya funciona (R33, campañas 45–46). Ruido de infraestructura: los fallos son deterministas por tarea.

## 7. Cambios derivados

- **v34** (ya hecho): `hubo_edicion` lee el historial plegado (R33 de la 47 acabó `LimitePasos` sin validar con la edición en disco).
- **v35** (nuevo): tras 2 ediciones «sin efecto» o llamadas bloqueadas seguidas, la temperatura de la petición sube (0,3 → 0,5 → 0,7 → máx. 0,9) y vuelve a 0 con la siguiente edición buena.
- **Resultado de v35 (campaña 48)**: 2/10. Los bucles bajan al 21 % (bloqueadas 48 → 11), **pero fallan las mismas pruebas** en R31, R32, R34, R35, R36, R38. §3 era un síntoma, no la causa.
- **v36** (R36 de la 48): el agente puso `version > 6` dentro del `else` de la rama v1–v3 (sólo leyó desde la línea 883) y su prueba propia nunca compiló (`cannot find macro vec` en un crate `no_std`): prompt «lee la función entera, la comprobación de entrada va al principio» y pista automática de `extern crate alloc`.
- **Siguiente**: lo que queda es el límite del modelo en tareas multi-caso (R31, R33, R35, R37, R38) y la ubicación del cambio; la vía con más sentido es otro programador/revisor más capaz.
