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
use soso_improve_core::cli::{self, Capacidad, Capacidades, Codigo, Informe};
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

/// Capacidad del búfer de una tubería en soso (`kernel/src/task/pipe.rs`).
/// Con 4 KiB, cualquier cosa que no quepa de una vez obliga a alternar lectura
/// y escritura, y eso hoy no se puede hacer sin bloquear (T61).
const PIPE_CAP: usize = 4096;

/// Tope de lo que se recoge de un hijo. Un proceso que escupe sin parar no
/// puede agotar la memoria del coordinador.
const SALIDA_MAX: usize = 1024 * 1024;

/// Texto hasta el primer NUL. `getcwd` rellena así el búfer.
fn cadena_c(buf: &[u8]) -> Option<&str> {
    let fin = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    core::str::from_utf8(&buf[..fin]).ok()
}

/// Espera a **nuestro** hijo, no al primero que salga.
///
/// `wait()` devuelve `(pid, estado)` de cualquier hijo. El adaptador anterior
/// se quedaba con el primero, así que con dos hijos vivos atribuía a uno el
/// código de salida del otro. Aquí se reconoce el pid y los ajenos se
/// conservan, para no perder su estado por el camino.
fn esperar_a(pid: u64, plazo: Plazo, reloj: &RelojSoso, ajenos: &mut Vec<(u64, u8)>) -> Option<u8> {
    loop {
        match sys::wait() {
            Ok((p, estado)) if p == pid => return Some(estado),
            Ok(otro) => ajenos.push(otro),
            Err(_) => return None,
        }
        if plazo.vencido(reloj) {
            return None;
        }
    }
}

impl Procesos for Soso {
    fn ejecutar(&self, orden: &Orden) -> Resultado<Salida> {
        if orden.argv.is_empty() {
            return Err(Error::uso("argv vacío"));
        }
        // Capacidades ausentes: se rechazan **antes** de lanzar nada, con el
        // nombre de la ficha que las traerá. Ignorar un campo en silencio es lo
        // que T47 viene a quitar.
        if orden.stdin.len() > PIPE_CAP {
            return Err(Error::uso(format!(
                "capacidad ausente: {} bytes de stdin pasan del búfer de tubería ({PIPE_CAP}); \
                 escribir más exige alternar con la lectura, que llega en T61",
                orden.stdin.len()
            )));
        }

        let (salida_r, salida_w) = sys::pipe().map_err(|e| fallo("pipe de salida", e))?;
        let (entrada_r, entrada_w) = match sys::pipe() {
            Ok(p) => p,
            Err(e) => {
                sys::close(salida_r);
                sys::close(salida_w);
                return Err(fallo("pipe de entrada", e));
            }
        };

        // argv de verdad: la ABI lleva tabla (`argv_ptr`/`argv_count`), así que
        // un argumento con espacios viaja entero. El adaptador anterior los
        // juntaba con espacios y el hijo recibía otra cosa.
        let argv: Vec<&str> = orden.argv.iter().map(|s| s.as_str()).collect();
        let entorno: Vec<String> = orden
            .entorno
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let entorno_ref: Vec<&str> = entorno.iter().map(|s| s.as_str()).collect();

        // N-003: el directorio va **en el spawn**, no en un `chdir` del
        // proceso. El apaño anterior (`CwdGuardado`: cambiar, lanzar y
        // restaurar por `Drop`) funcionaba, pero movía el directorio de todo
        // el proceso durante la ventana — y con hilos eso lo ve todo el mundo.
        let pid = sys::spawn_io_cwd(
            &orden.argv[0],
            &argv,
            &entorno_ref,
            [entrada_r, salida_w, salida_w, abi::FD_KERNEL_LOG],
            &orden.cwd,
        );
        // Los extremos del hijo se cierran aquí: si no, la tubería nunca da EOF.
        sys::close(salida_w);
        sys::close(entrada_r);
        if pid < 0 {
            sys::close(salida_r);
            sys::close(entrada_w);
            return Ok(Salida {
                codigo: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
                motivo: Some(format!(
                    "no se pudo lanzar {} (errno {pid})",
                    orden.argv[0]
                )),
            });
        }
        let pid = pid as u64;

        if !orden.stdin.is_empty() {
            let _ = sys::write_all(entrada_w, &orden.stdin);
        }
        // Cerrar siempre: el hijo espera EOF en su stdin.
        sys::close(entrada_w);

        let reloj = RelojSoso;
        let plazo = match orden.timeout_s {
            Some(s) => Plazo::en(&reloj, (s as u64).saturating_mul(1000)),
            None => Plazo::infinito(),
        };

        let mut salida = Vec::new();
        let mut buf = [0u8; 1024];
        let mut truncada = false;
        let mut vencido = false;
        loop {
            let n = sys::read(salida_r, &mut buf);
            if n <= 0 {
                break;
            }
            if salida.len() + n as usize > SALIDA_MAX {
                truncada = true;
                break;
            }
            salida.extend_from_slice(&buf[..n as usize]);
            if plazo.vencido(&reloj) {
                vencido = true;
                break;
            }
        }
        sys::close(salida_r);

        if vencido {
            // Matar antes de esperar: si no, el `wait` se queda con el hijo.
            sys::kill(pid as i64, abi::SIGKILL as u64);
        }

        let mut ajenos: Vec<(u64, u8)> = Vec::new();
        let estado = esperar_a(pid, Plazo::infinito(), &reloj, &mut ajenos);

        // Lo que no se ha podido cumplir se **dice**, no se calla.
        let mut notas: Vec<String> = Vec::new();
        notas.push(String::from(
            "stdout y stderr van mezclados: separarlos exige leer sin bloquear (T61)",
        ));
        if truncada {
            notas.push(format!("salida truncada en {SALIDA_MAX} bytes"));
        }
        if vencido {
            notas.push(format!(
                "plazo de {}s agotado; hijo terminado",
                orden.timeout_s.unwrap_or(0)
            ));
        }
        if !ajenos.is_empty() {
            notas.push(format!("{} hijo(s) ajeno(s) recogidos", ajenos.len()));
        }

        Ok(Salida {
            codigo: if vencido {
                None
            } else {
                estado.map(|e| e as i32)
            },
            stdout: salida,
            stderr: Vec::new(),
            motivo: Some(notas.join("; ")),
        })
    }
}

const USO: &str = "uso: soso-improve <orden> [--clave valor | posicional]\n\
                   \x20      capturar    --repo <arbol> --out <destino>\n\
                   \x20      reconstruir --captura <c> --destino <d>\n\
                   \x20      banco       listar|validar --banco <dir>\n\
                   \x20      verificar   protocolo --banco <dir> --caso <id> --respuesta <r>\n\
                   \x20      eco         [--puerto N]   (prueba de reloj y transporte, T48)\n\
                   \x20      procesos                   (prueba del adaptador de procesos, T47)\n\
                   \x20      pruebas     [--banco <dir>] (runner de pruebas en soso, T49)\n\
                   \x20      capacidades                (lista lo que este frontend sabe hacer)\n\
                   \x20      --orden-archivo <ruta>     (argv por archivo, una línea por argumento)\n\
                   \n\
                   \x20 codigos: 0 bien, 1 no se pudo usar, 2 falla la medida,\n\
                   \x20          3 captura inestable, 4 alguna suite fallo";

/// Lo que este frontend sabe hacer **de verdad**. `verificar programa` y
/// `verificar repo` no están: necesitan compilador (T40) y cargo (T41), y
/// fingir que se ejecutaron sería peor que no tenerlas.
fn capacidades() -> Capacidades {
    Capacidades::nueva(
        "guest",
        &[
            Capacidad::Capturar,
            Capacidad::Reconstruir,
            Capacidad::BancoListar,
            Capacidad::BancoValidar,
            Capacidad::VerificarProtocolo,
            Capacidad::Eco,
            Capacidad::Procesos,
            Capacidad::Pruebas,
            Capacidad::Delta,
            Capacidad::TareaPreparar,
            Capacidad::TareaReanudar,
            Capacidad::TareaInforme,
        ],
    )
}

/// Tope de argumentos en un archivo de orden: suficiente para cualquier orden
/// real y acotado para no leer un archivo cualquiera entero.
const MAX_ARGS_ARCHIVO: usize = 64;

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

fn capturar(arbol: &str, destino: &str) -> Resultado<usize> {
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
    Ok(0)
}

/// Devuelve cuántos problemas encontró la verificación: **cero no es lo mismo
/// que uno**, y antes las dos cosas salían con código 0.
fn reconstruir(captura: &str, destino: &str) -> Resultado<usize> {
    let manifiesto = base::leer_manifiesto(&Soso, captura)?;
    let mut soso = Soso;
    soso.crear_directorio(destino)?;
    let escritos = base::reconstruir(&Soso, captura, &mut soso, destino, &manifiesto)?;
    let verificacion = base::verificar(&Soso, destino, &manifiesto)?;
    if verificacion.ok {
        println!("reconstrucción verificada en {destino}: {escritos} archivo(s)");
        return Ok(0);
    }
    println!(
        "reconstrucción con {} problema(s)",
        verificacion.problemas.len()
    );
    for p in verificacion.problemas.iter().take(20) {
        println!("  {}: esperaba {}, obtuvo {}", p.ruta, p.esperado, p.obtenido);
    }
    Ok(verificacion.problemas.len())
}

fn banco(dir: &str, sub: &str) -> Resultado<usize> {
    let casos = caso::cargar(&Soso, dir)?;
    match sub {
        "listar" => {
            for c in &casos {
                println!("{}  {}  {}", c.caso.id, c.caso.particion, c.caso.titulo);
            }
            Ok(0)
        }
        _ => {
            let reservado = unir(dir, "reservado");
            let problemas = caso::comprobar(&Soso, dir, &reservado, &casos)?;
            for p in &problemas {
                println!("problema: {p}");
            }
            println!("{} casos, {} problema(s)", casos.len(), problemas.len());
            Ok(problemas.len())
        }
    }
}

fn verificar_protocolo(dir: &str, id: &str, respuesta: &str) -> Resultado<usize> {
    let reservado = unir(dir, "reservado");
    let esperado: protocolo::Esperado =
        serde_json_core_compat(&Soso.leer(&unir(&reservado, &format!("{id}/esperado.json")))?)?;
    let crudo: serde_json::Value = serde_json_core_compat(&Soso.leer(respuesta)?)?;
    let informe = protocolo::verificar(id, &esperado, &crudo)?;
    println!(
        "{id}: {} ({}/{} aserciones)",
        informe.estado, informe.pasadas, informe.total
    );
    let fallidas = informe.aserciones.iter().filter(|a| !a.paso).count();
    for a in informe.aserciones.iter().filter(|a| !a.paso) {
        println!("  {}: {}", a.tipo, a.motivo.clone().unwrap_or_default());
    }
    Ok(fallidas)
}

fn serde_json_core_compat<T: serde::de::DeserializeOwned>(datos: &[u8]) -> Resultado<T> {
    serde_json::from_slice(datos).map_err(Error::formato)
}

/// Ejecuta una orden ya interpretada y devuelve cuántos problemas encontró.
/// Los efectos empiezan **después** de comprobar la capacidad.
fn ejecutar(orden: &cli::Orden) -> Resultado<usize> {
    let capacidad = orden.capacidad()?;
    capacidades().exigir(capacidad)?;
    match capacidad {
        Capacidad::Capturar => capturar(orden.exigido("repo")?, orden.exigido("out")?),
        Capacidad::Reconstruir => {
            reconstruir(orden.exigido("captura")?, orden.exigido("destino")?)
        }
        Capacidad::BancoListar => banco(orden.exigido("banco")?, "listar"),
        Capacidad::BancoValidar => banco(orden.exigido("banco")?, "validar"),
        Capacidad::VerificarProtocolo => verificar_protocolo(
            orden.exigido("banco")?,
            orden.exigido("caso")?,
            orden.exigido("respuesta")?,
        ),
        Capacidad::Procesos => procesos(),
        Capacidad::Pruebas => pruebas(orden.uno("banco")),
        Capacidad::Delta => delta_prueba(),
        Capacidad::TareaPreparar => match orden.uno("spec") {
            Some(spec) => tarea_preparar(orden.exigido("estado")?, spec),
            // Sin enunciado, la orden es la autoprueba del formato: siembra el
            // suyo y recorre el ciclo. No finge haber preparado nada real.
            None => tarea_prueba(),
        },
        Capacidad::TareaReanudar => tarea_ver(orden.exigido("estado")?, orden.exigido("run")?, true),
        Capacidad::TareaInforme => {
            tarea_ver(orden.exigido("estado")?, orden.exigido("run")?, false)
        }
        Capacidad::Eco => {
            let puerto: u16 = orden
                .uno("puerto")
                .unwrap_or("9450")
                .parse()
                .map_err(|_| Error::uso("puerto inválido"))?;
            if orden.nombre == "eco-servidor" {
                eco_servidor(puerto)
            } else {
                eco(puerto)
            }
        }
        // `exigir` ya ha rechazado lo que este frontend no sabe hacer; llegar
        // aquí sería una capacidad declarada y no implementada.
        otra => Err(Error::uso(format!(
            "capacidad declarada sin implementación: {}",
            otra.nombre()
        ))),
    }
}

/// Argv de la orden.
///
/// Desde [T62](../../docs/self-improvement/T62-argv-en-los-programas.md) esto
/// recibe el argv de verdad, así que ya no hay que reconstruirlo. El desvío por
/// archivo se queda para el camino contrario: cuando la orden llega desde
/// `sosh`, cuyo tokenizador sigue partiendo por espacios sin entender comillas.
fn argv(args: &[String]) -> Resultado<Vec<String>> {
    if let [uno, ruta] = args {
        if uno == "--orden-archivo" {
            let datos = Soso.leer(ruta)?;
            let texto = core::str::from_utf8(&datos)
                .map_err(|_| Error::formato(format!("{ruta} no es UTF-8")))?;
            return cli::argv_de_texto(texto, MAX_ARGS_ARCHIVO);
        }
    }
    Ok(args.to_vec())
}

fn main(args: &[String]) -> u8 {
    let crudo = match argv(args) {
        Ok(v) => v,
        Err(e) => {
            println!("error: {e}");
            return Codigo::de_error(&e).como_u8();
        }
    };
    let refs: Vec<&str> = crudo.iter().map(|s| s.as_str()).collect();
    if refs.is_empty() || refs[0] == "-h" || refs[0] == "--help" {
        println!("{USO}");
        return if refs.is_empty() {
            Codigo::Error.como_u8()
        } else {
            Codigo::Exito.como_u8()
        };
    }
    // Sonda de T47: imprime el argv **real**, no la cadena que recibe `main`.
    // No es una capacidad del catálogo: es el hijo de su propia prueba.
    if refs[0] == "durable-fase1" {
        return match durable_fase1() {
            Ok(_) => Codigo::Exito.como_u8(),
            Err(e) => {
                println!("error: {e}");
                Codigo::de_error(&e).como_u8()
            }
        };
    }
    if refs[0] == "durable-fase2" {
        return match durable_fase2() {
            Ok(0) => Codigo::Exito.como_u8(),
            Ok(_) => Codigo::Verificacion.como_u8(),
            Err(e) => {
                println!("error: {e}");
                Codigo::de_error(&e).como_u8()
            }
        };
    }
    if refs[0] == "tuberias-hijo" {
        return match tuberias_hijo() {
            Ok(_) => Codigo::Exito.como_u8(),
            Err(_) => Codigo::Error.como_u8(),
        };
    }
    if refs[0] == "tuberias" {
        return match tuberias() {
            Ok(0) => Codigo::Exito.como_u8(),
            Ok(_) => Codigo::Verificacion.como_u8(),
            Err(e) => {
                println!("error: {e}");
                Codigo::de_error(&e).como_u8()
            }
        };
    }
    if refs[0] == "argv-eco" {
        for a in libsoso::argv() {
            println!("arg: {a}");
        }
        return Codigo::Exito.como_u8();
    }
    if refs[0] == "capacidades" {
        let caps = capacidades();
        for c in caps.lista() {
            println!("{}", c.nombre());
        }
        return Codigo::Exito.como_u8();
    }

    let orden = match cli::Orden::parsear(&refs) {
        Ok(o) => o,
        Err(e) => {
            println!("error: {e}");
            println!("{USO}");
            return Codigo::de_error(&e).como_u8();
        }
    };
    let informe = match ejecutar(&orden) {
        Ok(problemas) => Informe::segun_problemas(&orden, "guest", problemas),
        Err(e) => {
            println!("error: {e}");
            Informe::de_error(&orden, "guest", &e)
        }
    };
    if !informe.ok() {
        // Una verificación que falla se dice también por el código de salida:
        // imprimir FAIL y salir con 0 era el agujero que cierra T45.
        println!("resultado: {}", informe.json());
    }
    informe.codigo.como_u8()
}

// ---------------------------------------------------------------------------
// Reloj y transporte del guest (T48)
// ---------------------------------------------------------------------------

use soso_improve_core::tiempo::{Plazo, Reloj};
use soso_improve_core::transporte::{self, Conector, Destino, Paso, Transporte};

/// Reloj monotónico de soso. `uptime_ms` cuenta desde el arranque y no
/// retrocede; un valor negativo sólo puede venir de un errno, y entonces vale
/// más congelar el tiempo que inventarlo.
struct RelojSoso;

impl Reloj for RelojSoso {
    fn ahora_ms(&self) -> u64 {
        let t = sys::uptime_ms();
        if t < 0 { 0 } else { t as u64 }
    }
}

/// Conexión TCP de soso sobre `read_timeout`/`write`.
struct EnlaceSoso {
    fd: Option<u64>,
}

impl Transporte for EnlaceSoso {
    fn escribir(&mut self, datos: &[u8]) -> Resultado<Paso> {
        let Some(fd) = self.fd else {
            return Ok(Paso::Fin);
        };
        let n = sys::write(fd, datos);
        if n == -(abi::EAGAIN as i64) {
            return Ok(Paso::Espera);
        }
        if n == 0 {
            return Ok(Paso::Fin);
        }
        if n < 0 {
            return Err(fallo("escribir en el enlace", n));
        }
        Ok(Paso::Hecho(n as usize))
    }

    fn leer(&mut self, buf: &mut [u8], espera_ms: u64) -> Resultado<Paso> {
        let Some(fd) = self.fd else {
            return Ok(Paso::Fin);
        };
        // `read_timeout` con 0 es «sin plazo» y duerme para siempre (T55): la
        // espera nunca puede ser cero aquí.
        let espera = espera_ms.max(1);
        let n = sys::read_timeout(fd, buf, espera);
        if n == -(abi::EAGAIN as i64) {
            return Ok(Paso::Espera);
        }
        // Cero bytes es EOF: el otro extremo cerró. No es lo mismo que EAGAIN,
        // y todo el punto de T48 es no mezclarlos.
        if n == 0 {
            return Ok(Paso::Fin);
        }
        if n < 0 {
            return Err(fallo("leer del enlace", n));
        }
        Ok(Paso::Hecho(n as usize))
    }

    fn cerrar(&mut self) {
        if let Some(fd) = self.fd.take() {
            sys::close(fd);
        }
    }
}

struct ConectorSoso;

impl Conector for ConectorSoso {
    type Enlace = EnlaceSoso;

    fn conectar(&self, destino: &Destino, plazo: Plazo, reloj: &dyn Reloj) -> Resultado<EnlaceSoso> {
        let addr = sys::sock_addr(
            destino.ip[0],
            destino.ip[1],
            destino.ip[2],
            destino.ip[3],
            destino.puerto,
        );
        let espera = plazo.restante_ms(reloj).min(30_000).max(1);
        let fd = sys::tcp_connect(&addr, espera);
        if fd == -(abi::EAGAIN as i64) {
            return Err(Error::plazo(format!("conectando a {}", destino.texto())));
        }
        if fd < 0 {
            return Err(fallo(&format!("conectar a {}", destino.texto()), fd));
        }
        Ok(EnlaceSoso { fd: Some(fd as u64) })
    }
}

// ---------------------------------------------------------------------------
// Eco controlado entre procesos soso (T48, comprobación de la ficha)
// ---------------------------------------------------------------------------

/// Cabecera del protocolo de eco: modo y longitud en big-endian.
///
/// Longitud explícita a propósito: sin ella el servidor tendría que esperar un
/// EOF para saber que la petición terminó, y soso no expone medio cierre desde
/// userspace. Con la longitud, ninguno de los dos lados necesita adivinar.
const ECO_CABECERA: usize = 3;
/// Eco completo y cerrar.
const ECO_ENTERO: u8 = b'E';
/// Eco de los primeros 4 bytes y cerrar: cierre temprano a media respuesta.
const ECO_CORTADO: u8 = b'C';
/// No contestar: el cliente tiene que vencer por plazo, no por cierre.
const ECO_MUDO: u8 = b'M';
const ECO_CORTE: usize = 4;
const ECO_MAX: usize = 4096;

fn eco_servidor(puerto: u16) -> Resultado<usize> {
    let escucha = sys::tcp_listen(puerto);
    if escucha < 0 {
        return Err(fallo(&format!("escuchar en {puerto}"), escucha));
    }
    let cliente = sys::tcp_accept(escucha as u64, 30_000);
    if cliente < 0 {
        sys::close(escucha as u64);
        return Err(fallo("aceptar", cliente));
    }
    let reloj = RelojSoso;
    let plazo = Plazo::en(&reloj, 30_000);
    let mut enlace = EnlaceSoso {
        fd: Some(cliente as u64),
    };

    let mut cabecera = [0u8; ECO_CABECERA];
    transporte::leer_exacto(&mut enlace, &reloj, plazo, &mut cabecera)?;
    let modo = cabecera[0];
    let largo = ((cabecera[1] as usize) << 8) | cabecera[2] as usize;
    if largo > ECO_MAX {
        enlace.cerrar();
        sys::close(escucha as u64);
        return Err(Error::uso(format!("eco: {largo} bytes pasan de {ECO_MAX}")));
    }
    let mut cuerpo = alloc::vec![0u8; largo];
    transporte::leer_exacto(&mut enlace, &reloj, plazo, &mut cuerpo)?;

    match modo {
        ECO_MUDO => {
            // Callar el tiempo suficiente para que el cliente venza por plazo.
            sys::sleep_ms(3_000);
        }
        ECO_CORTADO => {
            let n = ECO_CORTE.min(cuerpo.len());
            transporte::escribir_todo(&mut enlace, &reloj, plazo, &cuerpo[..n])?;
        }
        _ => transporte::escribir_todo(&mut enlace, &reloj, plazo, &cuerpo)?,
    }
    enlace.cerrar();
    sys::close(escucha as u64);
    Ok(0)
}

/// Un intento del cliente: abre conexión, manda la petición en los trozos que
/// se le digan y devuelve el enlace listo para leer.
fn eco_peticion(
    puerto: u16,
    modo: u8,
    cuerpo: &[u8],
    trozo: usize,
    pausa_ms: u64,
) -> Resultado<EnlaceSoso> {
    let reloj = RelojSoso;
    let plazo = Plazo::en(&reloj, 15_000);
    let mut enlace = ConectorSoso.conectar(&Destino::local(puerto), plazo, &reloj)?;
    let cabecera = [modo, (cuerpo.len() >> 8) as u8, cuerpo.len() as u8];
    transporte::escribir_todo(&mut enlace, &reloj, plazo, &cabecera)?;
    let mut i = 0;
    while i < cuerpo.len() {
        let fin = (i + trozo).min(cuerpo.len());
        transporte::escribir_todo(&mut enlace, &reloj, plazo, &cuerpo[i..fin])?;
        if pausa_ms > 0 {
            sys::sleep_ms(pausa_ms);
        }
        i = fin;
    }
    Ok(enlace)
}

fn eco_lanzar_servidor(puerto: u16) -> Resultado<()> {
    let arg = alloc::format!("eco-servidor {puerto}");
    let pid = sys::spawn_io(
        "/bin/soso-improve",
        &arg,
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
    );
    if pid < 0 {
        return Err(fallo("lanzar el servidor de eco", pid));
    }
    // El servidor necesita llegar a `tcp_listen` antes de que el cliente
    // conecte; si no, el connect da ECONNREFUSED y parece un fallo de red.
    sys::sleep_ms(300);
    Ok(())
}

/// Comprobación de T48 dentro de soso: fragmentación byte a byte, UTF-8
/// partido, cierre temprano, cliente lento y plazo agotado.
///
/// Cada caso dice **por qué** falló: un cierre y un plazo no son la misma
/// cosa, y el informe no los puede mezclar.
fn eco(puerto_base: u16) -> Resultado<usize> {
    let reloj = RelojSoso;
    let texto = "café ☕ ¡hola, soso!";
    let mut fallos = 0usize;
    let t0 = reloj.ahora_ms();

    // 1. Byte a byte, y 2. UTF-8 partido: el mismo texto multibyte enviado de
    //    uno en uno parte cada carácter entre escrituras.
    eco_lanzar_servidor(puerto_base)?;
    let mut enlace = eco_peticion(puerto_base, ECO_ENTERO, texto.as_bytes(), 1, 0)?;
    let plazo = Plazo::en(&reloj, 15_000);
    let mut vuelta = alloc::vec![0u8; texto.len()];
    match transporte::leer_exacto(&mut enlace, &reloj, plazo, &mut vuelta) {
        Ok(()) if vuelta == texto.as_bytes() => {
            println!("eco byte-a-byte + utf8 partido: ok ({} bytes)", vuelta.len())
        }
        Ok(()) => {
            fallos += 1;
            println!("eco byte-a-byte: el texto volvió cambiado");
        }
        Err(e) => {
            fallos += 1;
            println!("eco byte-a-byte: {e}");
        }
    }
    enlace.cerrar();

    // 3. Cierre temprano: el servidor contesta 4 bytes y cierra. Tiene que
    //    verse como cierre, **no** como plazo agotado.
    eco_lanzar_servidor(puerto_base + 1)?;
    let mut enlace = eco_peticion(puerto_base + 1, ECO_CORTADO, texto.as_bytes(), 64, 0)?;
    let plazo = Plazo::en(&reloj, 15_000);
    let mut vuelta = alloc::vec![0u8; texto.len()];
    match transporte::leer_exacto(&mut enlace, &reloj, plazo, &mut vuelta) {
        Err(Error::Entorno(m)) if m.contains("cerró") => {
            println!("cierre temprano: ok ({m})")
        }
        otro => {
            fallos += 1;
            println!("cierre temprano: esperaba un cierre, hubo {otro:?}");
        }
    }
    enlace.cerrar();

    // 4. Cliente lento: pausas entre trozos; el servidor no se rinde.
    eco_lanzar_servidor(puerto_base + 2)?;
    let mut enlace = eco_peticion(puerto_base + 2, ECO_ENTERO, b"lento pero seguro", 4, 120)?;
    let plazo = Plazo::en(&reloj, 15_000);
    let mut vuelta = alloc::vec![0u8; b"lento pero seguro".len()];
    match transporte::leer_exacto(&mut enlace, &reloj, plazo, &mut vuelta) {
        Ok(()) if vuelta == b"lento pero seguro" => println!("cliente lento: ok"),
        otro => {
            fallos += 1;
            println!("cliente lento: {otro:?}");
        }
    }
    enlace.cerrar();

    // 5. Plazo: el servidor calla. Tiene que vencer por plazo, y decirlo.
    eco_lanzar_servidor(puerto_base + 3)?;
    let mut enlace = eco_peticion(puerto_base + 3, ECO_MUDO, b"hay alguien?", 64, 0)?;
    let corto = Plazo::en(&reloj, 400);
    let mut vuelta = [0u8; 8];
    match transporte::leer_exacto(&mut enlace, &reloj, corto, &mut vuelta) {
        Err(Error::Plazo(m)) => println!("plazo agotado: ok ({m})"),
        otro => {
            fallos += 1;
            println!("plazo agotado: esperaba un plazo, hubo {otro:?}");
        }
    }
    enlace.cerrar();

    println!(
        "eco: {} caso(s) mal, {} ms monotónicos",
        fallos,
        reloj.ahora_ms().saturating_sub(t0)
    );
    Ok(fallos)
}

// ---------------------------------------------------------------------------
// Autoprueba del adaptador de procesos (T47)
// ---------------------------------------------------------------------------

/// Comprueba dentro de soso lo que el adaptador promete: argv entero, stdin,
/// código de salida y cwd restaurado.
///
/// Se apoya en binarios que ya están (`cat`), no en un fixture nuevo: lo que se
/// está probando es el adaptador, no el hijo.
fn procesos() -> Resultado<usize> {
    let mut fallos = 0usize;

    // 1. argv con espacios, el caso decisivo. Se lanza **este mismo binario**
    //    con `argv-eco`, que imprime el argv real de `libsoso::argv()`. Usar
    //    `cat` no serviría para juzgar al adaptador: los coreutils parten la
    //    cadena juntada que recibían en `main(&str)` y rompían el argumento
    //    aunque el adaptador lo pasara perfecto. Eso lo arregló T62; la sonda
    //    que lo comprueba desde un programa corriente es `argv_espacios`.
    let con_espacios = "uno dos  tres";
    let r = Soso.ejecutar(&Orden::nueva(
        &["/bin/soso-improve", "argv-eco", con_espacios],
        "",
    ))?;
    let texto = String::from_utf8_lossy(&r.stdout).into_owned();
    let lineas: Vec<&str> = texto.lines().filter(|l| l.starts_with("arg: ")).collect();
    if lineas.len() == 3 && lineas[2] == alloc::format!("arg: {con_espacios}") {
        println!("argv con espacios: ok");
    } else {
        fallos += 1;
        println!("argv con espacios: llegó {lineas:?}");
    }

    // 2. stdin. `cat` sin argumentos copia su entrada; si no se le cierra el
    //    extremo de escritura, se queda esperando para siempre.
    let entrada = "hola desde stdin\n";
    let r = Soso.ejecutar(&Orden::nueva(&["/bin/cat"], "").con_stdin(entrada.as_bytes()))?;
    if r.codigo == Some(0) && r.stdout == entrada.as_bytes() {
        println!("stdin: ok");
    } else {
        fallos += 1;
        println!(
            "stdin: código {:?}, salida {:?}",
            r.codigo,
            core::str::from_utf8(&r.stdout).unwrap_or("<no utf8>")
        );
    }

    // 3. Código de salida de un fallo. Antes se cogía el `wait` de cualquiera.
    let r = Soso.ejecutar(&Orden::nueva(&["/bin/cat", "/no/existe/t47"], ""))?;
    if r.codigo.is_some() && r.codigo != Some(0) {
        println!("código de fallo: ok ({:?})", r.codigo);
    } else {
        fallos += 1;
        println!("código de fallo: esperaba distinto de 0, hubo {:?}", r.codigo);
    }

    // 4. cwd restaurado. El adaptador anterior hacía `chdir` y no volvía.
    let mut antes = [0u8; 512];
    sys::getcwd(&mut antes);
    let antes_s = cadena_c(&antes).unwrap_or("").to_string();
    let _ = Soso.ejecutar(&Orden::nueva(&["/bin/cat", "/etc/soso-hw"], "/tmp"));
    let mut despues = [0u8; 512];
    sys::getcwd(&mut despues);
    let despues_s = cadena_c(&despues).unwrap_or("").to_string();
    if !antes_s.is_empty() && antes_s == despues_s {
        println!("cwd restaurado: ok ({antes_s})");
    } else {
        fallos += 1;
        println!("cwd restaurado: antes {antes_s:?}, después {despues_s:?}");
    }

    // 5. Capacidad ausente: un stdin que no cabe en la tubería se rechaza
    //    **antes** de lanzar nada, nombrando la ficha que lo traerá.
    let grande = alloc::vec![b'x'; PIPE_CAP + 1];
    match Soso.ejecutar(&Orden::nueva(&["/bin/cat"], "").con_stdin(&grande)) {
        Err(e) if format!("{e}").contains("T61") => {
            println!("stdin grande: ok (capacidad ausente, T61)")
        }
        otro => {
            fallos += 1;
            println!("stdin grande: esperaba capacidad ausente, hubo {otro:?}");
        }
    }

    println!("procesos: {fallos} caso(s) mal");
    Ok(fallos)
}

// ---------------------------------------------------------------------------
// Sonda de T61: leer dos tuberías sin bloquearse
// ---------------------------------------------------------------------------

/// Escribe mucho en fd 2 y poco en fd 1, para provocar el interbloqueo.
///
/// Es el hijo de la sonda: si el padre drena sólo stdout, este proceso se
/// queda atascado escribiendo en stderr en cuanto pasa de los 4 KiB del búfer,
/// y el padre espera un EOF que no llega nunca.
fn tuberias_hijo() -> Resultado<usize> {
    let ruido = [b'e'; 512];
    for _ in 0..24 {
        // 12 KiB, tres veces el búfer.
        let _ = sys::write_all(2, &ruido);
    }
    let _ = sys::write_all(1, b"FIN-STDOUT\n");
    Ok(0)
}

/// Drena las dos tuberías alternando, con plazo corto en cada lectura.
///
/// Sin la lectura con plazo de T61 esto se cuelga: es exactamente el caso que
/// la ficha pide reproducir.
fn tuberias() -> Resultado<usize> {
    let (out_r, out_w) = sys::pipe().map_err(|e| fallo("pipe out", e))?;
    let (err_r, err_w) = sys::pipe().map_err(|e| fallo("pipe err", e))?;
    let pid = sys::spawn_io_ex(
        "/bin/soso-improve",
        &["/bin/soso-improve", "tuberias-hijo"],
        &[],
        abi::FD_SERIAL_TTY,
        out_w,
        err_w,
    );
    sys::close(out_w);
    sys::close(err_w);
    if pid < 0 {
        sys::close(out_r);
        sys::close(err_r);
        return Err(fallo("lanzar el hijo de tuberías", pid));
    }

    let reloj = RelojSoso;
    let plazo = Plazo::en(&reloj, 20_000);
    let (mut fin_out, mut fin_err) = (false, false);
    let (mut n_out, mut n_err) = (0usize, 0usize);
    let mut buf = [0u8; 1024];
    while !(fin_out && fin_err) {
        if plazo.vencido(&reloj) {
            println!("tuberías: plazo agotado con out={n_out} err={n_err}");
            break;
        }
        for (fd, fin, total) in [
            (out_r, &mut fin_out, &mut n_out),
            (err_r, &mut fin_err, &mut n_err),
        ] {
            if *fin {
                continue;
            }
            let n = sys::read_timeout(fd, &mut buf, 20);
            if n == -(abi::EAGAIN as i64) {
                continue;
            }
            if n == 0 {
                *fin = true;
                continue;
            }
            if n < 0 {
                println!("tuberías: error {n} leyendo");
                *fin = true;
                continue;
            }
            *total += n as usize;
        }
    }
    sys::close(out_r);
    sys::close(err_r);
    let _ = sys::wait();

    let mut fallos = 0usize;
    if fin_out && fin_err && n_err >= 12_288 && n_out >= 11 {
        println!("tuberías alternadas: ok (out={n_out}, err={n_err})");
    } else {
        fallos += 1;
        println!(
            "tuberías alternadas: out={n_out} (fin {fin_out}), err={n_err} (fin {fin_err})"
        );
    }
    println!("tuberias: {fallos} caso(s) mal");
    Ok(fallos)
}

// ---------------------------------------------------------------------------
// Durabilidad en soso (T46)
// ---------------------------------------------------------------------------

use soso_improve_core::durable::{self, Durable};

impl Durable for Soso {
    fn crear_exclusivo(&mut self, ruta: &str, datos: &[u8]) -> Resultado<()> {
        // Los directorios intermedios, uno a uno: no hay `mkdir -p`.
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
                sys::mkdir(&acumulado);
            }
        }
        let fd = sys::open(ruta, abi::O_WRONLY | abi::O_CREAT | abi::O_EXCL);
        if fd == -(abi::EEXIST as i64) {
            return Err(Error::uso(format!("{ruta} ya existe")));
        }
        if fd < 0 {
            return Err(fallo(&format!("crear exclusivo {ruta}"), fd));
        }
        let fd = fd as u64;
        let mut escrito = 0usize;
        while escrito < datos.len() {
            let n = sys::write(fd, &datos[escrito..]);
            if n <= 0 {
                sys::close(fd);
                return Err(fallo(&format!("escribir {ruta}"), n));
            }
            escrito += n as usize;
        }
        // `fsync` **antes** de cerrar: sin esto la promesa de durabilidad es
        // una suposición. En sosofs es lo que materializa el fichero.
        let rc = sys::fsync(fd);
        sys::close(fd);
        if rc < 0 {
            return Err(fallo(&format!("fsync {ruta}"), rc));
        }
        Ok(())
    }

    fn sincronizar(&mut self, ruta: &str) -> Resultado<()> {
        let fd = sys::open(ruta, abi::O_WRONLY);
        if fd < 0 {
            return Err(fallo(&format!("abrir {ruta}"), fd));
        }
        let rc = sys::fsync(fd as u64);
        sys::close(fd as u64);
        if rc < 0 {
            return Err(fallo(&format!("fsync {ruta}"), rc));
        }
        Ok(())
    }
}

/// Dónde vive la referencia de la sonda.
const DUR_DIR: &str = "/var/self-improvement/t46";
const DUR_REF: &str = "estado";

/// Sonda de durabilidad, en dos fases **con un reinicio de la máquina en
/// medio**.
///
/// La ficha lo exige así y tiene razón: un mock no acredita durabilidad. Lo que
/// se comprueba no es que el código sepa elegir generación, que eso ya lo fijan
/// las pruebas de host; es que lo escrito **sobrevive al apagón** y que una
/// generación a medias no se cuela como buena.
///
/// - `fase1` publica dos versiones y deja a mano una tercera generación
///   **truncada**, como la dejaría un corte de corriente.
/// - `fase2`, tras reiniciar, comprueba que la vigente es la segunda entera y
///   que la truncada se ignora.
fn durable_fase1() -> Resultado<usize> {
    let mut soso = Soso;
    soso.crear_directorio(DUR_DIR)?;

    let g0 = durable::publicar(&mut soso, DUR_DIR, DUR_REF, b"version-uno", None)?;
    let h1 = soso_improve_core::sha256_hex(b"version-uno");
    let g1 = durable::publicar(&mut soso, DUR_DIR, DUR_REF, b"version-dos", Some(&h1))?;
    println!("fase1: publicadas g{g0} y g{g1}");

    // Una generación a medias, escrita a propósito: es lo que deja un corte
    // justo mientras se escribía la siguiente.
    let truncada = unir(DUR_DIR, &format!("{DUR_REF}.{:016x}", g1 + 1));
    soso.escribir(&truncada, b"0123456789abcdef no es un hash y falta el resto", 0o644)?;
    soso.sincronizar(&truncada)?;
    println!("fase1: dejada la generación {} truncada", g1 + 1);
    println!("fase1: ok");
    Ok(0)
}

fn durable_fase2() -> Resultado<usize> {
    let soso = Soso;
    let mut fallos = 0usize;
    match durable::leer_vigente(&soso, DUR_DIR, DUR_REF)? {
        Some((gen, datos)) if datos == b"version-dos" && gen == 1 => {
            println!("fase2: vigente g{gen} = version-dos (la truncada se ignoró): ok")
        }
        otro => {
            fallos += 1;
            println!("fase2: esperaba g1 = version-dos, hay {otro:?}");
        }
    }
    // Y la truncada sigue en disco: no se ha «arreglado» sola ni se ha perdido
    // la anterior por su culpa.
    if Soso.existe(&unir(DUR_DIR, &format!("{DUR_REF}.{:016x}", 2))) {
        println!("fase2: la generación truncada sigue en disco, como debe");
    } else {
        fallos += 1;
        println!("fase2: la generación truncada desapareció");
    }

    // T63: y ahora lo que de verdad importa después de un apagón — **seguir
    // escribiendo**. Antes de esa ficha esto fallaba con «ya existe» y seguía
    // fallando en cada arranque: la referencia quedaba encallada en la última
    // versión buena y no había forma de avanzar sin borrar el resto a mano.
    // Aquí el corte lo dejó un arranque anterior de verdad, no un mock.
    let mut soso = Soso;
    let h2 = soso_improve_core::sha256_hex(b"version-dos");
    match durable::publicar(&mut soso, DUR_DIR, DUR_REF, b"version-tres", Some(&h2)) {
        Ok(g) if g > 2 => println!("fase2: publicada g{g} sobre el resto del corte: ok"),
        otro => {
            fallos += 1;
            println!("fase2: no se pudo publicar tras el corte: {otro:?}");
        }
    }
    match durable::leer_vigente(&soso, DUR_DIR, DUR_REF)? {
        Some((_, datos)) if datos == b"version-tres" => {
            println!("fase2: la vigente es version-tres: ok")
        }
        otro => {
            fallos += 1;
            println!("fase2: la vigente tras publicar es {otro:?}");
        }
    }

    println!("durable: {fallos} caso(s) mal");
    Ok(fallos)
}

/// Dónde vive la sonda de T63. Aparte de la de T46: aquí se ensucia el
/// directorio a propósito y no queremos estropear la evidencia del reinicio.
const DUR_CORTE_DIR: &str = "/var/self-improvement/t63";

/// T63 — publicar **sobre el resto de un corte**, en sosofs y sin limpiar nada.
///
/// Esta sonda no necesita reiniciar: lo que acredita no es que los datos
/// sobrevivan al apagón —eso es T46 y ya está—, sino que encontrarse un
/// fichero a medias no deja la referencia encallada. El resto del corte se
/// escribe aquí a propósito, igual que hace `durable_fase1`.
fn durable_corte() -> Resultado<usize> {
    let mut soso = Soso;
    let mut fallos = 0usize;
    soso.crear_directorio(DUR_CORTE_DIR)?;

    // Un directorio limpio: si quedó de una vuelta anterior, los números no
    // empiezan donde esta sonda cree y el resultado no diría nada.
    if let Ok(entradas) = soso.listar(DUR_CORTE_DIR) {
        for e in entradas {
            let _ = soso.borrar(&unir(DUR_CORTE_DIR, &e.ruta));
        }
    }

    durable::publicar(&mut soso, DUR_CORTE_DIR, DUR_REF, b"uno", None)?;
    let h1 = soso_improve_core::sha256_hex(b"uno");

    // El resto de un corte: cabecera a medias, sin datos que casen.
    let rota = unir(DUR_CORTE_DIR, &format!("{DUR_REF}.{:016x}", 1u64));
    soso.escribir(&rota, b"0123456789abcdef y se fue la luz", 0o644)?;
    soso.sincronizar(&rota)?;

    match durable::publicar(&mut soso, DUR_CORTE_DIR, DUR_REF, b"dos", Some(&h1)) {
        Ok(2) => println!("corte: publicada g2 por encima del resto: ok"),
        otro => {
            fallos += 1;
            println!("corte: esperaba g2, salió {otro:?}");
        }
    }
    match durable::leer_vigente(&soso, DUR_CORTE_DIR, DUR_REF)? {
        Some((2, datos)) if datos == b"dos" => println!("corte: la vigente es g2 = dos: ok"),
        otro => {
            fallos += 1;
            println!("corte: la vigente es {otro:?}");
        }
    }
    // Y la limpieza se lleva el resto, para que el directorio no crezca un
    // fichero por apagón.
    match durable::purgar(&mut soso, DUR_CORTE_DIR, DUR_REF, 2) {
        Ok(2) if !soso.existe(&rota) => println!("corte: purgado el resto del corte: ok"),
        otro => {
            fallos += 1;
            println!("corte: purgar dejó {otro:?} y rota={}", soso.existe(&rota));
        }
    }

    println!("corte: {fallos} caso(s) mal");
    Ok(fallos)
}

// --- T24: copia de tarea y exportación de su parche ------------------------

use soso_improve_core::workspace;

const WS_DIR: &str = "/tmp/t24";

/// El ciclo entero **dentro de soso**: capturar, copiar, dejar que alguien
/// cambie cosas, exportar el parche y aplicarlo al árbol original.
///
/// Sin Git en ninguna parte, que es el motivo de que todo esto exista. Lo que
/// se vigila especialmente es que el `TASK.md` que siembra el coordinador **no**
/// acabe aplicándose al repositorio: sería un archivo nuestro colado en el
/// trabajo del candidato.
fn workspace_prueba() -> Resultado<usize> {
    let mut soso = Soso;
    let repo = unir(WS_DIR, "repo");
    let cap = unir(WS_DIR, "cap");
    let copia_dir = unir(WS_DIR, "copia");
    let mut fallos = 0usize;

    // Un directorio limpio: restos de una vuelta anterior harían que «destino
    // ocupado» saltara donde no toca.
    for d in [&repo, &cap, &copia_dir] {
        if let Ok(entradas) = soso.listar(d) {
            for e in entradas {
                let _ = soso.borrar(&unir(d, &e.ruta));
            }
        }
    }
    soso.crear_directorio(WS_DIR)?;
    soso.crear_directorio(&repo)?;

    soso.escribir(&unir(&repo, "lib.rs"), b"pub fn suma(a: u32) -> u32 { a - 1 }", 0o644)?;
    soso.escribir(&unir(&repo, "sobra.txt"), b"esto se borra\n", 0o644)?;

    let manifiesto = base::capturar(
        &Soso,
        &repo,
        &mut soso,
        &cap,
        &base::Politica::default(),
        base::Git::default(),
        alloc::collections::BTreeMap::new(),
        Vec::new(),
        Vec::new(),
        "guest",
    )?;
    let huella_base = base::huella(&manifiesto.inventario);

    let spec = state::TaskSpec {
        schema_version: soso_improve_core::ESQUEMA,
        id: String::from("T99-guest"),
        base: huella_base.clone(),
        problema: String::from("suma resta"),
        rutas_editables: alloc::vec![String::from("lib.rs")],
        comprobaciones: alloc::vec![state::Comprobacion {
            argv: alloc::vec![String::from("cargo"), String::from("test")],
            cwd: String::from("."),
            timeout_s: 600,
        }],
        limites: state::Limites::default(),
        hashes_entrada: alloc::collections::BTreeMap::new(),
    };

    let copia = workspace::preparar(
        &Soso,
        &cap,
        &mut soso,
        &copia_dir,
        &manifiesto,
        &spec,
        "r-guest",
    )?;
    if copia.archivos == manifiesto.inventario.len() && soso.existe(&unir(&copia_dir, "TASK.md")) {
        println!("workspace: copia con {} archivo(s) y enunciado: ok", copia.archivos);
    } else {
        fallos += 1;
        println!("workspace: la copia salió {copia:?}");
    }

    // Un destino ocupado por otra ejecución no se pisa.
    match workspace::preparar(&Soso, &cap, &mut soso, &copia_dir, &manifiesto, &spec, "r-otro") {
        Err(e) if format!("{e}").contains("r-guest") => {
            println!("workspace: destino de otra ejecución rechazado: ok")
        }
        otro => {
            fallos += 1;
            println!("workspace: el destino ajeno no se rechazó: {otro:?}");
        }
    }

    // El «candidato» trabaja.
    soso.escribir(&unir(&copia_dir, "lib.rs"), b"pub fn suma(a: u32) -> u32 { a + 1 }", 0o644)?;
    soso.escribir(&unir(&copia_dir, "nuevo.rs"), b"// nuevo\n", 0o644)?;
    soso.borrar(&unir(&copia_dir, "sobra.txt"))?;

    let exp = workspace::exportar(&Soso, &copia, &manifiesto, &base::Politica::default())?;
    let rutas: Vec<&str> = exp.paquete.operaciones.iter().map(|o| o.ruta()).collect();
    if rutas == alloc::vec!["lib.rs", "nuevo.rs", "sobra.txt"] {
        println!("workspace: 3 cambios y ni TASK.md ni la marca: ok");
    } else {
        fallos += 1;
        println!("workspace: el paquete lleva {rutas:?}");
    }

    // Los contenidos nuevos al almacén, y el parche al árbol original.
    for op in &exp.paquete.operaciones {
        if let Some(h) = op.objeto() {
            let datos = soso.leer(&unir(&copia_dir, op.ruta()))?;
            soso.escribir(&unir(&cap, &ruta_objeto(h)), &datos, 0o644)?;
        }
    }
    soso_improve_core::delta::aplicar(
        &Soso,
        &cap,
        &mut soso,
        &repo,
        &manifiesto.inventario,
        &exp.paquete,
    )?;
    let aplicado = soso.leer(&unir(&repo, "lib.rs"))?;
    if aplicado == b"pub fn suma(a: u32) -> u32 { a + 1 }"
        && !soso.existe(&unir(&repo, "sobra.txt"))
        && soso.existe(&unir(&repo, "nuevo.rs"))
    {
        println!("workspace: el parche se aplicó al árbol original: ok");
    } else {
        fallos += 1;
        println!("workspace: el árbol quedó mal tras aplicar");
    }
    if soso.existe(&unir(&repo, "TASK.md")) {
        fallos += 1;
        println!("workspace: el TASK.md del coordinador se coló en el repositorio");
    } else {
        println!("workspace: el enunciado no se coló en el repositorio: ok");
    }

    println!("workspace: {fallos} caso(s) mal");
    Ok(fallos)
}

// --- T62: un programa corriente ve el argumento entero -------------------

const T62_DIR: &str = "/tmp/t62";

/// Una ruta con un espacio, a través de `main` de un programa **de verdad**.
///
/// Esto es distinto de la sonda de argv de T47: aquélla lanza `argv-eco`, que
/// lee `libsoso::argv()` directamente, así que pasaba ya cuando `main` recibía
/// la cadena juntada. Aquí se usa `cat`, que hasta T62 partía lo que le
/// llegaba y contestaba tres «no existe» para una sola ruta.
fn argv_espacios() -> Resultado<usize> {
    let mut soso = Soso;
    let mut fallos = 0usize;
    soso.crear_directorio(T62_DIR)?;
    let ruta = unir(T62_DIR, "con espacio.txt");
    let contenido = b"una linea\n";
    soso.escribir(&ruta, contenido, 0o644)?;

    let r = soso.ejecutar(&Orden::nueva(&["/bin/cat", &ruta], ""))?;
    if r.stdout == contenido {
        println!("argv: cat abrió «con espacio.txt» entero: ok");
    } else {
        fallos += 1;
        println!(
            "argv: cat devolvió {:?} (código {:?})",
            String::from_utf8_lossy(&r.stdout),
            r.codigo
        );
    }

    // Y un argumento vacío sigue siendo un argumento: `split_whitespace` lo
    // hacía desaparecer, así que el programa veía uno menos.
    let r = soso.ejecutar(&Orden::nueva(&["/bin/soso-improve", "argv-eco", "", "final"], ""))?;
    let texto = String::from_utf8_lossy(&r.stdout).into_owned();
    let args: Vec<&str> = texto.lines().filter(|l| l.starts_with("arg: ")).collect();
    if args.len() == 4 && args[2] == "arg: " && args[3] == "arg: final" {
        println!("argv: el argumento vacío no se pierde: ok");
    } else {
        fallos += 1;
        println!("argv: llegaron {args:?}");
    }

    println!("argv: {fallos} caso(s) mal");
    Ok(fallos)
}

// ---------------------------------------------------------------------------
// Runner de pruebas dentro de soso (T49)
// ---------------------------------------------------------------------------

use soso_improve_core::pruebas::{Estado, Informe as InformePruebas, Prueba};

/// Ejecuta una sonda midiendo su duración y traduce su resultado.
///
/// Las sondas devuelven **cuántos casos fueron mal**, no un booleano: así el
/// informe puede decir «3 de 5» en vez de «falló».
fn correr(id: &str, target: &str, f: impl FnOnce() -> Resultado<usize>) -> Prueba {
    let reloj = RelojSoso;
    let t0 = reloj.ahora_ms();
    let estado = match f() {
        Ok(0) => Estado::Paso,
        Ok(n) => Estado::Fallo(format!("{n} caso(s) mal")),
        Err(e) => Estado::Error(format!("{e}")),
    };
    Prueba::nueva(id, target, estado, reloj.ahora_ms().saturating_sub(t0))
}

/// Capacidad que todavía no existe. **Cuenta en el total** y no como acierto.
fn pendiente(id: &str, target: &str, motivo: &str) -> Prueba {
    Prueba::nueva(id, target, Estado::Pendiente(String::from(motivo)), 0)
}

/// `soso-improve pruebas`: ejecuta dentro de soso las comprobaciones que
/// acreditan el circuito, sin libtest.
///
/// No duplica las sondas: las reúne. Cada una sigue existiendo por su cuenta
/// para poder lanzarla suelta al depurar, que es lo que se ha hecho todo el
/// tiempo en T46–T48.
fn pruebas(banco: Option<&str>) -> Resultado<usize> {
    let mut lista: Vec<Prueba> = Vec::new();

    lista.push(correr("procesos/argv-stdin-cwd", "T47", procesos));
    lista.push(correr("transporte/eco", "T48", || eco(9460)));
    lista.push(correr("tuberias/dos-canales", "T61", tuberias));
    lista.push(correr("delta/aplicar-sin-git", "T50", delta_prueba));
    lista.push(correr("durable/publicar-sobre-un-corte", "T63", durable_corte));
    lista.push(correr("workspace/copia-y-parche", "T24", workspace_prueba));
    lista.push(correr("argv/ruta-con-espacios", "T62", argv_espacios));

    // Durabilidad: aquí sólo se puede **leer** lo que dejó un arranque
    // anterior. Acreditarla exige reiniciar la máquina, y eso no lo puede
    // hacer un proceso: se dice, no se finge.
    if Soso.existe(&unir(DUR_DIR, &format!("{DUR_REF}.{:016x}", 0u64))) {
        lista.push(correr("durable/tras-reinicio", "T46", durable_fase2));
    } else {
        lista.push(pendiente(
            "durable/tras-reinicio",
            "T46",
            "exige dos arranques: ejecutar durable-fase1, reiniciar y repetir",
        ));
    }

    // El banco **visible** viaja en la imagen y se puede cargar aquí. El
    // **reservado** no: C5 dice que los criterios viven fuera del alcance del
    // agente, y meterlos en la misma imagen en la que correrá sería poner la
    // trampa nosotros mismos. Por eso se lista, que ejercita cargar y validar
    // el formato, y la validación completa queda pendiente con ese motivo.
    let dir = banco.unwrap_or(BANCO_GUEST);
    if Soso.existe(&unir(dir, "banco.json")) {
        let d = String::from(dir);
        lista.push(correr("banco/listar", "T02", move || banco_listar(&d)));
    } else {
        lista.push(pendiente(
            "banco/listar",
            "T02",
            "no hay banco en la imagen",
        ));
    }
    lista.push(pendiente(
        "banco/validar",
        "T02",
        "exige la partición reservada, que no puede vivir en la imagen del agente (C5)",
    ));

    // Lo que necesita compilador o cargo no se cuenta como aprobado ni se
    // excluye del total: se informa con su ficha (paso 4 de T49).
    lista.push(pendiente(
        "verificar/programa",
        "T40",
        "necesita un compilador dentro de soso: T40",
    ));
    lista.push(pendiente(
        "verificar/repo",
        "T41",
        "necesita cargo dentro de soso: T41",
    ));

    let informe = InformePruebas::nuevo("guest", lista);
    for linea in informe.texto().lines() {
        println!("{linea}");
    }
    println!("json: {}", informe.json());
    Ok(informe.resumen.fallaron + informe.resumen.errores)
}

/// Dónde viaja el banco visible dentro de la imagen.
const BANCO_GUEST: &str = "/var/self-improvement/banco";

/// Cargar y listar el banco: ejercita leer los casos y validar su formato.
fn banco_listar(dir: &str) -> Resultado<usize> {
    let casos = caso::cargar(&Soso, dir)?;
    if casos.is_empty() {
        return Err(Error::uso(format!("{dir} no trae casos")));
    }
    println!("banco: {} casos cargados de {dir}", casos.len());
    Ok(0)
}

// ---------------------------------------------------------------------------
// Aplicar un paquete de cambios dentro de soso, sin Git (T50)
// ---------------------------------------------------------------------------

use soso_improve_core::captura::{huella as huella_inv, ruta_objeto, ArchivoBase};
use soso_improve_core::delta;

const DELTA_DIR: &str = "/tmp/t50";

/// Siembra el almacén por contenido y devuelve el inventario.
fn delta_sembrar(alm: &str, ficheros: &[(&str, &[u8])]) -> Resultado<Vec<ArchivoBase>> {
    let mut soso = Soso;
    let mut inv = Vec::new();
    for (ruta, datos) in ficheros {
        let h = soso_improve_core::sha256_hex(datos);
        soso.escribir(&unir(alm, &ruta_objeto(&h)), datos, 0o644)?;
        inv.push(ArchivoBase {
            ruta: String::from(*ruta),
            sha256: h,
            bytes: datos.len() as u64,
            modo: 0o644,
        });
    }
    inv.sort_by(|a, b| a.ruta.cmp(&b.ruta));
    Ok(inv)
}

/// Aplica el fixture compartido dentro de soso y compara la huella con la que
/// el host tiene fijada.
///
/// Es la comprobación que pide la ficha: **aplicar en soso sin Git instalado y
/// comparar hashes con el host**. Si las dos huellas no coinciden, el formato
/// no significa lo mismo en los dos sitios.
fn delta_prueba() -> Resultado<usize> {
    let mut soso = Soso;
    let alm = unir(DELTA_DIR, "alm");
    let arbol = unir(DELTA_DIR, "arbol");
    let mut fallos = 0usize;

    let base = delta_sembrar(&alm, delta::fixture::BASE)?;
    let destino = delta_sembrar(&alm, delta::fixture::DESTINO)?;
    for (ruta, datos) in delta::fixture::BASE {
        soso.escribir(&unir(&arbol, ruta), datos, 0o644)?;
    }

    let paquete = delta::exportar(&base, &destino);
    let inv = delta::aplicar(&Soso, &alm, &mut soso, &arbol, &base, &paquete)?;
    let h = huella_inv(&inv);
    if h == delta::HUELLA_FIXTURE {
        println!("delta: huella {h} igual que en el host: ok");
    } else {
        fallos += 1;
        println!("delta: huella {h}, el host fija {}", delta::HUELLA_FIXTURE);
    }

    // Y el árbol es el que dice el inventario, no sólo el inventario.
    let esperado = b"fn main() { println!(); }";
    match Soso.leer(&unir(&arbol, "src/main.rs")) {
        Ok(v) if v == esperado => println!("delta: contenido modificado: ok"),
        otro => {
            fallos += 1;
            println!("delta: src/main.rs quedó {otro:?}");
        }
    }
    if Soso.existe(&unir(&arbol, "nuevo/hondo.txt")) && !Soso.existe(&unir(&arbol, "sobra.txt")) {
        println!("delta: alta y borrado aplicados: ok");
    } else {
        fallos += 1;
        println!("delta: el alta o el borrado no se aplicaron");
    }

    // Base equivocada: se rechaza **antes** de tocar el árbol.
    let mut malo = paquete.clone();
    malo.base = String::from("0").repeat(64);
    match delta::validar(&Soso, &alm, &base, &malo) {
        Err(e) if format!("{e}").contains("base distinta") => {
            println!("delta: base incorrecta rechazada: ok")
        }
        otro => {
            fallos += 1;
            println!("delta: la base incorrecta no se rechazó: {otro:?}");
        }
    }

    println!("delta: {fallos} caso(s) mal");
    Ok(fallos)
}

// --- T23: tareas y estados del coordinador ---------------------------------

use libsoso::print;
use soso_improve_core::state::{self, Ids};

const TAREA_DIR: &str = "/tmp/t23";

/// El enunciado de la autoprueba. Se escribe desde aquí porque sosh no tiene
/// heredoc y el arnés no puede dejar un JSON en el guest sin inventar un canal.
const SPEC_EJEMPLO: &str = concat!(
    "{\"schema_version\":1,\"id\":\"T99-autoprueba\",",
    "\"base\":\"",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "\",\"problema\":\"el formato de estado tiene que sobrevivir a un corte\",",
    "\"rutas_editables\":[\"kernel/src/net/mod.rs\"],",
    "\"comprobaciones\":[{\"argv\":[\"cargo\",\"test\"],\"cwd\":\".\",\"timeout_s\":600}],",
    "\"limites\":{\"intentos_max\":3,\"herramientas_max\":30},",
    "\"hashes_entrada\":{}}"
);

fn tarea_preparar(estado: &str, spec: &str) -> Resultado<usize> {
    let mut soso = Soso;
    soso.crear_directorio(estado)?;
    let json = soso.leer(spec)?;
    let mut ids = state::IdsDelReloj {
        reloj: &RelojSoso,
        prefijo: "run",
    };
    let run_id = ids.nuevo_run_id();
    let m = state::preparar(&mut soso, estado, &json, &run_id, RelojSoso.ahora_ms())?;
    println!("tarea: preparada {} para {}", m.run_id, m.task_id);
    Ok(0)
}

fn tarea_ver(estado: &str, run: &str, reanudar: bool) -> Resultado<usize> {
    let r = state::reanudar(&Soso, estado, run)?;
    print!("{}", state::informe(&r));
    if reanudar {
        // Decir por dónde va no es continuar. Que la orden exista no puede
        // dar a entender que el trabajo siguió solo.
        println!("tarea: continuar es T25 (ejecutor) y T26 (validador); aquí no se ejecuta nada");
    }
    Ok(0)
}

/// Autoprueba del formato **dentro de soso**: preparar, avanzar, cerrar un
/// intento con una medida desconocida y volver a leerlo todo desde el disco.
fn tarea_prueba() -> Resultado<usize> {
    let mut soso = Soso;
    let dir = unir(TAREA_DIR, "estado");
    soso.crear_directorio(TAREA_DIR)?;
    soso.crear_directorio(&dir)?;
    let mut fallos = 0usize;

    let run_id = "r-autoprueba";
    let m = state::preparar(&mut soso, &dir, SPEC_EJEMPLO.as_bytes(), run_id, 1_000)?;
    println!("tarea: preparada {} para {}", m.run_id, m.task_id);

    // El estado tiene que salir del disco, no de la variable que quedó en
    // memoria: lo que se acredita es la persistencia, no la struct.
    let (mut leido, hash) = state::cargar(&soso, &dir, run_id)?
        .ok_or_else(|| Error::entorno("la ejecución recién preparada no está en el disco"))?;
    leido.transicion(state::Estado::Reproduciendo, state::Autoridad::Coordinador)?;
    leido.transicion(state::Estado::Editando, state::Autoridad::Coordinador)?;
    leido.abrir_intento(&state::Limites::default(), 1_100)?;
    leido.anotar_artefacto("tasks/T99/intento-0/salida.log")?;
    leido.cerrar_intento(
        state::Estado::Verificando,
        state::Medida::Valor(7),
        state::Medida::desconocida("el endpoint no devolvió usage"),
        1_900,
    )?;
    leido.transicion(state::Estado::Verificando, state::Autoridad::Coordinador)?;
    state::guardar(&mut soso, &dir, &leido, Some(&hash))?;

    // Sólo el validador acepta, también aquí.
    let (mut otra, _) = state::cargar(&soso, &dir, run_id)?.unwrap();
    match otra.transicion(state::Estado::Aceptada, state::Autoridad::Coordinador) {
        Err(e) if format!("{e}").contains("validador") => {
            println!("tarea: el coordinador no puede aceptar: ok")
        }
        otro => {
            fallos += 1;
            println!("tarea: el coordinador pudo aceptar: {otro:?}");
        }
    }

    let r = state::reanudar(&soso, &dir, run_id)?;
    print!("{}", state::informe(&r));
    if r.manifiesto.intentos.len() == 1 && r.intento_abierto.is_none() {
        println!("tarea: el intento cerrado sobrevivió al disco: ok");
    } else {
        fallos += 1;
        println!("tarea: el intento no se recuperó: {:?}", r.manifiesto.intentos);
    }
    match r.manifiesto.intentos[0].tokens {
        state::Medida::Desconocida { .. } => {
            println!("tarea: lo no medido sigue sin medirse tras el viaje: ok")
        }
        state::Medida::Valor(v) => {
            fallos += 1;
            println!("tarea: una medida desconocida volvió como {v}");
        }
    }

    println!("tarea: {fallos} caso(s) mal");
    Ok(fallos)
}
