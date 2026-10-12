# R12 — Buzón `KERNEL`: nada de tamaños inventados

## Qué hay hoy
`Mailbox::parse` lee líneas `KERNEL <tamaño> <sha256> <versión>`. Si el tamaño no es un número,
`size.parse().unwrap_or(0)` lo convierte en **0** y devuelve igualmente `MailboxCmd::Kernel`:
un buzón corrupto parece una orden válida de un kernel de 0 bytes. Y el hash se acepta
sea lo que sea.

## Qué se pide
Una línea `KERNEL …` sólo produce `MailboxCmd::Kernel` si:
- el tamaño es un entero sin signo válido, **y**
- el hash tiene exactamente **64 caracteres hexadecimales** (mayúsculas o minúsculas).

Si no se cumple, **la línea se ignora**: el comando queda como estaba antes de leerla (`Idle` si
no había otra orden). Lo demás (`PROBANDO`, `OK`, `REVERTIR`, `REVERTIDO`, la última línea
válida gana) no cambia.

## Alcance
Solo `crates/soso-update-core/src/mailbox.rs`, función `Mailbox::parse`.

## Comprobación
`cargo test -p soso-update-core --features std`, antes y después de tu cambio.
