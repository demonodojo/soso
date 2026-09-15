use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();

    // Incorrecta: se queda con la primera aparición y no mira si hay otra con
    // un valor distinto, que es justo la ambigüedad de longitud que hay que
    // rechazar.
    for linea in entrada.lines() {
        if let Some((nombre, valor)) = linea.split_once(':') {
            if nombre.trim().eq_ignore_ascii_case("content-length") {
                let v = valor.trim();
                if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
                    println!("ERROR invalido");
                } else {
                    println!("OK {v}");
                }
                return;
            }
        }
    }
    println!("ERROR ausente");
}
