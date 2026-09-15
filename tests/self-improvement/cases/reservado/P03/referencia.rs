use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();

    let mut valores: Vec<&str> = Vec::new();
    for linea in entrada.lines() {
        if let Some((nombre, valor)) = linea.split_once(':') {
            if nombre.trim().eq_ignore_ascii_case("content-length") {
                valores.push(valor.trim());
            }
        }
    }

    if valores.is_empty() {
        println!("ERROR ausente");
        return;
    }
    if valores.iter().any(|v| *v != valores[0]) {
        println!("ERROR duplicado");
        return;
    }
    let v = valores[0];
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
        println!("ERROR invalido");
        return;
    }
    match v.parse::<u64>() {
        Ok(n) => println!("OK {n}"),
        Err(_) => println!("ERROR invalido"),
    }
}
