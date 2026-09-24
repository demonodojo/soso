//! `cargo xtask test-probe` — las sondas de T33 que necesitan **reiniciar**.
//!
//! La suite normal arranca una máquina por shard, así que puede acreditar que
//! un fichero sobrevive a cerrar su descriptor, y no que sobreviva al apagado.
//! Son cosas distintas: lo primero lo cumple una caché.
//!
//! Aquí se arranca la misma imagen **dos veces**. El primer arranque escribe y
//! hace `halt`; el segundo reabre y compara. La máquina se apaga de verdad en
//! medio, que es lo que la comprobación de T33 pide.
//!
//! El lanzamiento de QEMU se reutiliza de `test_update` a propósito: duplicarlo
//! haría que las dos rutas se separaran y que un arreglo en una no llegara a la
//! otra.

use std::path::Path;
use std::time::Duration;

use crate::test::{esperar_en_fichero, ssh_guion_hasta};
use crate::test_update::{lanzar_live, Matar, SSH_PORT};

/// Sondas que necesitan dos arranques. Cada una es `(nombre, fase1, fase2)`.
const SONDAS: &[(&str, &str, &str)] = &[(
    "archivos persistentes",
    "archivos-fase1",
    "archivos-fase2",
)];

pub fn run(filtro: Option<&str>) {
    let quiere = |nombre: &str| filtro.is_none_or(|f| nombre.contains(f));
    let root = crate::project_root();
    let dir = root.join("target/test-probe");
    std::fs::create_dir_all(&dir).expect("test-probe dir");

    unsafe {
        std::env::set_var("SOSO_QEMU_LIVE", "1");
        std::env::set_var("SOSO_QEMU_LIVE_USB", "1");
        // Modelo sintético: esto mide el sistema de archivos, no inferencia, y
        // los 2,2 GB del modelo real habría que escribirlos y arrancarlos dos
        // veces por sonda.
        if std::env::var_os("SOSO_MODELS_DIR").is_none() {
            unsafe { std::env::set_var("SOSO_MODELS_DIR", root.join("target/tiny-model")) };
        }
    }
    crate::package_live::run();
    let live = crate::package_live::live_image_path();

    let Some((ovmf_code, ovmf_vars_src)) = crate::ovmf_paths() else {
        eprintln!("test-probe: necesita OVMF (cargo xtask test-install)");
        std::process::exit(1);
    };
    let vars = dir.join("OVMF_VARS.fd");
    std::fs::copy(&ovmf_vars_src, &vars).expect("OVMF_VARS");
    let key = root.join("target/soso_test_key");

    let mut fallos = 0u32;
    for (nombre, fase1, fase2) in SONDAS {
        if !quiere(nombre) {
            continue;
        }
        // Copia propia de la imagen: la sonda escribe en ella, y dejar que dos
        // sondas compartan disco haría que la segunda viera lo que sembró la
        // primera.
        let img = dir.join(format!("{}.img", fase1));
        crate::copy_sparse(&live, &img);
        match ciclo(&ovmf_code, &vars, &img, &dir, &key, fase1, fase2) {
            Ok(salida) => {
                println!("OK    {nombre}");
                println!("{salida}");
            }
            Err(e) => {
                println!("FALLO {nombre}: {e}");
                fallos += 1;
            }
        }
    }

    if fallos == 0 {
        println!("\n✅ cargo xtask test-probe: TODO OK");
    } else {
        println!("\n❌ cargo xtask test-probe: {fallos} fallo(s)");
        std::process::exit(1);
    }
}

/// Un arranque escribe y apaga; el siguiente reabre y compara.
fn ciclo(
    code: &Path,
    vars: &Path,
    img: &Path,
    dir: &Path,
    key: &Path,
    fase1: &str,
    fase2: &str,
) -> Result<String, String> {
    let s1 = dir.join(format!("{fase1}-1.log"));
    let _ = std::fs::remove_file(&s1);
    {
        let qemu = lanzar_live(code, vars, img, &s1)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s1, "sosh —", Duration::from_secs(300))?;
        // Acaba en `halt`, no en `exit`: lo que se mide es que los datos
        // sobrevivan a un apagado, así que la máquina tiene que apagarse.
        let salida = ssh_guion_hasta(
            key,
            SSH_PORT,
            &format!("soso-agent-probe {fase1}\nhalt\n"),
            Duration::from_secs(300),
            "probe: fase1 lista",
        )?;
        if salida.contains("FALLO") {
            return Err(format!("la fase 1 ya falló: {salida:?}"));
        }
    }

    let s2 = dir.join(format!("{fase2}-2.log"));
    let _ = std::fs::remove_file(&s2);
    let salida = {
        let qemu = lanzar_live(code, vars, img, &s2)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s2, "sosh —", Duration::from_secs(300))?;
        ssh_guion_hasta(
            key,
            SSH_PORT,
            &format!("soso-agent-probe {fase2}\nhalt\n"),
            Duration::from_secs(300),
            "probe-json:",
        )?
    };
    if salida.contains("FALLO") || !salida.contains("\"malos\":0") {
        return Err(format!("tras reiniciar: {salida:?}"));
    }
    Ok(crate::test_update::sangrar_pub(&salida))
}
