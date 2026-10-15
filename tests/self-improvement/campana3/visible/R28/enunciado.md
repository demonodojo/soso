# R28 — `Rot::parse` rechaza formas razonables de escribir una rotación

## Qué hay hoy
`soso_hw::fbrot::Rot::parse(s, phys_w, phys_h)` convierte el valor de la opción de arranque de
rotación de la consola en un `Rot`. Hoy sólo acepta, tras quitar espacios de los extremos, estas
cadenas exactas en minúscula: `0`, `normal`, `90`, `180`, `270` y `auto`. Todo lo demás da `None`,
y el kernel (que hace `unwrap_or(Rot::R0)`) lo interpreta como «sin rotación» sin avisar. Así,
`AUTO`, `Normal`, `90°`, `270deg`, `-90` o `450` dejan el panel de la Deck de lado.

## Qué se pide
Que `Rot::parse` admita, además de lo de hoy:

- `normal` y `auto` con cualquier combinación de mayúsculas y minúsculas (`AUTO`, `Normal`);
- un ángulo en grados como entero decimal con signo opcional (`+90`, `-90`);
- tras el número, un sufijo opcional `°` o `deg` (este último sin distinguir mayúsculas), con
  espacios opcionales entre número y sufijo (`90°`, `270 deg`, `180DEG`);
- ángulos fuera de 0..360, que se reducen módulo 360 con resultado siempre no negativo: `360` es
  `R0`, `450` es `R90`, `-90` es `R270`, `-180` es `R180`, `-270` es `R90`;
- el signo menos significa giro horario, coherente con que los grados «normales» son antihorarios.

Sigue devolviendo `None` para cualquier ángulo que, ya reducido, no sea 0, 90, 180 o 270 (`45`,
`91`, `-45`), para cadenas vacías, para un sufijo sin número (`°`, `deg`), para texto que no sea un
entero (`90.0`, `9 0`, `abc`) y para números que no caben en un `i32`. Nunca debe entrar en pánico,
sea cual sea la entrada (incluidos caracteres multibyte).

## Alcance
Solo `crates/soso-hw/src/fbrot.rs`, función `Rot::parse`. La firma no cambia.

## Comprobación
`cargo test -p soso-hw`, antes y después de tu cambio.
