//! `cargo xtask test-llm-api` — e2e API (T19).

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, exit};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use soso_llm_api::{
    check_busy_while_generating_guest, check_health_during_generation_guest, check_server_down,
    run_core_invariants_guest, ApiClient, GuestHttpLimits, ProfileFile, StepResult,
};

use crate::test::{lanzar_qemu, ssh_guion, ssh_vive, QemuSlot};

const SSH_PORT: u16 = 2299;
const ECHO_PORT: u16 = 7799;
const LLM_PORT: u16 = 17299;
const TOKEN: &str = "test-llm-api-token";
const MAC: &str = "52:54:00:12:34:99";
/// `serve` sólo escucha HTTP tras `preparar_sesion_api`, y esa carga depende
/// del modelo: un tiny tarda segundos y el Qwen2.5-Coder-3B minutos, paginando
/// 2,2 GB desde sosomfs. Los 180s de la versión con tiny declaraban muerto un
/// arranque sano. `SOSO_LLM_API_HEALTH_WAIT_S` lo ajusta sin recompilar.
const HEALTH_WAIT_DEFAULT: Duration = Duration::from_secs(1200);
/// La sesión que sostiene `serve` dura toda la prueba: no es un timeout de
/// respuesta, es el tope del proceso entero.
const SERVE_SESSION_LIMIT: Duration = Duration::from_secs(3600);

struct Args {
    synthetic: bool,
    model_dir: Option<PathBuf>,
    profile: Option<PathBuf>,
    inspect_argv: bool,
}

#[derive(Default)]
struct PhaseLog {
    entries: Vec<(String, f64)>,
}

impl PhaseLog {
    fn record(&mut self, name: &str, start: Instant) {
        let secs = start.elapsed().as_secs_f64();
        println!("test-llm-api: fase {name} {secs:.1}s");
        self.entries.push((name.to_string(), secs));
    }
}

pub fn run(from: &[String]) {
    let args = parse(from);
    if args.inspect_argv {
        print_argv_only(&args);
        return;
    }
    let root = crate::project_root();
    let arte = root.join("target/self-improvement/tasks/T19");
    let _ = fs::create_dir_all(&arte);

    if args.synthetic {
        run_synthetic(&root, &arte);
        return;
    }

    let Some(model_dir) = args.model_dir else {
        uso();
        exit(1);
    };
    let Some(profile_path) = args.profile else {
        eprintln!("test-llm-api: falta --profile <model-lock.json>");
        uso();
        exit(1);
    };
    if !model_dir.join("manifest.som").is_file() {
        eprintln!(
            "test-llm-api: BLOQUEADO — {} no tiene manifest.som",
            model_dir.display()
        );
        write_evidence(&arte, "bloqueado", &[], &PhaseLog::default(), 2);
        exit(2);
    }
    let profile = load_profile(&profile_path);
    let catalog = profile.catalog_name().to_string();
    run_guest(&root, &arte, &model_dir, &catalog);
}

fn parse(from: &[String]) -> Args {
    let mut synthetic = false;
    let mut model_dir = None;
    let mut profile = None;
    let mut inspect_argv = false;
    let mut i = 0;
    while i < from.len() {
        match from[i].as_str() {
            "--synthetic" => synthetic = true,
            "--inspect-argv" => inspect_argv = true,
            "--model-dir" => {
                i += 1;
                model_dir = from.get(i).map(PathBuf::from);
            }
            "--profile" => {
                i += 1;
                profile = from.get(i).map(PathBuf::from);
            }
            _ => {}
        }
        i += 1;
    }
    Args {
        synthetic,
        model_dir,
        profile,
        inspect_argv,
    }
}

fn uso() {
    eprintln!("uso:");
    eprintln!("  cargo xtask test-llm-api --synthetic");
    eprintln!("  cargo xtask test-llm-api --model-dir <dir> --profile <model-lock.json>");
    eprintln!("  cargo xtask test-llm-api --inspect-argv …  (solo imprime forwards QEMU)");
}

fn load_profile(path: &Path) -> ProfileFile {
    let data = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("test-llm-api: no leo {} ({e})", path.display());
        exit(1);
    });
    serde_json::from_str(&data).unwrap_or_else(|e| {
        eprintln!("test-llm-api: perfil JSON inválido ({e})");
        exit(1);
    })
}

fn run_synthetic(root: &Path, arte: &Path) {
    println!("test-llm-api: modo sintético (no acredita agente)");
    let ok = Command::new("cargo")
        .current_dir(root)
        .args([
            "test",
            "-p",
            "soso-llm-api",
            "--features",
            "std",
            "--test",
            "e2e_suite",
            "--",
            "--test-threads=1",
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    write_evidence(
        arte,
        if ok { "synthetic_ok" } else { "synthetic_fail" },
        &[],
        &PhaseLog::default(),
        if ok { 0 } else { 1 },
    );
    exit(if ok { 0 } else { 1 });
}

fn run_guest(root: &Path, arte: &Path, model_dir: &Path, catalog: &str) {
    println!(
        "test-llm-api: guest modelo={catalog} dir={} puerto_llm={LLM_PORT}",
        model_dir.display()
    );
    if llm_port_ocupado() {
        eprintln!(
            "test-llm-api: BLOQUEADO — 127.0.0.1:{LLM_PORT} ya acepta TCP (¿QEMU huérfano?). \
             Mata `qemu-system-x86` con test-llm-api y reintenta."
        );
        write_evidence(&arte, "bloqueado_puerto", &[], &PhaseLog::default(), 2);
        exit(2);
    }
    // El tamaño sale del árbol `.som`, no de una constante: con 512M fijos el
    // modelo real (2,2 GB) desborda el dispositivo y mkfs-sosomfs sólo dice
    // `construir sosomfs: "escribir shard"`.
    let models_size = crate::live_models::suggest_models_image_size_one(model_dir);
    unsafe {
        std::env::set_var("SOSO_MODELS_DIR", model_dir);
        if std::env::var_os("SOSO_MODELS_SIZE").is_none() {
            println!("test-llm-api: imagen de modelos {models_size} (calculada del árbol .som)");
            std::env::set_var("SOSO_MODELS_SIZE", &models_size);
        }
    }

    let mut phases = PhaseLog::default();

    let t = Instant::now();
    crate::build_user();
    phases.record("build_user", t);

    let t = Instant::now();
    let img = crate::build_image();
    phases.record("build_image", t);

    let t = Instant::now();
    let data = crate::mkfs_rootfs(false);
    let models = crate::mkfs_models(false);
    phases.record("mkfs", t);

    let key = root.join("target/soso_test_key");
    let serial = root.join("target/test-llm-api-serial.log");

    let slot = QemuSlot {
        id: "llm-api",
        ssh_port: SSH_PORT,
        echo_port: ECHO_PORT,
        llm_host_port: Some(LLM_PORT),
        mac: MAC.into(),
        serial: serial.clone(),
        bios: copiar_firmware(&img),
        data: copiar_si_necesario(&data, "data"),
        models: copiar_si_necesario(&models, "models"),
        // 2048M dejaban «presupuesto pesos 1040 MiB» para un modelo de
        // 2212 MiB: cada token paginaba desde disco y una petición de dos
        // tokens tardaba ~200 s (medido 2026-09-23). Con los pesos residentes
        // la campaña deja de ser una prueba de paciencia. El modelo real es la
        // entrada de esta ficha; el tamaño de la máquina, parte del arnés.
        mem: Some("5120M".into()),
        smp: None,
        monitor: None,
        guest: crate::QemuGuestConfig::from_env(),
    };

    let t = Instant::now();
    let mut qemu = match lanzar_qemu(&slot) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("test-llm-api: QEMU no arrancó ({e})");
            write_evidence(arte, "qemu_fail", &[], &phases, 1);
            exit(1);
        }
    };
    phases.record("lanzar_qemu", t);

    let mut steps: Vec<StepResult> = Vec::new();
    let exit_code = match run_guest_inner(&key, &serial, catalog, &mut steps, &mut qemu, &mut phases) {
        Ok(()) => 0,
        Err(msg) => {
            eprintln!("test-llm-api: {msg}");
            steps.push(StepResult {
                name: "guest_run",
                ok: false,
                detail: msg,
            });
            1
        }
    };

    let _ = qemu.kill();
    let _ = qemu.wait();
    write_evidence(
        arte,
        if exit_code == 0 { "guest_ok" } else { "guest_fail" },
        &steps,
        &phases,
        exit_code,
    );
    exit(exit_code);
}

fn run_guest_inner(
    key: &Path,
    serial: &Path,
    catalog: &str,
    steps: &mut Vec<StepResult>,
    qemu: &mut Child,
    phases: &mut PhaseLog,
) -> Result<(), String> {
    let t = Instant::now();
    wait_guest_ready(key, serial, qemu)?;
    phases.record("boot_ssh", t);

    let t = Instant::now();
    // El `echo` de soso no tiene `-n`: con el flag puesto el fichero acababa
    // siendo "-n test-llm-api-token" y ningún Bearer del arnés casaba. `serve`
    // hace `trim()` del fichero, así que el `\n` de `echo` es inofensivo.
    let token_guion = format!("echo {TOKEN} > /tmp/soso-llm-api.token\nexit\n");
    ssh_guion(key, SSH_PORT, &token_guion, Duration::from_secs(120))
        .map_err(|e| format!("provisión del token guest: {e}"))?;
    let serve = ServeSession::start(key, catalog);
    esperar_proceso_serve(key, &serve)?;
    phases.record("start_serve", t);

    let client = ApiClient::new(LLM_PORT, TOKEN, catalog);
    let t = Instant::now();
    wait_health_ready(&client, qemu, key, &serve)?;
    phases.record("health_ready", t);

    let guest_lim = GuestHttpLimits::default();
    let t = Instant::now();
    steps.extend(run_core_invariants_guest(&client, guest_lim));
    phases.record("core_invariants", t);

    let t = Instant::now();
    steps.push(check_busy_while_generating_guest(&client, guest_lim));
    steps.push(check_health_during_generation_guest(&client, guest_lim));
    phases.record("busy_health", t);

    let t = Instant::now();
    // `ask` con el modelo real y `serve` vivo: dos generaciones de dos tokens,
    // ~200 s cada una en este guest (medido 2026-09-23).
    let ask_guion = "ask :max 2\nask hola\nexit\n";
    match ssh_guion(key, SSH_PORT, ask_guion, Duration::from_secs(900)) {
        Ok(out) if out.contains("ask:") || out.is_empty() => steps.push(StepResult {
            name: "ask_con_serve",
            ok: true,
            detail: String::new(),
        }),
        Ok(out) => steps.push(StepResult {
            name: "ask_con_serve",
            ok: false,
            detail: format!("salida inesperada: {out:?}"),
        }),
        Err(e) => steps.push(StepResult {
            name: "ask_con_serve",
            ok: false,
            detail: e,
        }),
    }
    phases.record("ask", t);

    let _ = qemu.kill();
    std::thread::sleep(Duration::from_secs(1));
    steps.push(check_server_down(LLM_PORT));

    if steps.iter().any(|s| !s.ok) {
        return Err("alguna invariante falló".into());
    }
    Ok(())
}

/// El servidor guest en su **propia** sesión SSH, en primer plano.
///
/// `sosh` no tiene control de trabajos: un `&` final no es un operador, es un
/// argumento más de `soso-llm`. El guion `… serve … &` + `exit` dejaba la
/// sesión colgada para siempre y la prueba moría con «la sesión SSH no terminó
/// en 180s» sin haber llegado a la API. Aquí la sesión se queda ocupada a
/// propósito y el resto de la prueba abre sesiones aparte.
struct ServeSession {
    fin: Arc<Mutex<Option<Result<String, String>>>>,
}

impl ServeSession {
    fn start(key: &Path, catalog: &str) -> Self {
        let fin = Arc::new(Mutex::new(None));
        let fin_hilo = fin.clone();
        let key = key.to_path_buf();
        // `>>` a un fichero sí lo entiende sosh; `2>&1` **no**: lo tokeniza
        // como redirección del fd 2 que exige un destino, se queda sin él y
        // rechaza la línea entera. Con el `2>&1` puesto, `serve` no llegaba a
        // ejecutarse y el guest se quedaba mudo en el prompt. El stderr se
        // queda en la sesión SSH, que `ServeSession` conserva.
        let guion = format!(
            "soso-llm serve --model {catalog} --port 7422 --token-file /tmp/soso-llm-api.token \
             >> /tmp/soso-llm-serve.log\n"
        );
        std::thread::spawn(move || {
            let r = ssh_guion(&key, SSH_PORT, &guion, SERVE_SESSION_LIMIT);
            *fin_hilo.lock().unwrap() = Some(r);
        });
        Self { fin }
    }

    /// `Some(motivo)` si la sesión de `serve` ya terminó: el servidor no va a
    /// contestar nunca y esperar los 180s de `/health` sólo retrasa el informe.
    fn murio(&self) -> Option<String> {
        match &*self.fin.lock().unwrap() {
            None => None,
            Some(Ok(salida)) => Some(format!("serve terminó solo; salida: {salida:?}")),
            Some(Err(e)) => Some(format!("sesión de serve caída: {e}")),
        }
    }
}

/// Comprueba que `serve` está **corriendo** antes de sondear la API.
///
/// Si `sosh` rechaza la línea —por un operador que no tiene— la sesión vuelve
/// al prompt en silencio: `ssh_guion` no termina, `murio()` sigue a `None` y
/// la espera de `/health` se come su plazo entero sin decir la causa. Un `ps`
/// desde otra sesión separa «no arrancó» de «está cargando el modelo».
fn esperar_proceso_serve(key: &Path, serve: &ServeSession) -> Result<(), String> {
    let fin = Instant::now() + Duration::from_secs(90);
    let mut ultimo = String::from("(sin `ps` todavía)");
    loop {
        if let Some(motivo) = serve.murio() {
            return Err(motivo);
        }
        if Instant::now() >= fin {
            return Err(format!(
                "soso-llm no aparece en `ps` del guest tras 90s (¿sosh rechazó la línea?); ps: {ultimo:?}"
            ));
        }
        match ssh_guion(key, SSH_PORT, "ps\nexit\n", Duration::from_secs(60)) {
            Ok(out) if out.contains("soso-llm") => return Ok(()),
            Ok(out) => ultimo = out,
            Err(e) => ultimo = format!("(ps falló: {e})"),
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

#[derive(PartialEq, Eq)]
enum HealthProbe {
    Ready,
    NotReady,
    EmptyAfterConnect,
}

fn probe_health(client: &ApiClient) -> HealthProbe {
    let connect = Duration::from_secs(5);
    let read = Duration::from_secs(2);
    match client.get_timeouts("/health", true, connect, read) {
        Ok(r) if String::from_utf8_lossy(&r).contains("200") => HealthProbe::Ready,
        Ok(r) if r.is_empty() => HealthProbe::EmptyAfterConnect,
        Ok(_) => HealthProbe::NotReady,
        Err(e) if looks_like_empty_http(&e) => HealthProbe::EmptyAfterConnect,
        Err(_) => HealthProbe::NotReady,
    }
}

fn looks_like_empty_http(err: &str) -> bool {
    err.contains("read:") || err.to_lowercase().contains("timed out")
}

/// El reenvío `hostfwd` de slirp **acepta siempre** la conexión al puerto del
/// host, escuche o no alguien en el guest. Por eso «TCP acepta pero no habla
/// HTTP» no prueba nada: es el estado normal mientras el modelo carga, y
/// abortar a las tres sondas mataba la prueba a los seis segundos. Las únicas
/// señales honestas de fracaso son que `serve` haya terminado, que QEMU haya
/// muerto o que se agote el plazo total.
fn wait_health_ready(
    client: &ApiClient,
    qemu: &mut Child,
    key: &Path,
    serve: &ServeSession,
) -> Result<(), String> {
    let limite = health_wait();
    let t0 = Instant::now();
    let mut ultimo_aviso = Instant::now();
    loop {
        if let Some(motivo) = serve.murio() {
            let log = guest_serve_log(key);
            return Err(format!("{motivo}; log guest: {log:?}"));
        }
        if probe_health(client) == HealthProbe::Ready {
            return Ok(());
        }
        if t0.elapsed() > limite {
            let log = guest_serve_log(key);
            return Err(format!(
                "timeout {}s esperando /health (log guest: {log:?})",
                limite.as_secs()
            ));
        }
        if !qemu_alive(qemu) {
            return Err("QEMU murió cargando serve".into());
        }
        if ultimo_aviso.elapsed() >= Duration::from_secs(30) {
            println!(
                "test-llm-api: esperando /health ({}s de {}s)",
                t0.elapsed().as_secs(),
                limite.as_secs()
            );
            ultimo_aviso = Instant::now();
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn health_wait() -> Duration {
    match std::env::var("SOSO_LLM_API_HEALTH_WAIT_S").ok().and_then(|s| s.parse().ok()) {
        Some(s) => Duration::from_secs(s),
        None => HEALTH_WAIT_DEFAULT,
    }
}

fn qemu_alive(qemu: &mut Child) -> bool {
    matches!(qemu.try_wait(), Ok(None))
}

/// Userspace listo cuando hay `sosh —` en serial o el eco TCP responde (no basta con SYN en :2299).
fn wait_guest_ready(key: &Path, serial: &Path, qemu: &mut Child) -> Result<(), String> {
    const BOOT_WAIT: Duration = Duration::from_secs(180);
    let fin = Instant::now() + BOOT_WAIT;
    while Instant::now() < fin {
        if !qemu_alive(qemu) {
            let tail = serial_tail(serial, 400);
            return Err(format!("QEMU terminó durante arranque; serial: {tail:?}"));
        }
        if guest_userspace_up(serial) {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if !guest_userspace_up(serial) {
        let tail = serial_tail(serial, 400);
        return Err(format!(
            "guest no listo en {}s (sin sosh en serial ni eco :{ECHO_PORT}); serial: {tail:?}",
            BOOT_WAIT.as_secs()
        ));
    }
    let ssh_deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < ssh_deadline {
        if !qemu_alive(qemu) {
            return Err("QEMU terminó esperando SSH".into());
        }
        if ssh_vive(key, SSH_PORT).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(3));
    }
    let tail = serial_tail(serial, 400);
    Err(format!(
        "SSH no respondió tras userspace (90s); serial: {tail:?}"
    ))
}

fn guest_userspace_up(serial: &Path) -> bool {
    if fs::read_to_string(serial)
        .map(|s| s.contains("sosh —"))
        .unwrap_or(false)
    {
        return true;
    }
    echo_vive(ECHO_PORT)
}

fn echo_vive(echo_port: u16) -> bool {
    let mut s = match format!("127.0.0.1:{echo_port}")
        .parse()
        .ok()
        .and_then(|a| TcpStream::connect_timeout(&a, Duration::from_secs(2)).ok())
    {
        Some(stream) => stream,
        None => return false,
    };
    s.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let msg = b"soso echo test\n";
    if s.write_all(msg).is_err() {
        return false;
    }
    let mut buf = vec![0u8; msg.len()];
    s.read_exact(&mut buf).is_ok() && buf == msg
}

fn serial_tail(serial: &Path, max_chars: usize) -> String {
    let s = fs::read_to_string(serial).unwrap_or_default();
    if s.is_empty() {
        return "(vacío)".into();
    }
    if s.len() <= max_chars {
        s
    } else {
        format!("…{}", &s[s.len() - max_chars..])
    }
}

fn llm_port_ocupado() -> bool {
    let addr = format!("127.0.0.1:{LLM_PORT}");
    addr.parse()
        .ok()
        .and_then(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).ok())
        .is_some()
}

/// `sosh` no tiene `||` ni `2>/dev/null`: el guion anterior sólo devolvía
/// «comando vacío en el pipeline» y el fallo se informaba sin su causa.
fn guest_serve_log(key: &Path) -> String {
    ssh_guion(
        key,
        SSH_PORT,
        // `log` primero: `serve` diagnostica con `logln!` (fd de log del
        // kernel), así que el fichero redirigido suele estar vacío o ni
        // existir, y el informe salía sin la única línea que importaba.
        "log\ncat /tmp/soso-llm-serve.log\nexit\n",
        Duration::from_secs(60),
    )
    .unwrap_or_else(|e| format!("(no leí log: {e})"))
}

/// Copia firmware con sufijo `uefi`/`bios` para que `apply_firmware` monte OVMF.
fn copiar_firmware(src: &Path) -> PathBuf {
    let root = crate::project_root();
    let kind = crate::firmware_kind(src);
    let dst = root.join(format!("target/test-llm-api-{kind}.img"));
    if needs_refresh(src, &dst) {
        crate::copy_sparse(src, &dst);
    } else {
        println!("test-llm-api: reutilizo {}", dst.display());
    }
    dst
}

fn copiar_si_necesario(src: &Path, tag: &str) -> PathBuf {
    let root = crate::project_root();
    let dst = root.join(format!("target/test-llm-api-{tag}.img"));
    if needs_refresh(src, &dst) {
        crate::copy_sparse(src, &dst);
    } else {
        println!("test-llm-api: reutilizo {}", dst.display());
    }
    dst
}

fn needs_refresh(src: &Path, dst: &Path) -> bool {
    if !dst.is_file() {
        return true;
    }
    match (
        src.metadata().and_then(|m| m.modified()).ok(),
        dst.metadata().and_then(|m| m.modified()).ok(),
    ) {
        (Some(s), Some(d)) => s > d,
        _ => true,
    }
}

fn write_evidence(
    dir: &Path,
    modo: &str,
    steps: &[StepResult],
    phases: &PhaseLog,
    code: i32,
) {
    let path = dir.join("resultado.md");
    let mut f = fs::File::create(&path).expect("resultado T19");
    let _ = writeln!(f, "# T19 — {modo}\n");
    let _ = writeln!(f, "exit_code: {code}\n");
    if !phases.entries.is_empty() {
        let _ = writeln!(f, "## Fases (s)\n");
        for (name, secs) in &phases.entries {
            let _ = writeln!(f, "- {name}: {secs:.1}");
        }
        let _ = writeln!(f);
    }
    for s in steps {
        let mark = if s.ok { "OK" } else { "FAIL" };
        let _ = writeln!(f, "- {mark} **{}** {}", s.name, s.detail);
    }
}

fn print_argv_only(args: &Args) {
    let llm = args
        .model_dir
        .as_ref()
        .map(|_| Some(LLM_PORT))
        .unwrap_or(None);
    let forwards = crate::llm_ports::SlirpForwards {
        echo_port: ECHO_PORT,
        ssh_port: SSH_PORT,
        llm_host_port: llm,
    };
    match crate::llm_ports::build_user_netdev(&forwards) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("{e}");
            exit(1);
        }
    }
}
