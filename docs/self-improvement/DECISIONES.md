# Decisiones y prioridades del plan de automejora

**Revisión:** 26 de septiembre de 2026. Ver [informe de revisión](seguimiento/REVISION-2026-09-26.md).
Este documento resume las fichas; no convierte una elección en implementación.
La petición de revisar y replanificar autoriza fijar la secuencia de trabajo.

| ID | Situación actual | Acción planificada |
|---|---|---|
| D1 | **Tomada el 27-sep:** el perfil es Qwen2.5-Coder-7B-Instruct | GO 10/10 con banco y herramientas. T20 queda pendiente con esa identidad |
| D2 | Estrategia nativa pendiente; se conserva OpenCode como objetivo contractual | Resolver primero la evaluación; un agente Rust alternativo exigiría cambiar NATIVO y consumidores explícitamente |
| D3 | **Resuelta:** wild 0.10.0 instalado y probado | No volver a bloquear T39 por enlazador ausente |
| D4 | Semántica SIGTERM pendiente, T70 | Aplazada; T25 debe declarar que hoy la terminación usa SIGKILL y verificar descendientes |
| D5 | Auditoría del montón N-013 | Conservarla durante el diagnóstico; optimización fuera del camino crítico |
| D6 | Fiabilidad de tests sin entradas | Separar tokenizer, herramientas y capacidades de CPU; ver trabajo pendiente abajo |
| D7 | **Hecha en T72 (26-sep):** alocador compartido `soso-alloc` | T75 sincroniza la receta en guest; T39 compila std y ejecuta el humo |

## D1: perfil elegido, el 7B

El 27-sep-2026 la campaña 10×3 del 7B cerró con GO, cobertura `completa`,
30/30 y exit 0. Ese es el perfil de la automejora (`qwen2.5-coder-7b`,
directorio `target/qwen2.5-coder-7b-model`, id `soso-coder`). El 3B conserva
su NO-GO 8/10 y no se usa para OpenCode. No se rescata texto como
`tool_calls` ni se quitan herramientas. El selector del live USB no cambia.

Historia de la comparación, antes de ese GO:

| Modelo / intento | Cobertura | Interpretación |
|---|---|---|
| 3B, 25-sep | 8/10 sólidos, 0 inestables; 30 intentos | NO-GO válido de protocolo; Q04/Q07 fallan |
| 7B, 26-sep | 4 sólidos, 1 inestable; transporte perdido desde Q05 | Campaña interrumpida; no permite concluir calidad de Q06–Q10 |

Esa fila del 26-sep es la campaña interrumpida: Q07 aún no se había medido y
el exit 0 del arnés no acreditaba el banco. [T74](T74-campana-7b-interrumpida.md)
la distinguió de un NO-GO. La campaña del 27-sep, ya con Q06 y Q10 corregidos
en el arnés, es la que fija el perfil.

[T20](T20-opencode-config.md) queda pendiente con la identidad del 7B. T21/T22 y T25–T31 esperan esa
cadena; T37/T43/T44/T51 tienen además otras dependencias. No contar cada
ficha posterior como una decisión independiente. Un protocolo GO tampoco
acredita las microtareas, el primer parche ni las campañas finales.

## D7: alocador único, con alcance de implementación explícito

[T72](T72-heap-de-libstd.md) adoptó la opción 3 y la cerró el 26-sep: el
alocador de libsoso vive en `soso-alloc`, también enlazado por soso-rt. Se
conservan sbrk/mmap y sus políticas. La sonda guest de libsoso pasó. El
build de std del mismo día ya no falla en `sys/alloc`; falla por casts de
procedencia en `sys/random` y `sys/sync/futex`.

[T75](T75-receta-bootstrap-desfasada.md) aún debe acreditar en guest la
equivalencia entre receta Rust y script. Después faltan manifiesto del
sysroot y humo guest. Una libstd cruzada no acredita rustc ni Cargo
ejecutándose en soso.

## D2: no sustituir el objetivo nativo por una decisión implícita

T32–T34 inventarían OpenCode/Bun y el sustrato necesario. El backlog tiene
13 fichas, con N-006 aplazada, N-007/N-011 pendientes y N-008 decidida pero
sin implementación acreditada. Tampoco se ha portado el runtime por cerrar
las sondas POSIX. T35 no está lista sólo porque T19/T34 estén `done`.

El contrato actual sigue pidiendo OpenCode dentro de soso. Se mantiene ese
objetivo mientras T74/T14 resuelven el modelo. Si se cambia a un agente Rust,
el cambio debe reescribir NATIVO, SI-3/SI-6 y T20–T22/T25/T35/T37/T51, con
pruebas equivalentes de herramientas y aceptación. No es requisito para
avanzar ahora en T72/T75 y no se inicia un port de Bun en esta revisión.

## D4 y D5: aplazamientos deliberados

T70 midió SIGTERM sobre un hijo dormido con salida 0; SIGKILL termina con
137. El adaptador T47 ya usa SIGKILL. T25 debe mantener una causa de timeout
propia y comprobar terminación/descendientes, sin inferir éxito por el código
del hijo. Decidir SIGTERM o implementar manejadores no bloquea el diagnóstico
7B ni el bootstrap. No se cambia esa semántica durante esta revisión.

N-013 mide coste de auditoría del montón; se conserva el detector mientras
se investiga la pérdida del servicio. Cualquier cambio de frecuencia o
bandera debe comparar detección y rendimiento. Hacerlo condicional sí cambia
la cobertura diagnóstica del perfil sin bandera: no es una mejora sin coste.

## D6: separar entradas obligatorias y pruebas opcionales

La antigua alternativa «exigir 2,2 GB y AVX2 o permitir todo» mezclaba cosas
independientes. Los tests de render/request leen `tokenizer.som`, no los
pesos completos. **[T76](T76-tokenizer-sin-retorno.md)** (2026-09-26) alinea
los cuatro tests que retornaban en silencio con el criterio del test de cinco
fixtures: `panic!` con ruta `target/…` o `tests/self-improvement/reference/`.

**[T78](T78-entradas-integracion.md)** (2026-09-26) cierra el resto de D6 en
host: tests ASR `#[ignore]` con `--ignored` y `SOSO_REQUIRE_ASR`; compilación
del banco exige `rustc` (`SOSO_REQUIRE_RUSTC` en CI).

Tokenizer (T76), SIMD (T77) y entradas de integración (T78) quedan materializados;
D6 no tiene deuda abierta en el catálogo.

## D8: el parser acepta las formas con las que el 7B escribe una llamada

**Decisión (2026-10-08, T22).** Además de `<tool_call>` y la cerca ` ```json ` cerradas,
se aceptan etiqueta/cerca sin cerrar con el JSON completo y el objeto `{"name","arguments"}`
suelto cuando es todo el turno y nombra una herramienta declarada. Un JSON cortado sigue
siendo error. Detalle en `CONTRATO.md` (C3).

**Por qué.** Con el prompt real de OpenCode el 7B escribe esas formas (`generar_crudo`, 3 de
3 peticiones explícitas) y el parser estricto de T07 las rechazaba o las tomaba por texto: el
agente nunca llegaba a ejecutar nada. **Qué se pierde:** un turno que sea exactamente un
objeto con el nombre de una herramienta declarada se interpreta como llamada aunque el modelo
quisiera enseñarlo; es raro y los argumentos se validan contra el esquema igualmente.
**Revisar** si aparece un caso real de falso positivo.

## Cómo mantener el plan

Catálogo, ficha, fila del índice y seguimiento se actualizan juntos. Los
recuentos salen de `tasks.json`, no de números copiados entre documentos.
Las decisiones históricas y sus alternativas siguen en las fichas y resúmenes.
