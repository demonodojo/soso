# L6-H — Inferencia CUDA en host Linux (híbrido)

## Contexto

**CUDA no existe en userspace de soso bare-metal.** El runtime CUDA (driver
propietario, cuBLAS, kernels compilados con `nvcc`) solo corre en Linux/macOS/Windows
con el stack NVIDIA completo.

L6-H es el camino **práctico** para usar la RTX 5070 Ti Mobile (`10de:2f18`) con
CUDA mientras el roadmap nativo L6 (G1–G5, lxdde + nvkm + SASS) avanza en paralelo.

**Estado (2026-07-27): GO** — verificado en QEMU: `soso-llm --cuda-host 10.0.2.2:11400`
imprime texto y ~35 tok/s (`tinyllama-q4km`, proxy + llama-server en el host).

```
┌─────────────┐   TCP :11400    ┌──────────────────┐   HTTP :8080   ┌─────────────┐
│  soso-llm   │ ──────────────► │   cuda-proxy     │ ─────────────► │ llama-server│
│  (QEMU/placa)│ ◄────────────── │   (host Linux)   │ ◄───────────── │  (-ngl CUDA)│
└─────────────┘                 └──────────────────┘                └─────────────┘
```

## Requisitos en el host

| Componente | Notas |
|------------|-------|
| Driver NVIDIA propietario | GB205 / RTX 5070 Ti Mobile; `nvidia-smi` OK |
| llama-server con CUDA | Binario nativo **o** imagen Docker (ver abajo) |
| Modelo GGUF | p.ej. `target/tinyllama-q4km.gguf` |
| Red | IP alcanzable desde soso (QEMU: `10.0.2.2`; placa: IP LAN del host) |
| cuda-proxy | `cargo build -p cuda-proxy --release --target-dir target` |

## Arranque rápido — Docker (recomendado)

No requiere compilar llama.cpp en el host. Con la GPU en el driver `nvidia` (sin VFIO):

```bash
# 1) llama-server CUDA en Docker (puerto 8080)
docker run -d --name soso-llama \
  --device=/dev/nvidia0 --device=/dev/nvidiactl \
  --device=/dev/nvidia-uvm --device=/dev/nvidia-modeset \
  -p 8080:8080 -v "$PWD/target:/models" \
  ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/tinyllama-q4km.gguf -ngl 99 --host 0.0.0.0 --port 8080

curl -sf http://127.0.0.1:8080/health

# 2) cuda-proxy (terminal aparte; bloquea mientras escucha)
cargo build -p cuda-proxy --release --target-dir target
target/release/cuda-proxy --listen 0.0.0.0:11400 --llama http://127.0.0.1:8080
```

**CUDA real en el contenedor:** instala `nvidia-container-toolkit` y usa
`docker run --gpus all …` en lugar de `--device=/dev/nvidia*`. Sin el toolkit,
el contenedor puede inferir en CPU pero el **protocolo L6-H sigue funcionando**.

Parar:

```bash
docker rm -f soso-llama
pkill -f 'cuda-proxy.*11400'
```

## Arranque alternativo — llama-server nativo

Si tienes `llama-server` compilado con CUDA en PATH:

```bash
cargo build -p cuda-proxy --release --target-dir target
./scripts/l6-h-start-cuda.sh /path/to/model.gguf
# LLAMA_PORT=8080  PROXY_PORT=11400  NGL=99
```

## Uso desde soso

```bash
# Terminal 1: QEMU
SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask run

# En el prompt $ de soso (host slirp = 10.0.2.2):
soso-llm run tinyllama-q4km --cuda-host 10.0.2.2:11400 --prompt "hola" --max 32
```

Salida esperada (GO):

```
soso-llm: CUDA host 10.0.2.2:11400 (modelo tinyllama-q4km)
<texto generado>
soso-llm: CUDA host (~N tokens, X ms host, Y.YY tok/s)
```

En placa con IP LAN del host:

```bash
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
| Host | `cargo test -p cuda-proxy` verde (HTTP mockeado en tests) |
| Host | `printf 'INFER …' \| nc 127.0.0.1 11400` → línea `OK …` |
| soso | `soso-llm run … --cuda-host …` imprime texto y tok/s |
| tok/s | Mayor que inferencia CPU pura en el mismo modelo (opcional: `soso-llm run tiny …`) |

## Relación con L6 nativo (G1–G5)

| Camino | CUDA | Cuándo usar |
|--------|------|-------------|
| **L6-H** (este doc) | Sí (host) | Inferencia rápida ya, GPU en driver NVIDIA |
| **L6 G4e–G5** (lxdde) | No | GPU nativa en soso (G1–G4c GO en HW; G4e hostcheck OK) |

**No simultáneo en la misma máquina:** el bind VFIO persistente
(`l6-g1-vfio-persist.sh --enable`) reserva la dGPU para soso y bloquea
`nvidia-smi` en el host. Para L6-H: `sudo ./scripts/l6-g1-vfio-persist.sh --disable`
+ reboot, luego arrancar llama-server + cuda-proxy.

## Desarrollo diario (sin soltar la GPU)

Compatible con el driver NVIDIA del host — **no** hace falta VFIO:

| Paso | Comando |
|------|---------|
| Hostcheck GSP/G4 | `./scripts/l6-g3-gsp-hostcheck.sh` |
| Build | `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask build` |
| QEMU | `SOSO_LXDDE=1 SOSO_LXDDE_MODE=nouveau cargo xtask run` |
| L6-H | Docker + cuda-proxy (arriba) + `--cuda-host 10.0.2.2:11400` |

En QEMU sin passthrough no hay GPU NVIDIA en PCI: verás `nvidia: sin GPU NVIDIA en PCI`
y **no** `GSP booted (soft)` — es normal. El bring-up GSP real requiere
`SOSO_QEMU_GPU=vfio:…`.
