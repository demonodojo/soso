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
