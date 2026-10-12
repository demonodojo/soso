//! `cargo xtask fb-shot` — captura la pantalla del guest desde QEMU.
//!
//! La rotación de la consola (Steam Deck: panel vertical montado girado) no se
//! puede dar por buena con tests aritméticos: hay que **mirar** el resultado.
//! Este comando arranca la imagen, espera a un marcador en la serie, pide un
//! `screendump` por el monitor y deja un PNG.
//!
//! ```sh
//! cargo xtask fb-shot                      # rotación del build actual
//! SOSO_FB_ROT=270 cargo xtask build && cargo xtask fb-shot --out target/rot270.png
//! ```

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Marcador de la serie que indica que la consola ya ha pintado el arranque.
const MARCADOR: &str = "boot: task";

pub fn run(args: &[String]) {
    let root = crate::project_root();
    let mut out = root.join("target/fb-shot.png");
    let mut marcador = MARCADOR.to_string();
    let mut espera = 90u64;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => {
                if let Some(v) = it.next() {
                    out = PathBuf::from(v);
                }
            }
            "--marcador" => {
                if let Some(v) = it.next() {
                    marcador = v.clone();
                }
            }
            "--espera" => {
                if let Some(v) = it.next() {
                    espera = v.parse().unwrap_or(espera);
                }
            }
            otro => {
                eprintln!("fb-shot: argumento desconocido {otro}");
                std::process::exit(2);
            }
        }
    }

    // La captura se hace sobre la imagen **UEFI** siempre que haya OVMF: es lo
    // que arranca una placa real, y el camino BIOS no carga un kernel con la
    // capa lxdde enlazada (el bootloader ni llega a escribir en la serie).
    let img = {
        let por_defecto = crate::build_image();
        let uefi = root.join("target/soso-uefi.img");
        if uefi.exists() && crate::ovmf_paths().is_some() {
            uefi
        } else {
            por_defecto
        }
    };
    let serie = root.join("target/fb-shot-serial.log");
    let monitor = root.join("target/fb-shot-monitor.sock");
    let ppm = root.join("target/fb-shot.ppm");
    let _ = std::fs::remove_file(&serie);
    let _ = std::fs::remove_file(&monitor);
    let _ = std::fs::remove_file(&ppm);

    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-machine", "q35"])
        .args(["-cpu", "max"])
        .args(["-m", "2G"])
        .args(["-smp", "1"]);
    crate::apply_qemu_accel(&mut qemu);
    // `SOSO_QEMU_IOMMU=1`: AMD-Vi emulado, para ejercer el camino IVRS.
    if matches!(
        std::env::var("SOSO_QEMU_IOMMU").as_deref(),
        Ok("1") | Ok("true")
    ) {
        qemu.args(["-device", "amd-iommu"]);
        println!("fb-shot: amd-iommu emulado");
    }
    aplicar_firmware_fresco(&mut qemu, &root, &img);
    // Dispositivos USB según el entorno (`SOSO_QEMU_USB_KBD`, `SOSO_QEMU_USB_HUB`):
    // así una captura puede ejercer también la enumeración y el inventario USB.
    crate::apply_qemu_usb(&mut qemu, &crate::QemuGuestConfig::from_env());
    qemu.args(["-drive", &format!("format=raw,file={}", img.display())])
        .args(["-display", "none"])
        .args([
            "-monitor",
            &format!("unix:{},server,nowait", monitor.display()),
        ])
        .args(["-serial", &format!("file:{}", serie.display())])
        .arg("-no-reboot")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());

    let mut hijo = qemu.spawn().expect("no se pudo ejecutar qemu-system-x86_64");
    let resultado = capturar(&serie, &monitor, &ppm, &marcador, espera);
    let _ = hijo.kill();
    let _ = hijo.wait();
    esperar_muerte(&mut hijo);

    match resultado {
        Ok(()) => {}
        Err(e) => {
            eprintln!("fb-shot: {e}");
            if let Ok(s) = std::fs::read_to_string(&serie) {
                let cola: Vec<&str> = s.lines().rev().take(15).collect();
                for l in cola.iter().rev() {
                    eprintln!("  serie| {l}");
                }
            }
            std::process::exit(1);
        }
    }

    match ppm_a_png(&ppm, &out) {
        Ok((w, h)) => println!("fb-shot: {} ({w}x{h})", out.display()),
        Err(e) => {
            eprintln!("fb-shot: no pude convertir {}: {e}", ppm.display());
            std::process::exit(1);
        }
    }
}

/// OVMF con variables **nuevas en cada captura**.
///
/// La captura termina matando QEMU sin apagado limpio, y eso puede dejar a
/// medio escribir el fichero de variables que comparten el resto de comandos:
/// el arranque siguiente se queda dentro del firmware, sin llegar nunca al
/// kernel, y el síntoma es un log de serie vacío que parece un cuelgue del
/// propio soso.
fn aplicar_firmware_fresco(qemu: &mut Command, root: &Path, img: &Path) {
    let es_uefi = img.file_name().and_then(|n| n.to_str()) == Some("soso-uefi.img");
    if !es_uefi {
        return;
    }
    let Some((code, vars_src)) = crate::ovmf_paths() else {
        return;
    };
    let vars = root.join("target/fb-shot-vars.fd");
    if let Err(e) = std::fs::copy(&vars_src, &vars) {
        eprintln!("fb-shot: no pude copiar las variables OVMF: {e}");
        return;
    }
    qemu.args([
        "-drive",
        &format!("if=pflash,format=raw,readonly=on,file={}", code.display()),
    ]);
    qemu.args([
        "-drive",
        &format!("if=pflash,format=raw,file={}", vars.display()),
    ]);
}

fn esperar_muerte(hijo: &mut Child) {
    let limite = Instant::now() + Duration::from_secs(5);
    while Instant::now() < limite {
        if matches!(hijo.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn capturar(
    serie: &Path,
    monitor: &Path,
    ppm: &Path,
    marcador: &str,
    espera_s: u64,
) -> Result<(), String> {
    esperar_en_serie(serie, marcador, espera_s)?;
    // El marcador sale antes de que la consola termine de pintar la línea;
    // un respiro corto evita capturar media pantalla.
    std::thread::sleep(Duration::from_millis(500));

    let mut mon = conectar_monitor(monitor)?;
    let cmd = format!("screendump {}\n", ppm.display());
    mon.write_all(cmd.as_bytes())
        .map_err(|e| format!("monitor: {e}"))?;
    mon.flush().ok();

    let limite = Instant::now() + Duration::from_secs(20);
    while Instant::now() < limite {
        if let Ok(m) = std::fs::metadata(ppm) {
            if m.len() > 0 {
                // El screendump se escribe en caliente: esperar a que el
                // tamaño se estabilice antes de leerlo.
                std::thread::sleep(Duration::from_millis(300));
                let ahora = std::fs::metadata(ppm).map(|m| m.len()).unwrap_or(0);
                if ahora == m.len() {
                    return Ok(());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err("el screendump no apareció".into())
}

fn esperar_en_serie(serie: &Path, marcador: &str, espera_s: u64) -> Result<(), String> {
    let limite = Instant::now() + Duration::from_secs(espera_s);
    while Instant::now() < limite {
        if let Ok(mut f) = std::fs::File::open(serie) {
            let mut s = String::new();
            if f.read_to_string(&mut s).is_ok() && s.contains(marcador) {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(format!("no apareció «{marcador}» en {espera_s}s"))
}

fn conectar_monitor(monitor: &Path) -> Result<UnixStream, String> {
    let limite = Instant::now() + Duration::from_secs(10);
    loop {
        match UnixStream::connect(monitor) {
            Ok(s) => {
                s.set_read_timeout(Some(Duration::from_millis(500))).ok();
                return Ok(s);
            }
            Err(e) if Instant::now() < limite => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(format!("monitor {}: {e}", monitor.display())),
        }
    }
}

// ---- PPM (P6) → PNG ----
//
// Sin dependencias: el PNG se escribe con un único bloque deflate «stored», que
// no necesita compresor. Ocupa más, pero es una captura de depuración.

fn ppm_a_png(ppm: &Path, png: &Path) -> Result<(usize, usize), String> {
    let datos = std::fs::read(ppm).map_err(|e| e.to_string())?;
    let (w, h, pix) = leer_ppm(&datos)?;

    // Filas PNG: cada una con byte de filtro 0 delante.
    let mut raw = Vec::with_capacity(h * (1 + w * 3));
    for y in 0..h {
        raw.push(0u8);
        raw.extend_from_slice(&pix[y * w * 3..(y + 1) * w * 3]);
    }

    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8 bits, RGB
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(png, &out).map_err(|e| e.to_string())?;
    Ok((w, h))
}

fn leer_ppm(d: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    if d.len() < 2 || &d[0..2] != b"P6" {
        return Err("no es un PPM P6".into());
    }
    let mut pos = 2usize;
    let mut campos = Vec::new();
    while campos.len() < 3 {
        while pos < d.len() && (d[pos] as char).is_whitespace() {
            pos += 1;
        }
        if pos < d.len() && d[pos] == b'#' {
            while pos < d.len() && d[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        let ini = pos;
        while pos < d.len() && !(d[pos] as char).is_whitespace() {
            pos += 1;
        }
        let t = core::str::from_utf8(&d[ini..pos]).map_err(|e| e.to_string())?;
        campos.push(t.parse::<usize>().map_err(|e| e.to_string())?);
    }
    pos += 1; // el único separador tras maxval
    let (w, h) = (campos[0], campos[1]);
    let n = w * h * 3;
    if d.len() < pos + n {
        return Err(format!("PPM truncado: {} < {}", d.len() - pos, n));
    }
    Ok((w, h, d[pos..pos + n].to_vec()))
}

fn chunk(out: &mut Vec<u8>, tipo: &[u8; 4], datos: &[u8]) {
    out.extend_from_slice(&(datos.len() as u32).to_be_bytes());
    out.extend_from_slice(tipo);
    out.extend_from_slice(datos);
    let mut crc = Crc32::new();
    crc.update(tipo);
    crc.update(datos);
    out.extend_from_slice(&crc.valor().to_be_bytes());
}

/// Flujo zlib con bloques deflate sin comprimir.
fn zlib_stored(datos: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut i = 0usize;
    while i < datos.len() {
        let n = (datos.len() - i).min(65535);
        let ultimo = if i + n >= datos.len() { 1u8 } else { 0u8 };
        out.push(ultimo);
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&(!(n as u16)).to_le_bytes());
        out.extend_from_slice(&datos[i..i + n]);
        i += n;
    }
    out.extend_from_slice(&adler32(datos).to_be_bytes());
    out
}

fn adler32(d: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in d {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

struct Crc32 {
    v: u32,
}

impl Crc32 {
    fn new() -> Self {
        Crc32 { v: 0xffff_ffff }
    }
    fn update(&mut self, d: &[u8]) {
        for &b in d {
            self.v ^= b as u32;
            for _ in 0..8 {
                let m = (self.v & 1).wrapping_neg();
                self.v = (self.v >> 1) ^ (0xedb8_8320 & m);
            }
        }
    }
    fn valor(self) -> u32 {
        !self.v
    }
}
