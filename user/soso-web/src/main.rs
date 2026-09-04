//! soso-web — navegador mínimo para soso (modo lectura y gráfico).

#![no_std]
#![no_main]
#![allow(static_mut_refs)]

extern crate alloc;

mod fetch;
mod net;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use libsoso::linea::Lector;
use libsoso::{entry, print, println};
use soso_web::html::{reflow_html, ReflowOutput};

entry!(main);

const ANCHO_DEF: usize = 72;

struct Args {
    url: Option<String>,
    local: Option<String>,
    grafico: bool,
    ancho: usize,
    fuente: Option<String>,
}

fn parse_args(raw: &str) -> Args {
    let mut a = Args {
        url: None,
        local: None,
        grafico: false,
        ancho: ANCHO_DEF,
        fuente: None,
    };
    let mut it = raw.split_whitespace();
    while let Some(tok) = it.next() {
        match tok {
            "--local" => a.local = it.next().map(String::from),
            "--grafico" | "-g" => a.grafico = true,
            "--ancho" => {
                if let Some(n) = it.next().and_then(|s| s.parse().ok()) {
                    a.ancho = n;
                }
            }
            "--fuente" => a.fuente = it.next().map(String::from),
            "--help" | "-h" => {}
            other if !other.starts_with('-') => a.url = Some(String::from(other)),
            _ => {}
        }
    }
    a
}

fn main(args: &str) -> u8 {
    let a = parse_args(args);
    if args.trim().contains("--help") || args.trim().contains("-h") {
        ayuda();
        return 0;
    }
    if a.grafico {
        return modo_grafico(a);
    }
    modo_lectura(a)
}

fn ayuda() {
    println!("soso-web — navegador mínimo");
    println!("  soso-web <url>           Abre URL HTTPS");
    println!("  soso-web --local <ruta>  HTML local (pruebas)");
    println!("  soso-web --grafico <url> Modo gráfico");
    println!("  soso-web --fuente <ruta>   TTF para modo gráfico (default /lib/fonts/DejaVuSans.ttf)");
    println!("  --ancho N                Ancho de línea (modo lectura)");
    println!("Comandos: <n> enlace | u <url> | b atrás | q salir");
}

fn modo_lectura(a: Args) -> u8 {
    let mut hist: Vec<String> = Vec::new();
    if let Some(p) = a.local {
        match fetch::fetch_local(&p) {
            Ok(html) => {
                let out = reflow_html(&html, "file://local/", a.ancho);
                mostrar_pagina(&out, &format!("file://{p}"));
                unsafe { ULTIMO_REFLOW = Some(out); }
                hist.push(format!("file://{p}"));
                return repl_lectura(&mut hist, a.ancho);
            }
            Err(e) => {
                println!("soso-web: {} ({})", e.mensaje(), linea_err(&e));
                return 1;
            }
        }
    }
    let cur_url = match a.url {
        Some(u) => u,
        None => {
            println!("soso-web: falta URL o --local");
            return 2;
        }
    };

    if let Err(e) = cargar_y_mostrar(&cur_url, a.ancho) {
        println!("soso-web: {} ({})", e.mensaje(), linea_err(&e));
        return 1;
    }
    hist.push(cur_url);
    repl_lectura(&mut hist, a.ancho)
}

fn repl_lectura(hist: &mut Vec<String>, ancho: usize) -> u8 {
    let mut lector = Lector::new();
    loop {
        print!("\n[soso-web] ");
        let linea = match lector.siguiente() {
            Some(l) => l,
            None => continue,
        };
        let t = linea.trim();
        if t.is_empty() {
            continue;
        }
        if t == "q" || t == "salir" {
            return 0;
        }
        if t == "b" || t == "atras" {
            if hist.len() > 1 {
                hist.pop();
                let url = hist.last().unwrap().clone();
                if let Err(e) = cargar_y_mostrar(&url, ancho) {
                    println!("soso-web: {}", e.mensaje());
                }
            } else {
                println!("soso-web: no hay página anterior");
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("u ") {
            let url = normaliza_url(rest.trim());
            match cargar_y_mostrar(&url, ancho) {
                Ok(()) => hist.push(url),
                Err(e) => println!("soso-web: {}", e.mensaje()),
            }
            continue;
        }
        if let Ok(n) = t.parse::<usize>() {
            if let Some(url) = ultimo_enlace(n) {
                match cargar_y_mostrar(&url, ancho) {
                    Ok(()) => hist.push(url),
                    Err(e) => println!("soso-web: {}", e.mensaje()),
                }
            } else {
                println!("soso-web: enlace {n} no existe");
            }
            continue;
        }
        println!("soso-web: comando desconocido (n | u URL | b | q)");
    }
}

static mut ULTIMO_REFLOW: Option<ReflowOutput> = None;

fn cargar_y_mostrar(url: &str, ancho: usize) -> Result<(), fetch::FetchError> {
    let (html, final_url) = if url.starts_with("file://") {
        let path = url.strip_prefix("file://").unwrap_or("/etc/web-prueba.html");
        let html = fetch::fetch_local(path)?;
        (html, url.to_string())
    } else {
        fetch::fetch_https(url)?
    };
    let out = reflow_html(&html, &final_url, ancho);
    mostrar_pagina(&out, &final_url);
    unsafe {
        ULTIMO_REFLOW = Some(out);
    }
    Ok(())
}

fn mostrar_pagina(out: &ReflowOutput, url: &str) {
    println!("\n=== {url} ===\n");
    println!("{}", out.text);
    if !out.links.is_empty() {
        println!("\n--- enlaces ---");
        for l in &out.links {
            println!("[{}] {} -> {}", l.idx, l.text, l.href);
        }
    }
}

fn ultimo_enlace(n: usize) -> Option<String> {
    unsafe {
        ULTIMO_REFLOW
            .as_ref()?
            .links
            .iter()
            .find(|l| l.idx == n)
            .map(|l| l.href.clone())
    }
}

fn normaliza_url(s: &str) -> String {
    if s.starts_with("https://") || s.starts_with("http://") {
        s.to_string()
    } else {
        format!("https://{s}")
    }
}

fn linea_err(e: &fetch::FetchError) -> String {
    match e {
        fetch::FetchError::Io(code) => format!("errno {}", -code),
        fetch::FetchError::Status(st) => format!("HTTP {st}"),
        fetch::FetchError::Http(soso_http::HttpError::Parse) => String::from("parse"),
        fetch::FetchError::Http(soso_http::HttpError::Tls) => String::from("tls"),
        fetch::FetchError::Http(soso_http::HttpError::Io) => String::from("http-io"),
        fetch::FetchError::Http(soso_http::HttpError::Dns) => String::from("dns"),
    }
}

fn modo_grafico(a: Args) -> u8 {
    use soso_web::gui::{fb, input, layout, render};

    let url = match a.url {
        Some(u) => u,
        None => {
            println!("soso-web: --grafico requiere URL");
            return 2;
        }
    };
    let mut fb = match fb::Framebuffer::open() {
        Ok(f) => f,
        Err(e) => {
            println!("soso-web: framebuffer no disponible ({})", libsoso::errno_str(e));
            return 1;
        }
    };
    let font_path = a
        .fuente
        .as_deref()
        .unwrap_or(render::DEFAULT_FONT);
    let mut renderer = match render::Renderer::open(font_path) {
        Some(r) => r,
        None => {
            println!("soso-web: no se pudo cargar fuente {font_path}");
            return 1;
        }
    };
    let (html, base) = match fetch::fetch_https(&url) {
        Ok(x) => x,
        Err(e) => {
            println!("soso-web: {}", e.mensaje());
            return 1;
        }
    };
    let doc = layout::layout_html(&html, &base);
    let (boxes, _total_h) = layout::compute_layout(&doc, fb.info.width as f32);
    let mut input_st = input::InputState::default();
    let mut scroll = 0.0f32;
    let mut hist: Vec<String> = vec![url];
    let mut cur_doc = doc;
    let mut cur_boxes = boxes;
    let hover: Option<usize> = None;

    loop {
        input::poll_input(&mut input_st);
        if let Some(key) = input_st.key {
            match key {
                0x01 => return 0,
                k if k == b'q' as u32 || k == b'Q' as u32 => return 0,
                k if k == b'b' as u32 || k == b'B' as u32 => {
                    if hist.len() > 1 {
                        hist.pop();
                        if let Some(prev) = hist.last() {
                            if recargar_grafico(prev, &mut cur_doc, &mut cur_boxes, fb.info.width as f32).is_ok() {
                                scroll = 0.0;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if input_st.btn_left {
            let mut destino: Option<String> = None;
            for lb in &cur_boxes {
                if input_st.mouse_y as f32 + scroll >= lb.y
                    && input_st.mouse_y as f32 + scroll < lb.y + lb.h
                {
                    if let soso_web::gui::layout::Node::Link { href, .. } = &cur_doc.nodes[lb.node_idx] {
                        destino = Some(href.clone());
                        break;
                    }
                }
            }
            if let Some(href) = destino {
                hist.push(href.clone());
                let _ = recargar_grafico(&href, &mut cur_doc, &mut cur_boxes, fb.info.width as f32);
                scroll = 0.0;
            }
        }
        scroll = (scroll - input_st.mouse_y as f32 * 0.0).max(0.0);
        renderer.draw_document(&mut fb, &cur_doc, &cur_boxes, scroll, hover);
        let _ = fb.present();
        libsoso::sys::sleep_ms(16);
    }
}

fn recargar_grafico(
    url: &str,
    doc: &mut soso_web::gui::layout::Document,
    boxes: &mut Vec<soso_web::gui::layout::LayoutBox>,
    width: f32,
) -> Result<(), fetch::FetchError> {
    let (html, base) = fetch::fetch_https(url)?;
    *doc = soso_web::gui::layout::layout_html(&html, &base);
    let (b, _) = soso_web::gui::layout::compute_layout(doc, width);
    *boxes = b;
    Ok(())
}
