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

pub(crate) struct QemuSlot {
    pub id: &'static str,
    pub ssh_port: u16,
    pub echo_port: u16,
    /// Reenvío host→guest API (`127.0.0.1:puerto`); `None` en la mayoría de tests.
    pub llm_host_port: Option<u16>,
    pub mac: String,
    pub serial: PathBuf,
    pub bios: PathBuf,
    pub data: PathBuf,
    pub models: PathBuf,
    pub mem: Option<String>,
    pub smp: Option<String>,
    /// Socket UNIX del monitor QEMU (`sendkey` tras I/O USB).
    pub monitor: Option<PathBuf>,
    pub guest: super::QemuGuestConfig,
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

    /// Como `paso_con_reintento`, pero relanza el guest si SSH cae (p. ej. tras
    /// un pánico intermitente a mitad de A7).
    fn paso_con_reintento_guest<F: FnMut() -> Result<(), String>>(
        &self,
        qemu: &mut Child,
        slot: &QemuSlot,
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
                    let reinicio = guest_requiere_reinicio(&e);
                    println!(
                        "      [{shard}] (reintento {}/{} de «{nombre}»{}: {e})",
                        intento,
                        MAX - 1,
                        if reinicio { " tras reinicio" } else { " tras 5 s" },
                    );
                    if reinicio {
                        if let Err(re) = reiniciar_guest_sys(qemu, slot) {
                            self.marca(shard, &format!("{nombre}: {re}"), false);
                            *self.fallos.lock().unwrap() += 1;
                            return Err(());
                        }
                    } else {
                        std::thread::sleep(Duration::from_secs(5));
                    }
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
                    // Antes de reintentar —y sobre todo antes de reiniciar, que
                    // borra la evidencia— mirar la máquina viva. Un paso flaky
                    // sin esto sólo dice «no contestó»; con esto dice en qué
                    // estaba parado cada proceso, que es la diferencia entre
                    // «se colgó» y «no le dio tiempo».
                    diagnostico_de_fallo(slot, sid, nombre);
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
        llm_host_port: None,
        mac: shard.mac(),
        serial,
        // El nombre de la copia tiene que conservar el firmware: `apply_firmware`
        // decide por él si hay que añadir OVMF.
        bios: copiar_imagen(img, id, super::firmware_kind(img)),
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
            "ask: mmap huge (tiny-huge shard ≥2 MiB)",
            || {
                let serial = slot.serial.clone();
                let _ = report.paso_con_reintento(
                    sid,
                    "ask: mmap huge (tiny-huge shard ≥2 MiB)",
                    || ssh_ask_huge_mmap(key, port, &serial),
                );
            },
        );
        filter.if_step(
            sid,
            "soso-llm: 20 ciclos carga/generación/cambio (A7)",
            || {
                let _ = report.paso_con_reintento_guest(
                    &mut qemu,
                    slot,
                    sid,
                    "soso-llm: 20 ciclos carga/generación/cambio (A7)",
                    || ssh_llm_ciclos(key, port),
                );
            },
        );
    }
    let _ = qemu.kill();
    let _ = qemu.wait();
}

/// Un comando trivial que debe contestar rápido: detecta la máquina ahogada
/// por hilos que quedaron girando.
pub(crate) fn ssh_vive(key: &Path, ssh_port: u16) -> Result<(), String> {
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
        filter.if_step(sid, "ssh: dos sesiones concurrentes", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "ssh: dos sesiones concurrentes",
                || ssh_dos_sesiones(key, port),
            );
        });
        filter.if_step(sid, "ping ICMP a 10.0.2.2", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "ping ICMP a 10.0.2.2", || {
                ssh_ping(key, port)
            });
        });
        filter.if_step(sid, "ps lista procesos", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "ps lista procesos", || {
                ssh_ps(key, port)
            });
        });
        filter.if_step(sid, "soso-improve: runner de pruebas en soso", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "soso-improve: runner de pruebas en soso",
                || ssh_improve_pruebas(key, port),
            );
        });
        filter.if_step(sid, "soso-improve: receta del bootstrap (T39)", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "soso-improve: receta del bootstrap (T39)",
                || ssh_improve_receta(key, port),
            );
        });
        filter.if_step(sid, "soso-improve: estado de tareas", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "soso-improve: estado de tareas",
                || ssh_improve_tarea(key, port),
            );
        });
        filter.if_step(sid, "soso-improve: dos tuberías sin bloquear", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "soso-improve: dos tuberías sin bloquear",
                || ssh_improve_tuberias(key, port),
            );
        });
        filter.if_step(sid, "soso-improve: procesos (argv, stdin, cwd)", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "soso-improve: procesos (argv, stdin, cwd)",
                || ssh_improve_procesos(key, port),
            );
        });
        filter.if_step(sid, "soso-improve: eco, reloj y plazos", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "soso-improve: eco, reloj y plazos",
                || ssh_improve_eco(key, port),
            );
        });
        filter.if_step(sid, "soso-improve: órdenes y códigos de salida", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "soso-improve: órdenes y códigos de salida",
                || ssh_improve_cli(key, port),
            );
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
        filter.if_step(sid, "grep: stdin de un pipe", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "grep: stdin de un pipe", || {
                ssh_grep_pipe(key, port)
            });
        });
        filter.if_step(sid, "voz: transcribe WAV de prueba", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "voz: transcribe WAV de prueba", || {
                ssh_voz_wav(key, port)
            });
        });
        filter.if_step(sid, "probe: archivos persistentes", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "probe: archivos persistentes",
                || ssh_probe_archivos(key, port),
            );
        });
        filter.if_step(sid, "probe: páginas ejecutables (W+X)", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "probe: páginas ejecutables (W+X)",
                || ssh_probe_ejecutable(key, port),
            );
        });
        filter.if_step(sid, "probe: descriptores sobre el mismo fichero", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "probe: descriptores sobre el mismo fichero",
                || ssh_probe_compartir(key, port),
            );
        });
        filter.if_step(sid, "probe: señales y muerte de subprocesos", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "probe: señales y muerte de subprocesos",
                || ssh_probe_senales(key, port),
            );
        });
        filter.if_step(sid, "probe: salidas, entorno y cwd de una herramienta", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "probe: salidas, entorno y cwd de una herramienta",
                || ssh_probe_salidas(key, port),
            );
        });
        filter.if_step(sid, "probe: hilos, guarda de pila y relojes", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "probe: hilos, guarda de pila y relojes",
                || ssh_probe_hilos(key, port),
            );
        });
        filter.if_step(sid, "probe: tuberías con EOF y TCP con reconexión", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "probe: tuberías con EOF y TCP con reconexión",
                || ssh_probe_canales(key, port),
            );
        });
        filter.if_step(sid, "probe: coste de escritura en sosofs (N-001)", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "probe: coste de escritura en sosofs (N-001)",
                || ssh_probe_coste(key, port),
            );
        });
        filter.if_step(sid, "probe: carga de codigo ajeno (N-005)", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "probe: carga de codigo ajeno (N-005)",
                || ssh_probe_carga(key, port),
            );
        });
        filter.if_step(sid, "probe: busqueda con semantica declarada (N-009)", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "probe: busqueda con semantica declarada (N-009)",
                || ssh_probe_busqueda(key, port),
            );
        });
        filter.if_step(sid, "probe: normalizar rutas (N-012)", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "probe: normalizar rutas (N-012)", || {
                ssh_probe_rutas(key, port)
            });
        });
        filter.if_step(sid, "probe: enterarse de un cambio (N-008)", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "probe: enterarse de un cambio (N-008)", || {
                ssh_probe_vigilancia(key, port)
            });
        });
        filter.if_step(sid, "sosh: lo que no sabe hacer lo dice (N-010)", || {
            report.paso_ssh_sys(&mut qemu, slot, sid, "sosh: lo que no sabe hacer lo dice (N-010)", || {
                ssh_sosh_subconjunto(key, port)
            });
        });
        filter.if_step(sid, "sosh: comillas en rutas con espacios", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "sosh: comillas en rutas con espacios",
                || ssh_sosh_comillas(key, port),
            );
        });
        filter.if_step(sid, "fd 3: redirección y comando log", || {
            report.paso_ssh_sys(
                &mut qemu,
                slot,
                sid,
                "fd 3: redirección y comando log",
                || ssh_fd3_log(key, port),
            );
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

/// Retrato de la máquina en el momento en que un paso falla.
///
/// Abre una sesión SSH **nueva** —si el guest sigue vivo— y pide `ps`. Que la
/// sesión entre ya es un dato: significa que la red y el planificador funcionan
/// y que lo parado es el proceso del paso, no el sistema.
fn diagnostico_de_fallo(slot: &QemuSlot, sid: &str, nombre: &str) {
    let key = crate::project_root().join("target/soso_test_key");
    match ssh_guion_inner(
        &key,
        slot.ssh_port,
        "ps\nexit\n",
        Duration::from_secs(30),
        true,
        Some("COMANDO"),
    ) {
        Ok(texto) => {
            println!("      [{sid}] estado tras fallar «{nombre}»:");
            for l in texto.lines().filter(|l| l.contains("/bin/") || l.contains("ESTADO")) {
                println!("      [{sid}]   {}", l.trim_end());
            }
        }
        Err(e) => {
            // Si no se puede entrar, lo que queda es la serie — y `reiniciar_guest_sys`
            // la trunca justo después, así que o se lee aquí o se pierde.
            println!("      [{sid}] no se pudo mirar tras «{nombre}»: {e}");
            let serie = fs::read_to_string(&slot.serial).unwrap_or_default();
            let panico = serie.contains("!!! panic");
            // Copia fuera del camino que `reiniciar_guest_sys` trunca. Sin esto
            // sólo queda lo que quepa en la consola, y de un pánico hace falta
            // el rastro entero para resolverlo con addr2line.
            let copia = crate::project_root().join(format!(
                "target/panico-{sid}-{}.log",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            let guardada = fs::write(&copia, &serie).is_ok();
            println!(
                "      [{sid}] serie ({} B{}){}",
                serie.len(),
                if panico { ", CON PÁNICO" } else { "" },
                if guardada {
                    format!(" → {}", copia.display())
                } else {
                    String::new()
                }
            );
            // Desde el primer `rastro:` hasta el final: es el bloque que sirve.
            let lineas: Vec<&str> = serie.lines().collect();
            let desde = lineas
                .iter()
                .position(|l| l.contains("rastro:") || l.contains("!!! panic"))
                .unwrap_or(lineas.len().saturating_sub(12));
            for l in &lineas[desde..] {
                println!("      [{sid}]   {}", l.trim_end());
            }
        }
    }
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
                 pkill -f 'qemu-system-x86_64.*test-llm-api-' 2>/dev/null; \
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

pub(crate) fn lanzar_qemu(slot: &QemuSlot) -> std::io::Result<Child> {
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
    super::apply_qemu_nic_with_ports(
        &mut qemu,
        slot.ssh_port,
        slot.echo_port,
        Some(&slot.mac),
        slot.llm_host_port,
    );
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
        llm_host_port: None,
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
            // El **stderr** del ssh va aquí a propósito. Sólo se añadía a la
            // salida en el camino de éxito (más abajo), así que este error —el
            // único que se ve cuando el guest no contesta— decía «stdout
            // parcial: \"\"» y se tragaba el motivo real: «Connection refused»,
            // «Connection timed out», clave rechazada… Un fallo intermitente
            // cuyo mensaje oculta su causa no se puede diagnosticar nunca.
            let err = String::from_utf8_lossy(&stderr_acum.lock().unwrap()).into_owned();
            let _ = hijo.kill();
            let _ = hijo.wait();
            return Err(format!(
                "no apareció el prompt en 45s; stdout parcial: {:?}; ssh dijo: {:?}",
                String::from_utf8_lossy(&acum.lock().unwrap()),
                err.trim()
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

/// T49: el runner que ejecuta las comprobaciones **dentro** de soso.
///
/// Lo que se comprueba aquí, además de que pasen, es que las capacidades que
/// todavía no existen se informen como **pendientes**: cuentan en el total y no
/// como aciertos. Contarlas bien es lo que impide que el porcentaje suba solo.
/// T23 — el estado del coordinador, escrito y releído **desde el disco de
/// soso**.
///
/// Lo que acredita: que el formato sobrevive a un viaje por sosofs, que una
/// medida desconocida sigue desconocida al volver, y que el coordinador no
/// puede aceptar su propio trabajo tampoco aquí. En el host eso lo comprueban
/// `--test state`; esta es la mitad que no se puede deducir de aquella.

/// T39 — la receta del bootstrap también corre **dentro** de soso.
///
/// No hay un vendor de Rust en el guest, así que lo que se comprueba no es el
/// resultado de los parches sino que la orden esté enchufada de verdad: que la
/// capacidad se declare, que el despacho llegue y que el informe distinga
/// «falta» de «ancla rota». Apuntando a un directorio vacío eso da una forma
/// **determinista**: los dos pasos de copia de árbol dicen FALTA (el destino no
/// está) y los ocho restantes ROTO (no se puede leer el fichero).
///
/// Sin esto, «invocable desde los dos frontends» se quedaría en que compila.
fn ssh_improve_receta(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-improve receta comprobar /var/receta-vacia /var/receta-vacia\nexit\n",
        Duration::from_secs(120),
    )?;
    for esperado in [
        "receta: 10 pasos",
        "FALTA PAL: os/soso",
        "ROTO  build.rs: target soso",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta «{esperado}»: {salida:?}"));
        }
    }
    // La cuenta exacta: si un cambio hiciera que un paso dejara de informar,
    // el total lo delata aunque las tres líneas de arriba sigan estando.
    if !salida.contains("2 por aplicar · 8 con el ancla rota") {
        return Err(format!("la cuenta de la receta no cuadra: {salida:?}"));
    }
    Ok(())
}


fn ssh_improve_tarea(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-improve tarea preparar\nexit\n",
        Duration::from_secs(180),
    )?;
    for esperado in [
        "tarea: preparada r-autoprueba para T99-autoprueba",
        "el coordinador no puede aceptar: ok",
        "el intento cerrado sobrevivió al disco: ok",
        "lo no medido sigue sin medirse tras el viaje: ok",
        "tarea: 0 caso(s) mal",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta «{esperado}»: {salida:?}"));
        }
    }
    // Y el informe imprime la medida ausente como ausente, con su motivo: un
    // `0` aquí sería un dato inventado que después alguien promedia.
    if !salida.contains("tokens ? (") {
        return Err(format!("la medida desconocida no se informó: {salida:?}"));
    }
    Ok(())
}

fn ssh_improve_pruebas(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-improve pruebas\nexit\n",
        Duration::from_secs(300),
    )?;
    for esperado in [
        "procesos/argv-stdin-cwd",
        "transporte/eco",
        "tuberias/dos-canales",
        // T63: un resto de corte no encalla la referencia. Se comprueba aquí,
        // en sosofs, porque el patrón de generaciones existe precisamente
        // porque en soso `rename` no sustituye al destino.
        "durable/publicar-sobre-un-corte",
        // T24: el ciclo copia → parche → aplicar, sin Git, dentro de soso.
        "workspace/copia-y-parche",
        // T62: una ruta con un espacio, vista por `main` de un programa real.
        "argv/ruta-con-espacios",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta la prueba «{esperado}»: {salida:?}"));
        }
    }
    // Programa y repo no existen todavía: tienen que salir como pendientes,
    // ni aprobadas ni escondidas.
    if !salida.contains("pendiente") || !salida.contains("T40") {
        return Err(format!("las capacidades ausentes no se informaron: {salida:?}"));
    }
    if salida.contains("fallo ") || salida.contains("error  ") {
        return Err(format!("alguna prueba falló: {salida:?}"));
    }
    if !salida.contains("\"fallaron\":0") {
        return Err(format!("el resumen JSON no dice 0 fallos: {salida:?}"));
    }
    Ok(())
}

/// T61: leer dos tuberías alternando, sin bloquearse.
///
/// El hijo escribe 12 KiB en stderr —tres veces el búfer de 4 KiB— y una línea
/// en stdout. Sin `read_timeout` sobre tuberías, el padre se queda esperando en
/// una mientras el hijo se atasca en la otra, y el paso vence por plazo. Es el
/// interbloqueo que la ficha pide reproducir.
fn ssh_improve_tuberias(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-improve tuberias\nexit\n",
        Duration::from_secs(120),
    )?;
    if !salida.contains("tuberías alternadas: ok") {
        return Err(format!("el drenaje alternado falló: {salida:?}"));
    }
    if !salida.contains("tuberias: 0 caso(s) mal") {
        return Err(format!("algún caso de tuberías falló: {salida:?}"));
    }
    Ok(())
}

/// T47: el adaptador de procesos cumple `Orden`/`Salida` o dice qué le falta.
///
/// El caso decisivo es el de argv con espacios: el adaptador anterior juntaba
/// los argumentos con espacios, así que una ruta con un espacio llegaba al hijo
/// partida en dos. Con la tabla de argv de la ABI llega entera.
fn ssh_improve_procesos(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-improve procesos\nexit\n",
        Duration::from_secs(120),
    )?;
    for esperado in [
        "argv con espacios: ok",
        "stdin: ok",
        "código de fallo: ok",
        "cwd restaurado: ok",
        "stdin grande: ok",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta «{esperado}» en la salida: {salida:?}"));
        }
    }
    if !salida.contains("procesos: 0 caso(s) mal") {
        return Err(format!("algún caso de procesos falló: {salida:?}"));
    }
    Ok(())
}

/// T48: reloj monotónico y transporte, ejercitados **entre procesos soso**.
///
/// El cliente y el servidor son dos procesos del guest hablando por la pila de
/// red de soso; no hay proxy del host en medio, que es justo lo que la ficha
/// prohíbe para darla por verificada. Lo que se comprueba es que las tres
/// cosas se distinguen: hay datos, hay que esperar, y el otro cerró.
fn ssh_improve_eco(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-improve eco --puerto 9450\nexit\n",
        Duration::from_secs(180),
    )?;
    for esperado in [
        "eco byte-a-byte + utf8 partido: ok",
        "cierre temprano: ok",
        "cliente lento: ok",
        "plazo agotado: ok",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta «{esperado}» en la salida: {salida:?}"));
        }
    }
    if !salida.contains("eco: 0 caso(s) mal") {
        return Err(format!("algún caso de eco falló: {salida:?}"));
    }
    // Un fallo saldría con código != 0 y `sosh` lo diría.
    if salida.contains("salió con código") {
        return Err(format!("`eco` no salió con 0: {salida:?}"));
    }
    Ok(())
}

/// T45: los dos frontends comparten órdenes, capacidades y códigos de salida.
///
/// Lo que de verdad se comprueba aquí es el agujero que cerró la ficha: el
/// guest **imprimía** el fallo de una verificación y salía con 0, así que
/// cualquier arnés que mirara el código de salida daba por buena una medida
/// rota. `sosh` imprime «salió con código N» cuando N no es cero, que es la
/// única forma de leer el código desde una sesión SSH.
fn ssh_improve_cli(key: &Path, ssh_port: u16) -> Result<(), String> {
    const FIX: &str = "/var/self-improvement/fixtures-cli";
    let guion = format!(
        "soso-improve capacidades
         soso-improve verificar protocolo --banco {FIX} --caso Q04 --respuesta {FIX}/reservado/Q04/correcta.json
         soso-improve verificar protocolo --banco {FIX} --caso Q04 --respuesta {FIX}/reservado/Q04/incorrecta.json
         soso-improve verificar programa --caso P01 --candidato /tmp/no-existe.rs
         soso-improve inventada
         exit
"
    );
    let salida = ssh_guion(key, ssh_port, &guion, Duration::from_secs(120))?;

    // Capacidades publicadas: están las que hay y **no** las que no.
    if !salida.contains("verificar protocolo") {
        return Err(format!("`capacidades` no listó verificar protocolo: {salida:?}"));
    }
    // El caso correcto pasa…
    if !salida.contains("Q04: ok") {
        return Err(format!("el caso correcto no dio ok: {salida:?}"));
    }
    // …y el incorrecto falla **con código 2**, no con 0.
    if !salida.contains("salió con código 2") {
        return Err(format!(
            "una verificación fallida no salió con código 2: {salida:?}"
        ));
    }
    // Capacidad ausente: se rechaza antes de cualquier efecto y dice su ficha.
    if !salida.contains("capacidad ausente") || !salida.contains("T40") {
        return Err(format!(
            "`verificar programa` no se rechazó como capacidad ausente: {salida:?}"
        ));
    }
    if !salida.contains("orden desconocida") {
        return Err(format!("una orden inventada no se rechazó: {salida:?}"));
    }
    Ok(())
}

fn ssh_ps(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(key, ssh_port, "ps\nexit\n", Duration::from_secs(30))?;
    if !salida.contains("/bin/init") {
        return Err(format!("ps no listó init; stdout: {salida:?}"));
    }
    if !salida.contains("/bin/sosh") {
        return Err(format!("ps no listó sosh; stdout: {salida:?}"));
    }
    if !salida.contains("PID") || !salida.contains("COMANDO") {
        return Err(format!("ps sin cabecera esperada; stdout: {salida:?}"));
    }
    Ok(())
}

fn ssh_ping(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "ping -c 1 127.0.0.1\nping -c 1 10.0.2.2\nexit\n",
        Duration::from_secs(30),
    )?;
    if !salida.contains("127.0.0.1") || !salida.lines().any(|l| l.contains("seq=1") && l.contains("ms"))
    {
        return Err(format!("ping 127.0.0.1 no contestó; stdout: {salida:?}"));
    }
    if !salida.contains("10.0.2.2")
        || !salida
            .lines()
            .any(|l| l.contains("10.0.2.2") && l.contains("seq=1") && l.contains("ms"))
    {
        return Err(format!("ping 10.0.2.2 no contestó; stdout: {salida:?}"));
    }
    Ok(())
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

/// Dos clientes SSH a la vez: salidas aisladas (no cola en un solo canal).
fn ssh_dos_sesiones(key: &Path, ssh_port: u16) -> Result<(), String> {
    let key_path = key.to_path_buf();
    let limite_largo = Duration::from_secs(120);
    let limite_corto = Duration::from_secs(60);
    let key_a = key_path.clone();
    let hilo_a = std::thread::spawn(move || {
        ssh_guion(
            &key_a,
            ssh_port,
            "init sleep 4000\necho FIN-A\nexit\n",
            limite_largo,
        )
    });
    std::thread::sleep(Duration::from_secs(1));
    let texto_b = ssh_guion(
        &key_path,
        ssh_port,
        "echo FIN-B\nexit\n",
        limite_corto,
    )?;
    let texto_a = hilo_a
        .join()
        .map_err(|_| "hilo SSH A paniqueó".to_string())??;
    if !texto_a.contains("FIN-A") {
        return Err(format!("sesión A sin FIN-A: {texto_a:?}"));
    }
    if texto_a.contains("FIN-B") {
        return Err(format!("sesión A mezcló salida de B: {texto_a:?}"));
    }
    if !texto_b.contains("FIN-B") {
        return Err(format!("sesión B sin FIN-B: {texto_b:?}"));
    }
    if texto_b.contains("FIN-A") {
        return Err(format!("sesión B mezcló salida de A: {texto_b:?}"));
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
    // Tras «ask: mmap huge» askd sigue con tiny-huge residente; mezclar ask +
    // `soso-llm run tiny` en la misma sesión con ese modelo colgado provocaba
    // cortes de SSH a mitad del guion (3 «generado» y guest muerto en el reintento).
    let mut guion = String::from("ask :modelo tiny\nask :max 1\n");
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

/// Texto emitido por el modelo justo antes de `soso-llm: generado`.
///
/// El resumen GPU (`dispositivo «soft»`, subidas, y el aviso de `on_gpu=0` del
/// dispositivo software: «el silicio no calculó nada») se imprime **antes** de
/// esa línea. Tomar la vecina era comparar diagnósticos, no tokens — así fallaba
/// el shard `llm-moe` con cpu `""` y dev el aviso de GSP.
fn texto_generado_antes_de_resumen<'a>(lineas: &'a [&'a str], idx_generado: usize) -> &'a str {
    let mut i = idx_generado;
    while i > 0 {
        i -= 1;
        let l = lineas[i].trim();
        if l.starts_with("soso-llm:") || l.starts_with("askd:") || l.starts_with("gpu:") {
            continue;
        }
        return l;
    }
    ""
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
    // coincidiría. Tras el streaming hay un salto; el resumen GPU puede intercalarse
    // antes de `generado`, así que se saltan líneas de diagnóstico.
    let salidas: Vec<&str> = lineas
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains("soso-llm: generado"))
        .map(|(i, _)| texto_generado_antes_de_resumen(&lineas, i))
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
    // Tras el camino feliz, dos casos que **no** deben dejar nada en el
    // staging (T36). Van en el mismo guion para aprovechar el servidor y el
    // arranque que ya están en pie.
    //
    // El estado del último `sync` vive en un fichero, así que se puede
    // estropear desde la shell sin meter ganchos de prueba en el cliente:
    // 1. un build-id que no es el de este sync → el recibo no cuadra;
    // 2. sin estado ninguno → no hay contra qué comparar y no se construye.
    let guion = format!(
        "soso-forja write-hola --msg {MSG}\n\
         soso-forja all --host 10.0.2.2 --token soso-b3\n\
         /bin/hola-std\n\
         echo build-id=noesmio > /var/forja-cache/ultimo-sync.txt\n\
         echo source-manifest-sha256=00 >> /var/forja-cache/ultimo-sync.txt\n\
         soso-forja build --host 10.0.2.2 --token soso-b3\n\
         rm /var/forja-cache/ultimo-sync.txt\n\
         soso-forja build --host 10.0.2.2 --token soso-b3\n\
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
    // Lo que se vigila no es sólo que falle: es que **no escriba**. Un cliente
    // que rechaza el recibo pero ya dejó el pack en el staging no protege de
    // nada, porque de ahí se aplica.
    if !texto.contains("forja: recibo rechazado") {
        return Err(format!("un recibo ajeno no se rechazó: {texto}"));
    }
    if !texto.contains("no se escribió nada en") {
        return Err(format!("rechazó el recibo pero no dijo que no escribió: {texto}"));
    }
    if !texto.contains("no hay un sync previo") {
        return Err(format!("construyó sin sync previo: {texto}"));
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
# Las mismas features que `user/Cargo.toml`: `libsoso` usa
# `register_custom_getrandom!`, que sólo existe con `custom`. Este manifiesto
# se escribió con `rdrand` y funcionaba porque la copia de `rootfs/src` estaba
# vieja; al sincronizarla, el build de Forja dejó de compilar.
getrandom = { version = "0.2", features = ["custom"] }

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

/// `grep PATRON` sin ficheros lee el pipe (EOF al cerrar el escritor).
fn ssh_grep_pipe(key: &Path, ssh_port: u16) -> Result<(), String> {
    let token = "soso_grep_askd_42";
    let texto = ssh_guion(
        key,
        ssh_port,
        &format!("echo {token} | grep askd\nexit\n"),
        Duration::from_secs(60),
    )?;
    if !texto.contains(token) {
        return Err(format!(
            "log|grep: no se vio {token} en `echo | grep askd`; stdout: {texto:?}"
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

/// `ask` con `tiny-huge`: al menos un shard ≥2 MiB (attn_q 768×768 f32) fuerza
/// el camino de página grande en `handle_mmap_fault`. Comprueba que la carga no
/// mata askd y que la inferencia arranca (smoke del bug del 27B en placa).
fn ssh_ask_huge_mmap(key: &Path, ssh_port: u16, serial: &Path) -> Result<(), String> {
    let guion = "ask :modelo tiny-huge\nask :max 1\nask hola\nlog\nexit\n";
    let texto = ssh_guion(key, ssh_port, guion, Duration::from_secs(600))?;
    if texto.contains("ask: no pude cargar") || texto.contains("ask: error\n") {
        return Err(format!(
            "ask no cargó tiny-huge; stdout: {texto:?}"
        ));
    }
    if !texto.contains("ask: cargando tiny-huge") {
        return Err(format!(
            "falta «ask: cargando tiny-huge»; stdout: {texto:?}"
        ));
    }
    if !texto.contains("ask: generando") {
        return Err(format!(
            "ask no llegó a generar tras mmap huge; stdout: {texto:?}"
        ));
    }
    let serie = fs::read_to_string(serial).unwrap_or_default();
    if serie.contains("page fault de usuario") || serie.contains("matado: page fault") {
        return Err(format!(
            "page fault de usuario en serial durante ask huge; ver {}",
            serial.display()
        ));
    }
    if serie.contains("mmap-fault:") {
        return Err(format!(
            "mmap-fault en serial (camino huge falló); ver {}",
            serial.display()
        ));
    }
    if !texto.contains("askd: tiny-huge listo") {
        return Err(format!(
            "askd no terminó de cargar tiny-huge; `log` sin «listo»; stdout: {texto:?}"
        ));
    }
    Ok(())
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

/// T64 — `sosh` respeta las comillas.
///
/// Se comprueba el camino entero desde la shell: **escribir** un fichero cuyo
/// nombre lleva un espacio (redirección con la ruta entrecomillada) y volver a
/// **leerlo**. Antes de T64 la línea se partía en tres palabras y `cat`
/// contestaba dos «no existe»; antes de [T62] ni siquiera habría llegado
/// entero al programa.
/// T33, sonda 1 — archivos persistentes.
///
/// Lo que acredita es el **efecto**, no el código de retorno: se escribe, se
/// cierra, se reabre y se compara byte a byte. Un `write` que devolviera el
/// número de bytes sin guardarlos pasaría una comprobación de errno y fallaría
/// aquí.
///
/// Este paso cubre «sobrevive a cerrar el descriptor». «Sobrevive al apagado»
/// es la sonda de dos fases, que necesita reiniciar la máquina.
fn ssh_probe_archivos(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe archivos\nexit\n",
        Duration::from_secs(120),
    )?;
    for esperado in [
        "archivos/texto-utf8",
        "archivos/binario",
        "archivos/nombre-con-espacio",
        "archivos/stat-texto",
        "archivos/truncar",
        "archivos/borrar",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta el caso «{esperado}»: {salida:?}"));
        }
    }
    if salida.contains("FALLO") {
        return Err(format!("alguna capacidad falló: {salida:?}"));
    }
    if !salida.contains("\"malos\":0") {
        return Err(format!("el informe no dice 0 malos: {salida:?}"));
    }
    publicar_informe("archivos", &salida);
    Ok(())
}

/// T33, sonda 2 — páginas ejecutables y W+X.
///
/// La decisiva del inventario de T32: sin poder escribir código y ejecutarlo
/// no hay JIT, y sin JIT no hay JavaScriptCore. Se mide **ejecutando**, no
/// leyendo los bits de la tabla de páginas: si la página no fuera ejecutable
/// el proceso moriría, así que la ausencia del informe final también es un
/// resultado, y por eso la sonda imprime cada caso según lo decide.
fn ssh_probe_ejecutable(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe ejecutable\nexit\n",
        Duration::from_secs(120),
    )?;
    for esperado in [
        "exec/mmap",
        "exec/prot-exec-existe",
        "exec/w-mas-x",
        "exec/tras-quitar-escritura",
        "exec/volver-a-escribir",
        "exec/monton",
    ] {
        if !salida.contains(esperado) {
            return Err(format!(
                "falta el caso «{esperado}» — ¿murió el proceso antes de llegar?: {salida:?}"
            ));
        }
    }
    if !salida.contains("probe-json:") {
        return Err(format!("la sonda no llegó al informe final: {salida:?}"));
    }
    if salida.contains("FALLO") {
        return Err(format!("alguna capacidad falló: {salida:?}"));
    }
    publicar_informe("exec", &salida);
    Ok(())
}

/// Vuelca las líneas del informe de una sonda al log de la suite.
///
/// En T33 la **medida es el entregable**: un paso que sólo dice OK no deja
/// constancia de qué se observó, y el criterio no se puede revisar después.
fn publicar_informe(etiqueta: &str, salida: &str) {
    for l in salida.lines() {
        let l = l.trim();
        if l.starts_with("probe: ") || l.starts_with("probe-json: ") {
            println!("      [{etiqueta}] {l}");
        }
    }
}

/// T33, sonda 3 — varios descriptores sobre el mismo fichero.
///
/// Lo que necesita SQLite, y por tanto el `bun:sqlite` del inventario. El paso
/// **publica el informe pase o falle**: aquí la medida es el entregable, y un
/// resultado negativo es tan útil como uno positivo mientras quede escrito.
fn ssh_probe_compartir(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe compartir\nexit\n",
        Duration::from_secs(120),
    )?;
    if !salida.contains("probe-json:") {
        return Err(format!("la sonda no llegó al informe final: {salida:?}"));
    }
    publicar_informe("compartir", &salida);
    // Ojo: este paso **no** falla porque un caso salga negativo. La sonda
    // mide una capacidad que T32 ya sospechaba ausente, y el resultado —bueno
    // o malo— es el entregable. Falla si no llegó a medir, que es lo único
    // que invalidaría la medida.
    for esperado in [
        "compartir/offsets-independientes",
        "compartir/visibilidad-entre-descriptores",
        "compartir/escritura-conserva-el-resto",
        "compartir/dos-escritores-zonas-distintas",
        "compartir/pwrite-en-offset",
        "compartir/o-excl-excluye",
        "compartir/el-cerrojo-excluye-a-otro-proceso",
        "compartir/soltar-el-cerrojo-deja-pasar",
        "compartir/dos-compartidos-conviven",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta el caso «{esperado}»: {salida:?}"));
        }
    }
    Ok(())
}

/// T33, sonda 4 — señales, por lo que un runtime necesita de ellas.
///
/// No se mide «¿se puede manejar una señal?» —eso ya se sabe leyendo el ABI, y
/// es que no— sino matar un subproceso desbocado, distinguir esa muerte de una
/// salida con código, y qué pasa al escribir en una tubería sin lector.
fn ssh_probe_senales(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe senales\nexit\n",
        Duration::from_secs(180),
    )?;
    if !salida.contains("probe-json:") {
        return Err(format!("la sonda no llegó al informe final: {salida:?}"));
    }
    publicar_informe("senales", &salida);
    for esperado in [
        "senales/salida-normal",
        "senales/sigterm-mata-y-se-distingue",
        "senales/sigkill-se-distingue-de-sigterm",
        "senales/sondeo-distingue-vivo-de-inexistente",
        "senales/kill-devuelve-cuantos-no-cero",
        "senales/tuberia-sin-lector",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta el caso «{esperado}»: {salida:?}"));
        }
    }
    // El hijo dormilón no debe haber llegado al final de su siesta: si lo
    // hiciera, `kill` no habría cortado nada y el caso pasaría por otro
    // camino.
    if salida.contains("terminó de dormir 60000") {
        return Err(format!("el kill no cortó al hijo: {salida:?}"));
    }
    Ok(())
}

/// T33, sonda 5 — lo que necesita un lanzador de herramientas.
///
/// stdout y stderr **separados** (mezclarlos corrompe la salida que el agente
/// parsea), código de salida, entorno y directorio de trabajo.
fn ssh_probe_salidas(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe salidas\nexit\n",
        Duration::from_secs(180),
    )?;
    if !salida.contains("probe-json:") {
        return Err(format!("la sonda no llegó al informe final: {salida:?}"));
    }
    publicar_informe("salidas", &salida);
    for esperado in [
        "salidas/stdout-lleva-solo-lo-suyo",
        "salidas/stderr-lleva-solo-lo-suyo",
        "salidas/codigo-de-salida",
        "entorno/variable-heredada",
        "cwd/el-hijo-hereda-el-del-padre",
        "cwd/el-hijo-arranca-donde-se-le-dice",
        "cwd/el-padre-no-se-mueve",
        "cwd/un-directorio-inexistente-falla",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta el caso «{esperado}»: {salida:?}"));
        }
    }
    Ok(())
}

/// T33, sonda 6 — hilos, `join`, guarda de pila y relojes.
///
/// Mide una cosa que el código de `thread::spawn` da por hecha: que la guarda
/// de pila se instala. Como los demás casos negativos de T33, el paso publica
/// el informe y no falla por ello: la medida es el entregable.
fn ssh_probe_hilos(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe hilos\nexit\n",
        Duration::from_secs(180),
    )?;
    if !salida.contains("probe-json:") {
        return Err(format!("la sonda no llegó al informe final: {salida:?}"));
    }
    publicar_informe("hilos", &salida);
    for esperado in [
        "hilos/cuatro-corren-y-se-recogen",
        "hilos/join-espera-de-verdad",
        "hilos/guarda-de-pila-se-puede-instalar",
        "hilos/libsoso-dice-la-verdad",
        "hilos/tocar-la-guarda-mata-al-proceso",
        "temporizadores/dormir-no-vuelve-antes",
        "temporizadores/el-reloj-no-retrocede",
        "temporizadores/el-reloj-de-pared-avanza",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta el caso «{esperado}»: {salida:?}"));
        }
    }
    Ok(())
}

/// T33, sonda 7 — tuberías con EOF y TCP con cierre y reconexión.
///
/// Las dos son de lo mismo: saber **cuándo se acabó**. Confundir «todavía no
/// hay datos» con «ya no va a haber más» es esperar de más o truncar.
fn ssh_probe_canales(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe canales\nexit\n",
        Duration::from_secs(180),
    )?;
    if !salida.contains("probe-json:") {
        return Err(format!("la sonda no llegó al informe final: {salida:?}"));
    }
    publicar_informe("canales", &salida);
    for esperado in [
        "canales/salida-grande-intacta",
        "canales/eof-cuando-el-escritor-muere",
        "canales/tcp-dos-conexiones-al-mismo-destino",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta el caso «{esperado}»: {salida:?}"));
        }
    }
    Ok(())
}

/// N-012 — que normalizar rutas siga significando lo mismo.
///
/// `abs_path` se reescribió para hacer una reserva en vez de cinco, y es el
/// camino de **toda** llamada con ruta. El kernel no se compila para el host,
/// así que la comprobación vive aquí; y lo que de verdad se vigila es que
/// `..` **no deje salirse de la raíz**.
fn ssh_probe_rutas(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe rutas\nexit\n",
        Duration::from_secs(240),
    )?;
    publicar_informe("rutas", &salida);
    for esperado in [
        "rutas/el-fichero-esta-donde-se-dejo ok",
        "rutas/el-punto-no-cambia-nada ok",
        "rutas/las-barras-de-mas-no-cuentan ok",
        "rutas/punto-punto-sube-un-nivel ok",
        "rutas/subir-desde-la-raiz-no-escapa ok",
        "rutas/relativa-usa-el-cwd ok",
        "rutas/relativa-con-punto-punto ok",
        "rutas/una-ruta-demasiado-larga-se-rechaza ok",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta el caso «{esperado}»: {salida:?}"));
        }
    }
    Ok(())
}

/// N-008, paso 1 — ¿basta con sondear `stat` para enterarse de un cambio?
///
/// **No juzga la parte que decide: informa.** La ficha tiene que elegir entre
/// eventos en el kernel y sondeo, y poner aquí un umbral sería inventarme el
/// criterio antes de mirar el dato — la lección del paso 1 de N-001.
fn ssh_probe_vigilancia(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe vigilancia\nexit\n",
        Duration::from_secs(300),
    )?;
    if !salida.contains("probe-json:") {
        return Err(format!("la medida no llegó al informe final: {salida:?}"));
    }
    publicar_informe("vigilancia", &salida);
    for l in salida.lines() {
        let l = l.trim();
        if l.starts_with("vigilancia: ") {
            println!("      [vigilancia] {l}");
        }
    }
    for esperado in [
        "vigilancia/un-cambio-de-tamano-se-nota ok",
        "vigilancia/las-dos-escrituras-eran-del-mismo-tamano ok",
        "vigilancia/el-contenido-si-cambio ok",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta el caso «{esperado}»: {salida:?}"));
        }
    }
    Ok(())
}

/// N-010 — qué subconjunto de shell promete `sosh`.
///
/// Lo que se comprueba no es que falten operadores, sino que **no se cuelen
/// como argumentos**. `echo hola &` imprimiendo «hola &» es peor que un error:
/// quien lo escribió cree que lanzó algo en segundo plano.
///
/// Cada línea lleva su marca para que el fallo diga cuál falló, y las dos
/// últimas son el **control**: lo que sí se sabe hacer tiene que seguir
/// funcionando, y entrecomillar tiene que ser la salida de emergencia.
fn ssh_sosh_subconjunto(key: &Path, ssh_port: u16) -> Result<(), String> {
    let script_tag = "sosh_guion_e2e";
    let tag_and = "TAG_AND_SKIP_771";
    let tag_and2 = "TAG_AND2_SKIP_772";
    let tag_or = "TAG_OR_SKIP_773";
    let guion = format!(
        concat!(
            "echo uno &\n",
            "echo dos ; echo tres\n",
            "echo cuatro && echo cinco\n",
            "cd /no-existe-soso-xyz-a && echo {tag_and}\n",
            "cd /no-existe-soso-xyz && echo {tag_and2}\n",
            "echo si-aparece || echo {tag_or}\n",
            "cd /no-existe-soso-xyz || echo si-or\n",
            "ls 2>&1\n",
            "ls *.rs\n",
            "echo $HOME\n",
            "echo \"seis;siete\"\n",
            "echo ocho | grep ocho\n",
            "echo '# guion' > /tmp/sosh-test.sh\n",
            "echo 'echo {tag}-a' >> /tmp/sosh-test.sh\n",
            "echo 'cd /no-existe-soso-xyz && echo {tag}-skip' >> /tmp/sosh-test.sh\n",
            "echo 'echo {tag}-b' >> /tmp/sosh-test.sh\n",
            "sosh /tmp/sosh-test.sh\n",
            "echo FIN-SOSH-SUB\n",
            "exit\n",
        ),
        tag = script_tag,
        tag_and = tag_and,
        tag_and2 = tag_and2,
        tag_or = tag_or,
    );
    let salida = ssh_guion_hasta(key, ssh_port, &guion, Duration::from_secs(120), "FIN-SOSH-SUB")?;
    for (etiqueta, marca) in [
        ("& en segundo plano", "sosh: no sé ejecutar en segundo plano"),
        ("2>&1", "sosh: no sé duplicar descriptores"),
        ("* como comodín", "sosh: no expando comodines"),
        ("$ como variable", "sosh: no expando variables"),
    ] {
        if !salida.contains(marca) {
            return Err(format!("«{etiqueta}» no se rechazó ({marca:?}); salida: {salida:?}"));
        }
    }
    for (etiqueta, marca) in [
        ("; encadena", "tres"),
        ("&& encadena", "cinco"),
        ("entrecomillado literal", "seis;siete"),
        ("pipe", "ocho"),
        ("guion -a", &format!("{script_tag}-a")),
        ("guion -b", &format!("{script_tag}-b")),
        ("|| alternativa", "si-or"),
    ] {
        if !salida.contains(marca) {
            return Err(format!("«{etiqueta}» falló ({marca:?}); salida: {salida:?}"));
        }
    }
    // El eco de la consola repite la orden tecleada; si &&/|| cortan bien, el
    // marcador sólo aparece una vez (en la línea del prompt), no como salida.
    for (etiqueta, marca, max) in [
        ("&& corta tras fallo", tag_and, 1usize),
        ("&& corta cd", tag_and2, 1usize),
        ("|| corta si ok", tag_or, 1usize),
        ("guion && en fichero", &format!("{script_tag}-skip"), 1usize),
    ] {
        let n = salida.matches(marca).count();
        if n > max {
            return Err(format!(
                "«{etiqueta}» apareció {n} veces (máx {max}, {marca:?}); salida: {salida:?}"
            ));
        }
    }
    Ok(())
}

/// N-009 — qué promete el buscador de soso.
///
/// Los casos miran el **código de salida** y no el texto: es lo que un
/// programa que llame a la herramienta puede interpretar sin adivinar, y las
/// tres formas de mentir que se vigilan —«no hay» por «no pude», una regex
/// buscada tal cual, y una línea no-UTF8 saltada en silencio— se distinguen
/// ahí.
fn ssh_probe_busqueda(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe busqueda\nexit\n",
        Duration::from_secs(240),
    )?;
    publicar_informe("busqueda", &salida);
    for esperado in [
        "busqueda/hay-coincidencias-sale-0 ok",
        "busqueda/sin-coincidencias-sale-1 ok",
        "busqueda/una-regex-se-rechaza-en-vez-de-mentir ok",
        "busqueda/con-F-se-busca-literal ok",
        "busqueda/un-byte-invalido-no-esconde-el-resto ok",
        "busqueda/numera-las-lineas ok",
        "busqueda/recursivo-baja-a-los-subdirectorios ok",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta el caso «{esperado}»: {salida:?}"));
        }
    }
    Ok(())
}

/// N-005 — el experimento de carga de código ajeno.
///
/// Dos medidas en direcciones contrarias: C compilado desde fuente y enlazado
/// **sí** corre; un ELF que no es `ET_EXEC` **no** arranca. La segunda lleva su
/// control —la misma copia sin tocar, que sí arranca—, porque «no arrancó»
/// también lo diría un fallo de ruta o de permisos.
fn ssh_probe_carga(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe carga\nexit\n",
        Duration::from_secs(240),
    )?;
    publicar_informe("carga", &salida);
    for esperado in [
        "carga/c-ajeno-compilado-y-enlazado ok",
        "carga/c-ajeno-escribe-en-memoria-del-llamante ok",
        "carga/copia-intacta-arranca ok",
        "carga/un-et-dyn-no-arranca ok",
    ] {
        if !salida.contains(esperado) {
            return Err(format!("falta el caso «{esperado}»: {salida:?}"));
        }
    }
    Ok(())
}

/// N-001, paso 1 — cuánto cuesta una escritura en sosofs.
///
/// No juzga: **informa**. N-001 tiene que elegir entre dos formas de cambiar el
/// modelo de ficheros, y la segunda cuesta una transacción CoW por `write`.
/// Poner aquí un umbral sería inventarme el criterio que esa ficha debe decidir
/// con el dato delante.
fn ssh_probe_coste(key: &Path, ssh_port: u16) -> Result<(), String> {
    let salida = ssh_guion(
        key,
        ssh_port,
        "soso-agent-probe coste\nexit\n",
        Duration::from_secs(300),
    )?;
    if !salida.contains("probe-json:") {
        return Err(format!("la medida no llegó al informe final: {salida:?}"));
    }
    publicar_informe("coste", &salida);
    for l in salida.lines() {
        let l = l.trim();
        if l.starts_with("coste: ") {
            println!("      [coste] {l}");
        }
    }
    if salida.contains("FALLO") {
        return Err(format!("la medida no cuadró: {salida:?}"));
    }
    Ok(())
}

fn ssh_sosh_comillas(key: &Path, ssh_port: u16) -> Result<(), String> {
    let tok = "xtask_t64_ok";
    let guion = format!(
        concat!(
            "mkdir /tmp/t64\n",
            "echo {tok} > \"/tmp/t64/con espacio.txt\"\n",
            "cat \"/tmp/t64/con espacio.txt\"\n",
            "cat '/tmp/t64/con espacio.txt'\n",
            "cat \"/tmp/t64/no existe.txt\"\n",
            "cat \"sin cerrar\n",
            "echo FIN-T64\n"
        ),
        tok = tok
    );
    let texto = ssh_guion_hasta(key, ssh_port, &guion, Duration::from_secs(90), "FIN-T64")?;

    // Las dos comillas valen y las dos leen el mismo fichero.
    let lecturas = texto.matches(tok).count();
    if lecturas < 3 {
        return Err(format!(
            "esperaba el eco y dos lecturas de «con espacio.txt», hubo {lecturas}: {texto:?}"
        ));
    }
    // La ruta no se partió: nadie se quejó de «/tmp/t64/con» a secas.
    if texto.contains("/tmp/t64/con:") || texto.contains("espacio.txt:") {
        return Err(format!("la ruta se partió en palabras: {texto:?}"));
    }
    // Un fichero que no existe sigue dando un error con el nombre **entero**.
    if !texto.contains("/tmp/t64/no existe.txt") {
        return Err(format!("el error no llevó la ruta entera: {texto:?}"));
    }
    // Y una comilla sin cerrar se dice, no se adivina.
    if !texto.contains("sin cerrar") || !texto.contains("comilla doble sin cerrar") {
        return Err(format!("la comilla sin cerrar no se rechazó: {texto:?}"));
    }
    Ok(())
}

fn ssh_fd3_log(key: &Path, ssh_port: u16) -> Result<(), String> {
    let tok_f = "xtask_fd3_f_42";
    let tok_r = "xtask_fd3_r_42";
    // **Sin `halt`.** Este guion acababa apagando la máquina y usaba «halt»
    // como marcador de fin. El paso se marcaba OK y el siguiente se encontraba
    // el guest apagado: «Connection refused», reintento y reinicio de 40 s en
    // cada pasada de la suite. Parecía intermitente porque a veces el apagado
    // aún no había terminado cuando el paso siguiente conectaba. Un marcador
    // propio termina el guion igual de bien y deja la máquina viva para quien
    // venga detrás — apagar es cosa del último paso del shard.
    let guion = format!(
        "init log {tok_f} 3>/tmp/l.txt\ncat /tmp/l.txt\ninit log {tok_r}\nlog\necho FIN-FD3\n"
    );
    let texto = ssh_guion_hasta(key, ssh_port, &guion, Duration::from_secs(60), "FIN-FD3")?;
    let cat_chunk = texto
        .split("cat /tmp/l.txt")
        .nth(1)
        .and_then(|s| s.split("init log").next())
        .unwrap_or("");
    if !cat_chunk.contains(tok_f) {
        return Err(format!("cat no mostró {tok_f}; salida: {texto:?}"));
    }
    if cat_chunk.contains("pid=") {
        return Err(format!(
            "3>fichero incluyó sello del kernel; trozo: {cat_chunk:?}"
        ));
    }
    if !texto.contains(tok_r) {
        return Err(format!("no se vio {tok_r} en log; salida: {texto:?}"));
    }
    if !texto.contains("pid=") {
        return Err(format!("log no incluyó sello pid=; salida: {texto:?}"));
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

/// El SSH del guion ya ha salido cuando el arnés recoge; `kill` entonces imprime
/// `kill: (pid): No such process` y parece un fallo del shard.
fn kill_pid_quiet(pid: u32) {
    let _ = Command::new("kill")
        .arg(pid.to_string())
        .stderr(Stdio::null())
        .status();
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
            kill_pid_quiet(pid);
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
                kill_pid_quiet(pid);
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
                let bios = copiar_imagen(&img, esc.id, super::firmware_kind(&img));
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
        llm_host_port: None,
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

#[cfg(test)]
mod tests {
    use super::texto_generado_antes_de_resumen;

    #[test]
    fn q4k_ignora_el_aviso_gsp_del_dispositivo_software() {
        let texto = "\
alpha\n\
soso-llm: dispositivo «soft (CPU del kernel, pruebas)» — 10 matvec, 4 subidas de pesos, 4 matrices residentes, 0 sin sitio (a CPU), último on_gpu=0\n\
soso-llm: subidas — 4 de 4 en crudo (sin expandir a f32), 0 Mciclos descuantizando, 0 Mciclos en gpu_map\n\
soso-llm: el silicio no calculó nada — el GSP se quedó en la fase «»\n\
soso-llm: generado (3 tokens, 1 ms, 1.00 tok/s)\n\
";
        let lineas: Vec<&str> = texto.lines().collect();
        let i = lineas
            .iter()
            .position(|l| l.contains("soso-llm: generado"))
            .unwrap();
        assert_eq!(texto_generado_antes_de_resumen(&lineas, i), "alpha");
    }

    #[test]
    fn q4k_tokens_vacios_no_se_confunden_con_el_diagnostico() {
        let texto = "\
soso-llm: backend CPU (+0 ms)\n\
\n\
soso-llm: generado (3 tokens, 1 ms, 1.00 tok/s)\n\
";
        let lineas: Vec<&str> = texto.lines().collect();
        let i = lineas
            .iter()
            .position(|l| l.contains("soso-llm: generado"))
            .unwrap();
        assert_eq!(texto_generado_antes_de_resumen(&lineas, i), "");
    }

    #[test]
    fn q4k_cpu_y_gpu_soft_dan_el_mismo_texto() {
        let texto = "\
hola\n\
soso-llm: generado (3 tokens, 10 ms, 0.30 tok/s)\n\
hola\n\
soso-llm: dispositivo «soft (CPU del kernel, pruebas)» — 8 matvec, 4 subidas de pesos\n\
soso-llm: el silicio no calculó nada — el GSP se quedó en la fase «»\n\
soso-llm: generado (3 tokens, 40 ms, 0.08 tok/s)\n\
";
        let lineas: Vec<&str> = texto.lines().collect();
        let salidas: Vec<&str> = lineas
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains("soso-llm: generado"))
            .map(|(i, _)| texto_generado_antes_de_resumen(&lineas, i))
            .collect();
        assert_eq!(salidas, ["hola", "hola"]);
    }
}

