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
/// El binario que atiende `ask`: la inferencia vive donde ya estaba.
const LLM: &str = "/bin/soso-llm";

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
        match sys::wait() {
            Ok((got, 0)) if got == *pid as u64 => {}
            Ok((_, code)) => {
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

/// Lanza `/bin/soso-llm ask <texto>` heredando la tty: el hijo escribe la
/// respuesta y, si no hay texto, se queda con el terminal para su propio REPL.
fn ejecutar_ask(texto: &str) -> Option<u8> {
    let args = if texto.is_empty() {
        "ask".to_string()
    } else {
        format!("ask {texto}")
    };
    let pid = sys::spawn(LLM, &args);
    if pid < 0 {
        println!("sosh: ask: {} ({LLM})", errno_str(pid));
        return None;
    }
    match sys::wait() {
        Ok((_, code)) if code != 0 => println!("sosh: [ask salió con código {code}]"),
        Ok(_) => {}
        Err(e) => println!("sosh: ask: wait: {}", errno_str(e)),
    }
    None
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
