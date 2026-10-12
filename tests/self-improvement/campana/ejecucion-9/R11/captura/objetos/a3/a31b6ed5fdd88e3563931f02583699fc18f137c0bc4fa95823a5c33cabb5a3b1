//! El 4-way contra una transcripción real de hostapd + wpa_supplicant.
//!
//! El banco interno (`src/tests.rs`) contrasta el supplicant consigo mismo: una
//! segunda implementación de la PRF escrita desde la norma y un AP simulado que
//! deriva la PTK por ese otro camino. Eso demuestra que las dos mitades están
//! de acuerdo, no que el acuerdo sea el correcto.
//!
//! Aquí la referencia es ajena. Alimentamos el M1 y el M3 que emitió hostapd y
//! exigimos que **nuestro M2 y nuestro M4 salgan byte a byte iguales** a los que
//! emitió wpa_supplicant, MIC incluido. Si eso se cumple, PBKDF2, la PRF, la
//! derivación de la PTK, la codificación de `key_info`, las longitudes y el
//! HMAC son correctos frente a dos implementaciones ampliamente desplegadas.
//!
//! El fixture lo genera `scripts/l6-wifi-capture-4way.sh` (necesita root, carga
//! `mac80211_hwsim`). **Mientras no exista, este test se salta**: no hay forma
//! honesta de fingir una transcripción real, así que lo dice en voz alta en vez
//! de dar un verde que no ha ganado.

use soso_wpa2::{Supplicant, eapol};

const FIXTURE: &str = "tests/fixtures/4way-hostapd.txt";

/// Ruta alternativa, para apuntar a otra captura. **Una transcripción fabricada
/// aquí no vale para nada**: el valor de este test está en que los bytes vengan
/// de hostapd y wpa_supplicant, no de nosotros.
fn ruta() -> String {
    std::env::var("SOSO_4WAY_FIXTURE").unwrap_or_else(|_| FIXTURE.to_string())
}

struct Transcripcion {
    ssid: String,
    passphrase: String,
    sta: [u8; 6],
    ap: [u8; 6],
    m: [Vec<u8>; 4],
}

fn mac(s: &str) -> [u8; 6] {
    let mut out = [0u8; 6];
    for (i, p) in s.split(':').enumerate().take(6) {
        out[i] = u8::from_str_radix(p, 16).expect("MAC en hexadecimal");
    }
    out
}

fn hex(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    assert!(b.len() % 2 == 0, "hex de longitud impar");
    b.chunks(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).expect("hex"))
        .collect()
}

fn cargar() -> Option<Transcripcion> {
    let texto = std::fs::read_to_string(ruta()).ok()?;
    let mut ssid = None;
    let mut passphrase = None;
    let mut sta = None;
    let mut ap = None;
    let mut m: [Option<Vec<u8>>; 4] = [None, None, None, None];
    for linea in texto.lines() {
        let linea = linea.trim();
        if linea.is_empty() || linea.starts_with('#') {
            continue;
        }
        let (k, v) = linea.split_once('=').expect("clave=valor");
        match k {
            "ssid" => ssid = Some(v.to_string()),
            "passphrase" => passphrase = Some(v.to_string()),
            "sta" => sta = Some(mac(v)),
            "ap" => ap = Some(mac(v)),
            "m1" | "m2" | "m3" | "m4" => {
                let i = k[1..].parse::<usize>().unwrap() - 1;
                m[i] = Some(hex(v));
            }
            otro => panic!("clave desconocida en el fixture: {otro}"),
        }
    }
    Some(Transcripcion {
        ssid: ssid.expect("ssid"),
        passphrase: passphrase.expect("passphrase"),
        sta: sta.expect("sta"),
        ap: ap.expect("ap"),
        m: m.map(|x| x.expect("m1..m4 completos")),
    })
}

/// Compara dos tramas señalando el primer byte que difiere, con su significado.
fn igual(nuestro: &[u8], suyo: &[u8], cual: &str) {
    if nuestro == suyo {
        return;
    }
    let n = nuestro.len().min(suyo.len());
    let pos = (0..n).find(|&i| nuestro[i] != suyo[i]);
    let campo = pos.map(|p| campo_eapol(p)).unwrap_or("longitud");
    panic!(
        "{cual} difiere del de wpa_supplicant en {campo}\n  nuestro ({} B): {}\n  suyo    ({} B): {}",
        nuestro.len(),
        hexdump(nuestro),
        suyo.len(),
        hexdump(suyo)
    );
}

/// Nombre del campo en el que cae un offset de la trama Ethernet + EAPOL.
fn campo_eapol(off: usize) -> &'static str {
    if off < 14 {
        return "la cabecera Ethernet";
    }
    match off - 14 {
        0 => "la versión 802.1X",
        1 => "el tipo de paquete",
        2..=3 => "la longitud del cuerpo",
        4 => "el tipo de descriptor",
        5..=6 => "Key Information",
        7..=8 => "Key Length",
        9..=16 => "el Replay Counter",
        17..=48 => "la Key Nonce",
        49..=64 => "el EAPOL Key IV",
        65..=72 => "la Key RSC",
        73..=80 => "el campo reservado",
        81..=96 => "el MIC",
        97..=98 => "la longitud de Key Data",
        _ => "Key Data",
    }
}

fn hexdump(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn transcripcion_real() {
    let Some(t) = cargar() else {
        eprintln!(
            "SALTADO: no hay transcripción real en {}.\n\
             Genérala con `sudo ./scripts/l6-wifi-capture-4way.sh` (carga\n\
             mac80211_hwsim, levanta hostapd y wpa_supplicant, captura el 4-way\n\
             y descarga el módulo). Hasta entonces el supplicant sólo está\n\
             contrastado contra su propia implementación de referencia.",
            ruta()
        );
        return;
    };

    let pmk = soso_wpa2::pbkdf2_psk(&t.passphrase, &t.ssid);
    let [m1, m2, m3, m4] = &t.m;

    // La SNonce y el RSN IE los eligió wpa_supplicant: para reproducir su M2
    // byte a byte hay que partir de los suyos. Lo que se comprueba es todo lo
    // demás — y el MIC cubre ambos, así que no se cuelan sin verificar.
    let m2k = eapol::EapolKey::parse(&m2[14..]).expect("M2 del fixture");
    let snonce = m2k.nonce();
    let rsn_ie = m2k.key_data().to_vec();
    assert!(
        rsn_ie.first() == Some(&0x30),
        "el Key Data de M2 debería ser el RSN IE, empieza por {:02x?}",
        rsn_ie.first()
    );

    let mut sup = Supplicant::new(pmk, t.sta, t.ap, &rsn_ie, snonce);

    let r1 = sup.on_ethernet(m1);
    assert_eq!(r1.dropped, None, "M1 real descartado");
    assert!(r1.send, "M1 real no produjo M2");
    igual(sup.tx(), m2, "nuestro M2");

    let r3 = sup.on_ethernet(m3);
    assert_eq!(r3.dropped, None, "M3 real descartado");
    assert!(r3.send, "M3 real no produjo M4");
    igual(sup.tx(), m4, "nuestro M4");

    // Y las claves que salen del M3 real.
    let tk = r3.install_tk.expect("TK tras el M3 real");
    let ptk = sup.ptk().expect("PTK").0;
    assert_eq!(&tk[..], &ptk[32..48], "CCMP usa la TK, no la KCK");
    let gtk = r3.install_gtk.expect("GTK en el M3 real");
    assert_ne!(gtk.key, [0u8; 16], "GTK a cero");
    assert!(gtk.key_id == 1 || gtk.key_id == 2, "Key ID de la GTK fuera de 1..2");
    assert!(sup.authorized());
}
