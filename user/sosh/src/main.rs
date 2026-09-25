//! sosh: la shell de soso. Pipes (`|`) y redirecciones (`<`, `>`, `>>`).
//! Una línea puede ser un pipeline de comandos de /bin, más los builtins
//! `exit`, `help`, `cd`, `pwd`, `wifi` y `ask`.
//!
//! **Guiones:** `sosh /ruta` ejecuta una orden por línea (comentarios `#`, sin
//! prompt). En la misma línea: `;`, `&&` y `||` con el código de salida del
//! paso anterior.
//!
//! **Lo que no sabe hacer lo dice** ([N-010](../../../docs/self-improvement/native/N-010.md)):
//! segundo plano (`&`), duplicar descriptores (`2>&1`), comodines (`*`) y
//! variables (`$`) se **rechazan con un mensaje**. La salida de emergencia es
//! entrecomillar.
//!
//! `ask` es el único que se resuelve **antes** de tokenizar: todo lo que va
//! detrás es el texto de la pregunta, con sus comillas, sus tildes y sus `|` o
//! `>` si los lleva. Cualquier otro camino los interpretaría como pipe o
//! redirección, y no hay forma de escaparlos en esta shell.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use libsoso::linea::Lector;
use libsoso::{abi, errno_str, print, println, sys};

libsoso::entry!(main);

const PROMPT: &str = "$ ";
/// Demonio de sesión de máquina (modelo residente entre preguntas y SSH).
const ASKD: &str = "/bin/soso-llm";
const ASK_ADDR: &str = "127.0.0.1:7420";
const VOZD: &str = "/bin/soso-voz";
const PROTO_FIN: u8 = 0xFF;
const LINE_MAX: usize = 1024;
/// Tope de un fichero de guion leído entero en memoria.
const SCRIPT_MAX: usize = 256 * 1024;
const CONF: &str = "/etc/llm.conf";

/// ET_EXEC de soso: cabecera y PT_LOAD en 0x400000 (p_offset 0).
const ELF_BASE: usize = 0x400000;
const PT_LOAD: u32 = 1;
const PAGE: usize = 4096;

fn load_u16(p: *const u8) -> u16 {
    unsafe { core::ptr::read_unaligned(p.cast()) }
}
fn load_u32(p: *const u8) -> u32 {
    unsafe { core::ptr::read_unaligned(p.cast()) }
}
fn load_u64(p: *const u8) -> u64 {
    unsafe { core::ptr::read_unaligned(p.cast()) }
}

/// Falta cada página de los PT_LOAD **antes** de `/tmp/sosh-ready`.
/// Si el ELF perezoso no puede resolver una página, morimos sin marca y
/// init no confirma OTA.
fn prefault_imagen() {
    let base = ELF_BASE as *const u8;
    unsafe {
        if *base != 0x7f || *base.add(1) != b'E' || *base.add(2) != b'L' || *base.add(3) != b'F' {
            return;
        }
        let phoff = load_u64(base.add(32)) as usize;
        let phentsize = load_u16(base.add(54)) as usize;
        let phnum = load_u16(base.add(56)) as usize;
        if phentsize < 56 || phnum == 0 || phnum > 16 || phoff > 64 * 1024 {
            return;
        }
        for i in 0..phnum {
            let ph = base.add(phoff + i * phentsize);
            if load_u32(ph) != PT_LOAD {
                continue;
            }
            let vaddr = load_u64(ph.add(16)) as usize;
            let memsz = load_u64(ph.add(40)) as usize;
            if memsz == 0 || memsz > 32 * 1024 * 1024 {
                continue;
            }
            let end = vaddr.saturating_add(memsz);
            let mut page = vaddr & !(PAGE - 1);
            while page < end {
                let _ = core::ptr::read_volatile(page as *const u8);
                match page.checked_add(PAGE) {
                    Some(n) => page = n,
                    None => break,
                }
            }
        }
    }
}

/// Marca ligada a *esta* instancia (`pid=N`). init no confirma OTA con
/// un fichero viejo, un banner, ni un write ignorado.
fn escribir_marca_listo(prefault_ms: i64) {
    let t0 = sys::uptime_ms();
    let mkdir_r = sys::mkdir("/tmp");
    if mkdir_r < 0 {
        let mut st = abi::Stat::default();
        if sys::stat("/tmp", &mut st) < 0 {
            println!("sosh: mkdir /tmp falló ({mkdir_r}); sin marca OTA");
            return;
        }
    }
    let fd = sys::open("/tmp/sosh-ready", abi::O_WRONLY);
    if fd < 0 {
        println!("sosh: open /tmp/sosh-ready falló ({fd}); sin marca OTA");
        return;
    }
    let pid = sys::getpid();
    let linea = format!("pid={pid}\n");
    let n = sys::write(fd as u64, linea.as_bytes());
    let close_r = sys::close(fd as u64);
    if n < linea.len() as i64 {
        println!("sosh: write marca falló ({n}); sin marca OTA");
        return;
    }
    if close_r < 0 {
        println!("sosh: close marca falló ({close_r}); sin marca OTA");
        return;
    }
    let marca_ms = sys::uptime_ms().saturating_sub(t0);
    println!("sosh: marca lista pid={pid} prefault={prefault_ms}ms write={marca_ms}ms");
}

fn main(args: &[alloc::string::String]) -> u8 {
    let shell_pgid = sys::getpid();
    let _ = sys::setsid();
    let _ = sys::tcsetpgrp(shell_pgid);
    println!("sosh — escribe 'help' para la ayuda");
    let t0 = sys::uptime_ms();
    prefault_imagen();
    let prefault_ms = sys::uptime_ms().saturating_sub(t0);
    if args.is_empty() {
        escribir_marca_listo(prefault_ms);
    }
    if let Some(path) = args.first() {
        return ejecutar_guion(path);
    }
    let mut lector = Lector::new().con_hook_ptt(hook_ptt);
    loop {
        print!("{PROMPT}");
        match lector.siguiente() {
            Ok(Some(cmd)) => {
                if let Some(code) = ejecutar(&cmd, None).0 {
                    return code;
                }
            }
            Ok(None) => return 0,
            Err(e) => {
                println!("sosh: tty: {}", errno_str(e));
                return 1;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Word,
    /// `N>&M`: el descriptor `fd` pasa a apuntar donde `dup`.
    RedirectDup,
    Pipe,
    RedirectOut,
    RedirectAppend,
    RedirectIn,
    RedirectClose,
    Semicolon,
    AndAnd,
    OrOr,
}

#[derive(Clone)]
struct Token {
    kind: TokenKind,
    word: String,
    /// Descriptor destino (1–3) para redirecciones; 0 en palabras y pipes.
    fd: u64,
    /// La palabra llevaba comillas o escapes (T64).
    ///
    /// Importa por dos cosas: una palabra entrecomillada **nunca** es un
    /// operador —`">"` es un nombre de fichero, no una redirección— y `""` es
    /// un argumento vacío, que no es lo mismo que ningún argumento.
    entrecomillada: bool,
    /// La palabra traía `*` o `?` **fuera** de comillas: hay que expandirla.
    ///
    /// Se marca en el tokenizador y no se mira el texto después, porque
    /// después ya no se sabe si el `*` venía entrecomillado. Es el mismo
    /// motivo por el que existe `entrecomillada` (T64).
    con_comodin: bool,
}

#[derive(Clone)]
enum RedirSpec {
    Tty,
    Log,
    Closed,
    Path(String, u64),
    /// «Apunta a donde apunte el descriptor N» (`2>&1`).
    ///
    /// Se resuelve **después** de montar todas las redirecciones, así que
    /// `cmd 2>&1 >f` y `cmd >f 2>&1` hacen lo mismo. En POSIX **no**: allí el
    /// orden manda y el primero deja stderr en la tty. Es una divergencia
    /// declarada, no un descuido — `CmdSpec` guarda una ranura por descriptor y
    /// conservar el orden pediría otra estructura.
    Dup(u64),
}

struct CmdSpec {
    prog: String,
    /// Los argumentos **separados**, tal como salieron del tokenizador.
    ///
    /// Antes se volvían a juntar con espacios aquí y el hijo los recibía en una
    /// sola cadena, así que una ruta con un espacio dentro era imposible de
    /// pasar aunque el tokenizador la hubiera reconocido bien (T62). Las
    /// palabras ya estaban separadas: lo único que hacía falta era no tirarlas.
    args: Vec<String>,
    stdin: RedirSpec,
    stdout: RedirSpec,
    stderr: RedirSpec,
    log: RedirSpec,
}

/// Parte la línea en palabras y operadores, respetando comillas (T64).
///
/// Reglas, elegidas en la ficha y no ampliables sin volver a decidirlas:
///
/// - `'…'` es literal hasta la comilla de cierre; **dentro no hay escapes**.
/// - `"…"` es literal salvo `\"` y `\\`.
/// - Fuera de comillas, `\X` da `X` literal: `\ ` es un espacio y `\|` una barra.
/// - Una comilla sin cerrar es un **error**, no una palabra a medias.
///
/// Las comillas se quitan del resultado. sosh no expande variables, así que
/// `'` y `"` hacen hoy lo mismo; se aceptan las dos porque quien escriba la
/// otra recibiría la comilla dentro del argumento y un fallo que no se parece
/// a su causa.
/// Lo que `sosh` **no** sabe hacer, dicho en voz alta.
///
/// Es la semántica declarada de
/// [N-010](../../../docs/self-improvement/native/N-010.md): antes estos
/// operadores se colaban como argumentos del comando, así que una línea que
/// pedía una cosa hacía otra **sin decirlo**. Un guion que los usara parecía
/// colgarse, y el diagnóstico costaba caro.
///
/// La salida de emergencia es entrecomillar, igual que el `-F` de `grep`: si
/// de verdad quieres el carácter, `"a;b"` lo da.
const SEGUNDO_PLANO: &str = "no sé ejecutar en segundo plano (&); cada comando termina antes del siguiente";
const DUPLICAR: &str = "tras >& sólo entiendo un dígito (2>&1) o un guion (>&-)";
const COMODINES: &str = "no expando comodines (*); entrecomíllalo si es literal";
const VARIABLES: &str = "no expando variables ($); entrecomíllalo si es literal";

fn tokenize(line: &str) -> Result<Vec<Token>, &'static str> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        match c {
            '|' => {
                if chars.peek() == Some(&'|') {
                    chars.next();
                    tokens.push(Token {
                        kind: TokenKind::OrOr,
                        word: String::new(),
                        fd: 0,
                        con_comodin: false,
                    entrecomillada: false,
                    });
                } else {
                    tokens.push(Token {
                        kind: TokenKind::Pipe,
                        word: String::new(),
                        fd: 0,
                        con_comodin: false,
                    entrecomillada: false,
                    });
                }
            }
            '&' => {
                if chars.peek() == Some(&'&') {
                    chars.next();
                    tokens.push(Token {
                        kind: TokenKind::AndAnd,
                        word: String::new(),
                        fd: 0,
                        con_comodin: false,
                    entrecomillada: false,
                    });
                } else {
                    return Err(SEGUNDO_PLANO);
                }
            }
            ';' => tokens.push(Token {
                kind: TokenKind::Semicolon,
                word: String::new(),
                fd: 0,
                con_comodin: false,
                    entrecomillada: false,
            }),
            '>' => {
                let mut destino_dup = 0u64;
                let kind = if chars.peek() == Some(&'>') {
                    chars.next();
                    TokenKind::RedirectAppend
                } else if chars.peek() == Some(&'&') {
                    chars.next();
                    match chars.next() {
                        Some('-') => TokenKind::RedirectClose,
                        Some(d) if d.is_ascii_digit() => {
                            destino_dup = d.to_digit(10).unwrap_or(0) as u64;
                            TokenKind::RedirectDup
                        }
                        _ => return Err(DUPLICAR),
                    }
                } else {
                    TokenKind::RedirectOut
                };
                tokens.push(Token {
                    kind,
                    // El destino de un `>&N` viaja en `word`: es lo único
                    // variable que lleva ese token, y añadir un campo a `Token`
                    // para usarlo en un caso de cada cien es peor negocio.
                    word: if kind == TokenKind::RedirectDup {
                        format!("{destino_dup}")
                    } else {
                        String::new()
                    },
                    fd: 1,
                    entrecomillada: false,
                    con_comodin: false,
                });
            }
            '<' => tokens.push(Token {
                kind: TokenKind::RedirectIn,
                word: String::new(),
                fd: 0,
                con_comodin: false,
                    entrecomillada: false,
            }),
            _ => {
                let mut word = String::new();
                let mut entrecomillada = false;
                let mut con_comodin = false;
                // El primer carácter ya se consumió: se trata igual que el
                // resto para que `"a"b` y `a"b"` den los dos `ab`.
                let mut pendiente = Some(c);
                loop {
                    let ch = match pendiente.take() {
                        Some(ch) => ch,
                        None => match chars.peek() {
                            Some(&nc)
                                if nc.is_whitespace()
                                    || nc == '|'
                                    || nc == '>'
                                    || nc == '<' =>
                            {
                                break
                            }
                            Some(_) => chars.next().unwrap(),
                            None => break,
                        },
                    };
                    match ch {
                        '\'' => {
                            entrecomillada = true;
                            loop {
                                match chars.next() {
                                    Some('\'') => break,
                                    Some(x) => word.push(x),
                                    None => return Err("comilla simple sin cerrar"),
                                }
                            }
                        }
                        '"' => {
                            entrecomillada = true;
                            loop {
                                match chars.next() {
                                    Some('"') => break,
                                    Some('\\') => match chars.next() {
                                        // Sólo `\"` y `\\`: lo demás se queda
                                        // literal, barra incluida, para que
                                        // una ruta no se coma sus separadores.
                                        Some(x @ ('"' | '\\')) => word.push(x),
                                        Some(x) => {
                                            word.push('\\');
                                            word.push(x);
                                        }
                                        None => return Err("comilla doble sin cerrar"),
                                    },
                                    Some(x) => word.push(x),
                                    None => return Err("comilla doble sin cerrar"),
                                }
                            }
                        }
                        '\\' => {
                            entrecomillada = true;
                            match chars.next() {
                                Some(x) => word.push(x),
                                None => return Err("barra invertida al final de la línea"),
                            }
                        }
                        // Sin comillas, `*` y `$` sólo pueden ser una
                        // intención que esta shell no cumple. Dejarlos pasar
                        // daba respuestas que parecen hechos: `ls *.rs` decía
                        // «*.rs: no existe», que suena a que no hay ficheros.
                        // `*` y `?` sin comillas se expanden después, contra
                        // el directorio. Con comillas son literales, que es la
                        // salida de emergencia de siempre en esta shell.
                        '*' | '?' => {
                            con_comodin = true;
                            word.push(ch);
                        }
                        '$' => return Err(VARIABLES),
                        x => word.push(x),
                    }
                }
                // Un dígito suelto pegado a `>` es un descriptor… salvo que
                // viniera entrecomillado, en cuyo caso es una palabra.
                if !entrecomillada && word.len() == 1 {
                    if let Some(d) = word.chars().next().and_then(|ch| ch.to_digit(10)) {
                        if (1..=3).contains(&d) && chars.peek() == Some(&'>') {
                            chars.next();
                            let fd = d as u64;
                            let mut destino_dup = 0u64;
                            let kind = if chars.peek() == Some(&'>') {
                                chars.next();
                                TokenKind::RedirectAppend
                            } else if chars.peek() == Some(&'&') {
                                chars.next();
                                match chars.next() {
                                    Some('-') => TokenKind::RedirectClose,
                                    Some(d) if d.is_ascii_digit() => {
                                        destino_dup = d.to_digit(10).unwrap_or(0) as u64;
                                        TokenKind::RedirectDup
                                    }
                                    _ => return Err(DUPLICAR),
                                }
                            } else {
                                TokenKind::RedirectOut
                            };
                            tokens.push(Token {
                                kind,
                                word: if kind == TokenKind::RedirectDup {
                                    format!("{destino_dup}")
                                } else {
                                    String::new()
                                },
                                fd,
                                entrecomillada: false,
                                con_comodin: false,
                            });
                            continue;
                        }
                    }
                }
                tokens.push(Token {
                    kind: TokenKind::Word,
                    word,
                    fd: 0,
                    entrecomillada,
                    con_comodin,
                });
            }
        }
    }
    Ok(tokens)
}

fn parse(tokens: &[Token]) -> Result<Vec<CmdSpec>, &'static str> {
    let mut segments: Vec<Vec<&Token>> = Vec::new();
    segments.push(Vec::new());
    for t in tokens {
        if t.kind == TokenKind::Pipe {
            segments.push(Vec::new());
        } else {
            segments.last_mut().unwrap().push(t);
        }
    }

    let mut cmds = Vec::new();
    for seg in segments {
        if seg.is_empty() {
            return Err("comando vacío en el pipeline");
        }
        let mut stdin = RedirSpec::Tty;
        let mut stdout = RedirSpec::Tty;
        let mut stderr = RedirSpec::Tty;
        let mut log = RedirSpec::Log;
        let mut words = Vec::new();
        let mut i = 0usize;
        while i < seg.len() {
            let tok = &seg[i];
            match tok.kind {
                TokenKind::RedirectIn => {
                    i += 1;
                    let path = seg.get(i).ok_or("falta fichero tras <")?;
                    if path.kind != TokenKind::Word {
                        return Err("falta fichero tras <");
                    }
                    stdin = RedirSpec::Path(path.word.clone(), abi::O_RDONLY);
                    i += 1;
                }
                TokenKind::RedirectDup => {
                    let destino: u64 = tok.word.parse().unwrap_or(9);
                    if !(0..=3).contains(&destino) {
                        return Err("sólo se puede duplicar 0, 1, 2 o 3");
                    }
                    if destino == tok.fd {
                        return Err("un descriptor no puede duplicarse a sí mismo");
                    }
                    let spec = RedirSpec::Dup(destino);
                    match tok.fd {
                        1 => stdout = spec,
                        2 => stderr = spec,
                        3 => log = spec,
                        _ => return Err("descriptor de redirección inválido"),
                    }
                    i += 1;
                }
                TokenKind::RedirectOut => {
                    i += 1;
                    let path = seg.get(i).ok_or("falta fichero tras >")?;
                    if path.kind != TokenKind::Word {
                        return Err("falta fichero tras >");
                    }
                    let spec = RedirSpec::Path(path.word.clone(), abi::O_WRONLY);
                    match tok.fd {
                        1 => stdout = spec,
                        2 => stderr = spec,
                        3 => log = spec,
                        _ => return Err("descriptor de redirección inválido"),
                    }
                    i += 1;
                }
                TokenKind::RedirectAppend => {
                    i += 1;
                    let path = seg.get(i).ok_or("falta fichero tras >>")?;
                    if path.kind != TokenKind::Word {
                        return Err("falta fichero tras >>");
                    }
                    let spec =
                        RedirSpec::Path(path.word.clone(), abi::O_WRONLY | abi::O_APPEND);
                    match tok.fd {
                        1 => stdout = spec,
                        2 => stderr = spec,
                        3 => log = spec,
                        _ => return Err("descriptor de redirección inválido"),
                    }
                    i += 1;
                }
                TokenKind::RedirectClose => {
                    match tok.fd {
                        1 => stdout = RedirSpec::Closed,
                        2 => stderr = RedirSpec::Closed,
                        3 => log = RedirSpec::Closed,
                        _ => return Err("descriptor de redirección inválido"),
                    }
                    i += 1;
                }
                TokenKind::Word => {
                    words.push(tok.word.as_str());
                    i += 1;
                }
                TokenKind::Pipe
                | TokenKind::Semicolon
                | TokenKind::AndAnd
                | TokenKind::OrOr => return Err("sintaxis inválida"),
            }
        }
        if words.is_empty() {
            return Err("sin comando");
        }
        let prog = words[0].to_string();
        let args: Vec<String> = words[1..].iter().map(|w| String::from(*w)).collect();
        cmds.push(CmdSpec {
            prog,
            args,
            stdin,
            stdout,
            stderr,
            log,
        });
    }
    Ok(cmds)
}

#[derive(Clone, Copy)]
enum ChainLink {
    Always,
    And,
    Or,
}

/// Expande las palabras con comodines contra el sistema de ficheros.
///
/// **Un patrón que no casa con nada es un error**, no se pasa tal cual. Bash
/// hace lo contrario por omisión, y eso es justo lo que N-010 vino a quitar de
/// esta shell: `ls *.rs` sin ficheros `.rs` le entregaba a `ls` la cadena
/// `*.rs` y salía «*.rs: no existe», que suena a un hecho sobre el disco
/// cuando lo que pasa es que no había nada que expandir.
///
/// Sólo el **último** componente lleva comodín: `/tmp/*.txt` vale, `a/*/b` no.
/// Lo segundo pide recorrer el árbol y no lo pide nadie todavía.
fn expandir_comodines(tokens: Vec<Token>) -> Result<Vec<Token>, String> {
    let mut out = Vec::with_capacity(tokens.len());
    for t in tokens {
        if !t.con_comodin || t.kind != TokenKind::Word {
            out.push(t);
            continue;
        }
        let (dir, patron) = match t.word.rfind('/') {
            Some(i) => (&t.word[..=i], &t.word[i + 1..]),
            None => ("", t.word.as_str()),
        };
        if libsoso::glob::tiene_comodin(dir) {
            return Err(format!(
                "sólo el último componente puede llevar comodín: {}",
                t.word
            ));
        }
        let base = if dir.is_empty() { "." } else { dir };
        let mut casan = listar_que_casan(base, patron);
        if casan.is_empty() {
            return Err(format!("ningún fichero casa con {}", t.word));
        }
        // En orden, para que dos ejecuciones den lo mismo: `getdents` no
        // promete ninguno.
        casan.sort();
        for nombre in casan {
            out.push(Token {
                kind: TokenKind::Word,
                word: format!("{dir}{nombre}"),
                fd: 0,
                entrecomillada: false,
                con_comodin: false,
            });
        }
    }
    Ok(out)
}

/// Nombres de `dir` que casan con `patron`.
fn listar_que_casan(dir: &str, patron: &str) -> Vec<String> {
    let mut out = Vec::new();
    let fd = sys::open(dir, abi::O_RDONLY);
    if fd < 0 {
        return out;
    }
    let mut ents = [abi::Dirent::default(); 16];
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n <= 0 {
            break;
        }
        for e in &ents[..n as usize / abi::DIRENT_SIZE] {
            let Ok(nombre) = core::str::from_utf8(e.name_bytes()) else {
                continue;
            };
            if nombre == "." || nombre == ".." {
                continue;
            }
            // Los ocultos sólo salen si el patrón empieza por punto, como en
            // cualquier shell: si no, `rm *` se llevaría la configuración.
            if nombre.starts_with('.') && !patron.starts_with('.') {
                continue;
            }
            if libsoso::glob::casa(nombre, patron) {
                out.push(String::from(nombre));
            }
        }
    }
    sys::close(fd as u64);
    out
}

fn split_chain(tokens: &[Token]) -> Result<Vec<(ChainLink, Vec<Token>)>, &'static str> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut next_link = ChainLink::Always;
    for t in tokens {
        match t.kind {
            TokenKind::Semicolon => {
                if cur.is_empty() {
                    return Err("comando vacío en la cadena");
                }
                out.push((next_link, core::mem::take(&mut cur)));
                next_link = ChainLink::Always;
            }
            TokenKind::AndAnd => {
                if cur.is_empty() {
                    return Err("comando vacío en la cadena");
                }
                out.push((next_link, core::mem::take(&mut cur)));
                next_link = ChainLink::And;
            }
            TokenKind::OrOr => {
                if cur.is_empty() {
                    return Err("comando vacío en la cadena");
                }
                out.push((next_link, core::mem::take(&mut cur)));
                next_link = ChainLink::Or;
            }
            _ => cur.push(t.clone()),
        }
    }
    if cur.is_empty() {
        if out.is_empty() {
            return Ok(out);
        }
        return Err("comando vacío al final");
    }
    out.push((next_link, cur));
    Ok(out)
}

fn sosh_msg(ctx: Option<usize>, msg: &str) {
    if let Some(n) = ctx {
        println!("sosh: línea {n}: {msg}");
    } else {
        println!("sosh: {msg}");
    }
}

fn leer_fichero_script(path: &str) -> Result<Vec<u8>, i64> {
    let mut st = abi::Stat::default();
    let sr = sys::stat(path, &mut st);
    if sr < 0 {
        return Err(sr);
    }
    let size = st.size as usize;
    if size > SCRIPT_MAX {
        return Err(-abi::EINVAL);
    }
    let fd = sys::open(path, abi::O_RDONLY);
    if fd < 0 {
        return Err(fd);
    }
    let mut buf = alloc::vec![0u8; size];
    if size == 0 {
        let _ = sys::close(fd as u64);
        return Ok(buf);
    }
    let mut off = 0usize;
    while off < size {
        let n = sys::read(fd as u64, &mut buf[off..]);
        if n <= 0 {
            let _ = sys::close(fd as u64);
            return Err(if n < 0 { n } else { -abi::EIO });
        }
        off += n as usize;
    }
    let _ = sys::close(fd as u64);
    Ok(buf)
}

fn ejecutar_guion(path: &str) -> u8 {
    let data = match leer_fichero_script(path) {
        Ok(d) => d,
        Err(e) => {
            println!("sosh: {}: {}", path, errno_str(e));
            return 1;
        }
    };
    let mut ultimo = 0u8;
    let mut n = 1usize;
    for raw in data.split(|&b| b == b'\n') {
        let raw = if raw.last() == Some(&b'\r') {
            &raw[..raw.len().saturating_sub(1)]
        } else {
            raw
        };
        let line = core::str::from_utf8(raw).unwrap_or("");
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            n += 1;
            continue;
        }
        let (exit_shell, code) = ejecutar(trimmed, Some(n));
        ultimo = code;
        if let Some(c) = exit_shell {
            return c;
        }
        n += 1;
    }
    ultimo
}

fn open_redir(spec: &RedirSpec) -> Result<u64, i64> {
    match spec {
        RedirSpec::Tty => Ok(abi::FD_INHERIT_TTY),
        RedirSpec::Log => Ok(abi::FD_KERNEL_LOG),
        RedirSpec::Closed => Ok(abi::FD_CLOSED),
        RedirSpec::Path(path, flags) => {
            let fd = sys::open(path, *flags);
            if fd < 0 { Err(fd) } else { Ok(fd as u64) }
        }
        // Una duplicación no abre nada: apunta a un descriptor que el
        // pipeline ya ha montado, así que se resuelve allí y no aquí.
        RedirSpec::Dup(_) => Ok(abi::FD_INHERIT_TTY),
    }
}

/// Resuelve un `N>&M` contra los descriptores ya montados.
///
/// Se hace **después** de abrir todo, así que `2>&1 >f` y `>f 2>&1` dan lo
/// mismo. En POSIX el orden manda; aquí no, y está declarado en `RedirSpec`.
fn resolver_dup(spec: &RedirSpec, stdin: u64, stdout: u64, actual: u64) -> u64 {
    match spec {
        RedirSpec::Dup(0) => stdin,
        RedirSpec::Dup(1) => stdout,
        _ => actual,
    }
}

fn ejecutar_pipeline(cmds: &[CmdSpec], ctx: Option<usize>) -> u8 {
    let n = cmds.len();
    let mut pipes = Vec::new();
    for _ in 0..n.saturating_sub(1) {
        match sys::pipe() {
            Ok(pair) => pipes.push(pair),
            Err(e) => {
                sosh_msg(ctx, &format!("pipe: {}", errno_str(e)));
                return 1;
            }
        }
    }

    let mut pids = Vec::new();
    for (i, cmd) in cmds.iter().enumerate() {
        let stdin_fd = if i == 0 {
            match open_redir(&cmd.stdin) {
                Ok(fd) => fd,
                Err(e) => {
                    sosh_msg(ctx, &format!("{}: {}", cmd.prog, errno_str(e)));
                    return 1;
                }
            }
        } else {
            pipes[i - 1].0
        };

        let stdout_fd = if i + 1 == n {
            match open_redir(&cmd.stdout) {
                Ok(fd) => fd,
                Err(e) => {
                    sosh_msg(ctx, &format!("{}: {}", cmd.prog, errno_str(e)));
                    return 1;
                }
            }
        } else {
            pipes[i].1
        };

        let path = if cmd.prog.starts_with('/') {
            cmd.prog.clone()
        } else {
            format!("/bin/{}", cmd.prog)
        };

        let stderr_fd = match open_redir(&cmd.stderr) {
            Ok(fd) => fd,
            Err(e) => {
                sosh_msg(ctx, &format!("{}: {}", cmd.prog, errno_str(e)));
                return 1;
            }
        };
        let log_fd = match open_redir(&cmd.log) {
            Ok(fd) => fd,
            Err(e) => {
                sosh_msg(ctx, &format!("{}: {}", cmd.prog, errno_str(e)));
                return 1;
            }
        };

        let mut argv: Vec<&str> = Vec::with_capacity(cmd.args.len() + 1);
        argv.push(path.as_str());
        argv.extend(cmd.args.iter().map(|a| a.as_str()));
        // `2>&1` y `3>&1`: el hijo recibe un array de descriptores, así que
        // duplicar es pasar el mismo número dos veces.
        let stderr_fd = resolver_dup(&cmd.stderr, stdin_fd, stdout_fd, stderr_fd);
        let log_fd = resolver_dup(&cmd.log, stdin_fd, stdout_fd, log_fd);
        let pid = sys::spawn_io_full(&path, &argv, &[], [stdin_fd, stdout_fd, stderr_fd, log_fd]);
        if pid < 0 {
            sosh_msg(ctx, &format!("{}: {}", cmd.prog, errno_str(pid)));
            return 1;
        }
        pids.push(pid);
    }

    if let Some(&first) = pids.first() {
        for &pid in &pids {
            let _ = sys::setpgid(pid as u64, first as u64);
        }
        let _ = sys::tcsetpgrp(first as u64);
    }

    let shell_pgid = sys::getpid();
    let mut ultimo = 0u8;
    for (cmd, pid) in cmds.iter().zip(pids.iter()) {
        match wait_pid(*pid as u64) {
            Ok(0) => ultimo = 0,
            Ok(130) => {
                println!("sosh: [{} interrumpido]", cmd.prog);
                ultimo = 130;
            }
            Ok(code) => {
                println!("sosh: [{} salió con código {code}]", cmd.prog);
                ultimo = code;
            }
            Err(e) => {
                sosh_msg(ctx, &format!("wait: {}", errno_str(e)));
                ultimo = 1;
            }
        }
    }
    let _ = sys::tcsetpgrp(shell_pgid);
    ultimo
}

/// Un pipeline (sin operadores de cadena). `(Some, _)` = salir de la shell.
fn ejecutar_pipeline_o_builtin(cmds: &[CmdSpec], ctx: Option<usize>) -> (Option<u8>, u8) {
    let single_tty = cmds.len() == 1
        && matches!(cmds[0].stdin, RedirSpec::Tty)
        && matches!(cmds[0].stdout, RedirSpec::Tty);

    if single_tty {
        match cmds[0].prog.as_str() {
            "exit" if cmds[0].args.is_empty() => return (Some(0), 0),
            "help" if cmds[0].args.is_empty() => {
                ayuda();
                return (None, 0);
            }
            "cd" => {
                let dir = cmds[0].args.first().map(|s| s.as_str()).unwrap_or("");
                if dir.is_empty() {
                    sosh_msg(ctx, "cd: falta directorio");
                    return (None, 1);
                }
                let r = sys::chdir(dir);
                if r < 0 {
                    sosh_msg(ctx, &format!("cd: {}", errno_str(r)));
                    return (None, 1);
                }
                return (None, 0);
            }
            "pwd" => {
                let mut buf = [0u8; 256];
                let r = sys::getcwd(&mut buf);
                if r < 0 {
                    sosh_msg(ctx, &format!("pwd: {}", errno_str(r)));
                    return (None, 1);
                }
                let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                println!("{}", core::str::from_utf8(&buf[..n]).unwrap_or("?"));
                return (None, 0);
            }
            "wifi" => {
                ejecutar_wifi(&cmds[0].args.join(" "));
                return (None, 0);
            }
            "sosolog" if cmds[0].args.is_empty() => {
                ejecutar_sosolog();
                return (None, 0);
            }
            _ => {}
        }
    }

    if cmds.len() == 1 && cmds[0].args.is_empty() {
        match cmds[0].prog.as_str() {
            "exit" => return (Some(0), 0),
            "help" => {
                ayuda();
                return (None, 0);
            }
            "sosolog" => {
                ejecutar_sosolog();
                return (None, 0);
            }
            _ => {}
        }
    }

    if cmds.len() == 1 && cmds[0].prog == "exit" {
        let code = if cmds[0].args.is_empty() {
            0
        } else {
            cmds[0].args[0].parse().unwrap_or(0)
        };
        return (Some(code), code);
    }

    (None, ejecutar_pipeline(cmds, ctx))
}

fn wifi_uso() {
    println!("uso: wifi scan | status | connect <ssid> [psk]");
}

fn ejecutar_wifi(args: &str) {
    let args = args.trim();
    if args.is_empty() {
        wifi_uso();
        return;
    }
    if args == "scan" {
        let mut bss = [abi::WifiBss::default(); abi::WIFI_SCAN_MAX];
        let r = sys::wifi_scan(&mut bss);
        if r < 0 {
            if r == -abi::ENOTSUP {
                let mut st = abi::WifiStatus::default();
                if sys::wifi_status(&mut st) >= 0 {
                    if st.flags & abi::WIFI_FLAG_PRESENT == 0 {
                        println!("wifi: no hay adaptador");
                        return;
                    }
                    if st.flags & abi::WIFI_FLAG_ALIVE == 0 {
                        let phase_n = st
                            .phase
                            .iter()
                            .position(|&b| b == 0)
                            .unwrap_or(st.phase.len());
                        let phase = core::str::from_utf8(&st.phase[..phase_n]).unwrap_or("?");
                        println!("wifi: firmware no arrancó (phase={phase})");
                        return;
                    }
                }
            }
            println!("sosh: wifi scan: {}", errno_str(r));
            return;
        }
        if r == 0 {
            println!("wifi: ninguna red");
            return;
        }
        for e in &bss[..r as usize] {
            let n = (e.ssid_len as usize).min(abi::WIFI_SSID_MAX);
            let ssid = core::str::from_utf8(&e.ssid[..n]).unwrap_or("?");
            let sec = if e.open != 0 { "abierta" } else { "WPA" };
            println!("  {ssid}: {} dBm, canal {}, {sec}", e.rssi, e.channel);
        }
        return;
    }
    if args == "status" {
        let mut st = abi::WifiStatus::default();
        let r = sys::wifi_status(&mut st);
        if r < 0 {
            println!("sosh: wifi status: {}", errno_str(r));
            return;
        }
        if st.flags & abi::WIFI_FLAG_PRESENT == 0 {
            println!("wifi: no hay adaptador");
            return;
        }
        let phase_n = st
            .phase
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(st.phase.len());
        let phase = core::str::from_utf8(&st.phase[..phase_n]).unwrap_or("?");
        println!(
            "wifi: alive={} asociada={} autorizada={} phase={}",
            st.flags & abi::WIFI_FLAG_ALIVE != 0,
            st.flags & abi::WIFI_FLAG_CONNECTED != 0,
            st.flags & abi::WIFI_FLAG_AUTHORIZED != 0,
            phase
        );
        println!(
            "  mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            st.mac[0], st.mac[1], st.mac[2], st.mac[3], st.mac[4], st.mac[5]
        );
        return;
    }
    if let Some(rest) = args.strip_prefix("connect") {
        let rest = rest.trim();
        if rest.is_empty() {
            wifi_uso();
            return;
        }
        let (ssid, psk) = match rest.split_once(char::is_whitespace) {
            Some((s, p)) => (s, Some(p.trim())),
            None => (rest, None),
        };
        let r = sys::wifi_connect(ssid, psk);
        if r < 0 {
            println!("sosh: wifi connect: {}", errno_str(r));
        } else {
            println!("wifi: asociado a '{ssid}' (se usará al arrancar)");
        }
        return;
    }
    wifi_uso();
}

fn ejecutar_sosolog() {
    let r = sys::fatlog_flush();
    if r < 0 {
        println!("sosh: sosolog: {}", errno_str(r));
    } else {
        // El kernel vuelca a los destinos que haya: /var/log siempre que sosofs
        // esté montado, y además SOSOLOG.TXT si se arrancó de un live.
        println!("sosh: log volcado (/var/log y, en live, SOSOLOG.TXT)");
    }
}

fn ayuda() {
    println!("builtins: exit [código], help, cd, pwd, wifi, ask, voz, sosolog");
    println!("sosolog:  persistir el log ahora: /var/log/*.log y, en live, SOSOLOG.TXT");
    println!("wifi:     wifi scan | status | connect <ssid> [psk]");
    println!("ask:      ask <pregunta>  — el texto va literal al modelo");
    println!("          ask             — modo interactivo (Ctrl-D o «salir»)");
    println!("          /bin/ask-modelo — elegir el modelo que usa ask");
    println!("          log | grep askd  — trazas de carga de ask (fd 3)");
    println!("voz:      voz             — dictar; Enter confirma la línea");
    println!("          voz ask         — prefija «ask » al dictado");
    println!("          F4              — push-to-talk en la línea");
    println!("comandos: ELF de /bin o ruta absoluta (ip, ls, cat, …)");
    println!("install:  soso-install  — clonar live a un NVMe (elige disco)");
    println!("          soso-install list | nvme1 --yes | status");
    println!("guion:    sosh /ruta  — una orden por línea; # comentario al inicio");
    println!("cadena:   cmd1 ; cmd2   cmd1 && cmd2   cmd1 || cmd2");
    println!("pipes:    cmd1 | cmd2 | cmd3");
    println!("redirect: cmd > fichero, cmd >> fichero, cmd < fichero");
    println!("no hay:   & (segundo plano), 2>&1 (redirige cada uno),");
    println!("          * y $ (entrecomíllalos)");
    println!("ojo:      ask no admite pipes ni redirecciones, justamente para");
    println!("          que `|` y `>` puedan formar parte de la pregunta");
}

/// Si `line` es `cmd` o empieza por `cmd `, devuelve el resto **sin tocar**
/// (solo se recorta el espacio del final, que sobra siempre).
fn resto_de<'a>(cmd: &str, line: &'a str) -> Option<&'a str> {
    if line == cmd {
        return Some("");
    }
    line.strip_prefix(cmd)
        .filter(|r| r.starts_with(' '))
        .map(|r| r[1..].trim_end())
}

/// Espera a un hijo concreto; si otro hijo zombi (p. ej. askd) sale antes, lo
/// recoge y sigue esperando.
fn wait_pid(want: u64) -> Result<u8, i64> {
    loop {
        match sys::wait() {
            Ok((got, code)) if got == want => return Ok(code),
            Ok(_) => {}
            Err(e) if e == -abi::ECHILD => return Err(-abi::ECHILD),
            Err(e) => return Err(e),
        }
    }
}

fn parse_sock_addr(s: &str) -> Option<abi::SockAddr> {
    let (host, port) = s.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    let mut oct = [0u8; 4];
    for (i, part) in host.split('.').enumerate() {
        if i >= 4 {
            return None;
        }
        oct[i] = part.parse().ok()?;
    }
    Some(sys::sock_addr(oct[0], oct[1], oct[2], oct[3], port))
}

/// Lanza el demonio. Devuelve el pid, o el errno del spawn.
///
/// Stdio 0–2 a la consola serie: un panic o un `println!` olvidado no se
/// mezcla con la sesión SSH. El diagnóstico de askd va por fd 3 (`logln!`,
/// `spawn_io` ya pone `FD_KERNEL_LOG`): acaba en `log` y
/// `/var/log/aplicaciones.log`, no en el socket ni en `SOSOLOG.TXT`.
fn spawn_askd() -> Result<u64, i64> {
    let rc = sys::spawn_io(
        ASKD,
        "askd",
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
    );
    if rc < 0 { Err(rc) } else { Ok(rc as u64) }
}

fn connect_askd() -> Result<u64, i64> {
    let addr = parse_sock_addr(ASK_ADDR).ok_or(-abi::EINVAL)?;
    let fd = sys::tcp_connect(&addr, 5_000);
    if fd < 0 { Err(fd) } else { Ok(fd as u64) }
}

/// `Ok(true)` = respuesta completa; `Ok(false)` = Ctrl-C (EINTR).
fn copiar_respuesta_ask(fd: u64) -> Result<bool, ()> {
    let mut buf = [0u8; 512];
    // EAGAIN = el askd sigue vivo y no ha escrito (carga larga). EOF (n==0)
    // = el par cerró: el kernel lo señala cuando el otro extremo hace close,
    // no hay que inventar un tope de silencios (la carga de un GGUF en USB
    // dura más que cualquier timeout razonable).
    loop {
        let n = sys::read_timeout(fd, &mut buf, 120_000);
        if n == -(abi::EINTR as i64) {
            return Ok(false);
        }
        if n == -(abi::EAGAIN as i64) {
            continue;
        }
        if n <= 0 {
            break;
        }
        let n = n as usize;
        if let Some(i) = buf[..n].iter().position(|&b| b == PROTO_FIN) {
            if i > 0 {
                let _ = sys::write(1, &buf[..i]);
            }
            return Ok(true);
        }
        let _ = sys::write(1, &buf[..n]);
    }
    Ok(true)
}

fn preguntar_via_askd(texto: &str) -> u8 {
    let fd = match connect_askd() {
        Ok(f) => f,
        Err(e) if e == -(abi::EINTR as i64) => return 130,
        Err(_) => {
            if let Err(e) = spawn_askd() {
                println!("ask: no pude lanzar {ASKD} (errno {e})");
                return 1;
            }
            let mut fd = None;
            let mut ultimo = 0i64;
            for _ in 0..100 {
                let s = sys::sleep_ms(50);
                if s == -(abi::EINTR as i64) {
                    return 130;
                }
                match connect_askd() {
                    Ok(f) => {
                        fd = Some(f);
                        break;
                    }
                    Err(e) if e == -(abi::EINTR as i64) => return 130,
                    Err(e) => ultimo = e,
                }
            }
            match fd {
                Some(f) => f,
                None => {
                    println!(
                        "ask: el servicio no escuchó en {ASK_ADDR} tras 5 s (último errno {ultimo})"
                    );
                    return 1;
                }
            }
        }
    };
    let mut linea = alloc::vec![0u8; LINE_MAX];
    let bytes = texto.as_bytes();
    let n = bytes.len().min(LINE_MAX - 1);
    linea[..n].copy_from_slice(&bytes[..n]);
    linea[n] = b'\n';
    if sys::write_all(fd, &linea[..=n]).is_err() {
        let _ = sys::close(fd);
        println!("ask: error al enviar la pregunta");
        return 1;
    }
    let interrumpido = copiar_respuesta_ask(fd) == Ok(false);
    let _ = sys::close(fd);
    if interrumpido {
        130
    } else {
        0
    }
}

fn leer_conf_modelo() -> (String, usize) {
    let mut modelo = String::new();
    let mut max = 128usize;
    let fd = sys::open(CONF, abi::O_RDONLY);
    if fd < 0 {
        return (modelo, max);
    }
    let mut texto = String::new();
    let mut buf = [0u8; 256];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n <= 0 {
            break;
        }
        texto.push_str(core::str::from_utf8(&buf[..n as usize]).unwrap_or(""));
    }
    sys::close(fd as u64);
    for linea in texto.lines() {
        let linea = linea.trim();
        if let Some(v) = linea.strip_prefix("modelo=") {
            modelo = v.trim().to_string();
        } else if let Some(v) = linea.strip_prefix("max=") {
            if let Ok(n) = v.trim().parse() {
                max = n;
            }
        }
    }
    (modelo, max)
}

fn modelo_efectivo(conf_modelo: &str) -> Option<String> {
    let fd = sys::open("/models", abi::O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut disponibles = Vec::new();
    let mut ents = [abi::Dirent::default(); 16];
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n <= 0 {
            break;
        }
        for d in &ents[..n as usize / abi::DIRENT_SIZE] {
            if let Ok(nombre) = core::str::from_utf8(d.name_bytes()) {
                disponibles.push(nombre.to_string());
            }
        }
    }
    sys::close(fd as u64);
    if !conf_modelo.is_empty() && disponibles.iter().any(|m| m == conf_modelo) {
        return Some(conf_modelo.to_string());
    }
    disponibles.into_iter().next()
}

fn hook_ptt() -> Option<String> {
    transcribir_voz(":escucha")
}

fn spawn_vozd() -> Result<u64, i64> {
    let rc = sys::spawn_io(
        VOZD,
        "vozd",
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
    );
    if rc < 0 { Err(rc) } else { Ok(rc as u64) }
}

fn transcribir_voz(cmd: &str) -> Option<String> {
    const VOZ_ADDR: &str = "127.0.0.1:7421";
    let addr = parse_sock_addr(VOZ_ADDR)?;
    for intento in 0..100 {
        let fd = sys::tcp_connect(&addr, if intento == 0 { 2_000 } else { 100 });
        if fd < 0 {
            if intento == 0 {
                let _ = spawn_vozd();
            }
            let _ = sys::sleep_ms(50);
            continue;
        }
        let fd = fd as u64;
        let mut req = cmd.as_bytes().to_vec();
        req.push(b'\n');
        if sys::write_all(fd, &req).is_err() {
            sys::close(fd);
            continue;
        }
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        loop {
            let n = sys::read_timeout(fd, &mut buf, 120_000);
            if n <= 0 {
                break;
            }
            for &b in &buf[..n as usize] {
                if b == PROTO_FIN {
                    sys::close(fd);
                    return String::from_utf8(out).ok();
                }
                out.push(b);
            }
        }
        sys::close(fd);
    }
    None
}

fn ejecutar_voz(texto: &str) -> Option<u8> {
    let (prefijo, resto_ask) = if let Some(rest) = resto_de("ask", texto) {
        if rest.is_empty() {
            ("ask ", false)
        } else {
            // voz ask <extra>: el dictado se concatena tras «ask <extra>»
            match transcribir_voz(":escucha") {
                Some(t) => {
                    let linea = format!("ask {rest}{t}");
                    return ejecutar(&linea, None).0;
                }
                None => {
                    println!("voz: error de transcripción");
                    return None;
                }
            }
        }
    } else if texto.is_empty() {
        ("", false)
    } else {
        (texto, false)
    };
    let _ = resto_ask;
    match transcribir_voz(":escucha") {
        Some(t) => {
            let linea = format!("{prefijo}{t}");
            print!("{PROMPT}");
            let mut lector = Lector::new().con_texto_inicial(&linea);
            match lector.siguiente() {
                Ok(Some(cmd)) => ejecutar(&cmd, None).0,
                Ok(None) => Some(0),
                Err(e) => {
                    println!("sosh: tty: {}", errno_str(e));
                    Some(1)
                }
            }
        }
        None => {
            println!("voz: error de transcripción");
            None
        }
    }
}

fn repl_ask() -> Option<u8> {
    let (conf_modelo, max) = leer_conf_modelo();
    let Some(modelo) = modelo_efectivo(&conf_modelo) else {
        println!("ask: no hay ningún modelo en /models");
        return Some(1);
    };
    println!("ask: modelo {modelo}, máx {max} tokens");
    println!("ask: escribe la pregunta; «salir» o Ctrl-D para terminar");
    let mut lector = Lector::new();
    loop {
        print!("?> ");
        let linea = match lector.siguiente() {
            Ok(Some(l)) => l,
            Ok(None) => return Some(0),
            Err(e) => {
                println!("sosh: tty: {}", errno_str(e));
                return Some(1);
            }
        };
        let texto = linea.trim();
        if texto.is_empty() {
            continue;
        }
        if texto == "salir" || texto == "exit" {
            return Some(0);
        }
        if let Some(t) = resto_de(":eco", texto) {
            println!("{t}");
            continue;
        }
        let code = preguntar_via_askd(texto);
        if code != 0 {
            return Some(code);
        }
        println!();
    }
}

/// Cliente TCP del askd global; `:eco` va local sin demonio.
fn ejecutar_ask(texto: &str) -> Option<u8> {
    if let Some(t) = resto_de(":eco", texto) {
        println!("{t}");
        return None;
    }
    if texto.is_empty() {
        return repl_ask();
    }
    // Un `ask` que falla NO mata la shell: `Some(_)` aquí es «sal del bucle», y
    // devolver el código del comando hacía que init relanzara sosh en cada
    // pregunta fallida — en placa sin red se veía «sosh murió con código 1»
    // detrás de cada intento (2026-08-31).
    let _ = preguntar_via_askd(texto);
    None
}

/// Ejecuta una línea. `(Some(código), _)` = salir de la shell; el segundo valor
/// es el código del último paso (para guiones y encadenado).
fn ejecutar(line: &str, ctx: Option<usize>) -> (Option<u8>, u8) {
    let line = line.trim();
    if line.is_empty() {
        return (None, 0);
    }

    // `ask` va antes que el tokenizador a propósito: el resto de la línea es
    // texto para el modelo, no una expresión de la shell. Si pasara por
    // `tokenize`, un `¿2 > 1?` se leería como redirección a un fichero `1?` y
    // las comillas quedarían dentro de las palabras.
    if let Some(texto) = resto_de("ask", line) {
        return (ejecutar_ask(texto), 0);
    }
    if let Some(texto) = resto_de("voz", line) {
        return (ejecutar_voz(texto), 0);
    }

    let tokens = match tokenize(line) {
        Ok(t) => t,
        Err(msg) => {
            sosh_msg(ctx, msg);
            return (None, 1);
        }
    };
    let tokens = match expandir_comodines(tokens) {
        Ok(t) => t,
        Err(msg) => {
            sosh_msg(ctx, &msg);
            return (None, 1);
        }
    };
    let chain = match split_chain(&tokens) {
        Ok(c) => c,
        Err(msg) => {
            sosh_msg(ctx, msg);
            return (None, 1);
        }
    };
    if chain.is_empty() {
        return (None, 0);
    }

    let mut status = 0u8;
    for (link, seg_tokens) in chain {
        let run = match link {
            ChainLink::Always => true,
            ChainLink::And => status == 0,
            ChainLink::Or => status != 0,
        };
        if !run {
            continue;
        }
        let cmds = match parse(&seg_tokens) {
            Ok(c) => c,
            Err(msg) => {
                sosh_msg(ctx, msg);
                status = 1;
                continue;
            }
        };
        let (exit_shell, code) = ejecutar_pipeline_o_builtin(&cmds, ctx);
        status = code;
        if let Some(c) = exit_shell {
            return (Some(c), status);
        }
    }
    (None, status)
}
