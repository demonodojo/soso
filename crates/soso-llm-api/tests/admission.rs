//! T17 — admisión pura (sin sockets).

use soso_llm_api::{
    aux_feed_authenticated, classify_idle_request, classify_while_generating, format_response,
    AdmissionError, AdmissionState, AuxResponse, AuxSlot, IdleRoute, HttpRequest,
};

const TOKEN: &str = "sekret";
const HEALTH: &[u8] = br#"{"status":"ready"}"#;
const BUSY: &[u8] = br#"{"code":"busy"}"#;
const UNAUTH: &[u8] = br#"{"code":"invalid_api_key"}"#;

fn health_http() -> Vec<u8> {
    format_response(200, &[("Content-Type", "application/json")], HEALTH)
}

fn busy_http() -> Vec<u8> {
    format_response(429, &[("Content-Type", "application/json")], BUSY)
}

fn unauth_http() -> Vec<u8> {
    format_response(401, &[("Content-Type", "application/json")], UNAUTH)
}

#[test]
fn generation_exclusiva_y_aux_limitado() {
    let mut adm = AdmissionState::default();
    assert!(adm.begin_generation().is_ok());
    assert_eq!(adm.begin_generation(), Err(AdmissionError::AlreadyGenerating));
    assert!(adm.aux_vacant_index().is_some());
    adm.register_aux(0, 1, 0);
    adm.register_aux(1, 2, 0);
    assert!(adm.aux_vacant_index().is_none());
    assert_eq!(adm.aux_count(), 2);
    adm.end_generation();
    assert!(!adm.is_generating());
    assert_eq!(adm.aux_count(), 0);
    assert!(adm.begin_generation().is_ok());
}

#[test]
fn rutas_idle() {
    assert_eq!(
        classify_idle_request("GET", "/health"),
        IdleRoute::Health
    );
    assert_eq!(
        classify_idle_request("POST", "/v1/chat/completions"),
        IdleRoute::ChatCompletion
    );
}

#[test]
fn aux_health_completo() {
    let mut slot = AuxSlot::new(9, 0);
    let raw = format!(
        "GET /health HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {TOKEN}\r\n\r\n"
    );
    let out = aux_feed_authenticated(
        &mut slot,
        raw.as_bytes(),
        100,
        TOKEN,
        &health_http(),
        &busy_http(),
        &unauth_http(),
    )
    .unwrap();
    match out {
        AuxResponse::Reply(b) => assert!(b.windows(6).any(|w| w == b"200 OK")),
        _ => panic!("expected reply"),
    }
}

#[test]
fn aux_busy_sin_leer_cuerpo_post() {
    let mut slot = AuxSlot::new(9, 0);
    let raw = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 9999\r\n\r\n"
    );
    let out = aux_feed_authenticated(
        &mut slot,
        raw.as_bytes(),
        100,
        TOKEN,
        &health_http(),
        &busy_http(),
        &unauth_http(),
    )
    .unwrap();
    match out {
        AuxResponse::Reply(b) => assert!(b.windows(3).any(|w| w == b"429")),
        _ => panic!("expected busy"),
    }
}

#[test]
fn aux_auth_invalida() {
    let mut slot = AuxSlot::new(9, 0);
    let raw = "GET /health HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer mal\r\n\r\n";
    let out = aux_feed_authenticated(
        &mut slot,
        raw.as_bytes(),
        100,
        TOKEN,
        &health_http(),
        &busy_http(),
        &unauth_http(),
    )
    .unwrap();
    match out {
        AuxResponse::Reply(b) => assert!(b.windows(3).any(|w| w == b"401")),
        _ => panic!("expected 401"),
    }
}

#[test]
fn classify_while_generating_rutas() {
    let health_req = HttpRequest {
        method: String::from("GET"),
        path: String::from("/health"),
        headers: Vec::new(),
        body: Vec::new(),
    };
    match classify_while_generating(&health_req, &health_http(), &busy_http()) {
        AuxResponse::Reply(b) => assert!(b.windows(6).any(|w| w == b"200 OK")),
        _ => panic!(),
    }
    let chat_req = HttpRequest {
        method: String::from("POST"),
        path: String::from("/v1/chat/completions"),
        headers: Vec::new(),
        body: Vec::new(),
    };
    match classify_while_generating(&chat_req, &health_http(), &busy_http()) {
        AuxResponse::Reply(b) => assert!(b.windows(3).any(|w| w == b"429")),
        _ => panic!(),
    }
}

#[test]
fn aux_deadline_cierra() {
    let mut slot = AuxSlot::new(9, 0);
    let out = aux_feed_authenticated(
        &mut slot,
        b"GET /he",
        40_000,
        TOKEN,
        &health_http(),
        &busy_http(),
        &unauth_http(),
    )
    .unwrap();
    assert_eq!(out, AuxResponse::Close);
}
