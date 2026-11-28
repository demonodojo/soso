//! Fondos de escritorio para soso-wm.
//!
//! El WM no decodifica nada: lee `/var/fondo/fondo.sfi`, que ya trae los
//! píxeles al tamaño de la pantalla y en el orden de bytes del framebuffer, y
//! los copia fila a fila. Este crate hace el trabajo caro (generar un fondo,
//! decodificar una imagen, escalarla) y escribe ese formato.
//!
//! Formato `.sfi` (little-endian):
//!
//! | off | tamaño | campo |
//! |-----|--------|-------|
//! | 0   | 4      | `SFND` |
//! | 4   | 4      | versión (1) |
//! | 8   | 4      | ancho |
//! | 12  | 4      | alto |
//! | 16  | 4      | formato del framebuffer (0 RGB, 1 BGR) |
//! | 20  | 4      | reservado (0) |
//! | 24  | 8      | generación (cambia con cada fondo nuevo) |
//! | 32  | w·h·4  | píxeles en orden de memoria del framebuffer, alfa 255 |
//! | …   | h·4    | color medio de cada fila (0x00RRGGBB) |
//!
//! El color medio por fila es lo que mezcla el WM bajo la barra y el menú
//! translúcidos: con un degradado era el color de la fila, con una imagen no
//! hay uno solo.

#![no_std]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use libm::{expf, sinf, sqrtf};

pub const MAGIA: &[u8; 4] = b"SFND";
pub const VERSION: u32 = 1;
pub const CABECERA: usize = 32;
/// Tope de lado de una imagen de entrada (8K).
pub const MAX_LADO: u32 = 8192;

/// Fondos generados, en el orden en que los recorre `fondo siguiente`.
pub const GENERADOS: [&str; 5] = ["aurora", "ocaso", "oceano", "bosque", "grafito"];

/// Descripción corta de cada fondo generado (para `fondo` sin argumentos).
pub fn descripcion(nombre: &str) -> &'static str {
    match nombre {
        "aurora" => "aurora boreal sobre montañas, de noche",
        "ocaso" => "puesta de sol con cordilleras",
        "oceano" => "olas en azul profundo",
        "bosque" => "colinas con niebla, claro",
        "grafito" => "oscuro y sobrio, con dos luces suaves",
        _ => "",
    }
}

/// Imagen RGB en memoria: un `u32` 0x00RRGGBB por píxel.
#[derive(Debug)]
pub struct Imagen {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
}

// ---- color ------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Rgb(f32, f32, f32);

impl Rgb {
    const fn de(r: u8, g: u8, b: u8) -> Rgb {
        Rgb(r as f32, g as f32, b as f32)
    }
    fn mezcla(self, o: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        Rgb(
            self.0 + (o.0 - self.0) * t,
            self.1 + (o.1 - self.1) * t,
            self.2 + (o.2 - self.2) * t,
        )
    }
    fn suma(self, o: Rgb, k: f32) -> Rgb {
        Rgb(self.0 + o.0 * k, self.1 + o.1 * k, self.2 + o.2 * k)
    }
    fn escala(self, k: f32) -> Rgb {
        Rgb(self.0 * k, self.1 * k, self.2 * k)
    }
    /// Empaqueta con un pequeño tramado (`ruido` en −1..1) contra las bandas.
    fn u32(self, ruido: f32) -> u32 {
        let c = |v: f32| (v + ruido).clamp(0.0, 255.0) as u32;
        (c(self.0) << 16) | (c(self.1) << 8) | c(self.2)
    }
}

/// Degradado por paradas (posición 0..1, color).
fn degradado(paradas: &[(f32, Rgb)], t: f32) -> Rgb {
    let mut prev = paradas[0];
    if t <= prev.0 {
        return prev.1;
    }
    for &p in &paradas[1..] {
        if t <= p.0 {
            let k = (t - prev.0) / (p.0 - prev.0).max(1e-6);
            // Lineal: con suavizado cada parada se ve como una banda plana.
            return prev.1.mezcla(p.1, k);
        }
        prev = p;
    }
    prev.1
}

fn suave(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Ruido determinista en −1..1 por píxel (tramado y estrellas).
fn hash(x: u32, y: u32, semilla: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ semilla.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 32767.5 - 1.0
}

/// Altura (0..1, desde arriba) de una cresta de montaña en `u` (0..1).
fn cresta(u: f32, base: f32, amp: f32, semilla: f32) -> f32 {
    let s = semilla;
    let r = 0.50 * sinf(u * 5.3 + s * 1.7)
        + 0.27 * sinf(u * 11.1 + s * 3.1)
        + 0.13 * sinf(u * 23.7 + s * 5.3)
        + 0.06 * sinf(u * 51.3 + s * 7.9);
    base - amp * r
}

// ---- fondos generados -------------------------------------------------------

/// Genera el fondo `nombre` a `w`×`h`, o `None` si no existe.
pub fn generar(nombre: &str, w: u32, h: u32) -> Option<Imagen> {
    let f: fn(u32, u32, u32, u32) -> Rgb = match nombre {
        "aurora" => aurora,
        "ocaso" => ocaso,
        "oceano" => oceano,
        "bosque" => bosque,
        "grafito" => grafito,
        _ => return None,
    };
    let mut px = vec![0u32; w as usize * h as usize];
    for y in 0..h {
        for x in 0..w {
            let c = f(x, y, w, h);
            px[(y * w + x) as usize] = c.u32(hash(x, y, 7) * 0.9);
        }
    }
    Some(Imagen { w, h, px })
}

fn uv(x: u32, y: u32, w: u32, h: u32) -> (f32, f32, f32) {
    // `u` en 0..aspecto para que las formas no se estiren en pantallas anchas.
    let asp = w as f32 / h.max(1) as f32;
    (x as f32 / w.max(1) as f32 * asp, y as f32 / h.max(1) as f32, asp)
}

fn vineta(c: Rgb, u: f32, v: f32, asp: f32, k: f32) -> Rgb {
    let dx = (u / asp - 0.5) * 1.2;
    let dy = v - 0.5;
    let d = sqrtf(dx * dx + dy * dy);
    c.escala(1.0 - k * suave((d - 0.35) / 0.5))
}

fn aurora(x: u32, y: u32, w: u32, h: u32) -> Rgb {
    let (u, v, asp) = uv(x, y, w, h);
    let mut c = degradado(
        &[
            (0.0, Rgb::de(5, 8, 24)),
            (0.45, Rgb::de(12, 22, 52)),
            (0.80, Rgb::de(26, 24, 66)),
            (1.0, Rgb::de(18, 14, 40)),
        ],
        v,
    );
    // Estrellas: pocas, tenues, sólo arriba.
    let e = hash(x, y, 31);
    if e > 0.9985 && v < 0.6 {
        let brillo = (e - 0.9985) / 0.0015;
        c = c.mezcla(Rgb::de(220, 230, 255), 0.35 + 0.5 * brillo);
    }
    // Cintas de aurora: brillo gaussiano alrededor de una curva, con estelas
    // largas hacia arriba.
    let cintas: [(f32, f32, f32, Rgb, f32); 3] = [
        (0.30, 0.9, 0.0, Rgb::de(40, 230, 170), 0.85),
        (0.38, 1.4, 2.1, Rgb::de(110, 240, 140), 0.55),
        (0.24, 0.7, 4.2, Rgb::de(150, 95, 235), 0.60),
    ];
    for &(base, f, fase, col, int) in &cintas {
        let yc = base + 0.07 * sinf(u * 2.3 * f + fase) + 0.03 * sinf(u * 7.1 + fase * 1.3);
        let d = v - yc;
        let grosor = if d < 0.0 { 0.12 } else { 0.035 };
        let mut i = expf(-(d * d) / (grosor * grosor));
        // Rayos finos de cortina, y la cinta se aviva y se apaga a lo largo.
        i *= 0.72 + 0.28 * sinf(u * 140.0 + 6.0 * sinf(u * 5.0 + fase));
        i *= 0.55 + 0.45 * sinf(u * 1.9 + fase * 1.7);
        c = c.suma(col, int * i * 0.7);
    }
    // Dos cordilleras oscuras abajo.
    let lejos = cresta(u, 0.80, 0.045, 1.0);
    let cerca = cresta(u + 3.7, 0.88, 0.05, 2.5);
    if v > cerca {
        c = Rgb::de(7, 9, 20).mezcla(Rgb::de(4, 5, 12), (v - cerca) * 4.0);
    } else if v > lejos {
        c = Rgb::de(17, 20, 42).mezcla(c, 0.15);
    }
    vineta(c, u, v, asp, 0.35)
}

fn ocaso(x: u32, y: u32, w: u32, h: u32) -> Rgb {
    let (u, v, asp) = uv(x, y, w, h);
    let mut c = degradado(
        &[
            (0.0, Rgb::de(38, 22, 78)),
            (0.30, Rgb::de(112, 46, 120)),
            (0.52, Rgb::de(214, 92, 108)),
            (0.66, Rgb::de(250, 160, 96)),
            (1.0, Rgb::de(250, 196, 120)),
        ],
        v,
    );
    // Sol y su halo.
    let (sx, sy) = (asp * 0.68, 0.60);
    let d = sqrtf((u - sx) * (u - sx) + (v - sy) * (v - sy));
    c = c.suma(Rgb::de(255, 200, 140), 0.55 * expf(-d * d / 0.02));
    if d < 0.07 {
        c = c.mezcla(Rgb::de(255, 236, 196), suave((0.07 - d) / 0.006));
    }
    // Cordilleras: de lejos (claras, teñidas de cielo) a cerca (oscuras).
    let capas: [(f32, f32, f32, Rgb); 4] = [
        (0.62, 0.050, 0.3, Rgb::de(176, 84, 120)),
        (0.70, 0.060, 1.9, Rgb::de(118, 52, 108)),
        (0.79, 0.055, 3.3, Rgb::de(70, 32, 86)),
        (0.89, 0.050, 4.6, Rgb::de(34, 16, 52)),
    ];
    for &(base, amp, sem, col) in &capas {
        let r = cresta(u, base, amp, sem);
        if v > r {
            // Bruma: la falda de cada capa se aclara un poco hacia arriba.
            c = col.mezcla(col.escala(0.82), (v - r) * 3.0);
        }
    }
    vineta(c, u, v, asp, 0.25)
}

fn oceano(x: u32, y: u32, w: u32, h: u32) -> Rgb {
    let (u, v, asp) = uv(x, y, w, h);
    let mut c = degradado(
        &[
            (0.0, Rgb::de(10, 34, 72)),
            (0.40, Rgb::de(22, 88, 140)),
            (0.55, Rgb::de(70, 160, 190)),
            (1.0, Rgb::de(70, 160, 190)),
        ],
        v,
    );
    // Luz difusa arriba a la izquierda.
    let d = sqrtf((u - asp * 0.25) * (u - asp * 0.25) + (v - 0.15) * (v - 0.15));
    c = c.suma(Rgb::de(120, 200, 230), 0.25 * expf(-d * d / 0.08));
    // Olas: bandas con borde sinusoidal, cada vez más profundas.
    let olas: [(f32, f32, f32, Rgb); 5] = [
        (0.52, 0.020, 0.0, Rgb::de(40, 140, 178)),
        (0.62, 0.025, 1.3, Rgb::de(26, 112, 158)),
        (0.72, 0.030, 2.9, Rgb::de(16, 84, 132)),
        (0.82, 0.032, 4.1, Rgb::de(10, 60, 106)),
        (0.92, 0.030, 5.7, Rgb::de(6, 40, 80)),
    ];
    for &(base, amp, fase, col) in &olas {
        let borde = base + amp * sinf(u * 6.0 + fase) + amp * 0.5 * sinf(u * 13.0 + fase * 2.0);
        if v > borde {
            let espuma = expf(-((v - borde) * (v - borde)) / 0.00004);
            c = col.mezcla(Rgb::de(190, 235, 245), espuma * 0.55);
        }
    }
    vineta(c, u, v, asp, 0.3)
}

fn bosque(x: u32, y: u32, w: u32, h: u32) -> Rgb {
    let (u, v, asp) = uv(x, y, w, h);
    let mut c = degradado(
        &[
            (0.0, Rgb::de(206, 226, 222)),
            (0.45, Rgb::de(176, 208, 198)),
            (1.0, Rgb::de(150, 190, 176)),
        ],
        v,
    );
    let (sx, sy) = (asp * 0.3, 0.28);
    let d = sqrtf((u - sx) * (u - sx) + (v - sy) * (v - sy));
    c = c.suma(Rgb::de(255, 250, 225), 0.35 * expf(-d * d / 0.03));
    let capas: [(f32, f32, f32, Rgb); 5] = [
        (0.50, 0.040, 0.7, Rgb::de(146, 184, 168)),
        (0.60, 0.050, 2.2, Rgb::de(108, 156, 136)),
        (0.70, 0.050, 3.4, Rgb::de(72, 124, 106)),
        (0.80, 0.045, 4.8, Rgb::de(44, 92, 80)),
        (0.90, 0.040, 6.1, Rgb::de(24, 60, 54)),
    ];
    let mut niebla_base = c;
    for &(base, amp, sem, col) in &capas {
        let r = cresta(u, base, amp, sem);
        if v > r {
            // Niebla: la falda se funde con lo que hay detrás.
            c = col.mezcla(niebla_base, 0.45 * expf(-(v - r) * 18.0));
            niebla_base = c;
        }
    }
    vineta(c, u, v, asp, 0.18)
}

fn grafito(x: u32, y: u32, w: u32, h: u32) -> Rgb {
    let (u, v, asp) = uv(x, y, w, h);
    let mut c = degradado(&[(0.0, Rgb::de(26, 28, 36)), (1.0, Rgb::de(18, 19, 25))], v);
    let luz = |c: Rgb, cx: f32, cy: f32, r: f32, col: Rgb, k: f32| {
        let d2 = (u - cx) * (u - cx) + (v - cy) * (v - cy);
        c.suma(col, k * expf(-d2 / (r * r)))
    };
    c = luz(c, asp * 0.18, 0.22, 0.35, Rgb::de(70, 110, 230), 0.35);
    c = luz(c, asp * 0.85, 0.85, 0.40, Rgb::de(170, 80, 210), 0.28);
    vineta(c, u, v, asp, 0.3)
}

// ---- imágenes ---------------------------------------------------------------

/// Decodifica PNG (8 bits RGB/RGBA) o PPM binario (P6, 8 bits).
pub fn decodificar(datos: &[u8]) -> Result<Imagen, &'static str> {
    if datos.starts_with(&[137, 80, 78, 71]) {
        let img = soso_web_core::png::decode_png_max(datos, MAX_LADO)
            .map_err(|_| "PNG no soportado (sólo 8 bits RGB o RGBA, sin entrelazar)")?;
        let px = img
            .rgba
            .chunks_exact(4)
            .map(|p| ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32)
            .collect();
        return Ok(Imagen { w: img.w, h: img.h, px });
    }
    if datos.starts_with(b"P6") {
        return ppm(datos);
    }
    if datos.starts_with(&[0xFF, 0xD8]) {
        return Err("JPEG no soportado todavía: conviértela a PNG");
    }
    Err("formato desconocido (vale PNG o PPM)")
}

fn ppm(d: &[u8]) -> Result<Imagen, &'static str> {
    // Cabecera: P6, ancho, alto, máximo, separados por blancos (y comentarios #).
    let mut i = 2;
    let mut campos = [0u32; 3];
    for campo in campos.iter_mut() {
        loop {
            while i < d.len() && d[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < d.len() && d[i] == b'#' {
                while i < d.len() && d[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            break;
        }
        let ini = i;
        while i < d.len() && d[i].is_ascii_digit() {
            i += 1;
        }
        if ini == i {
            return Err("PPM mal formado");
        }
        let mut v = 0u32;
        for &b in &d[ini..i] {
            v = v.saturating_mul(10).saturating_add((b - b'0') as u32);
        }
        *campo = v;
    }
    i += 1; // un solo blanco antes de los datos
    let [w, h, max] = campos;
    if w == 0 || h == 0 || w > MAX_LADO || h > MAX_LADO || max != 255 {
        return Err("PPM no soportado (8 bits, hasta 8192 px)");
    }
    let n = w as usize * h as usize;
    let datos = d.get(i..i + n * 3).ok_or("PPM truncado")?;
    let px = datos
        .chunks_exact(3)
        .map(|p| ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32)
        .collect();
    Ok(Imagen { w, h, px })
}

/// Escala `img` para cubrir `dw`×`dh` sin deformarla, recortando lo que sobre
/// por el centro (como `background-size: cover`). Bilineal.
pub fn cubrir(img: &Imagen, dw: u32, dh: u32) -> Imagen {
    let (sw, sh) = (img.w as f32, img.h as f32);
    let k = (dw as f32 / sw).max(dh as f32 / sh);
    let ox = (sw - dw as f32 / k) * 0.5;
    let oy = (sh - dh as f32 / k) * 0.5;
    let mut px = vec![0u32; dw as usize * dh as usize];
    let lee = |x: i32, y: i32| -> u32 {
        let x = x.clamp(0, img.w as i32 - 1) as usize;
        let y = y.clamp(0, img.h as i32 - 1) as usize;
        img.px[y * img.w as usize + x]
    };
    for y in 0..dh {
        let fy = oy + (y as f32 + 0.5) / k - 0.5;
        let y0 = libm::floorf(fy);
        let ty = fy - y0;
        for x in 0..dw {
            let fx = ox + (x as f32 + 0.5) / k - 0.5;
            let x0 = libm::floorf(fx);
            let tx = fx - x0;
            let (xi, yi) = (x0 as i32, y0 as i32);
            let (a, b, c, d) = (lee(xi, yi), lee(xi + 1, yi), lee(xi, yi + 1), lee(xi + 1, yi + 1));
            let canal = |s: u32| -> u32 {
                let g = |p: u32| ((p >> s) & 255) as f32;
                let top = g(a) + (g(b) - g(a)) * tx;
                let bot = g(c) + (g(d) - g(c)) * tx;
                ((top + (bot - top) * ty) + 0.5).clamp(0.0, 255.0) as u32
            };
            px[(y * dw + x) as usize] = (canal(16) << 16) | (canal(8) << 8) | canal(0);
        }
    }
    Imagen { w: dw, h: dh, px }
}

// ---- formato .sfi -----------------------------------------------------------

/// Serializa `img` en `.sfi` para un framebuffer de formato `fmt` (0 RGB, 1 BGR).
pub fn codificar(img: &Imagen, fmt: u32, generacion: u64) -> Vec<u8> {
    let (w, h) = (img.w as usize, img.h as usize);
    let mut out = Vec::with_capacity(CABECERA + w * h * 4 + h * 4);
    out.extend_from_slice(MAGIA);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&img.w.to_le_bytes());
    out.extend_from_slice(&img.h.to_le_bytes());
    out.extend_from_slice(&fmt.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&generacion.to_le_bytes());
    for &p in &img.px {
        let (r, g, b) = ((p >> 16) as u8, (p >> 8) as u8, p as u8);
        if fmt == 1 {
            out.extend_from_slice(&[b, g, r, 255]);
        } else {
            out.extend_from_slice(&[r, g, b, 255]);
        }
    }
    for fila in img.px.chunks_exact(w.max(1)) {
        let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
        for &p in fila {
            r += ((p >> 16) & 255) as u64;
            g += ((p >> 8) & 255) as u64;
            b += (p & 255) as u64;
        }
        let n = fila.len().max(1) as u64;
        let medio = (((r / n) as u32) << 16) | (((g / n) as u32) << 8) | (b / n) as u32;
        out.extend_from_slice(&medio.to_le_bytes());
    }
    out
}

/// (ancho, alto, formato, generación) de una cabecera `.sfi`.
pub fn leer_cabecera(d: &[u8]) -> Option<(u32, u32, u32, u64)> {
    if d.len() < CABECERA || &d[0..4] != MAGIA {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
    if u(4) != VERSION {
        return None;
    }
    let generacion = u64::from_le_bytes(d[24..32].try_into().ok()?);
    Some((u(8), u(12), u(16), generacion))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todos_los_generados_existen_y_tienen_el_tamano_pedido() {
        for n in GENERADOS {
            let img = generar(n, 64, 40).expect(n);
            assert_eq!((img.w, img.h, img.px.len()), (64, 40, 64 * 40), "{n}");
            assert!(!descripcion(n).is_empty());
        }
        assert!(generar("no-existe", 8, 8).is_none());
    }

    #[test]
    fn los_generados_no_son_planos() {
        // Un fondo de un solo color sería un fallo de las fórmulas (NaN → 0).
        for n in GENERADOS {
            let img = generar(n, 96, 60).unwrap();
            let distintos = img.px.iter().filter(|&&p| p != img.px[0]).count();
            assert!(distintos > img.px.len() / 2, "{n}: {distintos}");
        }
    }

    #[test]
    fn sfi_ida_y_vuelta_en_rgb_y_bgr() {
        let img = Imagen { w: 2, h: 1, px: vec![0x112233, 0x445566] };
        let rgb = codificar(&img, 0, 42);
        assert_eq!(leer_cabecera(&rgb), Some((2, 1, 0, 42)));
        assert_eq!(&rgb[32..40], &[0x11, 0x22, 0x33, 255, 0x44, 0x55, 0x66, 255]);
        // Media de la fila: (0x11+0x44)/2 = 0x2a, etc.
        assert_eq!(&rgb[40..44], &0x2a3b4cu32.to_le_bytes());
        assert_eq!(rgb.len(), CABECERA + 2 * 4 + 4);
        let bgr = codificar(&img, 1, 7);
        assert_eq!(&bgr[32..36], &[0x33, 0x22, 0x11, 255]);
        assert_eq!(leer_cabecera(&bgr[..20]), None);
    }

    #[test]
    fn cubrir_recorta_por_el_centro_sin_deformar() {
        // 4×2 con la mitad izquierda negra y la derecha blanca, a 2×2: se queda
        // con las dos columnas centrales (una negra, una blanca).
        let img = Imagen { w: 4, h: 2, px: vec![0, 0, 0xffffff, 0xffffff, 0, 0, 0xffffff, 0xffffff] };
        let c = cubrir(&img, 2, 2);
        assert_eq!((c.w, c.h), (2, 2));
        assert_eq!(c.px[0], 0);
        assert_eq!(c.px[1], 0xffffff);
        // Ampliar no se sale de la imagen.
        let g = cubrir(&img, 40, 20);
        assert_eq!(g.px.len(), 800);
    }

    #[test]
    fn ppm_con_comentario() {
        let mut d = b"P6\n# hola\n2 1\n255\n".to_vec();
        d.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let img = decodificar(&d).unwrap();
        assert_eq!((img.w, img.h), (2, 1));
        assert_eq!(img.px, vec![0x010203, 0x040506]);
        assert!(decodificar(&d[..d.len() - 1]).is_err());
        assert!(decodificar(&[0xFF, 0xD8, 0, 0]).unwrap_err().contains("JPEG"));
    }
}
