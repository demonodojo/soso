//! `cargo xtask test`: batería de integración de soso.
//!
//! 1. Crash-safety de sosofs en el host (`cargo test -p sosofs`).
//! 2. Arranque en QEMU hasta la shell.
//! 3. Echo TCP en el puerto 7 (hostfwd 7777).
//! 4. Sesión SSH autenticada por clave, ejecuta un comando y apaga con
//!    `halt` (que a su vez comprueba el apagado limpio).
//!
//! Los pasos de guest se reparten en shards QEMU independientes (puertos e
//! imágenes propios) limitados por `SOSO_TEST_JOBS` (default 4 con KVM, 2 en
//! TCG). QEMU usa `-accel kvm` si `/dev/kvm` es legible (`SOSO_QEMU_ACCEL`).
//!
//! Sale con código 0 si todo pasa, 1 si algo falla.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// QEMU sale con (code<<1)|1; ExitCode::Success = 0x10 -> 33.
const HALT_EXIT: i32 = 33;

/// RAM del shard `reclaim`. Tiene que ser **lo más baja que arranque**: la
/// gracia del shard es que la presión de memoria dispare el reclaim de páginas.
/// Estuvo en 48M hasta que el kernel creció y el bootloader dejó de poder mapear
/// la memoria física (`FrameAllocationFailed` antes siquiera de arrancar), así
/// que si vuelve a fallar así, súbela — pero lo justo, y comprueba con
/// `memoria: N MiB libres tras el heap` en el log de serie que sigue habiendo
/// presión.
///
/// Medido con el kernel de 2026-08-16: 48M ni arranca (el bootloader no puede
/// mapear la memoria física), 64M arranca y deja 15 MiB libres, 72M deja 19 y
/// 96M deja 31. **Con 72M la inferencia muere a media generación** —pasó una
/// vez y falló a la siguiente, así que es inestable, no marginal— y un gate que
/// falla a veces no vale para nada. 96M es el valor que aguanta.
const RECLAIM_MEM: &str = "96M";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShardId {
    LlmDense,
    LlmMoe,
    Sys,
    Reclaim,
}

impl ShardId {
    const ALL: [ShardId; 4] = [
        ShardId::LlmDense,
        ShardId::LlmMoe,
        ShardId::Sys,
        ShardId::Reclaim,
    ];

    fn name(self) -> &'static str {
        match self {
            ShardId::LlmDense => "llm-dense",
            ShardId::LlmMoe => "llm-moe",
            ShardId::Sys => "sys",
            ShardId::Reclaim => "reclaim",
        }
    }

    fn index(self) -> u8 {
        match self {
            ShardId::LlmDense => 0,
            ShardId::LlmMoe => 1,
            ShardId::Sys => 2,
            ShardId::Reclaim => 3,
        }
    }

    fn ssh_port(self) -> u16 {
        2200 + 10 * u16::from(self.index())
    }

    fn echo_port(self) -> u16 {
        7700 + 10 * u16::from(self.index())
    }

    fn mac(self) -> String {
        format!("52:54:00:12:34:{:02x}", 0x20 + self.index())
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "llm-dense" | "dense" => Some(ShardId::LlmDense),
            "llm-moe" | "moe" => Some(ShardId::LlmMoe),
            "sys" => Some(ShardId::Sys),
            "reclaim" => Some(ShardId::Reclaim),
            _ => None,
        }
    }
}

/// Filtros opcionales tras `cargo xtask test -- …`.
///
/// Ejemplos:
///   `cargo xtask test -- --guest sys`
///   `cargo xtask test -- sys --only init,voz,ciclos`
#[derive(Clone)]
struct TestFilter {
    shards: Vec<ShardId>,
    skip_host: bool,
    only: Vec<String>,
}

impl TestFilter {
    fn from_args(args: &[String]) -> Self {
        let mut shards = Vec::new();
        let mut skip_host = false;
        let mut only = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--" => {}
                "--guest" | "--no-host" => skip_host = true,
                "--only" => {
                    i += 1;
                    if i < args.len() {
                        only.extend(parse_only_list(&args[i]));
                    }
                }
                s if s.starts_with("--only=") => {
                    only.extend(parse_only_list(&s["--only=".len()..]));
                }
                s => {
                    if let Some(shard) = ShardId::parse(s) {
                        shards.push(shard);
                    } else if s.starts_with('-') {
                        eprintln!("xtask test: aviso: opción desconocida {s:?}");
                    } else {
                        eprintln!("xtask test: aviso: shard desconocido {s:?} (sys, llm-dense, llm-moe, reclaim)");
                    }
                }
            }
            i += 1;
        }
        Self {
            shards,
            skip_host,
            only,
        }
    }

    fn shards(&self) -> &[ShardId] {
        if self.shards.is_empty() {
            ShardId::ALL.as_slice()
        } else {
            &self.shards
        }
    }

    fn step_enabled(&self, nombre: &str) -> bool {
        if self.only.is_empty() {
            return true;
        }
        let n = nombre.to_lowercase();
        self.only.iter().any(|f| n.contains(f))
    }

    fn if_step<F: FnOnce()>(&self, sid: &str, nombre: &str, f: F) {
        if !self.step_enabled(nombre) {
            println!("      [{sid}] omitido: {nombre}");
            return;
        }
        f();
    }
}

fn parse_only_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .collect()
}

struct QemuSlot {
    id: &'static str,
    ssh_port: u16,
    echo_port: u16,
    mac: String,
    serial: PathBuf,
    bios: PathBuf,
    data: PathBuf,
    models: PathBuf,
    mem: Option<String>,
    smp: Option<String>,
    /// Socket UNIX del monitor QEMU (`sendkey` tras I/O USB).
    monitor: Option<PathBuf>,
    guest: super::QemuGuestConfig,
}

struct Report {
    fallos: Mutex<u32>,
}

struct JobPool {
    available: Mutex<usize>,
    cvar: Condvar,
}

struct JobPermit<'a> {
    pool: &'a JobPool,
}

impl Report {
    fn new() -> Self {
        Self {
            fallos: Mutex::new(0),
        }
    }

    fn fallos(&self) -> u32 {
        *self.fallos.lock().unwrap()
    }

    fn marca(&self, shard: &str, nombre: &str, ok: bool) {
        println!(
            "{}  [{shard}] {nombre}",
            if ok { "OK  " } else { "FALLO" }
        );
    }

    fn paso<F: FnOnce() -> Result<(), String>>(
        &self,
        shard: &str,
        nombre: &str,
        f: F,
    ) -> Result<(), ()> {
        match f() {
            Ok(()) => {
                self.marca(shard, nombre, true);
                Ok(())
            }
            Err(e) => {
                self.marca(shard, &format!("{nombre}: {e}"), false);
                *self.fallos.lock().unwrap() += 1;
                Err(())
            }
        }
    }

    fn paso_con_reintento<F: FnMut() -> Result<(), String>>(
        &self,
        shard: &str,
        nombre: &str,
        mut f: F,
    ) -> Result<(), ()> {
        const MAX: u32 = 3;
        for intento in 1..=MAX {
            match f() {
                Ok(()) => {
                    self.marca(shard, nombre, true);
                    return Ok(());
                }
                Err(e) if intento < MAX => {
                    println!(
                        "      [{shard}] (reintento {}/{} de «{nombre}» tras 5 s: {e})",
                        intento,
                        MAX - 1
                    );
                    std::thread::sleep(Duration::from_secs(5));
                }
                Err(e) => {
                    self.marca(shard, &format!("{nombre}: {e}"), false);
                    *self.fallos.lock().unwrap() += 1;
                    return Err(());
                }
            }
        }
        unreachable!()
    }

    /// Paso SSH del shard `sys` con reinicio del guest si la sesión queda colgada
    /// (p. ej. `soso-voz` sigue corriendo tras un timeout del cliente).
    fn paso_ssh_sys<F: FnMut() -> Result<(), String>>(
        &self,
        qemu: &mut Child,
        slot: &QemuSlot,
        sid: &str,
        nombre: &str,
        mut f: F,
    ) {
        const MAX: u32 = 3;
        for intento in 1..=MAX {
            match f() {
                Ok(()) => {
                    self.marca(sid, nombre, true);
                    return;
                }
                Err(e) if intento < MAX => {
                    let reinicio = guest_requiere_reinicio(&e);
                    println!(
                        "      [{sid}] (reintento {}/{} de «{nombre}»{}: {e})",
                        intento,
                        MAX - 1,
                        if reinicio { " tras reinicio" } else { " tras 5 s" },
                    );
                    if reinicio {
                        if let Err(re) = reiniciar_guest_sys(qemu, slot) {
                            self.marca(sid, &format!("{nombre}: {re}"), false);
                            *self.fallos.lock().unwrap() += 1;
                            return;
                        }
                    } else {
                        std::thread::sleep(Duration::from_secs(5));
                    }
                }
                Err(e) => {
                    self.marca(sid, &format!("{nombre}: {e}"), false);
                    *self.fallos.lock().unwrap() += 1;
                    return;
                }
            }
        }
    }
}

impl JobPool {
    fn new(max: usize) -> Self {
        Self {
            available: Mutex::new(max),
            cvar: Condvar::new(),
        }
    }

    fn acquire(&self) -> JobPermit<'_> {
        let mut avail = self.available.lock().unwrap();
        while *avail == 0 {
            avail = self.cvar.wait(avail).unwrap();
        }
        *avail -= 1;
        JobPermit { pool: self }
    }
}

impl Drop for JobPermit<'_> {
    fn drop(&mut self) {
        let mut avail = self.pool.available.lock().unwrap();
        *avail += 1;
        self.pool.cvar.notify_one();
    }
}

fn test_jobs() -> usize {
    std::env::var("SOSO_TEST_JOBS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(super::test_jobs_default)
        .clamp(1, 4)
}

pub fn run() {
    run_with_filter(&TestFilter::from_args(
        &std::env::args().skip(2).collect::<Vec<_>>(),
    ));
}

fn run_with_filter(filter: &TestFilter) {
    let root = super::project_root();
    let report = Arc::new(Report::new());

    let img = std::thread::scope(|scope| {
        if !filter.skip_host {
            let report_host = Arc::clone(&report);
            let root_host = root.clone();
            scope.spawn(move || run_host_tests(&root_host, &report_host));
        } else {
            println!("xtask test: omitiendo tests host (--guest)");
        }

        scope.spawn(super::build_user);

        let kernel = scope.spawn(super::build_image);

        kernel.join().unwrap()
    });

    let data = super::mkfs_rootfs(true);
    let models = super::mkfs_models(true);
    let key = root.join("target/soso_test_key");

    let jobs = test_jobs();
    let accel = super::qemu_accel_mode();
    println!("xtask test: QEMU accel={accel} (SOSO_QEMU_ACCEL)");
    println!("xtask test: hasta {jobs} QEMU en paralelo (SOSO_TEST_JOBS)");
    let shard_names: Vec<_> = filter.shards().iter().map(|s| s.name()).collect();
    println!("xtask test: shards={}", shard_names.join(", "));
    if !filter.only.is_empty() {
        println!("xtask test: --only {}", filter.only.join(", "));
    }

    run_shards_parallel(&report, jobs, &img, &data, &models, &key, filter);

    exit_resumen(if report.fallos() == 0 { 0 } else { 1 });
}

fn run_host_tests(root: &Path, report: &Arc<Report>) {
    std::thread::scope(|scope| {
        let root_a = root.to_path_buf();
        let report_a = Arc::clone(report);
        scope.spawn(move || {
            run_cargo_test_batch(
                &root_a,
                &report_a,
                "host (sosofs+sosomfs+soso-llm-core)",
                &["sosofs", "sosomfs", "soso-llm-core"],
                true,
            );
        });

        let root_b = root.to_path_buf();
        let report_b = Arc::clone(report);
        scope.spawn(move || {
            run_cargo_test_batch(
                &root_b,
                &report_b,
                "host (gptdisk+soso-http+soso-web-core+sosomodel+convert-gguf+cuda-proxy)",
                &[
                    "gptdisk",
                    "soso-http",
                    "soso-web-core",
                    "sosomodel",
                    "convert-gguf",
                    "cuda-proxy",
                ],
                false,
            );
        });

        let root_c = root.to_path_buf();
        let report_c = Arc::clone(report);
        scope.spawn(move || {
            run_cargo_test_batch(
                &root_c,
                &report_c,
                "host (soso-update-core)",
                &["soso-update-core"],
                true,
            );
        });

        let root_d = root.to_path_buf();
        let report_d = Arc::clone(report);
        scope.spawn(move || {
            run_cargo_test_batch(
                &root_d,
                &report_d,
                "host (soso-audio+gguf2som)",
                &["soso-audio", "gguf2som"],
                true,
            );
        });
    });
}

fn run_cargo_test_batch(
    root: &Path,
    report: &Arc<Report>,
    nombre: &str,
    pkgs: &[&str],
    con_std: bool,
) {
    let result = (|| {
        let mut cmd = Command::new("cargo");
        cmd.current_dir(root).args(["test", "-q"]);
        for pkg in pkgs {
            cmd.args(["-p", pkg]);
        }
        if con_std {
            cmd.args(["--features", "std"]);
        }
        let st = cmd.status().map_err(|e| e.to_string())?;
        if st.success() {
            Ok(())
        } else {
            Err(format!("los tests de {nombre} fallaron"))
        }
    })();
    match result {
        Ok(()) => report.marca("host", nombre, true),
        Err(e) => {
            report.marca("host", &format!("{nombre}: {e}"), false);
            *report.fallos.lock().unwrap() += 1;
        }
    }
}

fn run_shards_parallel(
    report: &Arc<Report>,
    jobs: usize,
    img: &Path,
    data: &Path,
    models: &Path,
    key: &Path,
    filter: &TestFilter,
) {
    let pool = Arc::new(JobPool::new(jobs));
    std::thread::scope(|scope| {
        for &shard in filter.shards() {
            let report = Arc::clone(report);
            let pool = Arc::clone(&pool);
            let img = img.to_path_buf();
            let data = data.to_path_buf();
            let models = models.to_path_buf();
            let key = key.to_path_buf();
            let filter = filter.clone();
            scope.spawn(move || {
                let _permit = pool.acquire();
                let slot = make_slot(shard, &img, &data, &models);
                run_shard(shard, &slot, &key, &report, &filter);
            });
        }
    });
}

fn make_slot(shard: ShardId, img: &Path, data: &Path, models: &Path) -> QemuSlot {
    let id = shard.name();
    let root = super::project_root();
    let log_dir = root.join("target");
    let _ = std::fs::create_dir_all(&log_dir);
    let canonical = log_dir.join(format!("test-{id}-serial.log"));
    let serial = if std::fs::remove_file(&canonical).is_ok() || !canonical.exists() {
        canonical
    } else {
        // Tras `sudo cargo xtask flash-usb-live` el log puede quedar root:root
        // y QEMU no escribe; el test ve un fichero viejo o vacío.
        eprintln!(
            "xtask test: aviso: no pude borrar {} (¿root?); uso log alternativo",
            canonical.display()
        );
        log_dir.join(format!("test-{id}-serial-{}.log", std::process::id()))
    };
    QemuSlot {
        id,
        ssh_port: shard.ssh_port(),
        echo_port: shard.echo_port(),
        mac: shard.mac(),
        serial,
        bios: copiar_imagen(img, id, "bios"),
        data: copiar_imagen(data, id, "data"),
        models: copiar_imagen(models, id, "models"),
        mem: if shard == ShardId::Reclaim {
            Some(RECLAIM_MEM.into())
        } else {
            None
        },
        // El shard denso corre con DOS cores a propósito: es el único sitio
        // donde `ThreadPool` llega a crear workers (`ncpu - 1`), y con un solo
        // core ese código no se ejecuta jamás. Con 2 basta —un worker -- y no
        // carga el host como 4. Así se coló que el pool
        // no apagaba sus hilos al morir: giraban al 100 % para siempre y en
        // placa de 8 cores la segunda inferencia dejaba la máquina inservible.
        smp: match shard {
            ShardId::Reclaim => Some("1".into()),
            // smp 2 para ssh_llm (ThreadPool); askd en smp 2 se cuelga en TCG
            // (generate no vuelve). Los pools se prueban en ssh_llm; ask va a 1
            // hasta arreglar la contención (2026-09-01).
            ShardId::LlmDense => Some("1".into()),
            _ => None,
        },
        monitor: None,
        guest: super::QemuGuestConfig::from_env(),
    }
}

fn copiar_imagen(src: &Path, shard_id: &str, kind: &str) -> PathBuf {
    let dst = super::project_root()
        .join("target")
        .join(format!("test-{shard_id}-{kind}.img"));
    super::copy_sparse(src, &dst);
    dst
}

fn run_shard(shard: ShardId, slot: &QemuSlot, key: &Path, report: &Report, filter: &TestFilter) {
    match shard {
        ShardId::LlmDense => run_shard_llm_dense(slot, key, report, filter),
        ShardId::LlmMoe => run_shard_llm_moe(slot, key, report),
        ShardId::Sys => run_shard_sys(slot, key, report, filter),
        ShardId::Reclaim => run_shard_reclaim(slot, key, report),
    }
}

fn run_shard_llm_dense(slot: &QemuSlot, key: &Path, report: &Report, filter: &TestFilter) {
    let sid = slot.id;
    let qemu = match lanzar_qemu(slot) {
        Ok(c) => c,
        Err(e) => {
            report.marca(sid, &format!("lanzar QEMU: {e}"), false);
            *report.fallos.lock().unwrap() += 1;
            return;
        }
    };
    let _vivo = QemuVivo(qemu);
    let arrancado = report
        .paso(sid, "arranque hasta la shell", || {
            esperar_en_fichero(&slot.serial, "sosh —", Duration::from_secs(180))?;
            esperar_en_fichero(&slot.serial, "net: dhcp", Duration::from_secs(120))
        })
        .is_ok();
    if arrancado {
        let port = slot.ssh_port;
        filter.if_step(sid, "soso-llm run tiny --prompt test", || {
            let _ = report.paso_con_reintento(sid, "soso-llm run tiny --prompt test", || {
                ssh_llm(key, port)
            });
        });
        filter.if_step(
            sid,
            "soso-llm run tiny-mla --prompt test --max 2",
            || {
                let _ = report.paso_con_reintento(
                    sid,
                    "soso-llm run tiny-mla --prompt test --max 2",
                    || ssh_llm_mla(key, port),
                );
            },
        );
        filter.if_step(sid, "sigue viva tras dos pools de hilos", || {
            let _ = report.paso_con_reintento(sid, "sigue viva tras dos pools de hilos", || {
                ssh_vive(key, port)
            });
        });
        filter.if_step(
            sid,
            "ask: modelo residente (carga una vez, reconexión SSH)",
            || {
                let _ = report.paso_con_reintento(
                    sid,
                    "ask: modelo residente (carga una vez, reconexión SSH)",
                    || ssh_ask_resident(key, port),
                );
            },
        );
        filter.if_step(
            sid,
            "soso-llm: 20 ciclos carga/generación/cambio (A7)",
            || {
                let _ = report.paso_con_reintento(
                    sid,
                    "soso-llm: 20 ciclos carga/generación/cambio (A7)",
                    || ssh_llm_ciclos(key, port),
                );
            },
        );
    }
}

/// Un comando trivial que debe contestar rápido: detecta la máquina ahogada
/// por hilos que quedaron girando.
fn ssh_vive(key: &Path, ssh_port: u16) -> Result<(), String> {
    let texto = ssh_guion(key, ssh_port, "echo vivo\nexit\n", Duration::from_secs(45))?;
    if texto.contains("vivo") {
        Ok(())
    } else {
        Err(format!("sin respuesta al echo; stdout: {texto:?}"))
    }
}

fn run_shard_llm_moe(slot: &QemuSlot, key: &Path, report: &Report) {
    let sid = slot.id;
    let qemu = match lanzar_qemu(slot) {
        Ok(c) => c,
        Err(e) => {
            report.marca(sid, &format!("lanzar QEMU: {e}"), false);
            *report.fallos.lock().unwrap() += 1;
            return;
        }
    };
    let _vivo = QemuVivo(qemu);
    let arrancado = report
        .paso(sid, "arranque hasta la shell", || {
            esperar_en_fichero(&slot.serial, "sosh —", Duration::from_secs(90))
        })
        .is_ok();
    if arrancado {
        let port = slot.ssh_port;
        let _ = report.paso_con_reintento(
            sid,
            "soso-llm run tiny-moe --prompt @bos --max 2",
            || ssh_llm_moe(key, port),
        );
        let _ = report.paso_con_reintento(
            sid,
            "soso-llm run tiny-latent-moe --prompt @bos --max 2",
            || ssh_llm_latent_moe(key, port),
        );
        let _ = report.paso_con_reintento(
            sid,
            "tiny-q4k: mismos tokens con y sin dispositivo (subida en crudo)",
            || ssh_llm_q4k(key, port),
        );
    }
}

fn run_shard_sys(slot: &QemuSlot, key: &Path, report: &Report, filter: &TestFilter) {
    let sid = slot.id;
    let mut qemu = match lanzar_qemu(slot) {
        Ok(c) => c,
        Err(e) => {
            report.marca(sid, &format!("lanzar QEMU: {e}"), false);
            *report.fallos.lock().unwrap() += 1;
            return;
        }
    };
    let arrancado = report
        .paso(sid, "arranque hasta la shell", || {
            esperar_en_fichero(&slot.serial, "sosh —", Duration::from_secs(90))?;
            esperar_en_fichero(&slot.serial, "net: dhcp", Duration::from_secs(120))
        })
        .is_ok();
    if arrancado {
        let echo = slot.echo_port;
        let port = slot.ssh_port;
        let _ = report.paso(sid, &format!("echo TCP en :{echo}"), || echo_tcp(echo));
        let _ = report.paso_ssh_sys(&mut qemu, slot, sid, "marca /tmp/sosh-ready", || {
            ssh_sosh_ready(key, port)
        });
        filter.if_step(sid, "ask: el texto llega literal", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "ask: el texto llega literal", || {
                ssh_ask_literal(key, port)
            });
        });
        filter.if_step(sid, "soso-web: HTML local", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "soso-web: HTML local", || {
                ssh_soso_web_local(key, port)
            });
        });
        filter.if_step(
            sid,
            "init test (syscalls, hilos, FPU, GPU)",
            || {
                report.paso_ssh_sys(
                    &mut qemu,
                    slot,
                    sid,
                    "init test (syscalls, hilos, FPU, GPU)",
                    || ssh_init_test(key, port),
                );
            },
        );
        filter.if_step(sid, "forja: hola-std remoto", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "forja: hola-std remoto", || {
                ssh_forja_hola(key, port)
            });
        });
        filter.if_step(sid, "pipeline de sosh (6 KiB por un pipe)", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "pipeline de sosh (6 KiB por un pipe)", || {
                ssh_pipeline(key, port)
            });
        });
        filter.if_step(sid, "voz: transcribe WAV de prueba", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "voz: transcribe WAV de prueba", || {
                ssh_voz_wav(key, port)
            });
        });
        filter.if_step(
            sid,
            "SSH por clave pública + comando + halt",
            || {
                report.paso_ssh_sys(
                    &mut qemu,
                    slot,
                    sid,
                    "SSH por clave pública + comando + halt",
                    || ssh_sesion(key, port),
                );
            },
        );
    }
    let halt_esperado = filter.step_enabled("SSH por clave pública + comando + halt");
    if halt_esperado {
        match espera_salida(&mut qemu, Duration::from_secs(10)) {
            Some(code) if code == HALT_EXIT => {
                report.marca(sid, "apagado limpio por halt", true);
            }
            Some(code) => {
                report.marca(sid, &format!("QEMU salió con código inesperado {code}"), false);
                *report.fallos.lock().unwrap() += 1;
            }
            None => {
                report.marca(sid, "QEMU no se apagó con halt (matado)", false);
                *report.fallos.lock().unwrap() += 1;
                let _ = qemu.kill();
            }
        }
    } else {
        let _ = qemu.kill();
    }
    let _ = qemu.wait();
}

fn guest_ssh_caido(err: &str) -> bool {
    err.contains("no apareció el prompt")
        || err.contains("stdout parcial: \"\"")
        || err.contains("ConnectTimeout")
        || err.contains("Connection refused")
        || err.contains("Connection reset")
}

fn guest_requiere_reinicio(err: &str) -> bool {
    guest_ssh_caido(err) || err.contains("la sesión SSH no terminó")
}

fn reiniciar_guest_sys(qemu: &mut Child, slot: &QemuSlot) -> Result<(), String> {
    let _ = qemu.kill();
    let _ = qemu.wait();
    let _ = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&slot.serial);
    *qemu = lanzar_qemu(slot).map_err(|e| format!("relanzar QEMU: {e}"))?;
    esperar_en_fichero(&slot.serial, "sosh —", Duration::from_secs(90))?;
    esperar_en_fichero(&slot.serial, "net: dhcp", Duration::from_secs(120))?;
    Ok(())
}

fn run_shard_reclaim(slot: &QemuSlot, key: &Path, report: &Report) {
    let sid = slot.id;
    let qemu = match lanzar_qemu(slot) {
        Ok(c) => c,
        Err(e) => {
            report.marca(sid, &format!("lanzar QEMU: {e}"), false);
            *report.fallos.lock().unwrap() += 1;
            return;
        }
    };
    let _vivo = QemuVivo(qemu);
    let port = slot.ssh_port;
    let etiqueta = format!("soso-llm con RAM {RECLAIM_MEM} (reclaim)");
    let _ = report.paso(sid, &etiqueta, || {
        esperar_en_fichero(&slot.serial, "sosh —", Duration::from_secs(300))?;
        let texto = ssh_guion(
            key,
            port,
            "soso-llm run tiny --prompt x --max 2\nexit\n",
            Duration::from_secs(420),
        )?;
        if !texto.contains("soso-llm: generado") {
            return Err(format!(
                "inferencia con {RECLAIM_MEM} no completó; stdout: {texto:?}"
            ));
        }
        if !texto.contains("soso-llm: planificador") {
            return Err(format!(
                "falta salida del planificador con {RECLAIM_MEM}; stdout: {texto:?}"
            ));
        }
        Ok(())
    });
}


fn exit_resumen(code: i32) -> ! {
    if code == 0 {
        println!("\n✅ cargo xtask test: TODO OK");
    } else {
        println!("\n❌ cargo xtask test: hubo fallos");
    }
    std::process::exit(code);
}

/// Si un intento anterior murió a medias (timeout SSH, kill del terminal…),
/// QEMU y ssh siguen vivos y el siguiente arranque no escribe en el serial.
fn limpiar_huérfanos_shard(slot: &QemuSlot) {
    let id = slot.id;
    let puerto = slot.ssh_port;
    let _ = Command::new("sh")
        .args([
            "-c",
            &format!(
                "pkill -f 'qemu-system-x86_64.*test-{id}' 2>/dev/null; \
                 pkill -f 'ssh.*-p {puerto} ' 2>/dev/null; \
                 true"
            ),
        ])
        .status();
    std::thread::sleep(Duration::from_millis(800));
    let _ = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&slot.serial);
}

fn lanzar_qemu(slot: &QemuSlot) -> std::io::Result<Child> {
    limpiar_huérfanos_shard(slot);
    let mem = slot.mem.clone().unwrap_or_else(super::qemu_mem);
    let smp = slot.smp.clone().unwrap_or_else(super::qemu_smp);
    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35", "-cpu", "max"])
        .args(["-m", &mem])
        .args(["-smp", &smp]);
    super::apply_qemu_accel(&mut qemu);
    super::apply_firmware(&mut qemu, &slot.bios);
    qemu.args([
        "-drive",
        &format!("format=raw,file={}", slot.bios.display()),
    ]);
    super::apply_qemu_disks(&mut qemu, &slot.data, &slot.models, &slot.guest);
    super::apply_qemu_usb(&mut qemu, &slot.guest);
    super::apply_qemu_nic_with_ports(&mut qemu, slot.ssh_port, slot.echo_port, Some(&slot.mac));
    super::apply_qemu_gpu(&mut qemu);
    if let Some(mon) = &slot.monitor {
        let _ = std::fs::remove_file(mon);
        qemu.args([
            "-monitor",
            &format!("unix:{},server,nowait", mon.display()),
        ]);
    }
    qemu.args(["-serial", &format!("file:{}", slot.serial.display())])
        .args(["-display", "none"])
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .arg("-no-reboot")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

/// Arranque con puertos por defecto (test-usb, lx-e1000e smoke).
fn lanzar_qemu_legacy(
    img: &Path,
    data: &Path,
    models: &Path,
    serial: &Path,
) -> std::io::Result<Child> {
    lanzar_qemu_legacy_ports(img, data, models, serial, 2222, 7777, "52:54:00:12:34:15")
}

fn lanzar_qemu_legacy_ports(
    img: &Path,
    data: &Path,
    models: &Path,
    serial: &Path,
    ssh_port: u16,
    echo_port: u16,
    mac: &str,
) -> std::io::Result<Child> {
    let slot = QemuSlot {
        id: "legacy",
        ssh_port,
        echo_port,
        mac: mac.into(),
        serial: serial.to_path_buf(),
        bios: img.to_path_buf(),
        data: data.to_path_buf(),
        models: models.to_path_buf(),
        mem: None,
        smp: None,
        monitor: None,
        guest: super::QemuGuestConfig::from_env(),
    };
    lanzar_qemu(&slot)
}

fn qemu_monitor_session(mon: &Path) -> Result<std::os::unix::net::UnixStream, String> {
    use std::os::unix::net::UnixStream;

    for _ in 0..100 {
        if mon.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut s = UnixStream::connect(mon).map_err(|e| format!("monitor {mon:?}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_millis(400))).ok();
    s.set_write_timeout(Some(Duration::from_secs(2))).ok();
    let mut banner = [0u8; 512];
    let _ = s.read(&mut banner);
    Ok(s)
}

fn qemu_monitor_cmd(
    s: &mut std::os::unix::net::UnixStream,
    cmd: &str,
) -> Result<(), String> {
    s.write_all(format!("{cmd}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 256];
    let _ = s.read(&mut buf);
    Ok(())
}

/// Tras I/O del BOT en xHCI, el teclado (PS/2 vía `sendkey`, HID en el bus)
/// debe seguir llegando al sosh de la serie. Una conexión de monitor por
/// tecla dejaba el socket HMP a medias y QEMU tragaba el resto.
fn usb_kbd_vivo_despues_io(
    key: &Path,
    ssh_port: u16,
    serial: &Path,
    monitor: &Path,
) -> Result<(), String> {
    ssh_guion(key, ssh_port, "ls /models\nexit\n", Duration::from_secs(120))?;
    std::thread::sleep(Duration::from_secs(2));
    let mut mon = qemu_monitor_session(monitor)?;
    qemu_monitor_cmd(&mut mon, "sendkey ret")?;
    std::thread::sleep(Duration::from_millis(200));
    for k in ["h", "e", "l", "p", "ret"] {
        qemu_monitor_cmd(&mut mon, &format!("sendkey {k}"))?;
        std::thread::sleep(Duration::from_millis(200));
    }
    esperar_en_fichero(serial, "builtins:", Duration::from_secs(45))
}

/// Espera a que aparezca `patron` en el fichero de serie.
pub(crate) fn esperar_en_fichero(
    path: &std::path::Path,
    patron: &str,
    limite: Duration,
) -> Result<(), String> {
    let fin = Instant::now() + limite;
    while Instant::now() < fin {
        if let Ok(s) = std::fs::read_to_string(path)
            && s.contains(patron)
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(format!("no apareció {patron:?} en {limite:?}"))
}

fn echo_tcp(echo_port: u16) -> Result<(), String> {
    let mut s = conectar_reintentando(echo_port, Duration::from_secs(10))?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let msg = b"soso echo test\n";
    s.write_all(msg).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; msg.len()];
    s.read_exact(&mut buf).map_err(|e| e.to_string())?;
    if buf == msg { Ok(()) } else { Err(format!("eco distinto: {buf:?}")) }
}

fn conectar_reintentando(puerto: u16, limite: Duration) -> Result<TcpStream, String> {
    let fin = Instant::now() + limite;
    loop {
        match TcpStream::connect(("127.0.0.1", puerto)) {
            Ok(s) => return Ok(s),
            Err(_) if Instant::now() < fin => {
                std::thread::sleep(Duration::from_millis(200))
            }
            Err(e) => return Err(format!("no conecta a :{puerto}: {e}")),
        }
    }
}

fn prompt_listo(texto: &str) -> bool {
    if !texto.contains("sosh — escribe 'help' para la ayuda") {
        return false;
    }
    texto.ends_with("$ ")
        || texto.ends_with("$ \n")
        || texto.ends_with("$ \r\n")
        || texto.contains("\n$ \n")
        || texto.contains("\r\n$ \r\n")
        || texto.contains("\n$ \r\n")
}

/// Quita el aviso de cierre de OpenSSH y normaliza saltos de línea.
fn normalizar_salida_ssh(texto: String) -> String {
    let mut s = texto.replace("\r\n", "\n");
    for marker in [
        "\nConnection to localhost closed by remote host.",
        "Connection to localhost closed by remote host.",
    ] {
        if let Some(i) = s.find(marker) {
            s.truncate(i);
            break;
        }
    }
    s.trim_end().to_string()
}

/// Abre una sesión SSH, le pasa `guion` por stdin y espera a que **termine sola**,
/// como máximo `limite`. Devuelve el stdout de la sesión.
///
/// Antes cada prueba dormía un tiempo fijo (150 + 45 + 20 + 4 = 219 s de reloj en
/// total) porque no había señal de fin en la que esperar: sunset espejaba el
/// CHANNEL_EOF del cliente y OpenSSH cerraba su salida al recibirlo, así que una
/// sesión con stdin cerrado no devolvía nada y el arnés se apoyaba en dormir "de
/// sobra". Con el parche de `vendor/sunset-0.5.0` la shell cierra el canal al
/// ejecutar `exit` y el cliente sale, así que se espera al proceso y no al reloj:
/// el límite pasa a ser una red de seguridad y la suite tarda lo que tarde el
/// guest. Por eso los límites de abajo son holgados — ya no se pagan.
///
/// stdin se mantiene abierto con un FIFO: un `sleep` de fondo evita mandar EOF
/// al servidor antes de tiempo (mismo truco que `scripts/l6-g1-vfio-test.sh`).
/// Se espera al prompt antes de escribir el guion: mandarlo antes de que sosh
/// esté leyendo lo pierde en el arranque. La sesión termina cuando la shell
/// hace `exit`, no cuando el cliente cierra stdin.
pub(crate) fn ssh_guion(
    key: &Path,
    ssh_port: u16,
    guion: &str,
    limite: Duration,
) -> Result<String, String> {
    ssh_guion_inner(key, ssh_port, guion, limite, true, None)
}

/// Como `ssh_guion`, pero si aparece `marcador` se da por buena la sesión
/// aunque `halt` no cierre SSH (el guest se apaga y el cliente se cuelga).
pub(crate) fn ssh_guion_hasta(
    key: &Path,
    ssh_port: u16,
    guion: &str,
    limite: Duration,
    marcador: &str,
) -> Result<String, String> {
    ssh_guion_inner(key, ssh_port, guion, limite, true, Some(marcador))
}

fn ssh_guion_inner(
    key: &Path,
    ssh_port: u16,
    guion: &str,
    limite: Duration,
    tty: bool,
    marcador_ok: Option<&str>,
) -> Result<String, String> {
    let mut traza = SshTrace::new("guion");
    traza.ev(&format!("inicio: guion de {} B, límite {:?}", guion.len(), limite));
    let fifo = ssh_fifo_path("guion");
    let _ = fs::remove_file(&fifo);
    if !Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .map_err(|e| format!("mkfifo: {e}"))?
        .success()
    {
        return Err("mkfifo falló".into());
    }

    let holgura = limite.as_secs().saturating_add(60);
    let holder = Command::new("sh")
        .args(["-c", &format!("exec sleep {holgura} > {}", fifo.display())])
        .spawn()
        .map_err(|e| format!("sleep holder: {e}"))?;
    let mut guard = SshSessionGuard::new(fifo.clone(), holder);

    let key_s = key.display().to_string();
    let fifo_s = fifo.display().to_string();
    let modo_tty = if tty { "-tt" } else { "-T" };
    let ssh_cmd = format!(
        "exec ssh {modo_tty} -i '{key_s}' -p {ssh_port} \
         -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
         -o LogLevel=ERROR -o ConnectTimeout=10 soso@localhost < '{fifo_s}'"
    );
    let mut hijo = Command::new("sh")
        .args(["-c", &ssh_cmd])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("no se pudo lanzar ssh: {e}"))?;

    guard.track_ssh(hijo.id());
    let stdout = hijo.stdout.take().unwrap();
    let stderr = hijo.stderr.take();

    let acum = Arc::new(Mutex::new(Vec::new()));
    let prompt = Arc::new((Mutex::new(false), Condvar::new()));
    let acum_hilo = acum.clone();
    let prompt_hilo = prompt.clone();
    let lector = std::thread::spawn(move || -> Result<(), String> {
        let mut stdout = stdout;
        let mut buf = [0u8; 512];
        loop {
            let n = stdout
                .read(&mut buf)
                .map_err(|e| format!("leyendo stdout SSH: {e}"))?;
            if n == 0 {
                break;
            }
            let mut datos = acum_hilo.lock().unwrap();
            datos.extend_from_slice(&buf[..n]);
            let texto = String::from_utf8_lossy(&datos);
            let mut visto = prompt_hilo.0.lock().unwrap();
            if !*visto && prompt_listo(&texto) {
                *visto = true;
                prompt_hilo.1.notify_all();
            }
        }
        Ok(())
    });

    let stderr_acum = Arc::new(Mutex::new(Vec::new()));
    if let Some(stderr) = stderr {
        let stderr_acum_hilo = stderr_acum.clone();
        std::thread::spawn(move || {
            let mut stderr = stderr;
            let mut buf = [0u8; 512];
            loop {
                match stderr.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => stderr_acum_hilo.lock().unwrap().extend_from_slice(&buf[..n]),
                    Err(_) => break,
                }
            }
        });
    }

    let fin_prompt = Instant::now() + Duration::from_secs(45);
    loop {
        let mut visto = prompt.0.lock().unwrap();
        if *visto {
            break;
        }
        let resto = fin_prompt.saturating_duration_since(Instant::now());
        if resto.is_zero() {
            return Err(format!(
                "no apareció el prompt en 45s; stdout parcial: {:?}",
                String::from_utf8_lossy(&acum.lock().unwrap())
            ));
        }
        visto = prompt.1.wait_timeout(visto, resto).unwrap().0;
    }

    traza.ev(&format!(
        "prompt visto; parcial: {:?}",
        String::from_utf8_lossy(&acum.lock().unwrap())
    ));
    // El motd y el banner pueden llegar en el mismo read que el `$ `; un instante
    // de margen evita perder la primera línea del guion (p. ej. `ask :eco`).
    std::thread::sleep(Duration::from_millis(250));

    {
        let mut w = OpenOptions::new()
            .write(true)
            .open(&fifo)
            .map_err(|e| format!("escribir fifo: {e}"))?;
        w.write_all(guion.as_bytes()).map_err(|e| e.to_string())?;
        w.flush().ok();
    }
    traza.ev("guion escrito en la fifo");

    let fin = Instant::now() + limite;
    loop {
        if lector.is_finished() {
            break;
        }
        if let Some(m) = marcador_ok {
            let datos = acum.lock().unwrap();
            if String::from_utf8_lossy(&datos).contains(m) {
                drop(datos);
                std::thread::sleep(Duration::from_secs(3));
                if lector.is_finished() {
                    break;
                }
                let _ = hijo.kill();
                break;
            }
        }
        if Instant::now() >= fin {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let timed_out = Instant::now() >= fin;
    traza.ev(&format!(
        "fin del bucle (timeout={timed_out}); acumulado: {:?}",
        String::from_utf8_lossy(&acum.lock().unwrap())
    ));
    // Matar ssh ANTES de join del lector: si no, read() en stdout bloquea
    // para siempre y el timeout no sirve de nada (init test, voz, A7…).
    if timed_out && !lector.is_finished() {
        let _ = hijo.kill();
    }
    if lector.is_finished() {
        lector
            .join()
            .map_err(|_| "hilo lector ssh".to_string())?
            .map_err(|e: String| e)?;
    } else {
        let _ = lector.join();
    }
    let mut salida = String::from_utf8_lossy(&acum.lock().unwrap()).into_owned();
    salida.push_str(&String::from_utf8_lossy(&stderr_acum.lock().unwrap()));
    let salida = normalizar_salida_ssh(salida);
    let status = hijo.wait().map_err(|e| format!("wait ssh: {e}"))?;
    guard.finish();
    if timed_out && !status.success() {
        if marcador_ok.is_some_and(|m| salida.contains(m)) {
            return Ok(salida);
        }
        return Err(format!(
            "la sesión SSH no terminó en {}s; stdout: {salida:?}",
            limite.as_secs()
        ));
    }
    Ok(salida)
}

fn ssh_sosh_ready(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "cat /tmp/sosh-ready\nexit\n",
        Duration::from_secs(60),
    )?;
    if !salida.contains("pid=") {
        return Err(format!("sin marca pid= en /tmp/sosh-ready: {salida:?}"));
    }
    Ok(())
}

fn ssh_llm(key: &Path, ssh_port: u16) -> Result<(), String> {
    // Dos inferencias en la misma sesión: la de CPU y la que pasa por el camino de
    // syscalls GPU con el dispositivo software del kernel. La segunda es la única
    // cobertura que tiene ese camino sin tarjeta — alloc/map/submit/read, el
    // cacheo de pesos y el crecimiento de búferes entre capas de distinto tamaño.
    //
    // El límite (antes un sleep de 150 s) hay que dejarlo holgado por SMP: con el
    // scheduler multicore real cada syscall compite más por PROCS.lock() con los
    // cores ociosos sondeando, y user/libsoso hace ~15000 syscalls sbrk (una por
    // asignación pequeña, sin agrupar) incluso para el modelo sintético diminuto
    // de este test. El arreglo de fondo sería agrupar sbrk en un arena local.
    //
    // Y **no cuentes con el contador de «generado»** para dimensionarlo: sólo mide
    // la generación. En una máquina sin KVM (QEMU en TCG) las dos inferencias
    // suman 106 s de generación y la sesión entera tarda 269 s — el resto se lo
    // llevan el arranque de `soso-llm` (abrir el modelo, planificar, mapear los
    // shards) y el cierre, dos veces. Con 240 s el paso vencía por 30 s, mataba la
    // sesión SSH dejando al guest masticando, y a partir de ahí TODOS los pasos
    // siguientes salían en rojo con stdout vacío: un timeout ajustado no falla
    // solo, se lleva la suite por delante (medido el 2026-08-01).
    let texto = ssh_guion(
        key,
        ssh_port,
        "soso-llm run tiny --prompt test\n                  soso-llm run tiny --prompt test --gpu-soft --max 4\n                  exit\n",
        Duration::from_secs(600),
    )?;
    if !texto.contains("soso-llm: generado") {
        return Err(format!(
            "soso-llm no generó salida esperada; stdout: {texto:?}"
        ));
    }
    // El coste de disco de la carga en frío, a la vista aunque el paso pase.
    // Es gratis (ya se ha ejecutado) y es la única forma de que una regresión de
    // E/S se note sin volver a instrumentar el kernel a mano.
    for l in texto
        .lines()
        .filter(|l| l.contains("soso-llm: disco") || l.contains("soso-llm: carga en frío"))
    {
        println!("      {}", l.trim());
    }
    if !texto.contains("soso-llm: planificador") {
        return Err(format!(
            "soso-llm no mostró el planificador de recursos; stdout: {texto:?}"
        ));
    }
    if !texto.contains("dispositivo «soft") {
        return Err(format!(
            "--gpu-soft no enganchó el dispositivo software; stdout: {texto:?}"
        ));
    }
    // Lo que de verdad se comprueba: que los pesos se suban UNA vez por matriz y
    // no en cada matvec. Sin el cacheo, subidas == matvec, y con 4 tokens eso son
    // decenas de subidas de la matriz entera que en el log no se distinguían de
    // nada. Se exige subidas < matvec y que el resumen aparezca.
    let linea = texto
        .lines()
        .find(|l| l.contains("matvec,") && l.contains("subidas de pesos"))
        .ok_or_else(|| format!("falta el resumen del dispositivo; stdout: {texto:?}"))?;
    let num = |tras: &str| -> Option<usize> {
        let idx = linea.find(tras)?;
        linea[..idx]
            .split_whitespace()
            .next_back()
            .and_then(|t| t.parse().ok())
    };
    let calls = num("matvec,").ok_or_else(|| format!("no se pudo leer matvec: {linea:?}"))?;
    let uploads = num("subidas").ok_or_else(|| format!("no se pudo leer subidas: {linea:?}"))?;
    if calls == 0 {
        return Err(format!("el dispositivo no recibió ningún matvec: {linea:?}"));
    }
    if uploads >= calls {
        return Err(format!(
            "los pesos se resuben en cada matvec ({uploads} subidas / {calls} matvec):              el cacheo no está funcionando — {linea:?}"
        ));
    }
    Ok(())
}

/// A7: veinte ciclos de carga/generación con salida limpia del proceso y askd.
///
/// Cada cuarto ciclo pasa por `ask` (askd residente); a mitad forzamos `:modelo
/// tiny` para ejercitar recarga. Al final la shell debe seguir respondiendo.
const LLM_CICLOS_A7: usize = 20;

fn ssh_llm_ciclos(key: &Path, ssh_port: u16) -> Result<(), String> {
    let mut guion = String::from("ask :max 1\n");
    for i in 0..LLM_CICLOS_A7 {
        if i == LLM_CICLOS_A7 / 2 {
            guion.push_str("ask :modelo tiny\n");
        }
        if i % 4 == 3 {
            guion.push_str("ask x\n");
        } else {
            guion.push_str("soso-llm run tiny --prompt x --max 1\n");
        }
    }
    guion.push_str("echo ciclos_ok\nexit\n");
    let texto = ssh_guion_inner(
        key,
        ssh_port,
        &guion,
        Duration::from_secs(1200),
        true,
        Some("ciclos_ok"),
    )?;
    let runs = texto.matches("soso-llm: generado").count();
    let asks = LLM_CICLOS_A7 / 4;
    let expected_run = LLM_CICLOS_A7 - asks;
    if runs < expected_run {
        return Err(format!(
            "esperaba al menos {expected_run} líneas «generado», vi {runs}; stdout: {texto:?}"
        ));
    }
    if !texto.contains("ciclos_ok") {
        return Err(format!(
            "la shell no respondió tras {LLM_CICLOS_A7} ciclos; stdout: {texto:?}"
        ));
    }
    Ok(())
}

fn ssh_llm_moe(key: &Path, ssh_port: u16) -> Result<(), String> {
    let texto = ssh_guion(
        key,
        ssh_port,
        "soso-llm run tiny-moe --prompt @bos --max 2\nexit\n",
        Duration::from_secs(120),
    )?;
    if !texto.contains("soso-llm: generado") {
        return Err(format!(
            "soso-llm tiny-moe no generó salida esperada; stdout: {texto:?}"
        ));
    }
    if !texto.contains("moe_hits") && !texto.contains("moe_misses") {
        // El planificador puede omitir stats MoE si no hay planner; al menos debe inferir.
        if !texto.contains("soso-llm: planificador") {
            return Err(format!(
                "soso-llm tiny-moe sin planificador; stdout: {texto:?}"
            ));
        }
    }
    Ok(())
}

fn ssh_llm_mla(key: &Path, ssh_port: u16) -> Result<(), String> {
    let texto = ssh_guion(
        key,
        ssh_port,
        "soso-llm run tiny-mla --prompt test --max 2\n                  soso-llm run tiny-mla --prompt test --gpu-soft --max 2\n                  exit\n",
        Duration::from_secs(600),
    )?;
    if !texto.contains("soso-llm: generado") {
        return Err(format!(
            "soso-llm tiny-mla no generó salida esperada; stdout: {texto:?}"
        ));
    }
    if !texto.contains("dispositivo «soft") {
        return Err(format!(
            "--gpu-soft no enganchó el dispositivo software en tiny-mla; stdout: {texto:?}"
        ));
    }
    if let Some(linea) = texto
        .lines()
        .find(|l| l.contains("matvec,") && l.contains("subidas de pesos"))
    {
        let num = |tras: &str| -> Option<usize> {
            let idx = linea.find(tras)?;
            linea[..idx]
                .split_whitespace()
                .next_back()
                .and_then(|t| t.parse().ok())
        };
        if let (Some(calls), Some(uploads)) = (num("matvec,"), num("subidas")) {
            if calls > 0 && uploads >= calls {
                return Err(format!(
                    "tiny-mla gpu-soft: pesos resubidos en cada matvec ({uploads}/{calls}): {linea:?}"
                ));
            }
        }
    }
    Ok(())
}

/// Modelo Q4_K por los DOS caminos: CPU y dispositivo. Es lo único que ejercita en
/// QEMU la subida de pesos **en crudo** y el comando `MATVQ`, y lo que exige es lo
/// que de verdad importa: **los mismos tokens**. Un `row_bytes` mal calculado, un
/// nibble cruzado o una escala de 6 bits mal desempaquetada dan números finitos y
/// texto plausible; sólo la comparación los caza.
///
/// Antes de esto no había ni un modelo cuantizado en la imagen de QEMU, así que todo
/// el camino Q4_K sólo se ejercitaba con un modelo real de gigabytes — que es por lo
/// que el offload pudo estar meses sin admitir cuantizados sin que salte nada.
fn ssh_llm_q4k(key: &Path, ssh_port: u16) -> Result<(), String> {
    let texto = ssh_guion(
        key,
        ssh_port,
        "soso-llm run tiny-q4k --prompt test --max 3\n                  soso-llm run tiny-q4k --prompt test --gpu-soft --max 3\n                  exit\n",
        Duration::from_secs(600),
    )?;
    let lineas: Vec<&str> = texto.lines().collect();
    // El TEXTO generado, no la línea de tok/s: ésa lleva milisegundos y nunca
    // coincidiría. `soso-llm` lo emite en streaming y lo cierra con un salto, así
    // que es la línea justo antes del resumen.
    let salidas: Vec<&str> = lineas
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains("soso-llm: generado"))
        .map(|(i, _)| if i > 0 { lineas[i - 1].trim() } else { "" })
        .collect();
    if salidas.len() < 2 {
        return Err(format!(
            "tiny-q4k: faltan las dos ejecuciones ({} vistas); stdout: {texto:?}",
            salidas.len()
        ));
    }
    if !texto.contains("dispositivo «soft") {
        return Err(format!(
            "--gpu-soft no enganchó el dispositivo software en tiny-q4k; stdout: {texto:?}"
        ));
    }
    let cpu = salidas[salidas.len() - 2];
    let dev = salidas[salidas.len() - 1];
    if cpu != dev {
        return Err(format!(
            "tiny-q4k: el dispositivo da otros tokens que la CPU\n  cpu: {cpu:?}\n  dev: {dev:?}"
        ));
    }
    // Que las subidas hayan ido EN CRUDO. Sin esto el test pasa igual por el camino
    // del plano f32 —da los mismos tokens— y se pierden el 8× de VRAM y de PCIe sin
    // que nada se ponga rojo: la misma forma del bug que dejó el DMA sin usar tres
    // meses. `tiny-q4k` tiene hidden 256 y ffn 512, los dos múltiplos de 256, así
    // que TODAS las matrices cuantizadas tienen que entrar por ahí.
    let crudas = texto
        .lines()
        .find(|l| l.contains("en crudo (sin expandir"))
        .ok_or_else(|| format!("tiny-q4k: falta la línea de subidas; stdout: {texto:?}"))?;
    if crudas.contains(" 0 de ") {
        return Err(format!(
            "tiny-q4k: ninguna subida fue en crudo — se está descuantizando a f32: {crudas:?}"
        ));
    }

    // Y que los pesos no se resuban por matvec ni haya desalojo: con Q4_K en crudo
    // el modelo entero cabe de sobra en el dispositivo de pruebas.
    if texto.contains("desalojos de pesos") {
        return Err(format!(
            "tiny-q4k: hubo desalojo de pesos con un modelo de 2 capas; stdout: {texto:?}"
        ));
    }
    if let Some(linea) = texto
        .lines()
        .find(|l| l.contains("matvec,") && l.contains("subidas de pesos"))
    {
        let num = |tras: &str| -> Option<usize> {
            let idx = linea.find(tras)?;
            linea[..idx]
                .split_whitespace()
                .next_back()
                .and_then(|t| t.parse().ok())
        };
        if let (Some(calls), Some(uploads)) = (num("matvec,"), num("subidas")) {
            if calls > 0 && uploads >= calls {
                return Err(format!(
                    "tiny-q4k: pesos resubidos en cada matvec ({uploads}/{calls}): {linea:?}"
                ));
            }
        }
    }
    Ok(())
}

fn ssh_llm_latent_moe(key: &Path, ssh_port: u16) -> Result<(), String> {
    let texto = ssh_guion(
        key,
        ssh_port,
        "soso-llm run tiny-latent-moe --prompt @bos --max 2\nexit\n",
        Duration::from_secs(120),
    )?;
    if !texto.contains("soso-llm: generado") {
        return Err(format!(
            "soso-llm tiny-latent-moe no generó salida esperada; stdout: {texto:?}"
        ));
    }
    Ok(())
}

/// B3: editar `hola-std` en el guest, sync+build en host, aplicar el ELF y ejecutarlo.
fn ssh_forja_hola(key: &Path, ssh_port: u16) -> Result<(), String> {
    let root = super::project_root();
    let work = root.join("target/forja-work-test");
    let _ = fs::remove_dir_all(&work);
    seed_forja_hola_work(&root, &work)?;
    let bin = build_forja_server(&root)?;
    ssh_forja_hola_run(key, ssh_port, &root, &work, &bin)
}

fn ssh_forja_hola_run(
    key: &Path,
    ssh_port: u16,
    root: &Path,
    work: &Path,
    bin: &Path,
) -> Result<(), String> {
    const MSG: &str = "hola-astra-B3-guest";
    let mut srv = Command::new(bin)
        .env("SOSO_FORJA_RELEASE", "hola-std")
        .env("SOSO_FORJA_BIND", "0.0.0.0:8740")
        .env("SOSO_FORJA_TOKEN", "soso-b3")
        .env("SOSO_FORJA_WORK", work)
        .current_dir(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("forja-server: {e}"))?;
    if let Err(e) = esperar_tcp("127.0.0.1:8740", Duration::from_secs(15)) {
        let _ = srv.kill();
        return Err(e);
    }
    let guion = format!(
        "soso-forja write-hola --msg {MSG}\n\
         soso-forja all --host 10.0.2.2 --token soso-b3\n\
         /bin/hola-std\n\
         exit\n"
    );
    let texto = ssh_guion(key, ssh_port, &guion, Duration::from_secs(180));
    let _ = srv.kill();
    let _ = srv.wait();
    let texto = texto?;
    if !texto.contains("forja: sync OK") {
        return Err(format!("forja sync no OK: {texto}"));
    }
    if !texto.contains("forja: aplicado /bin/hola-std") {
        return Err(format!("forja apply no OK: {texto}"));
    }
    if !texto.contains(MSG) {
        return Err(format!("hola-std no mostró {MSG}: {texto}"));
    }
    Ok(())
}

fn build_forja_server(root: &Path) -> Result<PathBuf, String> {
    let status = Command::new("cargo")
        .current_dir(root)
        .args(["build", "-q", "-p", "soso-forja-server"])
        .status()
        .map_err(|e| format!("cargo build forja-server: {e}"))?;
    if !status.success() {
        return Err("cargo build -p soso-forja-server falló".into());
    }
    let dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let bin = dir.join("debug/soso-forja-server");
    if !bin.is_file() {
        return Err(format!("no está {}", bin.display()));
    }
    Ok(bin)
}

fn seed_forja_hola_work(root: &Path, work: &Path) -> Result<(), String> {
    fs::create_dir_all(work.join("user")).map_err(|e| e.to_string())?;
    fs::create_dir_all(work.join("crates")).map_err(|e| e.to_string())?;
    fs::create_dir_all(work.join("user/.cargo")).map_err(|e| e.to_string())?;
    let ver = fs::read_to_string(root.join("VERSION")).unwrap_or_else(|_| "0.2.2\n".into());
    fs::write(work.join("VERSION"), ver).map_err(|e| e.to_string())?;
    let _ = fs::copy(root.join("rust-toolchain.toml"), work.join("rust-toolchain.toml"));
    fs::write(
        work.join("user/Cargo.toml"),
        r#"[workspace]
resolver = "2"
members = ["libsoso", "hola-std"]

[workspace.dependencies]
soso-abi = { path = "../crates/soso-abi" }
libsoso = { path = "libsoso" }
getrandom = { version = "0.2", features = ["rdrand"] }

[profile.release]
panic = "abort"
opt-level = "s"
strip = "debuginfo"
"#,
    )
    .map_err(|e| e.to_string())?;
    for (src, dst) in [
        ("user/libsoso", "user/libsoso"),
        ("user/hola-std", "user/hola-std"),
        ("crates/soso-abi", "crates/soso-abi"),
        ("crates/soso-std", "crates/soso-std"),
    ] {
        copy_dir_skip_target(&root.join(src), &work.join(dst))?;
    }
    for rel in [
        "user/Cargo.lock",
        "user/link.ld",
        "user/x86_64-soso-user.json",
        "user/.cargo/config.toml",
    ] {
        let _ = fs::copy(root.join(rel), work.join(rel));
    }
    Ok(())
}

fn copy_dir_skip_target(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for e in fs::read_dir(src).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name();
        let s = name.to_string_lossy();
        if s == "target" || s == ".git" || s.starts_with('.') {
            continue;
        }
        let to = dst.join(&name);
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            copy_dir_skip_target(&e.path(), &to)?;
        } else {
            fs::copy(e.path(), to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn esperar_tcp(addr: &str, limite: Duration) -> Result<(), String> {
    let t0 = Instant::now();
    loop {
        if TcpStream::connect(addr).is_ok() {
            return Ok(());
        }
        if t0.elapsed() > limite {
            return Err(format!("timeout esperando {addr}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Un pipeline de sosh moviendo MÁS datos que la capacidad del pipe (4 KiB).
///
/// Es la prueba de punta a punta del camino que se arregló: el pipe transfería 256
/// bytes por llamada (el tamaño de un temporal del kernel) y el userspace no miraba
/// el retorno, así que se perdía la cola de cada trozo. Se piden DOS copias del
/// README por un solo pipe para que se llene de verdad y `write_all` tenga que
/// completar escrituras cortas; se compara byte a byte contra el fichero real, que
/// el host lee en el momento (nada hardcodeado).
fn ssh_pipeline(key: &Path, ssh_port: u16) -> Result<(), String> {
    let real = std::fs::read(super::project_root().join("rootfs/README.md"))
        .map_err(|e| format!("no se pudo leer rootfs/README.md: {e}"))?;

    let texto = ssh_guion(
        key,
        ssh_port,
        "cat /README.md /README.md | cat -\nexit\n",
        Duration::from_secs(90),
    )?;
    let marca = "cat -\n";
    let ini = texto
        .find(marca)
        .ok_or_else(|| format!("no se vio el comando en la salida: {texto:?}"))?
        + marca.len();
    let fin = texto[ini..]
        .rfind("\n$ ")
        .map(|p| ini + p)
        .or_else(|| {
            texto[ini..]
                .rfind("\n$ \n")
                .map(|p| ini + p + 1)
        })
        .unwrap_or(texto.len());
    let cuerpo = &texto[ini..fin];
    let esperado = String::from_utf8_lossy(&real).replace("\r\n", "\n").repeat(2);
    if cuerpo.trim_end().replace('\r', "") != esperado.trim_end().replace('\r', "") {
        return Err(format!(
            "el pipeline entregó {} bytes y el fichero ×2 son {}",
            cuerpo.trim_end().len(),
            esperado.trim_end().len()
        ));
    }
    Ok(())
}

/// Transcripción determinista desde fichero WAV (no requiere micrófono).
fn ssh_voz_wav(key: &Path, ssh_port: u16) -> Result<(), String> {
    let guion = "soso-voz dictar --wav /etc/voz-prueba.wav --max-tokens 128\nexit\n";
    let texto = ssh_guion_inner(
        key,
        ssh_port,
        guion,
        Duration::from_secs(300),
        true,
        Some("soso-voz: transcrito"),
    )?;
    if !texto.contains("soso-voz: transcrito") {
        return Err(format!(
            "voz WAV no devolvió «soso-voz: transcrito»; stdout: {texto:?}"
        ));
    }
    let transcrito = texto
        .lines()
        .find(|l| l.contains("soso-voz: transcrito"))
        .unwrap_or("");
    let cuerpo = transcrito.split('—').nth(1).unwrap_or("").trim();
    if cuerpo.is_empty() || !cuerpo.chars().any(|c| c.is_alphabetic()) {
        return Err(format!(
            "voz WAV devolvió transcripción vacía o sin texto; stdout: {texto:?}"
        ));
    }
    Ok(())
}

fn ssh_soso_web_local(key: &Path, ssh_port: u16) -> Result<(), String> {
    let guion = "soso-web --local /etc/web-prueba.html\nq\nexit\n";
    let texto = ssh_guion(key, ssh_port, guion, Duration::from_secs(120))?;
    // El HTML va en UTF-8; por el canal SSH a veces llega como Latin-1 (PÃ¡gina).
    // Comprobamos cadenas ASCII que sí aparecen en la salida renderizada.
    if !texto.contains("Hola desde soso-web") {
        return Err(format!(
            "soso-web no mostró la página de prueba; stdout: {texto:?}"
        ));
    }
    if !texto.contains("[1]") {
        return Err(format!(
            "soso-web no listó enlaces numerados; stdout: {texto:?}"
        ));
    }
    Ok(())
}

/// `ask` tiene que entregar al modelo exactamente lo que se escribió.
///
/// Es el punto entero de este comando, y se rompe con una facilidad enorme: la
/// shell trocea por `|`, `>` y `<` y no entiende comillas, y hasta hace poco
/// descartaba en silencio todo byte ≥ 0x80 (adiós tildes). Con `:eco` la
/// comprobación no depende de lo que conteste un modelo: se compara la línea
/// devuelta carácter por carácter con la enviada.
fn ssh_ask_literal(key: &Path, ssh_port: u16) -> Result<(), String> {
    let payload = r#"¿2 > 1? | sí, "así" & <ñ>"#;
    let guion = format!("ask :eco {payload}\nexit\n");
    let texto = ssh_guion_inner(
        key,
        ssh_port,
        &guion,
        Duration::from_secs(90),
        true,
        Some(payload),
    )?;
    if texto.lines().any(|l| l == payload) || texto.contains(payload) {
        Ok(())
    } else {
        Err(format!(
            "ask :eco no devolvió la línea literal; stdout: {texto:?}"
        ))
    }
}

/// Dos `ask` en la misma sesión SSH cargan el modelo una sola vez; una sesión
/// nueva sigue sin recargar mientras askd siga vivo.
fn ssh_ask_resident(key: &Path, ssh_port: u16) -> Result<(), String> {
    // `:max 4` carga el modelo sin generar 128 tokens (el default de
    // `/etc/llm.conf`): en TCG eso no cabe en el tope de 600 s y el
    // paso mataba la sesión con askd aún masticando.
    let guion1 = "ask :max 4\nask test\nask test\nexit\n";
    let texto1 = ssh_guion(key, ssh_port, guion1, Duration::from_secs(600))?;
    let cargando = texto1.matches("ask: cargando").count();
    if cargando != 1 {
        return Err(format!(
            "esperaba «ask: cargando» una vez al cargar; vi {cargando} veces; stdout: {texto1:?}"
        ));
    }
    let guion2 = "ask test\nexit\n";
    let texto2 = ssh_guion(key, ssh_port, guion2, Duration::from_secs(240))?;
    if texto2.contains("ask: cargando") {
        return Err(format!(
            "segundo ask en nueva SSH recargó el modelo; stdout: {texto2:?}"
        ));
    }
    Ok(())
}

/// `init test`: la batería de regresión de syscalls que corre DENTRO del guest.
///
/// Existía desde la fase 6 y la suite no la ejecutaba: hilos+futex (L3b), estrés
/// FPU/YMM (L4) y ahora el camino de syscalls GPU eran subpruebas que sólo se
/// veían si alguien las lanzaba a mano. Un test que hay que acordarse de correr no
/// es una red de seguridad.
fn ssh_init_test(key: &Path, ssh_port: u16) -> Result<(), String> {
    // 150 s bastaban con KVM; sin él (TCG) esta batería —hilos, futex, estrés de
    // FPU/YMM y el camino de syscalls GPU— se pone en varios minutos. Ver la nota
    // del límite en `ssh_llm`: pasarse de corto aquí no cuesta un rojo, cuesta la
    // suite entera desde este punto.
    let texto = ssh_guion_inner(
        key,
        ssh_port,
        "init test\nexit\n",
        Duration::from_secs(420),
        true,
        Some("init: TODO OK"),
    )?;
    // El FALLO se mira ANTES del TODO OK: la suite del guest corta en el primer
    // fallo, así que sin esto un "FALLO" temprano y ningún "TODO OK" darían el
    // mismo error genérico que un timeout, y son cosas distintas.
    if let Some(l) = texto.lines().find(|l| l.contains("init: FALLO")) {
        return Err(format!("la suite del guest falló: {}", l.trim()));
    }
    if !texto.contains("init: TODO OK") {
        return Err(format!(
            "init test no llegó al final (¿timeout?); stdout: {texto:?}"
        ));
    }
    Ok(())
}

fn ssh_sesion(key: &Path, ssh_port: u16) -> Result<(), String> {
    let token = "soso_ssh_ok_42";
    // Acaba en `halt`: aquí la sesión no se cierra porque salga la shell, sino
    // porque el guest se apaga y se lleva la conexión por delante. Vale igual para
    // esperar al cliente, y de paso deja de ser una carrera contra un sleep de 4 s.
    let guion = format!("echo {token} > /tmp/xtask.txt\ncat /tmp/xtask.txt\nhalt\n");
    let texto = ssh_guion_hasta(key, ssh_port, &guion, Duration::from_secs(60), token)?;
    if texto.contains(token) {
        Ok(())
    } else {
        Err(format!("no se vio el token en la sesión SSH; salida: {texto:?}"))
    }
}

/// Espera a que QEMU salga; devuelve su código si sale a tiempo.
pub(crate) fn espera_salida(qemu: &mut Child, limite: Duration) -> Option<i32> {
    let fin = Instant::now() + limite;
    while Instant::now() < fin {
        match qemu.try_wait() {
            Ok(Some(st)) => return Some(st.code().unwrap_or(-1)),
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(_) => return None,
        }
    }
    None
}

/// QEMU que se muere solo al salir del ámbito, pase lo que pase.
///
/// El `?` de un paso que vence salía de la función SIN matar al hijo, y ese QEMU
/// huérfano se queda con el puerto 2222 y con el lock de escritura de la imagen:
/// a partir de ahí TODAS las ejecuciones siguientes fallan al arrancar («Failed
/// to get "write" lock») y el rojo que se ve no tiene nada que ver con el
/// cambio que se estaba probando. Costó dos líneas base enteras averiguarlo
/// (2026-08-01), así que el kill deja de depender del camino de salida.
struct QemuVivo(Child);

impl Drop for QemuVivo {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Smoke test lx-e1000e (`SOSO_LXDDE_TEST=1` tras `cargo xtask test`).
pub fn run_lx_e1000e_smoke() {
    let root = super::project_root();
    unsafe {
        std::env::set_var("SOSO_QEMU_NIC", "lx-e1000e");
    }
    let img = super::build_image();
    let data = super::mkfs_rootfs(true);
    let models = super::mkfs_models(true);
    let serial = root.join("target/test-lx-serial.log");
    let _ = std::fs::remove_file(&serial);
    let mut qemu = lanzar_qemu_legacy(&img, &data, &models, &serial).expect("QEMU lx-e1000e");
    let ok = esperar_en_fichero(&serial, "sosh —", Duration::from_secs(120)).is_ok();
    let _ = qemu.kill();
    let _ = qemu.wait();
    if ok {
        println!("OK    lx-e1000e smoke: arranque hasta sosh");
    } else {
        eprintln!("FALLO  lx-e1000e smoke (ver {})", serial.display());
        std::process::exit(1);
    }
}

struct UsbTestScenario {
    id: &'static str,
    name: &'static str,
    log: &'static str,
    index: u8,
    xhci: Option<&'static str>,
    usb_kbd: bool,
    usb_hub: bool,
    extra_serial: Option<&'static str>,
}


static SSH_SESSION_SEQ: AtomicU64 = AtomicU64::new(0);

/// Traza opcional de una sesión SSH del arnés (`SOSO_SSH_TRACE=<dir>`).
///
/// Un paso que falla con «stdout: "…$"» no dice si el guion llegó a escribirse,
/// si el prompt se vio tarde o si el guest no contestó. Con la traza cada
/// evento lleva su instante y los bytes que se vieron.
struct SshTrace {
    fichero: Option<std::fs::File>,
    t0: Instant,
}

impl SshTrace {
    fn new(tag: &str) -> Self {
        let fichero = std::env::var("SOSO_SSH_TRACE").ok().and_then(|dir| {
            let _ = fs::create_dir_all(&dir);
            let id = SSH_SESSION_SEQ.load(AtomicOrdering::Relaxed);
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(PathBuf::from(dir).join(format!("ssh-{tag}-{id}.trace")))
                .ok()
        });
        Self {
            fichero,
            t0: Instant::now(),
        }
    }

    fn ev(&mut self, que: &str) {
        if let Some(f) = self.fichero.as_mut() {
            let _ = writeln!(f, "[{:8.3}] {que}", self.t0.elapsed().as_secs_f64());
        }
    }
}

fn ssh_fifo_path(tag: &str) -> PathBuf {
    let id = SSH_SESSION_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
    super::project_root().join(format!(
        "target/.ssh-{tag}-{}-{id}.fifo",
        std::process::id()
    ))
}

struct SshSessionGuard {
    fifo: PathBuf,
    holder: Option<Child>,
    ssh_pid: Option<u32>,
}

impl SshSessionGuard {
    fn new(fifo: PathBuf, holder: Child) -> Self {
        Self {
            fifo,
            holder: Some(holder),
            ssh_pid: None,
        }
    }

    fn track_ssh(&mut self, pid: u32) {
        self.ssh_pid = Some(pid);
    }

    fn finish(mut self) {
        if let Some(pid) = self.ssh_pid.take() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        if let Some(mut holder) = self.holder.take() {
            let _ = holder.kill();
            let _ = holder.wait();
        }
        let _ = fs::remove_file(&self.fifo);
    }
}

impl Drop for SshSessionGuard {
    fn drop(&mut self) {
        if self.ssh_pid.is_some() || self.holder.is_some() {
            if let Some(pid) = self.ssh_pid {
                let _ = Command::new("kill").arg(pid.to_string()).status();
            }
            if let Some(mut holder) = self.holder.take() {
                let _ = holder.kill();
                let _ = holder.wait();
            }
            let _ = fs::remove_file(&self.fifo);
        }
    }
}

/// Batería USB/xHCI en QEMU (`cargo xtask test-usb`).
pub fn run_usb() {
    let root = super::project_root();
    let report = Arc::new(Report::new());

    let img = std::thread::scope(|scope| {
        scope.spawn(super::build_user);
        let kernel = scope.spawn(super::build_image);
        kernel.join().unwrap()
    });
    let data = super::mkfs_rootfs(true);
    let models = super::mkfs_models(true);

    let escenarios = [
        UsbTestScenario {
            id: "qemu",
            name: "BOT qemu-xhci",
            log: "target/test-usb-qemu.log",
            index: 0,
            xhci: None,
            usb_kbd: false,
            usb_hub: false,
            extra_serial: None,
        },
        UsbTestScenario {
            id: "nec",
            name: "BOT nec-usb-xhci",
            log: "target/test-usb-nec.log",
            index: 1,
            xhci: Some("nec"),
            usb_kbd: false,
            usb_hub: false,
            extra_serial: None,
        },
        UsbTestScenario {
            id: "kbd",
            name: "BOT + teclado HID",
            log: "target/test-usb-kbd.log",
            index: 2,
            xhci: None,
            usb_kbd: true,
            usb_hub: false,
            extra_serial: Some("kbd=true"),
        },
        UsbTestScenario {
            id: "hub",
            name: "BOT hub + teclado HID",
            log: "target/test-usb-hub.log",
            index: 3,
            xhci: None,
            usb_kbd: true,
            usb_hub: true,
            extra_serial: None,
        },
    ];

    let jobs = test_jobs();
    let accel = super::qemu_accel_mode();
    println!("test-usb: QEMU accel={accel}");
    println!("test-usb: hasta {jobs} escenarios en paralelo (SOSO_TEST_JOBS)");

    let pool = Arc::new(JobPool::new(jobs));
    std::thread::scope(|scope| {
        for esc in escenarios {
            let report = Arc::clone(&report);
            let pool = Arc::clone(&pool);
            let img = img.clone();
            let data = data.clone();
            let models = models.clone();
            let root = root.clone();
            scope.spawn(move || {
                let _permit = pool.acquire();
                let serial = root.join(esc.log);
                let _ = std::fs::remove_file(&serial);
                let bios = copiar_imagen(&img, esc.id, "bios");
                let data_img = copiar_imagen(&data, esc.id, "data");
                let models_img = copiar_imagen(&models, esc.id, "models");
                let live_src = super::package_live::live_image_path();
                super::package_live::ensure_live_image();
                let live_img = copiar_imagen(&live_src, esc.id, "live");
                let nombre = format!("USB: {}", esc.name);
                let ok = report
                    .paso("usb", &nombre, || {
                        run_usb_scenario(&bios, &data_img, &models_img, &live_img, &serial, esc)
                    })
                    .is_ok();
                if !ok {
                    eprintln!("      ver log serie: {}", serial.display());
                }
            });
        }
    });

    exit_resumen(if report.fallos() == 0 { 0 } else { 1 });
}

fn usb_ports(index: u8) -> (u16, u16, String) {
    // Rango 228x/778x: no choca con shards (220x/770x) ni test-install (2242).
    let ssh = 2280 + u16::from(index) * 10;
    let echo = 7780 + u16::from(index) * 10;
    let mac = format!("52:54:00:12:35:{:02x}", 0x10 + index);
    (ssh, echo, mac)
}

fn run_usb_scenario(
    img: &std::path::Path,
    data: &std::path::Path,
    models: &std::path::Path,
    live: &std::path::Path,
    serial: &std::path::Path,
    esc: UsbTestScenario,
) -> Result<(), String> {
    let (ssh_port, echo_port, mac) = usb_ports(esc.index);
    let guest = super::QemuGuestConfig {
        live: true,
        live_usb: true,
        xhci_model: esc.xhci.unwrap_or("qemu").into(),
        usb_kbd: esc.usb_kbd,
        usb_hub: esc.usb_hub,
        live_path: Some(live.to_path_buf()),
        ..Default::default()
    };

    let root = super::project_root();
    let monitor = if esc.usb_kbd {
        Some(root.join(format!("target/test-usb-{}-mon.sock", esc.id)))
    } else {
        None
    };

    let slot = QemuSlot {
        id: "usb",
        ssh_port,
        echo_port,
        mac: mac.into(),
        serial: serial.to_path_buf(),
        bios: img.to_path_buf(),
        data: data.to_path_buf(),
        models: models.to_path_buf(),
        mem: None,
        smp: None,
        monitor: monitor.clone(),
        guest,
    };

    let mut qemu = lanzar_qemu(&slot).map_err(|e| e.to_string())?;

    esperar_en_fichero(serial, "sosh —", Duration::from_secs(180))?;
    let serie = std::fs::read_to_string(serial).map_err(|e| e.to_string())?;
    if !serie.contains("sync_cache=true") {
        return Err("USB sin SYNCHRONIZE CACHE(10) (falta sync_cache=true)".into());
    }

    if let Some(patron) = esc.extra_serial {
        let contenido = std::fs::read_to_string(serial).map_err(|e| e.to_string())?;
        if !contenido.contains(patron) && !contenido.contains("teclado HID") {
            let _ = qemu.kill();
            let _ = qemu.wait();
            return Err(format!("no apareció {patron:?} ni «teclado HID» en el log serie"));
        }
    }

    if esc.usb_kbd {
        let key = root.join("target/soso_test_key");
        if let Some(ref mon) = monitor {
            usb_kbd_vivo_despues_io(&key, ssh_port, serial, mon)?;
        }
    }

    let _ = qemu.kill();
    let _ = qemu.wait();
    Ok(())
}

