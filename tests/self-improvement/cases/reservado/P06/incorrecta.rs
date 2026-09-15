use std::io::Read;

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();

    for ruta in entrada.lines() {
        let ruta = ruta.trim();
        if ruta.is_empty() {
            continue;
        }
        // Incorrecta: cualquier fallo se traduce a «no existe», así que un
        // directorio o un permiso denegado desaparecen del diagnóstico.
        match std::fs::read(ruta) {
            Ok(datos) => println!("LEIDO {}", datos.len()),
            Err(_) => println!("NOEXISTE"),
        }
    }
}
