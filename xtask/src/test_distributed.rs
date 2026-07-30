//! `cargo xtask test-distributed-llm`: dos QEMU enlazados por socket netdev.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub fn run() {
    let _root = super::project_root();
    super::build_user();
    let img = super::build_image();
    let data = super::mkfs_rootfs(true);
    let models = super::mkfs_models(true);

    let sock_port = 9009u16;
    let worker_port = 4455u16;
    let head_port = 4456u16;

    let head_img = copiar_imagen(&img, "soso-bios-dist.img");
    let head_data = copiar_imagen(&data, "soso-data-dist.img");
    let head_models = copiar_imagen(&models, "soso-models-dist.img");

    let mut worker = match lanzar_qemu_dist(
        &img,
        &data,
        &models,
        0,
        sock_port,
        true,
        worker_port,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FALLO lanzar QEMU worker: {e}");
            std::process::exit(1);
        }
    };
    std::thread::sleep(Duration::from_secs(5));
    if qemu_exited(&mut worker, "worker") {
        std::process::exit(1);
    }
    let mut head = match lanzar_qemu_dist(
        &head_img,
        &head_data,
        &head_models,
        1,
        sock_port,
        false,
        head_port,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FALLO lanzar QEMU head: {e}");
            let _ = worker.kill();
            std::process::exit(1);
        }
    };

    if qemu_exited(&mut head, "head") {
        let _ = worker.kill();
        std::process::exit(1);
    }

    let mut fallos = 0u32;
    let mut worker_con = None;
    let mut head_con = None;
    match conectar_serial(worker_port, Duration::from_secs(60)) {
        Ok(s) => worker_con = Some(s),
        Err(e) => {
            eprintln!("FALLO conectar consola worker: {e}");
            fallos += 1;
        }
    }
    match conectar_serial(head_port, Duration::from_secs(60)) {
        Ok(s) => head_con = Some(s),
        Err(e) => {
            eprintln!("FALLO conectar consola head: {e}");
            fallos += 1;
        }
    }

    if fallos == 0 {
        let w = worker_con.as_mut().unwrap();
        let h = head_con.as_mut().unwrap();
        if esperar_prompt(w, Duration::from_secs(120)).is_err() {
            eprintln!("FALLO worker no llegó a sosh");
            fallos += 1;
        }
        if esperar_prompt(h, Duration::from_secs(120)).is_err() {
            eprintln!("FALLO head no llegó a sosh");
            fallos += 1;
        }
        // DHCP tarda ~8 s en caer a IP estática por MAC.
        if fallos == 0 {
            if esperar_salida(w, "net: sin dhcp", Duration::from_secs(60)).is_err() {
                eprintln!("FALLO worker sin IP estática");
                fallos += 1;
            }
            if esperar_salida(h, "net: sin dhcp", Duration::from_secs(60)).is_err() {
                eprintln!("FALLO head sin IP estática");
                fallos += 1;
            }
        }
    }

    if fallos == 0 {
        let w = worker_con.as_mut().unwrap();
        let h = head_con.as_mut().unwrap();
        enviar(w, "soso-llm worker tiny --listen 9900 --layers 2:4\n");
        if esperar_salida(w, "nodo escuchando", Duration::from_secs(120)).is_err() {
            eprintln!("FALLO worker no arrancó soso-llm");
            fallos += 1;
        } else if fallos == 0 {
            enviar(
                h,
                "soso-llm run tiny --remote 10.0.2.15:9900 --split 2 --prompt test --max 2 --seed 42\n",
            );
            match esperar_salida(h, "soso-llm: generado distribuido", Duration::from_secs(300)) {
                Ok(_) => println!("OK  inferencia distribuida 2 nodos"),
                Err(head_buf) => {
                    let mut worker_buf = String::new();
                    volcar_serial(w, &mut worker_buf);
                    let mut head_buf = head_buf;
                    volcar_serial(h, &mut head_buf);
                    eprintln!(
                        "FALLO inferencia distribuida\n--- head ---\n{head_buf}\n--- worker ---\n{worker_buf}"
                    );
                    fallos += 1;
                }
            }
        }
        enviar(h, "halt\n");
        enviar(worker_con.as_mut().unwrap(), "halt\n");
    }

    let _ = super::test::espera_salida(&mut head, Duration::from_secs(15));
    let _ = super::test::espera_salida(&mut worker, Duration::from_secs(15));
    let _ = head.kill();
    let _ = worker.kill();

    if fallos == 0 {
        println!("\n✅ cargo xtask test-distributed-llm: TODO OK");
        std::process::exit(0);
    } else {
        println!("\n❌ cargo xtask test-distributed-llm: hubo fallos");
        std::process::exit(1);
    }
}

fn copiar_imagen(src: &std::path::Path, name: &str) -> std::path::PathBuf {
    let dst = super::project_root().join("target").join(name);
    std::fs::copy(src, &dst).unwrap_or_else(|e| {
        panic!("copiar {} → {}: {e}", src.display(), dst.display());
    });
    dst
}

fn lanzar_qemu_dist(
    img: &std::path::Path,
    data: &std::path::Path,
    models: &std::path::Path,
    instance: u8,
    sock_port: u16,
    listen: bool,
    serial_port: u16,
) -> std::io::Result<Child> {
    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35", "-cpu", "max"])
        .args(["-m", &super::qemu_mem()])
        .args(["-smp", &super::qemu_smp()]);
    super::apply_firmware(&mut qemu, img);
    qemu.args(["-drive", &format!("format=raw,file={}", img.display())]);
    super::apply_qemu_disks(&mut qemu, data, models);
    apply_qemu_nic_dist(&mut qemu, instance, sock_port, listen);
    super::apply_qemu_gpu(&mut qemu);
    qemu.args([
        "-chardev",
        &format!(
            "socket,id=ser0,host=127.0.0.1,port={serial_port},server=on,wait=off,telnet=on"
        ),
    ])
    .args(["-serial", "chardev:ser0"])
    .args(["-display", "none"])
    .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
    .arg("-no-reboot")
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .spawn()
}

fn qemu_exited(child: &mut Child, label: &str) -> bool {
    match child.try_wait() {
        Ok(Some(st)) => {
            eprintln!("FALLO {label} QEMU salió prematuramente: {st:?}");
            if let Some(mut err) = child.stderr.take() {
                use std::io::Read;
                let mut buf = String::new();
                err.read_to_string(&mut buf).ok();
                if !buf.is_empty() {
                    eprintln!("{label} stderr: {buf}");
                }
            }
            true
        }
        Ok(None) => false,
        Err(e) => {
            eprintln!("FALLO esperando {label}: {e}");
            true
        }
    }
}

fn apply_qemu_nic_dist(qemu: &mut Command, instance: u8, _sock_port: u16, _listen: bool) {
    let mac_suffix = 15u8.saturating_add(instance);
    let mac = format!("52:54:00:12:34:{mac_suffix:02x}");
    qemu.args(["-netdev", "socket,id=net0,mcast=224.0.0.1:9009"]);
    qemu.args(["-device", &format!("virtio-net-pci,netdev=net0,mac={mac}")]);
}

pub fn run_3() {
    super::build_user();
    let img = super::build_image();
    let data = super::mkfs_rootfs(true);
    let models = super::mkfs_models(true);

    let tail_port = 4455u16;
    let mid_port = 4456u16;
    let head_port = 4457u16;

    let imgs: Vec<_> = (0..3)
        .map(|i| {
            (
                copiar_imagen(&img, &format!("soso-bios-dist3-{i}.img")),
                copiar_imagen(&data, &format!("soso-data-dist3-{i}.img")),
                copiar_imagen(&models, &format!("soso-models-dist3-{i}.img")),
            )
        })
        .collect();

    let mut tail = lanzar_qemu_dist(&imgs[0].0, &imgs[0].1, &imgs[0].2, 0, 9009, false, tail_port)
        .unwrap_or_else(|e| panic!("FALLO lanzar tail: {e}"));
    std::thread::sleep(Duration::from_secs(4));
    let mut mid = lanzar_qemu_dist(&imgs[1].0, &imgs[1].1, &imgs[1].2, 1, 9009, false, mid_port)
        .unwrap_or_else(|e| panic!("FALLO lanzar mid: {e}"));
    std::thread::sleep(Duration::from_secs(4));
    let mut head = lanzar_qemu_dist(&imgs[2].0, &imgs[2].1, &imgs[2].2, 2, 9009, false, head_port)
        .unwrap_or_else(|e| panic!("FALLO lanzar head: {e}"));

    let mut fallos = 0u32;
    let mut tail_con = conectar_serial(tail_port, Duration::from_secs(60)).ok();
    let mut mid_con = conectar_serial(mid_port, Duration::from_secs(60)).ok();
    let mut head_con = conectar_serial(head_port, Duration::from_secs(60)).ok();
    if tail_con.is_none() {
        eprintln!("FALLO conectar consola tail");
        fallos += 1;
    }
    if mid_con.is_none() {
        eprintln!("FALLO conectar consola mid");
        fallos += 1;
    }
    if head_con.is_none() {
        eprintln!("FALLO conectar consola head");
        fallos += 1;
    }

    if fallos == 0 {
        let tail_c = tail_con.as_mut().unwrap();
        let mid_c = mid_con.as_mut().unwrap();
        let head_c = head_con.as_mut().unwrap();
        if esperar_prompt(tail_c, Duration::from_secs(120)).is_err() {
            eprintln!("FALLO tail no llegó a sosh");
            fallos += 1;
        }
        if esperar_prompt(mid_c, Duration::from_secs(120)).is_err() {
            eprintln!("FALLO mid no llegó a sosh");
            fallos += 1;
        }
        if esperar_prompt(head_c, Duration::from_secs(120)).is_err() {
            eprintln!("FALLO head no llegó a sosh");
            fallos += 1;
        }
        if fallos == 0 {
            if esperar_salida(tail_c, "net: sin dhcp", Duration::from_secs(60)).is_err() {
                eprintln!("FALLO tail sin IP estática");
                fallos += 1;
            }
            if esperar_salida(mid_c, "net: sin dhcp", Duration::from_secs(60)).is_err() {
                eprintln!("FALLO mid sin IP estática");
                fallos += 1;
            }
            if esperar_salida(head_c, "net: sin dhcp", Duration::from_secs(60)).is_err() {
                eprintln!("FALLO head sin IP estática");
                fallos += 1;
            }
        }
    }

    if fallos == 0 {
        let tail_c = tail_con.as_mut().unwrap();
        let mid_c = mid_con.as_mut().unwrap();
        let head_c = head_con.as_mut().unwrap();
        enviar(tail_c, "soso-llm node tiny --listen 9902 --layers 3:4\n");
        if esperar_salida(tail_c, "nodo escuchando", Duration::from_secs(120)).is_err() {
            eprintln!("FALLO tail no arrancó");
            fallos += 1;
        }
        if fallos == 0 {
            enviar(mid_c, "soso-llm node tiny --listen 9901 --layers 2:3\n");
            if esperar_salida(mid_c, "nodo escuchando", Duration::from_secs(120)).is_err() {
                eprintln!("FALLO mid no arrancó");
                fallos += 1;
            }
        }
        if fallos == 0 {
            enviar(
                head_c,
                "soso-llm run tiny --pipeline 10.0.2.16:9901,10.0.2.15:9902 --splits 2,3 --prompt test --max 2 --seed 42\n",
            );
            match esperar_salida(head_c, "soso-llm: generado distribuido", Duration::from_secs(300))
            {
                Ok(_) => println!("OK  inferencia distribuida 3 nodos (estrella)"),
                Err(buf) => {
                    eprintln!("FALLO inferencia 3 nodos\n--- head ---\n{buf}");
                    fallos += 1;
                }
            }
        }
        enviar(head_c, "halt\n");
        enviar(mid_c, "halt\n");
        enviar(tail_c, "halt\n");
    }

    let _ = super::test::espera_salida(&mut head, Duration::from_secs(15));
    let _ = super::test::espera_salida(&mut mid, Duration::from_secs(15));
    let _ = super::test::espera_salida(&mut tail, Duration::from_secs(15));
    let _ = head.kill();
    let _ = mid.kill();
    let _ = tail.kill();

    if fallos == 0 {
        println!("\n✅ cargo xtask test-distributed-llm-3: TODO OK");
        std::process::exit(0);
    } else {
        println!("\n❌ cargo xtask test-distributed-llm-3: hubo fallos");
        std::process::exit(1);
    }
}

fn conectar_serial(port: u16, limite: Duration) -> Result<TcpStream, String> {
    let fin = Instant::now() + limite;
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => return Ok(s),
            Err(_e) if Instant::now() < fin => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(format!("no conecta a serial :{port}: {e}")),
        }
    }
}

fn enviar(con: &mut TcpStream, cmd: &str) {
    con.write_all(cmd.as_bytes()).ok();
    con.flush().ok();
}

fn esperar_prompt(con: &mut TcpStream, limite: Duration) -> Result<(), String> {
    esperar_salida(con, "sosh —", limite).map(|_| ())
}

fn esperar_salida(con: &mut TcpStream, patron: &str, limite: Duration) -> Result<String, String> {
    con.set_read_timeout(Some(Duration::from_millis(500))).ok();
    let fin = Instant::now() + limite;
    let mut buf = String::new();
    let mut tmp = [0u8; 256];
    while Instant::now() < fin {
        match con.read(&mut tmp) {
            Ok(0) => std::thread::sleep(Duration::from_millis(100)),
            Ok(n) => {
                buf.push_str(&String::from_utf8_lossy(&tmp[..n]));
                if buf.contains(patron) {
                    return Ok(buf);
                }
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    Err(format!("no apareció {patron:?}; tenía:\n{buf}"))
}

fn volcar_serial(con: &mut TcpStream, buf: &mut String) {
    con.set_read_timeout(Some(Duration::from_millis(200))).ok();
    let mut tmp = [0u8; 4096];
    let fin = Instant::now() + Duration::from_secs(2);
    while Instant::now() < fin {
        match con.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.push_str(&String::from_utf8_lossy(&tmp[..n])),
            Err(_) => break,
        }
    }
}
