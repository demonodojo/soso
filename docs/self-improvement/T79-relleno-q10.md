# T79 — El relleno de Q10 no rebasa el contexto del 7B

**Hito:** SI-0 / SI-1 · **Estado:** hecha (2026-09-27).
**Dependencias:** [T14](T14-evaluacion-modelo.md), [T74](T74-campana-7b-interrumpida.md).
**Contrato:** C2, C5.
**Seguimiento:** [T79](seguimiento/T79.md).

## Problema

En la campaña completa del 7B, Q10 no midió el rechazo de contexto. El
manifiesto declara `max_seq` 131072 y el lanzador rellenaba para 32768
tokens a 8 bytes por token (~278 KiB). Ese texto cabe en 128k: el servidor
empezó a generar, el cliente recibió `EAGAIN` a los 0 ms y los dos reintentos
vieron 429 `busy`. El banco espera 422 `context_length_exceeded`.

Q06 no entra aquí. Su comprobación es que la frase sobreviva al corte, no
que la respuesta sea sólo esa frase.

## Alcance

- `tools/soso-improve/src/evaluar.rs`: relleno medido con el tokenizer del
  modelo, sin pasar de 1 MiB.
- `xtask/src/test_llm_api.rs`: la campaña pasa `--contexto` y `--modelo-dir`.
- `crates/soso-llm-api`: el cliente espera el plazo ante `EAGAIN`; el servicio
  de host rechaza el contexto antes de ocupar la generación.
- `user/soso-llm/src/serve.rs`: lo mismo en el guest.

No se toca el banco, el perfil C1 ni el selector live.

## Comprobación

```sh
cargo test -p soso-improve --lib evaluar::tests
cargo test -p soso-llm-api --features std --lib e2e::tests::una_respuesta_tardia_no_es_eagain
cargo test -p soso-llm-api --features std --test host_service
cargo test -p xtask --offline pruebas_campana
cargo check -p soso-llm --manifest-path user/Cargo.toml --offline
```

Con el tokenizer del 7B, el relleno sale a 131125 tokens y 574462 bytes de
cuerpo.

- [x] El relleno supera el `max_seq` del perfil y cabe en el cuerpo HTTP.
- [x] Un `EAGAIN` inmediato no cierra el plazo.
- [x] El exceso de contexto no ocupa el hueco de generación.
- [x] Q10 medido contra el guest el 27-sep: tres veces bien, dentro del GO 10/10.

**Validación nativa:** parcial. Q10 y la campaña 10×3 corrieron en el guest;
las lanza el arnés del host. Resumen: [seguimiento/T79.md](seguimiento/T79.md).
