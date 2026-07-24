# L6-H — Inferencia CUDA en host Linux (híbrido)

## Contexto

**CUDA no existe en userspace de soso bare-metal.** El runtime CUDA (driver
propietario, cuBLAS, kernels compilados con `nvcc`) solo corre en Linux/macOS/Windows
con el stack NVIDIA completo.

L6-H es el camino **práctico** para usar la RTX 5070 Ti Mobile (`10de:2f18`) con
CUDA mientras el roadmap nativo L6 (G1–G5, lxdde + nvkm + SASS) avanza en paralelo.

```
┌─────────────┐   TCP :11400    ┌──────────────────┐   HTTP :8080   ┌─────────────┐
│  soso-llm   │ ──────────────► │   cuda-proxy     │ ─────────────► │ llama-server│
│  (QEMU/placa)│ ◄────────────── │   (host Linux)   │ ◄───────────── │  (-ngl CUDA)│
└─────────────┘                 └──────────────────┘                └─────────────┘
```

## Requisitos en el host

| Componente | Notas |
|------------|-------|
| Driver NVIDIA propietario | GB205 / RTX 5070 Ti Mobile |
| llama.cpp compilado con CUDA | `llama-server` o binario equivalente |
| Modelo GGUF | Mismo modelo (o compatible) que en `/models/` de soso |
| Red | IP alcanzable desde soso (QEMU: `10.0.2.2`; placa: IP LAN del host) |

## Arranque rápido (host)

```bash
# 1) Compilar cuda-proxy
cargo build -p cuda-proxy --release

# 2) Lanzar llama-server + proxy (ajusta MODEL y -ngl)
./scripts/l6-h-start-cuda.sh /path/to/model.gguf

# Variables opcionales:
#   LLAMA_PORT=8080  PROXY_PORT=11400  NGL=99
```

El script deja:
- **llama-server** en `127.0.0.1:8080` con capas en GPU (`-ngl`).
- **cuda-proxy** en `0.0.0.0:11400` traduciendo el protocolo soso → HTTP OpenAI.

## Uso desde soso

```bash
# QEMU (host = 10.0.2.2)
soso-llm run tiny --cuda-host 10.0.2.2:11400 --prompt "hola" --max 32

# Placa con IP del host en LAN
soso-llm run llama3-8b --cuda-host 192.168.1.10:11400 --prompt "..." --max 64
```

Flags reconocidos en modo `--cuda-host`:

| Flag | Efecto |
|------|--------|
| `--prompt` | Texto de entrada |
| `--max` | `max_tokens` hacia llama-server |
| `--temp` | Temperatura (default 0.7) |
| `--top-p` | Top-p (default 0.9) |
| `--seed` | Semilla (default 42) |

El primer argumento posicional tras `run` es el **nombre del modelo** enviado a
llama-server (debe coincidir con el modelo cargado en el host).

## Protocolo TCP (soso ↔ cuda-proxy)

Petición (una conexión por inferencia):

```
INFER <model> <max_new> <temp_bps> <top_p_bps> <seed> <prompt_len>\n
<prompt_len bytes UTF-8>
```

- `temp_bps` / `top_p_bps`: temperatura y top-p × 10 000 (0.7 → 7000).

Respuesta OK:

```
OK <text_len> <elapsed_ms>\n
<text_len bytes UTF-8>
```

Respuesta error:

```
ERR <code> <msg_len>\n
<msg_len bytes ASCII>
```

## Verificación

| Paso | Criterio |
|------|----------|
| Host | `nvidia-smi` muestra la GPU |
| Host | `curl http://127.0.0.1:8080/health` (llama-server) |
| Host | `cargo test -p cuda-proxy` verde |
| soso | `soso-llm run … --cuda-host …` imprime texto y tok/s |
| tok/s | Mayor que inferencia CPU pura en el mismo modelo |

## Relación con L6 nativo (G1–G5)

| Camino | CUDA | Cuándo usar |
|--------|------|-------------|
| **L6-H** (este doc) | Sí (host) | Inferencia rápida ya |
| **L6 G3–G5** (lxdde) | No | GPU nativa en soso, meses de nvkm/SASS |

Ambos pueden coexistir: L6-H para producción/demos; G1–G5 para independencia del host.
