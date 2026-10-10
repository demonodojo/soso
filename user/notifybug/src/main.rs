//! Envía un informe de logs de la máquina a una issue de GitHub.

#![no_std]
#![no_main]

extern crate alloc;

mod io;
mod net;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use io::{log_rotado, read_log_tail, read_text, LEE_LOG_CAP};
use libsoso::{println, sys};
use soso_log_core::{DIR_LOG, FLUJO_APPS, FLUJO_KERNEL, FLUJO_OTA};
use soso_notify_core::{
    build_issue_json, construir_cuerpo, issues_url, parse_conf, parse_issue_response,
    titulo_issue, token_listo, FlujoEntrada, InformeEntrada,
};

libsoso::entry!(main);

const CONF: &str = "/etc/notifybug.conf";
const RELEASE: &str = "/etc/soso-release";
const HW: &str = "/etc/soso-hw";

fn main(args: &[String]) -> u8 {
    let mut ver = false;
    let mut mensaje = String::new();
    for a in args {
        if a == "--ver" {
            ver = true;
        } else if a.starts_with('-') {
            println!("notifybug: opción desconocida {a}");
            return 2;
        } else {
            if !mensaje.is_empty() {
                mensaje.push(' ');
            }
            mensaje.push_str(a);
        }
    }

    let r = sys::fatlog_flush();
    if r < 0 {
        println!("notifybug: aviso: no pude volcar el log a disco ({r})");
    }

    let conf_text = read_text(CONF, 2048).unwrap_or_default();
    let conf = parse_conf(&conf_text);

    let release = read_text(RELEASE, 4096);
    let hw = read_text(HW, 16384);

    let mut datos_log = Vec::new();
    for name in [FLUJO_KERNEL, FLUJO_APPS, FLUJO_OTA] {
        let base = format!("{DIR_LOG}/{name}");
        let rot_path = log_rotado(&base);
        let rotado = {
            let v = read_log_tail(&rot_path, LEE_LOG_CAP);
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        };
        datos_log.push(FlujoDatos {
            activo: read_log_tail(&base, LEE_LOG_CAP),
            rotado,
        });
    }
    let flujos: Vec<FlujoEntrada<'_>> = datos_log
        .iter()
        .zip([FLUJO_KERNEL, FLUJO_APPS, FLUJO_OTA])
        .map(|(d, name)| FlujoEntrada {
            nombre: name,
            activo: &d.activo,
            rotado: d.rotado.as_deref(),
        })
        .collect();
    let ent = InformeEntrada {
        mensaje: &mensaje,
        release: release.as_deref(),
        hw: hw.as_deref(),
        flujos: &flujos,
    };
    let cuerpo = construir_cuerpo(ent);
    let titulo = titulo_issue(&mensaje, release.as_deref());

    if ver {
        println!("--- título ---");
        println!("{titulo}");
        println!("--- cuerpo ---");
        println!("{cuerpo}");
        return 0;
    }

    if !token_listo(&conf) {
        println!("notifybug: falta token en {CONF}");
        println!("  Edita el fichero y pon un PAT de GitHub con permiso Issues (escritura):");
        println!("  token=ghp_…");
        println!("  repo={}", conf.repo);
        return 2;
    }

    let url = match issues_url(&conf.repo) {
        Ok(u) => u,
        Err(e) => {
            println!("notifybug: {e}");
            return 2;
        }
    };

    let json = build_issue_json(&titulo, &cuerpo);
    println!("notifybug: enviando informe a {}…", conf.repo);
    let resp = match net::https_post_json(&url, &conf.token, json.as_bytes()) {
        Ok(r) => r,
        Err(e) => {
            println!("notifybug: {}", net::map_http_err(e));
            return 1;
        }
    };

    match parse_issue_response(resp.status, &resp.body) {
        Ok(issue_url) => {
            println!("notifybug: issue creada:");
            println!("  {issue_url}");
            0
        }
        Err(e) => {
            println!("notifybug: {e}");
            if !resp.body.is_empty() {
                let preview = core::str::from_utf8(&resp.body[..resp.body.len().min(200)])
                    .unwrap_or("(binario)");
                println!("  respuesta: {preview}");
            }
            1
        }
    }
}

struct FlujoDatos {
    activo: Vec<u8>,
    rotado: Option<Vec<u8>>,
}
