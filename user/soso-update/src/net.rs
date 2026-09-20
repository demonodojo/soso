//! HTTPS para descargas de actualización.

use alloc::vec::Vec;
use libsoso::{abi, println, sys};
use soso_abi::SockAddr;
use soso_update_core::descarga;
use soso_http::TcpTransport;

struct Net;

/// Traza byte a byte del transporte, que enciende `--traza`. En una avería de
/// red lo que hace falta saber es si la petición salió y si volvió algo, y eso
/// no se deduce del mensaje de error. `sosh` no tiene variables de entorno, así
/// que es una bandera y no una variable.
static TRAZA_BYTES: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

fn ahora_ms() -> u64 {
    let mut ts = abi::Timespec::default();
    if sys::clock_gettime(abi::CLOCK_MONOTONIC, &mut ts) != 0 {
        return 0;
    }
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}

fn guest_wall_clock() -> Option<u64> {
    let mut ts = abi::Timespec::default();
    if sys::clock_gettime(abi::CLOCK_REALTIME, &mut ts) != 0 {
        return None;
    }
    let secs = ts.tv_sec as u64;
    if secs < 1_000_000_000 {
        None
    } else {
        Some(secs)
    }
}

fn ensure_wall_clock() {
    static INSTALLED: core::sync::atomic::AtomicBool =
        core::sync::atomic::AtomicBool::new(false);
    if !INSTALLED.swap(true, core::sync::atomic::Ordering::AcqRel) {
        soso_http::set_wall_clock(guest_wall_clock);
    }
}

/// Enciende la traza byte a byte del transporte (`--traza`).
pub fn encender_traza() {
    TRAZA_BYTES.store(true, core::sync::atomic::Ordering::Relaxed);
}

/// Las etapas de una descarga se dicen según pasan.
///
/// No es ruido: entre resolver un nombre y tener el manifiesto hay DNS, TCP,
/// TLS y HTTP, cada uno con su forma de quedarse parado. Sin esto, un fallo en
/// cualquiera de los cuatro se ve igual desde fuera —el comando callado— y no
/// hay manera de saber cuál. Costó dos pasadas de quince minutos averiguar que
/// se paraba **después** de resolver el origen.
impl TcpTransport for Net {
    fn dns_resolve(&self, host: &str, out: &mut [u8; 4]) -> Result<(), i64> {
        let r = sys::dns_resolve(host, out);
        match r {
            Ok(()) => println!(
                "  red: {host} → {}.{}.{}.{}",
                out[0], out[1], out[2], out[3]
            ),
            Err(e) => println!("  red: no pude resolver {host} (errno {})", -e),
        }
        r
    }

    fn tcp_connect(&self, addr: SockAddr, timeout_ms: u64) -> Result<u64, i64> {
        println!(
            "  red: conectando a {}.{}.{}.{}:{}…",
            addr.addr[0], addr.addr[1], addr.addr[2], addr.addr[3], addr.port
        );
        let fd = sys::tcp_connect(&addr, timeout_ms);
        if fd < 0 {
            println!("  red: conexión rechazada (errno {})", -fd);
            Err(fd)
        } else {
            println!("  red: conectado (fd {fd})");
            Ok(fd as u64)
        }
    }

    fn read_timeout(&self, fd: u64, buf: &mut [u8], timeout_ms: u64) -> i64 {
        let t0 = ahora_ms();
        let n = sys::read_timeout(fd, buf, timeout_ms);
        if TRAZA_BYTES.load(core::sync::atomic::Ordering::Relaxed) {
            println!("  red: read(fd {fd}, {timeout_ms}ms) = {n} en {}ms", ahora_ms() - t0);
        }
        n
    }

    fn write_all(&self, fd: u64, data: &[u8]) -> Result<(), i64> {
        let r = sys::write_all(fd, data);
        if TRAZA_BYTES.load(core::sync::atomic::Ordering::Relaxed) {
            println!("  red: write(fd {fd}, {} B) = {:?}", data.len(), r);
        }
        r
    }

    fn close(&self, fd: u64) {
        let _ = sys::close(fd);
    }
}

pub fn https_get_bytes(url: &str, token: Option<&str>) -> Result<Vec<u8>, &'static str> {
    ensure_wall_clock();
    let resp = soso_http::https_get(&Net, url, token).map_err(map_http_err)?;
    if resp.status != 200 {
        return Err("HTTP != 200");
    }
    Ok(resp.body)
}

/// Trozo máximo en vuelo. Lo fija `soso-update-core` (U4) y acota dos cosas a
/// la vez: la RAM del cliente y lo que se pierde si se corta la red.
const MAX_RANGE: u64 = soso_update_core::TROZO_MAX;

pub fn https_download_all(
    url: &str,
    token: Option<&str>,
    total: u64,
) -> Result<Vec<u8>, &'static str> {
    https_download_span(url, token, 0, total)
}

/// Descarga `len` bytes desde `start` con peticiones `Range` de como mucho
/// `MAX_RANGE`. Es la primitiva de la actualización parcial: pedimos sólo los
/// tramos del pack que cubren ficheros cambiados.
pub fn https_download_span(
    url: &str,
    token: Option<&str>,
    start: u64,
    len: u64,
) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::new();
    if len == 0 {
        return Ok(out);
    }
    if out.try_reserve(len as usize).is_err() {
        return Err("sin memoria");
    }
    https_download_span_a(url, token, start, len, &mut |trozo| {
        out.extend_from_slice(trozo);
        Ok(())
    })?;
    Ok(out)
}

/// Igual, pero entregando cada trozo según llega.
///
/// Es la variante que usa la descarga durable: el llamante lo escribe en el
/// área de preparación y lo hashea al vuelo, así que en RAM nunca hay más de
/// `MAX_RANGE`. Acumular el tramo entero —lo que se hacía antes— es memoria que
/// una máquina pequeña no tiene, y obliga a repetirlo todo si se corta la red.
pub fn https_download_span_a(
    url: &str,
    token: Option<&str>,
    start: u64,
    len: u64,
    recibe: &mut dyn FnMut(&[u8]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    if len == 0 {
        return Ok(());
    }
    let mut off = start;
    let fin = start + len;
    while off < fin {
        let end = (off + MAX_RANGE).min(fin) - 1;
        let resp =
            soso_http::https_get_range_full(&Net, url, token, off, end).map_err(map_http_err)?;
        let content_range = soso_http::header_value(&resp.headers, "content-range");
        // Qué respuesta vale y cuál no es política, y vive en el core para
        // poder probarla una a una en el host.
        let pet = descarga::Peticion {
            desde: off,
            hasta: end,
            restante: fin - off,
            desde_el_principio: start == 0,
        };
        let aceptado = descarga::juzgar_rango(&pet, resp.status, content_range, resp.body.len())
            .map_err(fallo_rango)?;
        recibe(&resp.body[..aceptado.usar])?;
        if aceptado.completo {
            return Ok(());
        }
        off += aceptado.usar as u64;
    }
    Ok(())
}

/// El porqué, con el dato que hace falta para arreglarlo.
fn fallo_rango(e: descarga::FalloRango) -> &'static str {
    match e {
        descarga::FalloRango::Vacia => "el servidor devolvió un tramo vacío",
        descarga::FalloRango::RangoAjeno { .. } => "el servidor devolvió otro tramo del pedido",
        descarga::FalloRango::Estado(404) => "el artefacto no está en el servidor (404)",
        descarga::FalloRango::Estado(429) => "el servidor pide esperar (429)",
        descarga::FalloRango::Estado(s) if s >= 500 => "el servidor falla (5xx)",
        descarga::FalloRango::Estado(s) if (300..400).contains(&s) => {
            "el servidor redirige y no seguimos redirecciones aquí"
        }
        descarga::FalloRango::Estado(_) => "el servidor no respeta Range",
    }
}

fn map_http_err(e: soso_http::HttpError) -> &'static str {
    match e {
        soso_http::HttpError::Clock => "reloj del sistema no utilizable",
        soso_http::HttpError::Dns => "DNS",
        // El motivo viaja con el error: se enseña, que para eso está.
        soso_http::HttpError::Tls(motivo) => {
            println!("  red: TLS falló — {motivo}");
            "TLS"
        }
        soso_http::HttpError::Parse => "HTTP parse",
        soso_http::HttpError::Io(donde) => {
            println!("  red: E/S falló — {donde}");
            "descarga HTTP"
        }
    }
}
