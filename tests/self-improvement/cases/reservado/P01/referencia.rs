use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    let (cabecera, resto) = entrada.split_once('\n').unwrap_or((entrada.as_str(), ""));
    let limite: usize = cabecera.trim().parse().unwrap();
    let texto = resto.strip_suffix('\n').unwrap_or(resto);

    // El corte cae siempre en frontera de carácter: se avanza mientras el
    // carácter entero quepa dentro del límite.
    let mut corte = 0;
    for (i, c) in texto.char_indices() {
        if i + c.len_utf8() > limite {
            break;
        }
        corte = i + c.len_utf8();
    }
    println!("{}", &texto[..corte]);
}
