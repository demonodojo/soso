//! Aceptación reservada de R37.

use soso_http::{parse_url, HttpError};

#[test]
fn url_basica() {
    assert_eq!(
        parse_url("https://huggingface.co/api/models/x").unwrap(),
        ("https", "huggingface.co", 443, "/api/models/x".to_string())
    );
    assert_eq!(parse_url("https://h.org:8443/a/b").unwrap(), ("https", "h.org", 8443, "/a/b".to_string()));
    assert_eq!(parse_url("https://h.org").unwrap(), ("https", "h.org", 443, "/".to_string()));
}

#[test]
fn la_query_se_conserva_aunque_no_haya_ruta() {
    assert_eq!(parse_url("https://h.org/a?x=1&y=2").unwrap().3, "/a?x=1&y=2");
    let (_, host, port, path) = parse_url("https://h.org?q=1").unwrap();
    assert_eq!((host, port, path.as_str()), ("h.org", 443, "/?q=1"));
    let (_, host, port, path) = parse_url("https://h.org:8080?q=1/2").unwrap();
    assert_eq!((host, port, path.as_str()), ("h.org", 8080, "/?q=1/2"));
}

#[test]
fn el_fragmento_se_descarta() {
    assert_eq!(parse_url("https://h.org/a/b#sec").unwrap().3, "/a/b");
    assert_eq!(parse_url("https://h.org/a?x=1#f?y=2/z").unwrap().3, "/a?x=1");
    assert_eq!(parse_url("https://h.org#f").unwrap().3, "/");
    assert_eq!(parse_url("https://h.org:99#f/x").unwrap().2, 99);
}

#[test]
fn rechaza_esquemas_y_hosts_invalidos() {
    assert_eq!(parse_url("http://h.org/"), Err(HttpError::Parse));
    assert_eq!(parse_url("h.org/x"), Err(HttpError::Parse));
    assert_eq!(parse_url("https://"), Err(HttpError::Parse));
    assert_eq!(parse_url("https:///x"), Err(HttpError::Parse));
    assert_eq!(parse_url("https://?q=1"), Err(HttpError::Parse));
    assert_eq!(parse_url("https://:443/x"), Err(HttpError::Parse));
}

#[test]
fn rechaza_puertos_invalidos() {
    assert_eq!(parse_url("https://h.org:/x"), Err(HttpError::Parse));
    assert_eq!(parse_url("https://h.org:0/x"), Err(HttpError::Parse));
    assert_eq!(parse_url("https://h.org:65536/x"), Err(HttpError::Parse));
    assert_eq!(parse_url("https://h.org:abc/x"), Err(HttpError::Parse));
    assert_eq!(parse_url("https://h.org:65535/x").unwrap().2, 65535);
}

#[test]
fn rechaza_credenciales_en_la_url() {
    assert_eq!(parse_url("https://user:pw@h.org/x"), Err(HttpError::Parse));
    assert_eq!(parse_url("https://user@h.org"), Err(HttpError::Parse));
}
