//! Dirección IPv4 fija: `/etc/red.conf` y la orden `ip estatica`.
//!
//! Sin fichero, o con `modo=dhcp`, la máquina usa DHCP.

use alloc::string::String;

/// Ruta del fichero en sosofs.
pub const RED_CONF: &str = "/etc/red.conf";

/// Dirección fija: `addr/prefijo`, pasarela y un DNS opcionales.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RedEstatica {
    pub addr: [u8; 4],
    pub prefijo: u8,
    pub gw: Option<[u8; 4]>,
    pub dns: Option<[u8; 4]>,
}

/// Por qué una configuración no vale. El texto sale tal cual por consola.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorRed {
    /// `modo=estatica` sin `addr=`.
    FaltaAddr,
    /// `addr=` que no es `a.b.c.d/p` con `p` entre 1 y 32.
    AddrInvalida,
    /// `gw=` que no es una IPv4.
    GwInvalida,
    /// `dns=` que no es una IPv4.
    DnsInvalido,
    /// `modo=` distinto de `estatica` y `dhcp`.
    ModoDesconocido,
    /// La pasarela no está en la red de `addr`.
    GwFueraDeRed,
    /// Una orden `ip estatica` con palabras que no entiende.
    Orden,
}

impl ErrorRed {
    pub fn texto(self) -> &'static str {
        match self {
            ErrorRed::FaltaAddr => "falta addr=",
            ErrorRed::AddrInvalida => "addr= debe ser a.b.c.d/p (p de 1 a 32)",
            ErrorRed::GwInvalida => "gw= no es una IPv4",
            ErrorRed::DnsInvalido => "dns= no es una IPv4",
            ErrorRed::ModoDesconocido => "modo= debe ser estatica o dhcp",
            ErrorRed::GwFueraDeRed => "la pasarela no está en la red de addr",
            ErrorRed::Orden => "uso: ip estatica <a.b.c.d/p> [gw <ip>] [dns <ip>]",
        }
    }
}

pub fn parsear_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut n = 0;
    for parte in s.split('.') {
        if n == 4 || parte.is_empty() || parte.len() > 3 || !parte.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        out[n] = parte.parse().ok()?;
        n += 1;
    }
    (n == 4).then_some(out)
}

fn parsear_cidr(s: &str) -> Option<([u8; 4], u8)> {
    let (ip, p) = s.split_once('/')?;
    let prefijo: u8 = p.parse().ok()?;
    if !(1..=32).contains(&prefijo) {
        return None;
    }
    Some((parsear_ipv4(ip)?, prefijo))
}

fn misma_red(a: [u8; 4], b: [u8; 4], prefijo: u8) -> bool {
    let mascara = if prefijo >= 32 { u32::MAX } else { !(u32::MAX >> prefijo) };
    (u32::from_be_bytes(a) & mascara) == (u32::from_be_bytes(b) & mascara)
}

fn validar(r: RedEstatica) -> Result<RedEstatica, ErrorRed> {
    match r.gw {
        Some(gw) if !misma_red(r.addr, gw, r.prefijo) => Err(ErrorRed::GwFueraDeRed),
        _ => Ok(r),
    }
}

/// Lee `red.conf`. `Ok(None)` es DHCP: fichero sin `modo=` o con `modo=dhcp`.
/// `modo=estatica` incompleto o con valores malos es un error, no DHCP en
/// silencio: quien lo escribió espera una dirección fija.
pub fn parsear_red_conf(texto: &str) -> Result<Option<RedEstatica>, ErrorRed> {
    let (mut estatica, mut addr, mut gw, mut dns) = (false, None, None, None);
    // Los valores malos sólo importan si el modo es estático: un `modo=dhcp`
    // con una `addr=` vieja encima sigue siendo DHCP.
    let mut error = None;
    for linea in texto.lines() {
        let linea = linea.trim();
        if linea.is_empty() || linea.starts_with('#') {
            continue;
        }
        let Some((clave, valor)) = linea.split_once('=') else {
            continue;
        };
        let valor = valor.trim();
        match clave.trim() {
            "modo" => match valor {
                "estatica" => estatica = true,
                "dhcp" => estatica = false,
                _ => return Err(ErrorRed::ModoDesconocido),
            },
            "addr" => match parsear_cidr(valor) {
                Some(a) => addr = Some(a),
                None => error = error.or(Some(ErrorRed::AddrInvalida)),
            },
            "gw" => match parsear_ipv4(valor) {
                Some(g) => gw = Some(g),
                None => error = error.or(Some(ErrorRed::GwInvalida)),
            },
            "dns" => match parsear_ipv4(valor) {
                Some(d) => dns = Some(d),
                None => error = error.or(Some(ErrorRed::DnsInvalido)),
            },
            _ => {}
        }
    }
    if !estatica {
        return Ok(None);
    }
    if let Some(e) = error {
        return Err(e);
    }
    let (addr, prefijo) = addr.ok_or(ErrorRed::FaltaAddr)?;
    validar(RedEstatica { addr, prefijo, gw, dns }).map(Some)
}

fn push_ip(out: &mut String, ip: [u8; 4]) {
    use core::fmt::Write;
    let _ = write!(out, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
}

/// Texto de `red.conf` para una dirección fija.
pub fn formatear_red_conf(r: &RedEstatica) -> String {
    use core::fmt::Write;
    let mut out = String::from("modo=estatica\naddr=");
    push_ip(&mut out, r.addr);
    let _ = write!(out, "/{}\n", r.prefijo);
    if let Some(gw) = r.gw {
        out.push_str("gw=");
        push_ip(&mut out, gw);
        out.push('\n');
    }
    if let Some(dns) = r.dns {
        out.push_str("dns=");
        push_ip(&mut out, dns);
        out.push('\n');
    }
    out
}

/// Texto de `red.conf` que devuelve la máquina a DHCP.
pub const RED_CONF_DHCP: &str = "modo=dhcp\n";

/// Orden `ip estatica`: `<a.b.c.d/p> [gw <ip>] [dns <ip>]`, ya sin la palabra
/// `estatica`. `gw` y `dns` pueden ir en cualquier orden.
pub fn parsear_orden(texto: &str) -> Result<RedEstatica, ErrorRed> {
    let mut palabras = texto.split_whitespace();
    let (addr, prefijo) = palabras
        .next()
        .and_then(parsear_cidr)
        .ok_or(ErrorRed::AddrInvalida)?;
    let (mut gw, mut dns) = (None, None);
    while let Some(clave) = palabras.next() {
        let valor = palabras.next().ok_or(ErrorRed::Orden)?;
        match clave {
            "gw" if gw.is_none() => gw = Some(parsear_ipv4(valor).ok_or(ErrorRed::GwInvalida)?),
            "dns" if dns.is_none() => {
                dns = Some(parsear_ipv4(valor).ok_or(ErrorRed::DnsInvalido)?)
            }
            _ => return Err(ErrorRed::Orden),
        }
    }
    validar(RedEstatica { addr, prefijo, gw, dns })
}
