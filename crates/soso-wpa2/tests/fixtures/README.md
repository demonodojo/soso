# Transcripciones reales del 4-way

Aquí va `4way-hostapd.txt`: un 4-way WPA2-PSK/CCMP capturado entre **hostapd** y
**wpa_supplicant**, que es lo que convierte `transcripcion_real.rs` de un test
saltado en una comparación byte a byte contra implementaciones ajenas.

No está en el repositorio porque generarlo necesita root (carga
`mac80211_hwsim`). Para tenerlo:

```sh
sudo ./scripts/l6-wifi-capture-4way.sh
cargo test -p soso-wpa2
```

**No lo fabriques a mano.** Una transcripción escrita por nosotros no acredita
nada que el banco interno (`src/tests.rs`) no acredite ya: el valor está
exactamente en que los bytes vengan de otro.
