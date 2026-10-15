# R27 — PMK desde una passphrase validada

## Qué hay hoy
`soso_wpa2::crypto::pbkdf2_psk(passphrase, ssid)` deriva la PMK con PBKDF2 sea cual sea la entrada:
acepta passphrases de 3 caracteres o de 200, con bytes de control, y SSID vacíos o de 60 bytes.
El resultado es una PMK que ningún AP real habría derivado, y el fallo sólo se nota después, como
un 4-way handshake que no termina. Tampoco existe forma de dar la PMK ya calculada, que el estándar
permite como 64 dígitos hexadecimales.

## Qué se pide
Añadir en `crypto.rs`, junto a `pbkdf2_psk`, una función pública
`psk_from_passphrase(passphrase: &str, ssid: &str) -> Result<[u8; 32], PskError>` y el tipo público
`PskError` (`Debug`, `PartialEq`, `Eq`, `Clone`, `Copy`) con tres variantes:
`SsidLength`, `PassphraseLength` y `PassphraseChars`.

Reglas, comprobadas en este orden (la primera que falla decide el error):

1. El SSID debe medir entre 1 y 32 **bytes** (no caracteres): si no, `SsidLength`.
2. Si la passphrase mide exactamente 64 bytes y todos son dígitos hexadecimales (`0-9`, `a-f`,
   `A-F`), es la PMK en claro: se devuelve decodificada (32 bytes) sin ejecutar PBKDF2.
3. Si no, debe medir entre 8 y 63 **bytes**: si no, `PassphraseLength`. Una cadena de 64 bytes
   que no es toda hexadecimal cae aquí.
4. Todos sus bytes deben estar entre 32 (espacio) y 126 (`~`) inclusive: si no, `PassphraseChars`.
5. Superadas las comprobaciones, el resultado es exactamente `pbkdf2_psk(passphrase, ssid)`.

`pbkdf2_psk` no cambia.

## Alcance
Solo `crates/soso-wpa2/src/crypto.rs`.

## Comprobación
`cargo test -p soso-wpa2`, antes y después de tu cambio.
