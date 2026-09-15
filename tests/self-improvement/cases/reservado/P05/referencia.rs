use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    let mut lineas = entrada.lines();
    let _raiz = lineas.next().unwrap_or("");
    let ruta = lineas.next().unwrap_or("").trim_end_matches('\r');

    if ruta.starts_with('/') {
        println!("RECHAZADA");
        return;
    }
    // Se resuelve por segmentos: `..` solo sube si es el segmento entero, y
    // subir por encima de la raíz es rechazo, no un recorte silencioso.
    let mut pila: Vec<&str> = Vec::new();
    for seg in ruta.split('/') {
        match seg {
            "" | "." => continue,
            ".." => {
                if pila.pop().is_none() {
                    println!("RECHAZADA");
                    return;
                }
            }
            otro => pila.push(otro),
        }
    }
    if pila.is_empty() {
        println!("OK .");
    } else {
        println!("OK {}", pila.join("/"));
    }
}
