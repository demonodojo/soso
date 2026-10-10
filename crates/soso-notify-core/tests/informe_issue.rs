use soso_notify_core::{
    build_issue_json, cola_flujo, construir_cuerpo, issues_url, parse_conf, parse_issue_response,
    titulo_issue, token_listo, FlujoEntrada, InformeEntrada, CUERPO_GITHUB_MAX,
};

#[test]
fn config_sin_token() {
    let c = parse_conf("repo=demonodojo/soso\ntoken=\n");
    assert_eq!(c.repo, "demonodojo/soso");
    assert!(!token_listo(&c));
    assert!(issues_url(&c.repo).unwrap().contains("/repos/demonodojo/soso/issues"));
}

#[test]
fn json_escapa_comillas_y_saltos() {
    let j = build_issue_json("t\"t", "línea1\nlínea2\\fin");
    assert!(j.contains("\\\""));
    assert!(j.contains("\\n"));
    assert!(j.contains("\\\\"));
    assert!(j.len() < CUERPO_GITHUB_MAX);
}

#[test]
fn activo_corto_incluye_rotado() {
    let activo = b"activo";
    let rotado = b"0123456789";
    let (merged, omit) = cola_flujo(activo, Some(rotado), 64);
    assert_eq!(omit, 0);
    let s = core::str::from_utf8(&merged).unwrap();
    assert!(s.contains("activo"));
    assert!(s.contains("continua desde .1"));
    assert!(s.contains("0123456789"));
}

#[test]
fn activo_largo_no_arrastra_rotado() {
    let activo = vec![b'k'; 50_000];
    let rotado = vec![b'r'; 10_000];
    let (merged, omit) = cola_flujo(&activo, Some(&rotado), 40 * 1024);
    assert_eq!(merged.len(), 40 * 1024);
    assert!(omit >= 10_000);
    assert!(!merged.windows(4).any(|w| w == b"rrrr"));
}

#[test]
fn respuesta_201_html_url() {
    let body = br#"{"html_url":"https://github.com/demonodojo/soso/issues/42","number":42}"#;
    let url = parse_issue_response(201, body).unwrap();
    assert_eq!(url, "https://github.com/demonodojo/soso/issues/42");
}

#[test]
fn respuesta_401() {
    assert_eq!(
        parse_issue_response(401, b"{}").unwrap_err(),
        "token inválido o caducado (HTTP 401)"
    );
}

#[test]
fn cuerpo_informe_bajo_limite_github() {
    let activo = vec![b'x'; 200_000];
    let flujos = [FlujoEntrada {
        nombre: "kernel.log",
        activo: &activo,
        rotado: None,
    }];
    let ent = InformeEntrada {
        mensaje: "fallo",
        release: Some("version=0.3.1"),
        hw: None,
        flujos: &flujos,
    };
    let cuerpo = construir_cuerpo(ent);
    assert!(cuerpo.len() <= CUERPO_GITHUB_MAX);
}

#[test]
fn titulo_sin_mensaje_usa_release() {
    let t = titulo_issue("", Some("version=0.3.1\nbuild=abc"));
    assert!(t.starts_with("notifybug "));
    assert!(t.contains("0.3.1"));
}
