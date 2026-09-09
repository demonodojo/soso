//! `cargo xtask test-install` — instalación nativa de punta a punta en QEMU.
//!
//! Reproduce lo que hace el usuario en la placa, sin tocar nada del host:
//!
//! 1. **Arranque 1** — UEFI (OVMF) desde el pendrive live, con un NVMe vacío
//!    como destino. Por SSH: `soso-install list` y `soso-install <id> --yes`.
//! 2. **Comprobaciones de host** sobre la imagen del NVMe: GPT válida según
//!    `sgdisk -v`, la última partición llena el disco, los GUID ya no son los
//!    del USB y la partición 2 empieza por el magic de sosofs.
//! 3. **Arranque 2** — el mismo pendrive: el shim UEFI encuentra la petición en
//!    `SOSOBOOT.TXT` y registra `Boot####`. Se comprueba leyendo el fichero de
//!    la ESP desde el host.
//! 4. **Arranque 3** — solo el NVMe, con la misma NVRAM: el disco instalado
//!    arranca soso por su cuenta, sin pendrive y sin Linux por medio.
//!
//! La NVRAM es una copia propia de `OVMF_VARS` (`target/test-install/`), así
//! que no se pisa con la de `cargo xtask run`.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::test::{esperar_en_fichero, ssh_guion, ssh_guion_hasta};

const SSH_PORT: u16 = 2242;
const MAC: &str = "52:54:00:12:34:42";
/// Margen sobre el tamaño real del live al dimensionar el NVMe destino: sirve
/// para que se note que `relayout` estira la última partición sin necesitar
/// un tamaño fijo — el live crece con los modelos que se empaqueten
/// (`SOSO_LIVE_MODELS`), y una constante fija queda obsoleta en cuanto crece.
const TARGET_MARGIN_BYTES: u64 = 512 * 1024 * 1024;

pub fn run() {
    let root = crate::project_root();
    let dir = root.join("target/test-install");
    std::fs::create_dir_all(&dir).expect("target/test-install");

    // El live tiene que existir y llevar SOSOBOOT.TXT: siempre se reempaqueta.
    unsafe {
        std::env::set_var("SOSO_QEMU_LIVE", "1");
        std::env::set_var("SOSO_QEMU_LIVE_USB", "1");
    }
    crate::package_live::run();
    let live = crate::package_live::live_image_path();
    let live_p1 = gpt_part_lba(&live, 1).expect("ESP del live");

    let Some((ovmf_code, ovmf_vars_src)) = crate::ovmf_paths() else {
        eprintln!(
            "test-install: no encuentro OVMF (instala ovmf o define \
             SOSO_OVMF_CODE/SOSO_OVMF_VARS); este test necesita UEFI"
        );
        std::process::exit(1);
    };
    let vars = dir.join("OVMF_VARS.fd");
    std::fs::copy(&ovmf_vars_src, &vars).expect("copiar OVMF_VARS");

    let target = dir.join("nvme-target.img");
    let target_bytes = std::fs::metadata(&live).expect("tamaño del live").len() + TARGET_MARGIN_BYTES;
    crear_vacia(&target, target_bytes);
    // Segundo NVMe haciéndose pasar por el disco de Linux: el instalador tiene
    // que negarse a tocarlo, que es justo lo que no puede fallar nunca.
    let ajeno = dir.join("nvme-linux.img");
    crear_disco_ajeno(&ajeno, 2 * 1024 * 1024 * 1024);

    let key = root.join("target/soso_test_key");
    let mut fallos = 0u32;

    // --- 1. instalar -------------------------------------------------------
    let serial1 = dir.join("boot1.log");
    let guids_antes = guids(&live);
    match fase_instalar(&ovmf_code, &vars, &live, &target, &ajeno, &serial1, &key) {
        Ok(salida) => {
            marca("arranque 1: soso-install", true);
            println!("{}", sangrar(&salida));
        }
        Err(e) => {
            marca(&format!("arranque 1: soso-install — {e}"), false);
            eprintln!("      ver log serie: {}", serial1.display());
            fallos += 1;
        }
    }

    // --- 2. la GPT del destino --------------------------------------------
    for (nombre, r) in comprobar_gpt(&target, &guids_antes) {
        let ok = r.is_ok();
        marca(
            &match r {
                Ok(()) => nombre.clone(),
                Err(e) => format!("{nombre} — {e}"),
            },
            ok,
        );
        if !ok {
            fallos += 1;
        }
    }

    // --- 3. el shim registra la entrada ------------------------------------
    let serial2 = dir.join("boot2.log");
    match fase_registrar(&ovmf_code, &vars, &live, &target, &ajeno, &serial2, live_p1) {
        Ok(linea) => {
            marca(&format!("arranque 2: NVRAM — {linea}"), true);
        }
        Err(e) => {
            marca(&format!("arranque 2: NVRAM — {e}"), false);
            eprintln!("      ver log serie: {}", serial2.display());
            fallos += 1;
        }
    }

    // --- 4. arrancar solo del disco instalado ------------------------------
    let serial3 = dir.join("boot3.log");
    match fase_arranque_solo(&ovmf_code, &vars, &target, &serial3, &key) {
        Ok(()) => marca("arranque 3: soso arranca del NVMe sin USB", true),
        Err(e) => {
            marca(&format!("arranque 3: soso arranca del NVMe sin USB — {e}"), false);
            eprintln!("      ver log serie: {}", serial3.display());
            fallos += 1;
        }
    }

    if fallos > 0 {
        eprintln!("\ntest-install: {fallos} comprobación(es) fallaron");
        std::process::exit(1);
    }
    println!("\ntest-install: instalación nativa OK");
}

// ---------------------------------------------------------------- fases

fn fase_instalar(
    code: &Path,
    vars: &Path,
    live: &Path,
    target: &Path,
    ajeno: &Path,
    serial: &Path,
    key: &Path,
) -> Result<String, String> {
    let _ = std::fs::remove_file(serial);
    let qemu = lanzar(
        code,
        vars,
        serial,
        Discos::LiveYDestino {
            live,
            target,
            ajeno: Some(ajeno),
        },
    )?;
    let _guard = Matar(qemu);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(240))?;

    // Ids de `raw_disk::RawId`: 0 = USB de arranque, 2 = Nvme0 (destino),
    // 3 = Nvme1 (el disco con pinta de Linux, que debe rechazar).
    let salida = ssh_guion(
        key,
        SSH_PORT,
        "soso-install list\nsoso-install 3\nsoso-install 2 --yes\nsoso-install status\nexit\n",
        Duration::from_secs(600),
    )?;
    if !salida.contains("particiones de otro sistema") || !salida.contains("swap") {
        return Err(format!(
            "no rechazó el disco con otro sistema; stdout: {salida:?}"
        ));
    }
    if !salida.contains("copia terminada") {
        return Err(format!("el instalador no terminó la copia; stdout: {salida:?}"));
    }
    if !salida.contains("GPT ajustada al disco") {
        return Err(format!("no reparó la GPT; stdout: {salida:?}"));
    }
    if !salida.contains("INSTALL ") {
        return Err(format!("`status` no ve la petición de arranque; stdout: {salida:?}"));
    }
    Ok(salida)
}

fn fase_registrar(
    code: &Path,
    vars: &Path,
    live: &Path,
    target: &Path,
    ajeno: &Path,
    serial: &Path,
    live_p1: u64,
) -> Result<String, String> {
    let _ = std::fs::remove_file(serial);
    let qemu = lanzar(
        code,
        vars,
        serial,
        Discos::LiveYDestino {
            live,
            target,
            ajeno: Some(ajeno),
        },
    )?;
    let _guard = Matar(qemu);
    // El shim actúa antes del kernel; basta con llegar al login.
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(240))?;

    let datos = crate::fat32_write::read_root_file(live, live_p1, b"SOSOBOOTTXT")
        .map_err(|e| format!("no pude leer SOSOBOOT.TXT del live: {e}"))?;
    let texto = String::from_utf8_lossy(&datos);
    let linea = texto
        .lines()
        .find(|l| l.starts_with("DONE") || l.starts_with("ERROR"))
        .ok_or_else(|| {
            format!(
                "el shim no dejó respuesta en SOSOBOOT.TXT; contenido: {:?}",
                texto.lines().take(4).collect::<Vec<_>>()
            )
        })?;
    if linea.starts_with("ERROR") {
        return Err(linea.to_string());
    }
    Ok(linea.to_string())
}

fn fase_arranque_solo(
    code: &Path,
    vars: &Path,
    target: &Path,
    serial: &Path,
    key: &Path,
) -> Result<(), String> {
    let _ = std::fs::remove_file(serial);
    let qemu = lanzar(code, vars, serial, Discos::SoloDestino { target })?;
    let _guard = Matar(qemu);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(240))?;
    let log = std::fs::read_to_string(serial).map_err(|e| e.to_string())?;
    if !log.contains("fs: sosofs") {
        return Err("arrancó pero no montó el rootfs instalado".into());
    }
    // Sin USB conectado (a propósito, esta fase sólo lleva el NVMe): antes del
    // fix de `boot_source()`, ningún disco quedaba marcado DISK_FLAG_BOOT y
    // `soso-update estado` no podía decir de dónde había arrancado.
    let salida = ssh_guion_hasta(
        key,
        SSH_PORT,
        "soso-update estado\nhalt\n",
        Duration::from_secs(120),
        "arranque: disco instalado",
    )?;
    if !salida.contains("arranque: disco instalado") {
        return Err(format!(
            "estado no reconoce el NVMe como disco de arranque: {salida:?}"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------- QEMU

enum Discos<'a> {
    LiveYDestino {
        live: &'a Path,
        target: &'a Path,
        ajeno: Option<&'a Path>,
    },
    SoloDestino {
        target: &'a Path,
    },
}

/// QEMU idéntico en las tres fases salvo por los discos: el orden de los
/// `-device` se mantiene para que el device path que se grabó en la NVRAM
/// siga apuntando al mismo sitio en el arranque siguiente.
fn lanzar(code: &Path, vars: &Path, serial: &Path, discos: Discos) -> Result<Child, String> {
    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35", "-cpu", "max"])
        .args(["-m", &crate::qemu_mem()])
        .args(["-smp", &crate::qemu_smp()]);
    qemu.args([
        "-drive",
        &format!("if=pflash,format=raw,readonly=on,file={}", code.display()),
    ]);
    qemu.args([
        "-drive",
        &format!("if=pflash,format=raw,file={}", vars.display()),
    ]);
    qemu.args(["-device", "qemu-xhci,id=xhci"]);

    match discos {
        Discos::LiveYDestino {
            live,
            target,
            ajeno,
        } => {
            qemu.args([
                "-drive",
                &format!("file={},format=raw,if=none,id=live0", live.display()),
            ]);
            qemu.args(["-device", "usb-storage,bus=xhci.0,port=1,drive=live0"]);
            qemu.args([
                "-drive",
                &format!("file={},format=raw,if=none,id=nvme0", target.display()),
            ]);
            qemu.args(["-device", "nvme,serial=soso-target,drive=nvme0"]);
            if let Some(ajeno) = ajeno {
                qemu.args([
                    "-drive",
                    &format!("file={},format=raw,if=none,id=nvme1", ajeno.display()),
                ]);
                qemu.args(["-device", "nvme,serial=fake-linux,drive=nvme1"]);
            }
        }
        Discos::SoloDestino { target } => {
            qemu.args([
                "-drive",
                &format!("file={},format=raw,if=none,id=nvme0", target.display()),
            ]);
            qemu.args(["-device", "nvme,serial=soso-target,drive=nvme0"]);
        }
    }

    crate::apply_qemu_nic_with_ports(&mut qemu, SSH_PORT, SSH_PORT + 1, Some(&MAC.to_string()));
    qemu.args(["-serial", &format!("file:{}", serial.display())])
        .args(["-display", "none"])
        .arg("-no-reboot")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("no se pudo lanzar QEMU: {e}"))
}

/// QEMU que muere al salir del ámbito, pase lo que pase: un huérfano se queda
/// con el puerto SSH y con el lock de escritura de las imágenes.
struct Matar(Child);

impl Drop for Matar {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// ------------------------------------------------------- comprobaciones

fn comprobar_gpt(target: &Path, guids_usb: &[String]) -> Vec<(String, Result<(), String>)> {
    let mut out = Vec::new();

    out.push((
        "GPT del destino consistente (sgdisk -v)".to_string(),
        sgdisk_verify(target),
    ));

    let tabla = match sgdisk_print(target) {
        Ok(t) => t,
        Err(e) => {
            out.push(("leer la tabla del destino".to_string(), Err(e)));
            return out;
        }
    };

    let total = std::fs::metadata(target)
        .map(|m| m.len() / 512)
        .unwrap_or(0);
    out.push((
        "la última partición llena el disco".to_string(),
        ultima_llega_al_final(&tabla, total),
    ));

    let nuevos = guids(target);
    let repetido = nuevos.iter().find(|g| guids_usb.contains(g));
    out.push((
        "GUID distintos a los del USB".to_string(),
        match repetido {
            Some(g) => Err(format!("el destino repite el GUID {g}")),
            None => Ok(()),
        },
    ));

    out.push(("magic sosofs en la partición 2".to_string(), sosofs_en_p2(target)));
    out
}

fn sgdisk_verify(img: &Path) -> Result<(), String> {
    let out = Command::new("sgdisk")
        .arg("-v")
        .arg(img)
        .output()
        .map_err(|e| format!("sgdisk: {e}"))?;
    let texto = String::from_utf8_lossy(&out.stdout).into_owned();
    if texto.contains("No problems found") {
        Ok(())
    } else {
        Err(texto.trim().replace('\n', " | "))
    }
}

fn sgdisk_print(img: &Path) -> Result<String, String> {
    let out = Command::new("sgdisk")
        .arg("-p")
        .arg(img)
        .output()
        .map_err(|e| format!("sgdisk: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `sgdisk -p` lista `número inicio fin …`. p4 (SOSOINSTALL) va **antes**
/// que p3 en el disco; hay que tomar el fin más alto, no la última línea.
fn ultima_llega_al_final(tabla: &str, total_sectores: u64) -> Result<(), String> {
    let mut ultimo = None;
    for line in tabla.lines() {
        let campos: Vec<&str> = line.split_whitespace().collect();
        if campos.len() < 3 || campos[0].parse::<u32>().is_err() {
            continue;
        }
        if let Ok(fin) = campos[2].parse::<u64>() {
            ultimo = Some(ultimo.map_or(fin, |u: u64| u.max(fin)));
        }
    }
    let fin = ultimo.ok_or("sgdisk -p no listó ninguna partición")?;
    let esperado = total_sectores - 34;
    if fin == esperado {
        Ok(())
    } else {
        Err(format!("la última partición acaba en {fin}, esperaba {esperado}"))
    }
}

fn sosofs_en_p2(img: &Path) -> Result<(), String> {
    let lba = gpt_part_lba(img, 2).ok_or("sin partición 2")?;
    let datos = leer_en(img, lba * 512, 8)?;
    if &datos == b"SOSOFS11" {
        Ok(())
    } else {
        Err(format!("magic inesperado: {:?}", String::from_utf8_lossy(&datos)))
    }
}

/// GUID únicos de las particiones más el del disco, tal como los ve sgdisk.
fn guids(img: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(o) = Command::new("sgdisk").arg("-p").arg(img).output() {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            if let Some(rest) = line.split("GUID:").nth(1) {
                out.push(rest.trim().to_string());
            }
        }
    }
    for part in 1..=4 {
        if let Ok(o) = Command::new("sgdisk")
            .args(["-i", &part.to_string()])
            .arg(img)
            .output()
        {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                if let Some(rest) = line.split("Partition unique GUID:").nth(1) {
                    out.push(rest.trim().to_string());
                }
            }
        }
    }
    out
}

fn gpt_part_lba(img: &Path, part: u32) -> Option<u64> {
    let out = Command::new("sgdisk")
        .args(["-i", &part.to_string()])
        .arg(img)
        .output()
        .ok()?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(rest) = line.split("First sector:").nth(1) {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

fn leer_en(img: &Path, off: u64, len: usize) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(img).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Disco que imita al de Linux: ESP + raíz + swap. Solo hace falta la tabla;
/// el instalador decide por los GUID de tipo, no por el contenido.
fn crear_disco_ajeno(path: &Path, bytes: u64) {
    crear_vacia(path, bytes);
    let st = Command::new("sgdisk")
        .args([
            "-n", "1:2048:+512M", "-t", "1:ef00",
            "-n", "2:0:+1G", "-t", "2:8300",
            "-n", "3:0:0", "-t", "3:8200",
        ])
        .arg(path)
        .output()
        .expect("sgdisk para el disco ajeno");
    if !st.status.success() {
        panic!(
            "sgdisk falló creando el disco ajeno: {}",
            String::from_utf8_lossy(&st.stderr)
        );
    }
}

fn crear_vacia(path: &Path, bytes: u64) {
    let f = std::fs::File::create(path).expect("crear imagen destino");
    f.set_len(bytes).expect("dimensionar imagen destino");
}

fn marca(nombre: &str, ok: bool) {
    println!("  {} {nombre}", if ok { "✔" } else { "✘" });
}

fn sangrar(texto: &str) -> String {
    texto
        .lines()
        .map(|l| format!("      {}", l.trim_end()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::ultima_llega_al_final;

    #[test]
    fn ultima_llega_al_final_usa_fin_mas_alto_no_ultima_linea() {
        let tabla = "\
Number  Start (sector)    End (sector)  Size       Code  Name\n\
   1              34          194593   95.0 MiB    EF00  boot\n\
   2          194594          981025   384.0 MiB   8300\n\
   3         1112098         2688990   770.0 MiB   8300\n\
   4          981026         1112097   64.0 MiB    0700  SOSOINSTALL\n";
        assert!(ultima_llega_al_final(tabla, 2688990 + 34).is_ok());
        assert!(ultima_llega_al_final(tabla, 2688990 + 100).is_err());
    }
}
