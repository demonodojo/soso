use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    let texto = entrada.strip_suffix('\n').unwrap_or(&entrada);

    let mut salida = String::with_capacity(texto.len() + 2);
    salida.push('"');
    for c in texto.chars() {
        match c {
            '"' => salida.push_str("\\\""),
            '\\' => salida.push_str("\\\\"),
            '\u{8}' => salida.push_str("\\b"),
            '\u{c}' => salida.push_str("\\f"),
            '\n' => salida.push_str("\\n"),
            '\r' => salida.push_str("\\r"),
            '\t' => salida.push_str("\\t"),
            c if (c as u32) < 0x20 => salida.push_str(&format!("\\u{:04x}", c as u32)),
            c => salida.push(c),
        }
    }
    salida.push('"');
    println!("{salida}");
}
