# T53 — Segmentar por fusiones BPE en `soso-llm-core`

**Hito:** SI-1 · **Tipo:** Implementación core · **Estado:** pendiente.
**Origen:** derivada de [T03](T03-perfil-modelo.md) (perfil del modelo).
**Dependencias:** [T52](T52-tokenizer-merges.md) — sin fusiones en el `.som` no
hay con qué segmentar.
**Bloquea:** [T06](T06-render-chat.md), cuya condición de entrada es que las
divergencias de tokenizer detectadas en T03 estén corregidas y verificadas.

## Problema reproducido

`VocabTokenizer::encode` segmenta **por la pieza más larga que encaje**:

```rust
// crates/soso-llm-core/src/tokenizer.rs
let max_len = self.max_piece_len.min(s.len() - i);
for l in (1..=max_len).rev() { … }   // primera coincidencia larga, y a otra cosa
```

Su propia prueba se llama `encode_greedy_y_decode`. Qwen2.5 es BPE byte-level:
parte el texto en símbolos y aplica **fusiones en orden de rango**. Las dos
secuencias son legítimas para el vocabulario, pero solo una es la que el modelo
vio entrenando:

```text
"Responde"   oficial: Res(1061) + ponde(75049)
             soso:    Respond(65354) + e(68)
```

Medido sobre los cinco fixtures de referencia
(`Qwen/Qwen2.5-Coder-3B-Instruct@488639f1`): **5 de 5 divergen**, la primera vez
en los tokens 3, 18, 18, 51 y 51.

```sh
soso-improve modelo comparar --modelo target/qwen2.5-coder-3b-model
soso-improve modelo detalle  --modelo target/qwen2.5-coder-3b-model --caso system
```

Ningún ID de la referencia se sale del vocabulario de soso (máx. 151 664 frente
a 151 936): **no es un problema de vocabulario**, es del algoritmo.

## Contexto mínimo

- [`crates/soso-llm-core/src/tokenizer.rs`](../../crates/soso-llm-core/src/tokenizer.rs):
  `VocabTokenizer::encode`, `gpt2_prepare`, `space_mark`, `leading_space`,
  byte-fallback `<0xXX>`.
- [`crates/soso-improve-core/src/referencia.rs`](../../crates/soso-improve-core/src/referencia.rs):
  el comparador y sus clases de divergencia.
- [`tests/self-improvement/reference/`](../../tests/self-improvement/reference/):
  los cinco fixtures con texto e IDs oficiales.
- Formato con fusiones: el que fije [T52](T52-tokenizer-merges.md).

## Contrato técnico

1. Implementar segmentación BPE por rangos para vocabularios marcados como
   `bpe-bytelevel`: pre-tokenización byte-level de GPT-2, símbolos iniciales,
   fusión repetida del par de menor rango.
2. **Conservar el camino actual** para vocabularios SentencePiece. TinyLlama y
   los modelos ya convertidos deben dar exactamente los mismos IDs que hoy;
   eso es una prueba, no una intención.
3. Los tokens especiales (`<|im_start|>`, `<|im_end|>`, `<|endoftext|>`) se
   reconocen como piezas atómicas y **no se funden con el texto vecino**. Hoy
   coinciden por suerte —el greedy los encuentra enteros—; con BPE hay que
   segmentarlos aparte de forma explícita.
4. Coste: el bucle de fusión no puede ser cuadrático sobre prompts largos. El
   contexto de la familia es 32 768 tokens y esto corre también en el guest,
   sin `std`. Medir con el fixture más largo y dejar el número en el resultado.

## Comprobación

- `cargo test -p soso-llm-core --features std`: round-trip encode/decode,
  byte-fallback, y los casos SentencePiece que ya existen, sin cambios.
- `soso-improve modelo comparar --modelo target/qwen2.5-coder-3b-model` →
  **5 fixtures iguales, 0 divergencias**. Es el criterio de aceptación de esta
  ficha.
- La misma comparación ejecutada **dentro de soso** cuando T49 provea el arnés
  guest; hasta entonces, registrar la ejecución host.
- `cargo xtask test`: la inferencia real de los modelos que ya funcionaban
  sigue dando la misma salida.

## Cierre y condición de bloqueo

- [ ] BPE por rangos para vocabularios byte-level, SentencePiece intacto.
- [ ] 5/5 fixtures iguales en la comparación.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.

Si tras implementar las fusiones quedara alguna divergencia, **no cerrar**:
clasificarla con `modelo detalle` y decidir si es normalización, token especial
o un hueco del formato, registrando la ficha que corresponda.

Entregar `target/self-improvement/tasks/T53/resultado.md` y actualizar la fila
de [README.md](README.md) al cerrar.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). `soso-llm-core` es `no_std + alloc` y se compila
para `x86_64-soso-user`: la segmentación nueva tiene que seguir haciéndolo, sin
`std` ni asignaciones desbocadas. El comparador ya es portable.
