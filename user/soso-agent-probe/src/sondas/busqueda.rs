//! Sonda 10 — buscar en el código: la semántica declarada de
//! [N-009](../../../../docs/self-improvement/native/N-009.md).
//!
//! [T32](../../../../docs/self-improvement/T32-opencode-inventario.md) midió
//! que la herramienta `grep` de un agente es **ripgrep**, un binario que
//! OpenCode se descarga en ejecución, y que en soso no hay ni ese binario ni
//! quien lo compile. Lo que hay es un `grep` propio con otra semántica.
//!
//! Lo que se mide aquí no es «¿encuentra cosas?» sino las tres formas en que
//! un buscador puede **mentir**, que es lo que importa cuando quien pregunta
//! es un agente y no una persona que puede sospechar:
//!
//! - Decir «no hay» cuando lo que pasa es que no pudo buscar.
//! - Aceptar una expresión regular, buscarla tal cual y devolver cero líneas
//!   — una respuesta que *parece* un hecho sobre el código.
//! - Saltarse en silencio las líneas que no son UTF-8, escondiendo el resto
//!   de un fichero por un byte suelto.
//!
//! Por eso casi todos los casos miran el **código de salida**, no el texto: es
//! lo que un programa que llama a la herramienta puede interpretar sin
//! adivinar.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{abi, println, sys};

use crate::{errno, unir, Caso};

const DIR: &str = "/var/self-improvement/probe/busqueda";
const GREP: &str = "/bin/grep";

fn anotar(casos: &mut Vec<Caso>, c: Caso) {
    if c.paso {
        println!("probe: {} ok ({})", c.id, c.observado);
    } else {
        println!(
            "probe: {} FALLO esperado={:?} observado={:?}",
            c.id, c.esperado, c.observado
        );
    }
    casos.push(c);
}

fn escribir(ruta: &str, datos: &[u8]) -> Result<(), String> {
    sys::unlink(ruta);
    let fd = sys::open(ruta, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
    if fd < 0 {
        return Err(format!("open {ruta} = {}", errno(fd)));
    }
    let fd = fd as u64;
    let mut n = 0;
    while n < datos.len() {
        let w = sys::write(fd, &datos[n..]);
        if w <= 0 {
            sys::close(fd);
            return Err(format!("write = {}", errno(w)));
        }
        n += w as usize;
    }
    sys::close(fd);
    Ok(())
}

/// Drena una tubería hasta EOF, con plazo (T61).
fn drenar(fd: u64) -> String {
    let mut out = Vec::new();
    let mut buf = [0u8; 256];
    let mut vacios = 0;
    loop {
        let n = sys::read_timeout(fd, &mut buf, 200);
        if n > 0 {
            out.extend_from_slice(&buf[..n as usize]);
            vacios = 0;
        } else if n == 0 {
            break;
        } else {
            vacios += 1;
            if vacios > 25 {
                break;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lanza `grep` con esos argumentos. Devuelve `(código, stdout)`.
///
/// Se drena **antes** de esperar: al revés, un `grep` que llenara la tubería
/// se quedaría bloqueado escribiendo mientras el padre lo espera.
fn correr(args: &[&str]) -> (i64, String) {
    let Ok((r, w)) = sys::pipe() else {
        return (-1, String::from("no hubo tubería"));
    };
    let mut argv = alloc::vec![GREP];
    argv.extend_from_slice(args);
    let pid = sys::spawn_io_full(
        GREP,
        &argv,
        &[],
        [abi::FD_SERIAL_TTY, w, abi::FD_SERIAL_TTY, abi::FD_KERNEL_LOG],
    );
    sys::close(w);
    if pid < 0 {
        sys::close(r);
        return (pid, String::new());
    }
    let texto = drenar(r);
    sys::close(r);
    match sys::wait() {
        Ok((_, c)) => (c as i64, texto),
        Err(e) => (e, texto),
    }
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();
    sys::mkdir("/var/self-improvement");
    sys::mkdir("/var/self-improvement/probe");
    sys::mkdir(DIR);
    sys::mkdir(&unir(DIR, "hondo"));

    let fuente = unir(DIR, "fuente.txt");
    let binario = unir(DIR, "binario.txt");
    let anidado = unir(&unir(DIR, "hondo"), "otro.txt");

    for (ruta, datos) in [
        (&fuente, b"fn alfa() {}\nfn beta() {}\nlet x = 1;\n".to_vec()),
        // Primera línea con un byte que no es UTF-8, y la aguja **detrás**:
        // si el buscador convierte a texto y se salta lo que no vale, esconde
        // el resto del fichero.
        (&binario, {
            let mut v = alloc::vec![0x66, 0x6f, 0x6f, 0xff, 0x0a];
            v.extend_from_slice(b"AGUJA_TRAS_BYTE_MALO\n");
            v
        }),
        (&anidado, b"fn gamma() {}\n".to_vec()),
    ] {
        if let Err(e) = escribir(ruta, &datos) {
            anotar(&mut casos, Caso::nuevo("busqueda/preparar", "ficheros", e));
            return casos;
        }
    }

    // 1. Hay coincidencias → 0. El caso fácil, que existe para que los dos
    //    siguientes signifiquen algo.
    let (c, texto) = correr(&["alfa", &fuente]);
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/hay-coincidencias-sale-0",
            "0 con la línea",
            if c == 0 && texto.contains("fn alfa") { String::from("0 con la línea") } else { format!("código {c}, {texto:?}") },
        ),
    );

    // 2. **No hay ninguna → 1.** Es el caso que separa «no aparece» de
    //    «funcionó»: mientras los dos valían 0, un fallo de búsqueda y un
    //    hecho sobre el código eran indistinguibles para quien llama.
    let (c, _) = correr(&["NO_ESTA_ESTE_SIMBOLO", &fuente]);
    anotar(
        &mut casos,
        Caso::nuevo("busqueda/sin-coincidencias-sale-1", "1", format!("{c}")),
    );

    // 3. **Una expresión regular se rechaza con 2**, no se busca tal cual
    //    devolviendo cero líneas. Un cero disfrazado de hecho es peor que un
    //    error: el error se puede reintentar de otra forma.
    let (c, texto) = correr(&["fn \\w+", &fuente]);
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/una-regex-se-rechaza-en-vez-de-mentir",
            "2 y lo explica",
            if c == 2 && texto.contains("literal") { String::from("2 y lo explica") } else { format!("código {c}, {texto:?}") },
        ),
    );

    // 4. Y con `-F` se busca tal cual, porque a veces es lo que se quiere.
    //    Sin este caso, el anterior pasaría también con un grep que rechazara
    //    cualquier patrón raro sin dar salida.
    let (c, _) = correr(&["-F", "fn \\w+", &fuente]);
    anotar(
        &mut casos,
        Caso::nuevo("busqueda/con-F-se-busca-literal", "1", format!("{c}")),
    );

    // 5. **Un byte que no es UTF-8 no esconde el resto del fichero.** Antes se
    //    comparaba línea a línea convirtiendo a texto, y lo que no convertía
    //    se saltaba en silencio.
    let (c, texto) = correr(&["AGUJA_TRAS_BYTE_MALO", &binario]);
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/un-byte-invalido-no-esconde-el-resto",
            "0 y la encuentra",
            if c == 0 && texto.contains("AGUJA_TRAS_BYTE_MALO") { String::from("0 y la encuentra") } else { format!("código {c}, {texto:?}") },
        ),
    );

    // 6. Números de línea: lo que convierte «está» en «está aquí».
    let (_, texto) = correr(&["-n", "let x", &fuente]);
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/numera-las-lineas",
            "3:let x = 1;",
            String::from(texto.trim()),
        ),
    );

    // 7. Recursivo: buscar en un árbol es lo que hace una herramienta de
    //    agente, y hasta ahora había que darle los ficheros uno a uno.
    let (c, texto) = correr(&["-r", "-l", "gamma", DIR]);
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/recursivo-baja-a-los-subdirectorios",
            format!("0 y nombra {anidado}"),
            if c == 0 && texto.contains("hondo/otro.txt") { format!("0 y nombra {anidado}") } else { format!("código {c}, {texto:?}") },
        ),
    );

    casos
}
