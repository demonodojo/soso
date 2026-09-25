//! Sondas de ABI para las capacidades que necesita un agente dentro de soso.
//!
//! [T32](../../../docs/self-improvement/T32-opencode-inventario.md) dejó una
//! lista de incógnitas; esta herramienta las convierte en **medidas**. Un
//! módulo por capacidad, y cada pase de
//! [T33](../../../docs/self-improvement/T33-sondas-abi.md) añade uno y ejecuta
//! sólo ese caso.
//!
//! Lo que distingue a una sonda de una prueba cualquiera es que tiene que
//! **detectar el stub**: una llamada que devuelve éxito sin hacer nada pasaría
//! una comprobación de código de retorno y fallaría en producción. Por eso
//! aquí no se mira el código de salida de la syscall, se mira el **efecto**:
//! se escribe, se cierra, se vuelve a abrir y se compara byte a byte.
//!
//! Cada sonda informa `esperado` y `observado`, nunca sólo «ok».

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::println;

mod sondas;

libsoso::entry!(main);

const USO: &str = "\
uso: soso-agent-probe <sonda> [fase]

  archivos              crear, escribir, cerrar, reabrir y comparar (un arranque)
  archivos-fase1        escribir y dejarlo en disco  (antes de reiniciar)
  archivos-fase2        reabrir y comparar           (después de reiniciar)
  ejecutable            escribir código máquina en una página y llamarlo
  compartir             varios descriptores sobre el mismo fichero (lo de SQLite)
  senales               matar un hijo, distinguir la muerte, tubería sin lector
  salidas               stdout/stderr separados, código de salida, entorno, cwd
  hilos                 hilos, join, guarda de pila y temporizadores
  canales               tubería con salida grande y EOF, TCP con reconexión
  coste                 coste de escritura en sosofs (paso 1 de N-001)

  codigos: 0 todo bien, 1 alguna capacidad falló, 2 uso incorrecto";

fn main(args: &[String]) -> u8 {
    let sonda = args.first().map(|s| s.as_str()).unwrap_or("");
    let informe = match sonda {
        "archivos" => sondas::archivos::completa(),
        "archivos-fase1" => sondas::archivos::fase1(),
        "archivos-fase2" => sondas::archivos::fase2(),
        "ejecutable" => sondas::ejecutable::ejecutar(),
        "compartir" => sondas::compartir::ejecutar(),
        "senales" => sondas::senales::ejecutar(),
        "salidas" => sondas::salidas::ejecutar(),
        "hilos" => sondas::hilos::ejecutar(),
        "canales" => sondas::canales::ejecutar(),
        "coste" => sondas::coste::ejecutar(),
        "carga" => sondas::carga::ejecutar(),
        "busqueda" => sondas::busqueda::ejecutar(),
        "vigilancia" => sondas::vigilancia::ejecutar(),
        "rutas" => sondas::rutas::ejecutar(),
        // Auxiliares: la sonda de señales se lanza a sí misma para tener
        // hijos de verdad a los que matar.
        "dormir" => return sondas::senales::dormir(args.get(1).map(|s| s.as_str()).unwrap_or("0")),
        "salir" => return sondas::senales::salir(args.get(1).map(|s| s.as_str()).unwrap_or("0")),
        "gritar" => return sondas::salidas::gritar(),
        "eco-entorno" => return sondas::salidas::eco_entorno(),
        "eco-cwd" => return sondas::salidas::eco_cwd(),
        "chorro" => return sondas::canales::chorro(),
        "eco-tcp" => return sondas::canales::eco_tcp(),
        "tocar-guarda" => return sondas::hilos::tocar_guarda(),
        "tomar-cerrojo" => {
            return sondas::compartir::tomar_cerrojo(
                args.get(1).map(|s| s.as_str()).unwrap_or(""),
                true,
            )
        }
        "tomar-cerrojo-compartido" => {
            return sondas::compartir::tomar_cerrojo(
                args.get(1).map(|s| s.as_str()).unwrap_or(""),
                false,
            )
        }
        "" | "-h" | "--help" => {
            println!("{USO}");
            return if sonda.is_empty() { 2 } else { 0 };
        }
        otra => {
            println!("soso-agent-probe: sonda desconocida «{otra}»");
            println!("{USO}");
            return 2;
        }
    };
    imprimir(&informe);
    if informe.iter().all(|c| c.paso) { 0 } else { 1 }
}

/// Un caso de una sonda. `esperado` y `observado` van siempre, también cuando
/// pasa: un informe que sólo dice «ok» no permite revisar el criterio después.
pub struct Caso {
    pub id: String,
    pub paso: bool,
    pub esperado: String,
    pub observado: String,
}

impl Caso {
    /// Una medida que **no juzga**: no hay «esperado» porque no hay criterio
    /// todavía — el criterio es lo que la ficha tiene que decidir con este dato
    /// delante. Cuenta como pasada para no ensuciar la señal del paso, y se
    /// distingue en el informe por su `esperado` vacío.
    ///
    /// Es la misma idea que `Medida::Desconocida` en T23: lo que no se puede
    /// juzgar se dice, no se fuerza a caber en un booleano.
    pub fn observacion(id: &str, observado: impl Into<String>) -> Self {
        Caso {
            id: String::from(id),
            paso: true,
            esperado: String::new(),
            observado: observado.into(),
        }
    }

    pub fn nuevo(id: &str, esperado: impl Into<String>, observado: impl Into<String>) -> Self {
        let esperado = esperado.into();
        let observado = observado.into();
        Caso {
            id: String::from(id),
            paso: esperado == observado,
            esperado,
            observado,
        }
    }
}

fn imprimir(casos: &[Caso]) {
    for c in casos {
        if c.paso && c.esperado.is_empty() {
            println!("probe: {} medido: {}", c.id, c.observado);
        } else if c.paso {
            println!("probe: {} ok ({})", c.id, c.observado);
        } else {
            println!(
                "probe: {} FALLO esperado={:?} observado={:?}",
                c.id, c.esperado, c.observado
            );
        }
    }
    let malos = casos.iter().filter(|c| !c.paso).count();
    println!("probe: {} caso(s), {malos} mal", casos.len());
    // JSON en una línea, para que el arnés lo recoja sin parsear el texto.
    let mut j = String::from("probe-json: {\"casos\":[");
    for (i, c) in casos.iter().enumerate() {
        if i > 0 {
            j.push(',');
        }
        j.push_str(&format!(
            "{{\"id\":\"{}\",\"paso\":{},\"esperado\":\"{}\",\"observado\":\"{}\"}}",
            esc(&c.id),
            c.paso,
            esc(&c.esperado),
            esc(&c.observado)
        ));
    }
    j.push_str(&format!("],\"malos\":{malos}}}"));
    println!("{j}");
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Nombre legible de un errno **y** su número.
///
/// El número va siempre: `errno_str` devuelve «error desconocido» para lo que
/// no conoce, y una sonda que informa eso no ha medido nada. Con el número, un
/// código sin nombre sigue siendo un dato.
pub fn errno(rc: i64) -> String {
    if rc >= 0 {
        return format!("{rc}");
    }
    format!("{} ({rc})", libsoso::errno_str(rc))
}

/// Hex de unos bytes, para que un fallo binario se pueda leer en la consola.
pub fn hex(datos: &[u8]) -> String {
    let mut s = String::with_capacity(datos.len() * 2);
    for b in datos.iter().take(64) {
        s.push_str(&format!("{b:02x}"));
    }
    if datos.len() > 64 {
        s.push_str("…");
    }
    s
}

/// Une dos trozos de ruta.
pub fn unir(base: &str, resto: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), resto.trim_start_matches('/'))
}

/// Deja un directorio vacío, creándolo si no está.
pub fn limpiar_dir(dir: &str) {
    use libsoso::{abi, sys};
    sys::mkdir(dir);
    let fd = sys::open(dir, abi::O_RDONLY);
    if fd < 0 {
        return;
    }
    let mut nombres: Vec<String> = Vec::new();
    let mut ents = [abi::Dirent::default(); 16];
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n <= 0 {
            break;
        }
        for d in &ents[..n as usize / abi::DIRENT_SIZE] {
            if let Ok(nombre) = core::str::from_utf8(d.name_bytes()) {
                nombres.push(String::from(nombre));
            }
        }
    }
    sys::close(fd as u64);
    for n in nombres {
        sys::unlink(&unir(dir, &n));
    }
}
