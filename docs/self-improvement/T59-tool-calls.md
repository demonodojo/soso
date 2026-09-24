# T59 — Las llamadas a herramienta llegan como texto, no como `tool_calls`

**Hito:** SI-2 / SI-3 · **Tipo:** Investigación · **Estado:** cerrada (2026-09-24) · **Resultado: no era el servicio, es el modelo.**

**Dependencias:** ninguna. **La origina:** [T14](T14-evaluacion-modelo.md), la
campaña del 23-sep-2026. **Bloquea:** el go de T14 y, con él,
[T16](T16-servicio-guest.md) y [T22](T22-primera-mejora.md).

## Problema reproducido

El modelo **sí** intenta usar la herramienta. Lo que no ocurre es que el
servicio lo reconozca: la llamada sale como texto plano dentro de `content`, y
la respuesta no trae `tool_calls` ni `finish_reason: "tool_calls"`.

Campaña T14, casos Q04 y Q07, idénticos en las tres repeticiones:

```
Q04  mal  (3/4 aserciones; contenido_contiene: el contenido no incluye "13:45":
          "{\"name\": \"obtener_hora\", \"arguments\": {\"zona\":\"local\"}}")
Q07  mal  (2/4 aserciones; llamada_unica: se esperaba exactamente una llamada y
          hay 0; finish_reason: finish_reason None, se esperaba "tool_calls")
```

Reproducción:

```sh
cargo run -p soso-improve -- evaluar --modelo <catalogo> \
  --banco tests/self-improvement/cases --token <t> --repeticiones 1
```

Al abrir la ficha se dio por hecho que era un fallo **del servicio**. La sonda
del paso 1 lo desmiente en parte: ver más abajo.

## Paso 1 ejecutado: el render está bien (2026-09-23)

Sonda: `cargo run -p soso-llm-api --features std --example render_dump -- \
target/qwen2.5-coder-3b-model tests/self-improvement/cases/visible/Q0N/peticion.json`.

**Q07** (una petición, `tool_choice: "required"`): el prompt trae la sección
`# Tools`, la firma de `leer_archivo` dentro de `<tools>` y la instrucción
explícita de responder dentro de `<tool_call></tool_call>`. Correcto.

**Q04** (ida y vuelta de herramienta): el prompt trae la llamada previa del
asistente y —lo que importa— **el resultado de la herramienta**:

```
<|im_start|>user
<tool_response>
13:45
</tool_response>
<|im_start|>assistant
```

El modelo tiene el `13:45` delante y aun así vuelve a emitir la llamada. Para
Q04, **el fallo es del modelo, no del servicio**.

### Dos hipótesis descartadas por el camino, y cómo

1. «Falta `<|im_end|>`»: el texto decodificado no lo enseña… porque `decode`
   oculta el EOS. Mirando los **ids** aparece `151645` en cada cierre. El
   prompt está bien formado; era un artefacto de la impresión.
2. «`decode` se traga las etiquetas `<tool_call>`»: no. `token_bytes` sólo
   omite `bos` y `eos`; el token 151657 se decodifica como su texto, y de hecho
   se ve en el volcado.

Las dos lecciones son la misma: **no concluir del texto decodificado lo que hay
que mirar en los ids**.

## Q07 resuelto con el documento crudo (2026-09-24)

Se añadió el documento crudo por intento al informe y un filtro `--caso`, y se
repitió sólo Q07 (cuatro minutos en vez de tres cuartos de hora):

```sh
cargo run -p soso-improve -- evaluar --modelo qwen2.5-coder-3b-merges   --banco tests/self-improvement/cases --token … --caso Q07 --repeticiones 1
```

El servidor contestó:

```
HTTP/1.1 400 Bad Request
{"error":{"message":"tool_choice inválido: tool_choice required pero no hay llamada",
          "type":"invalid_request_error","code":"invalid_request"}}
```

Es decir: con `tool_choice: "required"`, **el modelo no emitió ninguna llamada**
y el servicio lo detectó y lo rechazó **correctamente**. La premisa con la que
se abrió esta ficha —«el servicio no convierte la salida en `tool_calls`»— es
falsa.

## Conclusión: es el modelo

En los dos casos que el banco ejercita, el Qwen2.5-Coder-3B **no usa las
herramientas bien**:

- **Q04**: con el resultado de la herramienta delante (`13:45`), repite la
  llamada en vez de contestar.
- **Q07**: con `tool_choice: "required"` y la instrucción explícita de usar
  `<tool_call>`, no emite ninguna llamada.

El render es correcto y el servicio detecta la violación. Esto es un **límite
del modelo**, y como tal va al informe de [T14](T14-evaluacion-modelo.md).

Importa más allá de T14: [SI-3](../../SELF_IMPROVEMENT.md) necesita un agente
que llame a herramientas. Con este modelo y este banco, eso todavía no se
sostiene, y conviene saberlo antes de construir encima.

## Lo que sí era del servicio, y se lleva T60

Al mirar el documento crudo apareció otra cosa: el envoltorio dice
`http_status: 200` y el contenido es una respuesta `400` **completa**. La causa
es que `post_chat_stream` escribe las cabeceras SSE con 200 **antes** de
generar, y si algo falla después, `atender_http` añade una segunda respuesta
HTTP entera sobre la misma conexión. Un cliente que esté leyendo el flujo
recibe una respuesta HTTP como si fueran datos del stream.

Registrado en [T60](T60-validacion-peticion.md); no se arregla aquí.

## Causa: ni el render ni el parser

La sospecha inicial —que las herramientas no se renderizaban con el formato que
Qwen espera— la descartó la sonda. La segunda —que el servicio no convertía la
salida en `tool_calls`— la descartó el documento crudo de Q07. Queda la
tercera: el modelo.

## Contexto mínimo

- `crates/soso-llm-core/src/conversation/render.rs` — cómo se meten las
  herramientas en el prompt.
- `crates/soso-llm-core/src/conversation/tools.rs` — `ToolCallParser`, las
  etiquetas `OPEN`/`CLOSE`.
- `user/soso-llm/src/serve.rs` — `post_chat_inner`, `parse_assistant_output`,
  y de dónde sale `finish_reason`.
- `tests/self-improvement/cases/visible/Q04`, `Q07` y sus `esperado.json`
  reservados.
- `docs/self-improvement/modelo.md` — plantilla oficial del modelo fijado.

## Pasos

1. ~~Volcar el prompt renderizado y compararlo con la plantilla oficial.~~
   **Hecho**: el render es correcto (ver arriba).
1b. Guardar el **documento crudo** de cada intento en el informe de T14: hoy se
   pierde y por eso Q07 no se puede atribuir.
2. Corregir donde esté, con una prueba de host que fije el formato esperado.
3. Rellenar `tool_calls` y `finish_reason: "tool_calls"` en la respuesta JSON y
   en la SSE.
4. Reejecutar la campaña de T14 y comprobar Q04 y Q07.

## Comprobación

`cargo test -p soso-llm-core` con fixtures de render y parseo, y la campaña de
[T14](T14-evaluacion-modelo.md) como prueba de sistema.

## Cierre y condición de bloqueo

- [x] Investigación terminada: tres hipótesis, dos descartadas con evidencia.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado: **es el modelo**, con el límite dicho abajo.

**Límite.** Se han ejercitado los **dos** casos de herramienta que el banco
trae. Eso basta para no dar por buena la capacidad, pero no para caracterizar
cuánto falla: si SI-3 va a depender de ello, hace falta un banco de
herramientas más amplio, o un modelo mayor, y esa comparación es de T14 con
otro perfil —sin tocar el banco durante la comparación—.

Si resulta que el modelo no sabe usar herramientas con el formato correcto —y
no que el servicio no se lo pide bien—, **eso es un resultado**, y va al informe
de T14 como límite del modelo, no como una corrección pendiente eterna.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Validación nativa: **pendiente**.
