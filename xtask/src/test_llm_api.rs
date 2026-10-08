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
/// 10 casos × 3, cada una con el plazo de cliente de 10 min, más holgura.
/// Medido en T74: ~580 s por petición. Con 3600 s la sesión SSH mata
/// `soso-llm` hacia la sexta y el resto sale como pérdida de transporte.
const SERVE_SESSION_CAMPANA: Duration = Duration::from_secs(10 * 3 * 12 * 60);

struct Args {
    synthetic: bool,
    model_dir: Option<PathBuf>,
    profile: Option<PathBuf>,
    inspect_argv: bool,
    /// Correr la campaña de [T14] contra el endpoint vivo en vez de los
    /// invariantes de T19.
    ///
    /// **En vez de**, no además: son la evidencia de dos fichas distintas, y
    /// juntarlas da una tirada de hora y pico cuyo fallo podría venir de
    /// cualquiera de las dos. Los invariantes ya se corren solos.
    ///
    /// [T14]: ../../docs/self-improvement/T14-evaluacion-modelo.md
    campana: bool,
    repeticiones: Option<String>,
    caso: Option<String>,
    /// Programa a ejecutar con el endpoint vivo (T22: lanzar OpenCode contra
    /// la inferencia del guest). Implica `--campana`: es lo único que se corre.
    /// Recibe `SOSO_LLM_API_KEY`, `SOSO_LLM_PORT` y `SOSO_LLM_MODEL`.
    orden: Option<PathBuf>,
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

    let Some(model_dir) = args.model_dir.clone() else {
        uso();
        exit(1);
    };
    let Some(profile_path) = args.profile.clone() else {
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
    run_guest(&root, &arte, &model_dir, &catalog, &args);
}

fn parse(from: &[String]) -> Args {
    let mut synthetic = false;
    let mut model_dir = None;
    let mut profile = None;
    let mut inspect_argv = false;
    let mut campana = false;
    let mut repeticiones = None;
    let mut caso = None;
    let mut orden = None;
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
            "--campana" | "--campaña" => campana = true,
            "--repeticiones" => {
                i += 1;
                repeticiones = from.get(i).cloned();
            }
            "--caso" => {
                i += 1;
                caso = from.get(i).cloned();
            }
            "--orden" => {
                i += 1;
                orden = from.get(i).map(PathBuf::from);
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
        campana: campana || orden.is_some(),
        repeticiones,
        caso,
        orden,
    }
}

fn uso() {
    eprintln!("uso:");
    eprintln!("  cargo xtask test-llm-api --synthetic");
    eprintln!("  cargo xtask test-llm-api --model-dir <dir> --profile <model-lock.json>");
    eprintln!("  cargo xtask test-llm-api --inspect-argv …  (solo imprime forwards QEMU)");
    eprintln!();
    eprintln!("  --campana            corre la campaña de T14 contra el endpoint vivo");
    eprintln!("                       **en vez de** los invariantes de T19");
    eprintln!("  --repeticiones N     por caso (T14 usó 3; 1 para una pasada rápida)");
    eprintln!("  --caso Qxx           un solo caso, para no pagar la campaña entera");
    eprintln!("  --orden <script>     ejecuta el script con el endpoint vivo (SOSO_LLM_API_KEY,");
    eprintln!("                       SOSO_LLM_PORT, SOSO_LLM_MODEL) en vez de la campaña (T22)");
}

fn contexto_del_perfil(path: &Path) -> u32 {
    let Ok(data) = fs::read_to_string(path) else {
        return 32_768;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) else {
        return 32_768;
    };
    v.get("som")
        .and_then(|s| s.get("max_seq"))
        .and_then(|n| n.as_u64())
        .filter(|n| *n > 0 && *n <= u32::MAX as u64)
        .map(|n| n as u32)
        .unwrap_or(32_768)
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

fn run_guest(root: &Path, arte: &Path, model_dir: &Path, catalog: &str, args: &Args) {
    println!(
        "test-llm-api: guest modelo={catalog} dir={} puerto_llm={LLM_PORT}",
        model_dir.display()
    );
    if llm_port_ocupado() {
        eprintln!(
            "test-llm-api: BLOQUEADO — 127.0.0.1:{LLM_PORT} ya acepta TCP (¿QEMU huérfano?). \
             Mata `qemu-system-x86` con test-llm-api y reintenta."
        );
        let (dir, nombre) = destino_evidencia(root, arte, catalog, args.campana);
        write_evidence_en(&dir, &nombre, "bloqueado_puerto", &[], &PhaseLog::default(), 2);
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
        // 5120M cabe el 3B (2212 MiB). Un 7B (~4,8 GB) no: el presupuesto de
        // pesos es el 70 % de lo libre y, por debajo, cada token pagina.
        // `SOSO_QEMU_MEM` sube solo esta tirada; el 3B sigue en 5120M.
        mem: Some(std::env::var("SOSO_QEMU_MEM").unwrap_or_else(|_| "5120M".into())),
        smp: None,
        monitor: None,
        guest: crate::QemuGuestConfig::from_env(),
    };

    let t = Instant::now();
    let mut qemu = match lanzar_qemu(&slot) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("test-llm-api: QEMU no arrancó ({e})");
            let (dir, nombre) = destino_evidencia(root, arte, catalog, args.campana);
            write_evidence_en(&dir, &nombre, "qemu_fail", &[], &phases, 1);
            exit(1);
        }
    };
    phases.record("lanzar_qemu", t);

    let mut steps: Vec<StepResult> = Vec::new();
    let dir_t14 = root.join("target/self-improvement/tasks/T14");
    if args.campana {
        let _ = fs::create_dir_all(&dir_t14);
    }
    let contexto = args
        .profile
        .as_ref()
        .map(|p| contexto_del_perfil(p))
        .unwrap_or(32_768);
    let campana = args.campana.then(|| Campana {
        arte: &dir_t14,
        modelo: catalog,
        modelo_dir: model_dir,
        contexto,
        repeticiones: args.repeticiones.as_deref(),
        caso: args.caso.as_deref(),
        orden: args.orden.as_deref(),
    });
    let cierre = run_guest_inner(
        &key,
        &serial,
        catalog,
        &mut steps,
        &mut qemu,
        &mut phases,
        campana.as_ref(),
    );
    let (exit_code, modo) = match cierre {
        Ok(CierreGuest::Medido) => (0, "guest_ok"),
        Ok(CierreGuest::Parcial) => (0, "parcial"),
        Err(msg) => {
            eprintln!("test-llm-api: {msg}");
            steps.push(StepResult {
                name: "guest_run",
                ok: false,
                detail: msg,
            });
            (1, "guest_fail")
        }
    };

    if args.campana {
        let texto = guest_serve_log(&key);
        let _ = fs::write(dir_t14.join(format!("serve-{catalog}.log")), texto);
    }

    let _ = qemu.kill();
    let _ = qemu.wait();
    // La campaña no reescribe el informe de T19: cada modelo deja el suyo en T14.
    let (evid_dir, evid_nombre) = destino_evidencia(root, arte, catalog, args.campana);
    write_evidence_en(
        &evid_dir,
        &evid_nombre,
        modo,
        &steps,
        &phases,
        exit_code,
    );
    exit(exit_code);
}

enum CierreGuest {
    /// Campaña entera medida, sea GO o NO-GO de calidad.
    Medido,
    /// Subconjunto. El proceso puede salir 0; el rótulo no es `guest_ok`.
    Parcial,
}

/// Qué campaña correr, si es que hay que correr alguna.
struct Campana<'a> {
    arte: &'a Path,
    modelo: &'a str,
    modelo_dir: &'a Path,
    contexto: u32,
    repeticiones: Option<&'a str>,
    caso: Option<&'a str>,
    orden: Option<&'a Path>,
}

/// Informe de una campaña. El nombre lleva el modelo: el 7B no puede
/// reescribir el informe del 3B, ni el canónico compartido `campana-t14.json`.
fn ruta_informe_campana(
    dir: &Path,
    modelo: &str,
    caso: Option<&str>,
    repeticiones: Option<&str>,
) -> PathBuf {
    let seguro: String = modelo
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect();
    match caso {
        Some(caso) => {
            let reps = repeticiones.unwrap_or("3");
            dir.join(format!("campana-t14-parcial-{seguro}-{caso}-x{reps}.json"))
        }
        None => dir.join(format!("campana-t14-{seguro}.json")),
    }
}

/// Lanza `soso-improve evaluar` contra el endpoint reenviado.
///
/// Un **NO-GO no es un fallo del arnés**: la campaña se hizo y el resultado es
/// que no. `evaluar` ya sale con el código de verificación (no el de error)
/// justo por eso, así que aquí se distingue «no pude medir» de «medí y salió
/// que no», y sólo lo primero marca el paso como malo.
fn correr_campana(catalog: &str, c: &Campana<'_>) -> StepResult {
    if let Some(orden) = c.orden {
        return correr_orden(catalog, orden);
    }
    let root = crate::project_root();
    let banco = root.join("tests/self-improvement/cases");
    // **Una campaña filtrada no es la campaña.** `evaluar` marca
    // `cobertura: filtrada` y no da GO. El informe tampoco usa el nombre
    // canónico: lleva el modelo, y el aviso va también por pantalla.
    let parcial = c.caso.is_some() || c.repeticiones.is_some();
    let informe = ruta_informe_campana(c.arte, c.modelo, c.caso, c.repeticiones);
    let puerto = LLM_PORT.to_string();

    let mut cmd = Command::new("cargo");
    cmd.current_dir(&root).args([
        "run",
        "-q",
        "-p",
        "soso-improve",
        "--",
        "evaluar",
        "--modelo",
        catalog,
        "--banco",
        banco.to_str().unwrap_or_default(),
        "--puerto",
        &puerto,
        "--token",
        TOKEN,
        "--out",
        informe.to_str().unwrap_or_default(),
    ]);
    if let Some(r) = c.repeticiones {
        cmd.args(["--repeticiones", r]);
    }
    if let Some(caso) = c.caso {
        cmd.args(["--caso", caso]);
    }
    let contexto = c.contexto.to_string();
    let modelo_dir = c.modelo_dir.display().to_string();
    cmd.args([
        "--contexto",
        &contexto,
        "--modelo-dir",
        &modelo_dir,
    ]);

    println!("test-llm-api: campaña T14 contra el endpoint vivo (puerto {puerto})");
    if parcial {
        println!(
            "test-llm-api: AVISO — tirada **parcial**: su veredicto no es el de la campaña"
        );
    }
    match cmd.status() {
        // 0 = GO. El código de verificación es un NO-GO medido, que **también**
        // es un resultado: el paso pasa y el veredicto está en el informe.
        Ok(st) if st.success() => StepResult {
            name: "campana_t14",
            ok: true,
            detail: if parcial {
                format!("parcial — informe en {}", informe.display())
            } else {
                format!("GO — informe en {}", informe.display())
            },
        },
        Ok(st) if st.code() == Some(soso_improve_core::cli::Codigo::Verificacion.como_i32()) => {
            StepResult {
                name: "campana_t14",
                ok: true,
                detail: if parcial {
                    format!("parcial — informe en {}", informe.display())
                } else {
                    format!("NO-GO medido — informe en {}", informe.display())
                },
            }
        }
        Ok(st) => StepResult {
            name: "campana_t14",
            ok: false,
            detail: format!("evaluar salió con {:?}: no se pudo medir", st.code()),
        },
        Err(e) => StepResult {
            name: "campana_t14",
            ok: false,
            detail: format!("no pude lanzar soso-improve: {e}"),
        },
    }
}

/// Ejecuta un programa externo con el endpoint del guest vivo y deja su código
/// de salida como veredicto del paso. El arnés sólo presta el endpoint: qué se
/// hace con él (T22: OpenCode) y cómo se juzga es cosa del programa.
fn correr_orden(catalog: &str, orden: &Path) -> StepResult {
    let root = crate::project_root();
    println!(
        "test-llm-api: orden {} contra el endpoint vivo (puerto {LLM_PORT})",
        orden.display()
    );
    let st = Command::new("bash")
        .arg(orden)
        .current_dir(&root)
        .env("SOSO_LLM_API_KEY", TOKEN)
        .env("SOSO_LLM_PORT", LLM_PORT.to_string())
        .env("SOSO_LLM_MODEL", catalog)
        .status();
    match st {
        Ok(st) if st.success() => StepResult {
            name: "orden",
            ok: true,
            detail: format!("{} salió con 0", orden.display()),
        },
        Ok(st) => StepResult {
            name: "orden",
            ok: false,
            detail: format!("{} salió con {:?}", orden.display(), st.code()),
        },
        Err(e) => StepResult {
            name: "orden",
            ok: false,
            detail: format!("no pude lanzar {}: {e}", orden.display()),
        },
    }
}

fn run_guest_inner(
    key: &Path,
    serial: &Path,
    catalog: &str,
    steps: &mut Vec<StepResult>,
    qemu: &mut Child,
    phases: &mut PhaseLog,
    campana: Option<&Campana<'_>>,
) -> Result<CierreGuest, String> {
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
    let limite_serve = if campana.is_some() {
        SERVE_SESSION_CAMPANA
    } else {
        SERVE_SESSION_LIMIT
    };
    let serve = ServeSession::start(key, catalog, limite_serve);
    esperar_proceso_serve(key, &serve)?;
    phases.record("start_serve", t);

    let client = ApiClient::new(LLM_PORT, TOKEN, catalog);
    let t = Instant::now();
    wait_health_ready(&client, qemu, key, &serve)?;
    phases.record("health_ready", t);

    // Con `--campana` el endpoint ya está vivo y es lo único que la campaña
    // necesita: se corre y se vuelve, sin los invariantes de T19.
    if let Some(c) = campana {
        let t = Instant::now();
        let paso = correr_campana(catalog, c);
        let parcial = c.caso.is_some() || c.repeticiones.is_some();
        let malo = !paso.ok;
        let detalle = paso.detail.clone();
        steps.push(paso);
        phases.record("campana_t14", t);
        if malo {
            return Err(detalle);
        }
        return Ok(if parcial {
            CierreGuest::Parcial
        } else {
            CierreGuest::Medido
        });
    }

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
    // El veredicto de este paso era `out.contains("ask:") || out.is_empty()`, y
    // eso **no puede fallar por lo que el paso existe**: de los seis mensajes
    // `ask:` de `user/soso-llm/src/ask.rs`, **cuatro son errores** («no pude
    // lanzar», «el servicio no escuchó», «error al enviar», «no hay ningún
    // modelo») y los otros dos son el banner del REPL, que `ask hola` no
    // imprime. Una respuesta de verdad **no** lleva `ask:`. Así que el paso
    // aprobaba con cualquier error y con la salida vacía, y sólo podía suspender
    // cuando funcionaba.
    //
    // Ahora exige lo que dice su nombre: que `ask` conteste con `serve` vivo.
    let errores_ask = [
        "no pude lanzar",
        "el servicio no escuchó",
        "error al enviar",
        "no hay ningún modelo",
    ];
    match ssh_guion(key, SSH_PORT, ask_guion, Duration::from_secs(900)) {
        Ok(out) if out.trim().is_empty() => steps.push(StepResult {
            name: "ask_con_serve",
            ok: false,
            detail: "sin salida: `ask` no contestó nada".into(),
        }),
        Ok(out) if errores_ask.iter().any(|e| out.contains(e)) => steps.push(StepResult {
            name: "ask_con_serve",
            ok: false,
            detail: format!("`ask` informó de un error: {out:?}"),
        }),
        Ok(out) => {
            // El guion son dos órdenes: `ask :max 2` y `ask hola`. La primera
            // contesta `ask: máx 2 tokens` —el eco de una configuración— y eso
            // **ya bastaba** para el veredicto viejo, que por eso pasaba sin
            // mirar nunca si la segunda generó algo. Aquí se descuentan las
            // líneas que no son una respuesta y se exige que quede texto.
            let respuesta: String = out
                .lines()
                .map(str::trim)
                .filter(|l| {
                    // Todo esto lo imprime el sistema, no el modelo: el
                    // banner de bienvenida, las marcas de `sosh` y los ecos de
                    // configuración de `ask`. Quitarlo es el punto: con el
                    // filtro incompleto, **el banner de login bastaba para
                    // aprobar** —comprobado quitando la generación del guion,
                    // y el paso seguía en verde con 206 bytes de bienvenida—.
                    //
                    // Se filtran **todas** las líneas `ask:`, no una lista de
                    // ellas: las seis que existen son del sistema —errores,
                    // banner, configuración y progreso—, y la respuesta del
                    // modelo no lleva ese prefijo. Y los puntos de progreso,
                    // que si no dejarían pasar una generación que arranca y se
                    // cuelga: eso probaría «empezó», no «contestó».
                    const DEL_SISTEMA: &[&str] = &[
                        "bienvenido a soso",
                        "un OS minimalista",
                        "desde live, instalar",
                        "sosh —",
                        "sosh:",
                        "ask:",
                        "$",
                        "?>",
                    ];
                    !l.is_empty()
                        && !l.chars().all(|c| c == '.')
                        && !DEL_SISTEMA.iter().any(|p| l.starts_with(p))
                })
                .collect::<Vec<_>>()
                .join("\n");
            if respuesta.is_empty() {
                steps.push(StepResult {
                    name: "ask_con_serve",
                    ok: false,
                    detail: format!("sólo ecos de configuración, ninguna respuesta: {out:?}"),
                });
            } else {
                steps.push(StepResult {
                    name: "ask_con_serve",
                    ok: true,
                    detail: format!("contestó {} bytes: {respuesta:?}", respuesta.len()),
                });
            }
        }
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
    Ok(CierreGuest::Medido)
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
    fn start(key: &Path, catalog: &str, limite: Duration) -> Self {
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
            let r = ssh_guion(&key, SSH_PORT, &guion, limite);
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
        "log\ncat /tmp/soso-llm-serve.log\nps\nexit\n",
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

/// La campaña escribe en T14, con el nombre del modelo. El resto sigue en T19.
fn destino_evidencia(root: &Path, arte: &Path, catalog: &str, campana: bool) -> (PathBuf, String) {
    if campana {
        let dir = root.join("target/self-improvement/tasks/T14");
        let _ = fs::create_dir_all(&dir);
        (dir, format!("resultado-{catalog}.md"))
    } else {
        (arte.to_path_buf(), String::from("resultado.md"))
    }
}

fn write_evidence(
    dir: &Path,
    modo: &str,
    steps: &[StepResult],
    phases: &PhaseLog,
    code: i32,
) {
    write_evidence_en(dir, "resultado.md", modo, steps, phases, code);
}

fn write_evidence_en(
    dir: &Path,
    nombre: &str,
    modo: &str,
    steps: &[StepResult],
    phases: &PhaseLog,
    code: i32,
) {
    let path = dir.join(nombre);
    let mut f = fs::File::create(&path).expect("resultado T19");
    let titulo = if nombre == "resultado.md" { "T19" } else { "T14" };
    let _ = writeln!(f, "# {titulo} — {modo}\n");
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

#[cfg(test)]
mod pruebas_campana {
    use super::ruta_informe_campana;
    use std::path::Path;

    #[test]
    fn dos_modelos_no_comparten_el_informe() {
        let dir = Path::new("target/self-improvement/tasks/T14");
        let a = ruta_informe_campana(dir, "qwen2.5-coder-3b", None, None);
        let b = ruta_informe_campana(dir, "qwen2.5-coder-7b", None, None);
        assert_ne!(a, b);
        assert!(a.ends_with("campana-t14-qwen2.5-coder-3b.json"));
        assert!(b.ends_with("campana-t14-qwen2.5-coder-7b.json"));
        let parcial = ruta_informe_campana(dir, "qwen2.5-coder-7b", Some("Q05"), Some("1"));
        assert_ne!(parcial, b);
        assert!(parcial.to_string_lossy().contains("parcial"));
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
