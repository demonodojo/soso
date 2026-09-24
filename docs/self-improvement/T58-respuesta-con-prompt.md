# T58 — La respuesta de chat devolvía el prompt pegado a la generación

**Hito:** SI-2 · **Tipo:** Corrección de userspace · **Estado:** hecha (2026-09-23).

**Dependencias:** ninguna. **La origina:** [T14](T14-evaluacion-modelo.md), al
montar la primera campaña real contra el endpoint guest.

## Problema reproducido

El endpoint contesta 200 con un cuerpo bien formado, pero el `content` del
asistente trae **la plantilla renderizada entera** delante de lo generado:

```sh
curl -s -H "Authorization: Bearer …" -H "Content-Type: application/json" \
  -d '{"model":"qwen2.5-coder-3b-merges","messages":[{"role":"user","content":"Di la palabra LISTO y nada más."}],"max_tokens":32,"temperature":0,"stream":false}' \
  http://127.0.0.1:17299/v1/chat/completions
```

```json
{"choices":[{"message":{"role":"assistant",
 "content":"<|im_start|>system\nYou are Qwen, created by Alibaba Cloud. You are a helpful assistant.\n<|im_start|>user\nDi la palabra LISTO y nada más.\n<|im_start|>assistant\nLISTO"},
 "finish_reason":"stop"}],
 "usage":{"prompt_tokens":38,"completion_tokens":2,"total_tokens":40}}
```

Lo que lo delata sin discusión es que **el propio cuerpo se contradice**:
`usage.completion_tokens` dice **2**, y el `content` trae cuarenta y tantos
tokens de plantilla. Uno de los dos miente, y es el texto.

Un cliente que lea `content` recibe su propio prompt de vuelta, con los
marcadores de plantilla incluidos. Para una API compatible con OpenAI eso no es
una imperfección: es una respuesta equivocada.

## Causa

`Runtime::generate_stream_planned_observed` devuelve la **secuencia entera**:

```rust
let mut tokens: Vec<u32> = prompt.to_vec();   // crates/soso-llm-core/src/runtime.rs
```

Es una decisión razonable del generador —quien llama sabe cuánto medía su
prompt—, pero obliga al llamante a recortar. En todo el árbol sólo lo hacía
`asr.rs`:

```rust
Ok(self.tokenizer.decode(&tokens[prompt.len()..]))
```

`user/soso-llm/src/serve.rs` no recortaba, ni en la respuesta JSON
(`post_chat_inner`) ni en la de streaming (`post_chat_stream`): decodificaba
`token_ids` completo.

No se había visto antes porque las pruebas que existían miraban el **código de
estado**, no el contenido: `chat_json` de [T19](T19-qemu-e2e.md) pasa con esta
respuesta, y los modelos tiny de la suite generan tan poco que nadie leyó el
texto.

## Corrección

Una función explícita en `serve.rs`, aplicada en los dos caminos:

```rust
/// Se hace por longitud y no buscando un separador: los ids del prompt son
/// exactamente los que se le pasaron, y un separador podría aparecer también
/// en lo generado.
fn solo_generados(secuencia: &[u32], prompt_len: usize) -> Vec<u32>
```

## Comprobación

La misma petición, con el arreglo:

```json
{"choices":[{"message":{"role":"assistant","content":"LISTO"},"finish_reason":"stop"}],
 "usage":{"prompt_tokens":38,"completion_tokens":2,"total_tokens":40}}
```

Texto y `usage` ya dicen lo mismo. La campaña de [T14](T14-evaluacion-modelo.md)
es la comprobación de sistema: sus aserciones `contenido_contiene` juzgan el
contenido, no sólo el estado.

## Alcance

`user/soso-llm/src/serve.rs`.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

**Límite.** Se ha corregido el consumidor, no el contrato: `generate_*` sigue
devolviendo prompt + generación, y cualquier llamante nuevo puede repetir el
error. Dejar el recorte en el generador —o devolver las dos partes separadas—
es un cambio de API que afecta a `asr`, `ask` y `serve`, y merece su propia
ficha en vez de colarse aquí.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La comprobación vive en la campaña de T14 y
debería repetirse desde el runner nativo de [T49](T49-pruebas-guest.md).

Validación nativa: **pendiente**.
