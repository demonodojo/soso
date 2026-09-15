# Q10 — Contexto por encima del límite

El historial rende­rizado más la reserva de salida se pasan del límite del
modelo. Debe responderse 422 con `code: context_length_exceeded`. Truncar el
historial en silencio no es una respuesta válida: esconde el desbordamiento.

El relleno está en `relleno.txt`; el lanzador lo repite hasta pasarse del
límite declarado por el perfil del modelo.
