//! `soso-improve` dentro de soso.
//!
//! El objetivo del plan es que la automejora pueda correr **en soso**, no solo
//! en el host. La lógica ya vive en `soso-improve-core`, que es
//! `no_std + alloc` y habla con el sistema solo por traits; aquí están esos
//! traits implementados con las llamadas del sistema que soso ya tiene:
//! `open`/`read`/`write`/`stat`/`getdents` para archivos y `spawn_io`+`wait`
//! para procesos.
//!
//! Lo que **sí** se puede hacer hoy dentro de soso:
//!
//! - capturar una base y reconstruirla (el formato guarda contenidos, no
//!   parches de git: no hace falta `clone` ni `apply`);
//! - cargar y validar el banco de casos;
//! - verificar un caso de protocolo sobre una respuesta grabada.
//!
//! Lo que **no**, y por qué:
//!
//! - `verificar programa` necesita un compilador de verdad — `soso-rustc` es
//!   un stub hasta T40;
//! - `verificar repo` necesita cargo — T41.
//!
//! Esas dos órdenes existen y contestan «capacidad ausente» con el nombre de la
//! ficha que la traerá. Fingir que se ejecutaron sería peor que no tenerlas.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use libsoso::{abi, println, sys};
use soso_improve_core::captura::{self as base, Politica};
use soso_improve_core::caso;
use soso_improve_core::entorno::{Archivos, Entrada, Orden, Procesos, Salida, Tipo};
use soso_improve_core::ignorar::Reglas;
use soso_improve_core::{protocolo, unir, Error, Resultado};

libsoso::entry!(main);

/// El árbol y los procesos de soso.
struct Soso;

fn fallo(que: &str, codigo: i64) -> Error {
    Error::entorno(format!("{que}: errno {codigo}"))
}

impl Archivos for Soso {
    fn leer(&self, ruta: &str) -> Resultado<Vec<u8>> {
        let fd = sys::open(ruta, abi::O_RDONLY);
        if fd < 0 {
            return Err(fallo(&format!("abrir {ruta}"), fd));
        }
        let mut fuera = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = sys::read(fd as u64, &mut buf);
            if n < 0 {
                sys::close(fd as u64);
                return Err(fallo(&format!("leer {ruta}"), n));
            }
            if n == 0 {
                break;
            }
            fuera.extend_from_slice(&buf[..n as usize]);
        }
        sys::close(fd as u64);
        Ok(fuera)
    }

    fn escribir(&mut self, ruta: &str, datos: &[u8], _modo: u32) -> Resultado<()> {
        // Los directorios intermedios se crean uno a uno: `mkdir -p` no existe
        // como llamada.
        if let Some(corte) = ruta.rfind('/') {
            let mut acumulado = String::new();
            for seg in ruta[..corte].split('/') {
                if seg.is_empty() {
                    acumulado.push('/');
                    continue;
                }
                if !acumulado.is_empty() && !acumulado.ends_with('/') {
                    acumulado.push('/');
                }
                acumulado.push_str(seg);
                sys::mkdir(&acumulado); // si ya existe, da igual
            }
        }
        let fd = sys::open(ruta, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
        if fd < 0 {
            return Err(fallo(&format!("crear {ruta}"), fd));
        }
        let mut escrito = 0;
        while escrito < datos.len() {
            let n = sys::write(fd as u64, &datos[escrito..]);
            if n <= 0 {
                sys::close(fd as u64);
                return Err(fallo(&format!("escribir {ruta}"), n));
            }
            escrito += n as usize;
        }
        sys::close(fd as u64);
        Ok(())
    }

    fn existe(&self, ruta: &str) -> bool {
        let mut st = abi::Stat::default();
        sys::stat(ruta, &mut st) >= 0
    }

    fn listar(&self, ruta: &str) -> Resultado<Vec<Entrada>> {
        let fd = sys::open(ruta, abi::O_RDONLY);
        if fd < 0 {
            return Err(fallo(&format!("abrir {ruta}"), fd));
        }
        let mut fuera = Vec::new();
        let mut buf = [abi::Dirent::default(); 16];
        loop {
            let n = sys::getdents(fd as u64, &mut buf);
            if n < 0 {
                sys::close(fd as u64);
                return Err(fallo(&format!("listar {ruta}"), n));
            }
            if n == 0 {
                break;
            }
            // `getdents` devuelve **bytes**, no entradas: dividir por el tamaño
            // del registro. Tomarlo como un contador lee basura del búfer.
            for d in buf.iter().take(n as usize / abi::DIRENT_SIZE) {
                let nombre = match core::str::from_utf8(d.name_bytes()) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if nombre == "." || nombre == ".." {
                    continue;
                }
                let mut st = abi::Stat::default();
                let completa = unir(ruta, nombre);
                let tipo = if sys::stat(&completa, &mut st) >= 0 {
                    if st.file_type == abi::FT_DIR {
                        Tipo::Directorio
                    } else if st.file_type == abi::FT_FILE {
                        Tipo::Archivo
                    } else {
                        Tipo::Otro
                    }
                } else {
                    Tipo::Otro
                };
                fuera.push(Entrada {
                    ruta: nombre.to_string(),
                    tipo,
                    bytes: if tipo == Tipo::Archivo { st.size } else { 0 },
                    // soso no guarda permisos POSIX: se declara 0 y la
                    // reconstrucción no los exige.
                    modo: 0,
                });
            }
        }
        sys::close(fd as u64);
        Ok(fuera)
    }

    fn metadatos(&self, ruta: &str) -> Resultado<Entrada> {
        let mut st = abi::Stat::default();
        let r = sys::stat(ruta, &mut st);
        if r < 0 {
            return Err(fallo(&format!("stat {ruta}"), r));
        }
        Ok(Entrada {
            ruta: ruta.to_string(),
            tipo: if st.file_type == abi::FT_DIR {
                Tipo::Directorio
            } else if st.file_type == abi::FT_FILE {
                Tipo::Archivo
            } else {
                Tipo::Otro
            },
            bytes: st.size,
            modo: 0,
        })
    }

    fn crear_directorio(&mut self, ruta: &str) -> Resultado<()> {
        let mut acumulado = String::new();
        for seg in ruta.split('/') {
            if seg.is_empty() {
                acumulado.push('/');
                continue;
            }
            if !acumulado.is_empty() && !acumulado.ends_with('/') {
                acumulado.push('/');
            }
            acumulado.push_str(seg);
            sys::mkdir(&acumulado);
        }
        Ok(())
    }

    fn borrar(&mut self, ruta: &str) -> Resultado<()> {
        let r = sys::unlink(ruta);
        if r < 0 {
            return Err(fallo(&format!("borrar {ruta}"), r));
        }
        Ok(())
    }
}

impl Procesos for Soso {
    fn ejecutar(&self, orden: &Orden) -> Resultado<Salida> {
        if orden.argv.is_empty() {
            return Err(Error::uso("argv vacío"));
        }
        // `spawn_io` reparte los descriptores; la salida se recoge por una
        // tubería. El cwd no se puede fijar por proceso: se cambia antes.
        let (lector, escritor) = sys::pipe().map_err(|e| fallo("pipe", e))?;
        if !orden.cwd.is_empty() {
            sys::chdir(&orden.cwd);
        }
        let argumentos = orden.argv[1..].join(" ");
        let pid = sys::spawn_io(&orden.argv[0], &argumentos, 0, escritor, escritor);
        sys::close(escritor);
        if pid < 0 {
            sys::close(lector);
            return Ok(Salida {
                codigo: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
                motivo: Some(format!("no se pudo lanzar {}", orden.argv[0])),
            });
        }
        let mut stdout = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = sys::read(lector, &mut buf);
            if n <= 0 {
                break;
            }
            stdout.extend_from_slice(&buf[..n as usize]);
        }
        sys::close(lector);
        let codigo = sys::wait().map(|(_, estado)| estado as i32).ok();
        Ok(Salida {
            codigo,
            stdout,
            stderr: Vec::new(),
            motivo: None,
        })
    }
}

const USO: &str = "uso: soso-improve capturar <arbol> <destino>\n\
                   \x20      soso-improve reconstruir <captura> <destino>\n\
                   \x20      soso-improve banco <dir> listar|validar\n\
                   \x20      soso-improve protocolo <dir-banco> <caso> <respuesta>";

fn politica(arbol: &str) -> Politica {
    let ruta = unir(arbol, ".gitignore");
    match Soso.leer(&ruta) {
        Ok(datos) => match core::str::from_utf8(&datos).ok().map(Reglas::parsear) {
            Some(Ok(reglas)) => Politica::default().con_ignorados(reglas),
            _ => Politica::default(),
        },
        Err(_) => Politica::default(),
    }
}

fn capturar(arbol: &str, destino: &str) -> Resultado<()> {
    let mut soso = Soso;
    soso.crear_directorio(destino)?;
    let manifiesto = base::capturar(
        &Soso,
        arbol,
        &mut soso,
        destino,
        &politica(arbol),
        base::Git {
            disponible: false,
            nota: Some("dentro de soso no hay git: la base es su inventario".to_string()),
            ..Default::default()
        },
        alloc::collections::BTreeMap::new(),
        Vec::new(),
        base::suites_declaradas(),
        "guest",
    )?;
    println!(
        "captura en {destino}: {} archivo(s), huella {}",
        manifiesto.inventario.len(),
        &manifiesto.estabilidad.huella[..16]
    );
    Ok(())
}

fn reconstruir(captura: &str, destino: &str) -> Resultado<()> {
    let manifiesto = base::leer_manifiesto(&Soso, captura)?;
    let mut soso = Soso;
    soso.crear_directorio(destino)?;
    let escritos = base::reconstruir(&Soso, captura, &mut soso, destino, &manifiesto)?;
    let verificacion = base::verificar(&Soso, destino, &manifiesto)?;
    if verificacion.ok {
        println!("reconstrucción verificada en {destino}: {escritos} archivo(s)");
    } else {
        println!(
            "reconstrucción con {} problema(s)",
            verificacion.problemas.len()
        );
    }
    Ok(())
}

fn banco(dir: &str, sub: &str) -> Resultado<()> {
    let casos = caso::cargar(&Soso, dir)?;
    match sub {
        "listar" => {
            for c in &casos {
                println!("{}  {}  {}", c.caso.id, c.caso.particion, c.caso.titulo);
            }
        }
        _ => {
            let reservado = unir(dir, "reservado");
            let problemas = caso::comprobar(&Soso, dir, &reservado, &casos)?;
            for p in &problemas {
                println!("problema: {p}");
            }
            println!("{} casos, {} problema(s)", casos.len(), problemas.len());
        }
    }
    Ok(())
}

fn verificar_protocolo(dir: &str, id: &str, respuesta: &str) -> Resultado<()> {
    let reservado = unir(dir, "reservado");
    let esperado: protocolo::Esperado =
        serde_json_core_compat(&Soso.leer(&unir(&reservado, &format!("{id}/esperado.json")))?)?;
    let crudo: serde_json::Value = serde_json_core_compat(&Soso.leer(respuesta)?)?;
    let informe = protocolo::verificar(id, &esperado, &crudo)?;
    println!(
        "{id}: {} ({}/{} aserciones)",
        informe.estado, informe.pasadas, informe.total
    );
    for a in informe.aserciones.iter().filter(|a| !a.paso) {
        println!("  {}: {}", a.tipo, a.motivo.clone().unwrap_or_default());
    }
    Ok(())
}

fn serde_json_core_compat<T: serde::de::DeserializeOwned>(datos: &[u8]) -> Resultado<T> {
    serde_json::from_slice(datos).map_err(Error::formato)
}

fn main(args: &str) -> u8 {
    let piezas: Vec<&str> = args.split_whitespace().collect();
    let resultado = match piezas.as_slice() {
        ["capturar", arbol, destino] => capturar(arbol, destino),
        ["reconstruir", captura, destino] => reconstruir(captura, destino),
        ["banco", dir, sub] => banco(dir, sub),
        ["protocolo", dir, id, respuesta] => verificar_protocolo(dir, id, respuesta),
        ["programa", ..] => Err(Error::uso(
            "verificar programa necesita un compilador: `soso-rustc` es un stub hasta T40",
        )),
        ["repo", ..] => Err(Error::uso(
            "verificar repo necesita cargo dentro de soso: T41",
        )),
        _ => {
            println!("{USO}");
            return 2;
        }
    };
    match resultado {
        Ok(()) => 0,
        Err(e) => {
            println!("error: {e}");
            1
        }
    }
}
