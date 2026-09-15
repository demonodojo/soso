use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    let (cabecera, resto) = entrada.split_once('\n').unwrap_or((entrada.as_str(), ""));
    let limite: usize = cabecera.trim().parse().unwrap();
    let texto = resto.strip_suffix('\n').unwrap_or(resto);

    // Incorrecta: corta por bytes y tapa el destrozo con el carácter de
    // reemplazo, así que devuelve un carácter que el texto no tenía.
    let hasta = limite.min(texto.len());
    println!("{}", String::from_utf8_lossy(&texto.as_bytes()[..hasta]));
}
