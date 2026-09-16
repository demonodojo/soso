# T10 — Crear la crate API y adaptar peticiones JSON

**Hito:** SI-2 · **Tipo:** Implementación Rust host/no_std · **Estado:** pendiente.

**Dependencias:** [T05](T05-validacion-chat.md), [T06](T06-render-chat.md), [T08](T08-resultado-generacion.md)

## Objetivo y entrega

Entrada JSON validada y lista para inferencia o error preciso; ningún socket ni dependencia std en el camino guest.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C1–C3**, y los símbolos
pertinentes de estos archivos. Reutilizar los módulos existentes; crear solo los símbolos que falten
tras comprobar las entregas de las dependencias.

- [Cargo.toml](../../Cargo.toml)
- [crates/soso-llm-core/Cargo.toml](../../crates/soso-llm-core/Cargo.toml)

## Archivos que se pueden cambiar

Crear `crates/soso-llm-api/{Cargo.toml,src/lib.rs,src/request.rs,tests/request.rs}`; añadir miembro al workspace raíz.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Crear la crate no_std+alloc dependiente de core y serde_json. Añadir feature std para el arnés host posterior.
2. Parsear wire Chat Completions en tipos propios de transporte y convertir a ChatInput; admitir content string/null y partes de texto. Rechazar imagen/audio con error específico.
3. Aplicar campos, valores y defaults C3; diferenciar ausente, null e inválido. Verificar model id, max_tokens y alias, selección de herramienta y parámetros numéricos.
4. Validar historial con T05 y presupuestar tokens con T06 antes de comenzar inferencia. No tratar el tamaño en bytes como número de tokens.
5. Mapear error de dominio a código HTTP de C3, con mensaje corto que no vuelque todo el prompt.

## Comprobación

`cargo test -p soso-llm-api --features std --test request`; `cargo check -p soso-llm-api --no-default-features`. Tests límite exacto/límite+1, alias contradictorios, tool content null, partes text, modelo ausente, NaN rechazado y modalidad no soportada.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

Un campo desconocido visto en tráfico real se resuelve con fixture de T21, no ignorando todos los parámetros.

Entregar `target/self-improvement/tasks/T10/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
