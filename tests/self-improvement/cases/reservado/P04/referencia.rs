use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    let mut lineas = entrada.lines();
    let tope: u32 = lineas.next().unwrap().trim().parse().unwrap();

    let mut acc: u32 = 0;
    for linea in lineas {
        let linea = linea.trim();
        if linea.is_empty() {
            continue;
        }
        let n: u32 = linea.parse().unwrap();
        // Saturar primero y recortar después: al revés, la suma envuelve y el
        // recorte se aplica a un valor que ya es mentira.
        acc = acc.saturating_add(n).min(tope);
    }
    println!("{acc}");
}
