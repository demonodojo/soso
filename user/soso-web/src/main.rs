//! soso-web — navegador mínimo para soso (modo lectura y gráfico).

#![no_std]
#![no_main]
#![allow(static_mut_refs)]

extern crate alloc;

mod bookmarks;
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

fn parse_args(raw: &[String]) -> Args {
    let mut a = Args {
        url: None,
        local: None,
        grafico: false,
        ancho: ANCHO_DEF,
        fuente: None,
    };
    let mut it = raw.iter().map(|s| s.as_str());
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

fn main(args: &[String]) -> u8 {
    let a = parse_args(args);
    if args.iter().any(|x| x == "--help" || x == "-h") {
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
    println!("  soso-web --fuente <ruta>   TTF para modo gráfico (paquete o /lib/fonts/…)");
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
            Ok(Some(l)) => l,
            Ok(None) => continue,
            Err(e) => {
                println!("soso-web: tty: {e}");
                return 1;
            }
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
        let demo = soso_web::rutas::demo_html_local();
        let path = url.strip_prefix("file://").unwrap_or(&demo);
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

#[derive(Clone)]
struct TabPage {
    doc: soso_web::gui::layout::Document,
    boxes: Vec<soso_web::gui::layout::LayoutBox>,
    content_h: f32,
    base_url: String,
    field_values: Vec<String>,
}

fn field_values_init(doc: &soso_web::gui::layout::Document) -> Vec<String> {
    let mut max = 0usize;
    for f in &doc.form_fields {
        max = max.max(f.field_idx);
    }
    let mut v = vec![String::new(); max + 1];
    for f in &doc.form_fields {
        if f.field_idx < v.len() {
            v[f.field_idx] = f.value.clone();
        }
    }
    v
}

fn empty_page(fb_w: f32) -> TabPage {
    let (doc, boxes, content_h) =
        soso_web::gui::layout::layout_page("", "about:blank", fb_w, &[]);
    TabPage {
        doc,
        boxes,
        content_h,
        base_url: String::from("about:blank"),
        field_values: Vec::new(),
    }
}

fn build_tab_page_from_html(html: &str, base: &str, fb_w: f32) -> TabPage {
    let imgs = fetch::load_decoded_images(html, base);
    let (doc, boxes, content_h) = soso_web::gui::layout::layout_page(html, base, fb_w, &imgs);
    let field_values = field_values_init(&doc);
    TabPage {
        doc,
        boxes,
        content_h,
        base_url: base.to_string(),
        field_values,
    }
}

fn build_tab_page(spec: &str, fb_w: f32) -> Result<TabPage, fetch::FetchError> {
    let page = fetch::fetch_page_ex(spec)?;
    if page.status != 200 {
        return Err(fetch::FetchError::Status(page.status));
    }
    Ok(build_tab_page_from_html(&page.html, &page.base, fb_w))
}

fn gfx_navigate(
    spec: &str,
    chrome_st: &mut soso_web::gui::chrome::Chrome,
    fb_w: f32,
    fb: &soso_web::gui::fb::Framebuffer,
    record_hist: bool,
) -> TabPage {
    let norm = normaliza_url_grafico(spec);
    match build_tab_page(&norm, fb_w) {
        Ok(p) => {
            if record_hist {
                chrome_st.active_tab_mut().navigate(norm.clone());
            } else {
                let t = chrome_st.active_tab_mut();
                t.url_text = norm.clone();
                t.error = None;
            }
            actualizar_titulo_wm(fb, &p.doc, norm.as_str());
            p
        }
        Err(e) => {
            chrome_st
                .active_tab_mut()
                .set_error(format!("{} ({})", e.mensaje(), linea_err(&e)));
            empty_page(fb_w)
        }
    }
}

enum ContentHit {
    None,
    Link(String),
    Field(usize),
    Submit(usize),
}

fn content_hit(page: &TabPage, my: i32, scroll: f32) -> ContentHit {
    use soso_web::gui::layout::{Node, CHROME_HEIGHT};
    if (my as f32) < CHROME_HEIGHT {
        return ContentHit::None;
    }
    let doc_y = my as f32 + scroll;
    for lb in &page.boxes {
        if doc_y >= lb.y && doc_y < lb.y + lb.h {
            return match &page.doc.nodes[lb.node_idx] {
                Node::Link { href, .. } => ContentHit::Link(href.clone()),
                Node::FormInput { field_idx, .. } => ContentHit::Field(*field_idx),
                Node::FormSubmit { form_idx, .. } => ContentHit::Submit(*form_idx),
                _ => ContentHit::None,
            };
        }
    }
    ContentHit::None
}

fn gfx_submit_form(
    page: &TabPage,
    form_idx: usize,
    chrome_st: &mut soso_web::gui::chrome::Chrome,
    fb_w: f32,
    fb: &soso_web::gui::fb::Framebuffer,
) -> TabPage {
    let Some(form) = fetch::form_for_submit(&page.doc.forms, form_idx) else {
        return page.clone();
    };
    let method = form.method.to_ascii_lowercase();
    if method == "post" {
        let url = soso_web_core::html::resolve_url(&page.base_url, &form.action);
        let body = fetch::build_form_post_body(
            &page.doc.form_fields,
            &page.field_values,
            form_idx,
        );
        match fetch::submit_form_post(&url, &body) {
            Ok(resp) if resp.status == 200 => {
                let p = build_tab_page_from_html(&resp.html, &resp.base, fb_w);
                chrome_st.active_tab_mut().navigate(url.clone());
                actualizar_titulo_wm(fb, &p.doc, url.as_str());
                p
            }
            Ok(resp) => {
                chrome_st
                    .active_tab_mut()
                    .set_error(format!("HTTP {}", resp.status));
                page.clone()
            }
            Err(e) => {
                chrome_st
                    .active_tab_mut()
                    .set_error(format!("{} ({})", e.mensaje(), linea_err(&e)));
                page.clone()
            }
        }
    } else {
        let url = fetch::build_form_get_url(
            &form.action,
            &page.base_url,
            &page.doc.form_fields,
            &page.field_values,
            form_idx,
        );
        gfx_navigate(&url, chrome_st, fb_w, fb, true)
    }
}

fn linea_err(e: &fetch::FetchError) -> String {
    match e {
        fetch::FetchError::Io(code) => format!("errno {}", -code),
        fetch::FetchError::Status(st) => format!("HTTP {st}"),
        fetch::FetchError::Http(soso_http::HttpError::Parse)
        | fetch::FetchError::Http(soso_http::HttpError::ParseDetail(_)) => String::from("parse"),
        fetch::FetchError::Http(soso_http::HttpError::Tls(_)) => String::from("tls"),
        fetch::FetchError::Http(soso_http::HttpError::Io(_)) => String::from("http-io"),
        fetch::FetchError::Http(soso_http::HttpError::Dns) => String::from("dns"),
        fetch::FetchError::Http(soso_http::HttpError::Clock) => String::from("reloj"),
        fetch::FetchError::Http(soso_http::HttpError::Interrupted) => String::from("interrumpido"),
    }
}

fn modo_grafico(a: Args) -> u8 {
    use soso_web::gui::{chrome, fb, input, layout, render};
    use soso_web::gui::input::DecodedKey;

    let initial_spec = match (&a.local, &a.url) {
        (Some(p), _) => format!("file://{p}"),
        (None, Some(u)) => u.clone(),
        _ => {
            println!("soso-web: --grafico requiere URL o --local");
            return 2;
        }
    };
    let (cw, ch) = fb::Framebuffer::tamaño_inicial_cliente();
    let titulo = titulo_corto(&initial_spec);
    let mut fb_dev = match fb::Framebuffer::open_with_title(cw, ch, titulo) {
        Ok(f) => f,
        Err(e) => {
            println!(
                "soso-web: superficie gráfica no disponible ({})",
                libsoso::errno_str(e)
            );
            return 1;
        }
    };
    let font_default = soso_web::rutas::fuente_por_defecto();
    let font_path = a.fuente.as_deref().unwrap_or(&font_default);
    let mut renderer = match render::Renderer::open(font_path) {
        Some(r) => r,
        None => {
            println!("soso-web: no se pudo cargar fuente {font_path}");
            return 1;
        }
    };
    let mut chrome_st = chrome::Chrome::new(initial_spec.clone());
    let fb_w = fb_dev.info.width as f32;
    let mut tab_cache: Vec<Option<TabPage>> = vec![None; chrome_st.tabs.len()];
    let mut page = gfx_navigate(&initial_spec, &mut chrome_st, fb_w, &fb_dev, false);
    tab_cache[0] = Some(page.clone());
    let mut input_st = input::InputState::default();
    let mut scroll = 0.0f32;
    let mut hover: Option<usize> = None;
    let mut dirty = true;
    let wm = fb_dev.es_wm();
    let fb_h = fb_dev.info.height as f32;
    let view_h = || fb_h - layout::CHROME_HEIGHT;

    loop {
        input_st.key = None;
        if wm {
            while input::poll_input_from_wm(&mut input_st, &mut fb_dev) {}
            if input_st.layout_dirty {
                chrome_st.active_tab_mut().scroll = scroll;
                if tab_cache.len() < chrome_st.tabs.len() {
                    tab_cache.resize(chrome_st.tabs.len(), None);
                }
                tab_cache[chrome_st.active] = Some(page.clone());
                let spec = chrome_st.active_tab().url_text.clone();
                page = gfx_navigate(&spec, &mut chrome_st, fb_w, &fb_dev, false);
                tab_cache[chrome_st.active] = Some(page.clone());
                scroll = chrome_st.active_tab().scroll;
                let max_s = (page.content_h - view_h()).max(0.0);
                scroll = scroll.min(max_s);
                input_st.layout_dirty = false;
                dirty = true;
            }
        } else {
            input::poll_input(&mut input_st);
        }

        if let Some(raw) = input_st.key {
            if raw == 0x01 {
                return 0;
            }
            if let Some(dec) = input_st.decode_key(raw) {
                if chrome_st.url_focus {
                    match dec {
                        DecodedKey::Enter => {
                            let u = normaliza_url_grafico(chrome_st.active_tab().url_text.trim());
                            chrome_st.url_focus = false;
                            page = gfx_navigate(&u, &mut chrome_st, fb_w, &fb_dev, true);
                            tab_cache[chrome_st.active] = Some(page.clone());
                            scroll = 0.0;
                            dirty = true;
                        }
                        DecodedKey::Escape => chrome_st.url_focus = false,
                        DecodedKey::Backspace => {
                            chrome_st.active_tab_mut().url_text.pop();
                            dirty = true;
                        }
                        DecodedKey::Char(c) => {
                            chrome_st.active_tab_mut().url_text.push(c as char);
                            dirty = true;
                        }
                        _ => {}
                    }
                } else if let Some(field_idx) = chrome_st.field_focus {
                    match dec {
                        DecodedKey::Enter => {
                            if let Some(form_idx) = page.doc.form_fields.iter().find(|f| f.field_idx == field_idx).map(|f| f.form_idx) {
                                page = gfx_submit_form(&page, form_idx, &mut chrome_st, fb_w, &fb_dev);
                                tab_cache[chrome_st.active] = Some(page.clone());
                                scroll = 0.0;
                                chrome_st.field_focus = None;
                                dirty = true;
                            }
                        }
                        DecodedKey::Escape => chrome_st.field_focus = None,
                        DecodedKey::Backspace => {
                            if field_idx < page.field_values.len() {
                                page.field_values[field_idx].pop();
                                dirty = true;
                            }
                        }
                        DecodedKey::Char(c) => {
                            if field_idx >= page.field_values.len() {
                                page.field_values.resize(field_idx + 1, String::new());
                            }
                            page.field_values[field_idx].push(c as char);
                            dirty = true;
                        }
                        _ => {}
                    }
                } else {
                    match dec {
                        DecodedKey::Char(b'q') | DecodedKey::Char(b'Q') => return 0,
                        DecodedKey::Char(b'b') | DecodedKey::Char(b'B') => {
                            if let Some(u) = chrome_st.active_tab_mut().go_back() {
                                page = gfx_navigate(&u, &mut chrome_st, fb_w, &fb_dev, false);
                                tab_cache[chrome_st.active] = Some(page.clone());
                                scroll = 0.0;
                                dirty = true;
                            }
                        }
                        DecodedKey::Char(b'f') | DecodedKey::Char(b'F') => {
                            if let Some(u) = chrome_st.active_tab_mut().go_forward() {
                                page = gfx_navigate(&u, &mut chrome_st, fb_w, &fb_dev, false);
                                tab_cache[chrome_st.active] = Some(page.clone());
                                scroll = 0.0;
                                dirty = true;
                            }
                        }
                        DecodedKey::Up => {
                            scroll = (scroll - 24.0).max(0.0);
                            dirty = true;
                        }
                        DecodedKey::Down => {
                            let max_s = (page.content_h - view_h()).max(0.0);
                            scroll = (scroll + 24.0).min(max_s);
                            dirty = true;
                        }
                        DecodedKey::Space => {
                            let max_s = (page.content_h - view_h()).max(0.0);
                            scroll = (scroll + view_h() * 0.85).min(max_s);
                            dirty = true;
                        }
                        _ => {}
                    }
                }
            }
        }

        hover = hover_enlace(&page.doc, &page.boxes, input_st.mouse_y, scroll);

        if input_st.click_left() {
            let hit = chrome::hit_chrome(input_st.mouse_x, input_st.mouse_y, fb_w);
            match hit {
                chrome::ChromeHit::NewTab => {
                    if chrome_st.new_tab() {
                        if tab_cache.len() < chrome_st.tabs.len() {
                            tab_cache.push(None);
                        }
                        page = gfx_navigate(
                            &soso_web::rutas::demo_tab_url(),
                            &mut chrome_st,
                            fb_w,
                            &fb_dev,
                            false,
                        );
                        tab_cache[chrome_st.active] = Some(page.clone());
                        scroll = 0.0;
                        dirty = true;
                    }
                }
                chrome::ChromeHit::CloseTab => {
                    chrome_st.active_tab_mut().scroll = scroll;
                    tab_cache[chrome_st.active] = Some(page.clone());
                    if chrome_st.close_active_tab() {
                        tab_cache.remove(chrome_st.active);
                        page = tab_cache[chrome_st.active]
                            .clone()
                            .unwrap_or_else(|| empty_page(fb_w));
                        scroll = chrome_st.active_tab().scroll;
                        dirty = true;
                    }
                }
                chrome::ChromeHit::Tab(i) => {
                    if i != chrome_st.active && i < chrome_st.tabs.len() {
                        chrome_st.active_tab_mut().scroll = scroll;
                        tab_cache[chrome_st.active] = Some(page.clone());
                        chrome_st.select_tab(i);
                        page = tab_cache[i]
                            .clone()
                            .unwrap_or_else(|| {
                                let u = chrome_st.active_tab().url_text.clone();
                                gfx_navigate(&u, &mut chrome_st, fb_w, &fb_dev, false)
                            });
                        scroll = chrome_st.active_tab().scroll;
                        dirty = true;
                    }
                }
                chrome::ChromeHit::BookmarkList => {
                    page = gfx_navigate(bookmarks::SCHEME, &mut chrome_st, fb_w, &fb_dev, true);
                    tab_cache[chrome_st.active] = Some(page.clone());
                    scroll = 0.0;
                    dirty = true;
                }
                chrome::ChromeHit::BookmarkStar => {
                    let url = chrome_st.active_tab().url_text.clone();
                    let title = page
                        .doc
                        .page_title
                        .clone()
                        .unwrap_or_else(|| url.clone());
                    let _ = bookmarks::toggle(&title, &url);
                    dirty = true;
                }
                chrome::ChromeHit::Back => {
                    chrome_st.url_focus = false;
                    chrome_st.field_focus = None;
                    if let Some(u) = chrome_st.active_tab_mut().go_back() {
                        page = gfx_navigate(&u, &mut chrome_st, fb_w, &fb_dev, false);
                        tab_cache[chrome_st.active] = Some(page.clone());
                        scroll = 0.0;
                        dirty = true;
                    }
                }
                chrome::ChromeHit::UrlField => {
                    chrome_st.url_focus = true;
                    chrome_st.field_focus = None;
                    chrome_st.active_tab_mut().error = None;
                    dirty = true;
                }
                chrome::ChromeHit::Go => {
                    chrome_st.url_focus = false;
                    chrome_st.field_focus = None;
                    let u = normaliza_url_grafico(chrome_st.active_tab().url_text.trim());
                    page = gfx_navigate(&u, &mut chrome_st, fb_w, &fb_dev, true);
                    tab_cache[chrome_st.active] = Some(page.clone());
                    scroll = 0.0;
                    dirty = true;
                }
                chrome::ChromeHit::None => {
                    chrome_st.url_focus = false;
                    if input_st.mouse_y as f32 >= layout::CHROME_HEIGHT {
                        match content_hit(&page, input_st.mouse_y, scroll) {
                            ContentHit::Link(href) => {
                                chrome_st.field_focus = None;
                                page = gfx_navigate(&href, &mut chrome_st, fb_w, &fb_dev, true);
                                tab_cache[chrome_st.active] = Some(page.clone());
                                scroll = 0.0;
                                dirty = true;
                            }
                            ContentHit::Field(idx) => {
                                chrome_st.field_focus = Some(idx);
                                dirty = true;
                            }
                            ContentHit::Submit(form_idx) => {
                                page = gfx_submit_form(
                                    &page,
                                    form_idx,
                                    &mut chrome_st,
                                    fb_w,
                                    &fb_dev,
                                );
                                tab_cache[chrome_st.active] = Some(page.clone());
                                scroll = 0.0;
                                chrome_st.field_focus = None;
                                dirty = true;
                            }
                            ContentHit::None => {
                                if scroll_en_barra(fb_w, input_st.mouse_x, input_st.mouse_y) {
                                    let max_s = (page.content_h - view_h()).max(0.0);
                                    let track = fb_dev.info.height as f32 - layout::CHROME_HEIGHT;
                                    let rel = (input_st.mouse_y as f32 - layout::CHROME_HEIGHT)
                                        / track.max(1.0);
                                    scroll = rel * max_s;
                                    dirty = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        if dirty || !wm {
            let bookmarked =
                bookmarks::contains_url(chrome_st.active_tab().url_text.as_str());
            renderer.draw_frame(
                &mut fb_dev,
                &page.doc,
                &page.boxes,
                scroll,
                page.content_h,
                hover,
                &chrome_st,
                &page.field_values,
                bookmarked,
            );
            let _ = fb_dev.present();
            dirty = false;
        }
        input_st.end_frame();
        libsoso::sys::sleep_ms(16);
    }
}

fn titulo_corto(spec: &str) -> &str {
    if spec.len() > 16 {
        &spec[..16]
    } else {
        spec
    }
}

fn normaliza_url_grafico(s: &str) -> String {
    if s.starts_with("https://")
        || s.starts_with("http://")
        || s.starts_with("file://")
        || s.starts_with("soso:")
    {
        s.to_string()
    } else if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("https://{s}")
    }
}

fn actualizar_titulo_wm(
    fb: &soso_web::gui::fb::Framebuffer,
    doc: &soso_web::gui::layout::Document,
    spec: &str,
) {
    let t = doc
        .page_title
        .as_deref()
        .unwrap_or(spec);
    let _ = fb.enviar_titulo_wm(t);
}

fn hover_enlace(
    doc: &soso_web::gui::layout::Document,
    boxes: &[soso_web::gui::layout::LayoutBox],
    my: i32,
    scroll: f32,
) -> Option<usize> {
    use soso_web::gui::layout;
    if (my as f32) < layout::CHROME_HEIGHT {
        return None;
    }
    let doc_y = my as f32 + scroll;
    for lb in boxes {
        if doc_y >= lb.y && doc_y < lb.y + lb.h {
            if let soso_web::gui::layout::Node::Link { idx, .. } = &doc.nodes[lb.node_idx] {
                return Some(*idx);
            }
        }
    }
    None
}

fn scroll_en_barra(fb_w: f32, mx: i32, my: i32) -> bool {
    use soso_web::gui::layout;
    (my as f32) >= layout::CHROME_HEIGHT && (mx as f32) >= fb_w - layout::SCROLLBAR_W
}
