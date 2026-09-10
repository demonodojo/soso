//! `cargo xtask check` — comprobaciones locales reproducibles (host + builds).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio, exit};
use std::sync::{Arc, Mutex};

/// Cada paso deja su salida aquí, pase o falle: un fallo en CI se diagnostica
/// con el log, no repitiendo el paso a ciegas.
const ARTEFACTOS: &str = "target/check-artefactos";

pub fn run() {
    let root = crate::project_root();
    let fallos = Arc::new(Mutex::new(0u32));
    let arte = root.join(ARTEFACTOS);
    let _ = std::fs::remove_dir_all(&arte);
    std::fs::create_dir_all(&arte).expect("crear target/check-artefactos");

    println!("check: tests host…");
    run_host(&root, &fallos);
    if let Err(e) = crate::rtc_host::run_rtc_host_tests() {
        eprintln!("check: falló rtc host ({e})");
        *fallos.lock().unwrap() += 1;
    }

    println!("check: builds bare-metal…");
    crate::build_user();
    let img = crate::build_image();
    if !img.exists() {
        eprintln!("check: kernel/imagen no generada");
        *fallos.lock().unwrap() += 1;
    }
    if crate::build_boot_shim(&root).is_none() {
        eprintln!("check: boot-shim no compiló (¿target x86_64-unknown-uefi?)");
        *fallos.lock().unwrap() += 1;
    }

    // Los hostchecks obligatorios salen del perfil compilado: si el perfil
    // trae un port, su banco no se puede omitir por falta del script.
    let perfil = crate::drivers::profile_from_env_or_args();
    let ports = crate::drivers::lx_ports_for_build(&perfil);
    let minimo = check_profile_minimo();
    let requeridos = hostchecks_requeridos(&ports, minimo);
    println!(
        "check: hostchecks del perfil [{}]{}…",
        if ports.is_empty() {
            "sin lxdde".to_string()
        } else {
            ports.join(",")
        },
        if minimo { " (perfil mínimo)" } else { "" }
    );
    for (nombre, rel, port) in HOSTCHECKS {
        let obligatorio = requeridos.iter().any(|r| r == nombre);
        run_script(&root, rel, &fallos, nombre, obligatorio, &arte);
        if obligatorio {
            comprobar_firmware(&root, port, &perfil, &fallos);
        }
    }

    println!("check: tests xtask (parsers y modelos)…");
    run_xtask_tests(&root, &fallos, &arte);

    let n = *fallos.lock().unwrap();
    println!("check: artefactos en {}", arte.display());
    if n == 0 {
        println!("\n✅ cargo xtask check: TODO OK");
        exit(0);
    }
    println!("\n❌ cargo xtask check: {n} fallo(s)");
    exit(1);
}

/// (nombre, script, port lxdde que lo hace obligatorio)
const HOSTCHECKS: &[(&str, &str, &str)] = &[
    ("iwl hostcheck", "scripts/l6-iwl-fw-hostcheck.sh", "iwlwifi"),
    ("GSP hostcheck", "scripts/l6-g3-gsp-hostcheck.sh", "nouveau"),
    ("ath11k hostcheck", "scripts/l6-ath11k-hostcheck.sh", "ath11k"),
];

/// Perfil por defecto (`drv-all`, sin puertos lxdde explícitos): se conserva
/// la exigencia previa del check —iwlwifi y nouveau— y ath11k queda opcional,
/// que es su rama (Steam Deck). Con puertos explícitos manda el perfil.
const PORTS_POR_DEFECTO: &[&str] = &["iwlwifi", "nouveau"];

fn hostchecks_requeridos(ports: &[String], minimo: bool) -> Vec<String> {
    if minimo {
        return Vec::new();
    }
    let efectivos: Vec<String> = if ports.is_empty() {
        PORTS_POR_DEFECTO.iter().map(|p| p.to_string()).collect()
    } else {
        ports.to_vec()
    };
    HOSTCHECKS
        .iter()
        .filter(|(_, _, port)| efectivos.iter().any(|p| p == port || p == "all"))
        .map(|(nombre, _, _)| (*nombre).to_string())
        .collect()
}

/// Firmware que el perfil necesita en el rootfs para ese port.
fn firmware_requerido(port: &str) -> &'static [&'static str] {
    match port {
        "iwlwifi" => &[
            "lib/firmware/iwlwifi-cc-a0-77.ucode",
            "lib/firmware/iwlwifi-so-a0-gf-a0-89.ucode",
        ],
        "ath11k" => &[
            "lib/firmware/ath11k/WCN6855/hw2.1/amss.bin",
            "lib/firmware/ath11k/WCN6855/hw2.1/m3.bin",
            "lib/firmware/ath11k/WCN6855/hw2.1/board-2.bin",
        ],
        "nouveau" => &["lib/firmware/nvidia/ga102/gsp/gsp-570.144.bin"],
        _ => &[],
    }
}

fn comprobar_firmware(
    root: &Path,
    port: &str,
    perfil: &crate::drivers::DriverProfile,
    fallos: &Arc<Mutex<u32>>,
) {
    for rel in firmware_requerido(port) {
        // Un perfil que excluye ese firmware del rootfs no lo necesita.
        if perfil
            .firmware_exclude
            .iter()
            .any(|pat| glob_simple(pat, rel))
        {
            continue;
        }
        if !root.join("rootfs").join(rel).is_file() {
            eprintln!("check: FALLO firmware {port}: falta rootfs/{rel}");
            *fallos.lock().unwrap() += 1;
        }
    }
}

/// Coincidencia de los patrones que usa `firmware_exclude`: prefijo con `**`
/// o comodín final (`iwlwifi-*`).
fn glob_simple(patron: &str, ruta: &str) -> bool {
    if let Some(pre) = patron.strip_suffix("**") {
        return ruta.starts_with(pre);
    }
    if let Some(pre) = patron.strip_suffix('*') {
        return ruta.starts_with(pre);
    }
    patron == ruta
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
        ("soso-resize-core", &["soso-resize-core"], false),
        ("soso-hw", &["soso-hw"], false),
        ("xhci-nostd", &["xhci-nostd"], false),
        ("soso-audio+gguf2som", &["soso-audio", "gguf2som"], true),
        ("soso-forja-server", &["soso-forja-server"], false),
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
    cmd.current_dir(root).args(["test", "-q", "--locked"]);
    for pkg in pkgs {
        cmd.args(["-p", pkg]);
    }
    if con_std {
        cmd.args(["--features", "std"]);
    }
    if pkgs == &["soso-http"] || pkgs == &["soso-forja-server"] {
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

fn check_profile_minimo() -> bool {
    matches!(
        std::env::var("SOSO_CHECK_PROFILE").as_deref(),
        Ok("minimo") | Ok("mínimo")
    )
}

fn nombre_artefacto(nombre: &str) -> String {
    nombre
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Ejecuta un paso guardando su salida; si falla, la vuelca al terminal.
fn ejecutar_con_log(mut cmd: Command, log: &PathBuf, nombre: &str) -> bool {
    let salida = match cmd.stdin(Stdio::null()).output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("check: {nombre}: no se pudo ejecutar ({e})");
            let _ = std::fs::write(log, format!("no se pudo ejecutar: {e}\n"));
            return false;
        }
    };
    let mut texto = String::from_utf8_lossy(&salida.stdout).into_owned();
    texto.push_str(&String::from_utf8_lossy(&salida.stderr));
    let _ = std::fs::write(log, &texto);
    if !salida.status.success() {
        eprintln!("--- {nombre}: salida ---");
        eprint!("{texto}");
        eprintln!("--- fin {nombre} ({}) ---", log.display());
        return false;
    }
    true
}

fn run_script(
    root: &Path,
    rel: &str,
    fallos: &Arc<Mutex<u32>>,
    nombre: &str,
    required: bool,
    arte: &Path,
) {
    let script = root.join(rel);
    if !script.is_file() {
        if required {
            // El perfil compilado incluye este driver: sin banco no hay check.
            eprintln!("check: FALLO {nombre} (el perfil lo exige y falta {rel})");
            *fallos.lock().unwrap() += 1;
        } else {
            println!("check: omitido {nombre} (fuera del perfil, sin {rel})");
        }
        return;
    }
    let log = arte.join(format!("{}.log", nombre_artefacto(nombre)));
    let mut cmd = Command::new("bash");
    cmd.arg(&script).current_dir(root);
    if ejecutar_con_log(cmd, &log, nombre) {
        println!("check: OK  {nombre}");
    } else {
        eprintln!("check: FALLO {nombre}");
        *fallos.lock().unwrap() += 1;
    }
}

/// Todos los tests de xtask, no un subconjunto.
///
/// El filtro anterior (`hw_matrix::`/`test_install::`) dejaba fuera
/// `pci_stable::` y `elf_mmap_rules::`, entre otros. Estos bancos modelan el
/// comportamiento: valen como red, no como oráculo único — las rutas reales
/// las cubre QEMU en `cargo xtask test`.
fn run_xtask_tests(root: &Path, fallos: &Arc<Mutex<u32>>, arte: &Path) {
    let json = root.join("docs/hw-matrix.json");
    if !json.is_file() {
        println!("check: hw-matrix.json ausente — ejecuta `cargo xtask hw-matrix init`");
    }
    let log = arte.join("xtask-tests.log");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root)
        .args(["test", "-q", "--locked", "-p", "xtask", "--", "--nocapture"]);
    if ejecutar_con_log(cmd, &log, "tests xtask") {
        println!("check: OK  tests xtask");
    } else {
        eprintln!("check: FALLO tests xtask");
        *fallos.lock().unwrap() += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostchecks_del_perfil_live_son_obligatorios() {
        let ports = vec!["nouveau".to_string(), "iwlwifi".to_string()];
        let req = hostchecks_requeridos(&ports, false);
        assert!(req.iter().any(|r| r == "iwl hostcheck"));
        assert!(req.iter().any(|r| r == "GSP hostcheck"));
        assert!(
            !req.iter().any(|r| r == "ath11k hostcheck"),
            "el perfil live no trae ath11k"
        );
    }

    #[test]
    fn perfil_deck_exige_ath11k() {
        let req = hostchecks_requeridos(&["ath11k".to_string()], false);
        assert_eq!(req, vec!["ath11k hostcheck".to_string()]);
        // Perfil mínimo: nada obligatorio, pero se ejecuta lo que exista.
        assert!(hostchecks_requeridos(&["ath11k".to_string()], true).is_empty());
    }

    #[test]
    fn perfil_por_defecto_conserva_iwl_y_gsp() {
        // Antes de R10.2 el check exigía iwl y GSP salvo perfil mínimo: eso
        // no se relaja porque el perfil no liste puertos.
        let req = hostchecks_requeridos(&[], false);
        assert!(req.iter().any(|r| r == "iwl hostcheck"));
        assert!(req.iter().any(|r| r == "GSP hostcheck"));
        assert!(!req.iter().any(|r| r == "ath11k hostcheck"));
        assert!(hostchecks_requeridos(&[], true).is_empty());
    }

    #[test]
    fn perfil_all_exige_los_tres() {
        let req = hostchecks_requeridos(&["all".to_string()], false);
        assert_eq!(req.len(), HOSTCHECKS.len());
    }

    #[test]
    fn firmware_excluido_no_se_exige() {
        let live = crate::drivers::preset_live_usb();
        let mut fallos = 0u32;
        for rel in firmware_requerido("ath11k") {
            assert!(
                live.firmware_exclude
                    .iter()
                    .any(|p| glob_simple(p, rel)),
                "{rel} debería estar excluido en el perfil live"
            );
            fallos += 1;
        }
        assert_eq!(fallos, 3);
        // El firmware de iwlwifi sí se exige en live.
        for rel in firmware_requerido("iwlwifi") {
            assert!(!live.firmware_exclude.iter().any(|p| glob_simple(p, rel)));
        }
    }

    #[test]
    fn glob_simple_cubre_los_patrones_del_perfil() {
        assert!(glob_simple(
            "lib/firmware/ath11k/**",
            "lib/firmware/ath11k/WCN6855/hw2.1/m3.bin"
        ));
        assert!(glob_simple(
            "lib/firmware/iwlwifi-*",
            "lib/firmware/iwlwifi-cc-a0-77.ucode"
        ));
        assert!(!glob_simple(
            "lib/firmware/nvidia/**",
            "lib/firmware/iwlwifi-cc-a0-77.ucode"
        ));
    }

    #[test]
    fn firmware_requerido_del_perfil_live_existe_en_el_arbol() {
        // Cierra el hueco que motivó R10.2: el perfil compilado decide qué
        // firmware hace falta, y el árbol tiene que traerlo.
        let root = crate::project_root();
        let live = crate::drivers::preset_live_usb();
        for port in ["iwlwifi", "nouveau"] {
            for rel in firmware_requerido(port) {
                if live.firmware_exclude.iter().any(|p| glob_simple(p, rel)) {
                    continue;
                }
                assert!(
                    root.join("rootfs").join(rel).is_file(),
                    "falta rootfs/{rel} para el port {port}"
                );
            }
        }
    }
}
