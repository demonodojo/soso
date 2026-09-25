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
    // Una observación —`Caso::observacion`, sin `esperado`— **no es un
    // aprobado**: imprimirla como `ok` la disfraza de veredicto. Se distingue
    // aquí aunque esta sonda no tenga ninguna todavía, porque la trampa la
    // paga quien añada la primera.
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

    // 8. **Filtros por glob** (hueco declarado en N-009). Se preparan dos
    //    ficheros con la misma aguja y distinta extensión.
    let dir_glob = unir(DIR, "globs");
    sys::mkdir(&dir_glob);
    sys::mkdir(&unir(&dir_glob, "sub"));
    let aguja = b"AGUJA_GLOB\n";
    for r in [
        unir(&dir_glob, "uno.rs"),
        unir(&dir_glob, "dos.txt"),
        unir(&unir(&dir_glob, "sub"), "tres.rs"),
    ] {
        if let Err(e) = escribir(&r, aguja) {
            anotar(&mut casos, Caso::nuevo("busqueda/preparar-globs", "ficheros", e));
            return casos;
        }
    }

    // **El control, primero.** Sin filtro tienen que salir los tres: si no,
    // «sólo salen los .rs» sería compatible con «sólo se encuentra uno».
    let (c, texto) = correr(&["-r", "-l", "AGUJA_GLOB", &dir_glob]);
    let todos = texto.contains("uno.rs") && texto.contains("dos.txt") && texto.contains("tres.rs");
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/sin-filtro-salen-todos",
            "los tres",
            if c == 0 && todos { String::from("los tres") } else { format!("código {c}, {texto:?}") },
        ),
    );

    let (c, texto) = correr(&["-r", "-l", "--include=*.rs", "AGUJA_GLOB", &dir_glob]);
    let solo_rs = texto.contains("uno.rs") && texto.contains("tres.rs") && !texto.contains("dos.txt");
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/include-filtra-por-extension",
            "los .rs y sólo ésos",
            if c == 0 && solo_rs { String::from("los .rs y sólo ésos") } else { format!("código {c}, {texto:?}") },
        ),
    );

    // El glob mira el **nombre**, no la ruta: `*.rs` tiene que casar
    // `…/sub/tres.rs`, y comparando la ruta entera no casaría por las barras.
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/el-glob-mira-el-nombre-no-la-ruta",
            "encuentra el anidado",
            if texto.contains("sub/tres.rs") { String::from("encuentra el anidado") } else { format!("{texto:?}") },
        ),
    );

    // Y `--exclude` gana sobre `--include`: quien excluye algo lo hace para no
    // verlo.
    let (c, texto) = correr(&["-r", "-l", "--include=*.rs", "--exclude=tres*", "AGUJA_GLOB", &dir_glob]);
    let sin_tres = texto.contains("uno.rs") && !texto.contains("tres.rs");
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/exclude-gana-sobre-include",
            "queda uno.rs sin tres.rs",
            if c == 0 && sin_tres { String::from("queda uno.rs sin tres.rs") } else { format!("código {c}, {texto:?}") },
        ),
    );

    // Un glob que no casa con nada no es un error: es «no hay», código 1.
    let (c, _) = correr(&["-r", "-l", "--include=*.zzz", "AGUJA_GLOB", &dir_glob]);
    anotar(
        &mut casos,
        Caso::nuevo("busqueda/un-glob-sin-coincidencias-sale-1", "1", format!("{c}")),
    );

    // 9. **Binarios**: coinciden, pero no se vuelcan. Es el otro hueco que
    //    N-009 declaró: cuando quien lee es un agente, volcar bytes crudos no
    //    es sólo feo — se lleva por delante su contexto.
    let bin = unir(DIR, "binario.bin");
    let mut datos = alloc::vec![0x7fu8, b'E', b'L', b'F', 0x02, 0x01, 0x00, 0x00];
    datos.extend_from_slice(b"AGUJA_EN_BINARIO");
    datos.extend_from_slice(&[0u8; 32]);
    if let Err(e) = escribir(&bin, &datos) {
        anotar(&mut casos, Caso::nuevo("busqueda/preparar-binario", "fichero", e));
        return casos;
    }

    let (c, texto) = correr(&["AGUJA_EN_BINARIO", &bin]);
    // Coincide (código 0) y lo dice, pero **no** aparecen los bytes crudos: el
    // 0x7f del ELF no debe estar en la salida.
    let anuncia = texto.contains("binario coincide");
    let sin_basura = !texto.contains('\u{7f}');
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/un-binario-se-anuncia-no-se-vuelca",
            "0 y lo anuncia sin volcar",
            if c == 0 && anuncia && sin_basura {
                String::from("0 y lo anuncia sin volcar")
            } else {
                format!("código {c}, anuncia={anuncia}, sin_basura={sin_basura}")
            },
        ),
    );

    // **El control**: con `-a` sí se vuelca. Sin este caso, «no salieron los
    // bytes» sería compatible con «no encontró nada».
    let (c, texto) = correr(&["-a", "AGUJA_EN_BINARIO", &bin]);
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/con-a-el-binario-si-se-vuelca",
            "0 y sale la aguja",
            if c == 0 && texto.contains("AGUJA_EN_BINARIO") {
                String::from("0 y sale la aguja")
            } else {
                format!("código {c}, {texto:?}")
            },
        ),
    );

    // Y un binario que **no** coincide no se anuncia: anunciarlo sería decir
    // que hay algo donde no lo hay.
    let (c, texto) = correr(&["NO_ESTA_EN_EL_BINARIO", &bin]);
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/un-binario-que-no-coincide-calla",
            "1 y no dice nada",
            if c == 1 && !texto.contains("binario coincide") {
                String::from("1 y no dice nada")
            } else {
                format!("código {c}, {texto:?}")
            },
        ),
    );

    // 10. **Retroceso del `*`.** Es la parte del emparejador que puede fallar
    //     en silencio: `*a.rs` tiene que casar `aaa.rs` cediendo terreno. Se
    //     comprueba aquí porque `libsoso` es `no_std` y sus tests unitarios no
    //     se pueden ejecutar en el host.
    let (c, texto) = correr(&["-r", "-l", "--include=*o.rs", "AGUJA_GLOB", &dir_glob]);
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/el-asterisco-retrocede",
            "casa uno.rs",
            if c == 0 && texto.contains("uno.rs") && !texto.contains("tres.rs") {
                String::from("casa uno.rs")
            } else {
                format!("código {c}, {texto:?}")
            },
        ),
    );

    // Y `?` es exactamente un carácter: `?res.rs` no casa `tres.rs` (son
    // cuatro letras antes del punto), pero `????.rs` sí.
    let (_, texto_uno) = correr(&["-r", "-l", "--include=????.rs", "AGUJA_GLOB", &dir_glob]);
    anotar(
        &mut casos,
        Caso::nuevo(
            "busqueda/interrogante-es-un-caracter",
            "casa tres.rs y no uno.rs",
            if texto_uno.contains("tres.rs") && !texto_uno.contains("uno.rs") {
                String::from("casa tres.rs y no uno.rs")
            } else {
                format!("{texto_uno:?}")
            },
        ),
    );

    casos
}
