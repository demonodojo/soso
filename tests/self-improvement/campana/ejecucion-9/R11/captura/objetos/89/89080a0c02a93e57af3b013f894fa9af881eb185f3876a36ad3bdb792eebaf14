# T21 — Capturar el contrato real de OpenCode sin depender del modelo

**Hito:** SI-3 · **Tipo:** Implementación de integración host · **Estado:** pendiente.  
**Dependencias:** [T20](T20-opencode-config.md)

## Objetivo y entrega

Bucle OpenCode read→edit→test verificado y contrato HTTP compatible con fixtures versionados.

## Contexto mínimo

Leer [CONTRATO.md](CONTRATO.md), secciones **C3, C5–C6**, y los símbolos
pertinentes de estos archivos. Los módulos nuevos mencionados en los pasos
se obtienen de las dependencias; todavía no existen en la base del plan.

- [SELF_IMPROVEMENT.md](../../SELF_IMPROVEMENT.md)

## Archivos que se pueden cambiar

Crear `scripts/self-improvement/opencode_contract.py`, `tests/self-improvement/test_opencode_contract.py` y fixtures wire en `tests/self-improvement/opencode/`.

Se permiten los ajustes de lockfiles y documentación exigidos por C6.
Si hace falta cambiar lógica fuera de este alcance, registrar una ficha nueva
con la reproducción; no ampliar esta tarea de forma silenciosa.

## Pasos

1. Lanzar un servidor local de prueba y OpenCode de la revisión fijada en un repo temporal. Reutilizar el proveedor T20 cambiando solo endpoint y credencial de prueba.
2. Guionizar respuestas: pedir lectura, comprobar que llega resultado tool, pedir edición inocua y una prueba, responder fin. Probar tool_call completo y argumentos SSE divididos.
3. Capturar exactamente campos enviados, esquemas reales, partes content y nuevas peticiones auxiliares. Eliminar secretos; guardar versión y hash junto a los fixtures.
4. Reproducir las peticiones capturadas contra decoder/validator T05/T10. Si aparecen esquemas o campos no soportados, crear corrección con fixture y retomarla antes de cerrar.
5. Comprobar que OpenCode ejecutó herramientas observando el archivo y la salida reales. Un proceso con exit 0 o un texto que diga «he editado» no bastan.

## Comprobación

`python3 -m unittest discover -s tests/self-improvement -p test_opencode_contract.py`; ejecución real del script con OpenCode y servidor falso. Probar respuesta malformada, error HTTP y herramienta fallida.

Los comandos de crates, scripts o subcomandos nuevos se ejecutan **después de
crearlos en esta tarea o en sus dependencias**. Guardar salida y exit code;
los pesos reales/hardware necesarios son entradas, no fixtures inventados.

## Cierre y condición de bloqueo

- [ ] Implementación o artefactos de esta ficha terminados.
- [ ] Comprobaciones ejecutadas y evidencia guardada según C5–C6.
- [ ] Resultado entregado con límites y dependencias restantes explícitos.

No invocar modelos externos para completar este banco; una incompatibilidad debe quedar como fallo reproducible del adaptador.

Entregar `target/self-improvement/tasks/T21/resultado.md` y actualizar la
fila de [README.md](README.md) al cerrar. No avanzar automáticamente al resto
del hito en la misma sesión.

