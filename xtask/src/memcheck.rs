//! `cargo xtask memcheck` — tests de host con AddressSanitizer + LeakSanitizer.

use std::path::PathBuf;
use std::process::{Command, exit};

use crate::check::{configure_host_cargo_test, host_test_batches};

pub const ASAN_TARGET: &str = "x86_64-unknown-linux-gnuasan";

pub fn run(filter_pkgs: &[String]) {
    let root = crate::project_root();
    ensure_asan_target();
    let supp = root.join("xtask/lsan.supp");
    if !supp.is_file() {
        eprintln!("memcheck: falta {}", supp.display());
        exit(1);
    }
    let lsan = format!("suppressions={}", supp.display());
    let runs = plan_runs(filter_pkgs);
    if runs.is_empty() {
        eprintln!("memcheck: ningún paquete coincide con el filtro");
        exit(1);
    }
    for (label, pkgs, con_std) in runs {
        println!("memcheck: {label}…");
        let mut cmd = Command::new("cargo");
        configure_host_cargo_test(&mut cmd, &root, &pkgs, con_std, Some(ASAN_TARGET));
        cmd.env("ASAN_OPTIONS", "halt_on_error=1:abort_on_error=1");
        cmd.env("LSAN_OPTIONS", &lsan);
        if let Some(sym) = llvm_symbolizer_path() {
            cmd.env("ASAN_SYMBOLIZER_PATH", sym);
        }
        match cmd.status() {
            Ok(st) if st.success() => {}
            Ok(_) => {
                eprintln!("memcheck: falló ({label})");
                exit(1);
            }
            Err(e) => {
                eprintln!("memcheck: cargo test: {e}");
                exit(1);
            }
        }
    }
    println!("\n✅ cargo xtask memcheck: TODO OK");
}

/// LSan compara supresiones con nombres de función; hace falta un symbolizer.
fn llvm_symbolizer_path() -> Option<PathBuf> {
    for candidate in [
        "/usr/bin/llvm-symbolizer-18",
        "/usr/bin/llvm-symbolizer-19",
        "/usr/bin/llvm-symbolizer",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn ensure_asan_target() {
    let list = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .expect("rustup target list");
    let installed = String::from_utf8_lossy(&list.stdout);
    if installed.lines().any(|l| l.trim() == ASAN_TARGET) {
        return;
    }
    println!("memcheck: instalando target {ASAN_TARGET}…");
    let st = Command::new("rustup")
        .args(["target", "add", ASAN_TARGET])
        .status()
        .expect("rustup target add");
    if !st.success() {
        eprintln!("memcheck: no se pudo instalar {ASAN_TARGET}");
        exit(1);
    }
}

/// Sin filtro: mismos lotes que check + xtask. Con filtro: un `cargo test` por paquete.
fn plan_runs(filter_pkgs: &[String]) -> Vec<(String, Vec<&'static str>, bool)> {
    if filter_pkgs.is_empty() {
        let mut out: Vec<(String, Vec<&'static str>, bool)> = host_test_batches()
            .iter()
            .map(|b| {
                (
                    b.label.to_string(),
                    b.pkgs.to_vec(),
                    b.std,
                )
            })
            .collect();
        out.push(("xtask".to_string(), vec!["xtask"], false));
        return out;
    }
    let mut out = Vec::new();
    for name in filter_pkgs {
        let Some((pkgs, con_std)) = batch_for_pkg(name) else {
            eprintln!("memcheck: paquete desconocido o fuera de los lotes host: {name}");
            continue;
        };
        out.push((name.clone(), pkgs, con_std));
    }
    out
}

fn batch_for_pkg(name: &str) -> Option<(Vec<&'static str>, bool)> {
    for batch in host_test_batches() {
        for &pkg in batch.pkgs {
            if pkg == name {
                return Some((vec![pkg], batch.std));
            }
        }
    }
    if name == "xtask" {
        return Some((vec!["xtask"], false));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{configure_host_cargo_test, host_test_single_thread};
    use std::process::Command;

    fn host_pkg_wants_std(pkg: &str) -> bool {
        host_test_batches()
            .iter()
            .find(|b| b.pkgs.contains(&pkg))
            .is_some_and(|b| b.std)
    }

    #[test]
    fn filtro_por_paquete_un_solo_hilo_en_http() {
        let (pkgs, _) = batch_for_pkg("soso-http").unwrap();
        assert_eq!(pkgs, vec!["soso-http"]);
        assert!(host_test_single_thread(&["soso-http"]));
    }

    #[test]
    fn sosofs_lleva_std() {
        assert!(host_pkg_wants_std("sosofs"));
        assert!(!host_pkg_wants_std("gptdisk"));
    }

    #[test]
    fn configure_anade_target_asan() {
        let root = crate::project_root();
        let mut cmd = Command::new("cargo");
        configure_host_cargo_test(
            &mut cmd,
            &root,
            &["sosofs"],
            true,
            Some(ASAN_TARGET),
        );
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.windows(2).any(|w| w[0] == "--target" && w[1] == ASAN_TARGET));
        assert!(args.contains(&"--features".to_string()));
        assert!(args.contains(&"std".to_string()));
    }

    #[test]
    fn plan_sin_filtro_incluye_xtask() {
        let runs = plan_runs(&[]);
        assert!(runs.iter().any(|(l, _, _)| l == "xtask"));
        assert_eq!(runs.len(), host_test_batches().len() + 1);
    }
}
