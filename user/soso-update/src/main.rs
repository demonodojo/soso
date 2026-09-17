//! Descarga e instala actualizaciones de soso desde GitHub Releases.

#![no_std]
#![no_main]

extern crate alloc;

mod net;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use libsoso::{println, sys};
use soso_abi::{O_RDONLY, O_WRONLY, UPD_WHICH_KERNEL, UPD_WHICH_MAILBOX, UPD_WHICH_META};
use soso_update_core::canal::{self, Conf, Origen};
use soso_update_core::descarga::{self, Etapa};
use soso_update_core::txn::TxnId;
use soso_update_core::compat::{CompatError, Equipo};
use soso_update_core::hash::{hex_sha256, Hasher};
use soso_update_core::kernel_meta::KernelMeta;
use soso_update_core::manifest::{self, FileEntry, Manifest};
use soso_update_core::mailbox::Mailbox;
use soso_update_core::plan::{self, Span};
use soso_update_core::semver::{self, SemVer};

libsoso::entry!(main);

const SECTOR: usize = 512;
const CHUNK: usize = 64 * 1024;
/// Buffer de lectura para hashear ficheros ya instalados.
const HASH_BUF: usize = 256 * 1024;

fn main(args: &str) -> u8 {
    let parts: Vec<String> = args.split_whitespace().map(String::from).collect();
    let cmd = parts.first().map(|s| s.as_str()).unwrap_or("aplicar");
    let rest: &[String] = parts.get(1..).unwrap_or(&[]);
    match cmd {
        "estado" => cmd_estado(),
        "comprobar" => cmd_comprobar(rest),
        "aplicar" => cmd_aplicar(rest),
        "revertir" => cmd_revertir(),
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

fn print_usage() {
    println!("uso:");
    println!("  soso-update estado");
    println!("  soso-update comprobar [--local DIR] [--channel dev|stable]");
    println!("  soso-update aplicar [--forzar] [--sin-kernel] [--local DIR] [--channel dev|stable]");
    println!("  soso-update revertir");
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
    println!("origen: {} [{}]", describe_origen(&opts.origen), opts.motivo);
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
    if !opts.forzar && semver::cmp(&man.version, &actual) != Ordering::Greater {
        println!("soso-update: ya estás en {} (usa --forzar)", man.version_raw);
        return 0;
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

    if let Err(e) = comprobar_espacio(&man, &pendientes) {
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
                return 1;
            }
        }
    }

    // Y ahora, con todo verificado en la etapa, se escribe el sistema.
    for f in &cambian {
        if let Err(e) = instalar_desde_etapa(&etapa, f) {
            println!("soso-update: {}: {e}", f.path);
            libsoso::logln!("actualiza: {}: {e}", f.path);
            return 1;
        }
        println!("  ok {} ({})", f.path, humano(f.size));
    }

    if !opts.sin_kernel {
        if let Err(e) = apply_kernel(&opts, &man) {
            println!("soso-update: kernel: {e}");
            let _ = write_file(
                "/etc/actualiza.estado",
                &format!("KERNEL_FALLO {}\n", man.version_raw),
            );
            return 1;
        }
    }

    let kernel_hash = if opts.sin_kernel {
        read_release()
            .map(|r| r.kernel)
            .filter(|h| !h.is_empty())
            .unwrap_or(man.kernel_hash.clone())
    } else {
        man.kernel_hash.clone()
    };
    let release = format!(
        "version={}\nbuild={}\nfecha={}\nkernel={}\n",
        man.version_raw, man.build, man.fecha, kernel_hash
    );
    if !write_file("/etc/soso-release", &release) {
        println!("soso-update: no pude escribir /etc/soso-release");
        return 1;
    }
    let _ = sys::unlink("/etc/actualiza.estado");
    libsoso::logln!("actualiza: {} preparada; falta reiniciar", man.version_raw);
    println!("soso-update: listo — reinicia para arrancar soso {}", man.version_raw);
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

fn cmd_revertir() -> u8 {
    let payload = Mailbox::format_revertir();
    let r = sys::upd_write(UPD_WHICH_MAILBOX, 0, &payload);
    if r < 0 {
        libsoso::logln!("actualiza: REVERTIR no se pudo registrar ({r})");
        println!("soso-update: no pude escribir buzón ({r})");
        return 1;
    }
    libsoso::logln!("actualiza: REVERTIR registrado; falta reiniciar");
    println!("soso-update: REVERTIR registrado — reinicia para restaurar el kernel");
    0
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

/// Copia un fichero ya verificado de la etapa al sistema.
fn instalar_desde_etapa(etapa: &str, f: &FileEntry) -> Result<(), &'static str> {
    let datos = read_file_bytes(&ruta_etapa(etapa, f), f.size as usize + 1)
        .ok_or("falta en el área de preparación")?;
    if datos.len() as u64 != f.size || hex_sha256(&datos) != f.hash_hex {
        return Err("el fichero preparado no cuadra con el manifiesto");
    }
    let destino = format!("/{}", f.path);
    if let Some(padre) = parent_dir(&destino) {
        crear_arbol(padre);
    }
    escribir(&destino, &datos).map_err(|(fase, _)| match fase {
        "open" => "no pude abrirlo para escribir",
        "write" => "falló la escritura",
        _ => "falló al cerrarlo",
    })
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
fn comprobar_espacio(man: &Manifest, pendientes: &[FileEntry]) -> Result<(), &'static str> {
    let mut fs = soso_abi::FsInfo::default();
    if sys::fsinfo(&mut fs) < 0 || fs.block_size == 0 {
        // Sin el dato no se inventa una comprobación: se avisa y se sigue.
        println!("soso-update: aviso: no pude leer el espacio libre");
        return Ok(());
    }
    let necesidad = descarga::necesidad(man, pendientes, |ruta| {
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
