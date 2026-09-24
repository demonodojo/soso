# T63 — Una generación rota encalla la referencia para siempre

**Hito:** SI-4 · **Tipo:** Corrección (`soso-improve-core`) · **Estado:** completada (2026-09-24).

**Dependencias:** ninguna. **La origina:** [T23](T23-estado-coordinador.md).
**Bloquea:** [T27](T27-reanudacion.md) — reanudar tras una caída es exactamente
el caso que hoy no se puede escribir.

## Problema

El contrato durable de [T46](T46-archivos-durables.md) publica por generaciones
y comprueba la integridad al leer, así que un corte a media escritura **no**
pierde el estado anterior: `leer_vigente` salta la generación rota y devuelve la
anterior. Eso funciona.

Lo que no funciona es lo siguiente que pasa: **escribir otra vez**.

`publicar` numera la siguiente generación a partir de la **vigente íntegra**:

```rust
let siguiente = vigente.as_ref().map(|(g, _)| g + 1).unwrap_or(0);
```

Si la generación 1 quedó a medias, la vigente sigue siendo la 0, así que
`siguiente` vuelve a ser 1 — y el fichero de la 1 **sigue en el disco**.
`crear_exclusivo` lo ve y falla. Y fallará igual la próxima vez, y la
siguiente: la referencia queda encallada en el último estado bueno, sin forma
de avanzar, hasta que alguien borre el fichero roto a mano.

El síntoma no se parece a la causa: el coordinador lee su estado sin problema,
cree que todo está bien, y cada intento de guardar un paso muere con
«`…runs/run-r-1.0000000000000001 ya existe`». Parece una colisión de
identificadores; es un resto de un apagón de hace tres arranques.

## Reproducción

Estaba en `crates/soso-improve-core/tests/state.rs`
(`una_escritura_cortada_no_pierde_el_estado_anterior`), fijando a propósito la
conducta de entonces: se corta la escritura a los 20 bytes, el estado anterior
se recupera entero, y el `guardar` siguiente —ya sin corte— falla con «ya
existe». Esa aserción está **invertida** desde el arreglo: ahora exige que la
escritura pase y que el estado avance.

## Lo que también apareció

La prueba de T46 `se_puede_reintentar_tras_un_corte` **ya conocía el caso** y lo
daba por bueno: borraba el fichero roto a mano —«hay que quitar la basura
antes»— y seguía. En una máquina real no hay nadie para borrarlo. Una prueba que
documenta el defecto como si fuera el contrato es peor que no tenerla, porque
cierra la pregunta. Está reescrita: publica tras el corte **sin limpiar nada**.

## Contrato técnico

Después de un corte, la referencia tiene que poder seguir avanzando:

| Situación | Hoy | Debe |
|---|---|---|
| Leer con la última generación rota | devuelve la anterior íntegra | igual |
| Escribir con una generación rota presente | falla siempre | publica la siguiente libre |
| Precondición de hash | se compara contra la vigente íntegra | igual |

La precondición **no cambia**: se sigue comparando contra el contenido vigente,
que es lo que evita que dos escritores se pisen. Lo único que cambia es de dónde
sale el número de la generación nueva.

## Alcance

`crates/soso-improve-core/src/durable.rs` (`publicar`, y `purgar` si hace falta
revisar qué borra). Nada más: el formato del fichero y la comprobación de
integridad se quedan como están.

## Resultado

`publicar` numera desde la generación **presente** más alta, íntegra o no
(`gens.last().map(|g| g + 1)`), y no desde la vigente. La precondición de hash
no cambia: se sigue comparando contra el contenido vigente, que es lo que impide
que dos escritores se pisen. Se extrajo `vigente_entre` para que `publicar` mire
las dos cosas —cuál es la vigente y cuál la última presente— listando el
directorio **una sola vez**, sin arriesgarse a que cambie entre las dos.

**`purgar` se lleva también las rotas** (paso 3), y queda escrito por qué: una
generación a medias es la única huella de que hubo un corte, pero conservarla
para siempre hace crecer el directorio un fichero por apagón. Cuando `purgar`
corre, la nueva ya está publicada y el corte no explica nada que no esté en el
estado vigente.

Pruebas nuevas en `durable.rs`: `se_puede_reintentar_tras_un_corte` reescrita,
`dos_cortes_seguidos_no_encallan` —con un solo corte el acierto se podía
confundir con una casualidad de numeración— y `purgar_se_lleva_tambien_las_rotas`.

## Comprobaciones ejecutadas

    cargo test -p soso-improve-core --features std        79 + 14, 0 fallos
    cargo build -p soso-improve-core --no-default-features  ok (C1)
    cargo xtask test sys --only="runner de pruebas"       TODO OK
    cargo xtask test                                       TODO OK
    cargo xtask check                                      TODO OK

## Ejecución nativa

Aplicar [NATIVO.md](NATIVO.md). El caso es de soso, no del host: el patrón de
generaciones existe porque en sosofs `rename` no sustituye al destino, y los
cortes que esto arregla son los de una máquina que se apaga.

Sonda nueva `durable/publicar-sobre-un-corte` en `soso-improve pruebas`, que
corre en el shard `sys` de `cargo xtask test`: siembra el resto de un corte en
sosofs, publica por encima, comprueba que la vigente avanza y que `purgar` se
lleva el resto. No necesita reiniciar, porque lo que acredita no es que los
datos sobrevivan al apagón —eso es T46 y ya está— sino que encontrarse un
fichero a medias no encalla la referencia. Además, `durable_fase2` publica ahora
**después** del corte que dejó el arranque anterior; ahí el corte es de verdad.

Validación nativa: **verificada** para la sonda de un arranque. **Límite:** la
fase de dos arranques de T46 sigue lanzándose a mano, así que el caso con
reinicio real sólo se acredita cuando alguien repita los dos comandos.

## Cierre y condición de bloqueo

- [x] Implementación terminada.
- [x] Comprobaciones ejecutadas y evidencia guardada.
- [x] Resultado entregado con límites y dependencias restantes explícitos.

**Desbloquea** [T27](T27-reanudacion.md): reanudar tras una caída era
literalmente imposible mientras la caída encallara la referencia.
