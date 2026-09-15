use std::io::Read;

struct Recurso;

impl Drop for Recurso {
    fn drop(&mut self) {
        println!("liberado");
    }
}

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    let _guarda = Recurso;

    // Incorrecta: sale con `process::exit` desde dentro del ámbito, y eso no
    // ejecuta ningún Drop: el recurso se queda sin liberar.
    if entrada.trim() == "fallo" {
        std::process::exit(1);
    }
    println!("hecho");
}
