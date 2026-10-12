# Plan: Gemma 4 (12B, QAT Q4_0) en soso

**Decisión (2026-10-12, usuario):** preparar soso para la arquitectura de Gemma 4 con **núcleos Q4_0 nativos** y, si el modelo se confirma, ponerlo como modelo por defecto. Modelo objetivo: `google/gemma-4-12B-it-qat-q4_0-gguf` (`gemma-4-12b-it-qat-q4_0.gguf`, 6 975 879 296 B, Apache-2.0).

**Por qué:** en la campaña de automejora sobre el banco R11–R20 (agente congelado) Qwen2.5-Coder-14B saca 2/10 y Gemma 4 12B (servido con llama.cpp en GPU) va muy por delante (ver `docs/self-improvement/campana.md`, campaña 5).

## Lo que trae el modelo (leído del GGUF)
- `general.architecture = gemma4`; 48 capas, `n_embd` 3840, FFN 15360 (gate/up/down), 16 cabezas Q.
- **Atención mixta**: patrón `sliding_window_pattern` = 5 locales + 1 global (capas 5, 11, …):
  - local: ventana 1024, `head_dim` 256, 8 cabezas KV, RoPE base 10⁴ (`freq_base_swa`);
  - global: `head_dim` 512, **1 cabeza KV**, RoPE base 10⁶ con `rope_freqs.weight` [256] (factores por frecuencia), **sin tensor `attn_v`** (V = K).
- Por capa: `attn_norm`, `attn_q_norm`/`attn_k_norm` (por cabeza), `post_attention_norm`, `ffn_norm`, `post_ffw_norm`, `layer_output_scale` (escalar).
- `final_logit_softcapping = 30`; embeddings atados (`token_embd` Q6_K, sin `output.weight`).
- Tokenizer `gemma4`: BPE de 262 144 tokens, 514 906 merges. Plantilla de chat propia de 18 KB con mini-DSL de herramientas (`<|"|>`).
- Pesos: Q4_0 en casi todo (328 tensores), Q6_K en embeddings, F32 en normas.
- Multimodal (visión): **fuera de alcance**.

## Etapas (cada una se cierra con pruebas; la referencia numérica es llama.cpp en GPU)
| Etapa | Qué | Estado |
|---|---|---|
| **G1** | Formato `DTYPE_Q4_0` (GGML exacto, 18 B/bloque), descuantización, núcleos escalar y AVX2 (matvec y por lotes con zancada), despacho en capas | **hecha** (2026-10-12): `sosomodel/tests/q4_0.rs` (4), `soso-llm-core/tests/q4_0.rs` (4, incluye el AVX2 llamado directo) |
| **G2** | `gguf2som`: Q4_0 tal cual, arrays enteros de metadatos, arquitectura `gemma4`; manifest **v7** (extensiones `ManifestExt`: escala de embeddings, softcap, escala de atención, embeddings atados, ventana y base RoPE por capa; flags de capa nuevos: sándwich, V=K, V-norm, GeGLU, escala de capa, factores RoPE); validación de formas heterogéneas | **hecha** (2026-10-12): modelo real convertido en 41 s y `convert-gguf --check` OK; pruebas sintéticas en `gguf2som` |
| G3 | Forward de la capa Gemma: normas en sándwich, QK-norm por cabeza, atención local (ventana) y global (1 KV, K=V), RoPE con factores, GeGLU, `layer_output_scale`, escala de embeddings, softcap | hecha (2026-10-12): paridad con llama.cpp en 4 prompts por ids (mismo top-1, mismo orden top-5, prob. a ±0,03 en las no saturadas; `logits_ids`) |
| G4 | KV cache por capa con dimensiones distintas (256×8 local, 512×1 global) y ventana deslizante | hecha (2026-10-12): `LayerKv::expulsar_delante` + `dropped` (posiciones absolutas); las capas locales guardan ventana + 1024..2048 filas; la reutilización de prefijo respeta el suelo `dropped + ventana`; prompt de 3 700 tokens con expulsión activa: mismo top-1/top-2 que llama.cpp (`tests/kv_ventana.rs`). Prefill largo ~13 tok/s |
| G5 | Tokenizer `gemma4` (BPE) idéntico a llama.cpp (`/tokenize`) | hecha (2026-10-12): `BpeSpm` + `tokenizer.som` v3 (especiales por tipo); 23 textos idénticos a `/tokenize` y ida y vuelta (`tests/tokenizer_gemma4.rs`) |
| G6 | Plantilla de chat y parser de llamadas a herramientas de Gemma 4 en `conversation` | hecha (2026-10-12): `conversation/gemma4.rs`; render idéntico a `/apply-template` en 10 escenarios × (con, sin pensamiento) y parser de `<|tool_call>` (`tests/gemma4_plantilla.rs`) |
| G7 | Paridad extremo a extremo: mismos tokens (greedy) y logits que llama.cpp en prompts fijos; bench en host | hecha (2026-10-12): servidor de soso con Gemma de extremo a extremo; el prompt de 24 tokens coincide con llama.cpp y la respuesta greedy coincide 11 tokens hasta un empate real (0,4186 vs 0,4182). Prefill por bloques (`forward_layer_gemma_block`): ~20 tok/s (antes ~1,4); decode ~7 tok/s con 16 hilos en caliente. Carga en frío ~20 s (disco) |
| G8 | `soso-llm serve`/perfil/catálogo, memoria del guest, imagen live con Gemma como **modelo por defecto**; campaña de automejora sobre soso | en curso (2026-10-12): hecho el catálogo (`gemma-4-12b`, escalera de 16 GB), la plantilla de `ask` (`traducir_plantilla`/`anteponer_sistema`) y el perfil del guest (`perfil_api`: familia `gemma4`, paradas `<turn|>` y `<|tool_response>`). **Falta**: prueba en el guest (QEMU: 12 GB y 4 CPUs arrancan; 14 GB y 8 CPUs no llegaron a `sosh` en 180 s), campaña sobre soso y decidir `default_live_spec` |

## Riesgos
- Detalles del modelo no documentados fuera del GGUF (p. ej. la fórmula exacta de `rope_freqs`, si las normas llevan `1+w`, el escalado de embeddings): se fijan **comparando contra llama.cpp**, no suponiendo.
- Q4_0 con activaciones f32 es más lento que un camino entero (Q8 de activaciones, estilo `q8k`): primero corrección, después velocidad.
- 12B en el guest pide ≥ 8–9 GB de RAM solo para pesos; el 14B Q4_K ya iba justo.
