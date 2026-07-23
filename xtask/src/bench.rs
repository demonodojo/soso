//! `cargo xtask bench-llm`: mide tok/s del decode con modelo sintético bajo SMP.
//!
//! Variables de entorno:
//! - `SOSO_QEMU_MEM` (default 8G)
//! - `SOSO_BENCH_SMP` lista separada por comas (default 1,4)
//! - `SOSO_BENCH_MAX` tokens a generar (default 8)
//! - `SOSO_BENCH_TIMEOUT_SECS` (default 900)

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub fn run() {
    if std::net::TcpStream::connect(("127.0.0.1", 2222)).is_ok() {
        eprintln!("bench-llm: puerto 2222 ocupado — cierra QEMU previo antes de medir");
        std::process::exit(1);
    }

    let root = super::project_root();
    if std::env::var("SOSO_QEMU_MEM").is_err() {
        unsafe {
            std::env::set_var("SOSO_QEMU_MEM", "8G");
        }
    }
    super::build_user();
    let img = super::build_image();
    let data = super::mkfs_rootfs(true);

    let bench_dir = root.join("target/bench-model");
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["run", "-q", "--release", "-p", "mkmodel-soso", "--"])
        .arg(&bench_dir)
        .args([
            "--name",
            "bench",
            "--layers",
            "4",
            "--hidden",
            "1024",
            "--ffn",
            "2816",
            "--vocab",
            "2048",
        ])
        .status()
        .expect("mkmodel-soso bench");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    unsafe {
        std::env::set_var("SOSO_MODELS_DIR", &bench_dir);
        std::env::set_var("SOSO_MODELS_SIZE", "512M");
    }
    let models = super::mkfs_models(true);
    let key = root.join("target/soso_test_key");

    let smps: Vec<u32> = std::env::var("SOSO_BENCH_SMP")
        .unwrap_or_else(|_| "1,4".into())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let max_new: u32 = std::env::var("SOSO_BENCH_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let timeout_secs: u64 = std::env::var("SOSO_BENCH_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(900);

    println!(
        "bench-llm: modelo bench (~128 MiB pesos), --max {max_new}, timeout {timeout_secs}s"
    );
    println!("bench-llm: mem={}", super::qemu_mem());

    let mut rows: Vec<(u32, u32, f64)> = Vec::new();

    for smp in smps {
        unsafe {
            std::env::set_var("SOSO_QEMU_SMP", smp.to_string());
        }
        let serial = root.join(format!("target/bench-smp{smp}.log"));
        let _ = std::fs::remove_file(&serial);

        let mut qemu = lanzar_qemu(&img, &data, &models, &serial).expect("QEMU");
        let boot = crate::test::esperar_en_fichero(&serial, "sosh —", Duration::from_secs(120));
        if boot.is_err() {
            eprintln!("bench-llm: SMP={smp} no arrancó (ver {})", serial.display());
            let _ = qemu.kill();
            let _ = qemu.wait();
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_secs(2));

        // Warmup: carga mmap + primera pasada de pesos.
        if ssh_cmd(&key, "soso-llm run bench --prompt x --max 1\nexit\n", timeout_secs).is_err()
        {
            eprintln!("bench-llm: SMP={smp} warmup falló");
            let _ = qemu.kill();
            let _ = qemu.wait();
            std::process::exit(1);
        }

        let cmd = format!("soso-llm run bench --prompt x --max {max_new}\nhalt\n");
        match ssh_cmd(&key, &cmd, timeout_secs) {
            Ok(out) => {
                if let Some((workers, tok_s)) = parse_bench_line(&out) {
                    println!("bench-llm: SMP={smp} workers={workers} {tok_s:.2} tok/s");
                    rows.push((smp, workers, tok_s));
                } else {
                    eprintln!("bench-llm: SMP={smp} salida inesperada: {out:?}");
                    let _ = qemu.kill();
                    let _ = qemu.wait();
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("bench-llm: SMP={smp} {e}");
                let _ = qemu.kill();
                let _ = qemu.wait();
                std::process::exit(1);
            }
        }

        let _ = crate::test::espera_salida(&mut qemu, Duration::from_secs(15));
        let _ = qemu.wait();
    }

    println!("\nbench-llm: resumen");
    println!("  SMP  workers  tok/s");
    for (smp, workers, tok_s) in &rows {
        println!("  {smp:>3}  {workers:>7}  {tok_s:.2}");
    }
    if rows.len() >= 2 {
        let base = rows[0].2;
        if base > 0.0 {
            for (smp, _, tok_s) in rows.iter().skip(1) {
                println!("  SMP={smp}: ×{:.2} vs SMP=1", tok_s / base);
            }
        }
    }
}

fn lanzar_qemu(
    img: &std::path::Path,
    data: &std::path::Path,
    models: &std::path::Path,
    serial: &std::path::Path,
) -> std::io::Result<Child> {
    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35", "-cpu", "max"])
        .args(["-m", &super::qemu_mem()])
        .args(["-smp", &super::qemu_smp()]);
    super::apply_firmware(&mut qemu, img);
    qemu.args(["-drive", &format!("format=raw,file={}", img.display())]);
    super::apply_qemu_disks(&mut qemu, data, models);
    super::apply_qemu_nic(&mut qemu);
    super::apply_qemu_gpu(&mut qemu);
    qemu.args(["-serial", &format!("file:{}", serial.display())])
        .args(["-display", "none"])
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .arg("-no-reboot")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

fn ssh_cmd(key: &std::path::Path, script: &str, timeout_secs: u64) -> Result<String, String> {
    let mut hijo = Command::new("ssh")
        .args(["-tt", "-i"])
        .arg(key)
        .args(["-p", "2222"])
        .args(["-o", "StrictHostKeyChecking=no"])
        .args(["-o", "UserKnownHostsFile=/dev/null"])
        .args(["-o", "LogLevel=ERROR"])
        .args(["-o", "ConnectTimeout=10"])
        .arg("soso@localhost")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ssh: {e}"))?;

    let pid = hijo.id();
    let mut stdin = hijo.stdin.take().unwrap();
    stdin.write_all(script.as_bytes()).map_err(|e| e.to_string())?;
    stdin.flush().ok();

    let handle = std::thread::spawn(move || hijo.wait_with_output());

    let fin = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        if handle.is_finished() {
            break;
        }
        if Instant::now() >= fin {
            drop(stdin);
            let _ = Command::new("kill").arg(pid.to_string()).status();
            let _ = handle.join();
            return Err(format!("timeout tras {timeout_secs}s"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    drop(stdin);

    let salida = handle
        .join()
        .map_err(|_| "hilo ssh".to_string())?
        .map_err(|e| e.to_string())?;
    let texto = String::from_utf8_lossy(&salida.stdout);
    if texto.contains("soso-llm: generado") {
        Ok(texto.into_owned())
    } else {
        Err(format!(
            "ssh exit {} stdout={texto:?} stderr={}",
            salida.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&salida.stderr)
        ))
    }
}

/// Parsea `workers=N` y la línea `generado (... tok/s)`.
fn parse_bench_line(out: &str) -> Option<(u32, f64)> {
    let workers = out
        .lines()
        .find_map(|l| l.strip_prefix("soso-llm: workers="))
        .and_then(|v| v.trim().parse().ok())?;
    let line = out.lines().find(|l| l.contains("soso-llm: generado"))?;
    if let Some(rest) = line.split(',').nth(2) {
        if let Some(v) = rest.trim().strip_suffix(" tok/s)") {
            if let Ok(tok_s) = v.parse::<f64>() {
                return Some((workers, tok_s));
            }
        }
    }
    // compat: calcular desde tokens y ms
    let tokens: u32 = line
        .split('(')
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let ms: u64 = line
        .split(',')
        .nth(1)?
        .trim()
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((workers, tokens as f64 * 1000.0 / ms.max(1) as f64))
}
