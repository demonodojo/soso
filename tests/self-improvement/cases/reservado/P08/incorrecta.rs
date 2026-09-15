use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();

    let mut filas: Vec<(i64, usize, &str)> = Vec::new();
    for (i, linea) in entrada.lines().enumerate() {
        if linea.trim().is_empty() {
            continue;
        }
        let (clave, valor) = linea.split_once(' ').unwrap_or((linea, ""));
        filas.push((clave.trim().parse().unwrap(), i, valor));
    }

    // Incorrecta: desempata por orden de llegada invertido, que es lo que se
    // ve cuando alguien «ordena» con un criterio que no es estable.
    filas.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    for (clave, _, valor) in filas {
        println!("{clave} {valor}");
    }
}
