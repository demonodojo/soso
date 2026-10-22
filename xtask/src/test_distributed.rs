//! `cargo xtask test-distributed-llm`: dos QEMU enlazados por socket netdev.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use soso_llm_api::{run_core_invariants_guest, ApiClient, GuestHttpLimits};

use crate::test::ssh_guion;

pub fn run() {
    let _root = super::project_root();
    let _ = Command::new("pkill").arg("-f").arg("qemu-system-x86_64").status();
    std::thread::sleep(Duration::from_millis(500));
    super::build_user();
    let img = super::build_image();
    let data = super::mkfs_rootfs(true);
    let models = super::mkfs_models(true);

    let sock_port = 9009u16;
    let worker_port = 4455u16;
    let head_port = 4456u16;

    let head_img = copiar_imagen(&img, "soso-dist-head-uefi.img");
    let head_data = copiar_imagen(&data, "soso-data-dist.img");
    let head_models = copiar_imagen(&models, "soso-models-dist.img");

    let boot = Duration::from_secs(180);
    let mut worker = match lanzar_qemu_dist(
        &img,
        &data,
        &models,
        0,
        sock_port,
        true,
        worker_port,
        None,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FALLO lanzar QEMU worker: {e}");
            std::process::exit(1);
        }
    };
    let worker_serial = spawn_esperar_sosh(worker_port, boot);
    std::thread::sleep(Duration::from_secs(6));
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
        None,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FALLO lanzar QEMU head: {e}");
            let _ = worker.kill();
            std::process::exit(1);
        }
    };
    let head_serial = spawn_esperar_sosh(head_port, boot);

    if qemu_exited(&mut head, "head") {
        let _ = worker.kill();
        std::process::exit(1);
    }

    let mut fallos = 0u32;
    let mut worker_con = match worker_serial.join().unwrap_or(Err("worker hilo".into())) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("FALLO worker no llegó a sosh: {e}");
            fallos += 1;
            None
        }
    };
    let mut head_con = match head_serial.join().unwrap_or(Err("head hilo".into())) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("FALLO head no llegó a sosh: {e}");
            if let Some(mut err) = head.stderr.take() {
                use std::io::Read;
                let mut q = String::new();
                err.read_to_string(&mut q).ok();
                if !q.is_empty() {
                    eprintln!("head QEMU stderr:\n{q}");
                }
            }
            fallos += 1;
            None
        }
    };

    if fallos == 0 {
        let w = worker_con.as_mut().unwrap();
        let h = head_con.as_mut().unwrap();
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

/// Copia sparse de la ESP/kernel. Si es imagen UEFI, `name` debe acabar en `uefi.img`
/// (`apply_firmware` no mira el path completo) o el guest arranca sin OVMF y la serie queda muda.
fn copiar_imagen(src: &std::path::Path, name: &str) -> std::path::PathBuf {
    let dst = super::project_root().join("target").join(name);
    super::copy_sparse(src, &dst);
    dst
}

struct SlirpForwardsDist {
    ssh_port: Option<u16>,
    llm_port: Option<u16>,
    /// Puerto en el host (127.0.0.1) reenviado al `:9900` del nodo worker.
    node_host_port: Option<u16>,
    /// Echo TCP :7 en el host (sólo una VM slirp puede usar :7777).
    echo_port: Option<u16>,
}

fn build_slirp_netdev(id: &str, f: &SlirpForwardsDist) -> String {
    let mut fwd = Vec::new();
    if let Some(p) = f.echo_port {
        fwd.push(format!("hostfwd=tcp::{p}-:7"));
    }
    if let Some(p) = f.ssh_port {
        fwd.push(format!("hostfwd=tcp::{p}-:22"));
    }
    if let Some(p) = f.llm_port {
        fwd.push(format!("hostfwd=tcp:127.0.0.1:{p}-:7422"));
    }
    if let Some(p) = f.node_host_port {
        // Sin 127.0.0.1: el head slirp llega por 10.0.2.2 (gateway), no por loopback del host.
        fwd.push(format!("hostfwd=tcp::{p}-:9900"));
    }
    format!("user,id={id},restrict=off,{}", fwd.join(","))
}

/// Guest con **una** NIC slirp (el kernel sólo monta la primera virtio-net).
fn lanzar_qemu_slirp(
    img: &std::path::Path,
    data: &std::path::Path,
    models: &std::path::Path,
    instance: u8,
    serial_port: u16,
    slirp: SlirpForwardsDist,
) -> std::io::Result<Child> {
    let mac_suffix = 15u8.saturating_add(instance);
    let mac = format!("52:54:00:12:34:{mac_suffix:02x}");
    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35", "-cpu", "max"])
        .args(["-m", &super::qemu_mem()])
        .args(["-smp", &super::qemu_smp()]);
    let vars_name = format!("OVMF_VARS_dist_{instance}.fd");
    super::apply_firmware_named(&mut qemu, img, &vars_name);
    qemu.args(["-drive", &format!("format=raw,file={}", img.display())]);
    super::apply_qemu_disks(&mut qemu, data, models, &super::QemuGuestConfig::from_env());
    super::apply_qemu_usb(&mut qemu, &super::QemuGuestConfig::from_env());
    let netdev = build_slirp_netdev("net0", &slirp);
    qemu.args(["-netdev", &netdev]);
    qemu.args([
        "-device",
        &format!("virtio-net-pci,netdev=net0,mac={mac}"),
    ]);
    super::apply_qemu_gpu(&mut qemu);
    qemu.args([
        "-chardev",
        &format!(
            "socket,id=ser0,host=127.0.0.1,port={serial_port},server=on,wait=off,telnet=off"
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

fn lanzar_qemu_dist(
    img: &std::path::Path,
    data: &std::path::Path,
    models: &std::path::Path,
    instance: u8,
    sock_port: u16,
    listen: bool,
    serial_port: u16,
    slirp: Option<SlirpForwardsDist>,
) -> std::io::Result<Child> {
    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35", "-cpu", "max"])
        .args(["-m", &super::qemu_mem()])
        .args(["-smp", &super::qemu_smp()]);
    let vars_name = format!("OVMF_VARS_dist_{instance}.fd");
    super::apply_firmware_named(&mut qemu, img, &vars_name);
    qemu.args(["-drive", &format!("format=raw,file={}", img.display())]);
    super::apply_qemu_disks(&mut qemu, data, models, &super::QemuGuestConfig::from_env());
    super::apply_qemu_usb(&mut qemu, &super::QemuGuestConfig::from_env());
    apply_qemu_nic_dist(&mut qemu, instance, sock_port, listen);
    if let Some(f) = slirp {
        let netdev = build_slirp_netdev("net1", &f);
        qemu.args(["-netdev", &netdev]);
        qemu.args([
            "-device",
            "virtio-net-pci,netdev=net1,mac=52:54:00:12:34:99",
        ]);
    }
    super::apply_qemu_gpu(&mut qemu);
    qemu.args([
        "-chardev",
        &format!(
            "socket,id=ser0,host=127.0.0.1,port={serial_port},server=on,wait=off,telnet=off"
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
    let _ = Command::new("pkill").arg("-f").arg("qemu-system-x86_64").status();
    std::thread::sleep(Duration::from_millis(500));
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
                copiar_imagen(&img, &format!("soso-dist3-{i}-uefi.img")),
                copiar_imagen(&data, &format!("soso-data-dist3-{i}.img")),
                copiar_imagen(&models, &format!("soso-models-dist3-{i}.img")),
            )
        })
        .collect();

    let boot = Duration::from_secs(180);
    let mut tail = lanzar_qemu_dist(
        &imgs[0].0,
        &imgs[0].1,
        &imgs[0].2,
        0,
        9009,
        false,
        tail_port,
        None,
    )
        .unwrap_or_else(|e| panic!("FALLO lanzar tail: {e}"));
    let tail_serial = spawn_esperar_sosh(tail_port, boot);
    std::thread::sleep(Duration::from_secs(4));
    let mut mid = lanzar_qemu_dist(
        &imgs[1].0,
        &imgs[1].1,
        &imgs[1].2,
        1,
        9009,
        false,
        mid_port,
        None,
    )
        .unwrap_or_else(|e| panic!("FALLO lanzar mid: {e}"));
    let mid_serial = spawn_esperar_sosh(mid_port, boot);
    std::thread::sleep(Duration::from_secs(4));
    let mut head = lanzar_qemu_dist(
        &imgs[2].0,
        &imgs[2].1,
        &imgs[2].2,
        2,
        9009,
        false,
        head_port,
        None,
    )
        .unwrap_or_else(|e| panic!("FALLO lanzar head: {e}"));
    let head_serial = spawn_esperar_sosh(head_port, boot);

    let mut fallos = 0u32;
    let mut tail_con = None;
    let mut mid_con = None;
    let mut head_con = None;
    for (label, handle, slot) in [
        ("tail", tail_serial, &mut tail_con),
        ("mid", mid_serial, &mut mid_con),
        ("head", head_serial, &mut head_con),
    ] {
        match handle.join().unwrap_or(Err(format!("{label} hilo"))) {
            Ok(s) => *slot = Some(s),
            Err(e) => {
                eprintln!("FALLO {label} no llegó a sosh: {e}");
                fallos += 1;
            }
        }
    }

    if fallos == 0 {
        let tail_c = tail_con.as_mut().unwrap();
        let mid_c = mid_con.as_mut().unwrap();
        let head_c = head_con.as_mut().unwrap();
        {
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

/// Nodo `--auto` y head `--splits auto`.
pub fn run_auto() {
    let _root = super::project_root();
    let _ = Command::new("pkill").arg("-f").arg("qemu-system-x86_64").status();
    std::thread::sleep(Duration::from_millis(500));
    super::build_user();
    let img = super::build_image();
    let data = super::mkfs_rootfs(true);
    let models = super::mkfs_models(true);

    let sock_port = 9009u16;
    let worker_port = 4455u16;
    let head_port = 4456u16;

    let head_img = copiar_imagen(&img, "soso-dist-auto-uefi.img");
    let head_data = copiar_imagen(&data, "soso-data-dist-auto.img");
    let head_models = copiar_imagen(&models, "soso-models-dist-auto.img");

    let boot = Duration::from_secs(180);
    let mut worker = lanzar_qemu_dist(&img, &data, &models, 0, sock_port, true, worker_port, None)
        .expect("worker");
    let worker_serial = spawn_esperar_sosh(worker_port, boot);
    std::thread::sleep(Duration::from_secs(6));
    let mut head = lanzar_qemu_dist(
        &head_img,
        &head_data,
        &head_models,
        1,
        sock_port,
        false,
        head_port,
        None,
    )
    .expect("head");
    let head_serial = spawn_esperar_sosh(head_port, boot);

    let mut fallos = 0u32;
    let mut w = match worker_serial.join().unwrap() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("FALLO worker no llegó a sosh: {e}");
            let _ = head.kill();
            let _ = worker.kill();
            exit_dist(1, "test-distributed-llm-auto");
            unreachable!()
        }
    };
    let mut h = match head_serial.join().unwrap() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("FALLO head no llegó a sosh: {e}");
            let _ = head.kill();
            let _ = worker.kill();
            exit_dist(1, "test-distributed-llm-auto");
            unreachable!()
        }
    };
    if fallos == 0 {
        let _ = esperar_salida(&mut w, "net: sin dhcp", Duration::from_secs(60));
        let _ = esperar_salida(&mut h, "net: sin dhcp", Duration::from_secs(60));
        enviar(&mut w, "soso-llm node tiny --listen 9900 --auto\n");
        if esperar_salida(&mut w, "nodo --auto en", Duration::from_secs(120)).is_err() {
            eprintln!("FALLO worker --auto no arrancó");
            fallos += 1;
        } else {
            enviar(
                &mut h,
                "soso-llm run tiny --remote 10.0.2.15:9900 --splits auto --prompt test --max 2 --seed 42\n",
            );
            if esperar_salida(&mut h, "soso-llm: generado distribuido", Duration::from_secs(360))
                .is_err()
            {
                let mut worker_buf = String::new();
                let mut head_buf = String::new();
                volcar_serial(&mut w, &mut worker_buf);
                volcar_serial(&mut h, &mut head_buf);
                eprintln!(
                    "FALLO inferencia auto\n--- head ---\n{head_buf}\n--- worker ---\n{worker_buf}"
                );
                fallos += 1;
            } else {
                println!("OK  reparto auto + nodo --auto");
            }
        }
    }
    enviar(&mut h, "halt\n");
    enviar(&mut w, "halt\n");
    let _ = head.kill();
    let _ = worker.kill();
    exit_dist(fallos, "test-distributed-llm-auto");
}

/// `serve --cluster` + petición HTTP desde el host (reenvío slirp en el head).
pub fn run_serve() {
    let _root = super::project_root();
    let _ = Command::new("pkill").arg("-f").arg("qemu-system-x86_64").status();
    std::thread::sleep(Duration::from_millis(500));
    super::build_user();
    let key = ensure_test_key();
    let img = super::build_image();
    let data = super::mkfs_rootfs(true);
    let models = super::mkfs_models(true);

    const SSH: u16 = 4460;
    const LLM: u16 = 7460;
    const NODE_HOST: u16 = 9901;
    const TOKEN: &str = "dist-serve-token";

    let worker_port = 4455u16;
    let head_port = 4456u16;

    let head_img = copiar_imagen(&img, "soso-dist-serve-uefi.img");
    let head_data = copiar_imagen(&data, "soso-data-dist-serve.img");
    let head_models = copiar_imagen(&models, "soso-models-dist-serve.img");

    let boot = Duration::from_secs(180);
    // Una sola NIC por guest: slirp. El worker expone :9900 en el host (9901);
    // el head llega vía 10.0.2.2 (gateway slirp → host).
    let mut worker = lanzar_qemu_slirp(
        &img,
        &data,
        &models,
        0,
        worker_port,
        SlirpForwardsDist {
            ssh_port: None,
            llm_port: None,
            node_host_port: Some(NODE_HOST),
            echo_port: Some(7777),
        },
    )
    .expect("worker");
    let worker_serial = spawn_esperar_sosh(worker_port, boot);
    std::thread::sleep(Duration::from_secs(6));
    let mut head = lanzar_qemu_slirp(
        &head_img,
        &head_data,
        &head_models,
        1,
        head_port,
        SlirpForwardsDist {
            ssh_port: Some(SSH),
            llm_port: Some(LLM),
            node_host_port: None,
            echo_port: None,
        },
    )
    .expect("head");
    if qemu_exited(&mut head, "head") {
        let _ = worker.kill();
        exit_dist(1, "test-distributed-llm-serve");
    }
    let head_serial = spawn_esperar_sosh(head_port, boot);

    let mut fallos = 0u32;
    let mut w_opt = match worker_serial.join().unwrap_or(Err("worker hilo".into())) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("FALLO worker no llegó a sosh: {e}");
            fallos += 1;
            None
        }
    };
    let mut h_opt = match head_serial.join().unwrap_or(Err("head hilo".into())) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("FALLO head no llegó a sosh: {e}");
            fallos += 1;
            None
        }
    };

    if fallos == 0 {
        let w = w_opt.as_mut().unwrap();
        let _ = esperar_salida(w, "net: dhcp", Duration::from_secs(120));
        enviar(w, "soso-llm node tiny --listen 9900 --layers 2:4\n");
        if esperar_salida(w, "nodo escuchando", Duration::from_secs(120)).is_err() {
            eprintln!("FALLO nodo worker");
            fallos += 1;
        } else if let Err(e) = esperar_puerto_tcp(NODE_HOST, Duration::from_secs(60)) {
            eprintln!(
                "FALLO host no reenvía el nodo worker (esperaba 127.0.0.1:{NODE_HOST}): {e}"
            );
            fallos += 1;
        }
    }
    if fallos == 0 {
        if let Err(e) = esperar_puerto_tcp(SSH, Duration::from_secs(120)) {
            eprintln!("FALLO SSH slirp no abrió: {e}");
            fallos += 1;
        }
    }
    if fallos == 0 {
        let prep = format!("echo {TOKEN} > /tmp/soso-llm-api.token\nexit\n");
        if let Err(e) = ssh_guion(&key, SSH, &prep, Duration::from_secs(120)) {
            eprintln!("FALLO preparar token en head: {e}");
            fallos += 1;
        }
    }
    if fallos == 0 {
        let serve = ServeSessionDist::start(
            &key,
            SSH,
            &format!(
                "soso-llm serve --model tiny --port 7422 --token-file /tmp/soso-llm-api.token \
                 --cluster 10.0.2.2:{NODE_HOST} --splits 2 >> /tmp/soso-llm-serve.log\n"
            ),
        );
        let client = ApiClient::new(LLM, TOKEN, "tiny");
        let mut lim = GuestHttpLimits::default();
        lim.read = Duration::from_secs(600);
        let t0 = Instant::now();
        let mut ready = false;
        while t0.elapsed() < Duration::from_secs(600) {
            if let Some(m) = serve.murio() {
                eprintln!("FALLO serve terminó: {m}");
                fallos += 1;
                break;
            }
            if dist_health_ready(LLM, TOKEN) {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        if fallos == 0 && !ready {
            if let Some(m) = serve.murio() {
                eprintln!("FALLO /health (serve ssh): {m}");
            } else {
                eprintln!("FALLO /health no listo en 600s");
            }
            fallos += 1;
        }
        if fallos == 0 {
            lim.chat_max_tokens = 8;
            for step in run_core_invariants_guest(&client, lim) {
                match step.name {
                    "health" | "models" | "chat_json" => {
                        if !step.ok {
                            eprintln!("FALLO serve HTTP {}: {}", step.name, step.detail);
                            fallos += 1;
                        }
                    }
                    _ => {}
                }
            }
            if fallos == 0 {
                println!("OK  serve --cluster + chat HTTP");
            }
        }
    }
    if let Some(h) = h_opt.as_mut() {
        enviar(h, "halt\n");
    }
    if let Some(w) = w_opt.as_mut() {
        enviar(w, "halt\n");
    }
    let _ = head.kill();
    let _ = worker.kill();
    exit_dist(fallos, "test-distributed-llm-serve");
}

fn exit_dist(fallos: u32, name: &str) {
    if fallos == 0 {
        println!("\n✅ cargo xtask {name}: TODO OK");
        std::process::exit(0);
    } else {
        println!("\n❌ cargo xtask {name}: hubo fallos");
        std::process::exit(1);
    }
}

fn ensure_test_key() -> PathBuf {
    let key = super::project_root().join("target/soso_test_key");
    let pubk = super::project_root().join("target/soso_test_key.pub");
    if !pubk.exists() {
        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-C", "soso-test", "-f"])
            .arg(&key)
            .status()
            .expect("ssh-keygen");
        if !status.success() {
            eprintln!("FALLO generar clave de test");
            std::process::exit(1);
        }
    }
    key
}

struct ServeSessionDist {
    fin: Arc<Mutex<Option<Result<String, String>>>>,
}

impl ServeSessionDist {
    fn start(key: &Path, ssh_port: u16, guion: &str) -> Self {
        let fin = Arc::new(Mutex::new(None));
        let fin_h = fin.clone();
        let key = key.to_path_buf();
        let guion = guion.to_string();
        std::thread::spawn(move || {
            *fin_h.lock().unwrap() = Some(ssh_guion(&key, ssh_port, &guion, Duration::from_secs(3600)));
        });
        Self { fin }
    }

    fn murio(&self) -> Option<String> {
        match &*self.fin.lock().unwrap() {
            None => None,
            Some(Ok(s)) => Some(format!("serve terminó: {s:?}")),
            Some(Err(e)) => Some(format!("serve caído: {e}")),
        }
    }
}

/// Conecta al chardev en cuanto la VM arranca; si el lector llega tarde el buffer serial
/// se llena y el guest puede bloquearse en boot.
fn spawn_esperar_sosh(
    port: u16,
    limite: Duration,
) -> std::thread::JoinHandle<Result<TcpStream, String>> {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        abrir_sosh_una(port, limite)
    })
}

fn abrir_sosh_una(port: u16, limite: Duration) -> Result<TcpStream, String> {
    let mut con = conectar_serial(port, Duration::from_secs(90))?;
    esperar_prompt(&mut con, limite)?;
    Ok(con)
}

fn dist_health_ready(port: u16, token: &str) -> bool {
    let url = format!("http://127.0.0.1:{port}/health");
    let auth = format!("Authorization: Bearer {token}");
    Command::new("curl")
        .args(["-sf", "-m", "3", "-H", &auth, &url])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .is_some_and(|o| o.status.success() && !o.stdout.is_empty())
}

fn esperar_puerto_tcp(port: u16, limite: Duration) -> Result<(), String> {
    let fin = Instant::now() + limite;
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok(());
        }
        if Instant::now() >= fin {
            return Err(format!("127.0.0.1:{port} no acepta conexiones"));
        }
        std::thread::sleep(Duration::from_millis(200));
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
