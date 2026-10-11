# Campaña de diez tareas (T29)

**Estado:** terminada (2026-10-11). **NO-GO**: 1 aceptada de 10 en la ejecución 2 (la 1 se perdió por infraestructura). SI-4 **no** se cierra. Los resultados se añaden abajo; no se reescribe lo fijado.

## Lo que se fijó antes de ejecutar (no se cambia tras ver resultados)
- Tareas, orden, límites e inyecciones: [`tests/self-improvement/campaign.json`](../../tests/self-improvement/campaign.json), sha256 `967ac3f73ca9e0beb7d48c9c4780a43da2f499d91db54df7a7f0dd1aac533df0`.
- Las cinco reales de T02 (R01–R05) más cinco especificadas y comprobables (R06–R10) en `tests/self-improvement/campana/` (el banco sellado de T14 no se toca). Para cada una de las cinco nuevas se comprobó que **la base falla y la solución de referencia pasa** (`verificar repo` con y sin `--con-referencia`).
- Modelo: `qwen2.5-coder-14b`, T=0 (lock de T14). Agente: el nativo de T35, con `read`, `grep`, `glob`, `reemplazar_texto` y `bash` (lista blanca).
- Base inicial: `git archive 6f558f072` (883 archivos capturados). **Secuencial**: cada tarea parte de lo que el validador aceptó en las anteriores (el árbol del validador, base + paquete, es la base siguiente).
- Intentos: 1 por tarea; **2 en R02 y R07**, que llevan la inyección (el agente se mata con `kill -9` a los 75 s y 45 s, y el coordinador se reinicia con `tarea reanudar --continuar`; recuperar abre otro intento y gasta presupuesto, por eso el segundo).
- Aceptación: la prueba reservada de cada caso, ejecutada por `verificar repo` **sobre el árbol reconstruido del validador** (T26). El enunciado visible es el `problema` de la tarea; la prueba reservada no entra en la copia del agente.
- Criterio SI-4: ≥ 5 aceptadas de 10, recuperación acreditada (inyecciones recuperadas) y base final en verde (checks de C6 sobre lo promovido).
- Fallos clasificados por responsable: infraestructura / protocolo (el modelo no usó bien las herramientas) / programación (editó y el validador lo rechazó). Nada se quita del denominador.

## Ajustes al agente hechos ANTES de lanzar (con R03 como tarea de ensayo)
El arnés se probó tres veces con **R03** (una tarea de la campaña; el ensayo no cuenta: la campaña la vuelve a ejecutar desde cero). Qué enseñó:
1. Ensayo 1: el 14B leyó el archivo grande entero y se rindió sin editar (`read` devolvía 2000 líneas).
2. Ensayo 2: tras acotar `read` a 300 líneas por defecto leyó **por tramos sucesivos** sin usar `grep`, y acabó con una llamada a herramienta mal formada (`<|im_start|>` + JSON suelto).
3. Ensayo 3: con el aviso «archivo largo: no lo recorras por tramos; usa `grep`» (en la salida de `read` cuando faltan líneas y hay >400, y en el prompt de sistema) → **aceptada** en 915 s, 6 herramientas.
Cambios congelados: `read` 300 líneas por defecto + aviso; paso 2 del prompt; la conversación completa de cada intento se guarda aparte (`…conversacion.json`). Huellas al lanzar: `herramientas.rs` `a6e2bb36…`, `mod.rs` `01adc134…`, `correr.sh` `064330ac…`.

## Resultados

### Ejecución 1 (2026-10-11 19:40–20:30): 0 aceptadas de 10 — NO-GO, **con cinco tareas perdidas por la infraestructura**
Evidencia: `tests/self-improvement/campana/ejecucion-1/` (registros, estados, conversaciones de cada intento).

| tarea | resultado | de quién | segundos |
|---|---|---|---|
| R01 | rechazada | programación: el 14B rompió la sintaxis de `decode.rs` y se rindió | 452 |
| R02 | error | infraestructura (inyección: agente matado a los 75 s; al reanudar, `429 busy`) | 77 |
| R03–R06 | error | infraestructura: `429 busy` en el primer turno, 1 s cada una | 1 |
| R07 | rechazada | programación (la inyección no llegó a ocurrir: la ejecución acabó antes de los 45 s) | 1143 |
| R08 | rechazada | programación: puso las entidades detrás de `_ => None` (inalcanzable) | 401 |
| R09 | rechazada | programación | 840 |
| R10 | rechazada | programación | 576 |

**Lo que pasó con la infraestructura.** El servidor de inferencia admite **una generación a la vez** y contesta `429 busy` a quien llega mientras genera. El `kill -9` de R02 mató al cliente pero el servidor siguió generando para él (hasta `max_tokens`), así que R02 al reanudar y **R03–R06 recibieron 429 y murieron al instante sin haber sido evaluadas**. Es un defecto del arnés/cliente (no esperaba), no del modelo, y es justo la clase «infraestructura» del criterio: cuenta en el denominador. Por eso esta ejecución **no se retoca ni se descarta**: se publica como NO-GO y se corrige el cliente.

**Corrección (antes de la ejecución 2):** el cliente HTTP del agente espera y reintenta ante `429` hasta agotar el plazo del turno (prueba en `tests/agente.rs`: dos 429 seguidos → dos esperas de 5 s y un tercer intento bueno). Nada del modelo, el prompt, las herramientas, las tareas ni el criterio cambia entre ejecuciones.

### Ejecución 2 (2026-10-11 20:33–22:27): 1 aceptada de 10 — **NO-GO**
Misma selección, mismo modelo, mismo agente; sólo cambia el cliente (espera ante `429`). Evidencia: `tests/self-improvement/campana/ejecucion-2/` (`resumen.md`/`resumen.md.json` generados por `soso-improve report`, registros, estados, diarios y la conversación completa de cada intento).

| tarea | resultado | de quién | intentos | seg | qué pasó |
|---|---|---|---|---|---|
| R01 | rechazada | programación | 1 | 404 | el reemplazo dejó un `}` de más (error de sintaxis) y se rindió |
| R02 | rechazada | programación | 2 | 1051 | **inyección**; añadió una validación de CRC con un `to_vec()` sobre `&[u8]` en un crate `no_std` (no compila) |
| **R03** | **aceptada** | – | 1 | 833 | `k.to_lowercase() == key.to_lowercase()`; pasa la prueba reservada |
| R04 | bloqueada | protocolo | 1 | 657 | leyó y buscó, nunca editó: «el candidato no cambió nada» |
| R05 | rechazada | programación | 1 | 528 | intentó sustituir el encabezado de `oversized` y estropeó el archivo |
| R06 | rechazada | programación | 1 | 510 | su edición no compila |
| R07 | rechazada | programación | 2 | 1117 | **inyección**; editó y terminó con el texto «Continuar» sin probar |
| R08 | rechazada | programación | 1 | 401 | las entidades quedaron tras `_ => None` (inalcanzables) y no probó |
| R09 | rechazada | programación | 1 | 814 | editó y terminó con «esperando el resultado…» sin ejecutar nada |
| R10 | rechazada | programación | 1 | 558 | `}` de más (no compila) |

**Medidas** (todo lo gastado entre la única aceptada; R02 y R07 sin tokens/herramientas porque el intento matado no los dejó): 6873 s, 12 intentos por mejora aceptada; el resto sin calcular por esa razón (`resumen.md`). Fallos: 8 de programación, 1 de protocolo, 0 de infraestructura.

**Recuperación (T27) acreditada con procesos reales.** En R02 y R07 el agente se mató con `kill -9` a los 75 s y 45 s y el coordinador se reinició con `tarea reanudar --continuar`: el intento 0 quedó abierto, la reanudación comprobó que la copia seguía intacta, lo cerró como «bloqueada: caída sin efectos en la copia» (gastando presupuesto) y relanzó el intento 1 sin repetir nada ya hecho; el diario y el estado cuadran (`R02/informe-tras-kill.txt` frente a `R02/informe.txt`).

**Base final (C6):** sólo R03 se promovió; sobre esa base `cargo test -p soso-http` sale 0 (`base-final-r03.txt`). No se ejecutó `cargo xtask check/test` entero: ninguna otra mejora se aceptó y el criterio ya no se alcanza.

**Criterio SI-4:** ≥ 5 aceptadas **no** se cumple (1). Recuperación acreditada y base final en verde sí. Veredicto: **NO-GO**.

## Lectura de los fallos (para quien decida qué hacer)
- El patrón dominante **no** es no saber la solución: es la **edición**. La herramienta `reemplazar_texto` sustituye un fragmento por otro y el 14B lo usa para insertar funciones o ramas nuevas cortando una cabecera `fn …{` o un `match` por la mitad: salen `}` de más, código inalcanzable o llaves sin cerrar (R01, R05, R06, R08, R10). Ninguna herramienta le dice «tu edición no compila» *antes* de guardar.
- Segundo patrón, **protocolo**: termina con una frase en vez de llamar a la herramienta («Continuar», «Esperando el resultado…», o la llamada escrita como texto con `<|im_start|>` y JSON suelto: R07–R09), y por tanto no comprueba lo que hizo.
- R03 sí sale: el cambio es de **una sola expresión** dentro de una línea, que es lo que mejor encaja con `reemplazar_texto`.
- Sin un segundo intento (1 por tarea; 2 sólo en las inyecciones) no se aprovecha el diagnóstico del compilador que el propio agente recibe.
- Hay tres mejoras evidentes que **no** se han hecho porque cambiarían el agente a mitad de campaña: comprobar la sintaxis/`cargo check` tras cada edición dentro de la herramienta, una herramienta de inserción de bloques (no sólo sustitución), y más intentos. Si se hacen es una **campaña nueva** (nombre y hash nuevos), no una reescritura de ésta.

---

# Campaña 2 (`campana-si4-2`)

**Estado:** en ejecución (lanzada el 2026-10-11 22:32). Es una campaña **nueva**: la anterior no se retoca.

## Lo que se fijó antes de ejecutar
- [`tests/self-improvement/campaign-2.json`](../../tests/self-improvement/campaign-2.json), sha256 `3c16c600ea155bf41ae241899612eb3b588e22cae903f594909be202ddb68fd9`. **Mismas** diez tareas, orden, límites, inyecciones (R02 a los 75 s, R07 a los 45 s), modelo (14B, T=0) y criterio que `campana-si4-1`; sólo cambia el agente.
- Cambios del agente (motivados por la «lectura de los fallos» de arriba; todos con pruebas en `tests/agente.rs`):
  1. `reemplazar_texto` **no escribe** una edición que desequilibre `()`, `[]` o `{}` en un `.rs` que estaba equilibrado, y responde qué delimitador y cuánto («1 cierre de más»). El contador ignora comentarios, cadenas, cadenas crudas y caracteres.
  2. Herramienta `insertar` (`filePath`, `linea`, `texto`): añade bloques completos después de una línea de `read`, con la misma comprobación de equilibrio.
  3. Hasta **2 recordatorios** (no gastan llamada a herramienta ni límite) cuando el modelo acaba a medias: respuesta vacía, llamada escrita como texto, cambio sin comprobar, o comprobación fallida (con el error a la vista).
  4. Prompt: paso 3 distingue «cambiar» (`reemplazar_texto`) de «añadir» (`insertar`) y exige bloques completos.
- Huellas al lanzar: `herramientas.rs` `b4ea58ae…`, `mod.rs` `eb5dd0f1…`, `correr.sh` `2df7f25f…`.
- **Sin ensayo previo con tareas de la campaña** (el arnés ya estaba probado): el agente mejorado se estrena en la campaña.

## Resultados
(pendiente)
