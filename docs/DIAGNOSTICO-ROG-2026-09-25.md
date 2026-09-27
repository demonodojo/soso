# ROG RTX 3080 Laptop: diagnóstico (25 sep 2026)

## Evidencia y alcance

Lectura ESP `/dev/sda1` (`KERNEL`). Copia: `target/usb-diagnostic-2026-09-25/`.

| Campo | Valor |
|---|---|
| Kernel (banner SOSOLOG) | **soso 0.3.7 (eab04fb2e-dirty)** |
| Flush / uptime | **#49**, **4311919 ms** (~72 min) |
| Hardware | `10de:249c` + `8086:2723` + `10ec:8168` (sin cable) |

Arranque live hasta `sosh —`. GSP/VRAM/CE OK; WiFi ALIVE, scan, assoc, DHCP por **lx-wifi**.

## Fallo observado

Segundo `soso-update comprobar --traza` contra `release-assets.githubusercontent.com`:

```
red: read(fd 4, 60000ms) = -11 en 60000ms
soso-update: manifest inválido
```

`-11` = `EAGAIN`: plazo de lectura agotado a mitad del cuerpo HTTPS. El cliente HTTP trataba ese timeout como fin de respuesta; el manifiesto llegaba truncado y el parser fallaba con un mensaje engañoso.

## Corrección (repo)

- [`crates/soso-http/src/lib.rs`](../crates/soso-http/src/lib.rs): el plazo en lectura del cuerpo es error; validación de `Content-Length` en `HttpStreamState::finish`.
- [`user/soso-update/src/net.rs`](../user/soso-update/src/net.rs): mensajes «plazo agotado en la descarga HTTP» / «descarga HTTP incompleta».

Validación host: `cargo test -q -p soso-http`.

## Qué no se ha hecho

- No se reflasheó el USB con el kernel corregido (requiere `--only kernel` en placa).
- No se incrementó `arranques_consecutivos_ok` en la matriz.
