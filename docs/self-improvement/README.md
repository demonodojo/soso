# Subplanes de automejora para modelos pequeños

**Plan padre:** [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md).
**Revisión:** 27 de septiembre de 2026. **Estado:** en curso, sin cierre del circuito.
**Catálogo:** 80 fichas: 63 completadas, 0 bloqueadas, 1 en curso y 16 pendientes.
`done` acredita la entrega de una ficha, no un hito ni ejecución nativa completa.

## Por dónde empezar

> **Estado (2026-10-08).** [T82 — prefill por lotes](T82-prefill-por-lotes.md) está hecha (el prompt de OpenCode
> en el guest baja de 64 a 20 min; [T81](T81-reutilizar-kv.md) deja los turnos siguientes en ~30 s).
> [T22](T22-primera-mejora.md) sigue abierta: el 7B llama a herramientas, pero aún no completa una edición exacta.
> **Lo siguiente:** aislar si el KV int8 del planificador del guest degrada la llamada a herramientas
> (el host con KV f16 la hace mejor), y atención por lotes para recortar los 20 min del primer turno.


**Ficha cerrada: [T42 — C, ensamblador y empaquetado por perfil](T42-c-link-imagen.md)**
(2026-10-03 → 2026-10-07, hecha con límites; ver `resultado.md`). Hecho: perfil fijado ([build-profile.json](native/build-profile.json)),
fichas de los huecos (C-113…C-143). **Dentro del guest** ya se compilan: el kernel
(`-Zbuild-std`, `wild-soso -pie`), `user/` entero con su C (clang/LLVM hecho para soso), las
herramientas de imagen, **las cinco etapas del cargador** y la imagen BIOS/UEFI, que arrancan;
y los puertos lxdde e1000e, ath11k (Steam Deck), iwlwifi y nouveau (kernel con nouveau compilado y arrancado en el guest). Lo que sigue fuera: construir LLVM
(se hace en el host y se enlaza para soso), el `boot-shim` del live y el empaquetado USB.
Mantenimiento del 8-oct: [C-144](native/C-144.md) integra jobserver corregido
como fuentes versionadas, con resolución local y pruebas host; validación
guest de esa corrección pendiente.
[T80](T80-proc-macros.md) (proc macros) está cerrada y [T41](T41-cargo-offline.md) también.
El perfil elegido es `qwen2.5-coder-7b` (GO 10/10, cobertura `completa`,
27-sep). El 3B queda como candidato anterior, NO-GO 8/10.
[T20](T20-opencode-config.md) está **hecha** (2026-10-07, [opencode.md](opencode.md)); T21 también está hecha; sigue T22.

| Prioridad | Trabajo | Condición de salida |
|---|---|---|
| 1 | [T74](T74-campana-7b-interrumpida.md) → T14 | **Hecha.** 27-sep: perfil elegido `qwen2.5-coder-7b`, GO 10/10 |
| 2, independiente | [T72](T72-heap-de-libstd.md): alocador compartido | **Hecha** 26-sep: `soso-alloc`, sonda guest 7/7; std compila el montón y falla después (T39) |
| 3, independiente | [T75](T75-receta-bootstrap-desfasada.md): receta portable | **Hecha** 26-sep: hashes iguales a 32d94cc9; sonda guest del espejo y humo de directorio vacío, exit 0 |
| 4, independiente (host) | [T76](T76-tokenizer-sin-retorno.md): tokenizer en tests | **Hecha** 26-sep: cuatro tests ya no retornan en silencio |
| 5, independiente (host) | [T77](T77-simd-avx2.md): SIMD en `gemm` | **Hecha** 26-sep: escalares siempre; AVX2 sólo con `RUSTFLAGS +avx2,+fma` |
| 6, independiente (host) | [T78](T78-entradas-integracion.md): ASR y banco | **Hecha** 26-sep: ASR `ignored`; banco exige `rustc`; `SOSO_REQUIRE_*` |
| T39 hecha | T40 hecha → T41 hecha → T42 | rustc y cargo corren en el guest; falta cerrar C/ensamblador/empaquetado (T42) y la reconstrucción nativa |
| Sólo con T14 GO | T20 → T21 → T22; luego T25–T29 | Primer parche útil validado antes de campaña de automejora |
| Tras T39 | T40 hecha → T41 hecha → T42 | T69 hecha: PAL antigua retirada. rustc y cargo ya corren en el guest |

Son frentes independientes, no una instrucción para lanzar agentes ni varias
campañas simultáneas. Implementar una ficha por sesión; los puertos del arnés
son compartidos. T08/T18/T45/T46 ya están terminadas: no recomendarlas otra vez.

## Estado comprobado y límites

- **Servicio:** T19 conserva evidencia histórica guest de 12/12 invariantes
  con pesos 3B (23-sep, exit 0). T16/T17 pasan a validación nativa **parcial**:
  existe ejecución, pero faltan el GO del modelo y cobertura específica de
  cancelación/cliente lento. No se reejecutó QEMU durante esta revisión.
- **Modelo:** el perfil elegido es `qwen2.5-coder-7b` (GO 10/10, 27-sep). El 3B
  queda como candidato anterior, NO-GO 8/10. El lock lleva hashes `.som`; la
  revisión HF del 7B figura como desconocida. El selector del live USB no es
  este perfil. Cada modelo conserva su informe.
- **Coordinador:** T23/T24 y mecanismos T45–T50 están implementados; faltan
  ejecutor, validación, recuperación y campaña. No hay primera mejora aceptada
  ni circuito autónomo acreditado.
- **Toolchain:** `wild` está instalado; T73 está cerrada. rustc y **cargo** corren
  en el guest (T40, T41): `cargo build --offline` compila un workspace de dos
  crates, uno con `build.rs`, y `soso-abi`. Ambos se construyen en Linux con
  `x.py` (receta en `config/rust-soso/README.md`); sin proc macros ([T80](T80-proc-macros.md))
  ni compilación incremental. El std de soso (`library/std/src/sys/pal/soso/`)
  ya exporta `malloc`, claves TLS de pthread, reloj y entropía para el C que
  cargo arrastra (libgit2, OpenSSL). T75 está cerrada.
- **Nativo:** T32–T34 inventarían y prueban sustrato; T35 todavía no tiene un
  runtime OpenCode portado. Backlog N-xxx: 7 hechas, 2 decisiones tomadas,
  1 aplazada, 2 pendientes y 1 decisión pendiente. «Decidida» no significa
  capacidad implementada. El cierre SI-6/SI-7 sigue pendiente.

La revisión y sus evidencias están en
[seguimiento/REVISION-2026-09-26.md](seguimiento/REVISION-2026-09-26.md).
[DECISIONES.md](DECISIONES.md) distingue decisiones ya tomadas, ruta de trabajo
planificada y asuntos aplazados. La narración anterior se conserva como
[historial](seguimiento/HISTORIAL-INDICE-2026-09-26.md), sin vigencia operativa.

### Prompt listo para copiar

```text
Reanuda docs/self-improvement/T40-compilador-nativo.md.
Lee su seguimiento y el next_step del catálogo (C-002, getrandom 0.3.3).
Comprueba el checkout, las dependencias y las condiciones de entrada.
Implementa y verifica sólo esta ficha; guarda comandos, exit codes y artefactos.
Sincroniza catálogo, ficha, índice y seguimiento. Distingue host y guest.
No declares cierre sin evidencia ni empieces automáticamente la siguiente ficha.
```

## Decisiones resueltas para reducir el trabajo de arquitectura

- Tipos de chat en core; HTTP/SSE en una crate API separada; sockets en
  userspace. Direcciones de dependencias y firmas en C1–C2.
- Una sesión residente, una generación activa y ninguna generación en cola.
  Cancelación cooperativa y conflicto de listeners definidos en C3–C4.
- Perfil de modelo fijado mediante evidencia; JSON Schema admitido explícito;
  errores y límites sin truncamiento silencioso.
- El primer servidor host es una herramienta de desarrollo. El primer cierre
  útil exige inferencia guest y un parche real validado independientemente.
- Core Rust no_std y adaptadores host/guest existentes. T45–T50 cubren CLI,
  durabilidad, procesos, red/reloj, runner y paquetes por contenido sin Git.
  T51 comprueba el circuito sin Forja ni ejecutores Linux; T44 lo repite.
- Estado, ejecutor, aislamiento, validación y recuperación del coordinador
  están separados para que cada ficha tenga una responsabilidad.
- Datos de evaluación y casos reservados tienen trazabilidad; las mejoras se
  cuentan al aceptarse, no cuando el modelo las afirma.

El contrato precisa el plan padre. En particular, exige token desde la primera
API guest porque la syscall actual escucha por puerto, sin bind por dirección;
y separa captura base de calibración, que requiere un servidor ejecutable.
Las implementaciones puras pueden avanzar antes de terminar la evaluación del
modelo. Esto no rebaja los criterios de cierre de SI-0/SI-1.

## Índice y dependencias

`depends_on` ordena entregas de desarrollo. `native_validation` registra
estado, condiciones `requires` y evidencia de ejecución en soso. Estas
condiciones adicionales se comprueban al validar, sin crear ciclos para
implementar el bootstrap. Hay evidencia nativa parcial y verificada; consultar
el campo de cada ficha, sin inferirla de su estado de implementación.

Una dependencia indica entrega necesaria, no autorización para implementarla
junto con la ficha. **Pendiente** significa sin evidencia de implementación.
Un resultado de evaluación no-go se registra como bloqueo de consumidores.

Convención de estados y reanudación:
[seguimiento compartido de las skills](../../.claude/skills/soso-architecture/references/planes.md).
Sincronizar `tasks.json`, encabezado/casillas de la ficha y fila del índice;
registrar el resumen durable en `seguimiento/Txx.md` al comenzar esa tarea.

| ID | Entrega | Hito | Dependencias | Estado |
|---|---|---|---|---|
| [T01](T01-base.md) | Capturar una base reproducible sin alterar el checkout | SI-0 | — | Completada |
| [T02](T02-banco.md) | Definir los casos y sus verificadores | SI-0 | T01 | Completada |
| [T03](T03-perfil-modelo.md) | Fijar un modelo y obtener fixtures independientes de su chat | SI-0 / SI-1 | T01 | Completada |
| [T04](T04-dominio-chat.md) | Añadir tipos de conversación compartidos | SI-1 | — | Completada |
| [T05](T05-validacion-chat.md) | Validar historial y esquemas de herramientas | SI-1 | T04 | Completada |
| [T06](T06-render-chat.md) | Renderizar la familia elegida con historial completo | SI-1 | T03, T05, T52, T53 | Completada |
| [T07](T07-parse-herramientas.md) | Extraer llamadas válidas de la salida del modelo | SI-1 | T03, T05 | Completada |
| [T08](T08-resultado-generacion.md) | Devolver motivo de parada y consumo real del runtime | SI-1 / SI-2 | — | Completada |
| [T09](T09-cancelacion-runtime.md) | Añadir cancelación cooperativa a prefill y decode | SI-2 | T08 | Completada |
| [T10](T10-api-json.md) | Crear la crate API y adaptar peticiones JSON | SI-2 | T05, T06, T08 | Completada |
| [T11](T11-http.md) | Leer HTTP fragmentado con límites explícitos | SI-2 | T10 | Completada |
| [T12](T12-respuestas-sse.md) | Emitir respuestas completas y eventos SSE | SI-2 | T07, T08, T10 | Completada |
| [T13](T13-servidor-host.md) | Conectar el mismo runtime a un servidor de desarrollo en host | SI-1 / SI-2 | T09, T11, T12 | Completada |
| [T14](T14-evaluacion-modelo.md) | Medir calidad y fijar presupuestos antes de usar el agente | SI-0 / SI-1 | T02, T03, T13, T48 | Completada — perfil 7B **GO** 10/10 (27-sep) |
| [T15](T15-sesion-residente.md) | Extraer la sesión residente manteniendo ask | SI-2 | T08, T09 | Completada |
| [T16](T16-servicio-guest.md) | Servir HTTP en guest con el modelo residente | SI-2 | T10, T11, T12, T14, T15 | Completada (guest parcial; T14 NO-GO) |
| [T17](T17-admisiones-cancelacion.md) | Atender ocupado, health y desconexión durante inferencia | SI-2 | T09, T16 | Completada (guest parcial; cancelar/cliente lento pendientes) |
| [T18](T18-puertos-qemu.md) | Añadir reenvío HTTP configurable sin colisiones | SI-2 | — | Hecho (e2e T19) |
| [T19](T19-qemu-e2e.md) | Crear la prueba completa de API dentro de soso | SI-2 | T16, T17, T18 | Completada (guest 12/12 con pesos reales) |
| [T20](T20-opencode-config.md) | Configurar OpenCode para el proveedor soso | SI-3 | T14, T19 | **Hecha** (2026-10-07; `opencode.json` + agente `soso-improve`, sin inferencia) |
| [T21](T21-opencode-contrato.md) | Capturar el contrato real de OpenCode sin depender del modelo | SI-3 | T20, T47, T48 | **Hecha** (2026-10-07; contrato contrastado con OpenCode real, 3 correcciones del servidor) |
| [T22](T22-primera-mejora.md) | Resolver una tarea real usando la inferencia guest | SI-3 | T02, T19, T21 | En curso (2026-10-08): el 7B no llama a herramientas con el prompt de OpenCode; bloqueada por T82 |
| [T23](T23-estado-coordinador.md) | Crear el formato de tareas y estados del coordinador | SI-4 | T01, T02, T46, T45 | Completada (guest acreditado; destapó T63) |
| [T24](T24-checkout.md) | Preparar una copia de tarea y exportar su parche | SI-4 | T23, T50 | Completada (guest acreditado) |
| [T25](T25-ejecutor.md) | Ejecutar OpenCode con límites y logs | SI-4 | T21, T24, T47, T48 | Pendiente |
| [T26](T26-validador.md) | Validar candidatos y promover solo los aceptados | SI-4 | T24, T25 | Pendiente |
| [T27](T27-reanudacion.md) | Reanudar tras caída sin repetir efectos | SI-4 | T23, T25, T26 | Pendiente |
| [T28](T28-conocimiento.md) | Guardar y recuperar aprendizaje verificado | SI-4 | T26 | Pendiente |
| [T29](T29-campana.md) | Ejecutar una campaña reproducible de diez tareas | SI-4 | T22, T27, T28, T49 | Pendiente |
| [T30](T30-hardware.md) | Medir servicio e inferencia en una máquina física identificada | SI-5 | T19, T29, T48 | Pendiente |
| [T31](T31-restauracion.md) | Demostrar recuperación completa de la instalación | SI-5 | T30 | Pendiente |
| [T32](T32-opencode-inventario.md) | Inventariar dependencias del OpenCode que se quiere portar | SI-6 | T01 | Completada (v1.18.32 fijada) |
| [T33](T33-sondas-abi.md) | Crear sondas pequeñas para las capacidades requeridas | SI-6 | T32, T49 | Completada (7 sondas; abrió T65 y T66) |
| [T65](T65-o-excl-no-excluye.md) | `O_EXCL` no excluye mientras el primer descriptor sigue abierto | SI-4 | — | Completada (reserva el nombre al abrir) |
| [T66](T66-guarda-de-pila-fingida.md) | La guarda de pila de los hilos no existe, y el código finge que sí | SI-4 | — | Completada (deja de fingir; la guarda es N-002) |
| [T34](T34-tickets-port.md) | Convertir huecos del port en fichas implementables | SI-6 | T32, T33 | Completada (11 fichas N-xxx) |
| [T35](T35-opencode-nativo.md) | Probar OpenCode nativo en modo no interactivo | SI-6 | T19, T34 | Pendiente |
| [T36](T36-forja-trazabilidad.md) | Vincular fuentes, build y artefacto de Forja | SI-6 | T01 | Completada (recibo v2, 2026-09-25) |
| [T37](T37-mejora-nativa-forja.md) | Cerrar una mejora desde OpenCode nativo con build remoto | SI-6 | T22, T35, T36 | Pendiente |
| [T38](T38-toolchain-inventario.md) | Fijar revisiones y dependencias de la toolchain nativa | SI-7 | T01 | Completada (2026-09-25; abrió T67 y T68) |
| [T39](T39-bootstrap-libstd.md) | Hacer reproducible el bootstrap de libstd para soso | SI-7 | T38, T50, T72, T75 | **Completada** (2026-09-26; build-std, manifiesto, humo guest; native_validation pending) |
| [T40](T40-compilador-nativo.md) | Descomponer y acreditar el port del compilador | SI-7 | T38, T39 | **Completada** (2026-09-29; `/tmp/t` imprime `hola-t40` y sale con 7; native_validation parcial) |
| [T41](T41-cargo-offline.md) | Validar Cargo y fuentes reproducibles dentro de soso | SI-7 | T40 | Completada (2026-10-02: workspace, `build.rs` y `soso-abi` en el guest; sin proc macros) |
| [T42](T42-c-link-imagen.md) | Cerrar C, ensamblador y empaquetado por perfil | SI-7 | T38, T41, T80 | **Hecha** (2026-10-07; validación nativa parcial: LLVM y SASS en el host, falta placa) |
| [T43](T43-validacion-actualizacion-nativa.md) | Validar y recuperar candidatos construidos en soso | SI-7 | T31, T37, T42 | Pendiente |
| [T44](T44-cierre-nativo.md) | Repetir tres mejoras con agente, modelo y build en soso | SI-7 | T29, T43, T51 | Pendiente |
| [T45](T45-cli-capacidades.md) | Unificar órdenes, capacidades y códigos de salida | SI-0 | T01, T02 | Completada |
| [T46](T46-archivos-durables.md) | Acreditar persistencia y actualización de referencias en sosofs | SI-4 | T01 | Completada (reinicio real) |
| [T47](T47-procesos-nativos.md) | Ejecutar procesos con argumentos, canales y límites exactos | SI-4 | T45, T48 | Completada (con límites: T61, T62) |
| [T48](T48-reloj-red.md) | Añadir reloj y transporte nativos para evaluaciones | SI-2 | T45 | Completada |
| [T49](T49-pruebas-guest.md) | Ejecutar casos compartidos desde un runner nativo | SI-0 / SI-4 | T45, T46, T47, T48 | Completada (con límites) |
| [T50](T50-cambios-contenido.md) | Aplicar y exportar cambios sin Git | SI-4 | T01, T46 | Completada (pasos 4–5 aparte) |
| [T51](T51-aceptacion-circuito-nativo.md) | Acreditar todo el circuito de automejora dentro de soso | SI-7 | T29, T35, T37, T41, T42, T43, T49, T50 | Pendiente |
| [T52](T52-tokenizer-merges.md) | Llevar las fusiones BPE al formato .som y al convertidor | SI-1 | — | Completada |
| [T53](T53-tokenizer-bpe.md) | Segmentar por fusiones BPE en soso-llm-core | SI-1 | T52 | Completada |
| [T54](T54-accept-externo.md) | Aceptar conexiones externas en un listener de userspace | SI-2 | — | Completada |
| [T55](T55-accept-sin-plazo.md) | `tcp_accept(fd, 0)` duerme el servidor para siempre | SI-2 | T54 | Completada |
| [T56](T56-utf8-tool-parser.md) | El parser de salida parte caracteres UTF-8 y mata el servidor | SI-1 / SI-2 | — | Completada |
| [T57](T57-medio-cierre.md) | El medio cierre del cliente mataba la respuesta | SI-2 | T54, T55, T56 | Completada |
| [T58](T58-respuesta-con-prompt.md) | La respuesta de chat devolvía el prompt pegado a la generación | SI-2 | — | Completada |
| [T59](T59-tool-calls.md) | Herramientas: investigación — **resultado: límite del modelo** | SI-2 / SI-3 | — | Cerrada |
| [T60](T60-validacion-peticion.md) | Errores de la API: clasificación y doble respuesta en streaming | SI-2 | — | Completada |
| [T61](T61-pipe-no-bloqueante.md) | Lectura de tuberías con plazo, sin bloquear | SI-4 | — | Completada |
| [T62](T62-argv-en-los-programas.md) | Los programas reciben los argumentos juntados, no su argv | SI-4 | — | Completada (5 productores; abrió T64) |
| [T64](T64-sosh-comillas.md) | `sosh` no entiende comillas: una ruta con espacios es inescribible | SI-4 | T62 | Completada (guest acreditado) |
| [T63](T63-generacion-rota-encalla.md) | Desencallar una referencia durable con una generación rota | SI-4 | T46 | Completada (guest acreditado; desbloquea T27) |
| [T67](T67-sosoas-elf-desplazado.md) | El ELF de sosoas tiene la cabecera desplazada dos bytes | SI-7 | T38 | Completada (`objdump` desensambla; sin tabla de símbolos) |
| [T68](T68-wild-soso-nombre.md) | El enlazador wild-soso no existe con ese nombre | SI-7 | T38 | Completada (nombre y rutas; `wild` 0.10.0 instalado) |
| [T69](T69-apply-patches-no-completaba.md) | `dl.rs` se copia al vendor y nadie lo compila | SI-7 | T39 | **Completada** (2026-09-26; generación antigua retirada; build-std sigue en verde) |
| [T70](T70-sigterm-no-termina.md) | SIGTERM no termina un proceso dormido | SI-6 | — | **Hecha** (2026-10-07: SIGTERM mata a los bloqueados, 143; SIGINT conserva `EINTR`) |
| [T81](T81-reutilizar-kv.md) | Reutilizar el prefijo del KV entre peticiones | SI-3 | — | **Hecha** (2026-10-08; turnos 2+ de ~500 s a ~20 s, exacto) |
| [T82](T82-prefill-por-lotes.md) | Prefill por lotes | SI-3 | T81 | **Hecha** (2026-10-08: guest 3 846 s → 1 217 s con el prompt de OpenCode, 3,2×) |
| [T71](T71-vendor-sin-cache.md) | `xtask` buscaba el vendor de Rust donde nunca está | SI-7 | — | Completada (2026-09-25; destapó la cadena de T39) |
| [T72](T72-heap-de-libstd.md) | Compartir el alocador entre libsoso y std | SI-7 | — | **Completada** (2026-09-26; sonda guest 7/7; std deriva errores a T39) |
| [T73](T73-futex-sin-plazo.md) | El futex de soso no tiene plazo, y `Condvar::wait_timeout` lo necesita | SI-7 | — | **Completada** (2026-09-26; acreditada en QEMU; `std` baja a 2 errores) |

| [T74](T74-campana-7b-interrumpida.md) | Recuperar y acreditar campaña 7B | SI-0 / SI-2 | T14, T19 | Completada (NO-GO 8/10, cobertura completa) |
| [T75](T75-receta-bootstrap-desfasada.md) | Sincronizar receta Rust y bootstrap | SI-7 | T38, T50 | **Completada** (2026-09-26; host y sonda guest; std sigue en T39) |
| [T76](T76-tokenizer-sin-retorno.md) | Tests de tokenizer sin retorno silencioso | SI-1 | T03, T06 | Completada (2026-09-26) |

| [T77](T77-simd-avx2.md) | Tests SIMD de matvec sin retorno silencioso | SI-1 | — | Completada (2026-09-26) |
| [T78](T78-entradas-integracion.md) | Entradas ASR y banco sin ok silencioso | SI-0 | T02, T76 | Completada (2026-09-26) |
| [T79](T79-relleno-q10.md) | Relleno de Q10 según el contexto del modelo | SI-0 / SI-1 | T14, T74 | Completada (Q10 guest 3/3, 27-sep) |
| [T80](T80-proc-macros.md) | Proc macros y dylib en el guest | SI-7 | T41 | Completada (2026-10-03: proc macro mínimo y `zerocopy-derive`; `mkfs-soso` compila en el guest) |

## Condiciones adicionales de entrada

Los números del índice no describen por sí solos todas las condiciones. Estas
son obligatorias y también constan en [tasks.json](tasks.json):

- **[T03](T03-perfil-modelo.md)**: Pesos, tokenizer y plantilla originales de una revisión identificada.
- **[T06](T06-render-chat.md)**: Las divergencias de tokenizer detectadas en T03 están corregidas y verificadas.
- **[T14](T14-evaluacion-modelo.md)**: Pesos reales disponibles; un informe no-go no habilita las tareas consumidoras.
- **Criterio de la evaluación**: [evaluacion.md](evaluacion.md) — qué cuenta como éxito, los cinco desenlaces y de dónde salen los presupuestos.
- **[T16](T16-servicio-guest.md)**: T14 tiene resultado go, además de informe terminado.
- **[T19](T19-qemu-e2e.md)**: Modo de integración con pesos reales y entorno QEMU disponible.
- **[T54](T54-accept-externo.md)**: Entorno QEMU con reenvío de puertos (T18) y un servidor de userspace escuchando.
- **[T20](T20-opencode-config.md)**: GO de T14 con identidad y campaña completa; no basta `done`.
- **[T22](T22-primera-mejora.md)**: T14 mantiene resultado go para la misma identidad de modelo.
- **[T30](T30-hardware.md)**: Máquina física identificada, accesible y con red validada.
- **[T31](T31-restauracion.md)**: Destino de recuperación identificado y acceso de escritura autorizado.
- **[T35](T35-opencode-nativo.md)**: Todas las fichas N-xxx obligatorias de T34 están implementadas y sus sondas pasan.
- **[T39](T39-bootstrap-libstd.md)**: Revisión de toolchain de T38 y recursos de compilación disponibles.
- **[T40](T40-compilador-nativo.md)**: Fichas C-xxx necesarias implementadas antes del cierre de integración.
- **[T41](T41-cargo-offline.md)**: Cargo ejecutable en soso; las C-xxx adicionales se completan antes de la evaluación.
- **[T42](T42-c-link-imagen.md)**: Cada herramienta ausente se implementa mediante C-xxx antes del build de perfil.
- **[T43](T43-validacion-actualizacion-nativa.md)**: Destino de validación independiente y procedimiento de recuperación verificado.

**Los ports todavía no se pueden dividir honestamente por símbolo sin
inspeccionarlos.** T32/T38 fijan revisiones y dependencias; T33 produce sondas;
T34 y T40–T42 generan fichas N-xxx/C-xxx por hueco real, con firmas y archivos.
Estas fichas derivadas se ejecutan individualmente. T35/T37/T43/T44 son
integraciones condicionadas a su implementación, no encargos de «portar todo».

T33 y T40–T42 incluyen recetas repetibles por capacidad/componente. Ejecutar
un solo pase acotado cada vez y registrar cobertura de los restantes. Si se
necesita delegarlos a otro modelo, materializar primero la ficha N-xxx/C-xxx
correspondiente con la [plantilla](PLANTILLA.md).

## Cierres del plan padre

| Hito | Entregas y evidencia necesarias |
|---|---|
| SI-0 | T01–T03 y calibración T14; base reconstruible, banco y medidas |
| SI-1 | T04–T09, fixtures T03 y resultado go T14; compatibilidad y calidad |
| SI-2 | T10–T19 y evaluación T14 repetida contra el guest |
| SI-3 | T20–T22; herramienta real y primer parche aceptado |
| SI-4 | T23–T29 y mecanismos T45–T50; cinco mejoras de diez y recuperación; repetición íntegramente nativa en T51 |
| SI-5 | T30–T31; campaña física y restauración kernel/rootfs |
| SI-6 | T32–T37 más todas las N-xxx necesarias; OpenCode nativo y Forja trazable |
| SI-7 | T38–T44, T45–T51 y todas las N/C necesarias; campaña nativa T51 y tres ciclos finales T44 |

La numeración sirve de orientación. Por ejemplo, T23 puede prepararse después
del banco, y T32/T38 pueden investigarse sin esperar una campaña en hardware.

## Formato de entrega

Guardar por tarea en `target/self-improvement/tasks/Txx/` en host o
`/var/self-improvement/tasks/Txx/` en soso; raíz configurable según NATIVO.md:

```text
resultado.md     objetivo, cambio, evidencia, límites y siguiente estado
commands.json    argv, cwd, versiones, exit codes y duración
logs/            salida completa de verificaciones
artifacts.json   base, diff y hashes de artefactos relevantes
```

Para documentación basta verificar vínculos, estructura y coherencia. Para
código, ejecutar las pruebas de la ficha; al promover una base, las puertas
globales C6. Actualizar el estado del índice y de tasks.json juntos.

Esta revisión actualiza el seguimiento y el trabajo planificado; no acredita
nuevas ejecuciones ni implementa las fichas pendientes.
