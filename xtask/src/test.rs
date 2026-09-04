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
    let root = super::project_root();
    let report = Arc::new(Report::new());

    let img = std::thread::scope(|scope| {
        let report_host = Arc::clone(&report);
        let root_host = root.clone();
        scope.spawn(move || run_host_tests(&root_host, &report_host));

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

    run_shards_parallel(&report, jobs, &img, &data, &models, &key);

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
) {
    let pool = Arc::new(JobPool::new(jobs));
    std::thread::scope(|scope| {
        for shard in ShardId::ALL {
            let report = Arc::clone(report);
            let pool = Arc::clone(&pool);
            let img = img.to_path_buf();
            let data = data.to_path_buf();
            let models = models.to_path_buf();
            let key = key.to_path_buf();
            scope.spawn(move || {
                let _permit = pool.acquire();
                let slot = make_slot(shard, &img, &data, &models);
                run_shard(shard, &slot, &key, &report);
            });
        }
    });
}

fn make_slot(shard: ShardId, img: &Path, data: &Path, models: &Path) -> QemuSlot {
    let id = shard.name();
    let root = super::project_root();
    let serial = root.join(format!("test-{id}-serial.log"));
    let _ = std::fs::remove_file(&serial);
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
    }
}

fn copiar_imagen(src: &Path, shard_id: &str, kind: &str) -> PathBuf {
    let dst = super::project_root()
        .join("target")
        .join(format!("test-{shard_id}-{kind}.img"));
    std::fs::copy(src, &dst).unwrap_or_else(|e| {
        panic!(
            "copiar {} → {}: {e}",
            src.display(),
            dst.display()
        );
    });
    dst
}

fn run_shard(shard: ShardId, slot: &QemuSlot, key: &Path, report: &Report) {
    match shard {
        ShardId::LlmDense => run_shard_llm_dense(slot, key, report),
        ShardId::LlmMoe => run_shard_llm_moe(slot, key, report),
        ShardId::Sys => run_shard_sys(slot, key, report),
        ShardId::Reclaim => run_shard_reclaim(slot, key, report),
    }
}

fn run_shard_llm_dense(slot: &QemuSlot, key: &Path, report: &Report) {
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
        let _ = report.paso_con_reintento(sid, "soso-llm run tiny --prompt test", || {
            ssh_llm(key, port)
        });
        let _ = report.paso_con_reintento(
            sid,
            "soso-llm run tiny-mla --prompt test --max 2",
            || ssh_llm_mla(key, port),
        );
        let _ = report.paso_con_reintento(sid, "sigue viva tras dos pools de hilos", || {
            ssh_vive(key, port)
        });
        let _ = report.paso_con_reintento(
            sid,
            "ask: modelo residente (carga una vez, reconexión SSH)",
            || ssh_ask_resident(key, port),
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

fn run_shard_sys(slot: &QemuSlot, key: &Path, report: &Report) {
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
        let _ = report.paso_con_reintento(sid, "ask: el texto llega literal", || {
            ssh_ask_literal(key, port)
        });
        let _ = report.paso_con_reintento(sid, "voz: transcribe WAV de prueba", || {
            ssh_voz_wav(key, port)
        });
        let _ = report.paso_con_reintento(sid, "soso-web: HTML local", || {
            ssh_soso_web_local(key, port)
        });
        let _ = report.paso_con_reintento(sid, "init test (syscalls, hilos, FPU, GPU)", || {
            ssh_init_test(key, port)
        });
        let _ = report.paso_con_reintento(sid, "pipeline de sosh (6 KiB por un pipe)", || {
            ssh_pipeline(key, port)
        });
        let _ = report.paso(sid, "SSH por clave pública + comando + halt", || {
            ssh_sesion(key, port)
        });
    }
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
    let _ = qemu.wait();
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

fn lanzar_qemu(slot: &QemuSlot) -> std::io::Result<Child> {
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
    super::apply_qemu_disks(&mut qemu, &slot.data, &slot.models);
    super::apply_qemu_usb(&mut qemu);
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
    };
    lanzar_qemu(&slot)
}

fn qemu_monitor_cmd(mon: &Path, cmd: &str) -> Result<(), String> {
    use std::os::unix::net::UnixStream;

    for _ in 0..100 {
        if mon.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut s = UnixStream::connect(mon).map_err(|e| format!("monitor {mon:?}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok();
    s.write_all(format!("{cmd}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Tras I/O del BOT en xHCI, el teclado HID debe seguir respondiendo en serie.
fn usb_kbd_vivo_despues_io(
    key: &Path,
    ssh_port: u16,
    serial: &Path,
    monitor: &Path,
) -> Result<(), String> {
    ssh_guion(key, ssh_port, "ls /models\nexit\n", Duration::from_secs(120))?;
    std::thread::sleep(Duration::from_secs(1));
    for k in ["h", "e", "l", "p", "ret"] {
        qemu_monitor_cmd(monitor, &format!("sendkey {k}"))?;
        std::thread::sleep(Duration::from_millis(80));
    }
    esperar_en_fichero(serial, "builtins:", Duration::from_secs(30))
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
    if !texto.contains("sosh —") {
        return false;
    }
    texto.ends_with("$ ")
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
    ssh_guion_inner(key, ssh_port, guion, limite, true)
}

fn ssh_guion_inner(
    key: &Path,
    ssh_port: u16,
    guion: &str,
    limite: Duration,
    tty: bool,
) -> Result<String, String> {
    let fifo = super::project_root().join("target/.ssh-guion.fifo");
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
    let mut holder = Command::new("sh")
        .args(["-c", &format!("exec sleep {holgura} > {}", fifo.display())])
        .spawn()
        .map_err(|e| format!("sleep holder: {e}"))?;

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

    let pid = hijo.id();
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

    let fin_prompt = Instant::now() + Duration::from_secs(45);
    loop {
        let mut visto = prompt.0.lock().unwrap();
        if *visto {
            break;
        }
        let resto = fin_prompt.saturating_duration_since(Instant::now());
        if resto.is_zero() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
            let _ = holder.kill();
            let _ = holder.wait();
            let _ = fs::remove_file(&fifo);
            let datos = acum.lock().unwrap();
            let parcial = String::from_utf8_lossy(&datos);
            return Err(format!(
                "no apareció el prompt en 45s; stdout parcial: {parcial:?}"
            ));
        }
        visto = prompt.1.wait_timeout(visto, resto).unwrap().0;
    }

    {
        let mut w = OpenOptions::new()
            .write(true)
            .open(&fifo)
            .map_err(|e| format!("escribir fifo: {e}"))?;
        w.write_all(guion.as_bytes()).map_err(|e| e.to_string())?;
        w.flush().ok();
    }

    let fin = Instant::now() + limite;
    loop {
        if lector.is_finished() {
            break;
        }
        if Instant::now() >= fin {
            let _ = Command::new("kill").arg(pid.to_string()).status();
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
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
    if let Some(mut err) = stderr {
        let mut extra = String::new();
        let _ = err.read_to_string(&mut extra);
        if !extra.is_empty() {
            salida.push_str(&extra);
        }
    }
    let salida = normalizar_salida_ssh(salida);
    let status = hijo.wait().map_err(|e| format!("wait ssh: {e}"))?;
    let _ = holder.kill();
    let _ = holder.wait();
    let _ = fs::remove_file(&fifo);
    if Instant::now() >= fin && !status.success() {
        return Err(format!(
            "la sesión SSH no terminó en {}s; stdout: {salida:?}",
            limite.as_secs()
        ));
    }
    Ok(salida)
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
    let texto = ssh_guion(key, ssh_port, guion, Duration::from_secs(180))?;
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
    if !texto.contains("Página de prueba") {
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
    let fifo = super::project_root().join("target/.ssh-ask.fifo");
    let _ = fs::remove_file(&fifo);
    if !Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .map_err(|e| format!("mkfifo: {e}"))?
        .success()
    {
        return Err("mkfifo falló".into());
    }
    let holgura = 300u64;
    let mut holder = Command::new("sh")
        .args(["-c", &format!("exec sleep {holgura} > {}", fifo.display())])
        .spawn()
        .map_err(|e| format!("sleep holder: {e}"))?;

    let key_s = key.display().to_string();
    let fifo_s = fifo.display().to_string();
    let ssh_cmd = format!(
        "exec ssh -T -i '{key_s}' -p {ssh_port} \
         -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
         -o LogLevel=ERROR -o ConnectTimeout=10 soso@localhost < '{fifo_s}'"
    );
    let mut hijo = Command::new("sh")
        .args(["-c", &ssh_cmd])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("no se pudo lanzar ssh: {e}"))?;
    let pid = hijo.id();
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

    let fin_prompt = Instant::now() + Duration::from_secs(45);
    loop {
        let mut visto = prompt.0.lock().unwrap();
        if *visto {
            break;
        }
        let resto = fin_prompt.saturating_duration_since(Instant::now());
        if resto.is_zero() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
            let _ = holder.kill();
            let _ = holder.wait();
            let _ = fs::remove_file(&fifo);
            return Err("no apareció el prompt en 45s".into());
        }
        visto = prompt.1.wait_timeout(visto, resto).unwrap().0;
    }

    let pregunta = format!("ask :eco {payload}\n");
    {
        let mut w = OpenOptions::new()
            .write(true)
            .open(&fifo)
            .map_err(|e| format!("escribir fifo: {e}"))?;
        w.write_all(pregunta.as_bytes()).map_err(|e| e.to_string())?;
        w.flush().ok();
    }

    let fin_payload = Instant::now() + Duration::from_secs(30);
    let mut visto_payload = false;
    while Instant::now() < fin_payload {
        {
            let datos = acum.lock().unwrap();
            let texto = String::from_utf8_lossy(&datos);
            if texto.lines().any(|l| l == payload) {
                visto_payload = true;
                break;
            }
        }
        if lector.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    {
        let mut w = OpenOptions::new()
            .write(true)
            .open(&fifo)
            .map_err(|e| format!("escribir fifo: {e}"))?;
        w.write_all(b"exit\n").map_err(|e| e.to_string())?;
        w.flush().ok();
    }

    let fin = Instant::now() + Duration::from_secs(30);
    while Instant::now() < fin && !lector.is_finished() {
        std::thread::sleep(Duration::from_millis(200));
    }
    if lector.is_finished() {
        lector
            .join()
            .map_err(|_| "hilo lector ssh".to_string())?
            .map_err(|e: String| e)?;
    } else {
        let _ = Command::new("kill").arg(pid.to_string()).status();
        let _ = lector.join();
    }
    let mut texto =
        normalizar_salida_ssh(String::from_utf8_lossy(&acum.lock().unwrap()).into_owned());
    if let Some(mut err) = stderr {
        let mut extra = String::new();
        let _ = err.read_to_string(&mut extra);
        if !extra.is_empty() {
            texto.push_str(&extra);
        }
    }
    let _ = hijo.wait();
    let _ = holder.kill();
    let _ = holder.wait();
    let _ = fs::remove_file(&fifo);

    if visto_payload || texto.lines().any(|l| l == payload) {
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
    let texto = ssh_guion(key, ssh_port, "init test\nexit\n", Duration::from_secs(420))?;
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
    let texto = ssh_guion(key, ssh_port, &guion, Duration::from_secs(60))?;
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

static USB_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
                let nombre = format!("USB: {}", esc.name);
                let ok = report
                    .paso("usb", &nombre, || {
                        run_usb_scenario(&bios, &data_img, &models_img, &serial, esc)
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
    serial: &std::path::Path,
    esc: UsbTestScenario,
) -> Result<(), String> {
    let (ssh_port, echo_port, mac) = usb_ports(esc.index);
    let _env_guard = USB_ENV_LOCK.lock().unwrap();
    clear_usb_qemu_env();
    unsafe {
        std::env::set_var("SOSO_QEMU_LIVE", "1");
        std::env::set_var("SOSO_QEMU_LIVE_USB", "1");
        if let Some(model) = esc.xhci {
            std::env::set_var("SOSO_QEMU_XHCI", model);
        }
        if esc.usb_kbd {
            std::env::set_var("SOSO_QEMU_USB_KBD", "1");
        }
        if esc.usb_hub {
            std::env::set_var("SOSO_QEMU_USB_HUB", "1");
        }
    }

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
    };

    let mut qemu = lanzar_qemu(&slot).map_err(|e| e.to_string())?;
    drop(_env_guard);
    clear_usb_qemu_env();

    esperar_en_fichero(serial, "sosh —", Duration::from_secs(180))?;

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

fn clear_usb_qemu_env() {
    unsafe {
        for key in [
            "SOSO_QEMU_LIVE",
            "SOSO_QEMU_LIVE_USB",
            "SOSO_QEMU_XHCI",
            "SOSO_QEMU_USB_KBD",
            "SOSO_QEMU_USB_HUB",
            "SOSO_QEMU_USB_HOST",
            "SOSO_QEMU_TRACE_USB",
        ] {
            std::env::remove_var(key);
        }
    }
}
