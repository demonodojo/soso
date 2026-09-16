# T52 — Llevar las fusiones BPE al formato `.som` y al convertidor

**Hito:** SI-1 · **Tipo:** Implementación host + formato · **Estado:** pendiente.
**Origen:** derivada de [T03](T03-perfil-modelo.md) (perfil del modelo).
**Dependencias:** ninguna; puede empezarse ya.
**Bloquea:** [T53](T53-tokenizer-bpe.md) y, con ella, [T06](T06-render-chat.md).

## Problema reproducido

El tokenizer de soso no puede reproducir la segmentación oficial de Qwen2.5
porque **el formato `.som` no guarda la tabla de fusiones**. Comprobado sobre
`Qwen/Qwen2.5-Coder-3B-Instruct@488639f1`:

```sh
soso-improve modelo comparar --modelo target/qwen2.5-coder-3b-model
# 0 fixture(s) iguales, 5 con divergencia
soso-improve modelo detalle  --modelo target/qwen2.5-coder-3b-model --caso system --n 6
#  ≠  3    1061 "Res"     65354 "Respond"
#  ≠  4   75049 "ponde"      68 "e"
```

Y en el código:

```sh
grep -rn "merges" crates/gguf2som/src/lib.rs crates/soso-llm-core/src/tokenizer.rs \
    tools/convert-gguf/src/main.rs    # sin resultados
```

`Tokenizer::serialize` escribe piezas, `bos` y `eos`. No hay dónde poner los
rangos de fusión, así que arreglar solo el algoritmo (T53) no bastaría: le
faltaría el dato.

## Contexto mínimo

- [`crates/soso-llm-core/src/tokenizer.rs`](../../crates/soso-llm-core/src/tokenizer.rs):
  `VocabTokenizer::{new,parse,serialize}`, constantes `SPACE_SP`/`SPACE_GPT2`.
- [`crates/gguf2som/src/lib.rs`](../../crates/gguf2som/src/lib.rs): lectura de
  metadatos GGUF (`tokenizer.ggml.*`).
- [`tools/convert-gguf/src/main.rs`](../../tools/convert-gguf/src/main.rs):
  el convertidor que produce `tokenizer.som`.
- [`crates/sosomodel/src/lib.rs`](../../crates/sosomodel/src/lib.rs): contenedor
  `.som`, versiones y compatibilidad hacia atrás.

El GGUF de la familia trae `tokenizer.ggml.merges` (lista de pares). El
`tokenizer.json` original trae lo mismo en `model.merges`.

## Contrato técnico

1. Versionar el `tokenizer.som` para que pueda llevar, además de las piezas,
   una tabla de fusiones ordenada por rango. **Un `tokenizer.som` antiguo debe
   seguir cargando** y comportarse como hoy: los modelos SentencePiece que ya
   funcionan no pueden romperse.
2. Declarar explícitamente el tipo de segmentación del vocabulario
   (`sentencepiece` | `bpe-bytelevel`), en vez de deducirlo contando marcas de
   espacio como se hace ahora.
3. `convert-gguf` y `soso-hf pull` guardan las fusiones cuando el GGUF las trae;
   si no las trae, lo dicen en la salida y el modelo queda marcado como
   segmentación aproximada.
4. Tamaño: el vocabulario de Qwen2.5 tiene ~151 k piezas; medir cuánto ocupan
   las fusiones y decidir su representación (índices a piezas, no cadenas
   repetidas) antes de fijar el formato.

## Comprobación

- `cargo test -p soso-llm-core -p gguf2som -p sosomodel --features std`.
- Round-trip: serializar y parsear un vocabulario con fusiones devuelve lo
  mismo; un `tokenizer.som` sin fusiones sigue cargando.
- Reconvertir el Qwen del perfil y comprobar que `tokenizer.som` trae las
  fusiones y que su número coincide con el del `tokenizer.json` original.
- Las suites que ya usan modelos convertidos (`cargo xtask test`) siguen en
  verde: el formato cambia, el arranque no.

## Cierre y condición de bloqueo

- [ ] Formato versionado y compatible hacia atrás.
- [ ] Convertidor guardando fusiones, con aviso cuando no las haya.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.

Si el GGUF de la familia elegida no publicase las fusiones, esta ficha entrega
el formato y registra la entrada que falta; no se inventan fusiones a partir
del vocabulario.

Entregar `target/self-improvement/tasks/T52/resultado.md` y actualizar la fila
de [README.md](README.md) al cerrar.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). El formato y el parser viven en crates
`no_std + alloc` que ya compilan para el guest; la conversión es host por ahora
(lee GGUF de disco). No introducir dependencias nuevas que impidan compilar
`soso-llm-core` para `x86_64-soso-user`.
