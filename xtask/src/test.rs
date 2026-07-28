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

        // --- 5) regresión de syscalls dentro del guest (incl. GPU, hilos, FPU) ---
        let _ = paso("init test (syscalls, hilos, FPU, GPU)", &mut fallos, || {
            ssh_init_test(&key)
        });

        // --- 6) sesión SSH autenticada + halt ---
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
        // Dos inferencias en la misma sesión: la de CPU y la que pasa por el
        // camino de syscalls GPU con el dispositivo software del kernel. La
        // segunda es la única cobertura que tiene ese camino sin tarjeta —
        // alloc/map/submit/read, el cacheo de pesos y el crecimiento de búferes
        // entre capas de distinto tamaño.
        stdin
            .write_all(
                b"soso-llm run tiny --prompt test\n                  soso-llm run tiny --prompt test --gpu-soft --max 4\n                  exit\n",
            )
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
        // La segunda inferencia (--max 4, dispositivo software) añade su propio
        // tiempo: 150s en vez de 100 para las dos.
        std::thread::sleep(Duration::from_secs(150));
    }

    let salida = hijo.wait_with_output().map_err(|e| e.to_string())?;
    let texto = String::from_utf8_lossy(&salida.stdout);
    if !texto.contains("soso-llm: generado") {
        return Err(format!(
            "soso-llm no generó salida esperada; stdout: {texto:?}"
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

/// `init test`: la batería de regresión de syscalls que corre DENTRO del guest.
///
/// Existía desde la fase 6 y la suite no la ejecutaba: hilos+futex (L3b), estrés
/// FPU/YMM (L4) y ahora el camino de syscalls GPU eran subpruebas que sólo se
/// veían si alguien las lanzaba a mano. Un test que hay que acordarse de correr no
/// es una red de seguridad.
fn ssh_init_test(key: &std::path::Path) -> Result<(), String> {
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
            .write_all(b"init test\nexit\n")
            .map_err(|e| e.to_string())?;
        stdin.flush().ok();
        std::thread::sleep(Duration::from_secs(45));
    }

    let salida = hijo.wait_with_output().map_err(|e| e.to_string())?;
    let texto = String::from_utf8_lossy(&salida.stdout);
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
    let mut qemu = lanzar_qemu(&img, &data, &models, &serial).expect("QEMU lx-e1000e");
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
