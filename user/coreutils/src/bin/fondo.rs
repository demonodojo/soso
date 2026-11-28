//! `fondo` — el fondo de escritorio de soso-wm.
//!
//! Genera o convierte la imagen al tamaño y formato de la pantalla y la deja en
//! `/var/fondo/fondo.sfi` (formato en `crates/soso-fondo`). El WM vigila ese
//! fichero y cambia el fondo solo, sin reiniciarse.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use libsoso::{abi, errno_str, println, sys};
use soso_fondo::{GENERADOS, codificar, cubrir, decodificar, descripcion, generar};

libsoso::entry!(main);

const DIR: &str = "/var/fondo";
const SFI: &str = "/var/fondo/fondo.sfi";
const TMP: &str = "/var/fondo/fondo.tmp";
const ORIGEN: &str = "/var/fondo/origen";
/// Imágenes que recorre `fondo siguiente` además de las generadas.
const IMAGENES: &str = "/share/fondos";
const PREDETERMINADO: &str = "aurora";

fn uso() {
    println!("uso: fondo                 lista los fondos y dice cuál está puesto");
    println!("     fondo NOMBRE          pone uno de los incluidos");
    println!("     fondo RUTA            pone una imagen (PNG de 8 bits o PPM)");
    println!("     fondo siguiente       pasa al siguiente (incluidos e imágenes de {IMAGENES})");
    println!("     fondo ninguno         quita el fondo (degradado del tema)");
}

fn main(args: &[String]) -> u8 {
    match args.first().map(|s| s.as_str()) {
        None | Some("") => listar(),
        Some("-h") | Some("--help") | Some("ayuda") => {
            uso();
            0
        }
        Some("siguiente") => {
            let lista = candidatos();
            let actual = leer_origen().unwrap_or_default();
            let i = lista.iter().position(|c| *c == actual).map_or(0, |i| (i + 1) % lista.len());
            poner(&lista[i], true)
        }
        Some("ninguno") => {
            let _ = sys::unlink(SFI);
            escribir_origen("ninguno");
            println!("fondo: quitado; el escritorio usa el degradado del tema");
            0
        }
        // Lo usa el WM: al arrancar si no hay fondo, y si la pantalla cambió de tamaño.
        Some("--inicial") => {
            if leer_origen().is_some() {
                return 0;
            }
            poner(PREDETERMINADO, false)
        }
        Some("--regenerar") => match leer_origen() {
            Some(o) if o == "ninguno" => 0,
            Some(o) => poner(&o, false),
            None => poner(PREDETERMINADO, false),
        },
        Some(otro) => poner(otro, true),
    }
}

fn listar() -> u8 {
    let actual = leer_origen().unwrap_or_else(|| String::from("ninguno"));
    println!("Fondos incluidos:");
    for n in GENERADOS {
        let marca = if actual == n { "*" } else { " " };
        println!(" {marca} {n:<10} {}", descripcion(n));
    }
    let imgs = imagenes();
    if !imgs.is_empty() {
        println!("Imágenes en {IMAGENES}:");
        for p in &imgs {
            let marca = if actual == *p { "*" } else { " " };
            println!(" {marca} {p}");
        }
    }
    if !GENERADOS.contains(&actual.as_str()) && !imgs.contains(&actual) {
        println!("Puesto: {actual}");
    }
    println!("Cambia con `fondo NOMBRE`, `fondo /ruta/imagen.png` o desde el menú del escritorio.");
    0
}

fn candidatos() -> Vec<String> {
    let mut v: Vec<String> = GENERADOS.iter().map(|s| String::from(*s)).collect();
    v.extend(imagenes());
    v
}

/// PNG y PPM de `/share/fondos`, por nombre.
fn imagenes() -> Vec<String> {
    let mut out = Vec::new();
    let fd = sys::open(IMAGENES, abi::O_RDONLY);
    if fd < 0 {
        return out;
    }
    let mut ents = [abi::Dirent::default(); 16];
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n <= 0 {
            break;
        }
        for d in &ents[..n as usize / abi::DIRENT_SIZE] {
            let Ok(nombre) = core::str::from_utf8(d.name_bytes()) else { continue };
            let min = nombre.to_ascii_lowercase();
            if d.file_type == abi::FT_FILE && (min.ends_with(".png") || min.ends_with(".ppm")) {
                out.push(format!("{IMAGENES}/{nombre}"));
            }
        }
    }
    let _ = sys::close(fd as u64);
    out.sort();
    out
}

/// (ancho, alto, formato) del framebuffer; 1920×1080 RGB si no hay pantalla.
fn pantalla() -> (u32, u32, u32) {
    let mut fb = abi::FbInfo::default();
    if sys::fb_info(&mut fb) == 0 && fb.present != 0 && fb.width > 0 && fb.height > 0 {
        (fb.width, fb.height, fb.pixel_format as u32)
    } else {
        (1920, 1080, 0)
    }
}

fn poner(origen: &str, hablar: bool) -> u8 {
    let (w, h, fmt) = pantalla();
    let (img, guardar) = if let Some(img) = generar(origen, w, h) {
        (img, String::from(origen))
    } else {
        let ruta = absoluta(origen);
        let datos = match leer(&ruta) {
            Ok(d) => d,
            Err(e) => {
                println!("fondo: {origen}: {e}");
                if !origen.contains('/') && !origen.contains('.') {
                    println!("fondo: los incluidos son: {}", GENERADOS.join(", "));
                }
                return 1;
            }
        };
        match decodificar(&datos) {
            Ok(img) => (cubrir(&img, w, h), ruta),
            Err(e) => {
                println!("fondo: {origen}: {e}");
                return 1;
            }
        }
    };
    let bytes = codificar(&img, fmt, generacion());
    if let Err(e) = escribir_atomico(&bytes) {
        println!("fondo: no pude escribir {SFI}: {e}");
        return 1;
    }
    escribir_origen(&guardar);
    if hablar {
        println!("fondo: puesto {guardar} ({w}×{h})");
    }
    0
}

/// Cambia en cada fondo nuevo: el WM compara este número para recargar.
fn generacion() -> u64 {
    let mut t = abi::Timespec::default();
    let _ = sys::clock_gettime(abi::CLOCK_REALTIME, &mut t);
    (t.tv_sec as u64).wrapping_mul(1_000_000_007) ^ (t.tv_nsec as u64) ^ (sys::uptime_ms() as u64)
}

fn absoluta(p: &str) -> String {
    if p.starts_with('/') {
        return String::from(p);
    }
    let mut buf = [0u8; 256];
    let n = sys::getcwd(&mut buf);
    let cwd = if n > 0 {
        let fin = buf.iter().position(|&b| b == 0).unwrap_or(n as usize).min(n as usize);
        core::str::from_utf8(&buf[..fin]).unwrap_or("/")
    } else {
        "/"
    };
    if cwd.ends_with('/') { format!("{cwd}{p}") } else { format!("{cwd}/{p}") }
}

fn leer(ruta: &str) -> Result<Vec<u8>, &'static str> {
    let mut st = abi::Stat::default();
    if sys::stat(ruta, &mut st) < 0 {
        return Err("no existe");
    }
    if st.file_type != abi::FT_FILE {
        return Err("no es un fichero");
    }
    let fd = sys::open(ruta, abi::O_RDONLY);
    if fd < 0 {
        return Err(errno_str(fd));
    }
    let mut d = vec![0u8; st.size as usize];
    let r = sys::read_exact(fd as u64, &mut d);
    let _ = sys::close(fd as u64);
    r.map_err(errno_str)?;
    Ok(d)
}

/// Se escribe aparte y se renombra: el WM nunca lee un fondo a medias.
fn escribir_atomico(bytes: &[u8]) -> Result<(), &'static str> {
    let _ = sys::mkdir(DIR);
    let fd = sys::open(TMP, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
    if fd < 0 {
        return Err(errno_str(fd));
    }
    let w = sys::write_all(fd as u64, bytes);
    let c = sys::close(fd as u64);
    w.map_err(errno_str)?;
    if c < 0 {
        return Err(errno_str(c));
    }
    let r = sys::rename(TMP, SFI);
    if r < 0 {
        return Err(errno_str(r));
    }
    Ok(())
}

fn leer_origen() -> Option<String> {
    let d = leer(ORIGEN).ok()?;
    let s = core::str::from_utf8(&d).ok()?.trim();
    if s.is_empty() { None } else { Some(String::from(s)) }
}

fn escribir_origen(s: &str) {
    let _ = sys::mkdir(DIR);
    let fd = sys::open(ORIGEN, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
    if fd >= 0 {
        let _ = sys::write_all(fd as u64, s.as_bytes());
        let _ = sys::close(fd as u64);
    }
}
