# Plantilla para una ficha derivada N-xxx / C-xxx

Usarla cuando T03, T33, T34 o T40–T42 detecten una dependencia concreta.
Sustituir todos los campos antes de entregar la ficha a un modelo.

## Identidad

- ID y título:
- Hito padre y ficha que la origina:
- Estado: pendiente.
- Dependencias exactas y evidencia requerida:
- Revisión de soso y del código externo:
- Tipo: implementación / experimento / integración.

## Problema reproducido

Síntoma, comando mínimo, salida real y comportamiento esperado.
Enlazar una sonda o fixture que reproduzca el hueco sin ejecutar todo el port.

## Contexto mínimo

Enumerar archivos y símbolos que hay que leer. Para código externo, indicar
ruta del checkout y commit; no depender de una rama móvil o de la memoria del modelo.

## Contrato técnico

Firma exacta, ownership, ABI, errores, límites y relación con el llamante.
Elegir la solución de esta ficha; si falta evidencia para elegir, convertirla
en experimento con resultados que permitan decidir.

## Alcance

1–3 archivos de lógica y sus pruebas. Enumerar los archivos de integración,
manifiestos y documentación necesarios. Separar capacidades independientes.

## Pasos

De tres a seis pasos concretos que produzcan un solo cambio observable.
Si un paso dice «completar el runtime/compilador», volver a dividirlo.

## Comprobación

Comando, cwd, entradas y resultado esperado. Al menos un caso negativo relevante.
El test debe fallar con la base cuando se trata de un defecto y pasar con el cambio.
Indicar cuándo hacen falta pesos reales, QEMU, build externo o hardware.

## Entrega y cierre

Diff o parche contra la revisión externa, logs, comandos y hashes.
Condición observable de cierre; condición de bloqueo y dependencia que lo resuelve.
Actualizar backlog y consumidores cuando se verifique el cambio.

## Presupuesto de contexto

Entregar esta ficha, secciones necesarias de CONTRATO.md y sus fuentes.
Evitar cargar todo el backlog. Si excede el contexto disponible, dividir por
comportamiento, no omitir el test o el contrato.

