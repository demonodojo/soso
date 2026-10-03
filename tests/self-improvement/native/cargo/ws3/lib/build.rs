use std::{env, fs, path::Path};

fn main() {
    let out = env::var("OUT_DIR").expect("OUT_DIR");
    fs::write(Path::new(&out).join("gen.rs"), "pub const GEN: i32 = 11;").unwrap();
    println!("cargo:rustc-env=T41_ETIQUETA=build-rs-ok");
    println!("cargo:rerun-if-changed=build.rs");
}
