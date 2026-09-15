use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();

    let mut filas: Vec<(i64, &str)> = Vec::new();
    for linea in entrada.lines() {
        if linea.trim().is_empty() {
            continue;
        }
        let (clave, valor) = linea.split_once(' ').unwrap_or((linea, ""));
        filas.push((clave.trim().parse().unwrap(), valor));
    }

    // `sort_by_key` es estable: los empates salen en el orden en que entraron.
    filas.sort_by_key(|(clave, _)| *clave);
    for (clave, valor) in filas {
        println!("{clave} {valor}");
    }
}
