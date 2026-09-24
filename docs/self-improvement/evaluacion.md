# Evaluación del modelo: cómo se decide go/no-go

**Ficha:** [T14](T14-evaluacion-modelo.md) · **Módulo:**
`crates/soso-improve-core/src/evaluacion.rs` · **Revisión:** 23 de septiembre de 2026.

Este documento dice **qué se mide, cómo se cuenta y qué descalifica**. Está
separado de la ficha porque el criterio tiene que poder citarse sin releer un
plan entero, y porque una campaña que cambia su propio criterio a mitad no vale
nada.

## Las tres reglas que hacen que el número signifique algo

1. **Todos los intentos cuentan en el denominador.** Cada caso se ejecuta
   `repeticiones` veces (tres por defecto) y un caso solo está **aprobado** si
   salió bien **todas** las veces. Un caso que acierta dos de tres se cuenta
   aparte, como *inestable*, y no suma al umbral. Quedarse con el mejor intento
   es la forma más fácil de fabricar un go que luego no se sostiene.
2. **El verificador no ve lo que ve el modelo.** Las aserciones viven en la
   partición **reservada** del banco ([T02](T02-banco.md)) y se aplican fuera
   del contexto del modelo. No hay manera de acertar copiando el criterio.
3. **Lo que no se puede medir no se apunta como medido.** Si un caso espera una
   generación y el endpoint contesta bien pero no devuelve `usage`, el intento
   se marca `sin-uso` y **no** cuenta como éxito. No se estima el consumo: se
   acredita o no se acredita.

   Quién lo exige lo decide el **caso**, no el evaluador. La primera campaña
   real lo enseñó por las malas: un caso de error (400) y uno de streaming
   pasaban todas sus aserciones y se marcaban `sin-uso` porque el evaluador
   pedía un dato que esos casos nunca piden. Dos fallos que eran del evaluador,
   no del modelo.

## Desenlaces

Cada intento acaba en uno de cinco estados, y son cinco a propósito: agruparlos
en «falló» esconde justo lo que hace falta para corregir.

| Desenlace | Qué pasó | ¿Cuenta como éxito? |
|---|---|---|
| `bien` | El comprobador reservado lo da por bueno y hay `usage` | **Sí** |
| `mal` | Contestó y el comprobador dice que no | No |
| `plazo` | No contestó dentro del plazo | No |
| `sin-uso` | Contestó bien, pero el endpoint no dijo lo que costó | No |
| `error` | El endpoint o el transporte fallaron | No |

`mal` y `error` no son lo mismo: en el primero el endpoint funcionó y el modelo
se equivocó; en el segundo no hubo respuesta que juzgar. Confundirlos hace que
un problema de red parezca un problema de calidad.

## Umbrales

Del plan padre, y explícitos en el informe para que no se puedan bajar sin que
se vea:

- **protocolo: 10 de 10** casos aprobados.
- **microtareas de programación: al menos 8 de 10.**
- **3 repeticiones** por caso, con la semilla de cada intento registrada.

Una clase que el umbral exige y que **no se evaluó** es motivo de no-go, no un
aprobado por omisión.

## Frío y caliente

El primer intento de cada caso se marca `frio`: el servidor puede no tener nada
cacheado. Para la **corrección** cuenta igual que los demás; para la
**velocidad** se informa aparte, porque mezclar la primera carga con las
siguientes convierte la mediana en un número que no describe ninguna de las dos
situaciones.

## Presupuestos

Salen de los intentos que fueron **bien**: el coste de un fallo no es el coste
de una tarea, y meterlo baja la mediana justo cuando el modelo va peor.

| Campo | Qué es |
|---|---|
| `ms_primer_token_mediana` | Latencia hasta el primer token útil |
| `ms_total_mediana` | Lo que dura una tarea típica |
| `ms_total_maximo` | El peor caso observado |
| `timeout_sugerido_ms` | El doble del peor caso, con suelo de 1 s |
| `tokens_salida_maximo` | Techo observado de salida |

El `timeout_sugerido_ms` sale de la medida, no de un número redondo elegido a
ojo: la holgura es explícita (×2) y se puede discutir viendo el dato.

El plazo por petición **no puede ser infinito**: una campaña que no termina no
se puede comparar con otra. Pedirlo es un error de uso, no un aviso.

## Qué sigue a un no-go

Un informe no-go **no** habilita a los consumidores ([T16](T16-servicio-guest.md),
[T22](T22-primera-mejora.md)). El informe lista los motivos concretos —qué
clase, cuántos aprobados, cuántos inestables— para que la corrección siguiente
sea una acción y no una intuición. Si se prueba otro perfil de modelo, el banco
**no se toca** durante la comparación: cambiar la vara mientras se mide
invalida las dos medidas.

## Estado

La lógica y sus pruebas con backend simulado están hechas y verdes. La
**campaña real** contra el endpoint guest es evidencia aparte y es lo que
decide el go/no-go; hasta que exista, T14 no está cerrada y SI-2 no puede
cerrarse.
