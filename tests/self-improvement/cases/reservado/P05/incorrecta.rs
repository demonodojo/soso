use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    let mut lineas = entrada.lines();
    let _raiz = lineas.next().unwrap_or("");
    let ruta = lineas.next().unwrap_or("").trim_end_matches('\r');

    // Incorrecta: busca la subcadena ".." en vez de mirar segmentos, así que
    // rechaza nombres legítimos como `a..b` y no normaliza nada.
    if ruta.starts_with('/') || ruta.contains("..") {
        println!("RECHAZADA");
        return;
    }
    let pila: Vec<&str> = ruta.split('/').filter(|s| !s.is_empty() && *s != ".").collect();
    if pila.is_empty() {
        println!("OK .");
    } else {
        println!("OK {}", pila.join("/"));
    }
}
