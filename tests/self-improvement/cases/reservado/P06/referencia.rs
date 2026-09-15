use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();

    for ruta in entrada.lines() {
        let ruta = ruta.trim();
        if ruta.is_empty() {
            continue;
        }
        match std::fs::read(ruta) {
            Ok(datos) => println!("LEIDO {}", datos.len()),
            // El tipo del error es lo que separa «no está» de «está pero no
            // se puede leer»; el resto se informa como error, no como ausencia.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("NOEXISTE"),
            Err(_) => println!("ERROR"),
        }
    }
}
