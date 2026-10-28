//! Gestor de paquetes de aplicación.

#![no_std]
#![no_main]

extern crate alloc;

mod fs;
mod net;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use libsoso::{abi, println, sys};
use soso_pkg_core::{
    artefacto_remoto, comandos::reconstruir, decidir, indice::Indice, manifest::Manifest,
    indice_url, parse_version, AccionInstalacion, Conf, MAX_PACK_SIZE,
};
use soso_abi::ABI_VERSION;

libsoso::entry!(main);

const MAX_TXT: usize = 256 * 1024;

fn main(args: &[String]) -> u8 {
    if args.is_empty() {
        uso();
        return 2;
    }
    let mut it = args.iter().map(|s| s.as_str());
    let cmd = it.next().unwrap();
    let mut forzar = false;
    let mut origen: Option<String> = None;
    let mut resto: Vec<&str> = Vec::new();
    while let Some(a) = it.next() {
        if a == "--forzar" {
            forzar = true;
        } else if a == "--origen" {
            origen = it.next().map(|s| s.to_string());
            if origen.is_none() {
                println!("soso-pkg: --origen requiere un directorio");
                return 2;
            }
        } else {
            resto.push(a);
        }
    }
    match cmd {
        "listar" => cmd_listar(),
        "buscar" => {
            let filtro = resto.first().copied();
            cmd_buscar(filtro, origen.as_deref())
        }
        "instalar" => {
            let Some(arg) = resto.first() else {
                println!("uso: soso-pkg instalar <nombre|DIR> [--forzar] [--origen DIR]");
                return 2;
            };
            cmd_instalar(arg, forzar, origen.as_deref())
        }
        "actualizar" => cmd_actualizar(resto.first().copied(), forzar, origen.as_deref()),
        "borrar" => {
            let Some(n) = resto.first() else {
                println!("uso: soso-pkg borrar <nombre>");
                return 2;
            };
            cmd_borrar(n)
        }
        _ => {
            uso();
            2
        }
    }
}

fn uso() {
    println!("soso-pkg listar");
    println!("soso-pkg buscar [texto] [--origen DIR]");
    println!("soso-pkg instalar <nombre|DIR> [--forzar] [--origen DIR]");
    println!("soso-pkg actualizar [nombre] [--forzar] [--origen DIR]");
    println!("soso-pkg borrar <nombre>");
}

fn cmd_listar() -> u8 {
    fs::mkdir_all("/var/paquetes");
    for n in fs::list_dir("/var/paquetes") {
        if n.starts_with('.') {
            if n == ".staging" || n == ".viejo" {
                println!("  (incompleto: {n})");
            }
            continue;
        }
        let man_path = format!("/var/paquetes/{n}/paquete.txt");
        if let Some(bytes) = fs::read_bytes(&man_path, MAX_TXT) {
            if let Ok(text) = core::str::from_utf8(&bytes) {
                if let Ok(m) = Manifest::parse(text) {
                    println!("{n}  {}  {}", m.version_raw, m.nombre);
                    continue;
                }
            }
        }
        println!("{n}  (sin manifiesto)");
    }
    0
}

fn resolver_origen(origen_cli: Option<&str>) -> Result<Origen, u8> {
    if let Some(dir) = origen_cli {
        return Ok(Origen::Local(normalizar_dir(dir)));
    }
    let conf = Conf::parse(
        &fs::read_bytes("/etc/paquetes.conf", 4096)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default(),
    );
    if let Some(url) = conf.url {
        Ok(Origen::Remoto(url))
    } else {
        println!("soso-pkg: falta url= en /etc/paquetes.conf (o use --origen DIR)");
        Err(1)
    }
}

enum Origen {
    Local(String),
    Remoto(String),
}

fn cmd_buscar(filtro: Option<&str>, origen_cli: Option<&str>) -> u8 {
    let origen = match resolver_origen(origen_cli) {
        Ok(o) => o,
        Err(c) => return c,
    };
    let idx_bytes = match cargar_indice(&origen) {
        Ok(b) => b,
        Err(c) => return c,
    };
    let text = match core::str::from_utf8(&idx_bytes) {
        Ok(t) => t,
        Err(_) => {
            println!("soso-pkg: índice no UTF-8");
            return 1;
        }
    };
    let idx = match Indice::parse(text) {
        Ok(i) => i,
        Err(e) => {
            println!("soso-pkg: {e}");
            return 1;
        }
    };
    for e in idx.buscar(filtro) {
        println!("{}  {}", e.nombre, e.version_raw);
    }
    0
}

fn cargar_indice(origen: &Origen) -> Result<Vec<u8>, u8> {
    match origen {
        Origen::Local(dir) => {
            let path = format!("{}/indice.txt", dir.trim_end_matches('/'));
            fs::read_bytes(&path, MAX_TXT).ok_or_else(|| {
                println!("soso-pkg: no leo {path}");
                1
            })
        }
        Origen::Remoto(base) => net::https_get(&indice_url(base)).map_err(|e| {
            println!("soso-pkg: índice ({e})");
            1
        }),
    }
}

fn cmd_instalar(arg: &str, forzar: bool, origen_cli: Option<&str>) -> u8 {
    if arg.starts_with('/') || arg.starts_with("./") || arg.contains('/') {
        return instalar_local(normalizar_dir(arg), forzar);
    }
    let origen = match resolver_origen(origen_cli) {
        Ok(o) => o,
        Err(c) => return c,
    };
    instalar_remoto(arg, forzar, &origen)
}

fn normalizar_dir(p: &str) -> String {
    if p.starts_with("./") {
        p[2..].to_string()
    } else {
        p.to_string()
    }
}

fn instalar_remoto(nombre: &str, forzar: bool, origen: &Origen) -> u8 {
    let idx_bytes = match cargar_indice(origen) {
        Ok(b) => b,
        Err(c) => return c,
    };
    let text = core::str::from_utf8(&idx_bytes).unwrap_or("");
    let idx = match Indice::parse(text) {
        Ok(i) => i,
        Err(e) => {
            println!("soso-pkg: {e}");
            return 1;
        }
    };
    let Some(ent) = idx.mejor(nombre) else {
        println!("soso-pkg: «{nombre}» no está en el índice");
        return 1;
    };
    let base = match origen {
        Origen::Local(dir) => dir.clone(),
        Origen::Remoto(url) => url.clone(),
    };
    let url_txt = match origen {
        Origen::Local(_) => format!(
            "{}/{}/{}/paquete.txt",
            base.trim_end_matches('/'),
            ent.nombre,
            ent.version_raw
        ),
        Origen::Remoto(_) => artefacto_remoto(&base, &ent.nombre, &ent.version_raw, "paquete.txt"),
    };
    let url_pack = match origen {
        Origen::Local(_) => format!(
            "{}/{}/{}/paquete.pack",
            base.trim_end_matches('/'),
            ent.nombre,
            ent.version_raw
        ),
        Origen::Remoto(_) => artefacto_remoto(&base, &ent.nombre, &ent.version_raw, "paquete.pack"),
    };
    let man_bytes = match cargar_artefacto(&url_txt, origen) {
        Ok(b) => b,
        Err(c) => return c,
    };
    let pack = match cargar_artefacto_pack(&url_pack, origen, ent.pack_size) {
        Ok(b) => b,
        Err(c) => return c,
    };
    let man_text = core::str::from_utf8(&man_bytes).unwrap_or("");
    let man = match Manifest::parse(man_text) {
        Ok(m) => m,
        Err(e) => {
            println!("soso-pkg: {e}");
            return 1;
        }
    };
    aplicar_manifiesto(man, man_text, &pack, forzar)
}

fn cargar_artefacto(url: &str, origen: &Origen) -> Result<Vec<u8>, u8> {
    match origen {
        Origen::Local(_) => fs::read_bytes(url, MAX_TXT).ok_or(1),
        Origen::Remoto(_) => net::https_get(url).map_err(|_| 1),
    }
}

fn cargar_artefacto_pack(url: &str, origen: &Origen, esperado: u64) -> Result<Vec<u8>, u8> {
    if esperado > MAX_PACK_SIZE {
        println!("soso-pkg: pack demasiado grande");
        return Err(1);
    }
    let max = esperado as usize;
    match origen {
        Origen::Local(_) => fs::read_bytes(url, max).ok_or(1),
        Origen::Remoto(_) => {
            let b = net::https_get(url).map_err(|_| 1)?;
            if b.len() as u64 != esperado && esperado > 0 {
                // Si el índice no coincide, aceptar tamaño real verificado luego.
            }
            Ok(b)
        }
    }
}

fn instalar_local(dir: String, forzar: bool) -> u8 {
    let dir = dir.trim_end_matches('/').to_string();
    let man_path = format!("{dir}/paquete.txt");
    let Some(man_bytes) = fs::read_bytes(&man_path, MAX_TXT) else {
        println!("soso-pkg: no leo {man_path}");
        return 1;
    };
    let man_text = core::str::from_utf8(&man_bytes).unwrap_or("");
    let man = match Manifest::parse(man_text) {
        Ok(m) => m,
        Err(e) => {
            println!("soso-pkg: {e}");
            return 1;
        }
    };
    let pack_path = format!("{dir}/paquete.pack");
    let pack = if fs::exists(&pack_path) {
        let sz = man.pack_size() as usize;
        fs::read_bytes(&pack_path, sz).unwrap_or_default()
    } else {
        let mut blob = Vec::new();
        for f in &man.files {
            let p = format!("{dir}/{}", f.path);
            let Some(part) = fs::read_bytes(&p, f.size as usize) else {
                println!("soso-pkg: falta {p}");
                return 1;
            };
            blob.extend_from_slice(&part);
        }
        blob
    };
    aplicar_manifiesto(man, man_text, &pack, forzar)
}

fn version_instalada(nombre: &str) -> Option<soso_update_core::SemVer> {
    let p = format!("/var/paquetes/{nombre}/paquete.txt");
    let bytes = fs::read_bytes(&p, MAX_TXT)?;
    let text = core::str::from_utf8(&bytes).ok()?;
    let m = Manifest::parse(text).ok()?;
    Some(m.version)
}

fn aplicar_manifiesto(man: Manifest, man_text: &str, pack: &[u8], forzar: bool) -> u8 {
    if let Err(e) = man.verify_pack(pack, ABI_VERSION) {
        println!("soso-pkg: {e}");
        return 1;
    }
    let local = version_instalada(&man.nombre);
    match decidir(&man.version, local.as_ref(), forzar) {
        AccionInstalacion::YaInstalado => {
            println!("soso-pkg: {} {} ya instalado", man.nombre, man.version_raw);
            return 0;
        }
        AccionInstalacion::RequiereForzar => {
            println!("soso-pkg: use --forzar para instalar esta versión");
            return 1;
        }
        AccionInstalacion::Instalar | AccionInstalacion::Actualizar => {}
    }
    let necesita = man.pack_size().saturating_add(man_text.len() as u64 + 4096);
    if fs::espacio_libre() < necesita {
        println!("soso-pkg: espacio insuficiente en disco");
        return 1;
    }
    let staging = format!("/var/paquetes/.staging/{}", man.nombre);
    fs::borrar_arbol(&staging);
    fs::mkdir_all(&staging);
    if fs::write_bytes(&format!("{staging}/paquete.txt"), man_text.as_bytes()).is_err() {
        println!("soso-pkg: no pude escribir manifiesto");
        return 1;
    }
    for f in &man.files {
        let start = f.offset as usize;
        let end = start + f.size as usize;
        let slice = &pack[start..end];
        let dest = format!("{staging}/{}", f.path);
        if let Some(p) = dest.rfind('/') {
            fs::mkdir_all(&dest[..p]);
        }
        if fs::write_bytes(&dest, slice).is_err() {
            println!("soso-pkg: no pude escribir {}", f.path);
            return 1;
        }
    }
    if let Err(msg) = promover(&man.nombre) {
        println!("soso-pkg: {msg}");
        return 1;
    }
    if let Err(msg) = reconstruir_comandos() {
        println!("soso-pkg: {msg}");
        return 1;
    }
    println!(
        "soso-pkg: {} {} instalado",
        man.nombre, man.version_raw
    );
    0
}

fn promover(nombre: &str) -> Result<(), &'static str> {
    let staging = format!("/var/paquetes/.staging/{nombre}");
    let dest = format!("/var/paquetes/{nombre}");
    fs::mkdir_all("/var/paquetes");
    if !fs::exists(&staging) {
        return Err("preparación incompleta");
    }
    if fs::exists(&dest) {
        let viejo = format!("/var/paquetes/.viejo/{nombre}");
        fs::borrar_arbol(&viejo);
        fs::mkdir_all("/var/paquetes/.viejo");
        if sys::rename(&dest, &viejo) < 0 {
            fs::borrar_arbol(&dest);
        }
        if sys::rename(&staging, &dest) < 0 {
            return Err("no pude activar el paquete");
        }
        fs::borrar_arbol(&viejo);
    } else if sys::rename(&staging, &dest) < 0 {
        return Err("no pude activar el paquete");
    }
    Ok(())
}

fn paquetes_instalados() -> Vec<(String, Manifest)> {
    let mut out = Vec::new();
    for n in fs::list_dir("/var/paquetes") {
        if n.starts_with('.') {
            continue;
        }
        let man_path = format!("/var/paquetes/{n}/paquete.txt");
        let Some(bytes) = fs::read_bytes(&man_path, MAX_TXT) else {
            continue;
        };
        let Ok(text) = core::str::from_utf8(&bytes) else {
            continue;
        };
        if let Ok(m) = Manifest::parse(text) {
            out.push((n, m));
        }
    }
    out
}

fn reconstruir_comandos() -> Result<(), String> {
    let pkgs = paquetes_instalados();
    let bins: Vec<String> = fs::nombres_bin_sistema();
    let refs: Vec<&str> = bins.iter().map(|s| s.as_str()).collect();
    let idx = reconstruir(
        &pkgs
            .iter()
            .map(|(n, m)| (n.clone(), m.clone()))
            .collect::<Vec<_>>(),
        &refs,
    )?;
    fs::mkdir_all("/var/paquetes");
    fs::write_bytes("/var/paquetes/comandos", idx.format().as_bytes())
        .map_err(|_| "no pude escribir comandos".into())
}

fn cmd_actualizar(nombre: Option<&str>, forzar: bool, origen_cli: Option<&str>) -> u8 {
    let origen = match resolver_origen(origen_cli) {
        Ok(o) => o,
        Err(c) => return c,
    };
    let idx_bytes = match cargar_indice(&origen) {
        Ok(b) => b,
        Err(c) => return c,
    };
    let text = core::str::from_utf8(&idx_bytes).unwrap_or("");
    let idx = match Indice::parse(text) {
        Ok(i) => i,
        Err(e) => {
            println!("soso-pkg: {e}");
            return 1;
        }
    };
    let instalados = paquetes_instalados();
    let mut rc = 0u8;
    for (n, man) in instalados {
        if nombre.is_some_and(|filtro| filtro != n) {
            continue;
        }
        let Some(ent) = idx.mejor(&n) else {
            continue;
        };
        let rem = match parse_version(&ent.version_raw) {
            Some(v) => v,
            None => continue,
        };
        if decidir(&rem, Some(&man.version), forzar) != AccionInstalacion::Actualizar {
            continue;
        }
        if instalar_remoto(&n, forzar, &origen) != 0 {
            rc = 1;
        }
    }
    rc
}

fn cmd_borrar(nombre: &str) -> u8 {
    let dest = format!("/var/paquetes/{nombre}");
    if !fs::exists(&dest) {
        println!("soso-pkg: «{nombre}» no está instalado");
        return 1;
    }
    if proceso_usa_paquete(nombre) {
        println!("soso-pkg: hay procesos usando «{nombre}»");
        return 1;
    }
    fs::borrar_arbol(&dest);
    if let Err(msg) = reconstruir_comandos() {
        println!("soso-pkg: {msg}");
        return 1;
    }
    println!("soso-pkg: «{nombre}» borrado");
    0
}

fn proceso_usa_paquete(nombre: &str) -> bool {
    let man_path = format!("/var/paquetes/{nombre}/paquete.txt");
    let Some(bytes) = fs::read_bytes(&man_path, MAX_TXT) else {
        return false;
    };
    let Ok(text) = core::str::from_utf8(&bytes) else {
        return false;
    };
    let Ok(man) = Manifest::parse(text) else {
        return false;
    };
    let mut cmds = Vec::new();
    for f in &man.files {
        if let Some(c) = soso_pkg_core::rutas::nombre_bin(&f.path) {
            cmds.push(c);
        }
    }
    let mut buf = [abi::ProcInfo::default(); 32];
    let n = sys::pslist(&mut buf);
    if n <= 0 {
        return false;
    }
    let cuantas = (n as usize).min(buf.len());
    for p in &buf[..cuantas] {
        let name = core::str::from_utf8(&p.name)
            .unwrap_or("")
            .trim_end_matches('\0');
        if cmds.iter().any(|c| c == name) {
            return true;
        }
    }
    false
}
