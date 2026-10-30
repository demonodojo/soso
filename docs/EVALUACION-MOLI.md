# Evaluación del port nativo de moli a soso

Ficha de inventario y sondas para decidir si tiene sentido portar
[lexmount/moli](https://github.com/lexmount/moli) como motor **nativo** dentro
de soso (sin puente remoto). **No se ha construido ningún port.**

Fecha: 2026-10-30. Commit moli medido: `045041566f60f9d933a052dee22034f36550ab4f`
(shallow clone en `~/.cache/moli`).

## Resumen ejecutivo

| Veredicto | **No viable** portar el binario moli completo a soso en el horizonte
| razonable, con el stack actual del OS. |
| Alternativa | Inspirarse en crates **sin V8** (`moli-html2md`, `moli-css-parse`,
| `moli-layout`) vía `soso-web-core` / agente Rust (misma línea que D10 con
| OpenCode). |
| Alcance | moli es **headless**; no sustituye `soso-web --grafico` ni el WM. |

---

## 1. Medición en host (Linux)

### Binario preinstalado (release v1.1.19)

Descargado en `~/.cache/moli-bin/moli-v1.1.19-x86_64-unknown-linux-gnu/moli`.

| Métrica | Valor |
|--------|--------|
| Tamaño ELF | **108 MiB** (stripped) |
| Enlace | **dinámico** (`/lib64/ld-linux-x86-64.so.2`) |
| Dependencias dinámicas | `libc`, `libstdc++`, **libfontconfig**, freetype, zlib, brotli, … |
| `moli fetch --dump markdown` (example.com) | ~**0,21 s** wall; **~48 MiB** RSS pico |
| `moli fetch --layout --dump screenshot` | ~**0,72 s** wall; PNG ~93 KiB |

### Compilación desde fuente (mismo commit)

```text
cd ~/.cache/moli && cargo build -p moli --release
compile_elapsed=183.17
```

**Falló** en `yeslogic-fontconfig-sys`: falta `fontconfig.pc` en el host de
medición. En un host con Fontconfig instalado, el build seguiría descargando
**V8 precompilado** (`v8` crate 152.2.0 / `deno_v8` 0.3.0) para
`x86_64-unknown-linux-gnu`, no genera un target `soso`.

### Motor JavaScript (V8)

- Crate: `v8` **152.2.0** (`vendor/v8-152.2.0`), expuesto vía `deno_v8`.
- Features usadas en `moli-renderer-v8`: `v8_enable_pointer_compression`
  (jaula ~4 GiB de VA).
- GN por defecto en `build.rs`: `v8_enable_sandbox=false` salvo feature
  `v8_enable_sandbox`; pointer compression configurable.
- `V8_FROM_SOURCE=1` exige layout GN/ninja completo (Chromium toolchain).
- Allocator: `jemalloc` por defecto en el binario `moli` (salvo targets con
  PartitionAlloc publicado por rusty_v8).

### Red (libcurl)

- `moli-curl` → `curl` 0.4.51 con **`poll_7_68_0`**, `http2`, `ssl`
  (`static-curl` + **OpenSSL/aws-lc** vía `openssl-sys`), `websocket`.
- TLS: OpenSSL embebido en curl estático, no el stack `rustls` de soso.

### Runtime async

- **tokio** 1.51 (`rt-multi-thread`, `net`, `signal`, `sync`, `time`) en
  `moli`, `moli-core`, servidor CDP/WebDriver.
- **No** hay dependencia directa de `rayon` en el workspace raíz; paralelismo
  vía tokio + hilos V8.

### Layout / CSS

- **Stylo** (fork Lexmount), **Taffy**, **Parley**, **Vello CPU** (AnyRender).
- Fuentes Linux: `moli-system-fonts` → **fontconfig** (`yeslogic-fontconfig-sys`).

### strace -c -f (binario v1.1.19)

**`moli fetch --dump markdown`** (top):

| syscall | ~% tiempo | llamadas |
|---------|-----------|----------|
| futex | 79 | 179 |
| epoll_wait | 13 | 29 |
| mmap / mprotect | ~1,5 | 130 / 224 |
| clone3 | 0,4 | 20 |
| pselect6 / poll | ~0,5 | 13 / 4 |

**`moli fetch --layout --dump screenshot`** (top):

| syscall | ~% tiempo | llamadas |
|---------|-----------|----------|
| futex | 84 | 197 |
| epoll_wait | 11 | 29 |
| mmap / openat / readlink | alto | miles (carga fuentes/recursos) |

Conclusión: el perfil de syscalls es **Linux + epoll + pthreads + Fontconfig**,
no encaja con el ABI mínimo de soso.

---

## 2. Inventario → ABI soso

Leyenda: **S** soportado, **P** parcial, **A** ausente, **C** por comprobar en
guest (sonda `soso-agent-probe moli`).

| Componente | Para qué | Requisito SO / libc | soso |
|------------|----------|---------------------|------|
| **V8 / rusty_v8** | JS, DOM vivo | Reserva 4G VA, JIT W^X, threads, C++ runtime, prebuilt **linux-gnu** | **A** target; **P** JIT vía `PROT_EXEC` + alternancia RW/RX (`kernel/src/task/syscall.rs`) |
| **tokio** | CDP, fetch, servidor | **epoll**, **socketpair**, **rt_sigaction**, timers | **A** epoll; señales mínimas sin `sigaction` |
| **libcurl multi** | HTTP/S, WS | **poll** / epoll / pselect6 | **A** poll/select/epoll; **P** TCP + `read_timeout` |
| **OpenSSL (curl)** | TLS sitios reales | libc socket + RNG | **P** `soso-http`+rustls distinto stack |
| **SQLite bundled** | cookies, storage | fd compartido, **flock**, fsync, mmap; pread ideal | **P** `SYS_FLOCK`, `SYS_FSYNC`, `mmap`; sin **pread** (seek+read por fd) |
| **Stylo / stylo_dom** | CSS cascade | `std`, alloc, hash | **A** sin `soso-std` completo en rootfs productivo |
| **Fontconfig** | `--layout`, PDF | libfontconfig, freetype, rutas sistema | **A**; soso tiene PSF2/DejaVu vía `soso-web` |
| **moli (binario)** | CLI/serve | **ld-linux**, libstdc++, libfontconfig | **A** no hay dynamic linker |
| **Procesos / hilos** | pool V8, tokio | clone/futex, TLS | **S** `thread_spawn`, `SYS_FUTEX`, `SYS_SET_TLS` |
| **DNS** | fetch | getaddrinfo o resolver propio | **S** `SYS_DNS_RESOLVE`; moli usa resolver propio + curl |
| **Memoria** | jemalloc / V8 cage | mmap grande, mprotect PROT_NONE | **C** reserva 4G; **A** 1T (> ventana mmap ~416 GiB) |
| **Señales** | tokio graceful | sigaction, signalfd | **A** (diseño soso: kill mínimo) |

Referencias ABI: [`crates/soso-abi/src/lib.rs`](../crates/soso-abi/src/lib.rs),
W^X en [`sys_mprotect`](../kernel/src/task/syscall.rs) (`W|X` → `EINVAL`;
`PROT_EXEC` separado).

Precedente: inventario OpenCode/Bun
[`docs/self-improvement/native/opencode-deps.md`](self-improvement/native/opencode-deps.md);
decisión **D10** — agente nativo Rust, no portar runtime JS
([`T35-opencode-nativo.md`](self-improvement/T35-opencode-nativo.md)).

---

## 3. Sondas en guest (`soso-agent-probe moli`)

Implementación: [`user/soso-agent-probe/src/sondas/moli.rs`](../user/soso-agent-probe/src/sondas/moli.rs).

| Caso | Qué mide | Resultado esperado (kernel actual) |
|------|----------|-------------------------------------|
| `moli/reserva-4g-mmap` | `mmap` 4 GiB anónimo | **ok** (ventana 64–480 GiB) |
| `moli/reserva-4g-prot-none` | `mprotect(..., 0)` en subrango | **ok** (N-002, `sin_acceso`) |
| `moli/reserva-4g-subrango-rw` | RW tras reserva | **ok** |
| `moli/reserva-1t` | `mmap` 1 TiB (sandbox V8) | **fallo ENOMEM** → compilar V8 con `v8_enable_sandbox=false` |
| `moli/jit-alterna-rw-rx` | escribir → RX → ejecutar → RW → RX | **42 y 51** si W^X estricto funciona |
| `moli/32-hilos-futex` | 32× `thread::spawn` + join | **32** (mismo orden que hilos moli ~24) |
| `moli/espera-varios-sockets` | dos TCP + `read_timeout` | **medida**: falta poll/epoll para curl multi |

**Ejecución en esta sesión:** `cargo xtask mkfs` + build del probe OK; SSH guest
(`cargo xtask test --only "probe: hilos"`) falló con *Connection timed out during
banner exchange* en el entorno QEMU (mismo síntoma en otros probes). Re-ejecutar:

```sh
cargo xtask build && cargo xtask mkfs
cargo xtask test -- --guest sys --only "probe: …"   # cuando SSH responda
# o en consola/SSH: soso-agent-probe moli
```

---

## 4. Caminos analizados y coste

### A. V8 prebuilt “linux-gnu” + shims PAL (como `rustc` guest)

- **Problema:** el release moli es **dinámico** (ld-linux, fontconfig). Los
  archives rusty_v8 son por triple **linux-gnu**, no `unknown-soso`.
- **Coste:** fork de `v8/build.rs`, CI propia de snapshots, parches libc/PAL
  para C++ V8 + jemalloc + OpenSSL curl — **varios meses**, alto riesgo.
- **Rompe:** modelo W^X de soso si se exige W+X legacy; hay que validar solo
  alternancia RW/RX (sonda `jit-alterna-rw-rx`).

### B. V8 `V8_FROM_SOURCE` cross a `x86_64-unknown-soso`

- **Problema:** GN, clang Chromium, Python, sin Fontconfig en rootfs.
- **Coste:** mayor que A; insostenible sin equipo dedicado.

### C. V8 jitless / sandbox off

- Reduce superficie JIT pero **sigue siendo V8** + mismos syscalls tokio/curl.
- Compatibilidad web de moli cae; no alinea con el valor de moli (~81 % Lexbench).

### D. Parchear libcurl → sin poll (busy / select simulado en userspace)

- Factible en teoría sobre `soso-std` + TCP soso; **no** resuelve V8 ni tokio
  epoll ni OpenSSL embebido.

### E. `SYS_POLL` / epoll en kernel

- **Coste:** semanas de kernel + pruebas; **insuficiente** sin resolver V8 y
  dynamic linking.

### F. Mantenimiento upstream

- ~95 crates workspace, Stylo/Taffy forkados, 2650+ commits recientes.
- Parches estilo `config/rust-soso/patch-*.py` **por decenas** de crates `std`.

**Orden de magnitud:** port completo **>>** esfuerzo Sal + self-hosting + GPU;
comparable a traer Chromium sin sandbox.

---

## 5. Veredicto

### No viable (nativo completo)

Criterios cumplidos:

1. **Dependencia irreducible de V8** con binarios solo para Linux glibc y sin
   ruta credible a `unknown-soso`.
2. **tokio + epoll + señales** en el camino caliente (strace).
3. **curl multi + poll** (`poll_7_68_0`).
4. **Fontconfig** para layout/screenshot/PDF como en Linux.
5. **Binario de release dinámico**, incompatible con ELF estáticos actuales de
   soso (sin `ld-linux`).
6. **Coste de mantenimiento** desproporcionado frente a `soso-web` + agente LLM
   ya presentes.

### Viable solo a medias (descartado como objetivo)

- moli **sin** `--layout` y **sin** servidor tokio: seguiría necesitando V8 para
  JS; no es un subconjunto mantenible del upstream.

### Alternativa nativa recomendada (estilo D10)

1. **Agente / herramientas** en Rust (`soso-improve`, `soso-http`): fetch +
   extracción para el LLM, sin JS general.
2. **Reutilizar ideas/crates puros** evaluados aparte (licencia Apache/MIT):
   `moli-html2md`, parsers CSS/layout, **sin** arrastrar `moli-renderer-v8`.
3. **Seguir ampliando** [`soso-web-core`](../crates/soso-web-core/) y
   [`soso-web`](../user/soso-web/) para lectura gráfica estática (sin JS).
4. Si en el futuro soso tuviera **`soso-std` en rootfs**, epoll y libc C++ de
   producción, **reabrir** solo la fase “sondas + spike V8 hello world”, no un
   port del monolito moli.

---

## 6. Entregables de esta evaluación

| Entregable | Ubicación |
|------------|-----------|
| Documento | `docs/EVALUACION-MOLI.md` (este fichero) |
| Sondas guest | `user/soso-agent-probe/src/sondas/moli.rs` + registro en `main.rs` |
| Medición host | `~/.cache/moli` @ `0450415`, binario v1.1.19, logs `/tmp/moli-strace-*.txt` |

No se ha modificado el kernel, `soso-web`, ni se ha añadido moli al repositorio.
