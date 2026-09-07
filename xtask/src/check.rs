//! `cargo xtask check` — comprobaciones locales reproducibles (host + builds).

use std::path::Path;
use std::process::{Command, exit};
use std::sync::{Arc, Mutex};

pub fn run() {
    let root = crate::project_root();
    let fallos = Arc::new(Mutex::new(0u32));

    println!("check: tests host…");
    run_host(&root, &fallos);

    println!("check: builds bare-metal…");
    crate::build_user();
    let img = crate::build_image();
    if !img.exists() {
        eprintln!("check: kernel/imagen no generada");
        *fallos.lock().unwrap() += 1;
    }
    if crate::build_boot_shim(&root).is_none() {
        eprintln!("check: aviso — boot-shim no compiló (¿target uefi?)");
    }

    println!("check: hostchecks opcionales…");
    run_script_if_present(
        &root,
        "scripts/l6-iwl-fw-hostcheck.sh",
        &fallos,
        "iwl hostcheck",
    );
    run_script_if_present(
        &root,
        "scripts/l6-g3-gsp-hostcheck.sh",
        &fallos,
        "GSP hostcheck",
    );

    println!("check: hw-matrix…");
    run_hw_matrix(&root, &fallos);

    let n = *fallos.lock().unwrap();
    if n == 0 {
        println!("\n✅ cargo xtask check: TODO OK");
        exit(0);
    }
    println!("\n❌ cargo xtask check: {n} fallo(s)");
    exit(1);
}

fn run_host(root: &Path, fallos: &Arc<Mutex<u32>>) {
    let batches: &[(&str, &[&str], bool)] = &[
        (
            "sosofs+sosomfs+soso-llm-core",
            &["sosofs", "sosomfs", "soso-llm-core"],
            true,
        ),
        (
            "gptdisk+soso-http+soso-web-core+sosomodel+convert-gguf+cuda-proxy",
            &[
                "gptdisk",
                "soso-http",
                "soso-web-core",
                "sosomodel",
                "convert-gguf",
                "cuda-proxy",
            ],
            false,
        ),
        ("soso-update-core", &["soso-update-core"], true),
        ("soso-audio+gguf2som", &["soso-audio", "gguf2som"], true),
    ];
    for (nombre, pkgs, std) in batches {
        if !cargo_test(root, pkgs, *std) {
            eprintln!("check: falló host ({nombre})");
            *fallos.lock().unwrap() += 1;
        }
    }
}

fn cargo_test(root: &Path, pkgs: &[&str], con_std: bool) -> bool {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root).args(["test", "-q"]);
    for pkg in pkgs {
        cmd.args(["-p", pkg]);
    }
    if con_std {
        cmd.args(["--features", "std"]);
    }
    if pkgs == &["soso-http"] {
        cmd.args(["--", "--test-threads=1"]);
    }
    match cmd.status() {
        Ok(st) => st.success(),
        Err(e) => {
            eprintln!("check: cargo test: {e}");
            false
        }
    }
}

fn run_script_if_present(root: &Path, rel: &str, fallos: &Arc<Mutex<u32>>, nombre: &str) {
    let script = root.join(rel);
    if !script.is_file() {
        println!("check: omitido {nombre} (sin {rel})");
        return;
    }
    let ok = Command::new("bash")
        .arg(&script)
        .current_dir(root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        println!("check: OK  {nombre}");
    } else {
        eprintln!("check: FALLO {nombre}");
        *fallos.lock().unwrap() += 1;
    }
}

fn run_hw_matrix(root: &Path, fallos: &Arc<Mutex<u32>>) {
    let json = root.join("docs/hw-matrix.json");
    if !json.is_file() {
        println!("check: hw-matrix.json ausente — ejecuta `cargo xtask hw-matrix init`");
    }
    let ok = Command::new("cargo")
        .current_dir(root.join("xtask"))
        .args(["test", "-q", "hw_matrix::", "--", "--nocapture"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        println!("check: OK  hw-matrix parser");
    } else {
        eprintln!("check: FALLO hw-matrix tests");
        *fallos.lock().unwrap() += 1;
    }
}
