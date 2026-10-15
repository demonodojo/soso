# R29 — `TierManager::promote` con presupuesto ajustado

## Qué hay hoy
`TierManager::promote(nombre, datos, destino)` sube un tensor a RAM o VRAM respetando el presupuesto
del nivel (`ram_budget` / `vram_budget`), expulsando los menos usados (LRU) si no cabe. Tiene dos
defectos cuando el presupuesto va justo:

1. **Re-promover un nombre que ya está**: los bytes de la copia anterior se cuentan como ocupados
   mientras se decide si cabe la nueva, así que se expulsan tensores ajenos (o incluso la propia
   copia anterior) sin necesidad, aunque al reemplazarla sí cabría todo.
2. **Datos que no caben nunca** (más grandes que el presupuesto entero del nivel): devuelve `Err`,
   pero antes ha ido expulsando todo lo que había en ese nivel. El fallo deja el gestor vacío.

## Qué se pide
- Si `datos.len()` es mayor que el presupuesto del nivel de destino (o el destino es `Tier::Disk`),
  `promote` devuelve `Err(())` **sin tocar nada**: ni slots, ni `ram_used`/`vram_used`.
- Si `nombre` ya existe (en cualquier nivel), su copia anterior se considera liberada **antes** de
  comprobar si cabe: solo se expulsa a otros tensores cuando, sin la copia anterior, la nueva
  sigue sin caber. La copia anterior nunca se expulsa «por error» ni se contabiliza dos veces.
- Tras un `promote` correcto, `ram_used` y `vram_used` valen exactamente la suma de los bytes de
  los slots de cada nivel (un tensor que pasa de VRAM a RAM deja de contar en VRAM).
- Sin cambios en el resto: el orden de expulsión sigue siendo LRU, `Ok(())` en el caso normal, y
  `get` devuelve los bytes de la última copia.

## Alcance
Solo `crates/soso-llm-core/src/tier.rs`, función `TierManager::promote` (y, si te hace falta, el
auxiliar privado `evict_lru`).

## Comprobación
`cargo test -p soso-llm-core --features std`, antes y después de tu cambio.
