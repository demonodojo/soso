//! Vuelca los fondos generados como PPM para mirarlos en el host:
//! `cargo run -p soso-fondo --example vista -- DIR [ANCHO ALTO]`.
use std::io::Write;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = a.get(1).expect("uso: vista DIR [ANCHO ALTO]");
    let w = a.get(2).map_or(1280, |s| s.parse().unwrap());
    let h = a.get(3).map_or(800, |s| s.parse().unwrap());
    for n in soso_fondo::GENERADOS {
        let img = soso_fondo::generar(n, w, h).unwrap();
        let mut f = std::fs::File::create(format!("{dir}/{n}.ppm")).unwrap();
        write!(f, "P6\n{w} {h}\n255\n").unwrap();
        let px: Vec<u8> = img.px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
        f.write_all(&px).unwrap();
    }
}
