fn main() {
    if std::env::var("CARGO_FEATURE_LXDDE").is_ok() {
        let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let root = manifest_dir.parent().unwrap();
        let lib_dir = root.join("target/lxdde");
        println!("cargo:rerun-if-changed={}", root.join("lxdde").display());
        println!("cargo:rerun-if-env-changed=SOSO_LXDDE");
        if lib_dir.join("liblxdde.a").exists() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
            println!("cargo:rustc-link-lib=static=lxdde");
        } else {
            println!(
                "cargo:warning=lxdde: falta target/lxdde/liblxdde.a — ejecuta cargo xtask lx-build"
            );
        }
    }
}
