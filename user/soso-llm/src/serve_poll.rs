//! Sondeo cooperativo durante inferencia (T17): auxiliares, health, cancelación.

use core::sync::atomic::{AtomicBool, Ordering};

use libsoso::sys;
use soso_abi as abi;
use alloc::format;

use soso_llm_api::{
    aux_feed_authenticated, format_response, AdmissionState, AuxResponse, MAX_AUX_CONNECTIONS,
};

fn health_bytes(model: &str, backend: &str) -> alloc::vec::Vec<u8> {
    let body = format!(
        r#"{{"status":"ready","model":"{model}","backend":"{backend}"}}"#
    );
    format_response(200, &[("Content-Type", "application/json")], body.as_bytes())
}

fn busy_bytes() -> alloc::vec::Vec<u8> {
    let body = "{\"error\":{\"message\":\"generacion en curso\",\"type\":\"invalid_request_error\",\"code\":\"busy\"}}";
    format_response(429, &[("Content-Type", "application/json")], body.as_bytes())
}

fn unauth_bytes() -> alloc::vec::Vec<u8> {
    let body = "{\"error\":{\"message\":\"autenticacion invalida\",\"type\":\"invalid_request_error\",\"code\":\"invalid_api_key\"}}";
    format_response(401, &[("Content-Type", "application/json")], body.as_bytes())
}

fn write_all(fd: u64, data: &[u8]) -> Result<(), ()> {
    sys::write_all(fd, data).map_err(|_| ())
}

/// Espera mínima en lecturas de sondeo (no bloqueante efectivo).
const POLL_READ_MS: u64 = 1;

pub struct ServePollEnv<'a> {
    pub http_listener_fd: u64,
    pub active_fd: u64,
    pub admission: &'a mut AdmissionState,
    pub token: &'a str,
    pub model: &'a str,
    pub backend: &'a str,
}

pub fn poll_during_generation(env: &mut ServePollEnv<'_>, cancel: &AtomicBool) {
    if active_peer_closed(env.active_fd) {
        cancel.store(true, Ordering::Release);
    }
    try_accept_aux(env);
    poll_aux_connections(env);
    if cancel.load(Ordering::Acquire) {
        return;
    }
}

fn now_ms() -> u64 {
    sys::uptime_ms().max(0) as u64
}

fn active_peer_closed(fd: u64) -> bool {
    if fd == 0 {
        return false;
    }
    let mut buf = [0u8; 64];
    let n = sys::read_timeout(fd, &mut buf, POLL_READ_MS);
    if n == 0 {
        // EOF **no** es «el cliente se ha ido»: un cliente HTTP puede cerrar
        // su mitad de escritura en cuanto acaba de mandar la petición y seguir
        // esperando la respuesta (`shutdown(Write)`; lo hace el propio arnés
        // de T19). Cancelar aquí abortaba toda generación pedida por un
        // cliente así. Si el cliente se ha ido de verdad, la escritura de la
        // respuesta fallará y ahí sí se sabe.
        return false;
    }
    if n == -(abi::EAGAIN as i64) {
        return false;
    }
    if n < 0 {
        return true;
    }
    // Datos inesperados del cliente: no mezclar con otra petición; cancelar generación.
    true
}

fn try_accept_aux(env: &mut ServePollEnv<'_>) {
    if env.admission.aux_vacant_index().is_none() {
        return;
    }
    // `0` en `tcp_accept` **no** es «no bloquees»: es «sin plazo», y el proceso
    // se duerme para siempre. Con él, el primer punto de control de la
    // generación se quedaba aquí clavado y la petición no terminaba nunca: el
    // cliente veía una respuesta vacía tras su propio timeout y la sesión
    // quedaba ocupada, así que las siguientes recibían 429.
    let fd = sys::tcp_accept(env.http_listener_fd, POLL_READ_MS);
    if fd == -(abi::EAGAIN as i64) {
        return;
    }
    if fd < 0 {
        return;
    }
    let Some(idx) = env.admission.aux_vacant_index() else {
        let _ = sys::close(fd as u64);
        return;
    };
    env.admission.register_aux(idx, fd as u64, now_ms());
}

fn poll_aux_connections(env: &mut ServePollEnv<'_>) {
    let health = health_bytes(env.model, env.backend);
    let busy = busy_bytes();
    let unauth = unauth_bytes();
    let token = env.token;
    let t = now_ms();

    for i in 0..MAX_AUX_CONNECTIONS {
        let Some(fd) = env.admission.aux_slots_mut()[i].as_ref().map(|s| s.fd) else {
            continue;
        };
        let mut buf = [0u8; 512];
        let n = sys::read_timeout(fd, &mut buf, POLL_READ_MS);
        if n == 0 {
            env.admission.drop_aux(i);
            continue;
        }
        if n == -(abi::EAGAIN as i64) {
            continue;
        }
        if n < 0 {
            env.admission.drop_aux(i);
            let _ = sys::close(fd);
            continue;
        }
        let chunk = &buf[..n as usize];
        let outcome = {
            let slot = env.admission.aux_slots_mut()[i].as_mut().unwrap();
            aux_feed_authenticated(
                slot,
                chunk,
                t,
                token,
                &health,
                &busy,
                &unauth,
            )
        };
        match outcome {
            Ok(AuxResponse::NeedMore) => {}
            Ok(AuxResponse::Reply(resp)) => {
                let _ = write_all(fd, &resp);
                env.admission.drop_aux(i);
                let _ = sys::close(fd);
            }
            Ok(AuxResponse::Close) | Err(_) => {
                env.admission.drop_aux(i);
                let _ = sys::close(fd);
            }
        }
    }
}

/// Escritura que marca cancelación si el cliente cierra (SSE/JSON en vuelo).
pub fn write_bytes_or_cancel(fd: u64, data: &[u8], cancel: &AtomicBool) -> Result<(), ()> {
    match write_all(fd, data) {
        Ok(()) => Ok(()),
        Err(()) => {
            cancel.store(true, Ordering::Release);
            Err(())
        }
    }
}
