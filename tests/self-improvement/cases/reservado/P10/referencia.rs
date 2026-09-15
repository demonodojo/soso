use std::io::Read;

struct Recurso;

impl Drop for Recurso {
    fn drop(&mut self) {
        println!("liberado");
    }
}

fn trabajo(orden: &str) -> Result<(), ()> {
    let _guarda = Recurso;
    if orden == "fallo" {
        // Salir por aquí también destruye la guarda: no hace falta repetir la
        // liberación en cada rama.
        return Err(());
    }
    println!("hecho");
    Ok(())
}

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    // `process::exit` se llama cuando la guarda ya se ha destruido.
    if trabajo(entrada.trim()).is_err() {
        std::process::exit(1);
    }
}
