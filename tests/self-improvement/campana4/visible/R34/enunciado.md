# R34 — `Sampler`: logits NaN, infinitos y empates

## Qué hay hoy
`Sampler::sample(logits)` (en `soso-llm-core`, módulo `sample`) elige el siguiente token, en modo
greedy (`temp <= 0`) o con nucleus top-p. No tolera logits "raros", que salen en la práctica de
un modelo desbordado o de un token enmascarado con `-inf`:

- en greedy, el empate entre varios máximos se resuelve a favor del **último** índice, y un NaN
  puede acabar eligiéndose como "máximo";
- con temperatura, un logit NaN contamina la suma de probabilidades y el token devuelto es
  arbitrario; si todos son `-inf` (o hay un `+inf`) la normalización produce NaN y pasa lo mismo.

## Qué se pide
- Greedy: se devuelve el índice del mayor logit; en caso de empate, el **menor** índice. Los NaN
  nunca se eligen mientras haya algún logit que no sea NaN. Si todos son NaN, el resultado es 0.
- Con `temp > 0`: un logit NaN pesa como probabilidad cero (nunca sale). Si el máximo de los
  logits no es finito (todos NaN o `-inf`, o hay algún `+inf`), no hay distribución que muestrear
  y se devuelve exactamente lo que devolvería greedy con esos logits.
- Con logits finitos y `-inf` sueltos, el muestreo sigue como hasta ahora (`-inf` pesa cero).
- Nada más cambia: mismas semillas, mismos resultados con logits normales.

## Alcance
Solo `crates/soso-llm-core/src/sample.rs`, en `Sampler`.

## Comprobación
`cargo test -p soso-llm-core --features std --lib sample`, antes y después; la prueba completa del crate
es `cargo test -q -p soso-llm-core --features std`.
