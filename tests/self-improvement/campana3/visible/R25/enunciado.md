# R25 — `KernelMeta`: los hashes en mayúsculas no verifican

## Qué hay hoy
`KernelMeta::verify_new_kernel` y `KernelMeta::verify_backup` comparan el hash guardado en los
metadatos (`new_hash` / `backup_hash`) con el SHA-256 calculado **como texto, carácter a carácter**.
El hash calculado sale en minúsculas, así que un meta cuyo hash es correcto pero está escrito en
mayúsculas (o mezclando ambas) se rechaza como si el kernel estuviera corrupto. Otras rutas del
crate, como `verify_staged_kernel`, sí aceptan ese mismo hash.

## Qué se pide
Ambos métodos deben comparar el hash **sin distinguir mayúsculas de minúsculas**: si el contenido
coincide con el digest, el hash en cualquier combinación de mayúsculas y minúsculas verifica.

El resto del contrato no cambia:
- Se siguen comprobando primero los tamaños, con los mismos errores de hoy.
- Un hash que no sea de 64 dígitos hexadecimales (longitud distinta, caracteres no hex, espacios)
  sigue dando `MetaError::BadHash` en ambos métodos.
- Un hash bien formado pero que no corresponde al contenido da `MetaError::BadHash` en
  `verify_new_kernel` y `MetaError::BackupHashMismatch` en `verify_backup`.
- `verify_backup` sigue devolviendo exactamente los `backup_size` primeros bytes del hueco.

## Alcance
Solo `crates/soso-update-core/src/kernel_meta.rs`.

## Comprobación
`cargo test -p soso-update-core --features std`, antes y después de tu cambio.
