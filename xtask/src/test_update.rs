//! `cargo xtask test-update` — actualización local de punta a punta en QEMU/OVMF.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use soso_update_core::backup_digest;
use soso_update_core::hash::hex_sha256;
use soso_update_core::kernel_meta::{KernelMeta, KernelPhase, KERNEL_META_SIZE};
use soso_update_core::manifest::Manifest;
use soso_update_core::mailbox::Mailbox;
use soso_update_core::pack::pack_rootfs_con;
use soso_update_core::semver::parse as parse_semver;
use soso_update_core::UPD_KERNEL_SLOT_SIZE;

use crate::test::{esperar_en_fichero, ssh_guion_hasta};

const SSH_PORT: u16 = 2243;
const MAC: &str = "52:54:00:12:34:43";
const TEST_VER: &str = "0.2.1-prueba";
/// Versión de la release que **no arranca**: su `/bin/init` no es un ELF.
const VER_ROTA: &str = "0.2.8-rota";
/// Tercera versión, para la cadena A→B→C.
const VER_C: &str = "0.2.3-prueba";
const MARCA_C: &str = "tercera\n";
const MARCA_NUEVA: &str = "nueva\n";
const MARCA_VIEJA: &str = "vieja\n";

pub fn run(filtro: Option<&str>) {
    let quiere = |nombre: &str| filtro.is_none_or(|f| nombre.contains(f));
    let root = crate::project_root();
    let dir = root.join("target/test-update");
    std::fs::create_dir_all(&dir).expect("test-update dir");

    preparar_release_prueba(&root);
    preparar_release_rota(&root);
    preparar_release_c(&root);

    unsafe {
        std::env::set_var("SOSO_QEMU_LIVE", "1");
        std::env::set_var("SOSO_QEMU_LIVE_USB", "1");
        // Sólo el modelo sintético: estas pruebas son de actualización, no de
        // inferencia, y el modelo grande se lleva 2,2 GB de imagen —que hay que
        // escribir, arrancar y, en la fase del disco ajeno, **clonar**—. Si
        // alguien pide otro por entorno, se respeta.
        if std::env::var_os("SOSO_MODELS_DIR").is_none() {
            unsafe { std::env::set_var("SOSO_MODELS_DIR", root.join("target/tiny-model")) };
        }
    }
    crate::package_live::run();
    let live = crate::package_live::live_image_path();
    let live_base = dir.join("live-base.img");
    crate::copy_sparse(&live, &live_base);

    let Some((ovmf_code, ovmf_vars_src)) = crate::ovmf_paths() else {
        eprintln!("test-update: necesita OVMF (cargo xtask test-install)");
        std::process::exit(1);
    };
    let vars = dir.join("OVMF_VARS.fd");
    std::fs::copy(&ovmf_vars_src, &vars).expect("OVMF_VARS");

    let key = root.join("target/soso_test_key");
    let mut fallos = 0u32;

    let serial1 = dir.join("boot1.log");
    if quiere("aplicar") {
        match fase_aplicar(&ovmf_code, &vars, &live, &serial1, &key) {
            Ok(s) => {
                marca("arranque 1: soso-update aplicar --local", true);
                println!("{}", sangrar(&s));
            }
            Err(e) => {
                marca(&format!("arranque 1: aplicar — {e}"), false);
                fallos += 1;
            }
        }
    }

    let serial2 = dir.join("boot2.log");
    if quiere("version") {
        match fase_comprobar_version(&ovmf_code, &vars, &live, &serial2, &key, TEST_VER) {
            Ok(()) => marca(&format!("arranque 2: versión {TEST_VER} visible"), true),
            Err(e) => {
                marca(&format!("arranque 2: versión — {e}"), false);
                fallos += 1;
            }
        }
    }

    if quiere("vuelta") {
        let base_vuelta = dir.join("live-vuelta.img");
        crate::copy_sparse(&live_base, &base_vuelta);
        let ver_base = crate::version::read_version(&root);
        match fase_vuelta_atras(&ovmf_code, &vars, &base_vuelta, &dir, &key, &ver_base) {
            Ok(()) => marca(
                "vuelta atrás manual: revertir + arranque restaura la anterior",
                true,
            ),
            Err(e) => {
                marca(&format!("vuelta atrás manual — {e}"), false);
                fallos += 1;
            }
        }
    }

    if quiere("ajeno") {
        let serial = dir.join("ajeno.log");
        let ver = crate::version::read_version(&root);
        match fase_recuperar_ajeno(
            &ovmf_code, &vars, &live_base, &live, &dir, &key, &serial, &ver,
        ) {
            Ok(l) => marca(&format!("desde el live: recuperar otro disco — {l}"), true),
            Err(e) => {
                marca(&format!("desde el live: recuperar otro disco — {e}"), false);
                fallos += 1;
            }
        }
    }

    let serial3 = dir.join("boot-recovery.log");
    if quiere("recuperacion") {
        match fase_recuperacion_corte(&ovmf_code, &vars, &live_base, &serial3, &key) {
            Ok(()) => marca("arranque 3: recuperación tras corte (meta applying)", true),
            Err(e) => {
                marca(&format!("arranque 3: recuperación — {e}"), false);
                fallos += 1;
            }
        }
    }

    if quiere("cadena") {
        let base = dir.join("live-cadena.img");
        crate::copy_sparse(&live_base, &base);
        match fase_cadena(&ovmf_code, &vars, &base, &dir, &key) {
            Ok(l) => marca(&format!("cadena A→B→C y vuelta a B — {l}"), true),
            Err(e) => {
                marca(&format!("cadena A→B→C — {e}"), false);
                fallos += 1;
            }
        }
    }

    if quiere("confirmacion") {
        let base = dir.join("live-confirma.img");
        crate::copy_sparse(&live_base, &base);
        match fase_corte_confirmacion(&ovmf_code, &vars, &base, &dir, &key) {
            Ok(l) => marca(&format!("corte al confirmar: se completa, no se deshace — {l}"), true),
            Err(e) => {
                marca(&format!("corte al confirmar — {e}"), false);
                fallos += 1;
            }
        }
    }

    if quiere("respaldo") {
        let base = dir.join("live-respaldo.img");
        crate::copy_sparse(&live_base, &base);
        match fase_respaldo_roto(&ovmf_code, &vars, &base, &dir, &key) {
            Ok(l) => marca(&format!("respaldo corrupto: no arranca a medias — {l}"), true),
            Err(e) => {
                marca(&format!("respaldo corrupto — {e}"), false);
                fallos += 1;
            }
        }
    }

    if quiere("roto") {
        let base = dir.join("live-roto.img");
        crate::copy_sparse(&live_base, &base);
        let ver = crate::version::read_version(&root);
        match fase_init_roto(&ovmf_code, &vars, &base, &dir, &key, &ver) {
            Ok(l) => marca(&format!("arranque roto: se deshace solo — {l}"), true),
            Err(e) => {
                marca(&format!("arranque roto — {e}"), false);
                fallos += 1;
            }
        }
    }

    // Salida TCP desde userland: determinista y sin internet — el guest llega
    // al host por 10.0.2.2, así que el servidor de eco lo levanta el propio
    // banco. Dos connects al mismo puerto (cierra y reabre): es el patrón de
    // un redirect HTTP, y con uno solo `saliente` pasaba mientras `https` no.
    if quiere("saliente") {
        let serial = dir.join("saliente.log");
        let base = dir.join("live-saliente.img");
        crate::copy_sparse(&live_base, &base);
        match fase_tcp_saliente(&ovmf_code, &vars, &base, &serial, &key) {
            Ok(l) => marca(&format!("TCP saliente desde userland — {l}"), true),
            Err(e) => {
                marca(&format!("TCP saliente — {e}"), false);
                fallos += 1;
            }
        }
    }

    // Camino de red real: **no** entra en la pasada normal, porque depende de
    // internet y de que GitHub conteste. Se pide a mano con SOSO_TEST_RED=1.
    // Existe porque todo lo demás usa `--local` y el HTTPS de userspace no lo
    // ejercitaba nadie: el primer intento en placa se estrelló.
    if quiere("https") && std::env::var("SOSO_TEST_RED").is_ok() {
        let base = dir.join("live-https.img");
        crate::copy_sparse(&live_base, &base);
        let serial = dir.join("https.log");
        match fase_https(&ovmf_code, &vars, &base, &serial, &key) {
            Ok(l) => marca(&format!("HTTPS real: {l}"), true),
            Err(e) => {
                marca(&format!("HTTPS real — {e}"), false);
                fallos += 1;
            }
        }
    }

    let serial4 = dir.join("boot-manifest.log");
    if quiere("manifiesto") {
        match fase_manifiesto_invalido(&ovmf_code, &vars, &live, &serial4, &key) {
            Ok(()) => marca("arranque 4: manifiesto inválido rechazado", true),
            Err(e) => {
                marca(&format!("arranque 4: manifiesto — {e}"), false);
                fallos += 1;
            }
        }
    }

    // Las releases de prueba viven dentro de `rootfs/` para que el guest las
    // vea en `/var/actualiza-*`, así que hay que retirarlas al acabar: si no,
    // se quedan ahí e inflan **cualquier** imagen que se construya después
    // —42 MB que dejaron de caber en la partición de un USB ya flasheado—.
    limpiar_releases_prueba(&root);

    if fallos > 0 {
        eprintln!("\ntest-update: {fallos} fallo(s)");
        std::process::exit(1);
    }
    println!("\ntest-update: actualización local OK (+ recuperación OTA)");
}

/// Retira del rootfs las releases que fabrica este banco.
fn limpiar_releases_prueba(root: &Path) {
    for d in ["actualiza-prueba", "actualiza-rota", "actualiza-c"] {
        let _ = std::fs::remove_dir_all(root.join("rootfs/var").join(d));
    }
}

fn preparar_release_prueba(root: &Path) {
    let rel_dir = root.join("rootfs/var/actualiza-prueba");
    // De una pasada anterior pueden quedar ~160 MB aquí dentro; el rootfs se
    // empaqueta entero en la imagen y no cabría.
    let _ = std::fs::remove_dir_all(&rel_dir);
    std::fs::create_dir_all(&rel_dir).expect("actualiza-prueba");

    crate::build_user();

    let kernel_src = root.join("target/kernel/x86_64-soso/debug/kernel");
    if !kernel_src.exists() {
        let profile = crate::drivers::preset_live_usb();
        let _ = crate::build_image_with_profile(&profile, true);
    }
    let kernel_bytes = std::fs::read(&kernel_src).expect("kernel");

    // El pack lleva la versión nueva; el live se empaqueta con la versión actual (VERSION).
    let etc = root.join("rootfs/etc");
    std::fs::write(
        etc.join("soso-release"),
        format!("version={TEST_VER}\nbuild=prueba\nfecha=2026-09-04\n"),
    )
    .expect("soso-release prueba");
    // Un fichero que de verdad cambie: el pack lleva "nueva" y el live que se
    // instala lleva "vieja", así que `aplicar` tiene que bajar ese tramo y sólo
    // ese. Sin esto el pack sería idéntico al rootfs instalado y la descarga
    // parcial no se ejercitaría.
    let marca_path = etc.join("actualiza-marca.txt");
    std::fs::write(&marca_path, MARCA_NUEVA).expect("marca nueva");
    // Sin el firmware de la GPU: son 127 MB que el test no necesita y que, al
    // vivir el release dentro del propio rootfs, duplicarían la imagen.
    let (pack_blob, files) = pack_rootfs_con(&root.join("rootfs"), |rel| {
        // Ni el firmware de la GPU (127 MB que el test no necesita) ni las
        // **otras releases de prueba**: viven dentro del propio rootfs, así que
        // sin esto cada release empaquetaría a las anteriores y la tercera
        // ocuparía el triple que la primera —hasta llenar el disco a mitad de
        // descarga—.
        !rel.starts_with("lib/firmware/") && !rel.starts_with("var/actualiza")
    })
    .expect("pack");
    std::fs::write(&marca_path, MARCA_VIEJA).expect("marca vieja");
    crate::version::write_soso_release(root);

    // Igual que `cargo xtask release`: el kernel que se publica va sin símbolos
    // de depuración. Así la fase 2 arranca exactamente el ELF que recibiría una
    // placa real, y no un binario distinto al del release.
    let kernel_pub = rel_dir.join("kernel-x86_64");
    std::fs::write(&kernel_pub, &kernel_bytes).expect("kernel copy");
    crate::release::strip_kernel(&kernel_pub);
    let kernel_bytes = std::fs::read(&kernel_pub).expect("kernel stripped");
    std::fs::write(rel_dir.join("rootfs.pack"), &pack_blob).expect("pack");

    let manifest = Manifest {
        version: parse_semver(TEST_VER).unwrap(),
        version_raw: TEST_VER.into(),
        build: "prueba".into(),
        fecha: "2026-09-04".into(),
        kernel_hash: hex_sha256(&kernel_bytes),
        kernel_size: kernel_bytes.len() as u64,
        pack_hash: hex_sha256(&pack_blob),
        pack_size: pack_blob.len() as u64,
        // Desde U3 el cliente rechaza un manifiesto sin contrato de
        // compatibilidad, así que la release de prueba tiene que traerlo. El
        // banco arranca desde un USB, de ahí el driver declarado.
        compat: Some(soso_update_core::Compat {
            arch: "x86_64".into(),
            perfil: "live-usb".into(),
            drivers: vec![
                "usb".into(),
                "nvme".into(),
                "virtio-blk".into(),
                "live-disk".into(),
            ],
            abi: soso_abi::ABI_VERSION,
            fs: soso_update_core::compat::FS_FORMATO.into(),
            min_shim: soso_update_core::compat::SHIM_VERSION,
            min_recuperador: soso_update_core::compat::RECUPERADOR_VERSION,
        }),
        files,
    };
    std::fs::write(rel_dir.join("manifest.txt"), manifest.format()).expect("manifest");
    std::fs::write(
        rel_dir.join("manifest-malo.txt"),
        format!(
            "{}\nversion=9.9.9\nbuild=x\nfecha=2026-01-01\n\
             kernel {} 10\npack {} 20\nf {} 0 5 ../etc/passwd\n",
            soso_update_core::manifest::MANIFEST_MAGIC,
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
        ),
    )
    .expect("manifest-malo");
    println!("test-update: release de prueba en rootfs/var/actualiza-prueba/");
}

/// Una release cuyo `/bin/init` no arranca. Es el caso que da sentido a todo
/// el contrato: si la versión nueva no llega a userspace, el encendido
/// siguiente la deshace **solo**, sin que nadie pida nada.
fn preparar_release_rota(root: &Path) {
    let rel_dir = root.join("rootfs/var/actualiza-rota");
    let _ = std::fs::remove_dir_all(&rel_dir);
    std::fs::create_dir_all(&rel_dir).expect("actualiza-rota");

    let etc = root.join("rootfs/etc");
    let init = root.join("rootfs/bin/init");
    let init_bueno = std::fs::read(&init).expect("bin/init");
    // Un ELF que no lo es: el kernel resuelve el fichero, intenta lanzarlo y
    // falla. Se rompe **init** y no el kernel a propósito: así el fallo ocurre
    // ya con la pareja aplicada, que es el momento que hay que probar.
    std::fs::write(&init, b"esto no es un ELF
").expect("init roto");
    std::fs::write(
        etc.join("soso-release"),
        format!("version={VER_ROTA}
build=rota
fecha=2026-09-18
"),
    )
    .expect("soso-release rota");

    let empaquetado = pack_rootfs_con(&root.join("rootfs"), |rel| {
        // Ni el firmware de la GPU (127 MB que el test no necesita) ni las
        // **otras releases de prueba**: viven dentro del propio rootfs, así que
        // sin esto cada release empaquetaría a las anteriores y la tercera
        // ocuparía el triple que la primera —hasta llenar el disco a mitad de
        // descarga—.
        !rel.starts_with("lib/firmware/") && !rel.starts_with("var/actualiza")
    });
    // Pase lo que pase, el rootfs vuelve a tener su init de verdad: se empaqueta
    // entero en la imagen justo después.
    std::fs::write(&init, &init_bueno).expect("init restaurado");
    crate::version::write_soso_release(root);
    let (pack_blob, files) = empaquetado.expect("pack roto");

    // El kernel es el mismo que ya corre: lo que se prueba es un rootfs que no
    // arranca, no un kernel que no arranca.
    let kernel_src = root.join("target/kernel/x86_64-soso/debug/kernel");
    let kernel_pub = rel_dir.join("kernel-x86_64");
    std::fs::copy(&kernel_src, &kernel_pub).expect("kernel copy");
    crate::release::strip_kernel(&kernel_pub);
    let kernel_bytes = std::fs::read(&kernel_pub).expect("kernel stripped");
    std::fs::write(rel_dir.join("rootfs.pack"), &pack_blob).expect("pack");

    let manifest = Manifest {
        version: parse_semver(VER_ROTA).unwrap(),
        version_raw: VER_ROTA.into(),
        build: "rota".into(),
        fecha: "2026-09-18".into(),
        kernel_hash: hex_sha256(&kernel_bytes),
        kernel_size: kernel_bytes.len() as u64,
        pack_hash: hex_sha256(&pack_blob),
        pack_size: pack_blob.len() as u64,
        compat: Some(soso_update_core::Compat {
            arch: "x86_64".into(),
            perfil: "live-usb".into(),
            drivers: vec![
                "usb".into(),
                "nvme".into(),
                "virtio-blk".into(),
                "live-disk".into(),
            ],
            abi: soso_abi::ABI_VERSION,
            fs: soso_update_core::compat::FS_FORMATO.into(),
            min_shim: soso_update_core::compat::SHIM_VERSION,
            min_recuperador: soso_update_core::compat::RECUPERADOR_VERSION,
        }),
        files,
    };
    std::fs::write(rel_dir.join("manifest.txt"), manifest.format()).expect("manifest roto");
    println!("test-update: release que no arranca en rootfs/var/actualiza-rota/");
}

/// La tercera release de la cadena: cambia el mismo fichero que la segunda,
/// para que volver de C a B se note en el **contenido** y no sólo en el número.
fn preparar_release_c(root: &Path) {
    let rel_dir = root.join("rootfs/var/actualiza-c");
    let _ = std::fs::remove_dir_all(&rel_dir);
    std::fs::create_dir_all(&rel_dir).expect("actualiza-c");

    let etc = root.join("rootfs/etc");
    let marca_path = etc.join("actualiza-marca.txt");
    std::fs::write(
        etc.join("soso-release"),
        format!("version={VER_C}\nbuild=tercera\nfecha=2026-09-18\n"),
    )
    .expect("soso-release c");
    std::fs::write(&marca_path, MARCA_C).expect("marca c");
    let empaquetado = pack_rootfs_con(&root.join("rootfs"), |rel| {
        // Ni el firmware de la GPU (127 MB que el test no necesita) ni las
        // **otras releases de prueba**: viven dentro del propio rootfs, así que
        // sin esto cada release empaquetaría a las anteriores y la tercera
        // ocuparía el triple que la primera —hasta llenar el disco a mitad de
        // descarga—.
        !rel.starts_with("lib/firmware/") && !rel.starts_with("var/actualiza")
    });
    std::fs::write(&marca_path, MARCA_VIEJA).expect("marca vieja");
    crate::version::write_soso_release(root);
    let (pack_blob, files) = empaquetado.expect("pack c");

    let kernel_src = root.join("target/kernel/x86_64-soso/debug/kernel");
    let kernel_pub = rel_dir.join("kernel-x86_64");
    std::fs::copy(&kernel_src, &kernel_pub).expect("kernel copy");
    crate::release::strip_kernel(&kernel_pub);
    let kernel_bytes = std::fs::read(&kernel_pub).expect("kernel stripped");
    std::fs::write(rel_dir.join("rootfs.pack"), &pack_blob).expect("pack");

    let manifest = Manifest {
        version: parse_semver(VER_C).unwrap(),
        version_raw: VER_C.into(),
        build: "tercera".into(),
        fecha: "2026-09-18".into(),
        kernel_hash: hex_sha256(&kernel_bytes),
        kernel_size: kernel_bytes.len() as u64,
        pack_hash: hex_sha256(&pack_blob),
        pack_size: pack_blob.len() as u64,
        compat: Some(soso_update_core::Compat {
            arch: "x86_64".into(),
            perfil: "live-usb".into(),
            drivers: vec![
                "usb".into(),
                "nvme".into(),
                "virtio-blk".into(),
                "live-disk".into(),
            ],
            abi: soso_abi::ABI_VERSION,
            fs: soso_update_core::compat::FS_FORMATO.into(),
            min_shim: soso_update_core::compat::SHIM_VERSION,
            min_recuperador: soso_update_core::compat::RECUPERADOR_VERSION,
        }),
        files,
    };
    std::fs::write(rel_dir.join("manifest.txt"), manifest.format()).expect("manifest c");
    println!("test-update: tercera release en rootfs/var/actualiza-c/");
}

/// §3.6: A→B→C conserva **dos** puntos a propósito —el de A y el de B—, y
/// volver desde C tiene que devolver B entero, contenido incluido.
fn fase_cadena(
    code: &Path,
    vars: &Path,
    live: &Path,
    dir: &Path,
    key: &Path,
) -> Result<String, String> {
    let s1 = dir.join("cadena-1.log");
    let _ = std::fs::remove_file(&s1);
    {
        let qemu = lanzar_live(code, vars, live, &s1)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s1, "sosh —", Duration::from_secs(300))?;
        ssh_guion_hasta(
            key,
            SSH_PORT,
            "soso-update aplicar --local /var/actualiza-prueba --forzar
halt
",
            Duration::from_secs(300),
            "soso-update: listo",
        )?;
    }

    // B aplicada y acreditada; ahora se arma C. Con C armada tienen que
    // convivir dos puntos: el de A y el de B. Soltar el de A aquí sería
    // quedarse sin camino de vuelta si C resulta ser la mala.
    let s2 = dir.join("cadena-2.log");
    let _ = std::fs::remove_file(&s2);
    let salida = {
        let qemu = lanzar_live(code, vars, live, &s2)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s2, "sosh —", Duration::from_secs(300))?;
        ssh_guion_hasta(
            key,
            SSH_PORT,
            "soso-update aplicar --local /var/actualiza-c --forzar
soso-update estado
halt
",
            Duration::from_secs(300),
            "buzón:",
        )?
    };
    let puntos = salida
        .lines()
        .filter(|l| l.trim_start().starts_with("vuelta atrás: "))
        .count();
    if puntos != 2 {
        return Err(format!(
            "con C armada tenían que convivir dos puntos y hay {puntos}: {salida:?}"
        ));
    }
    if !salida.contains(&format!("vuelta atrás: {TEST_VER}")) {
        return Err(format!("falta el punto de B ({TEST_VER}): {salida:?}"));
    }

    // C aplicada; y desde C se vuelve a B, no a A.
    let s3 = dir.join("cadena-3.log");
    let _ = std::fs::remove_file(&s3);
    {
        let qemu = lanzar_live(code, vars, live, &s3)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s3, "sosh —", Duration::from_secs(300))?;
        let salida = ssh_guion_hasta(
            key,
            SSH_PORT,
            "soso-update estado
soso-update revertir --yes
halt
",
            Duration::from_secs(300),
            "reinicia y volverás",
        )?;
        if !salida.contains(&format!("rootfs: {VER_C}")) {
            return Err(format!("C no quedó aplicada: {salida:?}"));
        }
        if !salida.contains(&format!("volverás a {TEST_VER}")) {
            return Err(format!("la vuelta atrás no apunta a B: {salida:?}"));
        }
    }

    let s4 = dir.join("cadena-4.log");
    let _ = std::fs::remove_file(&s4);
    let qemu = lanzar_live(code, vars, live, &s4)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(&s4, "sosh —", Duration::from_secs(300))?;
    let salida = ssh_guion_hasta(
        key,
        SSH_PORT,
        "cat /etc/actualiza-marca.txt
soso-update estado
halt
",
        Duration::from_secs(300),
        &format!("rootfs: {TEST_VER}"),
    )?;
    if !salida.contains(&format!("rootfs: {TEST_VER}")) {
        return Err(format!("no volvió a B: {salida:?}"));
    }
    // El contenido, no sólo el número: B y C cambian el mismo fichero.
    if !salida.contains(MARCA_NUEVA.trim()) || salida.contains(MARCA_C.trim()) {
        return Err(format!("el fichero no volvió al contenido de B: {salida:?}"));
    }
    Ok(format!("dos puntos con C armada; de C se vuelve a {TEST_VER}"))
}

/// Un corte **entre las dos escrituras durables de la confirmación**: el diario
/// ya dice «confirmado» y la ESP todavía dice «probando».
///
/// Es el caso que más caro sale si la tabla se equivoca: el arranque siguiente
/// vería «probando» y desharía una actualización que había ido bien, por haber
/// perdido la corriente un segundo después de acertar.
fn fase_corte_confirmacion(
    code: &Path,
    vars: &Path,
    live: &Path,
    dir: &Path,
    key: &Path,
) -> Result<String, String> {
    let s1 = dir.join("confirma-1.log");
    let _ = std::fs::remove_file(&s1);
    {
        let qemu = lanzar_live(code, vars, live, &s1)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s1, "sosh —", Duration::from_secs(300))?;
        ssh_guion_hasta(
            key,
            SSH_PORT,
            "soso-update aplicar --local /var/actualiza-prueba --forzar
halt
",
            Duration::from_secs(300),
            "soso-update: listo",
        )?;
    }

    // Aplica y se acredita: diario y ESP quedan en «confirmado».
    let s2 = dir.join("confirma-2.log");
    let _ = std::fs::remove_file(&s2);
    {
        let qemu = lanzar_live(code, vars, live, &s2)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s2, "sosh —", Duration::from_secs(300))?;
        esperar_en_fichero(&s2, "txn: pareja confirmada", Duration::from_secs(120))?;
        // El marcador tiene que imprimirlo **la sesión**: «apagando» lo dice el
        // kernel por la consola serie y llega al canal SSH o no, según quién
        // gane la carrera con el cierre del socket.
        ssh_guion_hasta(
            key,
            SSH_PORT,
            "soso-update estado
halt
",
            Duration::from_secs(120),
            "rootfs:",
        )?;
    }

    // Y ahora el corte: se rebobina **sólo** la ESP a «probando», que es el
    // estado en que la deja perder la corriente justo después de anotar la
    // confirmación en el diario.
    let p1 = esp_p1(live);
    let raw = crate::fat32_write::read_root_file(live, p1, b"SOSOTXN BIN")
        .map_err(|e| format!("leer SOSOTXN.BIN: {e}"))?;
    let rec = soso_update_core::txn::bootrec::BootRecord::pick(&raw)
        .map_err(|e| format!("registro ilegible: {e:?}"))?;
    if rec.decision != soso_update_core::txn::bootrec::Decision::Confirmado {
        return Err(format!("esperaba «confirmado» y hay «{}»", rec.decision.as_str()));
    }
    let atras = soso_update_core::txn::bootrec::BootRecord::nuevo(
        soso_update_core::txn::bootrec::Decision::Probando,
        rec.id,
        &rec.version_nueva,
        &rec.version_anterior,
        rec.seq + 1,
    );
    let atras = match rec.punto {
        Some(p) => atras.con_punto(p),
        None => atras,
    };
    let bytes = atras.format().map_err(|e| format!("formatear: {e:?}"))?;
    let mut fichero = raw.clone();
    let off = atras.ranura() * soso_update_core::SLOT_SIZE;
    fichero[off..off + bytes.len()].copy_from_slice(&bytes);
    crate::fat32_write::overwrite_in_dir(live, p1, &[], b"SOSOTXN BIN", &fichero)
        .map_err(|e| format!("escribir SOSOTXN.BIN: {e}"))?;

    // El arranque tiene que **completar** la confirmación, no deshacerla.
    let s3 = dir.join("confirma-3.log");
    let _ = std::fs::remove_file(&s3);
    let qemu = lanzar_live(code, vars, live, &s3)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(&s3, "sosh —", Duration::from_secs(300))?;
    let serie = std::fs::read_to_string(&s3).unwrap_or_default();
    if serie.contains("txn: actualización revertida") {
        return Err("deshizo una actualización que había ido bien".into());
    }
    if !serie.contains("CompletarConfirmacion") {
        return Err(format!(
            "no completó la confirmación: {:?}",
            serie.lines().filter(|l| l.starts_with("txn:")).collect::<Vec<_>>()
        ));
    }
    let salida = ssh_guion_hasta(
        key,
        SSH_PORT,
        "soso-update estado
halt
",
        Duration::from_secs(120),
        &format!("rootfs: {TEST_VER}"),
    )?;
    if !salida.contains(&format!("rootfs: {TEST_VER}")) {
        return Err(format!("no se quedó en {TEST_VER}: {salida:?}"));
    }
    Ok(format!("sigue en {TEST_VER}"))
}

/// Qué pasa cuando falla **la propia recuperación**: con un respaldo que ya no
/// cuadra con su hash, deshacer dejaría la pareja mezclada. El arranque tiene
/// que plantarse y decirlo, no seguir adelante.
fn fase_respaldo_roto(
    code: &Path,
    vars: &Path,
    live: &Path,
    dir: &Path,
    key: &Path,
) -> Result<String, String> {
    // 1) Armar la release que no arranca: así la reversión ocurre sola.
    let s1 = dir.join("respaldo-1.log");
    let _ = std::fs::remove_file(&s1);
    {
        let qemu = lanzar_live(code, vars, live, &s1)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s1, "sosh —", Duration::from_secs(300))?;
        ssh_guion_hasta(
            key,
            SSH_PORT,
            "soso-update aplicar --local /var/actualiza-rota --forzar
halt
",
            Duration::from_secs(300),
            "soso-update: listo",
        )?;
    }

    // 2) Se estropea un respaldo dentro del sosofs de la imagen: es lo que ve
    //    el arranque cuando un sector se va o un corte deja la copia a medias.
    let estropeado = {
        let (lba, ultimo) = crate::package_live::partition_range(live, 2)
            .ok_or("sin partición 2 en la imagen")?;
        let mut fs = crate::sosofs_img::montar(live, lba, ultimo + 1 - lba)?;
        let op = crate::sosofs_img::dir_operacion(&mut fs)
            .ok_or("no encuentro la operación armada en el disco")?;
        let respaldo = format!("/var/lib/soso-update/{op}/respaldo");
        let fichero = crate::sosofs_img::primer_fichero(&mut fs, &respaldo)
            .ok_or("la operación no dejó respaldos")?;
        crate::sosofs_img::estropear(&mut fs, &fichero)?
    };

    // 3) Aplica (la release rota no arranca) y 4) al intentar deshacer, el
    //    respaldo no cuadra: diagnóstico, no un arranque a medias.
    let s2 = dir.join("respaldo-2.log");
    let _ = std::fs::remove_file(&s2);
    {
        let qemu = lanzar_live(code, vars, live, &s2)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s2, "fallo lanzando /bin/init", Duration::from_secs(300))?;
    }

    let s3 = dir.join("respaldo-3.log");
    let _ = std::fs::remove_file(&s3);
    let qemu = lanzar_live(code, vars, live, &s3)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(&s3, "no arranco con una pareja a medias", Duration::from_secs(300))?;
    let serie = std::fs::read_to_string(&s3).unwrap_or_default();
    if !serie.contains("txn: fallo") {
        return Err(format!(
            "no dijo por qué no podía deshacer: {:?}",
            serie.lines().filter(|l| l.starts_with("txn:")).collect::<Vec<_>>()
        ));
    }
    // Y no lanza userspace: arrancar con media versión puesta sería justo lo
    // que el contrato evita.
    if serie.contains("sosh — escribe") {
        return Err("arrancó la shell con la pareja mezclada".into());
    }
    Ok(format!("diagnóstico tras estropear {estropeado}"))
}

/// §3.6: «si el arranque nuevo no funciona, el siguiente lo deshace solo».
fn fase_init_roto(
    code: &Path,
    vars: &Path,
    live: &Path,
    dir: &Path,
    key: &Path,
    ver_base: &str,
) -> Result<String, String> {
    // 1) Armar la release que no arranca.
    let s1 = dir.join("roto-1.log");
    let _ = std::fs::remove_file(&s1);
    {
        let qemu = lanzar_live(code, vars, live, &s1)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s1, "sosh —", Duration::from_secs(300))?;
        ssh_guion_hasta(
            key,
            SSH_PORT,
            "soso-update aplicar --local /var/actualiza-rota --forzar
halt
",
            Duration::from_secs(300),
            "soso-update: listo",
        )?;
    }

    // 2) Se aplica… y no arranca. Nadie la acredita, y aquí no hay SSH al que
    //    pedirle nada: la máquina se queda en la consola de emergencia.
    let s2 = dir.join("roto-2.log");
    let _ = std::fs::remove_file(&s2);
    {
        let qemu = lanzar_live(code, vars, live, &s2)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s2, "fallo lanzando /bin/init", Duration::from_secs(300))?;
        let serie = std::fs::read_to_string(&s2).unwrap_or_default();
        if !serie.contains("txn: actualización aplicada") {
            return Err("no llegó a aplicar la release rota".into());
        }
    }

    // 3) El encendido siguiente la deshace **solo**: nadie ha pedido nada.
    let s3 = dir.join("roto-3.log");
    let _ = std::fs::remove_file(&s3);
    let qemu = lanzar_live(code, vars, live, &s3)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(&s3, "sosh —", Duration::from_secs(300))?;
    let serie = std::fs::read_to_string(&s3).unwrap_or_default();
    if !serie.contains("txn: actualización revertida") {
        return Err(format!(
            "no deshizo la actualización sin que nadie se lo pidiera: {:?}",
            serie.lines().filter(|l| l.starts_with("txn:")).collect::<Vec<_>>()
        ));
    }
    // Y nadie confirma lo que se acaba de deshacer: si la ESP se quedara en
    // «probando», el primer programa que salde la acreditación daría por buena
    // la versión que no arranca.
    if serie.contains("txn: pareja confirmada") {
        return Err("confirmó la versión que acababa de deshacer".into());
    }
    let salida = ssh_guion_hasta(
        key,
        SSH_PORT,
        "soso-update estado
halt
",
        Duration::from_secs(120),
        &format!("rootfs: {ver_base}"),
    )?;
    if !salida.contains(&format!("rootfs: {ver_base}")) {
        return Err(format!("no volvió a {ver_base}: {salida:?}"));
    }
    Ok(format!("volvió a {ver_base} sin que nadie lo pidiera"))
}

fn fase_aplicar(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
) -> Result<String, String> {
    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live(code, vars, live, serial)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(300))?;
    let salida = ssh_guion_hasta(
        key,
        SSH_PORT,
        // `init log` deja un registro por fd 3 y `sosolog` fuerza el volcado a
        // /var/log: así el arranque 2 puede comprobar que sobrevivieron.
        // Con la operación armada, las rutas que administra la release no se
        // tocan (U5) y el resto del disco sigue siendo tuyo. Y una segunda
        // instancia del cliente tiene que decirlo, no ponerse a armar encima.
        "soso-update aplicar --local /var/actualiza-prueba --forzar\n         echo intruso > /bin/prueba-exclusion\n         echo mias > /var/prueba-exclusion\n         cat /var/prueba-exclusion\n         soso-update aplicar --local /var/actualiza-prueba --forzar\nsoso-update transicion\ninit log marca-u1-aplicar\nsosolog\nhalt\n",
        Duration::from_secs(180),
        "soso-update: listo",
    )?;
    if !salida.contains("listo") {
        return Err(format!("aplicar no terminó bien: {salida:?}"));
    }
    if !salida.contains("etc/actualiza-marca.txt") {
        return Err(format!("no actualizó el fichero cambiado: {salida:?}"));
    }
    // U5b: antes de tocar nada tiene que haber una vuelta atrás **verificada**.
    // Sin punto no se actualiza, así que esto no es informativo: es la puerta.
    if !salida.contains("verificada") || !salida.contains("punto: vuelta atrás") {
        return Err(format!(
            "aplicó sin dejar un punto de recuperación verificado: {salida:?}"
        ));
    }
    // U5, exclusión de escritores: entre el respaldo y el reinicio nadie
    // reescribe lo que el punto acaba de copiar. Si se dejara, deshacer no
    // devolvería el sistema a un estado que existió: lo machacaría.
    if !salida.contains("actualización en curso; esa ruta no se toca") {
        return Err(format!(
            "se pudo escribir en /bin con la actualización armada: {salida:?}"
        ));
    }
    // Y lo que no administra la release sigue siendo del usuario: una
    // exclusión que deje la máquina de solo lectura no sirve de nada.
    if !salida.contains("mias") {
        return Err(format!(
            "la exclusión bloqueó también /var, que no es suyo: {salida:?}"
        ));
    }
    // Dos operaciones a la vez sobre las mismas rutas no tienen arreglo: la
    // segunda se planta con un mensaje que se entiende.
    if !salida.contains("ya hay una actualización en curso") {
        return Err(format!(
            "una segunda instancia se puso a armar encima de la primera: {salida:?}"
        ));
    }

    // U6: esta máquina tiene todos los huecos, así que la transición no tiene
    // nada que decir. La prueba está aquí para que, el día que un hueco cambie
    // de tamaño o desaparezca del empaquetado, se entere alguien.
    if !salida.contains("transición: nada que hacer") {
        return Err(format!(
            "la transición ve la imagen live como incompleta: {salida:?}"
        ));
    }

    // La gracia de la actualización parcial: se piden unos pocos ficheros, no
    // los 60 y pico del pack.
    if let Some(l) = salida.lines().find(|l| l.trim_start().starts_with("rootfs:"))
        && let Some(n) = l.split_whitespace().nth(1).and_then(|n| n.parse::<usize>().ok())
        && n > 5
    {
        return Err(format!("descargó {n} ficheros, esperaba unos pocos: {l}"));
    }
    Ok(salida)
}

fn fase_comprobar_version(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
    ver: &str,
) -> Result<(), String> {
    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live(code, vars, live, serial)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(300))?;
    // Esta fase da por hecho que la pareja quedó **acreditada**: hasta
    // entonces la exclusión sigue puesta —el arranque siguiente aún podría
    // tener que deshacerla— y el `echo` de más abajo no podría ensuciar nada.
    // Esperarlo en el serial es además la única prueba de que init acredita.
    esperar_en_fichero(serial, "txn: pareja confirmada", Duration::from_secs(60))?;
    let salida = ssh_guion_hasta(
        key,
        SSH_PORT,
        // El `cat` va primero: la última línea del guion se pierde a veces al
        // cerrar la sesión SSH, y la salida de `estado` sirve de barrera.
        // `halt` apaga el guest y SSH se cuelga: cortar al ver `rootfs:`.
        // Los `cat`/`grep` de los logs van **antes** de `estado` por el mismo
        // motivo que la marca: la lectura se corta al ver `rootfs:`. Y se usa
        // `grep` en kernel.log porque el fichero acumulado son decenas de KiB
        // que no hacen falta enteros por la sesión SSH.
        // Sin comillas: el tokenizador de sosh no las interpreta y «grep "a b"»
        // acaba buscando el fichero `b"`.
        // U4: se ensucia la marca para que el fichero vuelva a hacer falta, y
        // se reaplica. Lo bajado y verificado en el arranque anterior sigue en
        // el área de preparación, así que tiene que reanudar en vez de pedirlo
        // otra vez — y eso cruza un reinicio, que es lo que exige el criterio.
        "cat /etc/actualiza-marca.txt\ncat /var/log/aplicaciones.log\ngrep flujo /var/log/kernel.log\necho sucia > /etc/actualiza-marca.txt\nsoso-update aplicar --local /var/actualiza-prueba --forzar\nsoso-update estado\nhalt\n",
        Duration::from_secs(120),
        &format!("rootfs: {ver}"),
    )?;
    // Ojo: `estado` también imprime el fichero de progreso "APLICANDO <ver>"
    // que queda si `aplicar` se cortó a medias, así que exigimos la línea del
    // rootfs; si no, un fallo a mitad se colaría como éxito.
    if !salida.contains(&format!("rootfs: {ver}")) {
        return Err(format!("estado no muestra rootfs {ver}: {salida:?}"));
    }
    if !salida.contains(MARCA_NUEVA.trim()) {
        return Err(format!(
            "el fichero actualizado no sobrevivió al reinicio: {salida:?}"
        ));
    }
    // El USB de este test es un `usb-storage` real de QEMU (no virtio): el
    // disco de arranque debe reconocerse como tal por DISK_KIND_USB.
    if !salida.contains("arranque: USB live") {
        return Err(format!(
            "estado no reconoce el medio de arranque USB: {salida:?}"
        ));
    }
    // U1: los logs nativos sobreviven al reinicio. El registro de fd 3 lo
    // escribió el arranque anterior; si `/var/log` no persistiera, aquí no
    // quedaría rastro de él.
    if !salida.contains("marca-u1-aplicar") {
        return Err(format!(
            "el registro de fd 3 del arranque anterior no sobrevivió en /var/log/aplicaciones.log: {salida:?}"
        ));
    }
    // Y el log de kernel acumula una cabecera por arranque, no se reescribe.
    let cabeceras = salida.matches("flujo kernel.log").count();
    if cabeceras < 2 {
        return Err(format!(
            "kernel.log debería tener una cabecera por arranque, encontré {cabeceras}: {salida:?}"
        ));
    }
    // U5c: quien instaló fue el **recuperador del kernel**, antes de cargar
    // firmware y antes de `/bin/init`, no el cliente. Si esto desaparece, la
    // actualización habrá vuelto a ser «el programa escribe /bin» sin que
    // ninguna otra comprobación se entere.
    let serie = std::fs::read_to_string(serial).unwrap_or_default();
    if !serie.contains("txn: actualización aplicada") {
        return Err("la transacción no la aplicó el recuperador del kernel".into());
    }
    let (antes, _) = serie
        .split_once("txn: actualización aplicada")
        .unwrap_or((&serie, ""));
    if antes.contains("boot: ethernet") || antes.contains("task: /bin/init lanzado") {
        return Err("el recuperador corrió después del firmware o de init".into());
    }

    // U5a/U5b: tras reiniciar, `estado` ofrece una vuelta atrás y dice que esa
    // copia está **comprobada**. No se fija la versión a propósito: al
    // reaplicar, el punto pasa a reflejar la que estuviera activa entonces, que
    // es justo lo que debe guardar.
    if !salida.contains("vuelta atrás: ") || salida.contains("vuelta atrás: ninguna") {
        return Err(format!("estado no ofrece ninguna vuelta atrás: {salida:?}"));
    }
    if !salida.contains("verificada") || salida.contains("INCOMPLETA") {
        return Err(format!("el punto guardado no se relee entero: {salida:?}"));
    }

    // U4: la etapa sobrevivió al reinicio y no se vuelve a descargar nada.
    if !salida.contains("ya descargados, reanudando") {
        return Err(format!(
            "la segunda aplicación no reanudó desde el área de preparación: {salida:?}"
        ));
    }
    Ok(())
}

/// Simula un corte tras escribir el backup: meta `applying`, kernel activo corrupto.
/// U5d: vuelta atrás manual. Sobre una copia limpia: armar, aplicar y luego
/// `revertir`, y comprobar que el arranque siguiente deja la versión anterior
/// **y** el fichero que la actualización había cambiado.
fn fase_vuelta_atras(
    code: &Path,
    vars: &Path,
    live: &Path,
    dir: &Path,
    key: &Path,
    ver_base: &str,
) -> Result<(), String> {
    // 1) Armar.
    let s1 = dir.join("vuelta-1.log");
    let _ = std::fs::remove_file(&s1);
    {
        let qemu = lanzar_live(code, vars, live, &s1)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s1, "sosh —", Duration::from_secs(300))?;
        ssh_guion_hasta(
            key,
            SSH_PORT,
            "soso-update aplicar --local /var/actualiza-prueba --forzar
halt
",
            Duration::from_secs(180),
            "soso-update: listo",
        )?;
    }

    // 2) Se aplica sola al arrancar; después se pide la vuelta atrás.
    let s2 = dir.join("vuelta-2.log");
    let _ = std::fs::remove_file(&s2);
    {
        let qemu = lanzar_live(code, vars, live, &s2)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s2, "sosh —", Duration::from_secs(300))?;
        let salida = ssh_guion_hasta(
            key,
            SSH_PORT,
            "soso-update revertir --yes
halt
",
            Duration::from_secs(120),
            "reinicia y volverás",
        )?;
        // No se promete nada sin haber releído la copia.
        if !salida.contains("copia verificada") {
            return Err(format!("revertir no verificó el punto antes: {salida:?}"));
        }
        // La pareja nueva se acredita **antes** de registrar la vuelta atrás.
        // Si no, init y el cliente componen cada uno su registro desde la misma
        // lectura, escriben en la misma ranura y uno de los dos se pierde: la
        // vuelta atrás que acabamos de prometer, con suerte.
        let serie = std::fs::read_to_string(&s2).unwrap_or_default();
        if !serie.contains("txn: pareja confirmada") {
            return Err("la pareja no se acreditó antes de pedir la vuelta atrás".into());
        }
    }

    // 3) Y al arrancar, la versión anterior entera.
    // Cada arranque va en su bloque: el guardián del anterior tiene que caer
    // antes de lanzar el siguiente, o los dos QEMU se pelean por la imagen y el
    // segundo muere sin llegar a escribir su log de serie.
    let s3 = dir.join("vuelta-3.log");
    let _ = std::fs::remove_file(&s3);
    {
        let qemu = lanzar_live(code, vars, live, &s3)?;
        let _guard = Matar(qemu.child);
        esperar_en_fichero(&s3, "sosh —", Duration::from_secs(300))?;
        let serie = std::fs::read_to_string(&s3).unwrap_or_default();
        if !serie.contains("txn: restaurada la versión") {
            return Err("el arranque no restauró el punto".into());
        }
        let salida = ssh_guion_hasta(
            key,
            SSH_PORT,
            "cat /etc/actualiza-marca.txt
soso-update estado
halt
",
            Duration::from_secs(120),
            &format!("rootfs: {ver_base}"),
        )?;
        if !salida.contains(MARCA_VIEJA.trim()) {
            return Err(format!(
                "el fichero no volvió a su contenido anterior: {salida:?}"
            ));
        }
        if !salida.contains(&format!("rootfs: {ver_base}")) {
            return Err(format!("la versión no volvió a {ver_base}: {salida:?}"));
        }
        // La restaurada también se acredita: sin esto la vuelta atrás se queda
        // «a prueba» para siempre y el arranque siguiente la da por fallida.
        let serie = std::fs::read_to_string(&s3).unwrap_or_default();
        if !serie.contains("txn: versión restaurada acreditada") {
            return Err("la versión restaurada no llegó a acreditarse".into());
        }
    }

    // U5e: la versión restaurada queda **a prueba** hasta que ese arranque se
    // acredita. Un cuarto arranque tiene que encontrarlo ya cerrado y seguir
    // como si nada; si siguiera a prueba, el sistema diría que no arranca.
    let s4 = dir.join("vuelta-4.log");
    let _ = std::fs::remove_file(&s4);
    let qemu = lanzar_live(code, vars, live, &s4)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(&s4, "sosh —", Duration::from_secs(300))?;
    let serie4 = std::fs::read_to_string(&s4).unwrap_or_default();
    if serie4.contains("RestauradoNoArranca") {
        return Err("la restauración no se acreditó y el arranque la da por fallida".into());
    }
    if serie4.contains("txn: restaurada la versión") {
        return Err("volvió a restaurar: la vuelta atrás no quedó cerrada".into());
    }
    // Y el diario tiene que haber quedado cerrado con la ESP: si uno dice
    // «revertido» y el otro sigue en «probando», el arranque no sabe cuál manda.
    if serie4.contains("PAREJA INCOHERENTE") {
        return Err("el rescate dejó el diario y la ESP diciendo cosas distintas".into());
    }
    Ok(())
}

/// U6: desde el live, mirar la instalación de **otro** disco y dejarle pedida
/// la vuelta atrás. El disco ajeno es la copia que dejó la fase de vuelta
/// atrás: tiene un punto retenido de verdad, verificable.
fn fase_recuperar_ajeno(
    code: &Path,
    vars: &Path,
    live: &Path,
    origen: &Path,
    dir: &Path,
    key: &Path,
    serial: &Path,
    ver_base: &str,
) -> Result<String, String> {
    // La imagen ajena es la que quedó **actualizada** en las fases anteriores:
    // versión nueva corriendo y un punto retenido a la anterior. Es el estado
    // en que de verdad hace falta esto.
    let ajeno = dir.join("ajeno.img");
    crate::copy_sparse(origen, &ajeno);

    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live_con(code, vars, live, serial, Some(&ajeno))?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(300))?;
    let salida = ssh_guion_hasta(
        key,
        SSH_PORT,
        "soso-update recuperar
",
        Duration::from_secs(180),
        "copia verificada",
    )?;
    // Lo que importa: encontró la otra instalación, leyó su registro en una ESP
    // que no es la suya, montó su sosofs y **verificó** la copia antes de
    // ofrecer nada.
    if !salida.contains("vuelta atrás guardada") {
        return Err(format!("no encontró el punto del otro disco: {salida:?}"));
    }
    if !salida.contains("copia verificada") {
        return Err(format!("ofreció una vuelta atrás sin comprobarla: {salida:?}"));
    }
    let id = salida
        .lines()
        .find_map(|l| l.trim().strip_prefix("--- disco "))
        .and_then(|r| r.split_whitespace().next())
        .ok_or_else(|| format!("no enumeró ningún disco: {salida:?}"))?
        .to_string();
    // A qué versión dice que puede volver: es la que tendrá que estar corriendo
    // cuando ese disco arranque.
    let destino = salida
        .lines()
        .find_map(|l| l.trim().strip_prefix("vuelta atrás guardada: "))
        .and_then(|r| r.split_whitespace().next())
        .ok_or("no dijo a qué versión volvía")?
        .to_string();

    // Y ahora la reparación de verdad: restaurar desde aquí, que es la vía para
    // cuando el kernel de esa máquina ni siquiera arranca.
    let salida2 = ssh_guion_hasta(
        key,
        SSH_PORT,
        &format!("soso-update recuperar --disco {id} --restaurar\nhalt\n"),
        Duration::from_secs(180),
        "ya puede arrancar",
    )?;
    if !salida2.contains("restaurados") {
        return Err(format!("no restauró nada: {salida2:?}"));
    }
    drop(_guard);

    // La prueba que cuenta: esa imagen, arrancada por su cuenta, tiene que
    // estar en la versión anterior y acreditarla ella sola. Que el comando diga
    // que restauró no vale: lo que importa es que la máquina arranque así.
    let serial2 = dir.join("ajeno-2.log");
    let _ = std::fs::remove_file(&serial2);
    let qemu = lanzar_live(code, vars, &ajeno, &serial2)?;
    let _guard2 = Matar(qemu.child);
    esperar_en_fichero(&serial2, "sosh —", Duration::from_secs(300))?;
    let salida3 = ssh_guion_hasta(
        key,
        SSH_PORT,
        "soso-update estado
halt
",
        Duration::from_secs(120),
        &format!("rootfs: {destino}"),
    )?;
    if !salida3.contains(&format!("rootfs: {destino}")) {
        return Err(format!("el disco restaurado no arrancó en {destino}: {salida3:?}"));
    }
    let serie = std::fs::read_to_string(&serial2).unwrap_or_default();
    if serie.contains("txn: restaurada la versión") {
        return Err("volvió a restaurar: la reparación desde el live no quedó cerrada".into());
    }
    // Y sobre todo: el arranque no puede diagnosticar un fallo que no ha
    // ocurrido. Reparar desde fuera deja la máquina lista, no «a prueba»: nadie
    // ha intentado arrancarla todavía.
    if serie.contains("PAREJA INCOHERENTE") {
        return Err("el disco reparado desde el live arrancó dando diagnóstico".into());
    }
    let _ = ver_base;
    Ok(format!("restaurado desde el live; arranca en {destino}"))
}

/// El #PF de ring 0 no llega a SOSOLOG y el banco de HTTPS sólo miraba
/// «page fault de usuario». En placa el pánico era `EXCEPTION: page fault`
/// dentro de `talc` al **segundo** `connect` (redirect HTTP).
fn rastro_kernel_muerto(serie: &str) -> Option<String> {
    serie.lines().find(|l| {
        l.contains("EXCEPTION:")
            || l.contains("panicked at")
            || l.contains("!!! panic")
            || l.contains("page fault de usuario")
            || l.contains("BLOQUE DESBORDADO")
    }).map(|l| l.trim().to_string())
}

fn exigir_kernel_vivo(serial: &Path) -> Result<(), String> {
    let serie = std::fs::read_to_string(serial).unwrap_or_default();
    if let Some(linea) = rastro_kernel_muerto(&serie) {
        return Err(format!("el kernel reventó: {linea}"));
    }
    Ok(())
}

/// Abrir una conexión TCP **hacia fuera** desde un proceso de usuario.
///
/// Tres casos, y el del medio es el que `saliente` no cubría:
/// 1. Contra un servidor que escucha: conecta, manda y recibe.
/// 2. **Cierra y vuelve a conectar al mismo host:puerto.** Es el patrón de un
///    redirect HTTP (GitHub encadena varios para `releases/latest/download`).
///    Con una sola conexión no se ve ni el TIME_WAIT del puerto local ni un
///    `malloc` que recorre la lista de `talc` ya podrida.
/// 3. Contra un puerto cerrado: falla **a tiempo**. Un `connect` que se cuelga
///    para siempre es lo que dejó el OTA mudo quince minutos.
fn fase_tcp_saliente(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
) -> Result<String, String> {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // El eco vive en el host; el guest lo ve en 10.0.2.2 por slirp.
    let listener = TcpListener::bind("0.0.0.0:0").map_err(|e| format!("bind: {e}"))?;
    let puerto = listener
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?
        .port();
    // El servidor dice si **llegó a aceptar** una conexión: separa «el guest no
    // manda el SYN» de «lo manda y no procesa la respuesta». Con plazo: si
    // nadie conecta, un `accept` bloqueante colgaría la suite para siempre.
    // Dos aceptaciones: la segunda es el close+reconnect del redirect.
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("nonblocking: {e}"))?;
    let eco = std::thread::spawn(move || {
        let fin = std::time::Instant::now() + Duration::from_secs(240);
        let mut vistos = Vec::new();
        while std::time::Instant::now() < fin && vistos.len() < 2 {
            match listener.accept() {
                Ok((mut s, de)) => {
                    let _ = s.set_nonblocking(false);
                    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
                    let mut buf = [0u8; 256];
                    let n = s.read(&mut buf).unwrap_or(0);
                    let _ = s.write_all(&buf[..n]);
                    vistos.push(format!("{de} envió {n} B"));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => break,
            }
        }
        vistos
    });

    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live(code, vars, live, serial)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(300))?;
    let salida = match ssh_guion_hasta(
        key,
        SSH_PORT,
        &format!(
            "tcpconn 10.0.2.2 {puerto} hola-soso\n\
             tcpconn 10.0.2.2 {puerto} segunda\n\
             tcpconn 10.0.2.2 1 nadie --timeout 3000\n\
             halt\n"
        ),
        Duration::from_secs(180),
        "sin conexión",
    ) {
        Ok(s) => s,
        Err(e) => {
            exigir_kernel_vivo(serial)?;
            return Err(e);
        }
    };
    exigir_kernel_vivo(serial)?;
    let aceptadas = eco.join().unwrap_or_default();
    println!(
        "      eco del host: {} conexión(es) — {}",
        aceptadas.len(),
        if aceptadas.is_empty() {
            "NADIE conectó".into()
        } else {
            aceptadas.join("; ")
        }
    );

    let conectados = salida
        .lines()
        .filter(|l| l.contains("tcpconn: conectado"))
        .count();
    if conectados < 2 {
        return Err(format!(
            "hacían falta dos connects al mismo puerto (hubo {conectados}): {salida:?}"
        ));
    }
    if aceptadas.len() < 2 {
        return Err(format!(
            "el host sólo aceptó {} conexión(es): {salida:?}",
            aceptadas.len()
        ));
    }
    if !salida.contains("hola-soso") {
        return Err(format!("primera ida y vuelta falló: {salida:?}"));
    }
    if !salida.contains("segunda") {
        return Err(format!("segunda ida y vuelta falló: {salida:?}"));
    }
    // Y el puerto cerrado tiene que fallar **dentro** de su plazo.
    let linea = salida
        .lines()
        .find(|l| l.contains("sin conexión tras"))
        .ok_or_else(|| format!("el puerto cerrado no dio error: {salida:?}"))?;
    let ms: u64 = linea
        .split("tras ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(u64::MAX);
    if ms > 10_000 {
        return Err(format!("el plazo de 3 s no se respetó: tardó {ms} ms"));
    }
    Ok(format!(
        "dos idas y vueltas OK; puerto cerrado falla en {ms} ms"
    ))
}

/// `soso-update comprobar` contra el canal de verdad: DNS, TLS y descarga del
/// manifiesto. Lo que se comprueba no es que haya actualización —puede no
/// haberla— sino que el camino **no se estrella**.
fn fase_https(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
) -> Result<String, String> {
    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live(code, vars, live, serial)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(300))?;
    let salida = match ssh_guion_hasta(
        key,
        SSH_PORT,
        "soso-update comprobar --traza
echo FIN-COMPROBAR
halt
",
        // Generoso a propósito: lo que se quiere distinguir es «lento» de
        // «colgado», y con el límite corto los dos se parecen.
        Duration::from_secs(900),
        // El marcador es el `echo`, **no** «local:». Esperar por una línea que
        // sólo sale si el cliente tuvo éxito convierte cualquier fallo en una
        // espera de 900 s y un mensaje —«la sesión SSH no terminó»— que acusa
        // al SSH de algo que hizo la red.
        "FIN-COMPROBAR",
    ) {
        Ok(s) => s,
        Err(e) => {
            exigir_kernel_vivo(serial)?;
            return Err(e);
        }
    };
    exigir_kernel_vivo(serial)?;
    if !salida.contains("remoto:") {
        return Err(format!("no llegó a leer el manifiesto remoto: {salida:?}"));
    }
    Ok(salida
        .lines()
        .find(|l| l.trim_start().starts_with("remoto:"))
        .unwrap_or("")
        .trim()
        .to_string())
}

fn fase_recuperacion_corte(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
) -> Result<(), String> {
    let ver_base = crate::version::read_version(&crate::project_root());
    inyectar_corte_backup(live)?;

    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live(code, vars, live, serial)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(300))?;

    let log = std::fs::read_to_string(serial).unwrap_or_default();
    let p1 = esp_p1(live);
    let mark = crate::fat32_write::read_root_file(live, p1, b"BOOTMARKTXT").unwrap_or_default();
    let mark_s = String::from_utf8_lossy(&mark);
    let ok_serial = log.contains("recuperado tras corte") || log.contains("actualiza: recuperado");
    let ok_mark = mark_s.contains("recuperado tras corte");
    if !ok_serial && !ok_mark {
        return Err(format!(
            "sin recuperación en serial ni BOOTMARK; mark={mark_s:?} serial={:?}",
            log.lines()
                .filter(|l| l.contains("actualiza") || l.contains("soso-shim"))
                .take(8)
                .collect::<Vec<_>>()
        ));
    }

    let salida = ssh_guion_hasta(
        key,
        SSH_PORT,
        &format!("soso-update estado\nhalt\n"),
        Duration::from_secs(120),
        &format!("rootfs: {ver_base}"),
    )?;
    if !salida.contains(&format!("rootfs: {ver_base}")) {
        return Err(format!(
            "tras recuperar, rootfs debería seguir en {ver_base}: {salida:?}"
        ));
    }
    Ok(())
}

/// Manifiesto con path `..` debe fallar antes de mutar el rootfs.
fn fase_manifiesto_invalido(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    key: &Path,
) -> Result<(), String> {
    let _ = std::fs::remove_file(serial);
    let qemu = lanzar_live(code, vars, live, serial)?;
    let _guard = Matar(qemu.child);
    esperar_en_fichero(serial, "sosh —", Duration::from_secs(300))?;

    // sosh no tiene heredoc; el manifiesto malo va en el rootfs de prueba.
    let salida = ssh_guion_hasta(
        key,
        SSH_PORT,
        "cp /var/actualiza-prueba/manifest-malo.txt /var/actualiza-prueba/manifest.txt\n\
         soso-update aplicar --local /var/actualiza-prueba --forzar\n\
         cat /etc/actualiza-marca.txt\nhalt\n",
        Duration::from_secs(180),
        "manifest no válido",
    )?;
    if !salida.contains("manifest no válido") {
        return Err(format!("aplicar no rechazó el manifiesto: {salida:?}"));
    }
    if !salida.contains(MARCA_NUEVA.trim()) {
        return Err(format!(
            "el manifiesto inválido mutó el rootfs (marca cambió): {salida:?}"
        ));
    }
    Ok(())
}

fn inyectar_corte_backup(live: &Path) -> Result<(), String> {
    let p1 = esp_p1(live);
    let ((kname8, kext3), kernel) = esp_read_kernel(live, p1)?;
    let (backup_size, backup_hash) = backup_digest(&kernel);

    let mut slot = kernel.clone();
    slot.resize(UPD_KERNEL_SLOT_SIZE, 0);
    esp_write_root(live, p1, b"SOSOKRN ", b"BIN", &slot)?;

    let meta = KernelMeta {
        phase: KernelPhase::Applying,
        version: String::from(TEST_VER),
        new_size: 0,
        new_hash: String::new(),
        backup_size,
        backup_hash,
    };
    let mut meta_bytes = meta.format();
    meta_bytes.resize(KERNEL_META_SIZE, b'\n');
    esp_write_root(live, p1, b"SOSOKRN ", b"MET", &meta_bytes)?;

    let mut corrupt = kernel.clone();
    let n = corrupt.len().min(4096);
    corrupt[..n].fill(0xA5);
    esp_write_root(live, p1, &kname8, &kext3, &corrupt)?;

    let idle = Mailbox::format_idle();
    esp_write_root(live, p1, b"SOSOUPD ", b"TXT", &idle)?;
    Ok(())
}

fn esp_p1(live: &Path) -> u64 {
    crate::package_live::partition_first_sector(live, 1).expect("ESP p1 del live")
}

fn esp_write_root(
    live: &Path,
    p1: u64,
    name: &[u8; 8],
    ext: &[u8; 3],
    data: &[u8],
) -> Result<(), String> {
    crate::fat32_write::write_root_file(live, p1, name, ext, data)
}

fn esp_read_kernel(live: &Path, p1: u64) -> Result<(([u8; 8], [u8; 3]), Vec<u8>), String> {
    let files = crate::fat32_write::list_root_files(live, p1)?;
    let mut best: Option<([u8; 8], [u8; 3], u32)> = None;
    for (name11, size) in files {
        let label = String::from_utf8_lossy(&name11);
        if label.starts_with("SOSO") || label.starts_with("BOOTMARK") {
            continue;
        }
        if size < 1024 * 1024 {
            continue;
        }
        let mut n = [0u8; 8];
        let mut e = [0u8; 3];
        n.copy_from_slice(&name11[..8]);
        e.copy_from_slice(&name11[8..11]);
        if best.as_ref().map(|b| size > b.2).unwrap_or(true) {
            best = Some((n, e, size));
        }
    }
    let (n, e, _) = best.ok_or_else(|| "no encontré kernel-x86_64 en la ESP".to_string())?;
    let mut name11 = [0u8; 11];
    name11[..8].copy_from_slice(&n);
    name11[8..11].copy_from_slice(&e);
    let data = crate::fat32_write::read_root_file(live, p1, &name11)?;
    Ok(((n, e), data))
}

struct QemuProc {
    child: std::process::Child,
}

struct Matar(std::process::Child);

impl Drop for Matar {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn lanzar_live(code: &Path, vars: &Path, live: &Path, serial: &Path) -> Result<QemuProc, String> {
    lanzar_live_con(code, vars, live, serial, None)
}

/// Como `lanzar_live`, pero con **otro** disco colgado del NVMe: el de la
/// máquina a la que se va a mirar desde el live.
fn lanzar_live_con(
    code: &Path,
    vars: &Path,
    live: &Path,
    serial: &Path,
    ajeno: Option<&Path>,
) -> Result<QemuProc, String> {
    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args(["-machine", "q35", "-cpu", "max"])
        .args(["-m", "2048M"])
        .args(["-smp", "2"])
        .arg("-no-reboot")
        .args(["-display", "none"])
        .stdout(Stdio::null());
    cmd.args([
        "-drive",
        &format!("if=pflash,format=raw,readonly=on,file={}", code.display()),
    ]);
    cmd.args([
        "-drive",
        &format!("if=pflash,format=raw,file={}", vars.display()),
    ]);
    cmd.args(["-device", "qemu-xhci,id=xhci"]);
    cmd.args([
        "-drive",
        &format!("file={},format=raw,if=none,id=live0", live.display()),
    ]);
    cmd.args([
        "-device",
        "usb-storage,bus=xhci.0,port=1,drive=live0,bootindex=0",
    ]);
    if let Some(ajeno) = ajeno {
        cmd.args([
            "-drive",
            &format!("file={},format=raw,if=none,id=nvme0", ajeno.display()),
        ]);
        cmd.args(["-device", "nvme,serial=soso-ajeno,drive=nvme0,bootindex=1"]);
    }
    crate::apply_qemu_nic_with_ports(&mut cmd, SSH_PORT, SSH_PORT + 1, Some(&MAC.to_string()));
    cmd.args(["-serial", &format!("file:{}", serial.display())])
        .stderr(Stdio::null());
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    Ok(QemuProc { child })
}

fn marca(msg: &str, ok: bool) {
    let tag = if ok { "OK" } else { "FALLO" };
    println!("test-update: [{tag}] {msg}");
}

fn sangrar(s: &str) -> String {
    s.lines()
        .map(|l| format!("      {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod rastro_kernel {
    use super::rastro_kernel_muerto;

    #[test]
    fn ignora_un_arranque_sano() {
        let serie = "sosh — escribe 'help' para la ayuda\nnet: dhcp 10.0.2.15/24\n";
        assert_eq!(rastro_kernel_muerto(serie), None);
    }

    #[test]
    fn caza_el_pf_de_ring0_de_talc() {
        let serie = "\
lxdde: centinelas en #PF: 12 bloques vivos, 0 desbordados
!!! panic: panicked at src/arch/interrupts.rs:446:15:
EXCEPTION: page fault at 0x17a7e98 rip=0x1000043a5ed cs=0x8 err=0xe
";
        let linea = rastro_kernel_muerto(serie).expect("tenía que cazar el pánico");
        assert!(linea.contains("EXCEPTION:") || linea.contains("panicked at"));
    }

    #[test]
    fn caza_un_desbordamiento_de_kmalloc() {
        let serie = "lxdde: BLOQUE DESBORDADO en 0x444444441000 (64 B): centinela 0x41414141\n";
        assert!(rastro_kernel_muerto(serie)
            .unwrap()
            .contains("BLOQUE DESBORDADO"));
    }
}
