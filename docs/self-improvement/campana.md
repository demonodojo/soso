# Campaña de diez tareas (T29)

**Estado:** terminada (2026-10-12). **NO-GO** en las tres campañas: 1, 2 y 3 aceptadas de 10 con el agente cada vez mejorado (ver «Balance» al final). SI-4 **no** se cierra. Los resultados se añaden abajo; no se reescribe lo fijado.

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

**Estado:** terminada (2026-10-11 22:32–00:40). **NO-GO: 2 aceptadas de 10** (R06, R08). Es una campaña **nueva**: la anterior no se retoca.

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
Evidencia: `tests/self-improvement/campana/ejecucion-3/` (el directorio se llama así por el orden de ejecuciones: ejecución 1 y 2 de `campana-si4-1`, ésta es la tercera). `registros-originales/` conserva lo que escribió el arnés; `registros/` lleva dos correcciones **hechas después y anotadas en cada registro**: el fallo de cinco tareas pasa de «infraestructura» a «protocolo», y el recuento de intentos/recuperación de R02 y R07 se recalcula desde `informe.txt`.

| tarea | resultado | de quién | intentos | seg | qué pasó |
|---|---|---|---|---|---|
| R01 | error | protocolo | 1 | 519 | respuesta con argumentos que no son un objeto JSON → el servidor la rechazó con 400 y el agente se rindió |
| R02 | error | protocolo | 2 | 1051 | **inyección** (kill a los 75 s); reanudó bien; el intento 1 acabó en un 400 de formato |
| R03 | error | protocolo | 1 | 644 | 400: «varias llamadas o texto tras una llamada en un turno» |
| R04 | bloqueada | protocolo | 1 | 814 | nunca editó (el recordatorio no bastó) |
| R05 | error | protocolo | 1 | 868 | 400 de formato |
| **R06** | **aceptada** | – | 1 | 475 | con `insertar` (antes: no compilaba) |
| R07 | error | protocolo | 2 | 1259 | **inyección** (kill a los 45 s); reanudó bien; 400 de formato |
| **R08** | **aceptada** | – | 1 | 328 | entidades añadidas dentro del `match` (antes: tras `_ => None`) |
| R09 | rechazada | programación | 1 | 1041 | editó y la prueba reservada no pasa |
| R10 | rechazada | programación | 1 | 626 | editó y la prueba reservada no pasa |

**Resultado: 2 de 10 → NO-GO.** Base final (R06+R08 promovidas): `cargo test -p soso-http -p soso-web-core` sale 0. Recuperación (T27) acreditada otra vez con procesos reales (intento 0 cerrado como «caída sin efectos», intento 1 lanzado). Fallos: 6 de protocolo, 2 de programación, 0 de infraestructura.

**Lectura.** Las mejoras de edición funcionan donde se probaron: el equilibrio de delimitadores y `insertar` convirtieron dos fallos de compilación (R06, R08) en aceptaciones, y los rechazos por llaves de más desaparecieron. Lo que mató cinco tareas fue otra cosa: el servidor **rechaza con 400** una respuesta mal formada del modelo (texto tras una llamada, varias llamadas, argumentos que no son JSON) y el agente trataba ese 400 como fin de la tarea en vez de decírselo al modelo. Eso es del agente, y es lo siguiente a corregir.

---

# Campaña 3 (`campana-si4-3`)

**Estado:** terminada (2026-10-12 00:42–03:14). **NO-GO: 3 aceptadas de 10** (R03, R06, R08). Nueva campaña; las anteriores no se retocan.

## Lo que se fijó antes de ejecutar
- `tests/self-improvement/campaign-3.json` (hash en el propio documento de lanzamiento, abajo). Mismas diez tareas, orden, límites, inyecciones, modelo y criterio.
- **Un solo cambio en el agente respecto a la campaña 2:** cuando el servidor rechaza con 400 la respuesta del modelo por mal formada, el agente **le devuelve el motivo literal del servidor** («varias llamadas o texto tras una llamada…») y le deja reintentar, hasta 5 veces por tarea (`Config::errores_de_formato`); pasado el tope es error. El arnés clasifica el 400 como **protocolo** (era «infraestructura»).

## Resultados
Evidencia: `tests/self-improvement/campana/ejecucion-4/` (misma organización que la anterior; las mismas dos correcciones anotadas en R02 y R07: intentos recontados desde `informe.txt` y herramientas/tokens sin medir por el intento matado).

| tarea | resultado | de quién | intentos | seg | qué pasó |
|---|---|---|---|---|---|
| R01 | rechazada | programación | 1 | 668 | usó `insertar` y `reemplazar_texto` (3 recordatorios) y acabó escribiendo la llamada como texto; la prueba reservada no pasa |
| R02 | rechazada | programación | 2 | 1466 | **inyección** (kill a los 75 s) recuperada; tras 3 recordatorios sigue sin compilar (`Error::BadCrc` no existe) |
| **R03** | **aceptada** | – | 1 | 734 | |
| R04 | bloqueada | protocolo | 1 | 688 | leyó y buscó; con 2 recordatorios nunca editó |
| R05 | rechazada | programación | 1 | 1294 | añadió `split` con referencias a `pending` que no existe en el método; 4 recordatorios; se rinde |
| **R06** | **aceptada** | – | 1 | 472 | |
| R07 | bloqueada | protocolo | 2 | 1774 | **inyección** (kill a los 45 s) recuperada; el equilibrio de llaves rechazó su edición y no insistió |
| **R08** | **aceptada** | – | 1 | 332 | |
| R09 | rechazada | programación | 1 | 1049 | sus pruebas pasan (exit 0) pero la reservada no: entendió a medias el enunciado |
| R10 | rechazada | programación | 1 | 624 | idem: compila y las pruebas del crate pasan, la reservada no |

**Resultado: 3 de 10 → NO-GO.** Recuperación (T27) acreditada con procesos reales (R02, R07). Base final (R03+R06+R08): `cargo test -p soso-http -p soso-web-core` sale 0. Fallos: 2 de protocolo, 5 de programación, 0 de infraestructura. Del 400 de formato ya no muere ninguna tarea.

---

# Balance y advertencia metodológica

| ejecución | agente | aceptadas |
|---|---|---|
| `campana-si4-1` / ej. 1 | v1 | 0 (cinco perdidas por infraestructura: 429) |
| `campana-si4-1` / ej. 2 | v1 + espera ante 429 | **1** (R03) |
| `campana-si4-2` | + equilibrio de llaves, `insertar`, recordatorios | **2** (R06, R08) |
| `campana-si4-3` | + devolver el 400 de formato al modelo | **3** (R03, R06, R08) |

**Estas tres campañas no son tres medidas independientes.** Cada cambio del agente se decidió **mirando los fallos de las mismas diez tareas**, y esas tareas (sobre todo R06–R10, que escribí yo) ya no son un banco ciego para este agente. Que la cifra suba de 1 a 3 enseña que los cambios arreglan fallos reales y generales (llaves de más, formato de las llamadas); **no** enseña cuánto resolvería el agente tareas nuevas. Por la misma razón, **un GO obtenido retocando el agente sobre estas diez tareas no valdría como acreditación de SI-4**: habría que usar tareas **nuevas** fijadas de antemano. Por eso se para aquí.

**Lo que queda por hacer, en orden de rentabilidad** (según los fallos de la campaña 3):
1. R09 y R10 compilan y pasan sus propias pruebas pero no la reservada: son fallos de **comprensión del enunciado** (casos límite); más intentos con el resultado de una **comprobación propia más estricta** (que el agente escriba una prueba por cada punto del enunciado) es la palanca natural.
2. R04 y R07 **no editan**: el modelo lee y busca y se queda; un recordatorio más específico («ya sabes dónde está: edita») o más intentos.
3. R01, R02, R05: el 14B no llega a código que compile con varias APIs a la vez; ahí el modelo pesa más que el agente.
4. Un banco **nuevo** de tareas para la acreditación (no estas diez).

---

# Campaña 4 (`campana-si4-4`): banco nuevo, agente congelado

**Estado:** terminada (2026-10-12 09:38–12:23): **NO-GO, 2 aceptadas de 10** (R15, R19) (lanzada 09:38; el primer intento de las 09:02 no arrancó por un defecto del arnés —lista de tareas por defecto— y no cuenta). `campaign-4.json` sha256 `6178f9e56f244b9a276b421cc71b05afeaf74ecfb090c00c08ef480d498aff12`.

Es la medida que sí vale: diez tareas **nuevas** (R11–R20, `tests/self-improvement/campana2/`) escritas y fijadas antes de ejecutar nada, que el agente **nunca ha visto**, con el agente **congelado** tal como acabó la campaña 3 (`herramientas.rs` `b4ea58ae…`, `mod.rs` `986a8692…`; ningún cambio durante ni después). Para cada tarea se comprobó que la base falla (`verificar repo`, exit 2) y que la solución de referencia pasa (exit 0). Mismas reglas que las anteriores: base `git archive 6f558f072`, secuencial desde la base aceptada, 1 intento (2 en las inyecciones: R12 a los 75 s y R17 a los 45 s), 14B a T=0, criterio SI-4 (≥ 5 de 10, recuperación acreditada, base final en verde). `campaign-4.json` sha256 en el registro de lanzamiento.

| tarea | qué pide |
|---|---|
| R11 | `semver::parse`: prefijo `v` y rechazar un cuarto componente |
| R12 | `Mailbox::parse`: no inventar un tamaño 0 ni aceptar un hash inválido |
| R13 | `PartGeom`: tramo invertido = 0 sectores, `contains`, `overlaps` |
| R14 | `Manifest::find_file` ignora `./` y `/` iniciales |
| R15 | `PackReader::verify_all` |
| R16 | entidades numéricas inválidas → U+FFFD |
| R17 | `resolve_url` con `//host` y base `http://` |
| R18 | `kde::find_pmkid` |
| R19 | `last_used`: en un empate gana el menor índice |
| R20 | `SemVer`: `Display` y `max` |

## Resultados
Evidencia: `tests/self-improvement/campana/ejecucion-6/` (el número es el de la ejecución en `/tmp`; es la campaña 4). Mismas dos correcciones anotadas en los registros de R12 y R17 (intentos recontados desde `informe.txt`; herramientas y tokens sin medir por el intento matado). `registros-originales/` conserva lo que escribió el arnés.

| tarea | resultado | de quién | intentos | seg | qué pasó |
|---|---|---|---|---|---|
| R11 | rechazada | programación | 1 | 1468 | 4 recordatorios y 8 llamadas: dejó el `parse` mal (la comprobación del cuarto componente quedó antes de leer `patch`) |
| R12 | rechazada | programación | 2 | 899 | **inyección** recuperada; sus pruebas pasan, la reservada no (no validó el hash de 64 hex, o el tamaño no numérico) |
| R13 | rechazada | programación | 1 | 1386 | `reemplazar_texto` falló cinco veces («no encuentro ese fragmento») y no llegó a terminar |
| R14 | bloqueada | protocolo | 1 | 765 | editó, buscó con `grep` el texto de una firma (literal) y no volvió a editar |
| **R15** | **aceptada** | – | 1 | 607 | método nuevo, local |
| R16 | bloqueada | protocolo | 1 | 656 | leyó y reemplazó sin que el cambio quedara (el equilibrio de llaves lo rechazó) |
| R17 | bloqueada | protocolo | 2 | 1637 | **inyección** recuperada; el segundo intento leyó, corrió las pruebas del crate en verde y terminó sin haber editado |
| R18 | rechazada | programación | 1 | 753 | `insertar` dejó `pub const` pegado a una línea sin `;` (no compila) |
| **R19** | **aceptada** | – | 1 | 1072 | un `>=` por un `>` |
| R20 | rechazada | programación | 1 | 650 | compila y sus pruebas pasan, la reservada no (`max` o `Display`) |

**Resultado: 2 de 10 → NO-GO.** Recuperación (T27) acreditada otra vez con procesos reales (R12, R17). Base final (R15+R19): `cargo test -p soso-update-core --features std -p gptdisk` sale 0. Fallos: 5 de programación, 3 de protocolo, 0 de infraestructura.

---

# Conclusión de T29

| campaña | banco | agente | aceptadas |
|---|---|---|---|
| 1 (ej. 1) | R01–R10 | v1 | 0 (infraestructura) |
| 1 (ej. 2) | R01–R10 | v1 | 1 |
| 2 | R01–R10 | v2 | 2 |
| 3 | R01–R10 | v3 | 3 |
| **4** | **R11–R20 (nuevo)** | **v3 congelado** | **2** |

**La medida honesta de este agente con este modelo, sobre tareas que no ha visto, es 2 de 10.** El 3 de 10 de la campaña 3 era sobre tareas ya vistas; la bajada a 2 es pequeña (el ajuste a las diez primeras fue casi todo de herramientas y formato, no de memorizar tareas), pero la cifra que cuenta es la de la 4. **SI-4 no se cierra** (mínimo 5) y T29 queda hecha con NO-GO publicado.

**Dónde se pierden las ocho que no salen** (campañas 3 y 4, 14B a T=0):
- **Comprensión incompleta (≈ 5 de 8):** el candidato compila y pasa sus propias pruebas pero no la reservada (R12, R20, y R09/R10 del banco viejo): cubre el caso del enunciado y no los límites que el enunciado también pide. Una tabla de «puntos del enunciado → prueba propia» antes de dar por terminado es la palanca más prometedora.
- **No edita / edita sin dejar rastro (≈ 3):** R14, R16, R17: lee, busca, a veces intenta y el equilibrio de llaves o un `no encuentro` lo frena, y termina sin cambio. Más intentos con el diagnóstico a la vista y una vía de edición menos frágil (que `reemplazar_texto` acepte la línea completa con su número de `read`) lo atacan.
- **Edición que no compila (R11, R13, R18):** aun con equilibrio de llaves, errores de sintaxis menores (`pub const` pegado a otra línea, un `}` en sitio equivocado): un `cargo check` automático tras cada edición, devuelto al modelo antes de seguir, los cogería.

Esas tres mejoras son una **campaña nueva con banco nuevo** (el R11–R20 ya está visto ahora); no se hacen aquí.

---

# Campaña 5 (`campana-si4-5-gemma`): el mismo banco con otro modelo

**Estado:** terminada: **GO 6/10** (lanzada el 2026-10-12 ~18:00 en CPU; **reiniciada a las ~18:25 en GPU** al ver que la GPU sí estaba disponible: ver abajo).

**Idea (del usuario):** probar `google/gemma-4-12B-it-qat-q4_0-gguf`. **Lo que se hizo y por qué así:** soso **no puede ejecutar ese modelo hoy** (su convertidor y su runtime sólo conocen las arquitecturas llama/qwen y no la cuantización Q4_0; Gemma pide además su plantilla de chat y su formato de llamadas a herramientas). Antes de plantear un port se comprueba lo barato: **¿es mejor el modelo?** Se sirve con **llama.cpp** (compilado en CPU en el host, `llama-server --jinja`, `-c 24576`, T=0; 6 975 879 296 bytes) y se le pasa **el mismo banco R11–R20 con el mismo agente congelado** (mismas huellas que en la campaña 4). Sólo cambian el modelo y el runtime (y `max_tokens` 4096 en vez de 2048 porque Gemma «piensa» antes de contestar).
- `campaign-5.json` sha256 `4254b4581b19893dac284aa3554389355eddc00b011f353bf43940e69663156f` (la anterior, `9858780d…`, decía «CPU»); agente: `mod.rs` `986a8692…`, `herramientas.rs` `b4ea58ae…` (sin cambios). El arnés sólo gana `MODELO`, `PUERTO` y `MAX_TOKENS` por entorno.
- **Qué compara y qué no:** el banco R11–R20 ya está visto por el agente, así que esto **compara modelos** con la misma vara (Qwen2.5-Coder-14B: 2/10); **no acredita SI-4**. Un resultado ≥ 5 justificaría el port a soso y una campaña con banco nuevo.
- **CPU → GPU.** Se montó primero en CPU porque era lo más rápido de poner en marcha (sin `nvcc` y con Docker sin runtime de NVIDIA); pero la GPU del host (RTX 5070 Ti, driver 595) funciona y había una imagen `llama.cpp:server-cuda` en local: pasándole los dispositivos `/dev/nvidia*` y las dos librerías del driver a mano, el mismo modelo va a **30,6 tok/s generando y 496 tok/s de prefill** (CPU: 7,4 y 61) con 8 GB de VRAM. La campaña se reinició en GPU; lo de CPU (primera tarea, sin terminar) se descartó.

## Resultados
Evidencia: `tests/self-improvement/campana/ejecucion-8/` (misma organización; mismas dos correcciones anotadas en R12 y R17). Se ejecutó en **GPU** (R11 en 27 s; las diez en ~35 min).

| tarea | Qwen2.5-Coder-14B (campaña 4) | **Gemma 4 12B** (campaña 5) |
|---|---|---|
| R11 | rechazada | rechazada |
| R12 (inyección) | rechazada | **aceptada** (recuperada tras `kill -9`) |
| R13 | rechazada | **aceptada** |
| R14 | bloqueada (no editó) | **aceptada** |
| R15 | **aceptada** | **aceptada** |
| R16 | bloqueada (no editó) | rechazada |
| R17 (inyección) | bloqueada | rechazada (recuperada tras `kill -9`) |
| R18 | rechazada | **aceptada** |
| R19 | **aceptada** | **aceptada** |
| R20 | rechazada | rechazada |
| **aceptadas** | **2 de 10** | **6 de 10** |

**Resultado: 6 de 10 → GO en el criterio de SI-4** (≥ 5, recuperación acreditada con `kill -9` real, base final R12+R13+R14+R15+R18+R19 con `cargo test` en verde en los cinco crates tocados). Fallos: 4, todos de programación (editó y la prueba reservada no pasa); **cero** de protocolo y de infraestructura: Gemma usa las herramientas como se espera y no se rinde sin editar, que era el fallo dominante del Qwen.

**Cómo leerlo, sin pasarse:**
- Misma vara que la campaña 4: mismo banco, mismo agente congelado (huellas idénticas), mismos límites e inyecciones. Cambian el **modelo** y el **runtime** (llama.cpp en GPU; Gemma recibe las llamadas a herramientas en su formato nativo por `--jinja`; Qwen las recibía por el parser de soso).
- El banco R11–R20 ya lo había visto el agente en la campaña 4, pero **no se tocó nada del agente después**, y Gemma no participó en ningún ajuste: la ventaja es del modelo, no de un ajuste a estas tareas. Sigue siendo una sola tanda de diez; un 6/10 tiene un margen de error amplio.
- **El criterio SI-4 pide el modelo servido por soso**, y aquí se sirvió con llama.cpp. Por eso SI-4 **no se cierra aún**: se cerrará cuando esta misma campaña se repita con Gemma 4 **sobre soso** (etapa G8 de `docs/PLAN-GEMMA4.md`). Lo que queda acreditado es la **capacidad del modelo** y la decisión de portarlo.

---

# Campaña 6 (`campana-si4-6-gemma-soso`): el mismo banco, con Gemma servido por soso

**Estado:** terminada (2026-10-12 22:23 → 2026-10-13 01:10): **GO mínimo, 5 de 10**. Interrumpida una vez entre R12 y R13 por el reinicio de la sesión (unidades paradas; reanudada desde R13 con la base promovida tras R12; ver `ejecucion-9/reanudacion.txt`). R13 se repitió entera.

**Qué es:** la campaña 5 con una sola diferencia, el **runtime**: el modelo lo sirve **soso** (`serve_host`: núcleos Q4_0 nativos, prefill por bloques, KV f16 con ventana deslizante real, T=0, 16 hilos en CPU), no llama.cpp. Mismo banco R11–R20, mismo agente congelado (`mod.rs` `986a8692…`, `herramientas.rs` `b4ea58ae…`), mismos límites e inyecciones. `campaign-6.json` sha256 `c2998b40f2e624e935d83da7697bbe26445234fca3a1d3b76342695a879be6e5`. Evidencia: `tests/self-improvement/campana/ejecucion-9/`.

**Diferencia conocida de modelo:** llama-server activa el **pensamiento** de Gemma por defecto (`<|think|>` en el turno de sistema) y soso lo deja **apagado** (el aviso de generación acaba en `<|channel>thought\n<channel|>`). No se ha medido el efecto.

| tarea | Qwen2.5-Coder-14B (c. 4) | Gemma 4 12B, llama.cpp GPU (c. 5) | **Gemma 4 12B, soso CPU (c. 6)** |
|---|---|---|---|
| R11 | rechazada | rechazada | **aceptada** (428 s) |
| R12 (inyección) | rechazada | aceptada | **aceptada** (628 s; recuperada tras `kill -9`) |
| R13 | rechazada | aceptada | error: 30 herramientas agotadas |
| R14 | bloqueada | aceptada | **aceptada** (515 s) |
| R15 | aceptada | aceptada | error: 30 herramientas agotadas |
| R16 | bloqueada | rechazada | rechazada |
| R17 (inyección) | bloqueada | rechazada | rechazada (recuperada tras `kill -9`) |
| R18 | rechazada | aceptada | **aceptada** (424 s) |
| R19 | aceptada | aceptada | **aceptada** (306 s) |
| R20 | rechazada | rechazada | error: 30 herramientas agotadas |
| **aceptadas** | **2** | **6** | **5** |

**Resultado: 5 de 10 → GO en el criterio de SI-4, en el mínimo exacto** (≥ 5; recuperación acreditada con `kill -9` real en R12 y R17; base final R11+R12+R14+R18+R19 con las pruebas de los tres crates tocados en verde, `base-final.txt`; modelo servido por soso). `soso-improve report` lo da como GO. Fallos: 2 de programación (R16, R17: editó y la prueba reservada no pasa) y 3 que el arnés clasifica como «protocolo» (R13, R15, R20: el intento no terminó, `LimiteHerramientas`).

**Qué son esos tres «protocolo».** Mirando las conversaciones (`R1x/r-R1x-intento0.conversacion.json`): en R15 y R20 el modelo repite **20 y 22 veces la misma llamada** `reemplazar_texto` con un `buscar` de varias líneas que no coincide («No encuentro ese fragmento… Copia un trozo más corto»), y a T=0 no cambia de estrategia; R13 hace 30 llamadas con 20 variantes del mismo intento fallido. Es un fallo del **modelo con el agente congelado** (bucle ante una edición que no encaja), no de la infraestructura ni de soso. Con llama.cpp (pensamiento activo) no ocurrió ninguna vez.

**Cómo leerlo, sin pasarse:**
- 5 de 10 es el umbral, no un margen: una tarea menos habría sido NO-GO, y entre la campaña 5 (6) y esta (5) la diferencia es una tarea, dentro del ruido de una tanda de diez. Lo que sí queda claro es que **el port conserva la capacidad**: 5 tareas aceptadas contra las 2 del 14B con la misma vara.
- El banco R11–R20 ya lo vio el agente en la campaña 4, así que **no es una medida ciega**; SI-4 queda cerrado **en el criterio escrito**, no acreditado como generalización.
- Velocidad: tareas aceptadas en 5–10 minutos cada una en CPU; las que agotan herramientas tardan 20–40 minutos.
- Lo más rentable para subir la cifra está identificado en la campaña 4: que `reemplazar_texto` acepte la línea completa con su número de `read` y avisar al modelo cuando repite una llamada idéntica. Es un cambio del **agente**: va con campaña y banco nuevos, no aquí.

---

# Campaña 7 (`campana-si4-7-ciega`): banco nuevo, medida ciega

**Estado:** terminada (2026-10-13): **5 de 10, GO en el umbral exacto**, esta vez sobre un banco que **nadie había visto**.

**Qué es:** la comparación que faltaba. Banco **nuevo R21–R30** (`tests/self-improvement/campana3/`: un crate o función distinto en cada tarea, sin solape con R01–R20), escrito y verificado **antes** de correr (las diez: la base falla con la prueba reservada y la referencia pasa, comprobado de forma independiente con el comando del arnés), y el **agente congelado** (`mod.rs` `986a8692…`, `herramientas.rs` `b4ea58ae…`, sin cambios desde la campaña 4). Mismo modelo (Gemma 4 12B QAT Q4_0, servido por soso, T=0, sin pensamiento), mismos límites e inyecciones (`kill -9` en R22 y R27) y mismo criterio. `campaign-7.json` sha256 `32ed8dda6ddfd57c80e8e424646221e14cb11b735419c094084a3b68181dd8f0`. Evidencia: `ejecucion-11/` (y `ejecucion-11-R26/`, `ejecucion-10-parcial-cpu/`).

| tarea | resultado | detalle |
|---|---|---|
| R21 | **aceptada** | 496 s |
| R22 (`kill -9`) | rechazada (programación) | recuperada tras la inyección; la prueba reservada no pasa |
| R23 | **aceptada** | 645 s |
| R24 | error (protocolo) | 30 herramientas agotadas |
| R25 | **aceptada** | 359 s |
| R26 | **aceptada** | 164 s (repetida aparte, ver incidencias) |
| R27 (`kill -9`) | **aceptada** | 276 s, recuperada |
| R28 | error (contexto agotado) | 34 765 tokens > 32 768 del perfil; el arnés lo etiquetó «infraestructura» y se reclasificó (originales en `registros-originales/`) |
| R29 | rechazada (programación) | 170 s |
| R30 | rechazada (programación) | 186 s |
| **aceptadas** | **5 de 10** | |

**Resultado: 5 de 10 → GO en el criterio de SI-4** (≥ 5; recuperación acreditada con `kill -9` real en R22 y R27; base final con los cinco parches juntos en verde, `base-final.txt`; modelo servido por soso). `soso-improve report` lo da como GO.

**Cómo leerlo, sin pasarse:**
- **Es el mismo 5/10 de la campaña 6 sobre un banco ciego**: el modelo no dependía de haber «visto» R11–R20. Con el 14B salían 2/10 en la misma vara (banco ya visto); con Gemma/llama.cpp 6/10, con Gemma/soso 5/10 y 5/10. La capacidad es real y estable, pero **el margen sigue siendo el mínimo**: una tarea menos sería NO-GO y una tanda de diez no distingue 5 de 6.
- Fallos: tres de programación (editó y la prueba reservada no pasa) y dos de agotamiento (herramientas en R24; contexto en R28). Ninguno es del runtime ni de la infraestructura.
- Qué mejoraría la cifra, identificado ya en las campañas 4 y 6: aviso al modelo ante una llamada idéntica repetida, `reemplazar_texto` por línea con número y un tope de contexto con resumen. Es un cambio del **agente**: va con otro banco nuevo, no aquí.

**Incidencias (todas del asistente o de la máquina, ninguna del modelo):**
- Una prueba de GPU y otra de CPU lanzadas a la vez agotaron la RAM y el OOM killer mató el servidor: R25–R26 de esa tirada salieron como infraestructura. Se paró y se repitió entera; las cuatro tareas válidas de aquel intento (R21–R24) quedan en `ejecucion-10-parcial-cpu/`.
- Un reinicio ordenado del sistema a las 11:18 mató la campaña recién lanzada (sin resultados); se relanzó desde cero.
- Al cambiar el servidor al binario con la capa de Gemma entera en GPU justo al acabar R25, R26 se encontró el puerto cerrado (el arnés lanza la siguiente tarea en ~1 s). R26 se repitió aparte desde la base promovida tras R25 (`ejecucion-11-R26/`).
- **Qué sirvió a qué:** R21–R25 con el backend CPU + matmul en GPU; R26 (repetida), R27–R30 con la capa entera en GPU. Mismos pesos y mismo resultado numérico a ruido de f32 (paridad comprobada); no se midió el efecto en las decisiones del modelo.

---

# Campañas 8 y 9: A/B del agente sobre un tercer banco ciego (R31–R40)

**Estado:** terminadas (2026-10-13/14). **Resultado: 2 de 10 con el agente anterior y 2 de 10 con el agente mejorado → NO-GO en las dos.** Mejorar el agente no cambió la cifra, y este banco desmiente la lectura optimista de las campañas 6 y 7.

**Qué se hizo.** Banco **nuevo** R31–R40 (`tests/self-improvement/campana4/`: `sosomfs`, `sosofs`, `soso-audio`, `soso-llm-core`, `gguf2som`, `sosomodel`, `soso-http`, `soso-web-core`, `block-dev`, `soso-abi`; cinco subagentes lo escribieron y yo verifiqué las diez: la base falla y la referencia pasa), fijado **antes** de correr. Mismo modelo (Gemma 4 12B, soso, GPU, T=0), mismos límites e inyecciones (`kill -9` en R33 y R38), mismo servidor, y **dos agentes**:
- **Campaña 8, agente v1** (congelado desde la campaña 4: `mod.rs` `986a8692…`, `herramientas.rs` `b4ea58ae…`). `campaign-8.json` sha256 `956d22aa…`. Evidencia: `ejecucion-12-agente-v1/`.
- **Campaña 9, agente v2** (`mod.rs` `25484f48…`, `herramientas.rs` `9e7f908d…`): bloqueo de la llamada idéntica que ya falló (con consejo), pista de la línea más parecida y limpieza de números de línea en `reemplazar_texto`, y recorte de salidas antiguas antes de agotar el contexto. Mejoras genéricas diseñadas con los fallos de las campañas 4–7, antes de ver este banco. `campaign-9.json` sha256 `a4084349…`. Evidencia: `ejecucion-13-agente-v2/` (y `…-parcial-detenida/`, un primer intento que se paró a medias y no cuenta).

| tarea | v1 | v2 |
|---|---|---|
| R31 | rechazada | rechazada |
| R32 | error: contexto agotado (422, 32 777 > 32 768) | error: 400 por argumentos que no son JSON tras agotar los reintentos de formato |
| R33 (`kill -9`) | rechazada | rechazada |
| R34 | rechazada | rechazada |
| R35 | rechazada | error: 30 herramientas agotadas |
| R36 | **aceptada** | **aceptada** |
| R37, R38 (`kill -9`), R39 | rechazadas | rechazadas |
| R40 | **aceptada** | **aceptada** |
| **aceptadas** | **2 de 10** | **2 de 10** |

**Lo que se ve en las conversaciones.**
- **El recorte de contexto funciona:** en R32 recortó 7 salidas y el 422 desapareció, pero la tarea acabó por otra causa. No se puede atribuir una mejora.
- **Bloqueo de repeticiones y pista: no llegaron a dispararse** (0 en las dos tareas que acabaron en error). Los bucles de las campañas 6 y 7 no se repitieron en este banco; había otros fallos.
- **R35 fue peor con v2** (30 herramientas, 13 `reemplazar_texto` y 10 `read`): sospecho que recortar salidas antiguas le hizo releer, pero no lo he comprobado. Con diez tareas no se distingue de la variación entre tiradas.
- **Los rechazos de programación son rápidos (76–161 s):** el modelo hace una edición plausible, las pruebas visibles pasan y la reservada no. No es un problema de herramientas sino de comprensión del enunciado.

**Lo que cambia en las conclusiones anteriores.** Con Gemma servido por soso hay ya **tres bancos distintos**: 5 de 10 (R11–R20, visto), 5 de 10 (R21–R30, ciego) y 2 de 10 (R31–R40, ciego). Son **12 de 30 (40 %)** sobre treinta tareas distintas, por debajo del 50 % que fija el criterio. El GO de las campañas 6 y 7 fue real pero **frágil**: dos tandas en el umbral exacto y una por debajo. Lo honesto es que el modelo resuelve algo más de un tercio de estas tareas, no la mitad, y que una tanda de diez no distingue 2 de 5.
- **Qué no se ha probado:** que v2 sea mejor que v1 (empate a 2/10; el cambio más claro es que ya no hay 422 por contexto). Tampoco que sea peor.
- **Qué mejoraría de verdad la cifra:** no las herramientas sino cómo se usa el enunciado: leer la especificación punto por punto y escribir pruebas propias por punto antes de dar la tarea por hecha (la idea ya apuntada en la memoria del proyecto), o dejar que el modelo piense (el pensamiento de Gemma está apagado en soso y la campaña 5 de llama.cpp lo llevaba activo). Son experimentos distintos, con banco nuevo.

## Campaña 11 — Mellum 2.1 12B (JetBrains) en la GPU del host, banco R31–R40

**Qué se probó.** El mismo banco ciego de la campaña 8/9 (`tests/self-improvement/campana4/`, `campaign-11.json`) con otro modelo: **Mellum 2.1 12B** (MoE de 64 expertos, 8 activos; Q4_K_M, sólo existe la variante *Thinking*), servido por soso con la capa MoE entera en la GPU (`SOSO_CUDA=1`, ver `crates/soso-llm-core/src/moe_gpu.rs` y `crates/soso-llm-api/src/cuda/capa/moe.rs`). T=0, 30 herramientas, 1 intento, `max_tokens` 6144, mismo punto de partida (`base0`). **No se ejecutó** la comprobación de la base final ni la inyección de `kill -9`, así que ninguna de estas tandas podría acreditar SI-4 aunque hubiera llegado a 5/10.

| tanda | agente | aceptadas | evidencia versionada |
|---|---|---|---|
| 11a | v1 | **1/10** (R40) | [`campana-11-mellum-v1`](resultados-campanas/campana-11-mellum-v1/resumen.md) |
| 11b | v2, estimador de contexto a 3 car./token | **1/10** (R40) | [`campana-11-mellum-v2-sin-ajuste`](resultados-campanas/campana-11-mellum-v2-sin-ajuste/resumen.md) |
| 11c | v2, `chars_por_token = 1.5` | **0/10** | [`campana-11-mellum-v2-ctx`](resultados-campanas/campana-11-mellum-v2-ctx/resumen.md) |

Comparación sobre el **mismo banco**: Gemma 4 sin pensamiento 2/10 (campañas 8 y 9) y con pensamiento 3/10 (campaña 10, con 3 errores de infraestructura); Mellum 1/10, 1/10 y 0/10. **Mellum no mejora a Gemma aquí** y queda muy por debajo del mínimo de 5/10.

**Lo que se ve.**
- **Patrón estable entre tandas:** R31 agota las 30 herramientas; R32, R34 y R37 las rechazan las pruebas reservadas; R33, R38 y R39 acaban con «el candidato no cambió nada» (el modelo da la tarea por terminada sin haber editado). Sólo R35, R36 (ver abajo) y R40 cambian de una tanda a otra.
- **R40 pasó de aceptada (11a, 11b) a agotar herramientas (11c)** con el mismo modelo y T=0: es variación entre ejecuciones (el entorno de compilación y pruebas de cada tarea no es determinista), no efecto del agente. Con 0–1 de 10 no se distingue v1 de v2.
- **El estimador de contexto del agente v2 era inútil con este tokenizador.** R35 y R36 se caían con un 422 (`el render necesita 36243 / 33193 tokens y hay 32768`) tanto con v1 como con v2: el recorte de salidas antiguas ni se activaba. R35 tiene 56 k caracteres, que el estimador contaba como ~19 k tokens, pero el servidor los renderiza como 36 k: **~1,5 caracteres por token** (Mellum parte los dígitos uno a uno y los volcados con número de línea son carísimos). Se añadió `Config::chars_por_token` (por defecto 3,0; el adaptador de host lo fija con `SOSO_AGENTE_CHARS_POR_TOKEN`) y en 11c el 422 desaparece: R35 y R36 terminan, pero las rechazan las pruebas. Se elimina el fallo de infraestructura, no el de fondo.
- **Las ejecuciones largas de las primeras pruebas eran de infraestructura, no del modelo:** con el offload por operación a la GPU Mellum iba más lento que en CPU y una tarea agotó su hora con 15 llamadas. La capa MoE entera en GPU llevó el decode de 10,7 tok/s (CPU) a ~170 y el prefill de 13 a ~435; cada tarea pasó de agotar la hora a durar 6–18 min (detalles en la memoria del proyecto, `soso-mellum-gpu-capa-moe`).

**Qué no se ha probado.** Que el agente v2 sea mejor o peor que el v1 (ni siquiera con el recorte activo hay datos suficientes); que Mellum con otra plantilla o con el pensamiento acortado rinda más; la recuperación tras `kill -9` y la base final con este modelo.

**Estado del agente.** El agente v2 (con `chars_por_token`) sigue sin versionar y es neutro en resultados; queda como candidato, la decisión de hacer commit es de quien lo revise.

## Campaña 12 — Gemma 4 12B con agente v3 y reintentos (banco R31–R40, 2026-10-15/16)

**Qué se probó.** Dos palancas sobre el mismo banco (R31–R40, ya visto: ver aviso al final) y el mismo modelo (Gemma 4 12B por soso, GPU del host, ahora con prefill ~350 tok/s): (1) un **agente v3** que se niega a terminar sin haber editado nada (aviso con presupuesto propio, `avisos_sin_editar`), añade un paso «relee TASK.md punto por punto» al prompt y **no ejecuta una llamada que ya salió bien desde la última edición** (`repetidas_ok_max`, cuenta aunque se alterne entre dos comandos; con dos repeticiones más cierra el intento); (2) **hasta 3 rondas por tarea** (`ROUNDS=3` en `correr.sh`): si la validación independiente rechaza, se repite desde la misma base con una nota genérica de rechazo en el enunciado (sin nada de la prueba reservada), a T=0 en la ronda 1 y a `TEMP_REINTENTO` (0,7) en las siguientes.

| tanda | cambio | 1.ª ronda | con 3 rondas |
|---|---|---|---|
| 12a | v3, reintentos a T=0 | 2/10 (R34, R36) | 3/10 (+R35) |
| 12b | + guardián de repetición consecutiva, reintentos a T=0,7 | 1/10 (R36) | **5/10** (R32, R34, R35, R36, R37) |
| 12c | guardián «ya salió bien desde la última edición» | 3/10 (R34, R36, R40) | 4/10 (+R32) |

Referencia: Gemma con el agente v2 sobre este banco, un intento: 2/10 (campaña 9). Evidencia ligera en [`resultados-campanas/campana-12*`](resultados-campanas/README.md).

**Lo que se ve.**
- **A la primera, el v3 no se distingue del v2** (2, 1 y 3 de 10 frente a 2 de 10): la variación entre tandas con el mismo agente y T=0 es de ±1–2 tareas (R34 pasó a la primera en 12a y 12c y no en 12b). El ruido viene de la GPU y de los entornos de compilación y prueba de cada tarea, no del modelo, que se sirve de forma determinista.
- **Reintentar a T=0 es inútil:** las rondas 2 y 3 repiten la 1 (R31: 61 s, 61 s, 61 s). Con temperatura 0,7 sí aparecen aciertos nuevos: **+1 a +4 tareas por tanda**, el único efecto grande medido. Con verificación independiente, muestrear varias veces funciona.
- **El aviso «no has modificado nada» sirve a medias:** en Gemma ninguna tarea acabó ya sin editar en 12a y 12b, pero en 12c R35 volvió a cerrar con «el candidato no cambió nada» las tres rondas (14–26 herramientas): el aviso ni siquiera forzó la edición.
- **Bucle de comprobaciones en verde (R39/R40):** Gemma repetía `cargo test -p soso-abi` en verde (el crate tiene 0 pruebas) hasta agotar las 30 herramientas, alternando dos variantes del comando. El guardián de «repetición consecutiva» no lo cazaba; el de «ya salió bien desde la última edición» sí (R40 se acepta en 12 herramientas en 12c).
- **R33 y R38 (`kill -9` inyectado)** no pasan nunca en este banco y sus registros salen sin datos de herramientas; no se ha investigado.
- **Tareas que no pasan en ninguna ronda en ninguna tanda:** R31, R33, R37 (salvo una vez), R38, R39. Coinciden con el patrón de la campaña 9 (rechazos de programación rápidos: edición plausible que pasa las pruebas visibles y no la reservada).

**Conclusión honesta sobre la viabilidad.** Un modelo local de 12 B resuelve entre 1 y 3 de estas 10 tareas a la primera y 3–5 de 10 con tres rondas muestreadas, con un coste de ~1 h por tanda. **Eso es utilizable como generador de candidatos con validación independiente, no como agente autónomo sin ella.** La cifra con reintentos **no** es comparable con el mínimo de 5/10 a un intento de SI-4, y la comparación entre las tandas v3 y la v2 no demuestra que el v3 sea mejor.

**Avisos.** El banco R31–R40 se había usado ya con Gemma (campañas 8–10) y con Mellum (campaña 11), y los cambios del v3 salen de ver fallos de esas campañas: está contaminado como banco ciego. No se ha ejecutado la comprobación de la base final ni la recuperación tras `kill -9` con el v3. El agente v3 sigue sin versionar.

## Campaña 13 — Qwen3-Coder-30B-A3B (modelo más capaz) con flujo de expertos (banco R31–R40, 2026-10-16)

**Qué se probó.** El mismo banco y el mismo agente v3 con hasta 3 rondas (T=0 en la primera, 0,7 en las siguientes) que la campaña 12, pero con un modelo mucho mayor: **Qwen3-Coder-30B-A3B** (30 B de parámetros, 3 B activos, 128 expertos), que no cabe en los 12 GB de VRAM. Se sirve con el **flujo de expertos** (plan de VRAM con 12 de 48 capas de expertos residentes; las demás traen sus expertos por demanda a un pool de 265 huecos con LRU; ver la memoria del proyecto `soso-mellum-gpu-capa-moe`). Con ~19 GB de RAM libres: decode ~8–9 tok/s, prefill ~200 tok/s marginal.

| tarea | rondas | resultado |
|---|---|---|
| R32 | 3 | aceptada en la ronda 3 |
| R35 | 3 | aceptada en la ronda 3 |
| R36 | 1 | aceptada |
| R40 | 1 | aceptada |
| R31, R33, R34, R37 | 3 | rechazadas por la prueba reservada en las tres rondas |
| R38, R39 | 3 | agotan las 30 herramientas (R39 en las tres rondas) o rechazadas |

**2/10 a la primera y 4/10 con 3 rondas.** Gemma 4 12B con el mismo agente y las mismas rondas: 1–3/10 y 3–5/10 (campaña 12). **El modelo 2,5× mayor no mejora el resultado en este banco.**

**Lo que se ve.**
- **Las mismas tareas se atascan con tres modelos de familias distintas** (Gemma 12 B, Mellum 12 B y Qwen3-Coder 30 B): R31, R33, R34, R37, R38 y R39 no pasan casi nunca. El enunciado está bien especificado (R34 detalla el comportamiento ante NaN, infinitos y empates); lo que falla son los **casos límite** de la prueba reservada. No es un problema de capacidad bruta ni de herramientas.
- **R39 sigue agotando las 30 herramientas en las tres rondas** pese al guardián de repeticiones con éxito: es otro bucle, no una repetición exacta.
- **Con los reintentos a T=0,7 hay rondas idénticas** (R34: 180, 182 y 182 s; R35: 259 y 259 s): cuando el modelo está muy seguro, la temperatura no cambia la salida.
- **Coste:** ~3 h para las 10 tareas con hasta 3 rondas. Decodificar a 8–9 tok/s limita; el prefill ya no.

**Conclusión sobre la viabilidad.** Subir de 12 B a 30 B-A3B no mueve la aguja: la tasa de acierto con reintentos sigue en torno al 40 %. Lo siguiente con más probabilidad de funcionar no es un modelo mayor sino **mejor uso del enunciado y de las pruebas**: que el agente escriba pruebas propias para cada caso límite descrito antes de dar la tarea por hecha. Aviso: el banco ya estaba contaminado como ciego (ver campaña 12).

**Operativa aprendida:** el 30B sólo es utilizable con ≥ ~14 GB de RAM libres para la caché de páginas; con el escritorio cargado cae a ~1 GB/s de lectura, y a las 11:40 del 2026-10-16 el OOM killer mató Cursor mientras corría. El vigilante de la campaña ahora para el servidor si baja de 1,5 GB disponibles.

## Campañas 22–29 — «pruebas propias», interfaz de herramientas y tope de mensajes (banco R31–R40, 2026-10-17/18)

**Qué se probó.** Gemma 4 12B por soso (GPU del host), el mismo banco R31–R40 (`campaign-12.json`: 60 herramientas por intento y, aunque se pidieran 60, **40 turnos efectivos**: ver el defecto del arnés más abajo), una sola ronda salvo la 29. Dos palancas: (1) un prompt que obliga a **escribir primero pruebas propias** por cada caso límite del enunciado (`SISTEMA_AGENTE=pruebas`) y (2) **arreglos de la interfaz de herramientas** (agente v5–v11), hallados mirando las conversaciones de las tareas que fallaban.

| campaña | agente / prompt | resultado | validez |
|---|---|---|---|
| 22 | v4, pruebas propias, 30 herramientas, 3 rondas | ≈0, presupuesto agotado, se paró | válida pero inútil: 30 herramientas no bastan para escribir pruebas y arreglar |
| 23–26 | v4/v7, 60 herramientas | — | **inválidas**: el servidor valida `MAX_MENSAJES` = 64 y devolvía 400 («messages: 71 supera el máximo de 64») en 5 de 10 tareas |
| 27 | v10, pruebas propias | **1/10** (R40) | válida: 3 `LimitePasos` (R31, R32, R38), 2 «no cambió nada» (R35, R36), 4 rechazadas |
| 28 | v10, prompt estándar | **3/10** (R32, R36, R40) | válida: 0 errores de entorno ni `LimitePasos` |
| 29 | v11, prompt estándar, 3 rondas (T=0,7 desde la 2.ª) | **3/10** a la primera (R32, R36, R40) y **3/10** con rondas | válida: R35 agota los turnos en las rondas 2 y 3 (lecturas grandes y lentas) |

Evidencia ligera en [`resultados-campanas/campana-27-*` y `campana-28-*`](resultados-campanas/README.md). Las carpetas completas de las tandas descartadas siguen en local con sufijo (`-tope-64-mensajes`, `-sin-historial`, `-edicion-nula`, `-alternancia`, `-bucles`, `-parcial`).

**Defectos del agente y de las herramientas que falseaban las campañas** (todos con test; ninguno era del modelo):
- **Tope de 64 mensajes del servidor.** Con 60 herramientas la conversación pasaba de 64. `Config::mensajes_max = 60` pliega los intercambios más antiguos. Plegar sin más hacía que el modelo olvidara sus ediciones y las repitiera hasta agotar los turnos (campaña 27, primer intento), así que cada llamada plegada deja **una línea de historial** en la tarea.
- **Ediciones sin efecto que devolvían «Hecho»** (27 veces la misma línea en R31): ahora son error. Igual `buscar` + líneas añadidas que ya están (R31 de la campaña 28 duplicó el mismo guardia ~30 veces), y una `linea` lejos de la única aparición de `buscar`, que editaba otra línea en silencio.
- **Guarda de repetición ciega a los fallos alternados** (A, B, A, B…): agotaba los turnos (40 efectivos). Ahora cuenta los fallos por llamada desde la última edición correcta; con 8 llamadas bloqueadas el intento se cierra y pasa al validador.
- **Turnos recortados a 40 en silencio (dos defectos, binarios v12 y v18).** Primero: el modo `tarea` **no leía `--max-turnos`** (sólo lo leía el subcomando `agente`), así que todas las campañas hasta el binario v17 corrieron con los 40 turnos por defecto aunque se pidieran 60, 80, 120 o 150 (el registro de R31 dice `"pasos": 40` con `LimitePasos`); se vio al lanzar el equipo con 120. Segundo: `ejecutar_intento` construía `Limites` con `max_pasos` = 40 por defecto, y `ejecutar_agente` hace `max_turnos = min(--max-turnos, 40)`. Todas las campañas con `MAX_TURNOS=60` o 150 corrieron en realidad con 40 turnos: por eso los intentos de modelos que piensan acababan en `LimitePasos` con 38–39 herramientas (los turnos se van en respuestas vacías y reintentos de formato, no en llamadas). Arreglado en `runner.rs`: `max_pasos = max(--max-turnos, 40)`. Las comparaciones entre las campañas 22–30 siguen valiendo (todas con 40), pero los `LimitePasos` del LFM no prueban que no resuelva esas tareas.
- **`grep` v2** (espacios, mayúsculas, `.*` y `|`, sugerencias de nombres, definiciones primero, resumen por archivo), **`reemplazar_lineas`** (sustituir un rango sin copiar texto), sangría doble y atributo duplicado en `reemplazar_texto`, aviso de **`cargo test` con 0 pruebas** (el 15 % de los `cargo test` salía «exit 0» sin ejecutar nada), más alias de filtro de `cargo test` y emulación de `find`/`ls`/`cat`/`grep` y `cd DIR && orden` en `bash`.

**Lo que se ve.**
- **No se puede afirmar que suba el pass@1.** En este banco, una ronda: agente v2 (campaña 9) 2/10; v3 (campaña 12) 1, 2 y 3 de 10; v10 (campaña 28) 3/10. Está dentro del ruido de ±1–2 tareas ya medido. Lo que sí cambia es que **desaparecen los resultados falseados** por el agente: sin ello, las cifras anteriores podían estar subestimadas, y no sé cuánto.
- **Las rondas con T=0,7 no añadieron nada en la campaña 29** (3/10 a la primera y 3/10 con tres rondas), cuando en la campaña 12 sumaban entre +1 y +4. Con el agente corregido, los fallos que quedan son los de las tareas de casos límite (R31, R33, R34, R37, R38, R39 rechazadas en las tres rondas): muestrear más no los resuelve. La ganancia por reintentos medida antes puede haber venido en parte de arreglar intentos que fallaban por defectos del agente.
- **El prompt de «pruebas propias» empeora a Gemma** (1/10 frente a 3/10 con el mismo binario; ya casi 0 en la 22). La hipótesis de la ficha T29 («que el modelo trabaje la especificación») no se cumple con este modelo: escribe pruebas que no compilan y se queda iterando sobre ellas.
- **Las tareas atascadas lo siguen estando** (R31, R33, R34, R37, R38, R39 con tres familias de modelos): los fallos son los casos límite de la prueba reservada.
- **Un intento que agota los turnos cuenta como error y no se valida,** aunque deje un cambio correcto en disco. Es decisión de protocolo, no se ha cambiado.

**Avisos.** El banco R31–R40 **ya no es ciego**: las correcciones salen de leer las conversaciones de R31, R32, R34 y R38. Binarios: v10 (campaña 28), v11 (umbral de 8 bloqueadas; sólo probado con tests y en la campaña 29). Las campañas 27 y 28 se midieron con el v10, antes del cambio de umbral.

## Campañas 30–32 — LFM2.5-24B-A2B (Liquid AI) por llama.cpp (banco R31–R40, 2026-10-18)

**Qué se probó.** Un modelo nuevo: **LFM2.5-24B-A2B**, MoE de 24 B con ≈2,3–2,45 B activos y 40 capas híbridas (30 de convolución corta, 10 de atención; 64 expertos, top-4), que **siempre razona** antes de responder. Cuantización Q4_0 QAD (13,6 GB) servida por **llama.cpp** (commit `5aeac37`, compilado con CUDA para sm_120 en un contenedor, binarios y librerías en `target/lfm-work/out/bin/`), porque soso no tiene la arquitectura y portarla era el paso caro. Mismo banco, agente v11/v12 y prompt estándar que la campaña 28.

| campaña | cambio | resultado |
|---|---|---|
| 30 | v11, una ronda, `MAX_TOKENS=8192` | **2/10** (R36, R40); 3 `LimitePasos` (R35, R37, R39) |
| 31 | repetición de R35, R37 y R39 con `--max-turnos 150` | sin efecto: el arnés lo recortaba a 40 (ver el defecto en «Campañas 22–29») |
| 32 | repetición de las tres con `--max-turnos 120` (que en realidad eran 40) | R37 y R39 **rechazadas**; R35 vuelve a agotar los turnos: la diferencia con la 30 es variación entre ejecuciones, no más turnos |

**Lo que se ve.**
- **Más rápido, no mejor.** Prefill ≈1500 tok/s y decodificación ≈65–80 tok/s frente a ≈350 y ≈31 de Gemma por soso; cada intento tarda 5–15 minutos. Aun así **no resuelve más tareas** (2/10 frente a 3/10 de Gemma en las campañas 28 y 29). Es lo mismo que con Qwen3-Coder-30B: un modelo mayor o más rápido no mueve este banco.
- **El pensamiento se come los turnos.** Algunas respuestas llegan al tope de 8192 tokens razonando y salen vacías, y el agente las cuenta como turno: R35 agota los 40 turnos con 40 herramientas. Las campañas 30 y 32 no probaron «más turnos» (el flag no se leía): R37 y R39 pasan de `LimitePasos` a rechazadas sólo por variación entre ejecuciones. Habría que repetir con turnos reales.
- **Fallos del modelo, no de las herramientas.** En R31 escribió una corrección, vio fallar un test y reemplazó el rango entero por el código original: el candidato quedó sin cambios. Las llamadas a herramientas salen bien formadas.

**Conclusión.** No se justifica portar el LFM a soso por ahora: cabe esperar el mismo 2–3/10 con tres familias de modelos en este banco. Lo que podría cambiar el resultado es un banco nuevo o un uso distinto del razonamiento (presupuesto de pensamiento, o no contarlo como turno), no otro modelo.

**Avisos.** El banco R31–R40 ya no es ciego. Una ronda por tarea: el ruido de ±1–2 tareas impide distinguir 2 de 3. El LFM se sirvió por llama.cpp, no por soso.

## Campañas 33–35 — que el agente revise lo que ha hecho (banco R31–R40, 2026-10-18/19)

**Qué se probó.** Dar al agente una forma de **evaluar y revisar su propio trabajo** antes de terminar, con Gemma 4 12B por soso y una ronda. Se añadió la herramienta `revisar` (diff de lo editado respecto al estado inicial de cada archivo, más la sección «Qué se pide» de `TASK.md`) y dos modos de imponerla (`agente/revision.rs`):

| campaña | modo | resultado |
|---|---|---|
| 33 | `REVISAR_FINAL=1`: el agente ve su diff y los requisitos y se revisa a sí mismo (40 turnos) | **1/10** (R40) |
| 34 | `REVISOR=1`, 40 turnos | descartada: R31 y R32 agotan los 40 turnos revisando y corrigiendo |
| 35 | `REVISOR=1`, 80 turnos pedidos (**40 efectivos**): una petición aparte al mismo modelo, sin la conversación y sin herramientas, lista hasta cinco fallos con una entrada concreta y el agente la recibe | **2/10** (R36, R40); R31 y R32 `LimitePasos` |

Referencia sin revisión: 3/10 en las campañas 28 y 29 (misma configuración, 40 turnos).

**Lo que se ve.**
- **Autorrevisión = confirmación.** El agente contesta «he revisado y todo está cubierto» sin cambiar nada (R36 y R40 en la 33); R36 falla aun así la prueba reservada. El sesgo viene de revisar dentro de la misma conversación.
- **El revisor independiente sí encuentra huecos plausibles** (por ejemplo, un contador de modelos mal validado contra los bytes disponibles), **pero Gemma 12B los parchea mal**: sus ediciones rompen la compilación y gasta decenas de turnos arreglando su propio arreglo. Tareas que sin revisor se rechazaban en ~100 s (R31, R32) pasan a agotar los turnos.
- **No sube el pass@1:** 1/10 y 2/10 frente a 3/10, dentro del ruido de ±1–2 pero sin ninguna señal a favor. Con este modelo, revisar más no equivale a acertar más.

**Defecto de herramienta hallado por el camino** (arreglado, con test): cuando `reemplazar_lineas` rechazaba un rango por llaves desequilibradas, el mensaje no decía dónde acababa el bloque, y el modelo repetía la misma llamada ~13 veces. Ahora añade «el bloque que empieza en la línea N termina en la M».

**Conclusión.** La capacidad de corregir un parche complejo es el límite, no la información: ni ver su diff ni un revisor aparte la suplen. Sería más prometedor un revisor más fuerte que el agente (otro modelo) o una evaluación objetiva (pruebas que el agente pueda ejecutar), y ambas exigen un banco nuevo para medirse. Opciones activables: `--revisar-final` y `--revisor` (por defecto apagadas).

**Avisos.** Misma prueba de una ronda sobre el banco R31–R40 (ya no ciego). El revisor es el mismo Gemma 12B, no un modelo distinto. La campaña 34 con 40 turnos se conserva en local con sufijo `-40turnos-parcial`.

## Campañas 36–38 — equipo planificador + programador + revisor (banco R31–R40, 2026-10-19)

**Qué se probó.** El agente de siempre como **programador**, precedido por un **planificador** de sólo lectura (`read`, `grep`, `glob`) que entrega un plan, y seguido por un **revisor independiente** (petición aparte con `TASK.md` y el diff acumulado). Si la última comprobación del programador falla o el revisor señala fallos, el planificador replantea la corrección con el diff y los problemas, y el programador vuelve a empezar con una conversación nueva; hasta `--ciclos` (3) o hasta que todo pase (`agente/orquestador.rs`, `--multiagente`, `MULTIAGENTE=1`). Gemma 4 12B por soso, `campaign-13.json` (120 herramientas y 2 h por intento en total), una ronda.

| campaña | binario | qué pasó |
|---|---|---|
| 36 | v16 | R31 `LimitePasos` por una **oscilación**: el modelo alternaba la misma línea entre dos versiones (cada edición «Hecho»); se añadió un detector (huella de contenido por archivo, aviso y cierre a la tercera) |
| 37 | v17 | R31 `LimitePasos` con `"pasos": 40` aunque se pedían 120: **el modo `tarea` no leía `--max-turnos`** |
| 38 | v18 | con 120 turnos reales: R31 71 min, 114 herramientas, `LimitePasos`; R32 5 min «el candidato no cambió nada». Se paró tras dos tareas |

**Lo que se ve.**
- **El planificador no planifica bien con este modelo.** Su «plan» de R32 es un monólogo con «*Corrección:*» y «*Estrategia:*» a mitad de texto; el programador lo recibe, relee el mismo tramo de `fs.rs` y las guardas cierran el intento sin cambios. En R31, el planificador del ciclo siguiente planifica sobre un `rel_path` «dummy» que había metido el propio programador antes.
- **Coste:** 71 min por una tarea difícil; las diez habrían sido más de 10 h.
- **No hay cifra de pass@1** (dos tareas). Es una señal mala, no un veredicto.

**Defecto del arnés que invalida lecturas anteriores.** Hasta el binario v17 el modo `tarea` ignoraba `--max-turnos`: todas las campañas corrieron con los 40 turnos por defecto aunque se pidieran 60, 80, 120 o 150 (ver el bullet «Turnos recortados» en «Campañas 22–29»). Las comparaciones entre campañas siguen valiendo (todas con 40), pero «con más turnos» **nunca se probó**: las campañas 32 (LFM) y 35 (revisor, «80 turnos») deben leerse con 40.

**Pendiente con turnos reales:** repetir el revisor solo (`--revisor`, 80 turnos) y las tres tareas del LFM (R35, R37, R39). Un planificador más capaz que el programador (otro modelo) es lo que haría falta para que el equipo tenga sentido.

## Campañas 39–45 — herramientas de edición, puertas antes de terminar y análisis de rechazos (banco R31–R40, 2026-10-19)

**Qué cambió** (Gemma 4 12B, 40 turnos, 1 ronda; binarios v19–v31):
- Comprobación de **sintaxis** (rustfmt por stdin) en cada edición, rechazo de `mod pruebas_agente` duplicado, de cambios casi solo comentarios y de monólogos de dudas en comentarios, restauración de los espacios originales.
- Puertas al terminar: pruebas propias obligatorias (y ejecutadas), `comprobar_al_terminar` (órdenes de «Qué tiene que pasar»), alertas de pánico con **Clippy** (`indexing_slicing`, `arithmetic_side_effects`) sobre la función tocada, `validar_al_agotar`, **poda** de pruebas propias en rojo o que no compilan, redirección ante lecturas en bucle.
- Protocolo: tras cada campaña se analiza cada rechazo con `tests/self-improvement/campana/analizar.py` (verifica la copia contra las pruebas reservadas; actualiza mtime y usa `target/analisis-target`).

| campaña | pass@1 |
|---|---|
| 39 / 40 / 41 / 42 / 43 | 1 / 1 / 0 / 1 / 1 |
| 45 (v31) | 1 (R40) |
| 46 (v32) | 2 (R39, R40) |
| 47 (v33) | 2 (R39, R40) |
| 48 (v35) | 2 (R39, R40) |

**Campaña 45.** Nueve rechazos: siete por requisito o caso límite sin cubrir (R31, R33, R34, R36, R37, R38, R39), R32 por una función a medio escribir tras un monólogo de dudas dentro del código y bucles de `read` sobre un símbolo inexistente, y R35 por una interfaz (`TensorInfo::iter`) que la prueba reservada espera. R39: Clippy avisó una sola vez, el agente arregló la multiplicación y dejó `off + BLOCK_SIZE` sin proteger.

**Recuperación tras caída.** Con la inyección a los 75 s el agente ya había editado y la copia adoptada (medio trabajo) se rechazaba; el «segundo intento» no se abre porque la ejecución queda terminal. Se quitó ese bloque de `correr.sh` y la inyección pasa a los 30 s (`campaign-14.json`): el intento rearranca de cero y R33 recuperó (`recuperada: true`, intento 1 completo).

**Cambios tras el análisis (v32):** el aviso de Clippy se repite hasta 3 veces mientras queden líneas marcadas, e indica el patrón `checked_add` para `off..off + N` y que se corrijan todas las líneas; rechazo de ≥4 comentarios de duda/monólogo.

**Campaña 46 (v32): 2/10.** R39 pasa (el aviso de Clippy repetido le hizo cubrir `off + BLOCK_SIZE`). R32 acabó otra vez con **solo un módulo `pruebas_agente` vacío** (comentarios) y el código sin tocar: la edición «contaba» como edición y el atasco se cerró en vez de redirigirse. R37 mezcla «sin `:`» con «`:` y puerto vacío» (debe ser error). R36 deja pasar las versiones 7+ del manifiesto. Los demás son requisitos omitidos. **Cambio (v33):** un atasco con ediciones que son solo comentarios o un módulo de pruebas vacío se redirige igual que un atasco sin ediciones (`sin_codigo_nuevo`).

**Campaña 47 (v33): 2/10 (R39, R40).** R33 acabó en `error` (`LimitePasos`) tras la recuperación, con la edición en disco: **`hubo_edicion` miraba solo las respuestas «Hecho…» que siguen en la conversación, y al plegar los mensajes viejos esas desaparecían**, así que el intento no se validaba ni se podaba (también afectaba a las puertas de Clippy, comprobación y poda en tareas largas). **Arreglado (v34):** `hubo_edicion` lee también el historial plegado, y el historial conserva siempre (hasta 20) las líneas de ediciones que salieron bien aunque se recorte a 40. R36 implementó solo la mitad de «Qué se pide» (el techo de 1024 capas, no el rechazo de versiones 0 y >6): omisión de un punto, sin remedio general probado (la autorrevisión de las campañas 33–35 no movió el resultado).

**Campaña 48 (v35): 2/10 (R39, R40) — la temperatura adaptativa funciona y no basta.** Las llamadas «sin efecto» o bloqueadas bajaron del 35–40 % (45–47) al **21 %** (bloqueadas 48 → 11), pero fallan **las mismas pruebas reservadas**: R31 (3), R32 (2), R34 (2), R35 (3), R36 (1), R38 (3). Los bucles eran un síntoma, no la causa. Análisis completo en [resultados-campanas/analisis-rechazos-45-47.md](resultados-campanas/analisis-rechazos-45-47.md). En R36 el agente ya escribió `version == 0 || version > 6`, pero **dentro del `else` de la rama v1–v3** (sólo leyó desde la línea 883, no el principio de `parse`), y todo su intento se fue en que su módulo de pruebas compilara en un crate `no_std` (`cannot find macro vec`). **Cambios (v36):** prompt con «lee la función entera; una comprobación de entrada va al PRINCIPIO de la función» y pista automática de `extern crate alloc` cuando falta `vec!`/`Vec`/`String` en un crate `no_std`.

**Lectura honesta.** Entre 0 y 3 de 10 en todas, dentro del ruido (±1–2). Las puertas evitan fallos de forma, pero lo que se rechaza ya es lógica o requisitos omitidos: el límite es el modelo.

## Claude como modelo del arnés con la suscripción (preparado, sin lanzar; 2026-10-20)

Calibración pendiente: si Claude resuelve casi todo el banco con el mismo arnés, el límite es el modelo (Gemma 12B), no el arnés; si falla, el fallo está en el arnés y se verá en sus conversaciones.

- `tests/self-improvement/campana/claude_proxy.py`: proxy OpenAI-chat → `claude -p --safe-mode --tools "" --no-session-persistence --output-format json --json-schema …` (sin herramientas propias de Claude Code, nuestro system prompt, una llamada de herramienta por turno). Se niega a arrancar si hay `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN`/proveedor cloud (facturaría por API), no toca `~/.claude/.credentials.json`, atiende de una en una, tope de llamadas, archivo de parada y registro de consumo por llamada. `--seco` no llama a claude.
- `claude-piloto.sh [TAREAS]`: levanta el proxy y corre `correr.sh` (por defecto R40) con `claude-sonnet-5-5`. **No se ejecuta sin confirmación expresa.**
- Probado sólo con un `claude` de mentira (traducción de mensajes y herramientas, flags, rechazo con API key, tope, parada). **Sin verificar con el CLI real:** que el prompt por stdin con `-p` funcione, la forma de `structured_output`/`usage` en la salida JSON, el comportamiento de `--json-schema` junto con `--tools ""` y la latencia por llamada. El primer piloto lo comprueba.
- **Resultado (campaña 49, 2026-10-20/21): 10/10.** `claude-sonnet-5-5` acepta **todo el banco R31–R40** con el mismo arnés v36, 40 turnos y 1 ronda: R31 251 s/25 herr., R32 562 s/14, R33 149 s/10, R34 172 s/16, R35 209 s/24, R36 126 s/15, R37 101 s/9, R38 303 s/17, R39 143 s/16, R40 103 s/12. Incluye las cinco tareas que Gemma 12B no ha pasado nunca (R31, R33, R35, R37, R38) más R32, R34 y R36. **El arnés no es el límite: lo es el modelo.** Consumo total medido (205 llamadas, con el piloto de R40 y el intento fallido de R31): 7,30 M tokens de entrada y 170 mil de salida; equivalente API 15,36 USD (no se factura con suscripción). Un fallo puntual del CLI (exit 1) en R31 se resolvió reintentando; el proxy ahora reintenta y guarda el motivo. Las tareas aceptadas pasan a ser base de las siguientes (R31–R37 corrieron sobre una base con R40/R36/R38 ya aplicadas; crates distintos). n=1 por tarea y Claude no admite fijar temperatura: es una calibración, no una medida fina.
- Cupo y términos: el consumo sale de la ventana de 5 horas y del límite semanal de la suscripción; el uso automatizado propio y local con el binario oficial sin modificar no aparece contemplado de forma explícita en los términos (zona gris), y compartir el proxy con otras personas no es viable.

## Campaña 50 — Qwen3.6-35B-A3B local (banco R31–R40, 2026-10-21)

Tras la calibración con Claude (10/10, ver arriba) se buscó un modelo local más capaz que Gemma 12B. Elegido **Qwen3.6-35B-A3B** (MoE 35B/3B activos, Apache-2.0, 73,4 en SWE-bench Verified según su ficha; tool calling nativo): GGUF `unsloth/Qwen3.6-35B-A3B-GGUF` **UD-IQ3_XXS (13,2 GB)** con el llama.cpp CUDA ya compilado (`target/lfm-work/out/bin`, commit 5aeac37, que contiene `qwen35moe`).

- **Servidor:** `tests/self-improvement/campana/qwen36-servir.sh` (puerto 7460, `-ngl 99 --n-cpu-moe 14 -c 65536 -fa on`, `enable_thinking=false`, T=0,7 top-k 20 top-p 0,8). 10,4 GiB de VRAM. Medido: decode 62–70 tok/s, prefill ≈1500 tok/s con 31 k tokens. Una tarea tarda 2–17 min.
- **Arnés:** v37, 40 turnos, 1 ronda, `TEMP_BASE=0.7` (variable nueva de `correr.sh`), `max_tokens` 8192 (con 4096 el modelo cortaba a mitad de un `insertar` de un módulo de pruebas).
- **Fallo de infraestructura encontrado y arreglado:** llama-server contesta **500** («Failed to parse tool call arguments as JSON… missing closing quote») cuando la llamada llega cortada; el arnés sólo reintentaba el 400 del servidor propio y R38 acabó en `ErrorModelo`. Ahora se devuelve al modelo con el consejo de acortar (v37, con test).

**Resultado: 5/10 — R34 (431 s/28 herr.), R36 (831 s/40), R37 (638 s/29), R39 (464 s/37), R40 (125 s/14).** Rechazadas: R31 (fallan las mismas 3 pruebas reservadas que con Gemma: `cabecera_de_modelo_justa_no_basta`, `corrupcion_aleatoria…`, `cualquier_truncado…`), R32 (no compila con las pruebas reservadas), R33 (3 pruebas), R35 (1 prueba: `tensor_demasiadas_dimensiones`, a un paso), R38 (agotó los 40 turnos sin acabar gzip; tras la caída inyectada a los 30 s recuperó bien). **Gemma 12B: 2/10; Claude Sonnet 5.5: 10/10; Qwen3.6-35B-A3B (IQ3): 5/10.** R34, R36 y R37 no las pasa Gemma. n=1, T=0,7: la dispersión entre repeticiones no está medida.

**Campaña 51 — la misma con razonamiento activado: otra vez 5/10, pero otras cinco.** Servidor con `RAZONAR=1` (modo thinking, `--reasoning-budget 4096`, T=0,6/top-p 0,95/top-k 20, `max_tokens` 16384); 5/10: R31 (1424 s), R33 (1251 s), R37 (216 s), R39 (612 s), R40 (121 s). Las tareas tardan más (12–27 min). **Comparación con la 50 (sin razonamiento):** comunes R37, R39, R40; sólo con razonamiento R31 y R33; sólo sin él R34 y R36; la unión es 7/10 (R31, R33, R34, R36, R37, R39, R40). El razonamiento no mejora el total, y la variación tarea a tarea es grande (n=1 por tarea): el 5/10 no distingue entre las dos configuraciones.

**Hallazgo de la 51: ediciones fuera de «Qué puedes tocar» se perdían en silencio.** R35 dio «rechazada» aunque su copia pasa las pruebas reservadas (`verificar repo` sobre la copia → ok): arregló `crates/gguf2som/src/io.rs` y sólo se exporta `lib.rs`; R32 (campañas 50 y 51) y R36 editaron `tests/roundtrip.rs`. En 4 de 70 intentos de las campañas 45–51, todos de Qwen y todos rechazados, la edición salía «Hecho» y se descartaba al exportar. **Arreglo (v38):** las cuatro herramientas de edición rechazan la ruta que no figure en la sección «Qué puedes tocar» de `TASK.md` con la lista de permitidos (sin esa sección no hay restricción). Con v38, R35 habría pasado. Las otras dos rechazadas «no compila con las pruebas reservadas» (R34: `Option<&[u32]>.copied()`; R38: `Option::ok()`) cambian el tipo de retorno de una función cuando la tarea pide no cambiar la firma: sin puerta todavía.

**Campaña 52 (v38): inválida.** Dio 2/10 con tres `error` a los 3600 s (R33, R35, R38) porque Chrome, Cursor y un `/tmp` en tmpfs agotaron la RAM y el swap (decode 6–25 tok/s en vez de 65–70). No mide nada de v38; la carpeta quedó como `ejecucion-52-…-invalida-por-memoria`. Ahora el monitor avisa si la RAM libre baja de 4,5 GB o el swap pasa de 4 GB.

**Campaña 53 (v38, Qwen3.6-35B-A3B IQ3 sin razonamiento, T=0,7): 7/10** — R31 (865 s), R32 (791 s), R35 (394 s), R36 (334 s), R37 (115 s), R39 (369 s), R40 (57 s). Rechazadas R33, R34 y R38. Con v38 las llamadas «sin efecto»/bloqueadas bajan al 0 % de los resultados de herramienta (1 de 242) y no hay ediciones fuera de alcance. R35 y R32, que en la 51 se perdían por editar `io.rs` y `tests/roundtrip.rs`, pasan. **Resumen del banco: Gemma 12B 2/10 · Qwen3.6-35B-A3B 5/10, 5/10 (con razonamiento) y 7/10 (v38) · Claude Sonnet 5.5 10/10.**

**Rechazos de la 53 y puerta de API pública (v39).** R33 y R34 no compilan con las pruebas reservadas: R33 **borró `resample_to_16k` de `wav.rs`** al reescribir el archivo (su `copia` no la tiene; la base sí) y R34 y R38 cambian el tipo de retorno de una función cuando la tarea pide no cambiar la firma. v39 añade `proteger_api`: antes de terminar compara las cabeceras de los ítems `pub` de los archivos editados con las originales y, si falta alguna (borrada o con otra firma), se lo dice al modelo (1 vez; `--sin-proteger-api` la apaga) con test.

**Campaña 54 (v39, misma configuración): 6/10 + R38 invalidada.** Aceptadas R31 (947 s), R32 (705 s), R36 (412 s), R37 (283 s), R39 (211 s), R40 (95 s); rechazadas R33 (a una prueba: `estereo_se_promedia`, `attempt to add with overflow` en `wav.rs:77` al sumar dos `i16`), R34 (las mismas dos pruebas `greedy_*` de siempre) y R35 (rechaza valores límite válidos: 8 dimensiones, cadena en el límite). **Ningún rechazo es ya de compilación** (la puerta de API pública no tuvo ocasión de saltar; no consta ningún aviso). **R38 no cuenta:** la inyección de caída a los 30 s pilló a Qwen justo tras su primera edición («BLOQUEADA — la copia ya tiene 1 cambio»), se adoptó la copia a medias y se validó (32 s); con modelos más lentos la caída llegaba antes de editar. `tests/self-improvement/campaign-15.json` es la campaña 14 sin inyecciones; R38 repetida con ella (`ejecucion-54b-R38-sin-inyeccion`, v39): **rechazada igualmente** (321 s, 40 herr.; fallan `gzip_simple`, `deflate_zlib_y_crudo` y `gzip_con_campos_opcionales`, las mismas tres que con Gemma y con Qwen en todas las campañas; el `inflate` existente desborda en `decode.rs:36` y sus propias pruebas no ejercitan datos deflate reales; sólo Claude la resolvió). **Cifra efectiva de la 54: 6/10.**

**v40:** la puerta de Clippy ya no depende de que `TASK.md` hable de pánico: si no lo hace, avisa **una sola vez** y **sólo** de `arithmetic_side_effects` (las operaciones que pueden desbordar, que en las pruebas —compiladas en `debug`— son un pánico), con el consejo de ensanchar el tipo (`i16` → `i32`) o usar `checked_*`/`saturating_*`. Pensada para casos como R33 de la 54. Con test.

**Dónde se va el tiempo (campaña 54, Qwen3.6-35B-A3B): el 85 % es el modelo, no el arnés.** 368 peticiones: prefill 2179 s + decodificación 1829 s de 4732 s totales. **El 87 % del prefill (1890 s) lo causan 100 peticiones que reprocesan ~18 000 tokens**: el plegado de mensajes (`mensajes_max`=60) pliega de uno en uno y reescribe el principio de la conversación (la tarea lleva el historial) en cada turno, con lo que la caché de prefijo del servidor se invalida una y otra vez. **v41:** `plegado_holgura` (16 por defecto) pliega de golpe hasta `mensajes_max - 16`, así el prefijo se mantiene estable unos 8 turnos entre pliegues (test: ≥3× menos reescrituras). El resto de tiempo es decodificación (~5 s por petición).

## Campaña 55 — Qwen3.6-27B **denso** (IQ3_XXS) sobre las cuatro tareas disputadas (2026-10-22)

`unsloth/Qwen3.6-27B-GGUF` **UD-IQ3_XXS (12,0 GB)**; con 12 GB de VRAM no cabe entero con el KV: `qwen27-servir.sh` (puerto 7461) pone **57 de 64 capas en GPU** (59 se queda sin VRAM en el búfer de cómputo), contexto 38 912, KV q8_0, sin razonamiento, T=0,7. **Velocidad: decode 9–13 tok/s (7–8 con el swap lleno) frente a 62–70 del 35B-A3B; prefill ≈835 tok/s frente a ≈1500.** Arnés v41, `campaign-15.json` (sin inyección), `max_tokens` 6144.

**Resultado: R34 aceptada (1374 s/36 herr.), R35 aceptada (2876 s/40), R33 rechazada (1987 s/35), R38 rechazada (2529 s/40).** El arnés las registró como `error: Bloqueado` (R33 y R38). **Causa real (descubierta en la campaña 58):** la carpeta de salida se reutilizó tras una ejecución abortada y `refs/base-campana-Rxx.*` conservaba la base publicada por el intento anterior; la captura nueva partía de otra base y el validador contestó «la base cambió» (no tiene que ver con el swap). Corregido en `correr.sh` (borra esas referencias al empezar cada tarea). Repetidas a mano sobre la copia: **R33 pasa 9 de 10 pruebas reservadas** (falla `otros_formatos_no_se_admiten`) y **R38 no compila** (usa mal `miniz_oxide`: `FlushPending`, `BadData`). Con el 35B-A3B (campaña 54) esas cuatro salieron rechazadas las cuatro, y R35 había pasado en la 53: el 27B denso gana en las difíciles, pero a un coste de **2,4 h para cuatro tareas** (unos 35 min por tarea); el banco entero serían 5–6 h.

**Lectura.** El 35B-A3B es el modelo de trabajo (≈70 min por banco, 6–7/10) y el 27B denso es un modelo de **segunda oportunidad** para lo que el MoE no resuelve. La combinación (MoE primero; las rechazadas, al denso con `ROUNDS=2`) costaría unas 2–2,5 h por banco y podría llegar a 8–9/10; sin medir. Además la memoria manda: con Chrome (8–14 GB) + Cursor la máquina vive al límite y tres veces la guarda paró la campaña o el modelo cayó a 6–8 tok/s.

## Campaña 56 — combinación MoE rápido + denso para lo rechazado: **8/10** (2026-10-23)

`combo-moe-denso.sh 56` encadena dos pasadas con el arnés v41 (sin inyección, T=0,7, 40 turnos, 1 ronda): **A** = banco entero con Qwen3.6-35B-A3B (UD-IQ3_XXS, 75 min) y **B** = las tareas no aceptadas en A con Qwen3.6-27B denso (UD-IQ3_XXS, 57/64 capas en GPU, 3 h 24 min). Total 4 h 39 min.

- **A (35B-A3B): 5/10** — R34 (380 s), R35 (800 s), R36 (297 s), R39 (295 s), R40 (96 s); rechazadas R31 (247 s), R32 (254 s), R33 (1383 s), R37 (227 s), R38 (523 s). El plegado en bloques de v41 recorta mucho el tiempo (R31 pasa de 865–1424 s a 247 s; el banco entero, de 70–100 min a 75 min con las 10 tareas).
- **B (27B denso): 3/5** — R31 (2621 s), R32 (1649 s) y **R38 (3520 s, la que sólo Claude había resuelto)** aceptadas; R37 rechazada (825 s: `la_query_se_conserva_aunque_no_haya_ruta`, la misma prueba que falla con el MoE) y R33 `error` a los 3601 s (plazo de 3600 s agotado esperando al modelo; el estado en disco no compila).
- **Combinado: 8/10** (R31, R32, R34, R35, R36, R38, R39, R40), frente a Gemma 12B 2/10, Qwen35B-A3B solo 5–7/10 y Claude Sonnet 5.5 10/10. **El denso resuelve en la pasada B tres de las cinco que el MoE no resolvía, con ~35 min por tarea.** Pendiente: R33 (falla una prueba con el denso en la 55; en la 56 se agotó el plazo) y R37 (hueco estable). Un plazo mayor que 3600 s (R38 acabó a 3520 s) podría salvar R33; no medido. n=1 por tarea.

**Repetición de R33 y R37 con el denso y 5400 s por tarea (`campaign-16.json`, `denso-solo.sh`): R33 aceptada (1987 s, 38 herr.) y R37 rechazada (607 s, 20 herr.). Combo: 9/10** (R31, R32, R33, R34, R35, R36, R38, R39, R40). El plazo de 3600 s era lo que impedía acabar a R33 con el denso. **R37 es el único hueco:** `la_query_se_conserva_aunque_no_haya_ruta` falla con todos los modelos locales por la misma lectura: «el host termina en el **primer** `/` o `?`» se implementa cortando primero por `/` y luego por `?`, de modo que `https://h.org:8080?q=1/2` deja `h.org:8080?q=1` como host y el puerto no parsea (`Err(Parse)`); sólo Claude toma el mínimo de las dos posiciones.

## Banco ciego 5 (R41–R50): la medida honesta (2026-10-23/24)

Diez tareas mixtas (3 endurecimientos, 3 defectos reales, 2 funcionalidades, 1 rendimiento, 1 ampliación de API; [campana5/](../../tests/self-improvement/campana5/README.md)) redactadas por cinco agentes **sin mirar** los fallos del agente y verificadas con el verificador oficial (tal cual: fallo; `--con-referencia`: ok). Arnés congelado v41 (v42 sólo con arreglos de infraestructura), `campaign-17.json` (5400 s), T=0,7, 40 turnos, 1 ronda, sin inyección.

| Tarea | tipo | MoE 35B-A3B | denso 27B | **combo** | Claude |
|---|---|---|---|---|---|
| R41 gptdisk `Guid::parse` | defecto | ✓ 156 s | – | ✓ | ✓ 174 s |
| R42 gptdisk GPT corrupta | endurecimiento | ✗ | ✗ 1696 s | ✗ | ✓ 375 s |
| R43 wpa2 EAPOL/KDE | endurecimiento | ✗ (×2) | ✓ 1871 s | ✓ | ✓ 268 s |
| R44 semver pre-release | funcionalidad | ✗ | ✓ 1656 s | ✓ | ✓ 209 s |
| R45 http redirecciones | defecto | ✗ | ✓ 1597 s | ✓ | ✓ 147 s |
| R46 web-core percent | funcionalidad | ✓ 333 s | – | ✓ | ✓ 182 s |
| R47 sosofs rendimiento | rendimiento | ✗ | ✓ 1817 s | ✓ | ✓ 1185 s |
| R48 sosomfs API | API | ✓ 118 s | – | ✓ | ✓ 222 s |
| R49 llm-core tokenizador | defecto | ✗ (×2) | ✗ 1454 s | ✗ | ✓ 154 s |
| R50 resize-core | endurecimiento | ✗ | ✗ 2265 s | ✗ | ✓ 199 s |
| **Total** | | **3/10** | **4/7** | **7/10** | **10/10** |

**Lectura.** Con el banco ajustado (R31–R40) el MoE sacaba 5–7/10 y el combo 9/10; con el banco ciego, **3/10 y 7/10**: parte de la cifra anterior era ajuste del arnés a esas diez tareas. Claude da 10/10 en ambos, así que el arnés y el banco están bien. El denso cuesta ≈30 min por tarea (el combo entero, ≈5 h). R42 y R50 son desbordes aritméticos y la puerta de Clippy **sí saltó** (avisos en sus conversaciones) pero los arreglos no cubrieron todos los casos; R49 (streaming con prefijo incompleto) es semántica fina del tokenizador. Los 7/10 del combo son la cifra de referencia del agente local.

**Incidencias de infraestructura (no cambian la estrategia del agente):** (1) **R43 con v41:** una llamada con el JSON roto (el modelo se quedó repitiendo `0`) quedaba en el historial y llama-server contestaba 500 en cada petición: v42 guarda `{}` y devuelve el error al modelo. (2) **Refs de base antiguas:** reutilizar una carpeta tras una ejecución abortada daba «Bloqueado: la base cambió» aunque la solución fuera buena (también explicaba los «Bloqueado» de la campaña 55): `correr.sh` borra `refs/base-campana-Rxx.*` al empezar. (3) **OOM:** una prueba descontrolada del candidato (un binario de `soso-llm-core` llegó a 13,5 GB) mató la unidad entera: `correr.sh` limita la memoria de datos a 8 GB por proceso (`ulimit -d`) y las unidades llevan `OOMPolicy=continue`. (4) **Límite de sesión de la suscripción de Claude** a los ≈25 min de campaña: R46–R50 se repitieron en otra ventana.

## Arnés estable v43 (2026-10-24)

El arnés queda **congelado como estable en v43** (`target/gemma-work/target-v43/debug/soso-improve`, sha256 `f62f230e37af4a69d31bc552f9b1e0562579901d2727ceeae7f4c38848694855`), construido con
`CARGO_TARGET_DIR=target/gemma-work/target-v43 cargo build -p soso-improve`. Incluye: puertas antes de terminar (sintaxis, pruebas propias, comprobación de la
tarea, Clippy de pánico y aritmético, API pública, ediciones fuera de «Qué puedes tocar», poda), plegado de mensajes en bloques, temperatura adaptativa, llamadas con
JSON roto fuera del historial (v42) y captura que conserva `vendor/` salvo `vendor/rust/` (v43). Las campañas se lanzan con `correr.sh` (refs de base limpias y tope
de 8 GB de memoria de datos por proceso). **Cualquier cambio del arnés tras este punto invalida una medida en curso**: se hace entre medidas y se anota aquí.

---

**Nota sobre la evidencia de las campañas 6–13:** las carpetas `tests/self-improvement/campana/ejecucion-9` a `ejecucion-17-*` están en el `.gitignore` (sólo locales). Sus resúmenes, registros por tarea, comprobaciones de la base final e incidencias están versionados en [`resultados-campanas/`](resultados-campanas/README.md).


## Campaña 75 — Qwen3.6-35B-A3B servido por **soso** (banco ciego 5, R41–R50; 2026-10-26)

Misma configuración que la pasada A del combo (arnés v43, T=0,7, `MAX_TURNOS=40`, 8192 tokens de salida, `top_p 0,8`, `top_k 20`), pero con `serve_host` de soso (CPU AVX2 + CUDA) en lugar de llama.cpp. Pesos: el mismo GGUF UD-IQ3_XXS convertido con `convert-gguf` (`som-35b-iq3`).

| Tarea | Resultado | Tiempo |
|---|---|---|
| R41 | rechazada | 31 min |
| R42 | rechazada | 72 min |
| R43 | sin aceptar (límite de 40 llamadas a herramientas) | 40 min |
| R44 | rechazada | 31 min |
| R45 | rechazada | 37 min |
| **R46** | **aceptada** | 31 min |
| R47 | bloqueada (el candidato no cambió nada) | 24 min |
| R48 | rechazada | 21 min |
| R49 | sin aceptar (límite de llamadas) | 19 min |
| R50 | rechazada | 30 min |

**1/10 frente a 3/10 de llama.cpp con el mismo modelo y banco** (campaña 50-bis, banco ciego 5). Con n=10 y T=0,7 la diferencia no es significativa (con p=0,3, P(≤1 de 10) ≈ 0,15), pero tampoco acredita la paridad de calidad. Tiempo total ≈ 4 h (llama.cpp, ~2 h): ~2× más lento. 376 respuestas del servidor, 19 de más de 150 s (reprocesos tras un pliegue del historial).

Antes de este resultado hubo cuatro tandas descartadas por problemas de infraestructura de soso (no del modelo): presupuesto de 5400 s agotado por prefill lento (47 tok/s), `head_dim` 256 sin kernel AVX2, sin `top_k 20`/`top_p 0,8` por defecto y el 422 por `prompt + max_tokens` > contexto. Lo que movió el rendimiento está en `PLAN-PUESTA-EN-MARCHA.md` (A3/A4).
