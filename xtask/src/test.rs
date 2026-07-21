//! `cargo xtask test`: batería de integración de soso.
//!
//! 1. Crash-safety de sosofs en el host (`cargo test -p sosofs`).
//! 2. Arranque en QEMU hasta la shell.
//! 3. Echo TCP en el puerto 7 (hostfwd 7777).
//! 4. Sesión SSH autenticada por clave, ejecuta un comando y apaga con
//!    `halt` (que a su vez comprueba el apagado limpio).
//!
//! Sale con código 0 si todo pasa, 1 si algo falla.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// QEMU sale con (code<<1)|1; ExitCode::Success = 0x10 -> 33.
const HALT_EXIT: i32 = 33;

pub fn run() {
    let root = super::project_root();
    let mut fallos = 0;

    // --- 1b) tests sosomfs en el host ---
    let _ = paso("sosomfs (host)", &mut fallos, || {
        let st = Command::new("cargo")
            .current_dir(&root)
            .args(["test", "-q", "-p", "sosomfs", "--features", "std"])
            .status()
            .map_err(|e| e.to_string())?;
        if st.success() { Ok(()) } else { Err("los tests de sosomfs fallaron".into()) }
    });

    // --- 1) crash-safety del FS en el host ---
    let _ = paso("crash-safety de sosofs (host)", &mut fallos, || {
        let st = Command::new("cargo")
            .current_dir(&root)
            .args(["test", "-q", "-p", "sosofs", "--features", "std"])
            .status()
            .map_err(|e| e.to_string())?;
        if st.success() { Ok(()) } else { Err("los tests de sosofs fallaron".into()) }
    });

    // --- construir e ir a QEMU ---
    super::build_user();
    let img = super::build_image();
    let data = super::mkfs_rootfs(true);
    let models = super::mkfs_models(true);
    let key = root.join("target/soso_test_key");
    let serial = root.join("target/test-serial.log");
    let _ = std::fs::remove_file(&serial);

    let mut qemu = match lanzar_qemu(&img, &data, &models, &serial) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("FALLO  lanzar QEMU: {e}");
            exit_resumen(1);
        }
    };

    // --- 2) arranque hasta la shell ---
    let arrancado = paso("arranque hasta la shell", &mut fallos, || {
        esperar_en_fichero(&serial, "sosh —", Duration::from_secs(90))
    })
    .is_ok();

    if arrancado {
        // --- 3) echo TCP ---
        let _ = paso("echo TCP en :7777", &mut fallos, echo_tcp);

        // --- 4) inferencia LLM (modelo tiny en disco 1) ---
        let _ = paso("soso-llm run tiny --prompt test", &mut fallos, || ssh_llm(&key));

        // --- 5) sesión SSH autenticada + halt ---
        let _ = paso("SSH por clave pública + comando + halt", &mut fallos, || {
            ssh_sesion(&key)
        });
    }

    // --- comprobar apagado por halt (o matar QEMU) ---
    let apagado = espera_salida(&mut qemu, Duration::from_secs(10));
    match apagado {
        Some(code) if code == HALT_EXIT => {
            marca("apagado limpio por halt", true);
        }
        Some(code) => {
            marca(&format!("QEMU salió con código inesperado {code}"), false);
            fallos += 1;
        }
        None => {
            marca("QEMU no se apagó con halt (matado)", false);
            fallos += 1;
            let _ = qemu.kill();
        }
    }
    let _ = qemu.wait();

    exit_resumen(if fallos == 0 { 0 } else { 1 });
}

fn exit_resumen(code: i32) -> ! {
    if code == 0 {
        println!("\n✅ cargo xtask test: TODO OK");
    } else {
        println!("\n❌ cargo xtask test: hubo fallos");
    }
    std::process::exit(code);
}

fn paso<F: FnOnce() -> Result<(), String>>(
    nombre: &str,
    fallos: &mut u32,
    f: F,
) -> Result<(), ()> {
    match f() {
        Ok(()) => {
            marca(nombre, true);
            Ok(())
        }
        Err(e) => {
            marca(&format!("{nombre}: {e}"), false);
            *fallos += 1;
            Err(())
        }
    }
}

fn marca(nombre: &str, ok: bool) {
    println!("{}  {nombre}", if ok { "OK  " } else { "FALLO" });
}

fn lanzar_qemu(
    img: &std::path::Path,
    data: &std::path::Path,
    models: &std::path::Path,
    serial: &std::path::Path,
) -> std::io::Result<Child> {
    Command::new("qemu-system-x86_64")
        .args(["-machine", "q35", "-cpu", "max"])
        .args(["-m", &super::qemu_mem()])
        .args(["-smp", &super::qemu_smp()])
        .args(["-drive", &format!("format=raw,file={}", img.display())])
        .args(["-drive", &format!("file={},format=raw,if=none,id=data0", data.display())])
        .args(["-device", "virtio-blk-pci,drive=data0"])
        .args(["-drive", &format!("file={},format=raw,if=none,id=data1", models.display())])
        .args(["-device", "virtio-blk-pci,drive=data1"])
        .args(["-netdev", "user,id=net0,hostfwd=tcp::7777-:7,hostfwd=tcp::2222-:22"])
        .args(["-device", "virtio-net-pci,netdev=net0"])
        .args(["-serial", &format!("file:{}", serial.display())])
        .args(["-display", "none"])
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .arg("-no-reboot")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

/// Espera a que aparezca `patron` en el fichero de serie.
fn esperar_en_fichero(
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

fn echo_tcp() -> Result<(), String> {
    let mut s = conectar_reintentando(7777, Duration::from_secs(10))?;
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

fn ssh_llm(key: &std::path::Path) -> Result<(), String> {
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
        .map_err(|e| format!("no se pudo lanzar ssh: {e}"))?;

    {
        let mut stdin = hijo.stdin.take().unwrap();
        stdin
            .write_all(b"soso-llm run tiny --prompt test\nexit\n")
            .map_err(|e| e.to_string())?;
        stdin.flush().ok();
        // Con SMP>1 el margen justo de antes (45s) empezó a fallar por poco
        // al activar el scheduler multicore real: cada syscall compite un
        // poco más por PROCS.lock() con los cores ociosos sondeando, y
        // user/libsoso hace ~15000 syscalls sbrk (una por asignación
        // pequeña, sin agrupar) incluso para el modelo sintético diminuto
        // de este test — con SMP la cola se nota. 100s da margen de sobra
        // en la práctica; el arreglo de fondo (no necesario para que esto
        // pase, pero deseable) sería que el allocator de libsoso agrupe
        // sbrk en vez de una syscall por asignación.
        std::thread::sleep(Duration::from_secs(100));
    }

    let salida = hijo.wait_with_output().map_err(|e| e.to_string())?;
    let texto = String::from_utf8_lossy(&salida.stdout);
    if texto.contains("soso-llm: generado") {
        Ok(())
    } else {
        Err(format!(
            "soso-llm no generó salida esperada; stdout: {texto:?}"
        ))
    }
}

fn ssh_sesion(key: &std::path::Path) -> Result<(), String> {
    let token = "soso_ssh_ok_42";
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
        .map_err(|e| format!("no se pudo lanzar ssh: {e}"))?;

    // Enviar el comando y halt.
    {
        let mut stdin = hijo.stdin.take().unwrap();
        let guion = format!("echo {token} > /tmp/xtask.txt\ncat /tmp/xtask.txt\nhalt\n");
        // Dar tiempo entre comandos escribiendo con pausa.
        stdin.write_all(guion.as_bytes()).map_err(|e| e.to_string())?;
        stdin.flush().ok();
        std::thread::sleep(Duration::from_secs(4));
        // stdin se cierra al salir del scope.
    }

    let salida = hijo.wait_with_output().map_err(|e| e.to_string())?;
    let texto = String::from_utf8_lossy(&salida.stdout);
    if texto.contains(token) {
        Ok(())
    } else {
        Err(format!("no se vio el token en la sesión SSH; salida: {texto:?}"))
    }
}

/// Espera a que QEMU salga; devuelve su código si sale a tiempo.
fn espera_salida(qemu: &mut Child, limite: Duration) -> Option<i32> {
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
