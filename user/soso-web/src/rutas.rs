//! Rutas del paquete instalado (`/var/paquetes/soso-web/`).

use alloc::format;
use alloc::string::String;

use libsoso::sys;

pub const PKG: &str = "soso-web";
pub const FONT_REL: &str = "share/fonts/DejaVuSans.ttf";
pub const DEMO_HTML_REL: &str = "share/ejemplo/web-prueba.html";
pub const FONT_SISTEMA: &str = "/lib/fonts/DejaVuSans.ttf";

fn existe(path: &str) -> bool {
    let mut st = libsoso::abi::Stat::default();
    sys::stat(path, &mut st) >= 0
}

/// Raíz del paquete si el binario vive bajo `/var/paquetes/<nombre>/`.
pub fn prefijo_instalacion() -> Option<String> {
    let argv = libsoso::argv();
    let argv0 = argv.first()?.as_str();
    let marcador = format!("/var/paquetes/{PKG}/");
    if argv0.starts_with(&marcador) {
        return Some(format!("/var/paquetes/{PKG}"));
    }
    if let Some(i) = argv0.rfind("/bin/") {
        let pref = &argv0[..i];
        if pref.starts_with("/var/paquetes/") {
            return Some(pref.into());
        }
    }
    None
}

pub fn fuente_por_defecto() -> String {
    if let Some(p) = prefijo_instalacion() {
        let pkg = format!("{p}/{FONT_REL}");
        if existe(&pkg) {
            return pkg;
        }
    }
    let fija = format!("/var/paquetes/{PKG}/{FONT_REL}");
    if existe(&fija) {
        return fija;
    }
    String::from(FONT_SISTEMA)
}

pub fn demo_html_local() -> String {
    if let Some(p) = prefijo_instalacion() {
        let pkg = format!("{p}/{DEMO_HTML_REL}");
        if existe(&pkg) {
            return pkg;
        }
    }
    let fija = format!("/var/paquetes/{PKG}/{DEMO_HTML_REL}");
    if existe(&fija) {
        return fija;
    }
    String::from("/etc/web-prueba.html")
}

pub fn demo_tab_url() -> String {
    format!("file://{}", demo_html_local())
}
