//! sosh: la shell de soso. Pipes (`|`) y redirecciones (`<`, `>`, `>>`).
//! Una línea puede ser un pipeline de comandos de /bin, más los builtins
//! `exit`, `help`, `cd`, `pwd` y `ask`.
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
const PROTO_FIN: u8 = 0xFF;
const LINE_MAX: usize = 1024;
const CONF: &str = "/etc/llm.conf";

fn main(_args: &str) -> u8 {
    println!("sosh — escribe 'help' para la ayuda");
    let mut lector = Lector::new();
    loop {
        print!("{PROMPT}");
        let Some(cmd) = lector.siguiente() else {
            // Ctrl-D: salir como con `exit`.
            return 0;
        };
        if let Some(code) = ejecutar(&cmd) {
            return code;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Word,
    Pipe,
    RedirectOut,
    RedirectAppend,
    RedirectIn,
}

struct Token {
    kind: TokenKind,
    word: String,
}

#[derive(Clone)]
enum RedirSpec {
    Tty,
    Path(String, u64),
}

struct CmdSpec {
    prog: String,
    args: String,
    stdin: RedirSpec,
    stdout: RedirSpec,
}

fn tokenize(line: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        match c {
            '|' => tokens.push(Token {
                kind: TokenKind::Pipe,
                word: String::new(),
            }),
            '>' => {
                let kind = if chars.peek() == Some(&'>') {
                    chars.next();
                    TokenKind::RedirectAppend
                } else {
                    TokenKind::RedirectOut
                };
                tokens.push(Token {
                    kind,
                    word: String::new(),
                });
            }
            '<' => tokens.push(Token {
                kind: TokenKind::RedirectIn,
                word: String::new(),
            }),
            _ => {
                let mut word = String::new();
                word.push(c);
                while let Some(&nc) = chars.peek() {
                    if nc.is_whitespace() || nc == '|' || nc == '>' || nc == '<' {
                        break;
                    }
                    word.push(chars.next().unwrap());
                }
                tokens.push(Token {
                    kind: TokenKind::Word,
                    word,
                });
            }
        }
    }
    tokens
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
        let mut words = Vec::new();
        let mut i = 0usize;
        while i < seg.len() {
            match seg[i].kind {
                TokenKind::RedirectIn => {
                    i += 1;
                    let path = seg.get(i).ok_or("falta fichero tras <")?;
                    if path.kind != TokenKind::Word {
                        return Err("falta fichero tras <");
                    }
                    stdin = RedirSpec::Path(path.word.clone(), abi::O_RDONLY);
                    i += 1;
                }
                TokenKind::RedirectOut => {
                    i += 1;
                    let path = seg.get(i).ok_or("falta fichero tras >")?;
                    if path.kind != TokenKind::Word {
                        return Err("falta fichero tras >");
                    }
                    stdout = RedirSpec::Path(path.word.clone(), abi::O_WRONLY);
                    i += 1;
                }
                TokenKind::RedirectAppend => {
                    i += 1;
                    let path = seg.get(i).ok_or("falta fichero tras >>")?;
                    if path.kind != TokenKind::Word {
                        return Err("falta fichero tras >>");
                    }
                    stdout = RedirSpec::Path(path.word.clone(), abi::O_WRONLY | abi::O_APPEND);
                    i += 1;
                }
                TokenKind::Word => {
                    words.push(seg[i].word.as_str());
                    i += 1;
                }
                TokenKind::Pipe => return Err("sintaxis inválida"),
            }
        }
        if words.is_empty() {
            return Err("sin comando");
        }
        let prog = words[0].to_string();
        let args = if words.len() > 1 {
            words[1..].join(" ")
        } else {
            String::new()
        };
        cmds.push(CmdSpec {
            prog,
            args,
            stdin,
            stdout,
        });
    }
    Ok(cmds)
}

fn open_redir(spec: &RedirSpec) -> Result<u64, i64> {
    match spec {
        RedirSpec::Tty => Ok(abi::FD_INHERIT_TTY),
        RedirSpec::Path(path, flags) => {
            let fd = sys::open(path, *flags);
            if fd < 0 {
                Err(fd)
            } else {
                Ok(fd as u64)
            }
        }
    }
}

fn ejecutar_pipeline(cmds: &[CmdSpec]) -> Option<u8> {
    let n = cmds.len();
    let mut pipes = Vec::new();
    for _ in 0..n.saturating_sub(1) {
        match sys::pipe() {
            Ok(pair) => pipes.push(pair),
            Err(e) => {
                println!("sosh: pipe: {}", errno_str(e));
                return None;
            }
        }
    }

    let mut pids = Vec::new();
    for (i, cmd) in cmds.iter().enumerate() {
        let stdin_fd = if i == 0 {
            match open_redir(&cmd.stdin) {
                Ok(fd) => fd,
                Err(e) => {
                    println!("sosh: {}: {}", cmd.prog, errno_str(e));
                    return None;
                }
            }
        } else {
            pipes[i - 1].0
        };

        let stdout_fd = if i + 1 == n {
            match open_redir(&cmd.stdout) {
                Ok(fd) => fd,
                Err(e) => {
                    println!("sosh: {}: {}", cmd.prog, errno_str(e));
                    return None;
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

        let pid = sys::spawn_io(&path, &cmd.args, stdin_fd, stdout_fd, abi::FD_INHERIT_TTY);
        if pid < 0 {
            println!("sosh: {}: {}", cmd.prog, errno_str(pid));
            return None;
        }
        pids.push(pid);
    }

    let mut fallo = false;
    for (cmd, pid) in cmds.iter().zip(pids.iter()) {
        match wait_pid(*pid as u64) {
            Ok(0) => {}
            Ok(code) => {
                println!("sosh: [{} salió con código {code}]", cmd.prog);
                fallo = true;
            }
            Err(e) => {
                println!("sosh: wait: {}", errno_str(e));
                fallo = true;
            }
        }
    }
    let _ = fallo;
    None
}

fn ayuda() {
    println!("builtins: exit [código], help, cd, pwd, ask");
    println!("ask:      ask <pregunta>  — el texto va literal al modelo");
    println!("          ask             — modo interactivo (Ctrl-D o «salir»)");
    println!("          /bin/ask-modelo — elegir el modelo que usa ask");
    println!("comandos: ELF de /bin o ruta absoluta");
    println!("pipes:    cmd1 | cmd2 | cmd3");
    println!("redirect: cmd > fichero, cmd >> fichero, cmd < fichero");
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

fn spawn_askd() {
    if let Ok((r, w)) = sys::pipe() {
        let _ = sys::close(w);
        let _ = sys::spawn_io(ASKD, "askd", r, r, r);
        let _ = sys::close(r);
    } else {
        let _ = sys::spawn(ASKD, "askd");
    }
}

fn connect_askd() -> Result<u64, i64> {
    let addr = parse_sock_addr(ASK_ADDR).ok_or(-abi::EINVAL)?;
    let fd = sys::tcp_connect(&addr, 5_000);
    if fd < 0 {
        Err(fd)
    } else {
        Ok(fd as u64)
    }
}

fn copiar_respuesta_ask(fd: u64) {
    let mut buf = [0u8; 512];
    loop {
        let n = sys::read_timeout(fd, &mut buf, 120_000);
        if n == -(abi::EAGAIN as i64) {
            continue;
        }
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == PROTO_FIN {
                return;
            }
            let mut one = [b];
            let _ = sys::write(1, &mut one);
        }
    }
}

fn preguntar_via_askd(texto: &str) -> u8 {
    let fd = match connect_askd() {
        Ok(f) => f,
        Err(_) => {
            spawn_askd();
            let mut fd = None;
            for _ in 0..100 {
                let _ = sys::sleep_ms(50);
                if let Ok(f) = connect_askd() {
                    fd = Some(f);
                    break;
                }
            }
            match fd {
                Some(f) => f,
                None => {
                    println!("ask: no pude conectar con el servicio en {ASK_ADDR}");
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
    copiar_respuesta_ask(fd);
    let _ = sys::close(fd);
    0
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
        let Some(linea) = lector.siguiente() else {
            return Some(0);
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
    let code = preguntar_via_askd(texto);
    if code != 0 {
        Some(code)
    } else {
        None
    }
}

/// Ejecuta una línea. Some(código) = salir de la shell.
fn ejecutar(line: &str) -> Option<u8> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // `ask` va antes que el tokenizador a propósito: el resto de la línea es
    // texto para el modelo, no una expresión de la shell. Si pasara por
    // `tokenize`, un `¿2 > 1?` se leería como redirección a un fichero `1?` y
    // las comillas quedarían dentro de las palabras.
    if let Some(texto) = resto_de("ask", line) {
        return ejecutar_ask(texto);
    }

    let tokens = tokenize(line);
    let cmds = match parse(&tokens) {
        Ok(c) => c,
        Err(msg) => {
            println!("sosh: {msg}");
            return None;
        }
    };

    let single_tty = cmds.len() == 1
        && matches!(cmds[0].stdin, RedirSpec::Tty)
        && matches!(cmds[0].stdout, RedirSpec::Tty);

    if single_tty {
        match cmds[0].prog.as_str() {
            "exit" if cmds[0].args.is_empty() => return Some(0),
            "help" if cmds[0].args.is_empty() => {
                ayuda();
                return None;
            }
            "cd" => {
                let dir = cmds[0].args.trim();
                if dir.is_empty() {
                    println!("sosh: cd: falta directorio");
                    return None;
                }
                let r = sys::chdir(dir);
                if r < 0 {
                    println!("sosh: cd: {}", errno_str(r));
                }
                return None;
            }
            "pwd" => {
                let mut buf = [0u8; 256];
                let r = sys::getcwd(&mut buf);
                if r < 0 {
                    println!("sosh: pwd: {}", errno_str(r));
                } else {
                    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    println!("{}", core::str::from_utf8(&buf[..n]).unwrap_or("?"));
                }
                return None;
            }
            _ => {}
        }
    }

    if cmds.len() == 1 && cmds[0].args.is_empty() {
        match cmds[0].prog.as_str() {
            "exit" => return Some(0),
            "help" => {
                ayuda();
                return None;
            }
            _ => {}
        }
    }

    if cmds.len() == 1 && cmds[0].prog == "exit" {
        let code = if cmds[0].args.is_empty() {
            0
        } else {
            cmds[0].args.parse().unwrap_or(0)
        };
        return Some(code);
    }

    ejecutar_pipeline(&cmds);
    None
}
