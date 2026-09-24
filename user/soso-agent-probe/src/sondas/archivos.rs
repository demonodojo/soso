//! Sonda 1 — archivos persistentes (paso 1 de T33).
//!
//! Es la primera por una razón: las herramientas `read`, `write`, `edit` y
//! `apply-patch` del agente no hacen otra cosa, y SQLite —que T32 sitúa dentro
//! de Bun— se apoya en lo mismo. Si esto no es sólido, lo demás no importa.
//!
//! **Lo que se mide no es el código de retorno, es el efecto.** Un `write` que
//! devuelve el número de bytes y no los guarda pasaría cualquier comprobación
//! de errno; por eso todo se cierra, se reabre y se compara byte a byte. Y hay
//! dos fases con un reinicio en medio, porque «sobrevive al cierre del fichero»
//! y «sobrevive al apagado» no son lo mismo y confundirlos es exactamente cómo
//! se da por buena una caché.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use libsoso::{abi, sys};

use crate::{errno, hex, limpiar_dir, unir, Caso};

/// Raíz de los artefactos del guest, según NATIVO.md.
const DIR: &str = "/var/self-improvement/probe/archivos";

/// Texto con acentos, eñe, un signo de apertura y un emoji: cuatro longitudes
/// distintas en UTF-8 (1, 2, 3 y 4 bytes). Si algo trunca por «carácter» en vez
/// de por byte, se ve aquí y no en producción.
const TEXTO: &str = "café ☕ ñandú ¡hola! 𝒮";

/// Binario con los bytes que más se estropean: nulo, 0x80 (que sosh comía),
/// 0x0a y 0x0d (finales de línea), 0xff y la secuencia UTF-8 inválida 0xc3 0x28.
const BINARIO: &[u8] = &[
    0x00, 0x80, 0x0a, 0x0d, 0xff, 0xc3, 0x28, 0x1a, 0x7f, 0x81, 0x00, 0x00, 0xfe, 0x55, 0xaa,
];

/// Nombre con espacio y acento: hasta T62/T64 esto no se podía ni pasar por
/// argumento, así que conviene que la sonda lo ejercite.
const NOMBRE_RARO: &str = "un año raro.txt";

fn escribir(ruta: &str, datos: &[u8]) -> Result<(), String> {
    let fd = sys::open(ruta, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
    if fd < 0 {
        return Err(format!("open({ruta}) = {}", errno(fd)));
    }
    let fd = fd as u64;
    let mut escrito = 0usize;
    while escrito < datos.len() {
        let n = sys::write(fd, &datos[escrito..]);
        if n <= 0 {
            sys::close(fd);
            return Err(format!("write en {escrito} = {}", errno(n)));
        }
        escrito += n as usize;
    }
    // `fsync` antes de cerrar: sin esto, «está en disco» es una suposición.
    let rc = sys::fsync(fd);
    sys::close(fd);
    if rc < 0 {
        return Err(format!("fsync = {}", errno(rc)));
    }
    Ok(())
}

fn leer(ruta: &str) -> Result<Vec<u8>, String> {
    let fd = sys::open(ruta, abi::O_RDONLY);
    if fd < 0 {
        return Err(format!("open({ruta}) = {}", errno(fd)));
    }
    let fd = fd as u64;
    let mut out = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = sys::read(fd, &mut buf);
        if n < 0 {
            sys::close(fd);
            return Err(format!("read = {}", errno(n)));
        }
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    sys::close(fd);
    Ok(out)
}

/// Compara lo leído con lo escrito y devuelve el caso ya juzgado.
fn comparar(id: &str, ruta: &str, esperado: &[u8]) -> Caso {
    match leer(ruta) {
        Err(e) => Caso::nuevo(id, hex(esperado), format!("error: {e}")),
        Ok(v) => {
            // Se compara **byte a byte**, no por longitud: un fichero del
            // tamaño correcto con el contenido cambiado es el fallo que una
            // comprobación de `len()` deja pasar.
            if v == esperado {
                Caso::nuevo(id, hex(esperado), hex(esperado))
            } else {
                Caso::nuevo(
                    id,
                    format!("{} bytes {}", esperado.len(), hex(esperado)),
                    format!("{} bytes {}", v.len(), hex(&v)),
                )
            }
        }
    }
}

/// Escribe los tres ficheros. La usa tanto la sonda de un arranque como la
/// fase 1 de la de dos, para que las dos midan exactamente lo mismo.
fn sembrar() -> Vec<Caso> {
    let mut casos = Vec::new();
    limpiar_dir("/var/self-improvement");
    sys::mkdir("/var/self-improvement/probe");
    limpiar_dir(DIR);

    for (id, nombre, datos) in [
        ("archivos/texto-utf8", "texto.txt", TEXTO.as_bytes()),
        ("archivos/binario", "datos.bin", BINARIO),
        ("archivos/nombre-con-espacio", NOMBRE_RARO, TEXTO.as_bytes()),
    ] {
        let ruta = unir(DIR, nombre);
        match escribir(&ruta, datos) {
            Ok(()) => casos.push(Caso::nuevo(
                &format!("{id}/escribir"),
                "escrito",
                "escrito",
            )),
            Err(e) => casos.push(Caso::nuevo(&format!("{id}/escribir"), "escrito", e)),
        }
    }
    casos
}

fn releer() -> Vec<Caso> {
    vec![
        comparar(
            "archivos/texto-utf8",
            &unir(DIR, "texto.txt"),
            TEXTO.as_bytes(),
        ),
        comparar("archivos/binario", &unir(DIR, "datos.bin"), BINARIO),
        comparar(
            "archivos/nombre-con-espacio",
            &unir(DIR, NOMBRE_RARO),
            TEXTO.as_bytes(),
        ),
    ]
}

/// El tamaño que declara `stat` tiene que cuadrar con lo que se escribió.
///
/// Va aparte porque es el chivato de un stub: un sistema de archivos que
/// guarda los bytes pero no actualiza el inodo lee bien mientras el fichero
/// esté en caché y miente en cuanto se reinicia.
fn metadatos() -> Vec<Caso> {
    let mut casos = Vec::new();
    for (id, nombre, largo) in [
        ("archivos/stat-texto", "texto.txt", TEXTO.len()),
        ("archivos/stat-binario", "datos.bin", BINARIO.len()),
    ] {
        let ruta = unir(DIR, nombre);
        let mut st = abi::Stat::default();
        let rc = sys::stat(&ruta, &mut st);
        let observado = if rc < 0 {
            format!("stat = {}", errno(rc))
        } else {
            format!("{} bytes", st.size)
        };
        casos.push(Caso::nuevo(id, format!("{largo} bytes"), observado));
    }
    casos
}

/// Todo en un arranque: crear, escribir, cerrar, reabrir y comparar.
///
/// Acredita que el contenido sobrevive a **cerrar el descriptor**. No acredita
/// que sobreviva al apagado: eso es [`fase1`] + [`fase2`].
pub fn completa() -> Vec<Caso> {
    let mut casos = sembrar();
    casos.extend(releer());
    casos.extend(metadatos());

    // Reescribir encima con menos bytes: `O_TRUNC` tiene que dejar el fichero
    // corto de verdad, no un contenido viejo con el principio pisado.
    let ruta = unir(DIR, "texto.txt");
    let corto = b"ab";
    match escribir(&ruta, corto) {
        Ok(()) => casos.push(comparar("archivos/truncar", &ruta, corto)),
        Err(e) => casos.push(Caso::nuevo("archivos/truncar", "escrito", e)),
    }

    // Y borrar: `unlink` tiene que hacer desaparecer el fichero, no vaciarlo.
    let rc = sys::unlink(&ruta);
    let existe = sys::open(&ruta, abi::O_RDONLY);
    if existe >= 0 {
        sys::close(existe as u64);
    }
    casos.push(Caso::nuevo(
        "archivos/borrar",
        "borrado",
        if rc < 0 {
            format!("unlink = {}", errno(rc))
        } else if existe >= 0 {
            String::from("sigue abriéndose")
        } else {
            String::from("borrado")
        },
    ));
    casos
}

/// Fase 1: escribir y dejarlo en disco. Después hay que **reiniciar**.
pub fn fase1() -> Vec<Caso> {
    let casos = sembrar();
    libsoso::println!("probe: fase1 lista; reiniciar y ejecutar archivos-fase2");
    casos
}

/// Fase 2, tras el reinicio: reabrir y comparar.
///
/// Si esto pasa y `completa` también, la diferencia entre las dos dice algo
/// concreto: que lo que sobrevive al cierre del descriptor sobrevive también al
/// apagado. Si `completa` pasa y ésta no, lo que hay es una caché.
pub fn fase2() -> Vec<Caso> {
    let mut casos = releer();
    casos.extend(metadatos());
    casos
}
