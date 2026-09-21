//! Descarga e instala actualizaciones de soso desde GitHub Releases.

#![no_std]
#![no_main]

extern crate alloc;

mod net;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use libsoso::{print, println, sys};
use soso_abi::{O_RDONLY, O_WRONLY, UPD_WHICH_KERNEL, UPD_WHICH_MAILBOX, UPD_WHICH_META};
use soso_update_core::canal::{self, Conf, Origen};
use soso_update_core::descarga::{self, Etapa};
use soso_update_core::txn::journal::Contenido;
use soso_update_core::txn::punto::{crear_punto, Almacen, Cambio, CrearError, Punto};
use soso_update_core::txn::bootrec::{BootRecord, Decision};
use soso_update_core::txn::journal::{Accion, Entrada, Journal, Progreso};
use soso_update_core::txn::TxnState;
use soso_update_core::txn::TxnId;
use soso_update_core::compat::{CompatError, Equipo};
use soso_update_core::hash::{hex_sha256, Hasher};
use soso_update_core::kernel_meta::KernelMeta;
use soso_update_core::manifest::{self, FileEntry, Manifest};
use soso_update_core::migracion;
use soso_update_core::mailbox::Mailbox;
use soso_update_core::plan::{self, Span};
use soso_update_core::semver::{self, SemVer};

libsoso::entry!(main);

const SECTOR: usize = 512;
const CHUNK: usize = 64 * 1024;
/// Buffer de lectura para hashear ficheros ya instalados.
const HASH_BUF: usize = 256 * 1024;

fn main(args: &str) -> u8 {
    let mut parts: Vec<String> = args.split_whitespace().map(String::from).collect();
    // `--traza` vale para cualquier subcomando y se quita antes de repartir.
    if let Some(i) = parts.iter().position(|p| p == "--traza") {
        parts.remove(i);
        net::encender_traza();
    }
    let cmd = parts.first().map(|s| s.as_str()).unwrap_or("aplicar");
    let rest: &[String] = parts.get(1..).unwrap_or(&[]);
    saldar_acreditacion();
    match cmd {
        "estado" => cmd_estado(),
        "comprobar" => cmd_comprobar(rest),
        "aplicar" => cmd_aplicar(rest),
        "revertir" => cmd_revertir(rest),
        "transicion" | "transición" => cmd_transicion(rest),
        "recuperar" => cmd_recuperar(rest),
        "help" | "--help" | "-h" => {
            print_usage();
            0
        }
        other if other.starts_with('-') => cmd_aplicar(&parts),
        other => {
            println!("soso-update: subcomando desconocido {other}");
            print_usage();
            2
        }
    }
}

/// Salda la decisión pendiente antes de mirar o tocar el registro de arranque.
///
/// Que este programa esté corriendo **es** la prueba de que el arranque llegó a
/// userland, que es lo único que init espera para acreditarlo. Hacerlo aquí
/// arregla dos cosas: `estado` deja de enseñar un «probando» eterno en un
/// sistema que evidentemente arrancó, y sobre todo se acaba la carrera por la
/// secuencia —init y este programa componiendo cada uno su registro a partir de
/// la misma lectura y escribiendo los dos en la misma ranura, con lo que la
/// vuelta atrás que acabábamos de prometer podía perderse—.
fn saldar_acreditacion() {
    let _ = sys::txn_confirm();
}

fn print_usage() {
    println!("uso:");
    println!("  soso-update estado");
    println!("  soso-update comprobar [--local DIR] [--channel dev|stable]");
    println!("  soso-update aplicar [--forzar] [--sin-kernel] [--local DIR] [--channel dev|stable]");
    println!("  soso-update revertir");
    println!("  soso-update transicion [--disco <id>]   (--disco, sólo desde el live)");
    println!("  soso-update recuperar --disco <id> [--pedir]   (desde el live)");
}

/// De qué disco arrancó el sistema — mismo `sys::disk_list` + `DISK_FLAG_BOOT`
/// que usa `soso-install` para clasificar discos, aquí sólo para informar.
fn medio_arranque() -> &'static str {
    let mut discos = [soso_abi::DiskInfo::default(); 8];
    let n = sys::disk_list(&mut discos);
    if n <= 0 {
        return "desconocido";
    }
    discos[..n as usize]
        .iter()
        .find(|d| d.flags & soso_abi::DISK_FLAG_BOOT != 0)
        .map(|d| match d.kind {
            soso_abi::DISK_KIND_USB => "USB live (pendrive)",
            soso_abi::DISK_KIND_NVME => "disco instalado",
            soso_abi::DISK_KIND_VIRTIO => "virtio (prueba QEMU)",
            _ => "desconocido",
        })
        .unwrap_or("desconocido")
}

fn cmd_estado() -> u8 {
    println!("arranque: {}", medio_arranque());
    let rel = read_release();
    if let Some(r) = &rel {
        println!("rootfs: {} ({})", r.version, r.build);
    } else {
        println!("rootfs: (sin /etc/soso-release)");
    }
    let mut kbuf = [0u8; 128];
    let n = sys::version(&mut kbuf);
    if n > 0 {
        println!(
            "kernel: {}",
            core::str::from_utf8(&kbuf[..n as usize]).unwrap_or("?")
        );
    }
    // Vuelta atrás disponible (U5a/U5b): a qué versión se puede volver y si esa
    // copia se ha comprobado releyéndola.
    let puntos = puntos_guardados();
    if puntos.is_empty() {
        println!("vuelta atrás: ninguna guardada");
    } else {
        for p in &puntos {
            let mut almacen = AlmacenSoso { libre: u64::MAX };
            let estado = match verificar_punto(p, &mut almacen) {
                Ok(()) => "verificada",
                Err(_) => "INCOMPLETA",
            };
            println!(
                "vuelta atrás: {} ({}) — {} ficheros, {}",
                p.version,
                if p.build.is_empty() { "?" } else { &p.build },
                p.entradas.len(),
                estado
            );
        }
        // La otra vía existe justamente para cuando esto no se puede escribir:
        // decirla aquí, mientras el sistema arranca, es lo único que sirve.
        println!("  «soso-update revertir», o la entrada «soso — recuperar versión");
        println!("  anterior» del menú de arranque si el sistema no llega a arrancar");
    }
    // U6: lo primero que quiere saber quien mira `estado` es si su máquina
    // puede volver atrás, no cuántos huecos tiene la ESP.
    let faltas = migracion::diagnosticar(&inventario());
    if faltas.is_empty() {
        println!("transición: al día");
    } else if migracion::admite_recuperable(&faltas) {
        println!("transición: falta algo menor ({} cosas) — «soso-update transicion»", faltas.len());
    } else {
        println!("transición: esta instalación NO admite vuelta atrás todavía");
        println!("  detalle y arreglo: «soso-update transicion»");
    }

    let mut mbuf = [0u8; 4096];
    let mn = sys::upd_read(UPD_WHICH_MAILBOX, 0, &mut mbuf);
    if mn > 0 {
        let mb = Mailbox::parse(core::str::from_utf8(&mbuf[..mn as usize]).unwrap_or(""));
        println!("buzón: {:?}", mb.cmd);
    } else if mn == -libsoso::abi::ENOTSUP {
        println!("buzón: no disponible (reflashea/reinstala para actualizar kernel)");
    }
    if file_exists("/etc/actualiza.estado") {
        if let Some(s) = read_file("/etc/actualiza.estado", 256) {
            println!("estado: {s}");
        }
    }
    0
}

fn cmd_comprobar(args: &[String]) -> u8 {
    let opts = parse_opts(args);
    // El origen **antes** de ir a por el manifiesto: DNS, TLS y descarga pueden
    // tardar, y si no se dice nada hasta el final no hay forma de distinguir
    // «trabajando» de «colgado» —ni de saber a qué servidor fue—.
    println!("origen: {} [{}]", describe_origen(&opts.origen), opts.motivo);
    let man = match load_manifest(&opts) {
        Ok(m) => m,
        Err(e) => {
            println!("soso-update: {e}");
            return 1;
        }
    };
    let actual = read_release()
        .map(|r| r.version_parsed())
        .unwrap_or(SemVer {
            major: 0,
            minor: 0,
            patch: 0,
        });
    if let Some(c) = &man.compat {
        println!("perfil: {} — abi {} fs {}", c.perfil, c.abi, c.fs);
    }
    println!("remoto: {} ({})", man.version_raw, man.build);
    println!(
        "local:  {}",
        read_release()
            .map(|r| r.version.clone())
            .unwrap_or_else(|| "?".into())
    );
    match semver::cmp(&man.version, &actual) {
        Ordering::Greater => println!("hay actualización disponible"),
        Ordering::Equal => println!("ya estás en la última versión"),
        Ordering::Less => println!("remoto es más antiguo que local"),
    }
    let pendientes: Vec<FileEntry> = man
        .files
        .iter()
        .filter(|f| file_needs_update(f))
        .cloned()
        .collect();
    for f in &pendientes {
        println!("  ~ {} ({})", f.path, humano(f.size));
    }
    let mut total = 0u64;
    if pendientes.is_empty() {
        println!("rootfs: sin cambios de fichero");
    } else {
        let spans = plan::plan_spans(&pendientes, plan::GAP_MAX, plan::SPAN_MAX);
        let bytes = plan::plan_bytes(&spans);
        total += bytes;
        println!(
            "rootfs: {} en {} {} (pack completo: {})",
            humano(bytes),
            spans.len(),
            if spans.len() == 1 { "petición" } else { "peticiones" },
            humano(man.pack_size)
        );
    }
    if man.kernel_size > 0 {
        if read_release().map(|r| r.kernel) == Some(man.kernel_hash.clone()) {
            println!("kernel: sin cambios");
        } else {
            total += man.kernel_size;
            println!("kernel: {} (hash {})", humano(man.kernel_size), &man.kernel_hash[..16]);
        }
    }
    println!("descarga total: {}", humano(total));
    0
}

fn cmd_aplicar(args: &[String]) -> u8 {
    let opts = parse_opts(args);
    let (man, man_bytes) = match load_manifest_con_bytes(&opts) {
        Ok(m) => m,
        Err(e) => {
            println!("soso-update: {e}");
            return 1;
        }
    };
    let actual = read_release()
        .map(|r| r.version_parsed())
        .unwrap_or(SemVer {
            major: 0,
            minor: 0,
            patch: 0,
        });
    // No reinstalar a ciegas lo que esta máquina ya rechazó: sin esto, repetir
    // `aplicar` vuelve a armar lo mismo y se entra en el bucle de aplicar,
    // fallar y deshacer.
    if !opts.forzar {
        if let Some(rec) = leer_bootrec_actual() {
            if soso_update_core::txn::punto::candidata_fallida(
                rec.decision,
                &rec.version_nueva,
                &man.version_raw,
            ) {
                println!(
                    "soso-update: la versión {} ya se instaló y hubo que deshacerla",
                    man.version_raw
                );
                println!("  si aun así quieres intentarlo otra vez: --forzar");
                return 0;
            }
        }
    }
    if !opts.forzar && semver::cmp(&man.version, &actual) != Ordering::Greater {
        println!("soso-update: ya estás en {} (usa --forzar)", man.version_raw);
        return 0;
    }
    // U6: si esta máquina no tiene dónde escribir la decisión, no se puede
    // prometer vuelta atrás — y hay que decirlo **antes** de descargar y
    // respaldar, no al final, cuando el sistema ya está tocado.
    let faltas = migracion::diagnosticar(&inventario());
    if !migracion::admite_recuperable(&faltas) {
        println!("soso-update: esta instalación todavía no admite actualización recuperable");
        for f in faltas.iter().filter(|f| f.bloquea()) {
            println!("  {}", f.descripcion());
        }
        println!("  no actualizo: prefiero no tocar nada a dejarte sin camino de regreso");
        println!("  arréglalo desde un live actualizado («soso-update transicion» lo detalla)");
        libsoso::logln!("actualiza: instalación sin huecos de transición; no aplico");
        return 1;
    }

    // La exclusión, lo primero: una segunda instancia tiene que plantarse
    // **antes** de bajarse doce megas para nada. Desde aquí y hasta el
    // reinicio, las rutas administradas son de esta operación.
    if let Err(e) = tomar_exclusion() {
        println!("soso-update: {e}");
        return 1;
    }
    write_file(
        "/etc/actualiza.estado",
        &format!("APLICANDO {}\n", man.version_raw),
    );
    // fd 3: el relato de la actualización queda en el log persistente, no sólo
    // en la consola de quien lanzó el comando.
    libsoso::logln!(
        "actualiza: aplicando {} sobre {}.{}.{} (build {})",
        man.version_raw,
        actual.major,
        actual.minor,
        actual.patch,
        man.build
    );

    // Sólo los ficheros que de verdad cambian. El resto del pack ni se pide.
    let cambian: Vec<FileEntry> = man
        .files
        .iter()
        .filter(|f| file_needs_update(f))
        .cloned()
        .collect();

    let etapa = match preparar_etapa(&man, &man_bytes) {
        Ok(d) => d,
        Err(e) => {
            println!("soso-update: {e}");
            soltar_exclusion();
            return 1;
        }
    };

    // Lo que ya está bajado **y verificado** no se vuelve a pedir: es lo que
    // hace reanudable un corte de red o un reinicio a mitad.
    let pendientes = descarga::pendientes_de(&cambian, |f| etapa_lista(&etapa, f));
    let ya = cambian.len() - pendientes.len();
    if ya > 0 {
        println!("etapa: {ya} de {} ya descargados, reanudando", cambian.len());
    }

    if let Err(e) = comprobar_espacio(&man, &cambian, &pendientes) {
        println!("soso-update: {e}");
        return 1;
    }

    if pendientes.is_empty() {
        if cambian.is_empty() {
            println!("rootfs: sin cambios");
        }
    } else {
        let spans = plan::plan_spans(&pendientes, plan::GAP_MAX, plan::SPAN_MAX);
        let bytes = plan::plan_bytes(&spans);
        println!(
            "rootfs: {} de {} ficheros, {} de {} ({} {})",
            pendientes.len(),
            man.files.len(),
            humano(bytes),
            humano(man.pack_size),
            spans.len(),
            if spans.len() == 1 { "petición" } else { "peticiones" }
        );
        // Bajar **antes** de tocar nada: mientras dura esto, `/bin`, `/lib` y la
        // versión instalada siguen siendo las de siempre. Un corte aquí no deja
        // el sistema a medias, sólo la etapa incompleta.
        for span in &spans {
            if let Err(e) = bajar_span(&opts, &etapa, &pendientes, span) {
                println!("soso-update: {e}");
                println!("  lo descargado se conserva: vuelve a lanzarlo para reanudar");
                libsoso::logln!("actualiza: descarga interrumpida ({e}); etapa conservada");
                soltar_exclusion();
                return 1;
            }
        }
    }

    // Antes de tocar el sistema: la vuelta atrás. Si no se puede crear y
    // verificar entera, no se actualiza (U5b).
    let punto = match preparar_punto(&man, &cambian) {
        Ok(p) => {
            println!(
                "punto: vuelta atrás a {} verificada ({} ficheros)",
                p.version,
                p.entradas.len()
            );
            reservar_para_restaurar(&p);
            p
        }
        Err(e) => {
            println!("soso-update: {e}");
            libsoso::logln!("actualiza: sin punto de recuperación ({e}); no aplico");
            soltar_exclusion();
            return 1;
        }
    };

    // Y ahora se **arma**: el cliente no escribe `/bin` ni `/lib`. Deja el
    // diario y el registro de arranque, y aplica el recuperador del kernel en
    // el siguiente arranque, antes de cargar firmware y antes de `/bin/init`.
    // Mientras tanto se puede seguir usando la versión de siempre.
    for f in &cambian {
        println!("  armado {} ({})", f.path, humano(f.size));
    }
    if let Err(e) = armar(&opts, &man, &etapa, &cambian, &punto) {
        println!("soso-update: {e}");
        libsoso::logln!("actualiza: no pude armar la operación ({e})");
        soltar_exclusion();
        return 1;
    }
    // Armada: la exclusión deja de ser de este proceso y dura hasta el
    // reinicio, que es cuando se aplica. La ventana que hay que proteger va
    // del respaldo al reinicio, no de este `main` a su `return`.
    let _ = sys::txn_lock(soso_abi::TXN_LOCK_ARMADO, 0);

    let _ = sys::unlink("/etc/actualiza.estado");
    libsoso::logln!("actualiza: {} armada; falta reiniciar", man.version_raw);
    println!(
        "soso-update: listo — reinicia para instalar soso {}",
        man.version_raw
    );
    println!("  si el arranque nuevo falla, se vuelve solo a {}", punto.version);
    0
}

fn humano(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{},{} MB", bytes / (1024 * 1024), (bytes % (1024 * 1024)) / 104858)
    } else if bytes >= 1024 {
        format!("{} kB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

/// Vuelta atrás manual (U5d).
///
/// Es la vía para cuando la versión nueva arranca pero algo no funciona —una
/// aplicación, el WiFi—, que es justo cuando la confirmación ya se dio por
/// buena. Muestra qué va a hacer, **verifica el punto antes de prometer nada** y
/// deja la petición durable; la restauración la ejecuta el arranque siguiente,
/// que es la única forma de no sustituir ejecutables por debajo de procesos
/// vivos.
fn cmd_revertir(args: &[String]) -> u8 {
    let si = args.iter().any(|a| a == "--yes" || a == "--si");
    let puntos = puntos_guardados();
    let Some(punto) = elegir_punto(puntos) else {
        println!("soso-update: no hay ninguna vuelta atrás guardada");
        println!("  sólo se puede volver a una versión que esta máquina haya instalado");
        diagnostico_puntos();
        return 1;
    };

    let actual = read_release()
        .map(|r| r.version)
        .unwrap_or_else(|| "desconocida".into());
    println!("vuelta atrás: {actual} → {}", punto.version);
    println!("  operación {}", punto.id.dir());
    println!("  {} ficheros a restaurar", punto.entradas.len());

    // Verificar **antes** de escribir la petición: prometer una vuelta atrás
    // que no se ha comprobado es lo que este contrato evita.
    let mut almacen = AlmacenSoso { libre: u64::MAX };
    if let Err(e) = verificar_punto(&punto, &mut almacen) {
        println!("soso-update: la copia guardada no está completa ({e:?})");
        println!("  no registro la petición: arrancaría a medias");
        return 1;
    }
    println!("  copia verificada");

    if !si {
        print!("¿Volver a {} en el próximo arranque? [s/N] ", punto.version);
        let resp: String = match libsoso::linea::Lector::new().siguiente() {
            Ok(Some(t)) => t.trim().into(),
            _ => String::new(),
        };
        let c = resp.chars().next().unwrap_or('\0');
        if c != 's' && c != 'S' && c != 'y' && c != 'Y' {
            println!("cancelado; no se ha escrito nada");
            return 0;
        }
    }

    let rec = BootRecord::nuevo(
        Decision::Rescatar,
        punto.id,
        &actual,
        &punto.version,
        siguiente_seq_bootrec(),
    )
    .con_punto(punto.id);
    if let Err(e) = publicar_bootrec(&rec) {
        println!("soso-update: {e}");
        return 1;
    }
    // El kernel sigue teniendo su propia recuperación por el buzón.
    let _ = sys::upd_write(UPD_WHICH_MAILBOX, 0, &Mailbox::format_revertir());

    libsoso::logln!("actualiza: vuelta atrás a {} registrada", punto.version);
    println!("soso-update: registrado — reinicia y volverás a {}", punto.version);
    0
}

/// Por qué no se encontró ningún punto. Sin esto, «no hay vuelta atrás» no
/// distingue entre no haber actualizado nunca y no poder leer el directorio.
fn diagnostico_puntos() {
    let fd = sys::open("/var/lib/soso-update", O_RDONLY);
    if fd < 0 {
        println!("  (no puedo abrir /var/lib/soso-update: errno {})", -fd);
        return;
    }
    let mut ents = [soso_abi::Dirent::default(); 32];
    let mut total = 0usize;
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n <= 0 {
            break;
        }
        for e in &ents[..(n as usize / soso_abi::DIRENT_SIZE).min(ents.len())] {
            let nombre = core::str::from_utf8(e.name_bytes()).unwrap_or("?");
            if nombre == "." || nombre == ".." {
                continue;
            }
            total += 1;
            let ruta = alloc::format!("/var/lib/soso-update/{nombre}/punto.rec");
            let mut st = soso_abi::Stat::default();
            if sys::stat(&ruta, &mut st) < 0 {
                println!("  {nombre}: sin punto.rec");
                continue;
            }
            match read_file_bytes(&ruta, 256 * 1024) {
                None => println!("  {nombre}: punto.rec de {} B ilegible", st.size),
                Some(d) => match Punto::parse(&d) {
                    Ok(p) => println!("  {nombre}: punto de {} (leído {} B)", p.version, d.len()),
                    Err(e) => println!(
                        "  {nombre}: punto.rec no se puede interpretar ({e:?}); {} B en disco, {} leídos",
                        st.size,
                        d.len()
                    ),
                },
            }
        }
    }
    let _ = sys::close(fd as u64);
    if total == 0 {
        println!("  (el directorio está vacío)");
    }
}

/// De los puntos guardados, el que toca: el que referencia el registro de
/// arranque si lo dice, y si no, el único que haya.
fn elegir_punto(mut puntos: Vec<Punto>) -> Option<Punto> {
    if puntos.is_empty() {
        return None;
    }
    if let Some(p) = leer_bootrec_actual().and_then(|r| r.punto) {
        if let Some(i) = puntos.iter().position(|x| x.id == p) {
            return Some(puntos.remove(i));
        }
    }
    if puntos.len() == 1 {
        return puntos.pop();
    }
    println!("soso-update: hay {} puntos guardados y el registro no dice cuál", puntos.len());
    None
}

struct Opts {
    local: Option<String>,
    forzar: bool,
    sin_kernel: bool,
    channel: Option<String>,
    /// Origen resuelto **una sola vez** por comando: con `latest`, volver a
    /// resolverlo en cada petición puede traer artefactos de dos releases
    /// distintas si alguien publica a mitad de la descarga.
    origen: Origen,
    motivo: &'static str,
}

fn parse_opts(args: &[String]) -> Opts {
    let mut opts = Opts {
        local: None,
        forzar: false,
        sin_kernel: false,
        channel: None,
        origen: Origen::Remoto(String::new()),
        motivo: "",
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--local" => {
                i += 1;
                opts.local = args.get(i).cloned();
            }
            "--forzar" => opts.forzar = true,
            "--sin-kernel" => opts.sin_kernel = true,
            "--channel" => {
                i += 1;
                opts.channel = args.get(i).cloned();
            }
            _ => {}
        }
        i += 1;
    }
    let conf = Conf::parse(&read_file("/etc/actualiza.conf", 512).unwrap_or_default());
    opts.origen = canal::resolver(opts.local.as_deref(), opts.channel.as_deref(), &conf);
    opts.motivo = canal::motivo(opts.local.as_deref(), opts.channel.as_deref(), &conf);
    opts
}

struct ReleaseInfo {
    kernel: String,
    version: String,
    build: String,
    _fecha: String,
}

impl ReleaseInfo {
    fn version_parsed(&self) -> SemVer {
        semver::parse(&self.version).unwrap_or(SemVer {
            major: 0,
            minor: 0,
            patch: 0,
        })
    }
}

fn read_release() -> Option<ReleaseInfo> {
    let text = read_file("/etc/soso-release", 512)?;
    let mut version = String::new();
    let mut build = String::new();
    let mut fecha = String::new();
    let mut kernel = String::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("version=") {
            version = v.into();
        } else if let Some(v) = line.strip_prefix("build=") {
            build = v.into();
        } else if let Some(v) = line.strip_prefix("fecha=") {
            fecha = v.into();
        } else if let Some(v) = line.strip_prefix("kernel=") {
            kernel = v.into();
        }
    }
    if version.is_empty() {
        return None;
    }
    Some(ReleaseInfo {
        version,
        build,
        _fecha: fecha,
        kernel,
    })
}

fn load_manifest(opts: &Opts) -> Result<Manifest, &'static str> {
    load_manifest_con_bytes(opts).map(|(m, _)| m)
}

fn load_manifest_con_bytes(opts: &Opts) -> Result<(Manifest, String), &'static str> {
    let destino = opts.origen.artefacto("manifest.txt");
    let data = if opts.origen.es_local() {
        read_file(&destino, 256 * 1024).ok_or("manifest local")?
    } else {
        let bytes = net::https_get_bytes(&destino, None)?;
        String::from_utf8(bytes).map_err(|_| "manifest UTF-8")?
    };
    let m = Manifest::parse(&data).map_err(|_| "manifest inválido")?;
    m.validate().map_err(|_| "manifest no válido")?;
    comprobar_compat(&m)?;
    Ok((m, data))
}

/// Rechaza un paquete que no sirve para esta máquina **antes** de bajarlo.
///
/// Sin esto, una release con otra ABI, otro formato de FS o sin el driver del
/// disco de arranque se instala igual y sólo se descubre al reiniciar, que es
/// el peor momento posible.
fn comprobar_compat(m: &Manifest) -> Result<(), &'static str> {
    let equipo = Equipo {
        arch: "x86_64".into(),
        abi: soso_abi::ABI_VERSION,
        fs: soso_update_core::compat::FS_FORMATO.into(),
        shim: soso_update_core::compat::SHIM_VERSION,
        recuperador: soso_update_core::compat::RECUPERADOR_VERSION,
        drivers: drivers_necesarios(),
    };
    match soso_update_core::compat::exigir(m.compat.as_ref(), &equipo) {
        Ok(()) => Ok(()),
        Err(CompatError::NoDeclarada) => Err(
            "el manifiesto no declara compatibilidad (release anterior a este cliente)",
        ),
        Err(CompatError::Abi { .. }) => Err("la release es para otra ABI de syscalls"),
        Err(CompatError::Fs { .. }) => Err("la release es para otro formato de sistema de ficheros"),
        Err(CompatError::Arch { .. }) => Err("la release es para otra arquitectura"),
        Err(CompatError::ShimAntiguo { .. }) | Err(CompatError::RecuperadorAntiguo { .. }) => Err(
            "la release exige un shim/recuperador más nuevo: hace falta una release puente",
        ),
        Err(CompatError::DriverAusente(_)) => Err(
            "la release no trae el driver del disco desde el que arrancas",
        ),
        Err(_) => Err("manifiesto de compatibilidad inválido"),
    }
}

/// Drivers que esta máquina **necesita** que la release traiga. Hoy se deduce
/// del medio de arranque, que es el mínimo imprescindible: sin él la máquina
/// no vuelve a arrancar.
fn describe_origen(o: &Origen) -> String {
    match o {
        Origen::Local(d) => alloc::format!("{d} (local)"),
        Origen::Remoto(u) => u.clone(),
    }
}

fn drivers_necesarios() -> Vec<String> {
    let mut v = Vec::new();
    let mut discos = [soso_abi::DiskInfo::default(); 8];
    let n = sys::disk_list(&mut discos);
    if n > 0 {
        if let Some(d) = discos[..n as usize]
            .iter()
            .find(|d| d.flags & soso_abi::DISK_FLAG_BOOT != 0)
        {
            match d.kind {
                soso_abi::DISK_KIND_NVME => v.push("nvme".into()),
                soso_abi::DISK_KIND_USB => v.push("usb".into()),
                soso_abi::DISK_KIND_VIRTIO => v.push("virtio-blk".into()),
                _ => {}
            }
        }
    }
    v
}

/// Trae `[start, start+len)` del pack, de la copia local o por HTTP `Range`.
fn fetch_pack_span(opts: &Opts, start: u64, len: u64) -> Result<Vec<u8>, &'static str> {
    if opts.origen.es_local() {
        return read_file_span(&opts.origen.artefacto("rootfs.pack"), start, len)
            .ok_or("rootfs.pack local");
    }
    net::https_download_span(&opts.origen.artefacto("rootfs.pack"), None, start, len)
}

/// Lee un tramo de un fichero local sin cargarlo entero.
fn read_file_span(path: &str, start: u64, len: u64) -> Option<Vec<u8>> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    if sys::seek(fd as u64, start as i64, soso_abi::SEEK_SET) < 0 {
        let _ = sys::close(fd as u64);
        return None;
    }
    let mut out = Vec::new();
    if out.try_reserve(len as usize).is_err() {
        let _ = sys::close(fd as u64);
        return None;
    }
    let mut buf = alloc::vec![0u8; HASH_BUF];
    while (out.len() as u64) < len {
        let falta = (len - out.len() as u64).min(HASH_BUF as u64) as usize;
        let n = sys::read(fd as u64, &mut buf[..falta]);
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    let _ = sys::close(fd as u64);
    if out.len() as u64 == len { Some(out) } else { None }
}

fn apply_kernel(opts: &Opts, man: &Manifest) -> Result<(), &'static str> {
    // El kernel es el otro bloque grande. Si el que ya está instalado tiene el
    // hash que pide el manifest, no hay nada que descargar ni que reflashear.
    if !man.kernel_hash.is_empty()
        && read_release().map(|r| r.kernel) == Some(man.kernel_hash.clone())
    {
        println!("kernel: sin cambios");
        return Ok(());
    }
    println!("kernel: {}", humano(man.kernel_size));
    let destino = opts.origen.artefacto("kernel-x86_64");
    let data = if opts.origen.es_local() {
        read_file_bytes(&destino, man.kernel_size as usize + 1).ok_or("kernel local")?
    } else {
        net::https_download_all(&destino, None, man.kernel_size)?
    };
    if data.len() as u64 != man.kernel_size || hex_sha256(&data) != man.kernel_hash {
        return Err("kernel corrupto");
    }
    let mut off = 0u64;
    while off < man.kernel_size {
        let end = (off + CHUNK as u64).min(man.kernel_size);
        let chunk = &data[off as usize..end as usize];
        let pad = pad_sector(chunk);
        let r = sys::upd_write(UPD_WHICH_KERNEL, off, &pad);
        if r < 0 {
            if r == -libsoso::abi::ENOTSUP {
                return Err(match medio_arranque() {
                    "USB live (pendrive)" => {
                        "sin hueco de kernel en la ESP: reflashea este pendrive (cargo xtask flash-usb-live) con una imagen ≥0.2.0"
                    }
                    "disco instalado" => {
                        "sin hueco de kernel en la ESP: arranca el live y ejecuta soso-install de nuevo para renovar la instalación"
                    }
                    _ => "reflashea/reinstala el live para habilitar actualización de kernel",
                });
            }
            return Err("upd_write falló");
        }
        off = end;
    }
    let meta = KernelMeta::staged(&man.version_raw, man.kernel_size, &man.kernel_hash);
    let r = sys::upd_write(UPD_WHICH_META, 0, &meta.format());
    if r < 0 && r != -libsoso::abi::ENOTSUP {
        return Err("meta kernel falló");
    }
    let hash = hex_sha256(&data);
    let payload = Mailbox::format_kernel(man.kernel_size, &hash, &man.version_raw);
    let r = sys::upd_write(UPD_WHICH_MAILBOX, 0, &payload);
    if r < 0 {
        return Err("no pude escribir buzón KERNEL");
    }
    Ok(())
}

fn pad_sector(chunk: &[u8]) -> Vec<u8> {
    let mut v = chunk.to_vec();
    let rem = v.len() % SECTOR;
    if rem != 0 {
        v.resize(v.len() + (SECTOR - rem), 0);
    }
    v
}

fn file_needs_update(entry: &manifest::FileEntry) -> bool {
    let path = format!("/{}", entry.path);
    // El tamaño descarta la mayoría de los casos sin tocar el contenido: leer
    // los 63 MB de un firmware GSP sólo para descubrir que ha cambiado sería
    // tirar el disco a la basura.
    match file_size(&path) {
        None => true,
        Some(size) if size != entry.size => true,
        Some(_) => hash_file(&path).map(|h| h != entry.hash_hex).unwrap_or(true),
    }
}

fn file_size(path: &str) -> Option<u64> {
    let mut st = soso_abi::Stat {
        ino: 0,
        size: 0,
        mtime: 0,
        file_type: 0,
        _pad: [0u8; 7],
    };
    if sys::stat(path, &mut st) < 0 {
        return None;
    }
    Some(st.size)
}

/// SHA-256 de un fichero leyéndolo por trozos: el pico de RAM es el buffer, no
/// el fichero.
fn hash_file(path: &str) -> Option<String> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut h = Hasher::new();
    let mut buf = alloc::vec![0u8; HASH_BUF];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n < 0 {
            let _ = sys::close(fd as u64);
            return None;
        }
        if n == 0 {
            break;
        }
        h.update(&buf[..n as usize]);
    }
    let _ = sys::close(fd as u64);
    Some(h.finish_hex())
}

fn file_exists(path: &str) -> bool {
    let mut st = libsoso::abi::Stat::default();
    sys::stat(path, &mut st) >= 0
}

fn read_file(path: &str, max: usize) -> Option<String> {
    read_file_bytes(path, max).map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn read_file_bytes(path: &str, max: usize) -> Option<Vec<u8>> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
        if out.len() >= max {
            break;
        }
    }
    let _ = sys::close(fd as u64);
    Some(out)
}

fn write_file(path: &str, content: &str) -> bool {
    write_path(path, content.as_bytes())
}

fn write_path(path: &str, data: &[u8]) -> bool {
    escribir(path, data).is_ok()
}

/// Escribe un fichero devolviendo el errno y la fase que falló: con
/// `Fd::WriteBuf` el contenido se materializa en el `close`, así que un fallo
/// de disco aparece ahí y no en el `write`.
fn escribir(path: &str, data: &[u8]) -> Result<(), (&'static str, i64)> {
    let fd = sys::open(path, O_WRONLY);
    if fd < 0 {
        return Err(("open", fd));
    }
    if let Err(e) = sys::write_all(fd as u64, data) {
        let _ = sys::close(fd as u64);
        return Err(("write", e));
    }
    let r = sys::close(fd as u64);
    if r < 0 { Err(("close", r)) } else { Ok(()) }
}

fn parent_dir(path: &str) -> Option<&str> {
    path.rfind('/').filter(|&i| i > 0).map(|i| &path[..i])
}

// ---------------------------------------------------- área de preparación

/// Directorio de la operación: `/var/lib/soso-update/<id>/etapa`.
///
/// El identificador es el hash del manifiesto (contrato U0), no la versión: dos
/// builds del mismo número comparten versión, y reutilizar lo bajado entre
/// ellos mezclaría los offsets de dos packs distintos.
fn preparar_etapa(man: &Manifest, man_bytes: &str) -> Result<String, &'static str> {
    let id = TxnId::from_manifest(man_bytes.as_bytes());
    let base = format!("/var/lib/soso-update/{}", id.dir());
    for d in ["/var", "/var/lib", "/var/lib/soso-update", &base] {
        let _ = sys::mkdir(d);
    }
    let etapa = format!("{base}/etapa");
    let _ = sys::mkdir(&etapa);

    // Si lo que hay guardado es de otra release, no sirve: se descarta antes de
    // reutilizar nada.
    let rec = format!("{base}/etapa.rec");
    let vale = read_file_bytes(&rec, 4096)
        .and_then(|b| Etapa::parse(&b).ok())
        .map(|e| e.sirve_para(man_bytes.as_bytes()))
        .unwrap_or(false);
    if !vale {
        let _ = sys::unlink(&rec);
        let nueva = Etapa::nueva(id, &man.version_raw, man.pack_size);
        if escribir(&rec, &nueva.format()).is_err() {
            return Err("no pude abrir el área de preparación");
        }
    }
    Ok(etapa)
}

fn ruta_etapa(etapa: &str, f: &FileEntry) -> String {
    format!("{etapa}/{}", f.path)
}

/// ¿Está este fichero ya bajado **y verificado**? Sólo eso cuenta como hecho:
/// un fichero truncado por un corte tiene que volver a bajarse entero.
fn etapa_lista(etapa: &str, f: &FileEntry) -> bool {
    let ruta = ruta_etapa(etapa, f);
    match read_file_bytes(&ruta, f.size as usize + 1) {
        Some(d) if d.len() as u64 == f.size && hex_sha256(&d) == f.hash_hex => true,
        _ => false,
    }
}

/// Baja un tramo del pack y reparte sus bytes entre los ficheros de la etapa.
///
/// Se piden tramos, no ficheros sueltos, para no hacer una petición HTTP por
/// binario; pero en RAM sólo se sostiene el fichero en curso, no el tramo.
fn bajar_span(
    opts: &Opts,
    etapa: &str,
    pendientes: &[FileEntry],
    span: &Span,
) -> Result<(), &'static str> {
    let mut idx = 0usize;
    let mut pos = span.start;
    let mut buf: Vec<u8> = Vec::new();

    let volcar = |f: &FileEntry, buf: &mut Vec<u8>| -> Result<(), &'static str> {
        if hex_sha256(buf) != f.hash_hex {
            return Err("hash corrupto en fichero descargado");
        }
        let ruta = ruta_etapa(etapa, f);
        if let Some(padre) = parent_dir(&ruta) {
            crear_arbol(padre);
        }
        escribir(&ruta, buf).map_err(|_| "no pude escribir en el área de preparación")?;
        buf.clear();
        Ok(())
    };

    let mut recibir = |trozo: &[u8]| -> Result<(), &'static str> {
        let mut resto = trozo;
        while !resto.is_empty() {
            let Some(&i) = span.files.get(idx) else { return Ok(()) };
            let f = &pendientes[i];
            let fin = f.offset + f.size;
            if pos < f.offset {
                // Hueco entre ficheros: el tramo los cubre a los dos y lo de en
                // medio no interesa.
                let salto = ((f.offset - pos) as usize).min(resto.len());
                resto = &resto[salto..];
                pos += salto as u64;
                continue;
            }
            let cabe = ((fin - pos) as usize).min(resto.len());
            buf.extend_from_slice(&resto[..cabe]);
            resto = &resto[cabe..];
            pos += cabe as u64;
            if pos == fin {
                volcar(f, &mut buf)?;
                idx += 1;
            }
        }
        Ok(())
    };

    if opts.origen.es_local() {
        let datos = fetch_pack_span(opts, span.start, span.len())?;
        recibir(&datos)?;
    } else {
        net::https_download_span_a(
            &opts.origen.artefacto("rootfs.pack"),
            None,
            span.start,
            span.len(),
            &mut recibir,
        )?;
    }
    Ok(())
}

fn crear_arbol(dir: &str) {
    let mut acc = String::new();
    for parte in dir.trim_start_matches('/').split('/') {
        if parte.is_empty() {
            continue;
        }
        acc.push('/');
        acc.push_str(parte);
        let _ = sys::mkdir(&acc);
    }
}

/// Comprobación previa: sin sitio para preparar **y** deshacer, no se empieza.
fn comprobar_espacio(
    man: &Manifest,
    cambian: &[FileEntry],
    pendientes: &[FileEntry],
) -> Result<(), &'static str> {
    let mut fs = soso_abi::FsInfo::default();
    if sys::fsinfo(&mut fs) < 0 || fs.block_size == 0 {
        // Sin el dato no se inventa una comprobación: se avisa y se sigue.
        println!("soso-update: aviso: no pude leer el espacio libre");
        return Ok(());
    }
    let necesidad = descarga::necesidad(man, cambian, pendientes, |ruta| {
        let mut st = soso_abi::Stat::default();
        (sys::stat(&format!("/{ruta}"), &mut st) >= 0).then_some(st.size)
    });
    let cap = soso_update_core::txn::Capacidad {
        sosofs_libre: fs.free_blocks.saturating_mul(fs.block_size),
        hueco_kernel: soso_update_core::UPD_KERNEL_SLOT_SIZE as u64,
        registro_arranque: soso_update_core::UPD_BOOTREC_SIZE,
        meta_kernel: soso_update_core::UPD_KERNEL_META_SIZE,
    };
    match soso_update_core::txn::preflight(&necesidad, &cap) {
        Ok(()) => Ok(()),
        Err(soso_update_core::txn::PreflightError::SinEspacio { necesita, libre }) => {
            println!(
                "espacio: hacen falta {} y hay {}",
                humano(necesita),
                humano(libre)
            );
            Err("sin espacio para preparar la actualización y poder deshacerla")
        }
        Err(soso_update_core::txn::PreflightError::KernelNoCabe { .. }) => {
            Err("el kernel de la release no cabe en el hueco de la ESP")
        }
        Err(_) => Err("los huecos de la ESP no tienen el tamaño esperado"),
    }
}

// ------------------------------------------------- punto de recuperación

/// Directorio del punto de una operación.
fn dir_punto(id: TxnId) -> String {
    format!("/var/lib/soso-update/{}", id.dir())
}

/// `Almacen` sobre el sistema real: lee del rootfs y guarda las copias dentro
/// del directorio de la operación, donde el recuperador del kernel las busca.
struct AlmacenSoso {
    libre: u64,
}

impl Almacen for AlmacenSoso {
    fn leer_sistema(&mut self, ruta: &str) -> Option<Vec<u8>> {
        let mut st = soso_abi::Stat::default();
        if sys::stat(&format!("/{ruta}"), &mut st) < 0 {
            return None;
        }
        read_file_bytes(&format!("/{ruta}"), st.size as usize + 1)
    }

    fn guardar_copia(&mut self, punto: TxnId, ruta: &str, datos: &[u8]) -> Result<(), ()> {
        let destino = format!("{}/respaldo/{ruta}", dir_punto(punto));
        if let Some(padre) = parent_dir(&destino) {
            crear_arbol(padre);
        }
        escribir(&destino, datos).map_err(|_| ())
    }

    fn releer_copia(&mut self, punto: TxnId, ruta: &str) -> Option<Vec<u8>> {
        let destino = format!("{}/respaldo/{ruta}", dir_punto(punto));
        let mut st = soso_abi::Stat::default();
        if sys::stat(&destino, &mut st) < 0 {
            return None;
        }
        read_file_bytes(&destino, st.size as usize + 1)
    }

    fn guardar_punto(&mut self, p: &Punto) -> Result<(), ()> {
        let destino = format!("{}/punto.rec", dir_punto(p.id));
        if let Some(padre) = parent_dir(&destino) {
            crear_arbol(padre);
        }
        // Se borra antes: sobrescribir uno más largo dejaría cola del anterior.
        let _ = sys::unlink(&destino);
        escribir(&destino, &p.format()).map_err(|_| ())
    }

    fn borrar_punto(&mut self, punto: TxnId) -> Result<(), ()> {
        let _ = sys::unlink(&format!("{}/punto.rec", dir_punto(punto)));
        Ok(())
    }

    fn espacio_libre(&mut self) -> u64 {
        self.libre
    }
}

/// Comprueba un punto releyendo sus copias, que es la única forma de saber si
/// sirve. `Almacen` guarda; `Sistema` es lo que lee el verificador.
fn verificar_punto(p: &Punto, a: &mut AlmacenSoso) -> Result<(), soso_update_core::txn::aplicador::Fallo> {
    struct Lector<'a> {
        a: &'a mut AlmacenSoso,
        id: TxnId,
    }
    impl soso_update_core::txn::aplicador::Sistema for Lector<'_> {
        fn leer(
            &mut self,
            de: soso_update_core::txn::aplicador::De,
            ruta: &str,
        ) -> Option<Vec<u8>> {
            match de {
                soso_update_core::txn::aplicador::De::Respaldo => {
                    self.a.releer_copia(self.id, ruta)
                }
                soso_update_core::txn::aplicador::De::Preparado => None,
            }
        }
        fn escribir(&mut self, _r: &str, _d: &[u8]) -> Result<(), ()> {
            Ok(())
        }
        fn borrar(&mut self, _r: &str) -> Result<(), ()> {
            Ok(())
        }
        fn guardar_diario(
            &mut self,
            _j: &soso_update_core::txn::journal::Journal,
        ) -> Result<(), ()> {
            Ok(())
        }
    }
    p.verificar(&mut Lector { a, id: p.id })
}

/// Puntos de recuperación guardados, del más reciente al más antiguo según el
/// orden del directorio. Es lo que permite a `estado` decir **a qué versión se
/// puede volver** y si esa vuelta está verificada.
fn puntos_guardados() -> Vec<Punto> {
    let mut out = Vec::new();
    let fd = sys::open("/var/lib/soso-update", O_RDONLY);
    if fd < 0 {
        return out;
    }
    let mut ents = [soso_abi::Dirent::default(); 32];
    // `getdents` devuelve **bytes**, no entradas, y **por lotes**: hay que
    // llamarlo hasta que dé 0, como hace `ls`.
    let mut nombres: Vec<String> = Vec::new();
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n <= 0 {
            break;
        }
        let cuantas = (n as usize / soso_abi::DIRENT_SIZE).min(ents.len());
        for e in &ents[..cuantas] {
            if let Ok(s) = core::str::from_utf8(e.name_bytes()) {
                nombres.push(s.into());
            }
        }
    }
    let _ = sys::close(fd as u64);
    for nombre in nombres {
        let e = nombre.as_str();
        let nombre = e;
        if nombre == "." || nombre == ".." || nombre.is_empty() {
            continue;
        }
        let ruta = alloc::format!("/var/lib/soso-update/{nombre}/punto.rec");
        if let Some(datos) = read_file_bytes(&ruta, 256 * 1024) {
            if let Ok(p) = Punto::parse(&datos) {
                out.push(p);
            }
        }
    }
    out
}

/// GUID de **esta** ESP, leído de `SOSOMODE.TXT`. Es lo que ata un punto a su
/// instalación: uno de otra máquina no se aplica aquí.
fn guid_esp() -> String {
    let mut buf = alloc::vec![0u8; soso_update_core::UPD_MODE_SIZE];
    let n = sys::upd_read(soso_abi::UPD_WHICH_MODE, 0, &mut buf);
    if n <= 0 {
        return String::new();
    }
    match soso_update_core::resolver_identidad(Some(&buf[..n as usize]), "") {
        soso_update_core::Identidad::Explicita(r) | soso_update_core::Identidad::Ajena(r) => {
            r.esp_guid
        }
        _ => String::new(),
    }
}

/// Crea y verifica el punto de recuperación de la versión **actual** antes de
/// tocar nada.
///
/// Es la regla de U5b: si el punto no se puede crear entero y releer, la
/// actualización **no sigue**. Prometer una vuelta atrás que nadie ha
/// comprobado es peor que no ofrecerla.
fn preparar_punto(man: &Manifest, cambian: &[FileEntry]) -> Result<Punto, &'static str> {
    let actual = read_release();
    let version = actual
        .as_ref()
        .map(|r| r.version.clone())
        .unwrap_or_else(|| "desconocida".into());
    let build = actual.as_ref().map(|r| r.build.clone()).unwrap_or_default();
    let kernel_hash = actual.map(|r| r.kernel).unwrap_or_default();

    let mut fs = soso_abi::FsInfo::default();
    let libre = if sys::fsinfo(&mut fs) >= 0 && fs.block_size > 0 {
        fs.free_blocks.saturating_mul(fs.block_size)
    } else {
        u64::MAX
    };

    let mut cambios: Vec<Cambio> = cambian.iter().map(|f| Cambio::Trae(f.path.clone())).collect();
    // `/etc/soso-release` se aplica y se deshace como el resto: si no, volver
    // atrás dejaría los binarios viejos anunciando la versión nueva.
    cambios.push(Cambio::Trae("etc/soso-release".into()));
    let id = TxnId::from_manifest(man.format().as_bytes());
    let mut almacen = AlmacenSoso { libre };
    match crear_punto(
        id,
        &version,
        &build,
        &guid_esp(),
        // El kernel activo: su tamaño no lo sabe el cliente —está en la ESP—,
        // así que se anota el hash que declara `/etc/soso-release`.
        Contenido { size: 0, hash: kernel_hash },
        &cambios,
        &mut almacen,
    ) {
        Ok(p) => Ok(p),
        Err(CrearError::SinEspacio { necesita, libre }) => {
            println!(
                "punto: hacen falta {} para poder deshacer y hay {}",
                humano(necesita),
                humano(libre)
            );
            Err("sin espacio para guardar la vuelta atrás")
        }
        Err(CrearError::Copia(r)) => {
            println!("punto: no pude copiar {r}");
            Err("no pude guardar la vuelta atrás")
        }
        Err(CrearError::Incompleto(r)) => {
            println!("punto: {r} no se relee igual que se escribió");
            Err("la vuelta atrás no se pudo verificar")
        }
        Err(CrearError::Registro) => Err("no pude registrar la vuelta atrás"),
    }
}

// ------------------------------------------------------ transición (U6)

/// Qué tiene esta máquina y qué le falta para poder recibir una actualización
/// **recuperable**. No toca nada: mirar antes de prometer es justamente el
/// punto —una instalación hecha con un live antiguo se actualiza, pero sin
/// vuelta atrás, y eso hay que decirlo antes de descargar, no después—.
fn cmd_transicion(args: &[String]) -> u8 {
    if let Some(i) = args.iter().position(|a| a == "--disco") {
        let Some(id) = args.get(i + 1).and_then(|s| s.parse::<u32>().ok()) else {
            println!("soso-update: --disco necesita el id del disco (mira «soso-install»)");
            return 2;
        };
        return pedir_provision(id);
    }
    let inv = inventario();
    let faltas = migracion::diagnosticar(&inv);
    if faltas.is_empty() {
        println!("transición: nada que hacer, esta instalación está al día");
        return 0;
    }
    println!("transición: falta algo en esta instalación");
    for f in &faltas {
        let marca = if f.bloquea() { "!!" } else { "  " };
        println!("  {marca} {}", f.descripcion());
    }
    if migracion::admite_recuperable(&faltas) {
        println!("puede actualizarse con vuelta atrás; lo marcado con «!!» sería lo que la impide");
    } else {
        println!("NO puede actualizarse con vuelta atrás todavía");
        println!("  arréglalo desde un live actualizado; reinstalar no hace falta");
    }
    0
}

/// Pide al shim que ponga al día la ESP de otro disco.
///
/// Lo hace el shim y no el kernel porque **crear** ficheros en FAT exige un
/// driver FAT completo: el kernel sólo sabe sobrescribir por LBA huecos que ya
/// existen. Bajo UEFI ese driver está, y puede abrir la ESP del disco instalado
/// por GUID. Por eso esto sólo tiene sentido desde el live: la petición la
/// atiende el shim **del live**, que es el que trae el código nuevo.
fn pedir_provision(disco: u32) -> u8 {
    if !arrancado_de_live() {
        println!("soso-update: «--disco» es para poner al día otra instalación desde el live");
        println!("  arrancado así, la petición la atendería el shim de esta misma máquina,");
        println!("  que es justo el que todavía no sabe hacerlo");
        return 1;
    }
    let Some(esp) = esp_guid_de(disco) else {
        println!("soso-update: no encuentro una ESP en el disco {disco}");
        println!("  «soso-install» lista los discos y sus particiones");
        return 1;
    };
    let payload = format!("SOSOBOOT v1\nPROVISION {esp}\ndisco id {disco}\n");
    if sys::bootreq_write(payload.as_bytes()) < 0 {
        println!("soso-update: no pude escribir la petición en SOSOBOOT.TXT de este USB");
        return 1;
    }
    println!("transición: pedida para la ESP {esp} del disco {disco}");
    println!("  reinicia **con el USB puesto**: el shim creará los huecos que falten,");
    println!("  actualizará su cargador y registrará la entrada de recuperación");
    libsoso::logln!("actualiza: transición pedida para la ESP {}", esp);
    0
}

/// GUID **único** de la primera partición de tipo ESP del disco.
fn esp_guid_de(disco: u32) -> Option<String> {
    const T_ESP: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
    let mut sec = [0u8; 512];
    if sys::disk_read(disco, 1, &mut sec) < 0 {
        return None;
    }
    let hdr = gptdisk::Header::parse(&sec).ok()?;
    let mut entradas = alloc::vec![0u8; hdr.entries_sectors() as usize * 512];
    if sys::disk_read(disco, hdr.entries_lba, &mut entradas) < 0 {
        return None;
    }
    let esperado = gptdisk::Guid::parse(T_ESP)?;
    for i in 0..hdr.num_entries as usize {
        let e = gptdisk::entry(&entradas, &hdr, i)?;
        if gptdisk::entry_used(e) && gptdisk::entry_type(e) == esperado {
            return Some(format!("{}", gptdisk::entry_unique(e)));
        }
    }
    None
}

/// ¿Estamos en el live? Se pregunta al registro de identidad de la ESP (U2).
fn arrancado_de_live() -> bool {
    let mut buf = alloc::vec![0u8; soso_update_core::UPD_MODE_SIZE];
    let n = sys::upd_read(soso_abi::UPD_WHICH_MODE, 0, &mut buf);
    let datos = (n > 0).then(|| &buf[..n as usize]);
    match soso_update_core::resolver_identidad(datos, "") {
        soso_update_core::Identidad::Explicita(r) => r.modo == soso_update_core::BootMode::Live,
        // Sin registro legible se trata como live, que es lo que hace el resto
        // del sistema con una ESP anterior a U2.
        _ => true,
    }
}

/// Mira los huecos de la ESP uno a uno. El kernel sólo los da por buenos si
/// están, miden lo que deben y sus clusters son consecutivos, así que un
/// `upd_read` que falle es exactamente «no se puede usar».
fn inventario() -> migracion::Inventario {
    let mut huecos = Vec::new();
    for h in migracion::HUECOS {
        let which = match h.nombre {
            "SOSOTXN.BIN" => soso_abi::UPD_WHICH_TXN,
            "SOSOKRN.BIN" => soso_abi::UPD_WHICH_KERNEL,
            "SOSOKRN.MET" => soso_abi::UPD_WHICH_META,
            "SOSOUPD.TXT" => UPD_WHICH_MAILBOX,
            _ => soso_abi::UPD_WHICH_MODE,
        };
        let mut buf = [0u8; 512];
        let r = sys::upd_read(which, 0, &mut buf);
        // «No está» y «está y no sirve» piden arreglos distintos, y decirlo mal
        // manda a buscar el problema donde no está: costó una tarde creer que
        // un hueco estaba fragmentado cuando nunca se había creado.
        huecos.push(if r > 0 {
            migracion::EstadoHueco::Listo
        } else if r == -libsoso::abi::ENOENT {
            migracion::EstadoHueco::Ausente
        } else {
            migracion::EstadoHueco::Inservible
        });
    }
    migracion::Inventario {
        huecos,
        formato_registro: leer_bootrec_actual().map(|r| r.formato),
        entrada_rescate: entrada_rescate_registrada(),
        // Desde dentro no se puede listar la raíz de la ESP: el kernel sólo
        // sabe localizar huecos por nombre y tamaño. Así que esto se deja en
        // «no consta» en vez de inventárselo; quien puede mirarlo de verdad es
        // el live, con el disco delante.
        log_fat: false,
        es_live: arrancado_de_live(),
    }
}

/// La NVRAM no la puede leer ni el kernel (no hay Runtime Services tras
/// `ExitBootServices`), así que se mira lo que el shim dejó escrito al
/// registrar las entradas.
fn entrada_rescate_registrada() -> bool {
    let mut buf = alloc::vec![0u8; soso_abi::BOOTREQ_SIZE];
    let n = sys::bootreq_read(&mut buf);
    if n <= 0 {
        return false;
    }
    let texto = String::from_utf8_lossy(&buf[..n as usize]);
    texto.lines().any(|l| l.trim_start().starts_with("rescate Boot"))
}

// ------------------------------------------------- exclusión de escritores

/// Toma la exclusión para esta operación. El mensaje de error dice **qué**
/// pasa, no un número: quien lo lee está intentando actualizar su ordenador.
///
/// Se toma **antes** de crear el punto, sin reserva todavía: lo primero es que
/// nadie más escriba las rutas que se van a copiar. La reserva se fija después,
/// cuando el punto ya dice cuánto ocupará restaurarlo.
fn tomar_exclusion() -> Result<(), String> {
    match sys::txn_lock(soso_abi::TXN_LOCK_TOMAR, 0) {
        r if r >= 0 => Ok(()),
        r if r == -libsoso::abi::EBUSY => Err(
            "ya hay una actualización en curso en esta máquina\n               espera a que termine, o reinicia si se quedó a medias"
                .into(),
        ),
        // Un kernel viejo no conoce la syscall. No es motivo para no
        // actualizar: es exactamente la máquina que más falta le hace.
        r if r == -libsoso::abi::ENOSYS => Ok(()),
        r => Err(format!("no pude tomar la exclusión de escritores (errno {})", -r)),
    }
}

fn soltar_exclusion() {
    let _ = sys::txn_lock(soso_abi::TXN_LOCK_SOLTAR, 0);
}

/// Con el punto ya creado se sabe el peor caso de la restauración: se reserva
/// para que los demás escritores no se lo coman antes de que haga falta.
fn reservar_para_restaurar(punto: &Punto) {
    let mut fs = soso_abi::FsInfo::default();
    let bloque = if sys::fsinfo(&mut fs) >= 0 && fs.block_size > 0 {
        fs.block_size
    } else {
        4096
    };
    let bloques = soso_update_core::txn::punto::reserva_efectiva(punto).div_ceil(bloque);
    let _ = sys::txn_lock(soso_abi::TXN_LOCK_TOMAR, bloques);
}

// --------------------------------------------------------------- armar

/// Contenido de `/etc/soso-release` para la versión nueva.
fn texto_release(man: &Manifest, sin_kernel: bool) -> String {
    let kernel_hash = if sin_kernel {
        read_release()
            .map(|r| r.kernel)
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| man.kernel_hash.clone())
    } else {
        man.kernel_hash.clone()
    };
    format!(
        "version={}\nbuild={}\nfecha={}\nkernel={}\n",
        man.version_raw, man.build, man.fecha, kernel_hash
    )
}

/// Deja la operación **armada**: diario durable, kernel preparado y registro de
/// arranque publicado. A partir de aquí no instala el cliente, sino el
/// recuperador del kernel en el siguiente arranque, antes de cargar firmware y
/// antes de `/bin/init`.
///
/// El orden es el del contrato U0: nada se publica en la ESP hasta que
/// completar o deshacer es posible **sólo** con lo que ya está en sosofs.
fn armar(
    opts: &Opts,
    man: &Manifest,
    etapa: &str,
    cambian: &[FileEntry],
    punto: &Punto,
) -> Result<(), &'static str> {
    let id = punto.id;
    let actual = read_release();
    let version_anterior = actual
        .as_ref()
        .map(|r| r.version.clone())
        .unwrap_or_else(|| "desconocida".into());

    // `/etc/soso-release` es un fichero administrado más: así la versión se
    // aplica y se deshace con el resto, en vez de escribirla alguien aparte.
    let release = texto_release(man, opts.sin_kernel);
    let ruta_release = format!("{etapa}/etc/soso-release");
    if let Some(p) = parent_dir(&ruta_release) {
        crear_arbol(p);
    }
    escribir(&ruta_release, release.as_bytes()).map_err(|_| "no pude preparar la versión")?;

    let mut j = Journal::nuevo(id, &man.version_raw, &version_anterior);
    j.kernel_nuevo = Some(Contenido {
        size: man.kernel_size,
        hash: man.kernel_hash.clone(),
    });
    // Sólo si de verdad consta: en una máquina recién instalada nadie ha
    // anotado el hash del kernel activo. Su vuelta atrás la gobierna
    // `SOSOKRN.MET`, que sí lo respalda al arrancar.
    j.kernel_anterior = (punto.kernel.hash.len() == 64).then(|| punto.kernel.clone());

    let respaldo_de = |ruta: &str| {
        punto
            .entradas
            .iter()
            .find(|e| e.path == ruta)
            .and_then(|e| e.respaldo.clone())
    };
    for f in cambian {
        let respaldo = respaldo_de(&f.path);
        j.entradas.push(Entrada {
            accion: if respaldo.is_some() { Accion::Reemplazar } else { Accion::Crear },
            progreso: Progreso::Respaldado,
            path: f.path.clone(),
            nuevo: Some(Contenido { size: f.size, hash: f.hash_hex.clone() }),
            respaldo,
        });
    }
    let resp_release = respaldo_de("etc/soso-release");
    j.entradas.push(Entrada {
        accion: if resp_release.is_some() { Accion::Reemplazar } else { Accion::Crear },
        progreso: Progreso::Respaldado,
        path: "etc/soso-release".into(),
        nuevo: Some(Contenido {
            size: release.len() as u64,
            hash: hex_sha256(release.as_bytes()),
        }),
        respaldo: resp_release,
    });

    // La secuencia tiene que seguir a la del diario que ya hubiera: se elige
    // siempre la copia de secuencia más alta, y empezar de cero dejaría ganar
    // al diario de la operación anterior.
    j.seq = seq_diario_existente(id) + 1;
    j.estado = TxnState::Preparado;
    j.validate().map_err(|_| "el diario de la operación no es coherente")?;
    guardar_diario(&j)?;

    // El kernel a su hueco de la ESP; su recuperación sigue siendo la del buzón.
    if !opts.sin_kernel {
        apply_kernel(opts, man).map_err(|_| "no pude preparar el kernel")?;
    }

    // Punto de compromiso: sólo ahora se publica el registro de arranque.
    let anterior = leer_bootrec_actual();
    let rec = BootRecord::nuevo(
        Decision::Armado,
        id,
        &man.version_raw,
        &version_anterior,
        anterior.as_ref().map(|r| r.seq + 1).unwrap_or(1),
    )
    .con_punto(id);
    publicar_bootrec(&rec)?;

    j.seq += 1;
    j.estado = TxnState::Armado;
    guardar_diario(&j)?;

    // Con la operación ya armada, sobra lo que no referencia nadie. El punto de
    // la versión activa y el que acabamos de crear se conservan: durante un
    // A→B→C conviven dos a propósito.
    let mut refs = alloc::vec![id];
    if let Some(p) = anterior.and_then(|r| r.punto) {
        if !refs.contains(&p) {
            refs.push(p);
        }
    }
    recoger_puntos(&refs);
    Ok(())
}

/// Secuencia más alta de los diarios que ya existan para esta operación.
fn seq_diario_existente(id: TxnId) -> u64 {
    let mut max = 0u64;
    for n in 0..2 {
        let ruta = format!("{}/diario.{n}", dir_punto(id));
        if let Some(d) = read_file_bytes(&ruta, 256 * 1024) {
            if let Ok(j) = Journal::parse(&d) {
                max = max.max(j.seq);
            }
        }
    }
    max
}

/// El diario se escribe por turnos entre dos copias: una escritura cortada no
/// puede llevarse por delante la anterior.
fn guardar_diario(j: &Journal) -> Result<(), &'static str> {
    let destino = format!("{}/diario.{}", dir_punto(j.id), j.seq % 2);
    if let Some(p) = parent_dir(&destino) {
        crear_arbol(p);
    }
    escribir(&destino, &j.format()).map_err(|_| "no pude escribir el diario de la operación")
}

fn leer_bootrec_actual() -> Option<BootRecord> {
    let mut buf = alloc::vec![0u8; soso_update_core::UPD_BOOTREC_SIZE];
    let n = sys::upd_read(soso_abi::UPD_WHICH_TXN, 0, &mut buf);
    (n > 0).then(|| BootRecord::pick(&buf[..n as usize]).ok()).flatten()
}

fn siguiente_seq_bootrec() -> u64 {
    leer_bootrec_actual().map(|r| r.seq + 1).unwrap_or(1)
}

/// Escribe el registro en la ranura que le toca por secuencia.
fn publicar_bootrec(rec: &BootRecord) -> Result<(), &'static str> {
    let bytes = rec.format().map_err(|_| "registro de arranque inválido")?;
    let off = (rec.ranura() * soso_update_core::SLOT_SIZE) as u64;
    if sys::upd_write(soso_abi::UPD_WHICH_TXN, off, &bytes) < 0 {
        return Err("no pude publicar el registro de arranque (¿falta SOSOTXN.BIN?)");
    }
    Ok(())
}

// ------------------------------------------------- limpieza de puntos (U5e)

/// Borra un árbol entero. `unlink` quita ficheros y directorios **vacíos**, así
/// que hay que vaciarlos de dentro afuera.
fn borrar_arbol(ruta: &str) {
    let fd = sys::open(ruta, O_RDONLY);
    if fd >= 0 {
        let mut ents = [soso_abi::Dirent::default(); 32];
        let mut hijos: Vec<String> = Vec::new();
        loop {
            let n = sys::getdents(fd as u64, &mut ents);
            if n <= 0 {
                break;
            }
            let cuantas = (n as usize / soso_abi::DIRENT_SIZE).min(ents.len());
            for e in &ents[..cuantas] {
                if let Ok(nombre) = core::str::from_utf8(e.name_bytes()) {
                    if nombre != "." && nombre != ".." && !nombre.is_empty() {
                        hijos.push(nombre.into());
                    }
                }
            }
        }
        let _ = sys::close(fd as u64);
        for h in hijos {
            borrar_arbol(&format!("{ruta}/{h}"));
        }
    }
    let _ = sys::unlink(ruta);
}

/// Recoge los puntos que ya no referencia nadie.
///
/// `referencias` son el punto con el que la versión activa puede volver atrás y
/// el que se está armando. Si no consta ninguna, **no se recoge nada**: sin
/// saber cuál es el bueno, borrar es peor que ocupar sitio. Y no se recoge por
/// tiempo ni por falta de espacio, nunca (sección 3.6 del plan).
fn recoger_puntos(referencias: &[TxnId]) {
    let todos: Vec<TxnId> = puntos_guardados().into_iter().map(|p| p.id).collect();
    for id in soso_update_core::txn::punto::a_recoger(&todos, referencias) {
        println!("  recogido el punto {} (ya no lo referencia nadie)", id.dir());
        borrar_arbol(&dir_punto(id));
    }
}

// ------------------------------------------ recuperación desde el live (U6)

/// Mira, desde el live, qué vuelta atrás tiene guardada **otra** instalación, y
/// —con `--pedir`— se la deja registrada para su próximo arranque.
///
/// Es la cuarta vía de recuperación de §3.6: la de cuando falla el shim, la ESP
/// o la recuperación interna. No llama al instalador: aquí no se formatea nada.
fn cmd_recuperar(args: &[String]) -> u8 {
    let pedir = args.iter().any(|a| a == "--pedir");
    let restaurar = args.iter().any(|a| a == "--restaurar");
    let disco = match args.iter().position(|a| a == "--disco") {
        Some(i) => match args.get(i + 1).and_then(|s| s.parse::<u32>().ok()) {
            Some(d) => d,
            None => {
                println!("soso-update: --disco necesita el id del disco (mira «soso-install»)");
                return 2;
            }
        },
        // Sin disco se enumera: quien llega aquí desde un live no tiene por qué
        // saberse los números de sus discos de memoria.
        None => return enumerar_recuperables(),
    };

    let (esp_lba, fs_lba, fs_bloques) = match particiones_de(disco) {
        Some(v) => v,
        None => {
            println!("soso-update: el disco {disco} no parece una instalación de soso");
            println!("  hace falta una ESP y una partición con sosofs");
            return 1;
        }
    };

    // 1) El registro de arranque de esa máquina: qué dice y a qué punto apunta.
    let mut sectores = SectoresDisco { disco, base: esp_lba };
    let Some(raw) = leer_hueco_esp(&mut sectores, b"SOSOTXN ", b"BIN", soso_update_core::UPD_BOOTREC_SIZE) else {
        println!("soso-update: esa ESP no tiene SOSOTXN.BIN utilizable");
        println!("  ponla al día primero: soso-update transicion --disco {disco}");
        return 1;
    };
    let rec = match BootRecord::pick(&raw) {
        Ok(r) => r,
        // Un hueco a ceros no es un registro roto: es una máquina que no ha
        // actualizado nunca, y decirlo como avería sería alarmar por nada.
        Err(soso_update_core::txn::bootrec::BootRecError::Registro(
            soso_update_core::RecordError::Vacia,
        )) => {
            println!("disco {disco}: nunca ha actualizado; no hay nada que deshacer");
            return 1;
        }
        Err(e) => {
            println!("soso-update: su registro de arranque no se puede leer ({e:?})");
            return 1;
        }
    };
    println!("disco {disco}: {} → decisión «{}»", rec.version_efectiva(), rec.decision.as_str());
    let Some(punto_id) = rec.punto else {
        println!("  no consta ninguna copia guardada a la que volver");
        return 1;
    };

    // 2) Su sosofs. Sólo lectura salvo que se pida restaurar aquí mismo: el
    //    disco de otra máquina no se escribe por si acaso.
    let dev = BloquesDisco {
        disco,
        base: fs_lba,
        bloques: fs_bloques,
        escritura: restaurar,
    };
    let mut fs = match sosofs::Sosofs::mount(dev) {
        Ok(fs) => fs,
        Err(e) => {
            println!("soso-update: no pude montar su sistema de ficheros ({e:?})");
            return 1;
        }
    };
    let dir = punto_id.dir();
    let ruta = format!("/var/lib/soso-update/{dir}/punto.rec");
    let datos = fs
        .resolve(&ruta)
        .ok()
        .and_then(|ino| fs.read_file(ino).ok());
    let Some(datos) = datos else {
        println!("  el registro apunta a un punto que no está en su disco ({ruta})");
        return 1;
    };
    let punto = match Punto::parse(&datos) {
        Ok(p) => p,
        Err(e) => {
            println!("  su punto guardado está ilegible ({e:?})");
            return 1;
        }
    };
    println!("  vuelta atrás guardada: {} ({} ficheros)", punto.version, punto.entradas.len());

    let mut sis = SistemaAjeno {
        fs: &mut fs,
        dir: dir.clone(),
        ahora: ahora_secs(),
    };
    if let Err(e) = punto.verificar(&mut sis) {
        println!("  pero la copia NO está completa ({e:?})");
        println!("  no registro nada: arrancaría a medias");
        return 1;
    }
    println!("  copia verificada");

    if restaurar {
        return restaurar_ahora(&mut sis, &punto, &rec, punto_id, &mut sectores, disco);
    }
    if !pedir {
        println!("para que la haga ese arranque:  soso-update recuperar --disco {disco} --pedir");
        println!("para hacerla desde aquí ahora:  soso-update recuperar --disco {disco} --restaurar");
        return 0;
    }

    // 3) Se **pide**, no se restaura desde aquí. Quien escribe en ese sosofs
    //    tiene que ser su propio kernel: es el único que puede excluir a otros
    //    escritores y llevar el diario. Aquí sólo se deja la decisión.
    let nuevo = BootRecord::nuevo(
        Decision::Rescatar,
        rec.id,
        rec.version_efectiva(),
        &rec.version_anterior,
        rec.seq + 1,
    )
    .con_punto(punto_id);
    let bytes = match nuevo.format() {
        Ok(b) => b,
        Err(e) => {
            println!("soso-update: no pude formar el registro ({e:?})");
            return 1;
        }
    };
    let off = nuevo.ranura() * soso_update_core::SLOT_SIZE;
    if !escribir_hueco_esp(&mut sectores, b"SOSOTXN ", b"BIN", soso_update_core::UPD_BOOTREC_SIZE, off, &bytes) {
        println!("soso-update: no pude escribir en su SOSOTXN.BIN");
        return 1;
    }
    println!("registrado: al arrancar ese disco volverá a {}", rec.version_anterior);
    libsoso::logln!("actualiza: rescate registrado para el disco {} → {}", disco, rec.version_anterior);
    0
}

/// Restaura el punto **desde aquí**, sobre el disco de la otra máquina.
///
/// Es la vía para cuando su kernel no arranca y por tanto no puede atender un
/// `rescatar`. El orden es el mismo que usa ese kernel cuando sí puede: el
/// punto ya está verificado entero, se restauran los ficheros, se cierra el
/// diario de la operación deshecha y sólo entonces se publica la decisión —si
/// se corta antes, el registro sigue pidiendo rescate y repetirlo es inofensivo,
/// porque cada paso es idempotente—.
///
/// Se publica `revertido`, no `restaurado-a-prueba`. «A prueba» significa que
/// **un arranque de esa máquina lo intentó y no sabemos cómo acabó**, y aquí no
/// ha arrancado nadie: dejarlo a prueba hace que su primer encendido diagnostique
/// un fallo que no ha ocurrido —que es exactamente lo que pasó la primera vez
/// que se probó esto—. Un estado que describe un arranque sólo lo puede escribir
/// ese arranque.
fn restaurar_ahora(
    sis: &mut SistemaAjeno<'_>,
    punto: &Punto,
    rec: &BootRecord,
    punto_id: TxnId,
    sectores: &mut SectoresDisco,
    disco: u32,
) -> u8 {
    use soso_update_core::txn::aplicador;

    if let Err(e) = aplicador::restaurar(&punto.entradas, sis) {
        println!("soso-update: fallo restaurando ({e:?})");
        println!("  su disco puede haber quedado a medias: repite esta misma orden");
        return 1;
    }
    println!("restaurados {} ficheros de {}", punto.entradas.len(), punto.version);

    if let Some(mut j) = leer_diario_ajeno(sis) {
        if soso_update_core::txn::rescate::cerrar_diario(&mut j) {
            use soso_update_core::txn::aplicador::Sistema;
            if sis.guardar_diario(&j).is_err() {
                println!("soso-update: no pude cerrar su diario; no publico la decisión");
                println!("  repite la orden: hasta que se publique, su registro sigue pidiendo rescate");
                return 1;
            }
        }
    }

    let nuevo = BootRecord::nuevo(
        Decision::Revertido,
        rec.id,
        rec.version_efectiva(),
        &rec.version_anterior,
        rec.seq + 1,
    )
    .con_punto(punto_id);
    let Ok(bytes) = nuevo.format() else {
        println!("soso-update: no pude formar su registro de arranque");
        return 1;
    };
    let off = nuevo.ranura() * soso_update_core::SLOT_SIZE;
    if !escribir_hueco_esp(
        sectores,
        b"SOSOTXN ",
        b"BIN",
        soso_update_core::UPD_BOOTREC_SIZE,
        off,
        &bytes,
    ) {
        println!("soso-update: restauré los ficheros pero no pude escribir su SOSOTXN.BIN");
        println!("  repite la orden con el disco conectado");
        return 1;
    }
    println!("disco {disco}: vuelto a {}; ya puede arrancar", rec.version_anterior);
    libsoso::logln!(
        "actualiza: restaurado el disco {} a {} desde el live",
        disco,
        rec.version_anterior
    );
    0
}

/// El diario de la operación que se acaba de deshacer, si está.
fn leer_diario_ajeno(sis: &mut SistemaAjeno<'_>) -> Option<Journal> {
    let mut mejor: Option<Journal> = None;
    for n in 0..2 {
        let ruta = format!("/var/lib/soso-update/{}/diario.{n}", sis.dir);
        let datos = sis.fs.resolve(&ruta).ok().and_then(|i| sis.fs.read_file(i).ok());
        if let Some(j) = datos.and_then(|d| Journal::parse(&d).ok()) {
            if mejor.as_ref().is_none_or(|m| j.seq > m.seq) {
                mejor = Some(j);
            }
        }
    }
    mejor
}

/// Recorre los discos que no son el de arranque y dice de cuáles se puede
/// recuperar algo. No escribe nada.
fn enumerar_recuperables() -> u8 {
    let mut discos = [soso_abi::DiskInfo::default(); 8];
    let n = sys::disk_list(&mut discos);
    if n <= 0 {
        println!("soso-update: no veo ningún disco");
        return 1;
    }
    let mut vistos = 0;
    for d in &discos[..n as usize] {
        if d.flags & soso_abi::DISK_FLAG_BOOT != 0 {
            continue;
        }
        if particiones_de(d.id).is_none() {
            continue;
        }
        vistos += 1;
        println!("--- disco {} ---", d.id);
        let _ = cmd_recuperar(&[String::from("--disco"), alloc::format!("{}", d.id)]);
    }
    if vistos == 0 {
        println!("soso-update: ningún otro disco tiene una instalación de soso");
        return 1;
    }
    0
}

/// `(primer LBA de la ESP, primer LBA del sosofs, bloques del sosofs)`.
fn particiones_de(disco: u32) -> Option<(u64, u64, u64)> {
    const T_ESP: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
    let mut sec = [0u8; 512];
    if sys::disk_read(disco, 1, &mut sec) < 0 {
        return None;
    }
    let hdr = gptdisk::Header::parse(&sec).ok()?;
    let mut entradas = alloc::vec![0u8; hdr.entries_sectors() as usize * 512];
    if sys::disk_read(disco, hdr.entries_lba, &mut entradas) < 0 {
        return None;
    }
    let tipo_esp = gptdisk::Guid::parse(T_ESP)?;
    let (mut esp, mut fs) = (None, None);
    for i in 0..hdr.num_entries as usize {
        let Some(e) = gptdisk::entry(&entradas, &hdr, i) else {
            continue;
        };
        if !gptdisk::entry_used(e) {
            continue;
        }
        let primero = gptdisk::entry_first_lba(e);
        if esp.is_none() && gptdisk::entry_type(e) == tipo_esp {
            esp = Some(primero);
            continue;
        }
        // El sosofs se reconoce por su superbloque, no por el tipo de
        // partición: el tipo lo elige quien particiona y puede ser cualquiera.
        if fs.is_none() {
            let mut cab = [0u8; 512];
            if sys::disk_read(disco, primero, &mut cab) >= 0 && sosofs::layout::looks_like_sosofs(&cab) {
                let bloques = (gptdisk::entry_last_lba(e) + 1 - primero) / 8;
                fs = Some((primero, bloques));
            }
        }
    }
    let (fs_lba, fs_bloques) = fs?;
    Some((esp?, fs_lba, fs_bloques))
}

fn leer_hueco_esp(
    s: &mut SectoresDisco,
    nombre: &[u8; 8],
    ext: &[u8; 3],
    tamano: usize,
) -> Option<Vec<u8>> {
    let slot = {
        let mut vol = espfat_core::Volumen::abrir(&mut *s).ok()?;
        vol.localizar(nombre, ext, tamano).ok()?
    };
    let mut datos = alloc::vec![0u8; tamano];
    for (i, trozo) in datos.chunks_mut(espfat_core::SECTOR).enumerate() {
        if sys::disk_read(s.disco, s.base + slot.data_lba + i as u64, trozo) < 0 {
            return None;
        }
    }
    Some(datos)
}

fn escribir_hueco_esp(
    s: &mut SectoresDisco,
    nombre: &[u8; 8],
    ext: &[u8; 3],
    tamano: usize,
    offset: usize,
    datos: &[u8],
) -> bool {
    let Some(slot) = espfat_core::Volumen::abrir(&mut *s)
        .ok()
        .and_then(|mut v| v.localizar(nombre, ext, tamano).ok())
    else {
        return false;
    };
    let base = slot.data_lba + (offset / espfat_core::SECTOR) as u64;
    for (i, trozo) in datos.chunks(espfat_core::SECTOR).enumerate() {
        if sys::disk_write(s.disco, s.base + base + i as u64, trozo) < 0 {
            return false;
        }
    }
    true
}

/// Acceso por sectores a una partición de **otro** disco, para `espfat-core`.
struct SectoresDisco {
    disco: u32,
    base: u64,
}

// Por referencia también: los dos ayudantes de abajo prestan el mismo acceso a
// sectores dos veces —una para localizar el hueco y otra para leerlo o
// escribirlo— y sin esto habría que duplicarlo.
impl espfat_core::Sectores for &mut SectoresDisco {
    fn leer(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), espfat_core::FatError> {
        (**self).leer(lba, buf)
    }
    fn escribir(&mut self, lba: u64, buf: &[u8]) -> Result<(), espfat_core::FatError> {
        (**self).escribir(lba, buf)
    }
}

impl espfat_core::Sectores for SectoresDisco {
    fn leer(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), espfat_core::FatError> {
        if sys::disk_read(self.disco, self.base + lba, buf) < 0 {
            return Err(espfat_core::FatError::Io);
        }
        Ok(())
    }
    fn escribir(&mut self, lba: u64, buf: &[u8]) -> Result<(), espfat_core::FatError> {
        if sys::disk_write(self.disco, self.base + lba, buf) < 0 {
            return Err(espfat_core::FatError::Io);
        }
        Ok(())
    }
}

/// Acceso por bloques de 4 KiB al sosofs de otro disco.
///
/// Nace **sólo lectura** a propósito: mirar el disco de otra máquina es lo
/// normal, y escribirlo la excepción que hay que pedir a mano. Con `escritura`
/// puesto, es la reparación offline: para cuando el kernel de esa máquina no
/// arranca y por tanto no puede atender una petición de rescate.
struct BloquesDisco {
    disco: u32,
    base: u64,
    bloques: u64,
    escritura: bool,
}

impl block_dev::BlockDevice for BloquesDisco {
    fn block_count(&self) -> u64 {
        self.bloques
    }
    fn read_block(&mut self, block: u64, buf: &mut block_dev::Block) -> Result<(), block_dev::BlockError> {
        if block >= self.bloques {
            return Err(block_dev::BlockError::OutOfRange);
        }
        if sys::disk_read(self.disco, self.base + block * 8, buf) < 0 {
            return Err(block_dev::BlockError::Io);
        }
        Ok(())
    }
    fn write_block(&mut self, block: u64, buf: &block_dev::Block) -> Result<(), block_dev::BlockError> {
        if !self.escritura {
            return Err(block_dev::BlockError::Io);
        }
        if block >= self.bloques {
            return Err(block_dev::BlockError::OutOfRange);
        }
        if sys::disk_write(self.disco, self.base + block * 8, buf) < 0 {
            return Err(block_dev::BlockError::Io);
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), block_dev::BlockError> {
        Ok(())
    }
}

/// El sistema de ficheros de la otra máquina, visto por el verificador y por el
/// restaurador. Lo que pueda hacer depende del dispositivo: si se montó sólo
/// lectura, las escrituras fallan abajo, no aquí.
struct SistemaAjeno<'a> {
    fs: &'a mut sosofs::Sosofs<BloquesDisco>,
    dir: String,
    ahora: u64,
}

impl soso_update_core::txn::aplicador::Sistema for SistemaAjeno<'_> {
    fn leer(&mut self, de: soso_update_core::txn::aplicador::De, ruta: &str) -> Option<Vec<u8>> {
        use soso_update_core::txn::aplicador::De;
        let completa = match de {
            De::Preparado => format!("/var/lib/soso-update/{}/etapa/{ruta}", self.dir),
            De::Respaldo => format!("/var/lib/soso-update/{}/respaldo/{ruta}", self.dir),
        };
        let ino = self.fs.resolve(&completa).ok()?;
        self.fs.read_file(ino).ok()
    }
    fn escribir(&mut self, rel: &str, datos: &[u8]) -> Result<(), ()> {
        let destino = format!("/{}", rel.trim_start_matches('/'));
        let (dir, nombre) = partir(&destino).ok_or(())?;
        let padre = self.crear_arbol(dir)?;
        // Igual que el aplicador del kernel: quitar y crear, no modificar en
        // sitio. La copia en escritura del sosofs deja la generación anterior
        // intacta hasta el commit, así que un corte aquí no mezcla nada.
        let _ = self.fs.unlink(padre, nombre);
        self.fs
            .create_file(padre, nombre, datos, self.ahora)
            .map(|_| ())
            .map_err(|_| ())
    }

    fn borrar(&mut self, rel: &str) -> Result<(), ()> {
        let destino = format!("/{}", rel.trim_start_matches('/'));
        let Some((dir, nombre)) = partir(&destino) else {
            return Ok(());
        };
        let Ok(padre) = self.fs.resolve(dir) else {
            return Ok(());
        };
        // Que ya no esté no es un error: restaurar es idempotente.
        let _ = self.fs.unlink(padre, nombre);
        Ok(())
    }

    fn guardar_diario(&mut self, j: &Journal) -> Result<(), ()> {
        let destino = format!(
            "/var/lib/soso-update/{}/diario.{}",
            self.dir,
            j.seq % 2
        );
        let (dir, nombre) = partir(&destino).ok_or(())?;
        let padre = self.crear_arbol(dir)?;
        let datos = j.format();
        let _ = self.fs.unlink(padre, nombre);
        self.fs
            .create_file(padre, nombre, &datos, self.ahora)
            .map(|_| ())
            .map_err(|_| ())
    }
}

impl SistemaAjeno<'_> {
    /// `mkdir -p` sobre el sistema de ficheros ajeno.
    fn crear_arbol(&mut self, ruta: &str) -> Result<u64, ()> {
        let mut ino = sosofs::layout::ROOT_INODE;
        for parte in ruta.split('/').filter(|p| !p.is_empty()) {
            ino = match self.fs.lookup(ino, parte) {
                Ok(hijo) => hijo,
                Err(_) => self.fs.mkdir(ino, parte, self.ahora).map_err(|_| ())?,
            };
        }
        Ok(ino)
    }
}

/// Hora de pared del **live**, para las fechas de lo que se escriba en el
/// disco ajeno. No es la de esa máquina, pero es la única que hay y anotar cero
/// sería peor.
fn ahora_secs() -> u64 {
    let mut t = soso_abi::Timespec::default();
    if sys::clock_gettime(soso_abi::CLOCK_REALTIME, &mut t) < 0 {
        return 0;
    }
    t.tv_sec as u64
}

/// `/a/b/c` → `("/a/b", "c")`.
fn partir(ruta: &str) -> Option<(&str, &str)> {
    let i = ruta.rfind('/')?;
    let (dir, nombre) = ruta.split_at(i);
    let nombre = &nombre[1..];
    if nombre.is_empty() {
        return None;
    }
    Some((if dir.is_empty() { "/" } else { dir }, nombre))
}
