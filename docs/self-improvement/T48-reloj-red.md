# T48 — Añadir reloj y transporte nativos para evaluaciones

**Hito:** SI-2 · **Tipo:** Implementación portable y validación guest · **Estado:** hecha (2026-09-23).

**Dependencias:** [T45](T45-cli-capacidades.md)

## Objetivo y entrega

Cliente y servidor de pruebas usan reloj monotónico y transporte inyectables sin std en la política.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), C5–C7, y [NATIVO.md](NATIVO.md).

- [user/libsoso/src/sys.rs](../../user/libsoso/src/sys.rs)
- [user/soso-llm/src/net.rs](../../user/soso-llm/src/net.rs)
- [crates/soso-improve-core/src/entorno.rs](../../crates/soso-improve-core/src/entorno.rs)

## Archivos que se pueden cambiar

Traits pequeños de reloj/transporte y adaptadores host/guest del coordinador; fixtures de TCP y plazo. Reutilizar codec de soso-llm-api cuando T10–T12 estén disponibles.

Se permiten manifiestos, lockfiles, manual y skills relacionados. Si una capacidad exige cambios independientes, usar [PLANTILLA.md](PLANTILLA.md) antes de ampliar alcance.

## Pasos

1. Definir tiempo monotónico, unidad, overflow y deadline; adaptar clock_gettime y simular el reloj en pruebas.
2. Definir conexión, lectura/escritura parciales y cierre distinguiendo pendiente/EAGAIN, EOF y error; todo buffer y espera tiene límite.
3. Implementar TCP guest con libsoso y adaptador std host. No duplicar HTTP/SSE dentro del transporte.
4. Comprobar cancelación y vencimiento durante conexión, escritura y lectura; liberar recursos y mantener el motivo real de fallo.

## Comprobación

Echo controlado entre procesos soso: fragmentación byte a byte, UTF-8 partido, cierre temprano, cliente lento y timeout. El informe registra tiempos monotónicos y nunca confunde EOF con espera. No requiere modelo ni OpenCode.

Los subcomandos nuevos se ejecutan después de implementarlos. Guardar comando, cwd, entradas, plataforma, exit code y hashes; las pruebas host son apoyo de desarrollo.

## Cierre y condición de bloqueo

- [x] Entrega y comprobaciones terminadas con evidencia.
- [x] Catálogo, índice y seguimiento sincronizados.
- [x] Validación nativa registrada por separado, sin inferirla de la compilación.

Entregado en `crates/soso-improve-core/src/tiempo.rs` (`Reloj`, `Plazo`,
`RelojSimulado`) y `transporte.rs` (`Paso::{Hecho,Espera,Fin}`, `Transporte`,
`Conector`, `escribir_todo`/`leer_hasta_fin`/`leer_exacto`, `TransporteSimulado`),
con adaptadores host (`std::net`, `Instant`) y guest (`tcp_connect`,
`read_timeout`, `uptime_ms`). La orden `soso-improve eco` ejecuta la
comprobación **entre dos procesos de soso**, sin proxy del host. Resumen en
[seguimiento/T48.md](seguimiento/T48.md); evidencia en
`target/self-improvement/tasks/T48/resultado.md`.

**Límites.** El transporte mueve bytes: no hay HTTP ni SSE dentro. El
`Transport` de `soso-llm-core` —que sí confunde espera con cierre— no se ha
migrado; hacerlo aquí habría arrastrado `soso-llm` entero. No hay cancelación
explícita todavía, y por eso el sondeo está acotado a 20 ms.
`native_validation` queda en **partial**: el eco corre en soso, pero lo lanza
el arnés del host, y repetirlo desde el runner nativo es
[T49](T49-pruebas-guest.md).

Una syscall ausente origina una ficha con sonda. No reemplazar el transporte guest por un proxy host para marcarlo verificado.

Resumen durable en seguimiento/T48.md al comenzar; artefactos en la raíz configurable de NATIVO.md, bajo tasks/T48/.

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). La lógica y las aserciones se comparten con el guest; los adaptadores usan capacidades acreditadas de soso. Las pruebas host permiten desarrollar esta entrega, pero no sustituyen su validación nativa.

Validación nativa: **pendiente**. Estas condiciones no son dependencias para iniciar el desarrollo. Registrar evidencia y capacidades pendientes en tasks.json y seguimiento. Artefactos guest bajo /var/self-improvement/ (raíz configurable).
