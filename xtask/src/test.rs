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

    // --- 1c) planificador de recursos (host) ---
    let _ = paso("planificador soso-llm-core (host)", &mut fallos, || {
        let st = Command::new("cargo")
            .current_dir(&root)
            .args(["test", "-q", "-p", "soso-llm-core", "--features", "std"])
            .status()
            .map_err(|e| e.to_string())?;
        if st.success() {
            Ok(())
        } else {
            Err("los tests del planificador fallaron".into())
        }
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
        let _ = paso_con_reintento("soso-llm run tiny --prompt test", &mut fallos, || {
            ssh_llm(&key)
        });

        // --- 4b) inferencia MoE (tiny-moe, streaming por experto) ---
        let _ = paso_con_reintento("soso-llm run tiny-moe --prompt @bos --max 2", &mut fallos, || {
            ssh_llm_moe(&key)
        });

        // --- 5) regresión de syscalls dentro del guest (incl. GPU, hilos, FPU) ---
        let _ = paso_con_reintento("init test (syscalls, hilos, FPU, GPU)", &mut fallos, || {
            ssh_init_test(&key)
        });

        // --- 6) pipeline con más datos que la capacidad del pipe ---
        let _ = paso_con_reintento("pipeline de sosh (6 KiB por un pipe)", &mut fallos, || {
            ssh_pipeline(&key)
        });

        // --- 7) sesión SSH autenticada + halt ---
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

    // --- inferencia con RAM reducida (reclaim de pesos mmap) ---
    let _ = paso("soso-llm con RAM 48M (reclaim)", &mut fallos, || {
        test_reclaim_low_mem(&img, &data, &models)
    });

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

/// Como `paso`, pero reintenta una vez tras una pausa.
///
/// Para los pasos que van por SSH: una reconexión inmediata tras cerrar la
/// sesión anterior falla a veces (el servidor de soso aún está soltando la
/// sesión previa). Es un fallo conocido del arnés, no del sistema, y hacía que la
/// suite diese rojo por algo que a la segunda va — que es la peor clase de test,
/// porque enseña a desconfiar de los rojos. El reintento se ANUNCIA, para que un
/// flake permanente siga siendo visible en vez de quedar tapado.
fn paso_con_reintento<F: FnMut() -> Result<(), String>>(
    nombre: &str,
    fallos: &mut u32,
    mut f: F,
) -> Result<(), ()> {
    if let Err(e) = f() {
        println!("      (reintento de «{nombre}» tras 5 s: {e})");
        std::thread::sleep(Duration::from_secs(5));
        return paso(nombre, fallos, f);
    }
    marca(nombre, true);
    Ok(())
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
/// stdin se mantiene abierto hasta que el hijo muere. Con el parche ya no es
/// imprescindible, pero cerrarlo antes manda un EOF que el servidor no necesita
/// ver, y esa es justo la piedra en la que tropezó todo esto (2026-07-28).
fn ssh_guion(key: &std::path::Path, guion: &str, limite: Duration) -> Result<String, String> {
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

    let pid = hijo.id();
    let mut stdin = hijo.stdin.take().unwrap();
    stdin.write_all(guion.as_bytes()).map_err(|e| e.to_string())?;
    stdin.flush().ok();

    // `wait_with_output` en un hilo: además de esperar, drena stdout/stderr, que
    // con `cat /README.md` dos veces mueven más que el buffer de un pipe.
    let hilo = std::thread::spawn(move || hijo.wait_with_output());

    let fin = Instant::now() + limite;
    while !hilo.is_finished() {
        if Instant::now() >= fin {
            drop(stdin);
            let _ = Command::new("kill").arg(pid.to_string()).status();
            let texto = hilo
                .join()
                .map_err(|_| "hilo ssh".to_string())?
                .map(|s| String::from_utf8_lossy(&s.stdout).into_owned())
                .unwrap_or_default();
            return Err(format!(
                "la sesión SSH no terminó en {}s; stdout: {texto:?}",
                limite.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    drop(stdin);

    let salida = hilo
        .join()
        .map_err(|_| "hilo ssh".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&salida.stdout).into_owned())
}

fn ssh_llm(key: &std::path::Path) -> Result<(), String> {
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

fn ssh_llm_moe(key: &std::path::Path) -> Result<(), String> {
    let texto = ssh_guion(
        key,
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

/// Un pipeline de sosh moviendo MÁS datos que la capacidad del pipe (4 KiB).
///
/// Es la prueba de punta a punta del camino que se arregló: el pipe transfería 256
/// bytes por llamada (el tamaño de un temporal del kernel) y el userspace no miraba
/// el retorno, así que se perdía la cola de cada trozo. Se piden DOS copias del
/// README por un solo pipe para que se llene de verdad y `write_all` tenga que
/// completar escrituras cortas; se compara byte a byte contra el fichero real, que
/// el host lee en el momento (nada hardcodeado).
fn ssh_pipeline(key: &std::path::Path) -> Result<(), String> {
    let real = std::fs::read(super::project_root().join("rootfs/README.md"))
        .map_err(|e| format!("no se pudo leer rootfs/README.md: {e}"))?;

    let texto = ssh_guion(
        key,
        "cat /README.md /README.md | cat -\nexit\n",
        Duration::from_secs(90),
    )?
    .replace("\r\n", "\n");
    let marca = "cat -\n";
    let ini = texto
        .find(marca)
        .ok_or_else(|| format!("no se vio el comando en la salida: {texto:?}"))?
        + marca.len();
    let fin = texto[ini..]
        .rfind("$ ")
        .map(|p| ini + p)
        .unwrap_or(texto.len());
    let cuerpo = &texto[ini..fin];
    let esperado = String::from_utf8_lossy(&real).replace("\r\n", "\n").repeat(2);
    if cuerpo.trim_end() != esperado.trim_end() {
        return Err(format!(
            "el pipeline entregó {} bytes y el fichero ×2 son {}",
            cuerpo.trim_end().len(),
            esperado.trim_end().len()
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
    // 150 s bastaban con KVM; sin él (TCG) esta batería —hilos, futex, estrés de
    // FPU/YMM y el camino de syscalls GPU— se pone en varios minutos. Ver la nota
    // del límite en `ssh_llm`: pasarse de corto aquí no cuesta un rojo, cuesta la
    // suite entera desde este punto.
    let texto = ssh_guion(key, "init test\nexit\n", Duration::from_secs(420))?;
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
    // Acaba en `halt`: aquí la sesión no se cierra porque salga la shell, sino
    // porque el guest se apaga y se lleva la conexión por delante. Vale igual para
    // esperar al cliente, y de paso deja de ser una carrera contra un sleep de 4 s.
    let guion = format!("echo {token} > /tmp/xtask.txt\ncat /tmp/xtask.txt\nhalt\n");
    let texto = ssh_guion(key, &guion, Duration::from_secs(60))?;
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

/// Segunda instancia QEMU con poca RAM: el reclaim del kernel debe permitir
/// completar la inferencia del modelo tiny (pesos en disco, streaming).
fn test_reclaim_low_mem(
    img: &std::path::Path,
    data: &std::path::Path,
    models: &std::path::Path,
) -> Result<(), String> {
    let root = super::project_root();
    let serial = root.join("target/test-reclaim-serial.log");
    let _ = std::fs::remove_file(&serial);
    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35", "-cpu", "max"])
        .args(["-m", "48M"])
        .args(["-smp", "1"]);
    super::apply_firmware(&mut qemu, img);
    qemu.args(["-drive", &format!("format=raw,file={}", img.display())]);
    super::apply_qemu_disks(&mut qemu, data, models);
    super::apply_qemu_nic(&mut qemu);
    super::apply_qemu_gpu(&mut qemu);
    qemu.args(["-serial", &format!("file:{}", serial.display())])
        .args(["-display", "none"])
        .arg("-no-reboot")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _vivo = QemuVivo(qemu.spawn().map_err(|e| e.to_string())?);
    // Arrancar con 48 MiB obliga al kernel a reclamar desde el primer momento, y
    // sin KVM eso se pasa de los 120 s que bastaban con aceleración.
    esperar_en_fichero(&serial, "sosh —", Duration::from_secs(300))?;
    let key = root.join("target/soso_test_key");
    let texto = ssh_guion(
        &key,
        "soso-llm run tiny --prompt x --max 2\nexit\n",
        // Dos tokens, pero con 48 MiB el reclaim relee shards todo el rato y sin
        // KVM eso son minutos. Mismo razonamiento que en `ssh_llm`.
        Duration::from_secs(420),
    )?;
    drop(_vivo);
    if !texto.contains("soso-llm: generado") {
        return Err(format!(
            "inferencia con 48M no completó; stdout: {texto:?}"
        ));
    }
    if !texto.contains("soso-llm: planificador") {
        return Err(format!(
            "falta salida del planificador con 48M; stdout: {texto:?}"
        ));
    }
    Ok(())
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
