# T60 — Errores de la API: genera donde debe rechazar, clasifica mal y responde dos veces

**Hito:** SI-2 · **Tipo:** Corrección de userspace (API) · **Estado:** hecha (2026-09-24).

**Dependencias:** ninguna. **La origina:** [T14](T14-evaluacion-modelo.md), la
campaña del 23-sep-2026. **Bloquea:** el go de T14.

## Problema reproducido

Dos casos del banco, idénticos en las tres repeticiones:

**Q10 — genera donde debería rechazar.** El caso describe una petición que el
contrato declara inválida y espera `422`. El servicio la atiende:

```
Q10  mal  (0/3 aserciones;
     estado_http: estado HTTP Some(200), se esperaba Some(422);
     error_presente: se esperaba un objeto error en el cuerpo;
     sin_texto_de_asistente: un error no debe traer texto del asistente)
```

Lo caro no es sólo el código: son **149 segundos de generación** gastados en
una petición que no debía atenderse. Una validación que llega tarde no es un
detalle de forma.

**Q08 — clasifica mal el error.** Rechaza, con el código equivocado:

```
Q08  mal  (2/3 aserciones;
     error_presente: error.code "invalid_request", se esperaba "unknown_tool")
```

`invalid_request` es el cajón de sastre; `unknown_tool` es lo que permite a un
cliente distinguir «te he pedido una herramienta que no existe» de «tu JSON
está mal», que son cosas que se arreglan de formas distintas.

**Q07 — dos respuestas HTTP en la misma conexión.** Encontrado el 2026-09-24 al
guardar el documento crudo de los intentos. El envoltorio grabado dice
`http_status: 200` y su contenido es una respuesta **`400` completa**:

```
HTTP/1.1 400 Bad Request
{"error":{"message":"tool_choice inválido: tool_choice required pero no hay llamada",…}}
```

La causa está en `serve.rs`: `post_chat_stream` escribe las cabeceras SSE con
`200` **antes** de generar, y si algo falla después —aquí, la validación de
`tool_choice`—, `atender_http` añade una respuesta HTTP entera sobre la misma
conexión:

```rust
match dispatch(rt, req, &mut conn) {
    …
    Err(e) => { let _ = write_bytes(conn.fd, &error_response(&e)); }
}
```

Un cliente que esté leyendo el flujo recibe una respuesta HTTP como si fueran
datos del stream. Una vez enviadas las cabeceras, el error tiene que ir
**dentro** del flujo (evento de error y `[DONE]`), nunca como segunda
respuesta.

Reproducción:

```sh
cargo run -p soso-improve -- evaluar --modelo <catalogo> \
  --banco tests/self-improvement/cases --token <t> --repeticiones 1 --caso Q07
```

## Contexto mínimo

- `crates/soso-llm-api/src/` — `ApiError`, `status_code()`, los códigos.
- `crates/soso-llm-core/src/conversation/validacion.rs` — validación de
  historial y esquemas ([T05](T05-validacion-chat.md)).
- `user/soso-llm/src/serve.rs` — `post_chat_inner`, `validate_tool_choice`,
  `error_response`.
- `tests/self-improvement/cases/visible/Q08`, `Q10` y sus `esperado.json`.

## Contrato técnico

- Lo que el contrato declara inválido se rechaza **antes de generar**, con
  `422` y un objeto `error` en el cuerpo, y **sin texto de asistente**.
- `error.code` distingue al menos `invalid_json`, `invalid_request` y
  `unknown_tool`. Un código no puede cubrir tres causas distintas.
- **Una petición, una respuesta HTTP.** Si las cabeceras ya salieron, el error
  viaja dentro del flujo; `atender_http` no puede escribir una segunda.

## Pasos

1. Leer los `esperado.json` de Q08 y Q10 para fijar qué exige el contrato.
2. Mover la validación delante de la generación y devolver 422 con su cuerpo.
3. Añadir `unknown_tool` y usarlo donde corresponde.
4. Que un fallo posterior a las cabeceras SSE se emita **dentro** del flujo, y
   que `atender_http` no pueda escribir una segunda respuesta (por ejemplo,
   marcando la conexión como «ya respondida»).
5. Pruebas de host por cada código, y reejecutar la campaña de T14.

## Comprobación

`cargo test -p soso-llm-api --features std` y la campaña de T14.

## Lo que resultó ser (y lo que no)

De los tres síntomas con los que se abrió la ficha, **sólo dos eran del
servicio**:

| Síntoma | Veredicto | Arreglo |
|---|---|---|
| Q08: `error.code` genérico | **defecto real** | `HerramientaDesconocida` → `unknown_tool` en `ApiError::code()` |
| Q07: dos respuestas HTTP | **defecto real** | si las cabeceras SSE ya salieron, el error va dentro del flujo con `encode_stream_failure` |
| Q10: 200 donde tocaba 422 | **no era del servicio** | era el arnés: no expandía el marcador `{relleno}` |

Sobre Q10 conviene ser explícito, porque la ficha nació acusando al servidor:
el caso trae `{relleno}` como contenido y su enunciado dice que **el lanzador**
lo repite hasta pasarse del límite. Enviándolo literal la petición cabía, el
servidor generaba y contestaba 200 — que es lo correcto—. El mapeo a 422 con
`context_length_exceeded` ya existía y estaba bien. Corregido el arnés, Q10
pasa en **2,4 segundos**.

## Comprobación ejecutada

Contra el endpoint guest con pesos reales (2026-09-24):

```
Q08 rep0  bien          (unknown_tool)
Q10 rep0  bien   2411 ms (422 context_length_exceeded)
Q07 rep0  documento: data: {"error":…,"code":"invalid_request"}\n\ndata: [DONE]
```

El documento de Q07 enseña el arreglo del stream: antes era una respuesta
`HTTP/1.1 400` entera pegada detrás de las cabeceras SSE; ahora el error viaja
como evento y se cierra con `[DONE]`.

`cargo test -p soso-llm-api --features std --test request`: 14 pruebas, con
`codigos_de_error_distinguen_la_causa` nueva, que fija `unknown_tool` y
comprueba que sus vecinos no se mueven.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

**Límite.** Q07 sigue fallando su caso, pero por el modelo, no por esto: no
emite llamada con `tool_choice: "required"` ([T59](T59-tool-calls.md)). Arreglar
esta ficha sube la campaña de 6/10 a 8/10; el 10/10 no llega con este modelo.

Si al mirar los casos resulta que el contrato grabado pide algo que la API no
debería hacer, la corrección es **el caso**, no el código — pero eso se
argumenta y se registra, no se decide para que salga verde.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). Validación nativa: **pendiente**.
