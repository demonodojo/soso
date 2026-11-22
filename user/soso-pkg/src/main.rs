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
use soso_abi::ABI_VERSION;
use soso_pkg_core::{
    AccionInstalacion, Conf, MAX_PACK_SIZE, artefacto_remoto,
    comandos::reconstruir,
    decidir,
    indice::{Catalogo, EntradaIndice, Indice},
    indice_url,
    manifest::Manifest,
    parse_version,
};

libsoso::entry!(main);

const MAX_TXT: usize = 256 * 1024;
const MAX_DEP_DEPTH: usize = 32;

struct PlanPaquete {
    entrada: EntradaIndice,
    /// Posición del origen en la lista con la que se cargó el catálogo.
    origen: usize,
    manifiesto: Manifest,
    texto: String,
}

fn main(args: &[String]) -> u8 {
    activar_paquetes_sistema_instalados();
    if args.is_empty() {
        uso();
        return 2;
    }
    let mut it = args.iter().map(|s| s.as_str());
    let cmd = it.next().unwrap();
    let mut forzar = false;
    // `--origen DIR` se puede repetir: varios repositorios locales, en orden.
    let mut origen: Vec<String> = Vec::new();
    let mut resto: Vec<&str> = Vec::new();
    while let Some(a) = it.next() {
        if a == "--forzar" {
            forzar = true;
        } else if a == "--origen" {
            let Some(dir) = it.next() else {
                println!("soso-pkg: --origen requiere un directorio");
                return 2;
            };
            origen.push(dir.to_string());
        } else {
            resto.push(a);
        }
    }
    match cmd {
        "listar" => cmd_listar(),
        "buscar" => {
            let filtro = resto.first().copied();
            cmd_buscar(filtro, &origen)
        }
        "instalar" => {
            let Some(arg) = resto.first() else {
                println!("uso: soso-pkg instalar <nombre|DIR> [--forzar] [--origen DIR]");
                return 2;
            };
            cmd_instalar(arg, forzar, &origen)
        }
        "actualizar" => cmd_actualizar(resto.first().copied(), forzar, &origen),
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
    println!("soso-pkg buscar [texto] [--origen DIR]…");
    println!("soso-pkg instalar <nombre|DIR> [--forzar] [--origen DIR]…");
    println!("soso-pkg actualizar [nombre] [--forzar] [--origen DIR]…");
    println!("orígenes: url= y origen= en /etc/paquetes.conf (por defecto, soso y Sal)");
    println!("soso-pkg borrar <nombre>");
}

fn cmd_listar() -> u8 {
    fs::mkdir_all("/var/paquetes");
    for n in fs::list_dir("/var/paquetes") {
        if n.starts_with('.') {
            // Tras una instalación buena el directorio queda vacío: sólo es
            // una instalación a medias si dentro queda algún paquete.
            if (n == ".staging" || n == ".viejo")
                && !fs::list_dir(&format!("/var/paquetes/{n}")).is_empty()
            {
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

/// Orígenes en orden de prioridad: los `--origen` de la línea de órdenes o, si
/// no hay, los de `/etc/paquetes.conf`.
fn resolver_origenes(cli: &[String]) -> Vec<Origen> {
    if !cli.is_empty() {
        return cli.iter().map(|d| Origen::Local(normalizar_dir(d))).collect();
    }
    Conf::parse(
        &fs::read_bytes("/etc/paquetes.conf", 4096)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default(),
    )
    .origenes_efectivos()
    .into_iter()
    .map(Origen::Remoto)
    .collect()
}

enum Origen {
    Local(String),
    Remoto(String),
}

impl Origen {
    /// Para los mensajes: sin `https://` ni el prefijo de raw.githubusercontent.
    fn etiqueta(&self) -> String {
        match self {
            Origen::Local(d) => d.clone(),
            Origen::Remoto(u) => {
                let u = u.trim_start_matches("https://").trim_end_matches('/');
                u.strip_prefix("raw.githubusercontent.com/")
                    .map(|r| format!("github.com/{r}"))
                    .unwrap_or_else(|| u.to_string())
            }
        }
    }
}

/// Índice de cada origen. Un origen que no responde no tumba a los demás: se
/// avisa y se sigue; sólo falla si no queda ninguno. Devuelve los orígenes que
/// sí cargaron, en el mismo orden que los índices del catálogo.
fn cargar_catalogo(origenes: Vec<Origen>) -> Result<(Catalogo, Vec<Origen>), u8> {
    let mut cat = Catalogo::default();
    let mut vivos = Vec::new();
    for origen in origenes {
        let Ok(bytes) = cargar_indice(&origen) else {
            println!("soso-pkg: sigo sin el origen {}", origen.etiqueta());
            continue;
        };
        let Ok(text) = core::str::from_utf8(&bytes) else {
            println!("soso-pkg: índice no UTF-8 en {}", origen.etiqueta());
            continue;
        };
        match Indice::parse(text) {
            Ok(i) => {
                cat.indices.push(i);
                vivos.push(origen);
            }
            Err(e) => println!("soso-pkg: {} ({e})", origen.etiqueta()),
        }
    }
    if vivos.is_empty() {
        println!("soso-pkg: ningún origen de paquetes disponible");
        return Err(1);
    }
    Ok((cat, vivos))
}

fn cmd_buscar(filtro: Option<&str>, origen_cli: &[String]) -> u8 {
    let (cat, origenes) = match cargar_catalogo(resolver_origenes(origen_cli)) {
        Ok(x) => x,
        Err(c) => return c,
    };
    for (o, e) in cat.buscar(filtro) {
        if origenes.len() > 1 {
            println!("{}  {}  ({})", e.nombre, e.version_raw, origenes[o].etiqueta());
        } else {
            println!("{}  {}", e.nombre, e.version_raw);
        }
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
            println!("soso-pkg: índice de {} ({e})", origen.etiqueta());
            1
        }),
    }
}

fn cmd_instalar(arg: &str, forzar: bool, origen_cli: &[String]) -> u8 {
    if arg.starts_with('/') || arg.starts_with("./") || arg.contains('/') {
        return instalar_local(normalizar_dir(arg), forzar);
    }
    let (cat, origenes) = match cargar_catalogo(resolver_origenes(origen_cli)) {
        Ok(x) => x,
        Err(c) => return c,
    };
    instalar_remoto(arg, forzar, &cat, &origenes)
}

fn normalizar_dir(p: &str) -> String {
    if p.starts_with("./") {
        p[2..].to_string()
    } else {
        p.to_string()
    }
}

fn instalar_remoto(nombre: &str, forzar: bool, cat: &Catalogo, origenes: &[Origen]) -> u8 {
    let mut plan = Vec::new();
    let mut pila = Vec::new();
    if !planificar_paquete(nombre, None, cat, origenes, &mut pila, &mut plan) {
        return 1;
    }
    for paquete in plan {
        let origen = &origenes[paquete.origen];
        let url_pack = url_paquete(&paquete.entrada, origen, "paquete.pack");
        let pack = match cargar_artefacto_pack(&url_pack, origen, paquete.entrada.pack_size) {
            Ok(b) => b,
            Err(c) => return c,
        };
        if aplicar_manifiesto(paquete.manifiesto, &paquete.texto, &pack, forzar) != 0 {
            return 1;
        }
    }
    0
}

fn planificar_paquete(
    nombre: &str,
    requisito: Option<&str>,
    cat: &Catalogo,
    origenes: &[Origen],
    pila: &mut Vec<String>,
    plan: &mut Vec<PlanPaquete>,
) -> bool {
    if let Some(p) = plan.iter().find(|p| p.entrada.nombre == nombre) {
        let ok = requisito
            .map(|r| soso_pkg_core::version_req::satisface(&p.manifiesto.version, r))
            .unwrap_or(true);
        if !ok {
            println!("soso-pkg: requisitos incompatibles para {nombre}");
        }
        return ok;
    }
    if pila.iter().any(|n| n == nombre) {
        println!(
            "soso-pkg: ciclo de dependencias: {} -> {nombre}",
            pila.join(" -> ")
        );
        return false;
    }
    if pila.len() >= MAX_DEP_DEPTH {
        println!("soso-pkg: dependencias anidadas por encima del límite ({MAX_DEP_DEPTH})");
        return false;
    }

    let instalado = requisito.and_then(|r| {
        manifest_instalada(nombre).filter(|m| soso_pkg_core::version_req::satisface(&m.version, r))
    });
    let (entrada, manifiesto, texto) = if let Some(m) = instalado {
        (None, m, String::new())
    } else {
        let entrada = match requisito {
            Some(r) => cat.mejor_compatible(nombre, r),
            None => cat.mejor(nombre),
        };
        let Some((origen, entrada)) = entrada else {
            if let Some(r) = requisito {
                println!("soso-pkg: no hay una versión de {nombre} que cumpla {r}");
            } else {
                println!("soso-pkg: «{nombre}» no está en el índice");
            }
            return false;
        };
        let entrada = entrada.clone();
        let (texto, manifiesto) = match cargar_manifiesto_indice(&entrada, &origenes[origen]) {
            Ok(x) => x,
            Err(_) => return false,
        };
        if requisito.is_some_and(|r| !soso_pkg_core::version_req::satisface(&manifiesto.version, r))
        {
            println!(
                "soso-pkg: {} no cumple la versión requerida",
                manifiesto.nombre
            );
            return false;
        }
        (Some((origen, entrada)), manifiesto, texto)
    };

    pila.push(nombre.into());
    for dep in &manifiesto.dependencias {
        if !planificar_paquete(&dep.nombre, Some(&dep.version), cat, origenes, pila, plan) {
            pila.pop();
            return false;
        }
    }
    pila.pop();
    if let Some((origen, entrada)) = entrada {
        plan.push(PlanPaquete {
            entrada,
            origen,
            manifiesto,
            texto,
        });
    }
    true
}

fn cargar_manifiesto_indice(
    entrada: &EntradaIndice,
    origen: &Origen,
) -> Result<(String, Manifest), u8> {
    let url = url_paquete(entrada, origen, "paquete.txt");
    let bytes = cargar_artefacto(&url, origen)?;
    if bytes.len() as u64 != entrada.txt_size
        || soso_update_core::hex_sha256(&bytes) != entrada.txt_hash
    {
        println!(
            "soso-pkg: manifiesto de {} no coincide con el índice",
            entrada.nombre
        );
        return Err(1);
    }
    let text = core::str::from_utf8(&bytes)
        .map_err(|_| {
            println!("soso-pkg: manifiesto de {} no es UTF-8", entrada.nombre);
            1
        })?
        .to_string();
    let man = Manifest::parse(&text).map_err(|e| {
        println!("soso-pkg: {e}");
        1
    })?;
    if man.nombre != entrada.nombre || man.version_raw != entrada.version_raw {
        println!("soso-pkg: nombre o versión del manifiesto no coincide con el índice");
        return Err(1);
    }
    if man.pack_size() != entrada.pack_size {
        println!("soso-pkg: tamaño declarado del pack no coincide con el índice");
        return Err(1);
    }
    Ok((text, man))
}

fn url_paquete(entrada: &EntradaIndice, origen: &Origen, fichero: &str) -> String {
    match origen {
        Origen::Local(dir) => format!(
            "{}/{}/{}/{}",
            dir.trim_end_matches('/'),
            entrada.nombre,
            entrada.version_raw,
            fichero
        ),
        Origen::Remoto(base) => {
            artefacto_remoto(base, &entrada.nombre, &entrada.version_raw, fichero)
        }
    }
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
    manifest_instalada(nombre).map(|m| m.version)
}

fn manifest_instalada(nombre: &str) -> Option<Manifest> {
    let p = format!("/var/paquetes/{nombre}/paquete.txt");
    let bytes = fs::read_bytes(&p, MAX_TXT)?;
    let text = core::str::from_utf8(&bytes).ok()?;
    Manifest::parse(text).ok()
}

fn aplicar_manifiesto(man: Manifest, man_text: &str, pack: &[u8], forzar: bool) -> u8 {
    if let Err(e) = man.verify_pack(pack, ABI_VERSION) {
        println!("soso-pkg: {e}");
        return 1;
    }
    if !dependencias_cumplidas(&man) || !dependientes_compatibles(&man.nombre, Some(&man.version)) {
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
    let respaldo = if man.destino_sistema {
        espacio_respaldo_sistema(&man)
    } else {
        0
    };
    let necesita = man
        .pack_size()
        .saturating_add(man_text.len() as u64 + 4096)
        .saturating_add(respaldo);
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
    if man.destino_sistema {
        if preparar_respaldo_sistema(&man, &staging).is_err() {
            fs::borrar_arbol(&staging);
            println!("soso-pkg: no pude preparar los respaldos del sistema");
            return 1;
        }
    }
    if let Err(msg) = promover(&man.nombre) {
        println!("soso-pkg: {msg}");
        return 1;
    }
    if man.destino_sistema {
        if let Some(v) = sistema_mas_nuevo(indice_del_sistema().as_ref(), &man) {
            ceder_al_sistema(&man);
            println!(
                "soso-pkg: el sistema ya trae {} {v}, más nueva; se queda la del sistema",
                man.nombre
            );
        } else if !activar_destinos_sistema(&man).is_empty() {
            println!("soso-pkg: no pude activar todos los ficheros del sistema");
            return 1;
        }
    }
    if let Err(msg) = reconstruir_comandos() {
        println!("soso-pkg: {msg}");
        return 1;
    }
    println!("soso-pkg: {} {} instalado", man.nombre, man.version_raw);
    0
}

fn dependencias_cumplidas(man: &Manifest) -> bool {
    for dep in &man.dependencias {
        let Some(instalado) = manifest_instalada(&dep.nombre) else {
            println!(
                "soso-pkg: falta la dependencia {} {}",
                dep.nombre, dep.version
            );
            return false;
        };
        if !soso_pkg_core::version_req::satisface(&instalado.version, &dep.version) {
            println!(
                "soso-pkg: {} {} no cumple {} {}",
                dep.nombre, instalado.version_raw, dep.nombre, dep.version
            );
            return false;
        }
    }
    true
}

fn dependientes_compatibles(nombre: &str, version: Option<&soso_update_core::SemVer>) -> bool {
    for (instalado, man) in paquetes_instalados() {
        if instalado == nombre {
            continue;
        }
        for dep in &man.dependencias {
            if dep.nombre != nombre {
                continue;
            }
            if !version.is_some_and(|v| soso_pkg_core::version_req::satisface(v, &dep.version)) {
                let objetivo = version
                    .map(|v| format!("{}.{}.{}", v.major, v.minor, v.patch))
                    .unwrap_or_else(|| "ausente".into());
                println!(
                    "soso-pkg: {instalado} requiere {nombre} {}, incompatible con {objetivo}",
                    dep.version
                );
                return false;
            }
        }
    }
    true
}

fn espacio_respaldo_sistema(man: &Manifest) -> u64 {
    let raiz = format!("/var/paquetes/{}", man.nombre);
    let anterior = manifest_instalada(&man.nombre);
    let mut total = 0u64;
    for f in &man.files {
        let Some(destino) = soso_pkg_core::rutas::ruta_destino_sistema(&f.path) else {
            continue;
        };
        let copia_previa = format!("{raiz}/.respaldo/{}", f.path);
        let ausente = format!("{copia_previa}.ausente");
        let mismo_archivo = anterior
            .as_ref()
            .is_some_and(|m| m.destino_sistema && m.files.iter().any(|old| old.path == f.path));
        let tam = if mismo_archivo && fs::exists(&copia_previa) {
            fs::read_bytes(&copia_previa, MAX_PACK_SIZE as usize)
                .map_or(MAX_PACK_SIZE as u64, |data| data.len() as u64)
        } else if mismo_archivo && fs::exists(&ausente) {
            1
        } else {
            fs::read_bytes(&destino, MAX_PACK_SIZE as usize)
                .map_or(1, |data| data.len().max(1) as u64)
        };
        total = total.saturating_add(tam);
    }
    total
}

fn preparar_respaldo_sistema(man: &Manifest, staging: &str) -> Result<(), ()> {
    let raiz = format!("/var/paquetes/{}", man.nombre);
    let anterior = manifest_instalada(&man.nombre);
    for f in &man.files {
        let Some(destino) = soso_pkg_core::rutas::ruta_destino_sistema(&f.path) else {
            continue;
        };
        let backup = format!("{staging}/.respaldo/{}", f.path);
        let ausente = format!("{backup}.ausente");
        if anterior
            .as_ref()
            .is_some_and(|m| m.destino_sistema && m.files.iter().any(|old| old.path == f.path))
        {
            let old_backup = format!("{raiz}/.respaldo/{}", f.path);
            let old_ausente = format!("{old_backup}.ausente");
            if let Some(data) = fs::read_bytes(&old_backup, MAX_PACK_SIZE as usize) {
                escribir_ruta_sistema(&backup, &data)?;
            } else if fs::exists(&old_ausente) {
                escribir_ruta_sistema(&ausente, b"1")?;
            } else if let Some(data) = fs::read_bytes(&destino, MAX_PACK_SIZE as usize) {
                // Paquete instalado por un gestor anterior, que ignoraba
                // destino=sistema: el archivo de sistema aún conserva su valor.
                escribir_ruta_sistema(&backup, &data)?;
            } else {
                escribir_ruta_sistema(&ausente, b"1")?;
            }
        } else if let Some(data) = fs::read_bytes(&destino, MAX_PACK_SIZE as usize) {
            escribir_ruta_sistema(&backup, &data)?;
        } else {
            escribir_ruta_sistema(&ausente, b"1")?;
        }
    }
    Ok(())
}

/// ¿Tiene ya `destino` el contenido que dice el manifiesto? Primero el
/// tamaño (barato) y sólo si coincide, el hash.
fn destino_al_dia(destino: &str, f: &soso_pkg_core::FileEntry) -> bool {
    fs::size(destino) == Some(f.size)
        && fs::read_bytes(destino, f.size as usize + 1)
            .is_some_and(|b| soso_update_core::hex_sha256(&b) == f.hash_hex)
}

fn escribir_ruta_sistema(path: &str, data: &[u8]) -> Result<(), ()> {
    if let Some(fin) = path.rfind('/') {
        fs::mkdir_all(&path[..fin]);
    }
    fs::write_bytes_replace(path, data).map_err(|_| ())
}

fn activar_destinos_sistema(man: &Manifest) -> Vec<String> {
    let mut errores = Vec::new();
    let raiz = format!("/var/paquetes/{}", man.nombre);
    for f in &man.files {
        let Some(destino) = soso_pkg_core::rutas::ruta_destino_sistema(&f.path) else {
            continue;
        };
        let origen = format!("{raiz}/{}", f.path);
        let Some(data) = fs::read_bytes(&origen, MAX_PACK_SIZE as usize) else {
            errores.push(destino);
            continue;
        };
        if escribir_ruta_sistema(&destino, &data).is_err() {
            errores.push(destino);
        }
    }
    errores
}

/// Índice de los paquetes que trae el propio sistema (llega con cada
/// actualización): de ahí sale qué versión de cada programa es la del sistema.
const INDICE_DEL_SISTEMA: &str = "/share/paquetes/indice.txt";

fn indice_del_sistema() -> Option<soso_pkg_core::Indice> {
    let bytes = fs::read_bytes(INDICE_DEL_SISTEMA, MAX_TXT)?;
    soso_pkg_core::Indice::parse(core::str::from_utf8(&bytes).ok()?).ok()
}

fn sistema_mas_nuevo(idx: Option<&soso_pkg_core::Indice>, man: &Manifest) -> Option<String> {
    let del_sistema = idx?.mejor(&man.nombre)?.version_raw.clone();
    soso_pkg_core::manda_el_sistema(&man.version_raw, Some(&del_sistema)).then_some(del_sistema)
}

/// El sistema trae una versión más nueva que el paquete: sus ficheros se
/// quedan y pasan a ser lo que restaura `borrar`, que si no repondría lo que
/// había antes de instalar el paquete.
fn ceder_al_sistema(man: &Manifest) {
    let raiz = format!("/var/paquetes/{}", man.nombre);
    for f in &man.files {
        let Some(destino) = soso_pkg_core::rutas::ruta_destino_sistema(&f.path) else {
            continue;
        };
        let Some(actual) = fs::read_bytes(&destino, MAX_PACK_SIZE as usize) else {
            continue;
        };
        let backup = format!("{raiz}/.respaldo/{}", f.path);
        let igual = fs::size(&backup) == Some(actual.len() as u64)
            && fs::read_bytes(&backup, actual.len() + 1).as_deref() == Some(&actual[..]);
        if !igual {
            let _ = escribir_ruta_sistema(&backup, &actual);
        }
        let ausente = format!("{backup}.ausente");
        if fs::exists(&ausente) {
            let _ = fs::unlink(&ausente);
        }
    }
}

/// Permite que un soso-pkg antiguo instale esta versión en `/var/paquetes`;
/// al ejecutarla por primera vez, la versión nueva activa los destinos de
/// sistema y conserva el binario que había antes.
fn activar_paquetes_sistema_instalados() {
    let idx = indice_del_sistema();
    for nombre in fs::list_dir("/var/paquetes") {
        if nombre.starts_with('.') {
            continue;
        }
        let Some(man) = manifest_instalada(&nombre) else {
            continue;
        };
        if !man.destino_sistema {
            continue;
        }
        // Una actualización del sistema trajo una versión más nueva que la
        // del paquete: reponer la del paquete la degradaría en cada orden.
        if sistema_mas_nuevo(idx.as_ref(), &man).is_some() {
            ceder_al_sistema(&man);
            continue;
        }
        let raiz = format!("/var/paquetes/{nombre}");
        for f in &man.files {
            let Some(destino) = soso_pkg_core::rutas::ruta_destino_sistema(&f.path) else {
                continue;
            };
            let backup = format!("{raiz}/.respaldo/{}", f.path);
            let ausente = format!("{backup}.ausente");
            if !fs::exists(&backup) && !fs::exists(&ausente) {
                if let Some(previo) = fs::read_bytes(&destino, MAX_PACK_SIZE as usize) {
                    let _ = escribir_ruta_sistema(&backup, &previo);
                } else {
                    let _ = escribir_ruta_sistema(&ausente, b"1");
                }
            }
            // Sólo si hace falta: reescribir un destino al día cambia su inodo,
            // y si es un ejecutable en marcha —el propio /bin/soso-pkg, que
            // pasa por aquí en cada ejecución— pierde las páginas que aún no
            // había cargado y muere con un page fault al final de la orden.
            // Además así no se copian en cada orden los 37 MB de `sal`.
            if destino_al_dia(&destino, f) {
                continue;
            }
            if let Some(data) =
                fs::read_bytes(&format!("{raiz}/{}", f.path), MAX_PACK_SIZE as usize)
            {
                let _ = escribir_ruta_sistema(&destino, &data);
            }
        }
    }
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

fn cmd_actualizar(nombre: Option<&str>, forzar: bool, origen_cli: &[String]) -> u8 {
    let (cat, origenes) = match cargar_catalogo(resolver_origenes(origen_cli)) {
        Ok(x) => x,
        Err(c) => return c,
    };
    let instalados = paquetes_instalados();
    let mut rc = 0u8;
    for (n, man) in instalados {
        if nombre.is_some_and(|filtro| filtro != n) {
            continue;
        }
        let Some((_, ent)) = cat.mejor(&n) else {
            continue;
        };
        let rem = match parse_version(&ent.version_raw) {
            Some(v) => v,
            None => continue,
        };
        if decidir(&rem, Some(&man.version), forzar) != AccionInstalacion::Actualizar {
            continue;
        }
        if instalar_remoto(&n, forzar, &cat, &origenes) != 0 {
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
    if !dependientes_compatibles(nombre, None) {
        return 1;
    }
    if proceso_usa_paquete(nombre) {
        println!("soso-pkg: hay procesos usando «{nombre}»");
        return 1;
    }
    if let Some(man) = manifest_instalada(nombre) {
        if man.destino_sistema {
            for f in &man.files {
                let Some(destino) = soso_pkg_core::rutas::ruta_destino_sistema(&f.path) else {
                    continue;
                };
                let backup = format!("{dest}/.respaldo/{}", f.path);
                let ausente = format!("{backup}.ausente");
                if let Some(data) = fs::read_bytes(&backup, MAX_PACK_SIZE as usize) {
                    if escribir_ruta_sistema(&destino, &data).is_err() {
                        println!("soso-pkg: no pude restaurar {destino}");
                        return 1;
                    }
                } else if fs::exists(&ausente) {
                    let _ = fs::unlink(&destino);
                }
                // Sin archivo de estado significa que lo instaló un gestor
                // anterior y /bin nunca se modificó: se conserva tal cual.
            }
        }
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
