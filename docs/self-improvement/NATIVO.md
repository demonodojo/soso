# Contrato de ejecución nativa de la automejora

**Revisión:** 16 de septiembre de 2026. **Requisito del usuario:** todo el
circuito de automejora debe poder ejecutarse dentro de soso. Este documento
precisa [CONTRATO.md](CONTRATO.md) y sus fichas; usarlo al elegir dependencias,
implementar adaptadores y decidir qué significa «completado».

## 1. Alcance: la aplicación completa, no solo la inferencia

Deben ejecutarse en soso captura/reconstrucción, lectura del banco, selección
de tareas, evaluación, OpenCode y herramientas, compilación, verificadores,
promoción de versiones, informes, recuperación y reanudación. La lógica
común va en Rust `no_std + alloc`; los adaptadores guest usan la ABI de soso.
Que una crate compile sin std no demuestra que su aplicación funcione allí.

Linux puede ayudar a desarrollar y producir la primera imagen. Sus arneses
no serán dependencias de ejecución del producto final. Un uso temporal de
Forja o OpenCode host se registra como etapa mixta. Antes del cierre final se
debe repetir la operación sin ese servicio, no solo declarar que es portable.

No introducir Python, Bash, `sh -c`, Git CLI, contenedores, `std::os::unix`,
QEMU o herramientas Linux como requisitos permanentes. OpenCode conserva sus
dependencias reales de runtime, que deberán portarse y probarse en SI-6;
reescribir el coordinador en Rust no elimina esa tarea.

## 2. Base existente y huecos observados

Inspección de código del 16 de septiembre; **no es una ejecución guest**:

| Pieza | Estado observado | Trabajo que acredita su uso nativo |
|---|---|---|
| `crates/soso-improve-core` | Captura por contenido, banco y verificadores; traits `Archivos` y `Procesos` | Reutilizar; ampliar mecanismos en T45–T50 |
| `tools/soso-improve` | Adaptador host existente, pruebas T01/T02 en Rust | No recrear la crate en T23; extraer cualquier política host restante |
| `user/soso-improve` | Captura, reconstrucción, banco y protocolo conectados al core | Probar CLI y resultados en T45/T49 |
| CLI guest | Sintaxis posicional distinta del host; varias verificaciones imprimen fallo y devuelven `Ok(())` | T45: contrato compartido y exit codes correctos |
| Procesos guest | Une argv con espacios, cambia cwd sin restaurarlo, mezcla stderr/stdout; no usa stdin, entorno ni timeout de `Orden` | T47: implementación completa o error de capacidad |
| Persistencia | `Archivos` aún no expresa rename/sync/operaciones durables | T46: contrato real sobre sosofs; las syscalls existen pero hay que comprobar su semántica |
| Casos de programación/repositorio | Core compartido; ejecución guest pendiente de compilador/Cargo | T40/T41 y pruebas completas T51 |
| Referencias de casos repo | El host aún usa `git apply` mediante callback | T50: paquete de cambios por contenido, aplicable sin Git |

T01/T02 conservan su cierre histórico y sus pruebas. El campo adicional
`native_validation` del catálogo registra qué queda por demostrar dentro de
soso; no atribuir a esos cierres el comportamiento futuro de T45–T51.

## 3. Arquitectura que se debe reutilizar

```text
                    soso-improve-core (no_std + alloc)
                    política, formatos, casos y resultados
                              ↓ traits
          ┌───────────────────┴───────────────────┐
   tools/soso-improve                       user/soso-improve
   adaptadores std, desarrollo               adaptadores libsoso, producto

   soso-llm-core ← soso-llm-api ← servicio soso-llm
   formatos y cliente de evaluación compartidos; sin dependencia cíclica
```

Conservar nombres de tipos y módulos existentes: `Archivos`, `Procesos`,
`Orden`, `Salida`, `captura`, `caso`, `programa`, `protocolo`, `repo`.
Extender capacidades mediante traits pequeños, evitando un nuevo framework.
La crate API no dependerá del coordinador; el coordinador puede consumirla.

| Mecanismo | Contrato y propietario |
|---|---|
| CLI / capacidades | T45; una orden serializable y un resultado compartidos, errores explícitos |
| Archivos durables | T46; rename/sync/creación exclusiva, errores de close y recuperación |
| Procesos | T47; argv/env/cwd/stdin exactos, stdout y stderr separados, deadline, identidad y limpieza de hijos |
| Reloj y red | T48; reloj monotónico inyectable, TCP incremental con EOF/EAGAIN, deadlines y cancelación |
| Harness guest | T49; pruebas compartidas llamables sin libtest, fixtures y resultados estructurados |
| Fuentes / cambios | T50; manifiestos y objetos por hash, aplicar/exportar sin Git |
| Aceptación completa | T51; todas las capacidades anteriores y los ports de agente/toolchain realmente ejecutados |

No ignorar un campo de una orden porque el adaptador no lo soporta. Consultar
capacidades y rechazarlo antes del efecto. Los límites de memoria y salida se
aplican durante lectura/generación, no después de reservar todo el contenido.

## 4. Fuentes, autoridad y almacenamiento

- Identidad primaria de una base = hash del inventario y sus objetos. Commit,
  rama, staged/unstaged y parches Git son metadatos o exportaciones opcionales.
  Construir un árbol de trabajo desde T01; no depender de `git worktree`.
- Un cambio aplicable contiene base esperada, rutas, hashes antes/después,
  adiciones y borrados; binarios como objetos. Construir candidato inmutable
  y validar antes de cambiar la referencia activa. No aplicar parcialmente
  a la base viva. T50 especifica el formato sin cambiar silenciosamente el
  manifiesto de T01 o la huella del banco.
- Guardar resultados en raíz configurable. Convención host:
  `target/self-improvement/`; guest: `/var/self-improvement/`. Las rutas dentro
  de manifiestos se resuelven respecto a la raíz declarada y conservan hashes.
  El código nunca exige `target/` ni un home Linux para recuperar un intento.
- Directorios diferentes no aíslan procesos: soso es monousuario. Las pruebas
  reservadas y la autoridad de promoción deben estar fuera de la instancia
  modificable por el candidato. Puede usarse otra instancia de **soso**, con
  receptor/ejecutor nativo, paquetes identificados y resultados correlacionados.
  Definir ese protocolo y sus límites en T26/T43; no asumir un sandbox POSIX.
- QEMU/SSH desde Linux son infraestructura de laboratorio. La campaña final
  usa instancias físicas de soso o infraestructura cuya puesta en marcha,
  control y recuperación no requieran un comando Linux durante el ciclo.
  Si falta control nativo del destino candidato, el cierre queda pendiente.

## 5. Pruebas y dependencias de bootstrap

Toda función del ciclo tiene dos comprobaciones: pruebas rápidas host con
fixtures y prueba de su adaptador/ejecución guest. Las primeras no sustituyen
las segundas. No exigir libtest ni Cargo dentro de soso para probar captura,
HTTP, estado o protocolo: T49 ejecuta casos compartidos desde un binario
guest precompilado. Compilar nuevos candidatos sí necesita T40/T41.

El banco reservado conserva entradas y resultados, no ejecutables Linux.
Identificar para cada caso target, compilador, runner y fixture. Si el caso
presupone permisos, herramientas o rutas Unix, escribir una adaptación
equivalente documentada o declarar capacidad pendiente. Cambiar el banco
requiere nueva huella y comparación; no sustituir tests fallidos por otros
más débiles ni cambiar el denominador.

| Dependencia de desarrollo | Sustitución / prueba de salida obligatoria |
|---|---|
| Tokenizer oficial o utilidades de referencia externas | T03 importa fixtures independientes con revisión/hashes; consumo y comparación nativos. Para nuevas entradas, implementar tokenización y renderer de la familia en Rust; no invocar el generador externo por petición |
| `cargo test` host / scripts hostcheck | T49 y T43: mismo núcleo de aserciones con runner guest; C requiere toolchain de T42 |
| `rustup`, bootstrap `.sh`, `x.py`, helpers Python de upstream | T38 inventaría dependencias transitivas; T39 captura receta y artefactos semilla; T40–T42 reproducen build nativo con ejecutor Rust y herramientas portadas |
| Git y `git apply` | T01/T24/T50: árboles y paquetes por contenido; referencia Git solo auxiliar |
| Forja | T36/T37 conservan la etapa mixta; T51 ejecuta con el servidor inaccesible |
| Empaquetado `.sh`, utilidades ELF/GPT externas | T35/T42: bibliotecas o ejecutables nativos; scripts externos solo envoltorios de desarrollo |
| OpenCode host y su gestión de paquetes | T32–T35: runtime y dependencias fijados, instalación offline y herramientas nativas. No basta portar el ejecutable sin búsqueda, edición y proceso de prueba |

La semilla inicial precompilada debe tener origen y hashes. A partir de ella
la herramienta nativa debe poder reconstruir el perfil declarado. Una receta
que todavía ejecute `x.py` en Linux no completa SI-7. Auditar también build.rs,
proc macros, generadores, ensamblador y herramientas transitivas.

## 6. Dependencias sin bloquear el bootstrap con su propio cierre

`depends_on` sigue ordenando entregas de desarrollo. Se añaden T45–T50 como
dependencias directas cuando un componente usa su mecanismo. Los campos
`native_validation.requires` describen condiciones para **validación nativa**,
no para empezar a implementar: exigir OpenCode nativo para desarrollar su
propio banco produciría un ciclo artificial.

Estados de `native_validation.status`: `pending`, `partial`, `verified`.
`not_applicable` solo para arneses de laboratorio o inventarios sin ejecución;
deben indicar qué consumidor nativo acredita su función. No calcular
`verified` a partir de `status: done`, lenguaje Rust o un ELF generado.
Adjuntar evidencia guest (build, comando, inputs, exit code, artefactos y
capacidades ausentes). T01/T02 tienen cierre previo con validación nativa
pendiente de T49 y, para casos compilados, T40/T41/T51.

Actualizar índice, ficha y catálogo al cambiar dependencias; preservar los
informes históricos. Todas las fichas remiten a este contrato, incluidas las
N-xxx/C-xxx derivadas. T44 exige T51 en verde y los criterios del padre.

## 7. Cierre que acredita «todo en soso»

T51 prueba captura → tarea → OpenCode → herramientas → build → verificación
→ promoción → reinicio/recuperación → informe desde soso. Deshabilitar Forja,
endpoints de modelos externos y ejecutores Linux antes de empezar. Registrar
cada ejecutable, plataforma, recursos y conexión usada. La red entre instancias
soso está permitida; una compilación Linux escondida tras TCP no lo está.

El cierre incluye al menos una prueba negativa por mecanismo (fallo de test,
timeout, corte de red y corte de escritura) con exit codes correctos. T44
repite tres mejoras reales con esta configuración y registra cualquier paso
externo residual como bloqueo, no como limitación compatible con el cierre.
