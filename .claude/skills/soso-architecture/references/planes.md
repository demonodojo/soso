# Identificación y seguimiento de planes de soso

Leer esta referencia al ejecutar, retomar, desglosar o consultar el estado de
un plan. Las rutas en código son relativas a la raíz del repositorio; los
enlaces se resuelven desde este archivo. Las instrucciones del usuario fijan
el alcance: consultar o editar documentación no inicia su implementación.

## Encontrar el plan y la unidad de trabajo

1. Si el usuario indica archivo o ID, localizarlo primero. Un ID se interpreta
   dentro de su plan: `R5`, `G5`, `SI-5` y `T05` no son intercambiables.
2. Si dice «continúa», recuperar el plan y la tarea de la conversación y del
   seguimiento persistido. No reiniciar por T01 ni cambiar de plan por el
   archivo que esté abierto en el IDE.
3. Si falta una ruta o se desconoce el plan, buscar antes de darlo por ausente:

   ```sh
   rg --files -g '*PLAN*' -g '*SELF*' -g '*ROADMAP*' -g '*backlog*.json' -g 'tasks.json' -g '!target/**' -g '!rootfs/src/**' -g '!vendor/**' -g '!lxdde/linux/**' -g '!lxdde/reference/**'
   rg -n 'T05|SI-1|Estado|Dependencias' SELF_IMPROVEMENT.md docs/self-improvement
   ```

   Adaptar ID y directorio a la petición. Leer encabezados y enlaces antes de
   cargar un documento largo. Si quedan dos planes incompatibles, resolver la
   ambigüedad con el contexto o preguntar mientras se revisa lo independiente.

| Trabajo | Entrada y relación |
|---|---|
| Automejora, OpenCode, modelo de soso | Skill **soso-self-improvement**. Plan: [SELF_IMPROVEMENT.md](../../../../SELF_IMPROVEMENT.md) → [índice](../../../../docs/self-improvement/README.md) → [tasks.json](../../../../docs/self-improvement/tasks.json) → ficha Txx |
| Contratos y fichas derivadas | [CONTRATO.md](../../../../docs/self-improvement/CONTRATO.md), solo secciones de la tarea; [PLANTILLA.md](../../../../docs/self-improvement/PLANTILLA.md) para N-xxx/C-xxx |
| Entregas generales y pendientes | [PLAN_ASTRA.md](../../../../PLAN_ASTRA.md); comprobar su revisión e IDs actuales |
| Actualizaciones de instalaciones y logs en sosofs | [PLAN-ACTUALIZACIONES.md](../../../../docs/PLAN-ACTUALIZACIONES.md), entregas U0–U8; transición legacy y recuperación conjunta kernel/rootfs |
| Inferencia y modelos grandes | [PLAN-MODELOS-GRANDES.md](../../../../PLAN-MODELOS-GRANDES.md); distinguir etapas históricas de pendientes reales |
| GPU nativa y VRAM | [L6-native-autonomy.md](../../../../docs/L6-native-autonomy.md), gates enlazados y [PLAN-VRAM-16G-ROG.md](../../../../docs/PLAN-VRAM-16G-ROG.md) |
| Steam Deck / ath11k | [PLAN-STEAMDECK.md](../../../../docs/PLAN-STEAMDECK.md) |
| WiFi operativo | [WIFI-OPERATIVA.md](../../../../docs/WIFI-OPERATIVA.md) y diagnóstico del equipo relevante |
| Toolchain y compilación nativa | [SELF-HOSTING.md](../../../../docs/SELF-HOSTING.md); SI-6/SI-7 añaden agente e integración, no sustituyen los requisitos de toolchain |
| Estado y evidencia física | [ESTADO.md](../../../../docs/ESTADO.md), [HW-MATRIX.md](../../../../docs/HW-MATRIX.md), [hw-matrix.json](../../../../docs/hw-matrix.json) y `docs/DIAGNOSTICO-*.md` |

El mapa localiza documentos; no congela su estado. Contrastar revisión del
código y logs del mismo equipo/modelo/build. Un diagnóstico posterior puede
invalidar un GO histórico sin invalidar el trabajo de otros equipos. Registrar
la discrepancia y el alcance, no reemplazar estados a partir de la fecha sola.

## Elegir y retomar una ficha de automejora

Para implementar o avanzar, skill **soso-self-improvement** (no esta
referencia sola). Resumen:

- Resolver `tasks[].id` y abrir `file` relativo al directorio del catálogo.
  `context` contiene rutas desde la raíz. Leer la ficha, `depends_on`,
  `entry_conditions` y las secciones `contract_sections`; no todas las fichas.
- Si hay tarea activa dentro del encargo, retomar desde su resultado y siguiente
  paso persistidos. Antes de repetir pruebas, comprobar qué diff/base y perfil
  cubría la evidencia anterior y si siguen siendo los mismos.
- Una tarea candidata necesita dependencias cerradas **y** condiciones de
  entrada satisfechas con evidencia vigente. `done` en una auditoría no implica
  un port implementado. Un informe no-go no habilita un consumidor que exige go.
- Una dependencia ausente se registra; avanzar en otra tarea independiente
  solo si el alcance pedido lo permite. Los nombres de módulos y comandos en
  fichas pendientes son entregables propuestos, no capacidades existentes.
- Una ficha por contexto de modelo pequeño. Si el usuario encarga el plan
  completo, continuar con fichas habilitadas; si encarga una sola, terminarla
  sin iniciar otras. El catálogo no autoriza por sí mismo lanzar subagentes.

## Ejecución nativa de automejora

Leer [NATIVO.md](../../../../docs/self-improvement/NATIVO.md) al implementar
o redefinir una ficha. Reutilizar soso-improve-core no_std y adaptadores
tools/user existentes. La lógica del circuito debe ejecutarse dentro de soso;
host, Forja y scripts de bootstrap solo son etapas transitorias con prueba
de retirada. Revisar argumentos, exit codes, plazos, durabilidad y procesos:
una implementación de trait que ignora campos no acredita la capacidad.

Distinguir `depends_on` (orden de desarrollo) de `native_validation.requires`
(condiciones para probar en soso). Conservar cierres históricos T01/T02;
registrar validación nativa pending/partial/verified y evidence por separado.
`not_applicable` solo vale para laboratorio/inventario con consumidor nativo
identificado. No derivar verified de done, Rust o compilación cruzada.
Al cambiar estado nativo, sincronizar catálogo, sección de ficha y resumen;
T51 exige circuito completo sin Linux y T44 sus tres mejoras finales.

## Estados de seguimiento del catálogo

Esta convención se aplica a `docs/self-improvement/tasks.json`. Conservar el
vocabulario propio de los otros planes y anotar su equivalencia al informar.
Estos estados describen trabajo del plan; no son la máquina de estados del
futuro coordinador `soso-improve` ni un servicio de objetivos de la herramienta.

| `status` JSON | Ficha / índice | Uso |
|---|---|---|
| `pending` | Pendiente | No iniciado |
| `in_progress` | En curso | Trabajo iniciado, cierre aún incompleto |
| `blocked` | Bloqueada | Falta una dependencia, entrada o capacidad concreta que impide el siguiente paso |
| `done` | Completada | Se cumple el cierre de la ficha y se conserva evidencia |

Actualizar estado al empezar trabajo real y antes de entregar, interrumpir o
pasar de ficha. Un test fallido que se está corrigiendo sigue en curso; una
pausa de sesión no implica bloqueo. Registrar implementación parcial y qué
validación falta en el resumen, sin marcar `done` por un build aislado.

Si el resultado de una evaluación es no-go, registrar ese veredicto incluso
cuando su entrega sea un informe terminado. Verificar el cierre de esa ficha:
si exige go, sigue sin completarse. Sus consumidores con requisito go quedan
inhabilitados en ambos casos. Nunca traducir «informe creado» a «modelo apto».

## Persistencia y sincronización

En cada cambio de estado de una Txx, actualizar en la misma entrega:

1. `tasks.json`: el `status` de la entrada y, cuando haya seguimiento, un objeto
   opcional `tracking` con `updated_at`, `base`, `summary`, `evidence`,
   `blocker`, `next_step` y `verdict` si procede. Conservar campos desconocidos.
   Fecha real ISO 8601; base = commit y hash del diff si hay cambios sin commit.
   `summary` y `evidence` son rutas desde la raíz; `blocker` puede ser null.
   `verdict`: `go`, `no_go` o `not_evaluated`; omitir si no es una evaluación.
2. Ficha: encabezado de estado y casillas que estén acreditadas. Añadir fecha,
   enlace al resumen y siguiente paso cuando quede trabajo.
3. README del subplan: misma etiqueta en su fila. Revisar «Por dónde empezar»
   cuando cambie la tarea recomendada; no conservar «empezar por T01» al haberla
   cerrado. El catálogo es el registro estructurado y las tablas su vista.
4. `docs/self-improvement/seguimiento/Txx.md`: crear al empezar esa ficha y
   mantener un resumen durable con fecha/base, trabajo hecho, pruebas y
   resultados, limitaciones, bloqueo y próximo paso ejecutable. Añadir entradas
   por intento para preservar historia; no crear informes vacíos para todas.

Los logs completos siguen en `target/self-improvement/tasks/Txx/` en host
o `/var/self-improvement/tasks/Txx/` en soso (raíz configurable); para varios
intentos usar subdirectorios identificados y conservar `resultado.md` como
entrada al último resultado. El resumen versionable debe retener comandos,
exit codes, hashes y conclusiones aunque se limpie `target/`. Un log desaparecido
no autoriza a afirmar que se ha revalidado el cambio: reponerlo si el cierre lo
requiere. No inventar artefactos para rellenar `tracking.evidence`.

Modificar `status` del catálogo completo y estados SI-* del plan padre solo
cuando cambie realmente su situación. Cerrar el padre exige todas sus pruebas
de integración y condiciones, incluidas fichas N-xxx/C-xxx; no basta contar
filas done. Crear subplanes no cierra ningún hito de implementación.

Si se añade una N-xxx/C-xxx, seguir el backlog nativo indicado por su ficha
origen: registrar archivo, dependencias, entradas y evidencia allí, y enlazar
desde el consumidor. No generar IDs reutilizados ni insertar ciclos. Para
planes sin JSON, mantener estos datos en su sección de estado existente,
con fecha y enlaces; no imponer otro catálogo a todo el repositorio.

## Verificación y entrega al siguiente agente

- Comparar los tres estados (catálogo, ficha, fila) de cada tarea tocada.
  Validar JSON, IDs únicos, destinos de enlaces y dependencias sin ciclos,
  incluidas fichas derivadas. Si había discrepancias, resolverlas contra la
  evidencia o anotarlas; no copiar el estado más optimista automáticamente.
- Anotar entorno: host, QEMU o placa; modelo real o fixture; perfil, build y
  hardware. Un hostcheck no certifica GPU/WiFi físicos. Aplicar los tests
  exigidos por la ficha; no ejecutar QEMU para un cambio solo de seguimiento.
- Actualizar la skill de dominio si cambia el conocimiento operativo,
  `soso-dev` si cambian comandos/pruebas y el manual solo para UX implementada.
- Resumir al entregar: plan/ID, estado, evidencia clave, bloqueo si existe y
  siguiente ficha o paso habilitado. Leer este resumen al reanudar; no
  depender exclusivamente de la conversación o del contexto del IDE.

## Dónde editar las skills

Fuente del proyecto: `.claude/skills/`. Comprobar `ls -ld .agents/skills
.cursor/skills`: en este checkout ambos son enlaces a `../.claude/skills`.
Editar la fuente una sola vez; no copiar sobre el destino ni sustituir los
symlinks. Si en otro checkout son copias reales, comparar antes de sincronizar
solo los archivos cambiados, preservando modificaciones independientes.
