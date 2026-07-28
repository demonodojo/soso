# vendor/ — dependencias de crates.io con parches propios

Copias literales de crates publicados, con los cambios mínimos que soso
necesita y que no existen aguas arriba. Se enganchan con `[patch.crates-io]`
en `kernel/Cargo.toml`.

Regla: cada parche va comentado **en el sitio del código** con el prefijo
`PARCHE soso` y explicado aquí. Si el crate publica una versión que lo
incorpore, se borra la copia y se vuelve a crates.io.

## sunset-0.5.0

Copia de `sunset 0.5.0` (última publicada a 2026-07-28; no hay 0.6).

**Un solo cambio**, en `src/channel.rs::handle_eof`: no se manda un
`ChannelEof` de vuelta al recibir el del par.

El original hacía:

```rust
if !self.sent_eof {
    s.send(packets::ChannelEof { num: self.send_num()? })?;
    self.sent_eof = true;
}
```

Es decir, anunciaba "no enviaré más datos" sólo porque el par dejó de enviar.
El EOF de SSH es por dirección (RFC 4254 §5.3), así que es incorrecto, y el
propio sunset lo tenía marcado con un `//TODO: check existing state?`.

Cómo se manifestaba en soso (2026-07-28, seis minutos de ciclo VFIO colgado):
`printf 'cmd\n' | ssh soso@localhost` **ejecutaba** el comando —verificado
escribiendo un fichero y leyéndolo en otra sesión— pero no devolvía ni un byte
de su salida. OpenSSH recibe el EOF espejado, hace `output drain -> closed` y
descarta lo que llegue después; el eco de `sosh` desaparece con ella, así que
parecía que el comando no había llegado.

No se toca nada más: el canal sigue pasando a `ChanState::RecvEof`, así que
`read_channel` sigue devolviendo `ChannelEOF` en la dirección de entrada y
`kernel/src/net/ssh.rs` no necesita cambios. `sent_eof` queda sin uso (sólo se
leía en ese bloque).

Los arneses (`xtask/src/test.rs`, `scripts/l6-g1-vfio-test.sh`) mantienen stdin
abierto con un FIFO de todas formas: es más robusto y no depende de este parche.
