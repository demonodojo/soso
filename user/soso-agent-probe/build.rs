//! Compila el C de `c/ajeno.c` para el target de soso y lo enlaza en la sonda.
//!
//! Es el experimento de [N-005](../../docs/self-improvement/native/N-005.md):
//! `soso-http` ya enlaza el C de `ring`, pero eso llegó con una dependencia
//! pregenerada. Aquí se compila un fichero **nuevo**, escrito para la ocasión,
//! para que el caso mida la capacidad y no una herencia.
//!
//! Sólo para el target sin OS: en el host no se compila nada, así que
//! `cargo check` desde la raíz no necesita un cross-compiler.

fn main() {
    println!("cargo:rerun-if-changed=c/ajeno.c");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if os != "none" || arch != "x86_64" {
        return;
    }
    let target = std::env::var("TARGET").unwrap();
    cc::Build::new()
        .file("c/ajeno.c")
        .flag("-std=c11")
        // Sin protector de pila: lo pide el mismo motivo que en `soso-http`, y
        // además el canario de C fue un diagnóstico falso caro aquí (el page
        // fault en 0x28 que parecía un puntero nulo).
        .flag("-fno-stack-protector")
        .flag("-ffreestanding")
        .target(&target)
        .compile("soso_probe_ajeno");
}
