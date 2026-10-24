# R43 — EAPOL-Key y KDE de GTK: rechazar tramas manipuladas o mal formadas

## Qué hay hoy
`soso-wpa2` procesa las tramas EAPOL-Key del AP con `EapolKey::parse` (`src/eapol.rs`), comprueba su MIC
con `eapol::verify_mic` y extrae la clave de grupo con `kde::find_gtk` (`src/kde.rs`). Son entradas que
controla un atacante en el aire y hoy se aceptan cosas que no deberían:

- `parse` no mira la versión 802.1X (byte 0 de la trama): una trama con versión 0 o 9 pasa.
- `parse` admite un cuerpo (bytes 2..4) más largo que `95 + Key Data Length`; esos bytes sobrantes
  quedan dentro de `bytes()` y entran en el cálculo del MIC.
- `verify_mic` acepta cualquier longitud de KCK, aunque la KCK de WPA2 mide 16 bytes.
- `find_gtk` acepta un KDE de GTK con una carga mayor de 16 bytes de clave (se queda con los 16
  primeros), con Key ID 0 (que no es un índice de GTK válido), y si el primer KDE de GTK está truncado
  sigue buscando y puede aceptar un segundo KDE que venga detrás.

## Qué se pide
Endurecer sin cambiar ninguna firma existente:

1. `EapolKey::parse`: si el byte 0 no es 1, 2 ni 3, devuelve el nuevo `Err(ParseError::BadEapolVersion)`
   (variante nueva en `ParseError`). El orden de las comprobaciones es: longitud mínima (`TooShort`),
   versión 802.1X (`BadEapolVersion`), tipo de paquete (`NotKeyFrame`), tipo de descriptor (`NotRsn`),
   longitudes (`BadLength`).
2. `EapolKey::parse`: el cuerpo declarado debe medir **exactamente** `EAPOL_BODY_LEN + Key Data Length`;
   cualquier otro valor (menor o mayor) es `Err(ParseError::BadLength)`. Los bytes de la trama recibida
   que queden más allá de `4 + cuerpo` (relleno de Ethernet) siguen ignorándose, y `bytes()` sigue
   devolviendo sólo `4 + cuerpo` bytes.
3. `eapol::verify_mic`: si `kck.len() != 16` devuelve `false`, sin pánico.
4. `kde::find_gtk`: el KDE de GTK debe tener una carga de **exactamente** 18 bytes (1 de Key ID/Tx, 1
   reservado, 16 de clave); con otra longitud, el KDE está mal formado.
5. `kde::find_gtk`: el Key ID (los 2 bits bajos del primer byte de la carga) debe ser 1, 2 o 3; con 0
   el KDE está mal formado.
6. `kde::find_gtk`: sólo cuenta el **primer** KDE de GTK que aparezca (data type 1 con OUI `00-0f-ac`).
   Si está bien formado, se devuelve (el resto se ignora, como hoy); si está mal formado, el resultado es
   `None` aunque más adelante haya otro KDE de GTK correcto. Otros KDE y elementos de otros tipos
   delante de él siguen saltándose como hasta ahora.

El resto del comportamiento (`Gtk { key, key_id, tx }`, el recorrido de `for_each_rsn_kde`, todas las
demás variantes de `ParseError`) no cambia.

## Alcance
Sólo `crates/soso-wpa2/src/eapol.rs` y `crates/soso-wpa2/src/kde.rs`. No cambies `lib.rs` ni las
pruebas existentes.

## Comprobación
`cargo test -p soso-wpa2`, antes y después de tu cambio; las pruebas existentes deben seguir pasando.
