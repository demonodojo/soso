//! Banco host de `red.conf` y de la lista de redes WiFi.

use soso_red_core::wifi::RedWifi;
use soso_red_core::{
    formatear_red_conf, formatear_redes, parsear_orden, parsear_red_conf, parsear_redes, promover,
    ErrorRed, RedEstatica, MAX_REDES,
};

fn red(ssid: &str, psk: Option<&str>) -> RedWifi {
    RedWifi::new(ssid, psk)
}

// ---- red.conf ----

#[test]
fn sin_modo_es_dhcp() {
    assert_eq!(parsear_red_conf(""), Ok(None));
    assert_eq!(parsear_red_conf("# nada\n\n"), Ok(None));
    assert_eq!(parsear_red_conf("modo=dhcp\n"), Ok(None));
}

#[test]
fn estatica_completa() {
    let t = "modo=estatica\naddr=192.168.1.20/24\ngw=192.168.1.1\ndns=192.168.1.1\n";
    assert_eq!(
        parsear_red_conf(t),
        Ok(Some(RedEstatica {
            addr: [192, 168, 1, 20],
            prefijo: 24,
            gw: Some([192, 168, 1, 1]),
            dns: Some([192, 168, 1, 1]),
        }))
    );
}

#[test]
fn estatica_sin_gw_ni_dns() {
    let r = parsear_red_conf("modo=estatica\naddr=10.0.0.5/8\n").unwrap().unwrap();
    assert_eq!((r.gw, r.dns), (None, None));
}

#[test]
fn estatica_incompleta_es_error() {
    assert_eq!(parsear_red_conf("modo=estatica\n"), Err(ErrorRed::FaltaAddr));
    assert_eq!(
        parsear_red_conf("modo=estatica\ngw=192.168.1.1\n"),
        Err(ErrorRed::FaltaAddr)
    );
}

#[test]
fn valores_invalidos() {
    let base = "modo=estatica\n";
    for (extra, esperado) in [
        ("addr=192.168.1.20\n", ErrorRed::AddrInvalida),
        ("addr=192.168.1.20/0\n", ErrorRed::AddrInvalida),
        ("addr=192.168.1.20/33\n", ErrorRed::AddrInvalida),
        ("addr=192.168.1/24\n", ErrorRed::AddrInvalida),
        ("addr=192.168.1.256/24\n", ErrorRed::AddrInvalida),
        ("addr=1.2.3.4/24\ngw=nada\n", ErrorRed::GwInvalida),
        ("addr=1.2.3.4/24\ndns=1.2.3\n", ErrorRed::DnsInvalido),
        ("addr=192.168.1.20/24\ngw=10.0.0.1\n", ErrorRed::GwFueraDeRed),
    ] {
        assert_eq!(parsear_red_conf(&format!("{base}{extra}")), Err(esperado), "{extra}");
    }
    assert_eq!(parsear_red_conf("modo=raro\n"), Err(ErrorRed::ModoDesconocido));
}

#[test]
fn dhcp_ignora_valores_viejos() {
    assert_eq!(parsear_red_conf("modo=dhcp\naddr=basura\n"), Ok(None));
}

#[test]
fn red_conf_ida_y_vuelta() {
    let r = RedEstatica {
        addr: [192, 168, 68, 40],
        prefijo: 22,
        gw: Some([192, 168, 68, 1]),
        dns: None,
    };
    assert_eq!(parsear_red_conf(&formatear_red_conf(&r)), Ok(Some(r)));
}

#[test]
fn orden_ip_estatica() {
    let r = parsear_orden("192.168.1.20/24 gw 192.168.1.1 dns 9.9.9.9").unwrap();
    assert_eq!(r.addr, [192, 168, 1, 20]);
    assert_eq!(r.gw, Some([192, 168, 1, 1]));
    assert_eq!(r.dns, Some([9, 9, 9, 9]));
    // `gw` y `dns` en cualquier orden.
    assert_eq!(parsear_orden("192.168.1.20/24 dns 9.9.9.9 gw 192.168.1.1"), Ok(r));
}

#[test]
fn orden_rechazada() {
    assert_eq!(parsear_orden(""), Err(ErrorRed::AddrInvalida));
    assert_eq!(parsear_orden("192.168.1.20"), Err(ErrorRed::AddrInvalida));
    assert_eq!(parsear_orden("192.168.1.20/24 gw"), Err(ErrorRed::Orden));
    assert_eq!(parsear_orden("192.168.1.20/24 mtu 1500"), Err(ErrorRed::Orden));
    assert_eq!(parsear_orden("192.168.1.20/24 gw 1.1.1.1 gw 1.1.1.2"), Err(ErrorRed::Orden));
    assert_eq!(parsear_orden("192.168.1.20/24 gw 10.1.1.1"), Err(ErrorRed::GwFueraDeRed));
}

// ---- wifi.conf ----

#[test]
fn una_red_es_el_formato_de_siempre() {
    assert_eq!(
        parsear_redes("ssid=MiRed\npsk=clave1234\n"),
        vec![red("MiRed", Some("clave1234"))]
    );
    assert_eq!(parsear_redes("ssid=Abierta\n"), vec![red("Abierta", None)]);
    // El orden de las dos líneas nunca estuvo fijado.
    assert_eq!(
        parsear_redes("psk=clave1234\nssid=MiRed\n"),
        vec![red("MiRed", Some("clave1234"))]
    );
}

#[test]
fn comentarios_y_vacias_se_ignoran() {
    assert!(parsear_redes("# ssid=MiRed\n\n").is_empty());
    assert!(parsear_redes("").is_empty());
    assert!(parsear_redes("ssid=\npsk=x\n").is_empty());
}

#[test]
fn cuatro_redes_con_su_clave() {
    let t = "ssid=A\npsk=aaaaaaaa\nssid=B\nssid=C\npsk=cccccccc\nssid=D\npsk=dddddddd\n";
    assert_eq!(
        parsear_redes(t),
        vec![
            red("A", Some("aaaaaaaa")),
            red("B", None),
            red("C", Some("cccccccc")),
            red("D", Some("dddddddd")),
        ]
    );
}

#[test]
fn la_quinta_se_ignora_con_su_clave() {
    let t = "ssid=A\nssid=B\nssid=C\nssid=D\nssid=E\npsk=eeeeeeee\n";
    let redes = parsear_redes(t);
    assert_eq!(redes.len(), MAX_REDES);
    assert!(redes.iter().all(|r| r.psk.is_none()));
}

#[test]
fn ssid_repetido_no_duplica_ni_pisa_la_primera() {
    let redes = parsear_redes("ssid=A\npsk=aaaaaaaa\nssid=A\npsk=otra\n");
    assert_eq!(redes, vec![red("A", Some("aaaaaaaa"))]);
}

#[test]
fn lista_ida_y_vuelta() {
    let redes = vec![red("Casa", Some("clavecasa")), red("Cafe", None), red("Oficina", Some("12345678"))];
    assert_eq!(parsear_redes(&formatear_redes(&redes)), redes);
}

#[test]
fn clave_vacia_es_red_abierta() {
    assert_eq!(red("X", Some("")).psk, None);
}

// ---- promover ----

#[test]
fn promover_pone_la_nueva_delante() {
    let lista = vec![red("A", None), red("B", Some("bbbbbbbb"))];
    let nueva = promover(lista, red("C", Some("cccccccc")));
    let ssids: Vec<_> = nueva.iter().map(|r| r.ssid.as_str()).collect();
    assert_eq!(ssids, ["C", "A", "B"]);
}

#[test]
fn promover_sin_duplicar_y_con_clave_nueva() {
    let lista = vec![red("A", Some("vieja123")), red("B", None)];
    let nueva = promover(lista, red("B", Some("nueva123")));
    assert_eq!(nueva, vec![red("B", Some("nueva123")), red("A", Some("vieja123"))]);
}

#[test]
fn promover_con_la_lista_llena_pierde_la_ultima() {
    let lista: Vec<_> = ["A", "B", "C", "D"].iter().map(|s| red(s, None)).collect();
    let nueva = promover(lista, red("E", None));
    let ssids: Vec<_> = nueva.iter().map(|r| r.ssid.as_str()).collect();
    assert_eq!(ssids, ["E", "A", "B", "C"]);
}
