//! U7, fila «host, HTTP simulado»: qué respuestas de un servidor valen y
//! cuáles no. Aquí no hay red ni TLS; lo que se prueba es la **política**, que
//! es lo que decide si una actualización sigue o se planta.

use soso_update_core::descarga::{juzgar_rango, parse_content_range, FalloRango, Peticion};

fn pidiendo(desde: u64, restante: u64) -> Peticion {
    Peticion {
        desde,
        hasta: desde + restante - 1,
        restante,
        desde_el_principio: desde == 0,
    }
}

#[test]
fn un_206_del_tramo_pedido_se_acepta() {
    let p = pidiendo(100, 50);
    let a = juzgar_rango(&p, 206, Some("bytes 100-149/1000"), 50).expect("vale");
    assert_eq!(a.usar, 50);
    assert!(!a.completo);
}

#[test]
fn un_206_de_otro_tramo_se_rechaza_diciendo_cual() {
    // Sin esto los bytes irían al sitio equivocado y el fallo aparecería
    // después como «fichero corrupto», culpando al fichero y no al servidor.
    let p = pidiendo(100, 50);
    assert_eq!(
        juzgar_rango(&p, 206, Some("bytes 0-49/1000"), 50),
        Err(FalloRango::RangoAjeno {
            pedido: 100,
            recibido: 0
        })
    );
}

#[test]
fn un_206_vacio_no_avanza_y_se_dice() {
    let p = pidiendo(100, 50);
    assert_eq!(juzgar_rango(&p, 206, Some("bytes 100-149/1000"), 0), Err(FalloRango::Vacia));
}

#[test]
fn un_servidor_que_manda_de_mas_no_desborda_el_plan() {
    let p = pidiendo(100, 50);
    let a = juzgar_rango(&p, 206, None, 4096).expect("vale");
    assert_eq!(a.usar, 50, "se usa lo que cabe en el tramo, no lo que llegó");
}

#[test]
fn un_content_range_ilegible_no_tumba_la_descarga() {
    // El hash del fichero sigue siendo la comprobación de verdad; plantarse por
    // una cabecera rara dejaría sin actualizar a quien tenga delante un proxy
    // pintoresco.
    let p = pidiendo(100, 50);
    assert!(juzgar_rango(&p, 206, Some("vaya cosa"), 50).is_ok());
}

#[test]
fn un_200_sirve_solo_si_pedíamos_desde_el_principio() {
    let entero = pidiendo(0, 500);
    let a = juzgar_rango(&entero, 200, None, 4096).expect("vale");
    assert_eq!(a.usar, 500);
    assert!(a.completo, "el artefacto entero cierra el tramo");

    // A mitad, un 200 significa que el servidor ignoró el Range: lo que manda
    // no es lo que hace falta y colocarlo sería inventarse el offset.
    let medio = pidiendo(100, 50);
    assert_eq!(juzgar_rango(&medio, 200, None, 4096), Err(FalloRango::Estado(200)));
}

#[test]
fn un_200_corto_desde_el_principio_tampoco_vale() {
    let p = pidiendo(0, 500);
    assert_eq!(juzgar_rango(&p, 200, None, 499), Err(FalloRango::Estado(200)));
}

#[test]
fn los_errores_del_servidor_se_cuentan_con_su_codigo() {
    let p = pidiendo(0, 10);
    for code in [301, 302, 400, 401, 403, 404, 416, 429, 500, 502, 503] {
        assert_eq!(
            juzgar_rango(&p, code, None, 10),
            Err(FalloRango::Estado(code)),
            "{code} tiene que rechazarse diciendo cuál era"
        );
    }
}

#[test]
fn content_range_se_lee_como_dice_el_estandar() {
    assert_eq!(parse_content_range("bytes 0-99/1000"), Some(0));
    assert_eq!(parse_content_range("bytes 500-999/1000"), Some(500));
    assert_eq!(parse_content_range("  bytes   42-99/*  "), Some(42));
    assert_eq!(parse_content_range("items 0-99/1000"), None);
    assert_eq!(parse_content_range(""), None);
}
