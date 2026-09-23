# T56 — El parser de salida parte caracteres UTF-8 y mata el servidor

**Hito:** SI-1 / SI-2 · **Tipo:** Corrección de core · **Estado:** hecha (2026-09-23).

**Dependencias:** ninguna. **La origina:** [T19](T19-qemu-e2e.md), vía
[T55](T55-accept-sin-plazo.md).

## Problema reproducido

Con [T55](T55-accept-sin-plazo.md) la generación ya avanza, pero una petición
de chat **no devuelve nada**: el cliente ve «empty reply from server» tras
203 s. El proceso `serve` ha desaparecido del `ps` del guest —queda su hilo
huérfano— y su sesión ha vuelto al prompt:

```
$ soso-llm serve --model qwen2.5-coder-3b-merges --port 7422 --token-file /tmp/soso-llm-api.token
soso-llm: plan de carga — 36 capas, hidden 2048, pesos disco 2212 MiB
…
panic de usuario: panicked at crates/soso-llm-core/src/conversation/tools.rs:171:33:
start byte index 136 is not a char boundary; it is inside '¡' (bytes 135..137 of string)
sosh: [soso-llm salió con código 101]
```

El modelo contesta en español, la respuesta acaba en `¡`, y el parser de
`<tool_call>` revienta. Un panic en el servidor no es un 500: es el proceso
entero, así que el cliente no recibe ni un byte y la campaña de T19 informaba
«sin respuesta» y luego `429` en las peticiones siguientes.

Reproducción en host, sin QEMU ni pesos:

```sh
cargo test -p soso-llm-core --test conversation_tools texto_acabado_en_caracter_multibyte
```

## Causa

`crates/soso-llm-core/src/conversation/tools.rs`, `split_partial_suffix`.
Busca si la cola del texto es el principio de `<tool_call>` partido entre dos
trozos, y lo hace cortando **por índice de byte**:

```rust
for n in (1..=max).rev() {
    if tag.starts_with(&data[data.len() - n..]) { … }
}
```

Con una cola multibyte —`¡`, `☕`, cualquier acento— ese corte cae dentro del
carácter y `&data[..]` entra en pánico. No lo veía nadie porque los modelos
tiny de la suite generan ASCII; hace falta el modelo real contestando en
español.

## Corrección

Saltar los offsets que no son frontera de carácter. Un corte que parte un
carácter no puede ser el principio de una etiqueta ASCII, así que la guarda no
pierde ninguna coincidencia:

```rust
let split = data.len() - n;
if !data.is_char_boundary(split) {
    continue;
}
```

## Comprobación

- `cargo test -p soso-llm-core --test conversation_tools` — 17 pruebas, incluida
  la nueva `texto_acabado_en_caracter_multibyte_no_revienta`, que **falla sin
  el arreglo** con el panic exacto del guest.
- Campaña guest de [T19](T19-qemu-e2e.md).

## Alcance

`crates/soso-llm-core/src/conversation/tools.rs` y su prueba.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

Límite: se ha corregido **este** corte por bytes. No se ha auditado el resto
del core en busca del mismo patrón; conviene hacerlo en una pasada aparte, con
la salida real del modelo como entrada.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La prueba es de host puro y debería ejecutarse
también desde el runner nativo de [T49](T49-pruebas-guest.md).

Validación nativa: **pendiente**.
