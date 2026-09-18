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
const MARCA_NUEVA: &str = "nueva\n";
const MARCA_VIEJA: &str = "vieja\n";

pub fn run(filtro: Option<&str>) {
    let quiere = |nombre: &str| filtro.is_none_or(|f| nombre.contains(f));
    let root = crate::project_root();
    let dir = root.join("target/test-update");
    std::fs::create_dir_all(&dir).expect("test-update dir");

    preparar_release_prueba(&root);

    unsafe {
        std::env::set_var("SOSO_QEMU_LIVE", "1");
        std::env::set_var("SOSO_QEMU_LIVE_USB", "1");
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

    if fallos > 0 {
        eprintln!("\ntest-update: {fallos} fallo(s)");
        std::process::exit(1);
    }
    println!("\ntest-update: actualización local OK (+ recuperación OTA)");
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
        !rel.starts_with("lib/firmware/")
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
