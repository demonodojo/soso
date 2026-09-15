use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    let mut lineas = entrada.lines();

    let cabecera = lineas.next().unwrap();
    let mut piezas = cabecera.split_whitespace();
    let a: i64 = piezas.next().unwrap().parse().unwrap();
    let b: i64 = piezas.next().unwrap().parse().unwrap();

    for linea in lineas {
        let linea = linea.trim();
        if linea.is_empty() {
            continue;
        }
        let v: i64 = linea.parse().unwrap();
        // Semiabierto: `a` entra, `b` no. Un rango vacío o invertido no admite
        // a nadie, y esa comparación ya lo cubre sin caso especial.
        if v >= a && v < b {
            println!("SI");
        } else {
            println!("NO");
        }
    }
}
