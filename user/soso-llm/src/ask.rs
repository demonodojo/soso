//! `ask`: preguntar al modelo escribiendo el texto tal cual.
//!
//! El modelo vive en `soso-llm askd` (sesión de máquina en 127.0.0.1:7420).
//! Consola, SSH y reconexiones comparten la misma carga; sólo se recarga al
//! cambiar de modelo, si el planificador necesita RAM, o al apagar.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use libsoso::{abi, print, println, sys};
use soso_llm_core::chat;
use soso_llm_core::plan::MemoryPlanConfig;
use soso_llm_core::sample::Sampler;

use crate::net::{parse_sock_addr, TcpFd};
use crate::{generar_tokens, preparar_sesion, Sesion};

const CONF: &str = "/etc/llm.conf";
pub const ASK_PORT: u16 = 7420;
const ASK_ADDR: &str = "127.0.0.1:7420";
/// Fin de respuesta en el protocolo askd↔cliente.
pub const PROTO_FIN: u8 = 0xFF;
const LINE_MAX: usize = 1024;

/// Si `args` es `cmd` o empieza por `cmd `, devuelve el resto **literal**.
pub fn resto_tras<'a>(cmd: &str, args: &'a str) -> Option<&'a str> {
    let args = args.trim_start();
    if args == cmd {
        return Some("");
    }
    args.strip_prefix(cmd)
        .filter(|r| r.starts_with(' '))
        .map(|r| r[1..].trim_end())
}

pub struct Conf {
    pub modelo: String,
    pub max: usize,
    pub temp: f32,
    pub top_p: f32,
    pub seed: u64,
    pub plantilla: String,
}

pub const PLANTILLA_CRUDA: &str = "crudo";

impl Default for Conf {
    fn default() -> Self {
        Conf {
            modelo: String::new(),
            max: 128,
            temp: 0.7,
            top_p: 0.9,
            seed: 42,
            plantilla: String::new(),
        }
    }
}

pub fn plantilla_efectiva<'a>(conf: &'a Conf, sesion: &'a Sesion) -> &'a str {
    if !sesion.bundle.tokenizer.tiene_vocabulario() {
        return "";
    }
    match conf.plantilla.as_str() {
        PLANTILLA_CRUDA => "",
        "" => sesion.bundle.rt.manifest.chat_template.as_str(),
        otra => otra,
    }
}

pub fn leer_conf() -> Conf {
    let mut c = Conf::default();
    let Some(texto) = leer_fichero(CONF) else {
        return c;
    };
    for linea in texto.lines() {
        let linea = linea.trim();
        if linea.is_empty() || linea.starts_with('#') {
            continue;
        }
        if let Some(v) = linea.strip_prefix("modelo=") {
            c.modelo = v.trim().to_string();
        } else if let Some(v) = linea.strip_prefix("max=") {
            if let Ok(n) = v.trim().parse() {
                c.max = n;
            }
        } else if let Some(v) = linea.strip_prefix("temp=") {
            if let Ok(n) = v.trim().parse() {
                c.temp = n;
            }
        } else if let Some(v) = linea.strip_prefix("top_p=") {
            if let Ok(n) = v.trim().parse() {
                c.top_p = n;
            }
        } else if let Some(v) = linea.strip_prefix("seed=") {
            if let Ok(n) = v.trim().parse() {
                c.seed = n;
            }
        } else if let Some(v) = linea.strip_prefix("plantilla=") {
            c.plantilla = chat::desescapar(v.trim_start());
        }
    }
    c
}

pub fn leer_fichero(path: &str) -> Option<String> {
    let fd = sys::open(path, abi::O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut out = String::new();
    let mut buf = [0u8; 512];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n <= 0 {
            break;
        }
        out.push_str(core::str::from_utf8(&buf[..n as usize]).unwrap_or(""));
    }
    sys::close(fd as u64);
    Some(out)
}

pub fn modelos() -> Vec<String> {
    let mut out = Vec::new();
    let fd = sys::open("/models", abi::O_RDONLY);
    if fd < 0 {
        return out;
    }
    let mut ents = [abi::Dirent::default(); 16];
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n <= 0 {
            break;
        }
        for d in &ents[..n as usize / abi::DIRENT_SIZE] {
            if let Ok(nombre) = core::str::from_utf8(d.name_bytes()) {
                out.push(nombre.to_string());
            }
        }
    }
    sys::close(fd as u64);
    out
}

pub fn modelo_efectivo(conf: &Conf) -> Option<String> {
    let disponibles = modelos();
    if !conf.modelo.is_empty() && disponibles.iter().any(|m| *m == conf.modelo) {
        return Some(conf.modelo.clone());
    }
    if !conf.modelo.is_empty() {
        println!(
            "ask: el modelo «{}» de {CONF} no está en /models; uso el primero",
            conf.modelo
        );
    }
    disponibles.into_iter().next()
}

fn socket_write(fd: u64, data: &[u8]) {
    let _ = sys::write_all(fd, data);
}

fn socket_write_str(fd: u64, s: &str) {
    socket_write(fd, s.as_bytes());
}

fn socket_fin(fd: u64) {
    socket_write(fd, &[PROTO_FIN]);
}

fn read_line_fd(fd: u64, buf: &mut [u8]) -> Option<usize> {
    let mut pos = 0usize;
    loop {
        let mut b = [0u8; 1];
        let n = sys::read_timeout(fd, &mut b, 120_000);
        if n <= 0 {
            return if pos > 0 { Some(pos) } else { None };
        }
        if b[0] == b'\n' {
            return Some(pos);
        }
        if pos < buf.len() {
            buf[pos] = b[0];
            pos += 1;
        }
    }
}

fn spawn_askd() -> Result<u64, i64> {
    let rc = sys::spawn_io(
        "/bin/soso-llm",
        "askd",
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
    );
    if rc < 0 { Err(rc) } else { Ok(rc as u64) }
}

fn connect_askd() -> Result<TcpFd, i64> {
    let addr = parse_sock_addr(ASK_ADDR).ok_or(-abi::EINVAL)?;
    TcpFd::connect(&addr, 5_000)
}

/// Cliente: conecta (arrancando askd si hace falta), manda la línea, copia la
/// respuesta a la tty hasta `PROTO_FIN`.
pub fn preguntar_via_askd(texto: &str) -> u8 {
    if let Ok(mut sock) = connect_askd() {
        return preguntar_en_socket(&mut sock, texto);
    }
    if let Err(e) = spawn_askd() {
        println!("ask: no pude lanzar /bin/soso-llm (errno {e})");
        return 1;
    }
    let mut ultimo = 0i64;
    for _ in 0..100 {
        let _ = sys::sleep_ms(50);
        match connect_askd() {
            Ok(mut sock) => return preguntar_en_socket(&mut sock, texto),
            Err(e) => ultimo = e,
        }
    }
    println!("ask: el servicio no escuchó en {ASK_ADDR} tras 5 s (último errno {ultimo})");
    1
}

fn preguntar_en_socket(sock: &mut TcpFd, texto: &str) -> u8 {
    let mut linea = alloc::vec![0u8; LINE_MAX];
    let bytes = texto.as_bytes();
    let n = bytes.len().min(LINE_MAX - 1);
    linea[..n].copy_from_slice(&bytes[..n]);
    linea[n] = b'\n';
    if sys::write_all(sock.fd, &linea[..=n]).is_err() {
        println!("ask: error al enviar la pregunta");
        return 1;
    }
    let mut buf = [0u8; 512];
    let mut silencios = 0;
    loop {
        let n = sys::read_timeout(sock.fd, &mut buf, 120_000);
        if n == -(abi::EAGAIN as i64) {
            silencios += 1;
            if silencios >= 2 {
                println!("\nask: el servicio dejó de responder");
                return 1;
            }
            continue;
        }
        silencios = 0;
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == PROTO_FIN {
                return 0;
            }
            let mut one = [b];
            let _ = sys::write(1, &mut one);
        }
    }
    0
}

pub fn run_ask(texto: &str) -> u8 {
    if let Some(t) = resto_tras(":eco", texto) {
        println!("{t}");
        return 0;
    }
    if texto.is_empty() {
        return repl_interactivo();
    }
    preguntar_via_askd(texto)
}

fn repl_interactivo() -> u8 {
    let conf = leer_conf();
    let Some(modelo) = modelo_efectivo(&conf) else {
        println!("ask: no hay ningún modelo en /models");
        return 1;
    };
    println!("ask: modelo {modelo}, máx {} tokens", conf.max);
    println!("ask: escribe la pregunta; «salir» o Ctrl-D para terminar");
    let mut lector = libsoso::linea::Lector::new();
    loop {
        print!("?> ");
        let Some(linea) = lector.siguiente() else {
            return 0;
        };
        let texto = linea.trim();
        if texto.is_empty() {
            continue;
        }
        if texto == "salir" || texto == "exit" {
            return 0;
        }
        if let Some(t) = resto_tras(":eco", texto) {
            println!("{t}");
            continue;
        }
        let code = preguntar_via_askd(texto);
        if code != 0 {
            return code;
        }
        println!();
    }
}

/// Demonio de máquina: escucha en loopback y atiende una pregunta por conexión.
pub fn run_askd() -> u8 {
    let listener = match TcpFd::listen(ASK_PORT) {
        Ok(l) => l,
        Err(e) if e == -abi::EADDRINUSE => return 0,
        Err(e) => {
            println!("askd: listen {ASK_PORT} falló ({e})");
            return 1;
        }
    };
    println!("askd: escuchando en {ASK_ADDR}");

    let mut sesion: Option<Sesion> = None;
    let mut conf = leer_conf();
    let mut modelo = String::new();

    loop {
        let conn = match TcpFd::accept(&listener, 0) {
            Ok(c) => c,
            Err(e) if e == -abi::EAGAIN => {
                let _ = sys::sleep_ms(50);
                continue;
            }
            Err(e) => {
                println!("askd: accept falló ({e})");
                let _ = sys::sleep_ms(200);
                continue;
            }
        };
        let mut line_buf = [0u8; LINE_MAX];
        let Some(n) = read_line_fd(conn.fd, &mut line_buf) else {
            continue;
        };
        let texto = core::str::from_utf8(&line_buf[..n])
            .unwrap_or("")
            .trim();
        if texto.is_empty() {
            socket_fin(conn.fd);
            continue;
        }
        let rc = tratar_linea_askd(
            &mut sesion,
            &mut conf,
            &mut modelo,
            texto,
            conn.fd,
        );
        if rc != 0 {
            socket_write_str(conn.fd, "ask: error\n");
        }
        socket_fin(conn.fd);
        // Ceder el CPU: sosh está bloqueado en el socket y, si no salimos
        // del hilo, en SMP el despertar del read puede tardar una rodaja.
        let _ = sys::sleep_ms(1);
    }
}

fn asegurar_modelo(
    sesion: &mut Option<Sesion>,
    conf: &Conf,
    modelo: &mut String,
    fd: u64,
) -> Result<(), u8> {
    let want = match modelo_efectivo(conf) {
        Some(m) => m,
        None => {
            socket_write_str(fd, "ask: no hay ningún modelo en /models\n");
            return Err(1);
        }
    };
    let recargar = sesion
        .as_ref()
        .map(|s| s.modelo != want)
        .unwrap_or(true);
    if recargar {
        socket_write_str(fd, &format!("ask: cargando {want}…\n"));
        println!("askd: cargando {want}");
        match preparar_sesion(&want, false, MemoryPlanConfig::default(), false, false) {
            Ok(s) => {
                println!("askd: {want} listo");
                *sesion = Some(s);
                *modelo = want;
                // Sin hilo de staging: sosh está bloqueado en el socket y
                // con SMP>1 el worker no llega a poner `done` (askd cuelga
                // en generate; 2026-08-31). El prefetch va en este hilo.
                if let Some(ses) = sesion.as_mut() {
                    ses.bundle.source.disable_worker();
                }
            }
            Err(c) => {
                println!("askd: no pude cargar {want} (código {c})");
                socket_write_str(fd, &format!("ask: no pude cargar «{want}» (código {c})\n"));
                return Err(c);
            }
        }
    }
    Ok(())
}

fn tratar_linea_askd(
    sesion: &mut Option<Sesion>,
    conf: &mut Conf,
    modelo: &mut String,
    texto: &str,
    fd: u64,
) -> u8 {
    if let Some(t) = resto_tras(":eco", texto) {
        socket_write_str(fd, t);
        socket_write(fd, b"\n");
        return 0;
    }
    if texto == ":modelos" {
        for m in modelos() {
            let marca = if m == *modelo { '*' } else { ' ' };
            socket_write_str(fd, &format!("{marca} {m}\n"));
        }
        return 0;
    }
    if let Some(n) = resto_tras(":modelo", texto) {
        let n = n.trim();
        if n.is_empty() {
            socket_write_str(fd, &format!("ask: modelo actual: {modelo}\n"));
            return 0;
        }
        if !modelos().iter().any(|m| m == n) {
            socket_write_str(fd, &format!("ask: no hay ningún modelo «{n}» en /models\n"));
            return 0;
        }
        conf.modelo = n.to_string();
        *sesion = None;
        if asegurar_modelo(sesion, conf, modelo, fd).is_err() {
            return 1;
        }
        socket_write_str(fd, &format!("ask: modelo {modelo}\n"));
        return 0;
    }
    if let Some(n) = resto_tras(":max", texto) {
        match n.trim().parse() {
            Ok(v) => {
                conf.max = v;
                socket_write_str(fd, &format!("ask: máx {v} tokens\n"));
            }
            Err(_) => socket_write_str(fd, "ask: :max necesita un número\n"),
        }
        return 0;
    }

    if asegurar_modelo(sesion, conf, modelo, fd).is_err() {
        return 1;
    }
    let ses = sesion.as_mut().unwrap();

    let plantilla = plantilla_efectiva(conf, ses);
    let tokens = chat::render(plantilla, texto, &ses.bundle.tokenizer);
    let vocab = ses.bundle.rt.manifest.vocab_size;
    if let Some(t) = tokens.iter().find(|&&t| t >= vocab) {
        socket_write_str(
            fd,
            &format!(
                "ask: este modelo sólo entiende {vocab} tokens y el texto usa el {t}\n"
            ),
        );
        return 1;
    }
    let mut sampler = Sampler::new(conf.temp, conf.top_p, conf.seed);
    println!("askd: generando (máx {})", conf.max);
    let rc = generar_tokens(
        ses,
        &tokens,
        conf.max,
        &mut sampler,
        false,
        None,
        Some(fd),
        true,
    );
    println!("askd: generar rc={rc}");
    rc
}
