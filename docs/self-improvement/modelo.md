# Perfil del modelo candidato

**Ficha:** [T03](T03-perfil-modelo.md). **Fecha:** 16 de septiembre de 2026.
**Estado:** perfil publicado; **la calidad sigue sin medir** — eso es
[T14](T14-evaluacion-modelo.md), que decidirá si sirve para el agente.

## Qué modelo

| Campo | Valor |
|---|---|
| Repositorio | `Qwen/Qwen2.5-Coder-3B-Instruct` |
| Revisión | `488639f1ff808d1d3d0ba301aef8c11461451ec5` (2025-01-12) |
| Familia | `qwen2` · `Qwen2ForCausalLM` |
| Capas / vocab / contexto | 36 · 151 936 · 32 768 |
| Convertido en | `target/qwen2.5-coder-3b-model` (`manifest.som`, `index.som`, `tokenizer.som`, shards) |
| Catálogo del proyecto | `qwen2.5-coder-3b` en [`xtask/src/live_models.rs`](../../xtask/src/live_models.rs) |

Es el candidato que fija C1 del [contrato](CONTRATO.md). Los hashes de cada
pieza —los `.som` y los archivos originales— están en el `model-lock.json` que
escribe `soso-improve modelo perfil`; su formato está en
[`model-profile.schema.json`](../../tests/self-improvement/model-profile.schema.json).

Los pesos **no viajan en Git**: 2,2 GB en `target/`. El tokenizer y la
plantilla originales se descargan de la revisión fijada a
`target/self-improvement/modelo/`.

## Formato exacto de la familia

Plantilla oficial (ChatML). Cada turno:

```text
<|im_start|>{rol}\n{contenido}<|im_end|>\n
```

y la generación arranca con `<|im_start|>assistant\n`.

- **Sin mensaje de sistema**, la plantilla **inserta uno**: `You are Qwen,
  created by Alibaba Cloud. You are a helpful assistant.` No es opcional: el
  modelo siempre ve un turno de sistema.
- **Con herramientas**, la declaración va dentro del turno de sistema, entre
  `<tools>` y `</tools>`, un JSON por línea.
- **Llamada**: el asistente escribe, dentro de su turno,

  ```text
  <tool_call>
  {"name": <function-name>, "arguments": <args-json-object>}
  </tool_call>
  ```

- **Resultado**: vuelve como turno de **usuario** (no de `tool`), envuelto en
  `<tool_response>` … `</tool_response>`.

Esto es texto, no una API: **que la plantilla sepa escribir `<tool_call>` no
demuestra que el modelo las use bien**. C2 ya lo dice y T14 lo medirá.

## Paradas

| Origen | Valor |
|---|---|
| `eos_token` del tokenizer | `<|im_end|>` = 151645 |
| `generation_config.eos_token_id` | `[151645, 151643]` |
| `pad_token` | `<|endoftext|>` = 151643 |

Hay **dos** paradas, no una. Un bucle que solo corte en 151645 se comerá el
final cuando el modelo emita 151643.

## Tratamiento de errores

- Un `<tool_call>` con JSON incompleto **no es una llamada**: se rechaza como
  error de la petición, no se ejecuta a medias (C3, casos Q07 y Q09 del banco).
- Un nombre de función que no está en `tools` es petición inválida (Q08).
- Pasarse del contexto es 422 con `context_length_exceeded`, sin truncar el
  historial en silencio (Q10).

## Fixtures de referencia

Cinco conversaciones en
[`tests/self-improvement/reference/`](../../tests/self-improvement/reference/):
`simple`, `system`, `historial`, `herramienta-esquema` y `herramienta-resultado`.
Cada una guarda el **texto que produce la plantilla oficial** y los **token IDs
que produce el tokenizer oficial**, con sus hashes y la revisión de la que
salen.

Los genera `tools/tokenizer-ref`, una herramienta **offline y fuera del
workspace** que usa la implementación de Hugging Face (`tokenizers`) y un motor
Jinja. Está separada a propósito: la referencia tiene que ser independiente de
lo que se va a probar, y esas dependencias no deben entrar en un sistema que
quiere compilarse a sí mismo.

```sh
cargo run --manifest-path tools/tokenizer-ref/Cargo.toml -- \
    --modelo target/self-improvement/modelo \
    --salida tests/self-improvement/reference \
    --revision 488639f1ff808d1d3d0ba301aef8c11461451ec5
```

Quien compara es código del proyecto y corre también dentro de soso:

```sh
soso-improve modelo comparar --modelo target/qwen2.5-coder-3b-model
soso-improve modelo detalle  --modelo target/qwen2.5-coder-3b-model --caso system
```

## Divergencias encontradas

**Los cinco fixtures divergen.** Ningún ID de la referencia se sale del
vocabulario de soso (máximo 151 664 frente a 151 936), así que no es un
problema de vocabulario: es de **segmentación**.

| Fixture | Primer token distinto | Referencia | soso |
|---|---|---|---|
| `simple` | 18 de 38 | `.` (13) | `.<` (15757) |
| `system` | 3 de 37 | `Res` (1061) | `Respond` (65354) |
| `historial` | 18 de 74 | `.` (13) | `.<` (15757) |
| `herramienta-esquema` | 51 de 197 | `<` (27) | `<t` (62752) |
| `herramienta-resultado` | 51 de 252 | `<` (27) | `<t` (62752) |

Causa raíz, mirando el código y no solo el síntoma: `VocabTokenizer::encode`
de [`soso-llm-core`](../../crates/soso-llm-core/src/tokenizer.rs) segmenta
**por la pieza más larga que encaje** —su propia prueba se llama
`encode_greedy_y_decode`—, mientras que Qwen2.5 es BPE byte-level, que aplica
**fusiones en orden de rango**. Con «Responde», el oficial saca `Res`+`ponde` y
soso saca `Respond`+`e`: las dos secuencias son legítimas para el vocabulario,
pero solo una es la que el modelo vio entrenando.

Y hay un segundo hueco debajo: **el formato `.som` no guarda la tabla de
fusiones**. `gguf2som`, `convert-gguf` y `tokenizer.som` solo llevan piezas, así
que hoy no se puede reproducir la segmentación oficial ni arreglando el
algoritmo.

Eso son dos fichas, previas a [T06](T06-render-chat.md):

- **[T52](T52-tokenizer-merges.md)** — llevar las fusiones al formato `.som` y
  al convertidor.
- **[T53](T53-tokenizer-bpe.md)** — segmentar por fusiones en `soso-llm-core`,
  sin romper los modelos SentencePiece que ya funcionan.

## Un tercer hallazgo, que ya tenía ficha

La plantilla que guarda el `.som` convertido es

```text
<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n
```

Un solo turno de usuario: **no puede expresar system, historial ni
herramientas**. No es un defecto del convertidor —`{prompt}`/`{eos}` es todo lo
que sabe decir `chat::render` hoy—, sino la razón de ser de
[T04](T04-dominio-chat.md) y [T06](T06-render-chat.md). Queda anotado aquí
porque el perfil de una familia con herramientas no se sostiene sobre esa
plantilla.
