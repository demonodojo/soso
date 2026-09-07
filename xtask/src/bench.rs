//! `cargo xtask bench-llm`: mide tok/s del decode con modelo sintético bajo SMP.
//!
//! Variables de entorno:
//! - `SOSO_QEMU_MEM` (default 8G)
//! - `SOSO_BENCH_SMP` lista separada por comas (default 1)
//! - `SOSO_BENCH_MAX` tokens a generar (default 8)
//! - `SOSO_BENCH_TIMEOUT_SECS` (default 900)
//! - `SOSO_BENCH_MATRIX=0` solo baseline denso (compat)

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct BenchScenario {
    model: &'static str,
    mem_flag: &'static str,
    label: &'static str,
}

struct BenchRow {
    label: String,
    smp: u32,
    workers: u32,
    tok_s: f64,
}

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
    let bench_packed = root.join("target/bench-model-packed");
    mkmodel_bench(&root, &bench_dir, false);
    mkmodel_bench(&root, &bench_packed, true);

    let models = mkfs_bench_models(&root, &[bench_dir.clone(), bench_packed.clone()]);
    let key = root.join("target/soso_test_key");

    let matrix_mode = std::env::var("SOSO_BENCH_MATRIX").unwrap_or_else(|_| "1".into()) != "0";
    let scenarios: Vec<BenchScenario> = if matrix_mode {
        vec![
            BenchScenario {
                model: "bench",
                mem_flag: "",
                label: "dense",
            },
            BenchScenario {
                model: "bench-packed",
                mem_flag: "",
                label: "packed",
            },
            BenchScenario {
                model: "bench",
                mem_flag: "--mem-tight",
                label: "dense+mem-tight",
            },
            BenchScenario {
                model: "bench-packed",
                mem_flag: "--mem-tight",
                label: "packed+mem-tight",
            },
        ]
    } else {
        vec![BenchScenario {
            model: "bench",
            mem_flag: "",
            label: "dense",
        }]
    };

    let smps: Vec<u32> = std::env::var("SOSO_BENCH_SMP")
        .unwrap_or_else(|_| "1".into())
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
        "bench-llm: modelos bench + bench-packed (~128 MiB), --max {max_new}, timeout {timeout_secs}s"
    );
    println!("bench-llm: mem={}", super::qemu_mem());

    let mut rows: Vec<BenchRow> = Vec::new();

    for smp in &smps {
        for sc in &scenarios {
            let serial = root.join(format!(
                "target/bench-smp{smp}-{}.log",
                sc.label.replace('+', "-")
            ));
            let _ = std::fs::remove_file(&serial);

            let mut qemu = lanzar_qemu(&img, &data, &models, &serial).expect("QEMU");
            unsafe {
                std::env::set_var("SOSO_QEMU_SMP", smp.to_string());
            }
            let boot = crate::test::esperar_en_fichero(&serial, "sosh —", Duration::from_secs(120));
            if boot.is_err() {
                eprintln!(
                    "bench-llm: SMP={smp} {} no arrancó (ver {})",
                    sc.label,
                    serial.display()
                );
                let _ = qemu.kill();
                let _ = qemu.wait();
                std::process::exit(1);
            }
            std::thread::sleep(Duration::from_secs(2));

            let warmup = format!("soso-llm run {} --prompt x --max 1\nhalt\n", sc.model);
            if ssh_cmd(&key, &warmup, timeout_secs).is_err() {
                eprintln!("bench-llm: SMP={smp} {} warmup falló", sc.label);
                let _ = qemu.kill();
                let _ = qemu.wait();
                std::process::exit(1);
            }

            let cmd = if sc.mem_flag.is_empty() {
                format!(
                    "soso-llm run {} --prompt x --max {max_new}\nhalt\n",
                    sc.model
                )
            } else {
                format!(
                    "soso-llm run {} --prompt x --max {max_new} {}\nhalt\n",
                    sc.model, sc.mem_flag
                )
            };
            match ssh_cmd(&key, &cmd, timeout_secs) {
                Ok(out) => {
                    if let Some((workers, tok_s)) = parse_bench_line(&out) {
                        println!(
                            "bench-llm: SMP={smp} {} workers={workers} {tok_s:.2} tok/s",
                            sc.label
                        );
                        echo_telemetry(&out, *smp, sc.label);
                        rows.push(BenchRow {
                            label: sc.label.to_string(),
                            smp: *smp,
                            workers,
                            tok_s,
                        });
                    } else {
                        eprintln!(
                            "bench-llm: SMP={smp} {} salida inesperada: {out:?}",
                            sc.label
                        );
                        let _ = qemu.kill();
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("bench-llm: SMP={smp} {} {e}", sc.label);
                    let _ = qemu.kill();
                    std::process::exit(1);
                }
            }

            let _ = crate::test::espera_salida(&mut qemu, Duration::from_secs(15));
            let _ = qemu.wait();
        }
    }

    println!("\nbench-llm: resumen");
    println!("  escenario          SMP  workers  tok/s");
    for row in &rows {
        println!(
            "  {:<18}  {:>3}  {:>7}  {:.2}",
            row.label, row.smp, row.workers, row.tok_s
        );
    }
    if rows.len() >= 2 {
        let base = rows[0].tok_s;
        if base > 0.0 {
            for row in rows.iter().skip(1) {
                println!(
                    "  {} vs {}: ×{:.2}",
                    row.label, rows[0].label, row.tok_s / base
                );
            }
        }
    }
}

fn mkmodel_bench(root: &Path, out: &Path, pack_trunk: bool) {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root)
        .args(["run", "-q", "--release", "-p", "mkmodel-soso", "--"])
        .arg(out)
        .args([
            "--name",
            if pack_trunk {
                "bench-packed"
            } else {
                "bench"
            },
            "--layers",
            "4",
            "--hidden",
            "1024",
            "--ffn",
            "2816",
            "--vocab",
            "2048",
        ]);
    if pack_trunk {
        cmd.arg("--pack-trunk");
    }
    let status = cmd.status().expect("mkmodel-soso bench");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn mkfs_bench_models(root: &Path, dirs: &[PathBuf]) -> PathBuf {
    let path = root.join("target/soso-bench-models.img");
    let mut args = vec![
        "run".to_string(),
        "-q".to_string(),
        "--release".to_string(),
        "-p".to_string(),
        "mkfs-sosomfs".to_string(),
        "--".to_string(),
    ];
    for d in dirs {
        args.push(d.to_string_lossy().into_owned());
    }
    args.push(path.to_string_lossy().into_owned());
    args.push("--size".to_string());
    args.push("512M".to_string());
    let status = Command::new("cargo")
        .current_dir(root)
        .args(&args)
        .status()
        .expect("mkfs-sosomfs bench");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    path
}

fn echo_telemetry(out: &str, smp: u32, label: &str) {
    for etiqueta in [
        "soso-llm: generado",
        "soso-llm: carga en frío",
        "soso-llm: streaming — prefetch",
        "soso-llm: hot path",
        "soso-llm: disco",
        "soso-llm: tronco —",
        "soso-llm: MoE cache —",
    ] {
        if let Some(l) = out.lines().find(|l| l.contains(etiqueta)) {
            println!("bench-llm: SMP={smp} {label} {}", l.trim());
        }
    }
}

fn lanzar_qemu(
    img: &Path,
    data: &Path,
    models: &Path,
    serial: &Path,
) -> std::io::Result<Child> {
    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35", "-cpu", "max"])
        .args(["-m", &super::qemu_mem()])
        .args(["-smp", &super::qemu_smp()]);
    super::apply_firmware(&mut qemu, img);
    qemu.args(["-drive", &format!("format=raw,file={}", img.display())]);
    super::apply_qemu_disks(&mut qemu, data, models, &super::QemuGuestConfig::from_env());
    super::apply_qemu_usb(&mut qemu, &super::QemuGuestConfig::from_env());
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

fn ssh_cmd(key: &Path, script: &str, timeout_secs: u64) -> Result<String, String> {
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
