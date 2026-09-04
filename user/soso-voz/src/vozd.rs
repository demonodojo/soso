//! Demonio de voz en 127.0.0.1:7421.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use libsoso::{print, println, sys};
use soso_abi::{self as abi, AudioFormat, O_RDONLY};
use soso_audio::{log_mel_spectrogram, pcm16_to_f32, rms_pcm16, wav, whisper_mel_frame_count, N_MELS, SAMPLE_RATE};
use soso_llm_core::asr::{AsrProfile, AsrRuntime};
use soso_llm_core::gpu::GpuDispatch;
use soso_llm_core::source::{FileMapper, MappedShard, MmapTensorSource};
use soso_llm_core::tokenizer::Tokenizer;
use sosomodel::index::TensorIndex;
use sosomodel::manifest::Manifest;

use crate::net::{parse_sock_addr, read_timeout, write_all, TcpFd};

pub const VOZ_PORT: u16 = 7421;
const VOZ_ADDR: &str = "127.0.0.1:7421";
pub const PROTO_FIN: u8 = 0xFF;
const CONF: &str = "/etc/voz.conf";
const LINE_MAX: usize = 1024;

struct Mapper;

impl FileMapper for Mapper {
    fn map_file(&mut self, path: &str) -> Result<MappedShard, ()> {
        let fd = sys::open(path, O_RDONLY);
        if fd < 0 {
            return Err(());
        }
        let mut st = abi::Stat::default();
        if sys::stat(path, &mut st) < 0 {
            sys::close(fd as u64);
            return Err(());
        }
        let size = st.size as usize;
        let map = sys::mmap(0, size as u64, fd as u64, 0);
        sys::close(fd as u64);
        if map < 0 {
            return Err(());
        }
        let ptr = map as *const u8;
        let _ = unsafe { core::ptr::read_volatile(ptr) };
        Ok(MappedShard {
            addr: map as u64,
            len: size,
        })
    }

    fn unmap_file(&mut self, shard: &MappedShard) {
        let aligned = shard.len.next_multiple_of(4096);
        let _ = sys::munmap(shard.addr, aligned as u64);
    }
}

struct Sesion {
    rt: AsrRuntime<MmapTensorSource<Mapper>>,
    gpu: Option<soso_gpu::SysGpu>,
    use_gpu: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GpuMode {
    Auto,
    On,
    Off,
}

struct Conf {
    modelo: String,
    idioma: u32,
    vad_umbral: f32,
    gpu: GpuMode,
}

impl Default for Conf {
    fn default() -> Self {
        Self {
            modelo: String::new(),
            idioma: 3,
            vad_umbral: 0.01,
            gpu: GpuMode::Auto,
        }
    }
}

fn leer_conf() -> Conf {
    let mut c = Conf::default();
    let Some(texto) = leer_fichero(CONF) else {
        return c;
    };
    for linea in texto.lines() {
        let linea = linea.trim();
        if linea.is_empty() || linea.starts_with('#') {
            continue;
        }
        if let Some(v) = linea.strip_prefix("modelo=") {
            c.modelo = v.trim().to_string();
        } else if let Some(v) = linea.strip_prefix("idioma=") {
            if let Ok(n) = v.trim().parse() {
                c.idioma = n;
            }
        } else if let Some(v) = linea.strip_prefix("vad=") {
            if let Ok(n) = v.trim().parse() {
                c.vad_umbral = n;
            }
        } else if let Some(v) = linea.strip_prefix("gpu=") {
            c.gpu = match v.trim() {
                "on" | "1" | "true" => GpuMode::On,
                "off" | "0" | "false" => GpuMode::Off,
                _ => GpuMode::Auto,
            };
        }
    }
    c
}

fn leer_fichero(path: &str) -> Option<String> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut out = String::new();
    let mut buf = [0u8; 512];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n <= 0 {
            break;
        }
        out.push_str(core::str::from_utf8(&buf[..n as usize]).unwrap_or(""));
    }
    sys::close(fd as u64);
    Some(out)
}

fn leer_bytes(path: &str) -> Option<Vec<u8>> {
    let fd = sys::open(path, O_RDONLY);
    if fd < 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sys::read(fd as u64, &mut buf);
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    sys::close(fd as u64);
    Some(out)
}

fn modelos() -> Vec<String> {
    let mut out = Vec::new();
    let fd = sys::open("/models", O_RDONLY);
    if fd < 0 {
        return out;
    }
    let mut ents = [abi::Dirent::default(); 16];
    loop {
        let n = sys::getdents(fd as u64, &mut ents);
        if n <= 0 {
            break;
        }
        for d in &ents[..n as usize / abi::DIRENT_SIZE] {
            if let Ok(nombre) = core::str::from_utf8(d.name_bytes()) {
                out.push(nombre.to_string());
            }
        }
    }
    sys::close(fd as u64);
    out
}

fn modelo_efectivo(conf: &Conf) -> Option<String> {
    let disp = modelos();
    if !conf.modelo.is_empty() && disp.iter().any(|m| *m == conf.modelo) {
        return Some(conf.modelo.clone());
    }
    if let Some(m) = disp.iter().find(|m| m.contains("asr") || m.contains("whisper")) {
        return Some(m.clone());
    }
    disp.first().cloned()
}

fn cargar_sesion(nombre: &str, gpu_mode: GpuMode) -> Result<Sesion, u8> {
    let base = format!("/models/{nombre}");
    let manifest = Manifest::parse(&leer_bytes(&format!("{base}/manifest.som")).ok_or(1u8)?).map_err(|_| 1u8)?;
    let index = TensorIndex::parse(&leer_bytes(&format!("{base}/index.som")).ok_or(2u8)?).map_err(|_| 2u8)?;
    let tok_data = leer_bytes(&format!("{base}/tokenizer.som"));
    let tokenizer = tok_data
        .and_then(|d| Tokenizer::parse(&d).ok())
        .unwrap_or(Tokenizer::byte_level());
    let source = MmapTensorSource::new(format!("{base}/shards"), index.clone(), Mapper).with_sync_prefetch();
    let rt = AsrRuntime::new(manifest, index, source, tokenizer).map_err(|_| 3u8)?;
    let gpu = match gpu_mode {
        GpuMode::Off => None,
        GpuMode::On | GpuMode::Auto => soso_gpu::SysGpu::new(),
    };
    let use_gpu = match gpu_mode {
        GpuMode::On => gpu.is_some(),
        GpuMode::Off => false,
        GpuMode::Auto => gpu.is_some(),
    };
    if use_gpu {
        if let Some(ref g) = gpu {
            println!(
                "soso-voz: ASR en GPU «{}» (fase={})",
                g.device_name(),
                g.phase()
            );
        }
    } else if gpu_mode == GpuMode::On {
        println!("soso-voz: gpu=on pero sin dispositivo — ASR en CPU");
    }
    let mut sesion = Sesion { rt, gpu, use_gpu };
    if sesion.use_gpu {
        if let Some(ref mut g) = sesion.gpu {
            g.calibrar();
            sesion.rt.sched.model = g.cost_model();
        }
        let mut gref = sesion
            .gpu
            .as_mut()
            .map(|g| g as &mut dyn soso_llm_core::gpu::GpuDispatch);
        let n = sesion.rt.warmup_gpu(&mut gref);
        println!("soso-voz: warmup GPU {n} tensores");
    }
    Ok(sesion)
}

fn transcribir(
    sesion: &mut Sesion,
    mel: &[f32],
    nf: usize,
    idioma: u32,
    max_new_tokens: Option<usize>,
) -> Result<String, ()> {
    sesion.rt.profile = Some(AsrProfile::new(true));
    let mut gpu_ref = sesion.gpu.as_mut().map(|g| g as &mut dyn soso_llm_core::gpu::GpuDispatch);
    let r = sesion.rt.transcribe_with_gpu(
        mel,
        nf,
        idioma,
        sesion.use_gpu,
        &mut gpu_ref,
        max_new_tokens,
    );
    if let Some(ref prof) = sesion.rt.profile {
        if prof.enabled {
            print!("{}", prof.format_phase_summary());
            if !prof.tokens.is_empty() {
                print!("{}", prof.format_token_summaries());
            }
        }
    }
    if sesion.use_gpu {
        if let Some(ref g) = sesion.gpu {
            let (calls, uploads, resident, sin_sitio) = g.stats();
            let gs = g.gpu_stats();
            println!(
                "soso-voz: GPU {calls} matvec, {uploads} subidas, {resident} residentes, {sin_sitio} a CPU, launches={} total_ns={}",
                gs.launches,
                gs.total_ns
            );
        }
    }
    r
}

fn mel_desde_pcm(pcm: &[i16]) -> (Vec<f32>, usize) {
    let mut f32s = vec![0.0f32; pcm.len()];
    pcm16_to_f32(pcm, &mut f32s);
    let cap = whisper_mel_frame_count(f32s.len()).max(1);
    let mut mel = vec![0.0f32; N_MELS * cap];
    let nf = log_mel_spectrogram(&f32s, &mut mel);
    (mel, nf)
}

fn mel_desde_wav(path: &str) -> Result<(Vec<f32>, usize), u8> {
    let data = leer_bytes(path).ok_or(4u8)?;
    let (rate, pcm) = wav::parse_pcm16_mono(&data).map_err(|_| 5u8)?;
    let samples = wav::resample_to_16k(rate, &pcm);
    let cap = whisper_mel_frame_count(samples.len()).max(1);
    let mut mel = vec![0.0f32; N_MELS * cap];
    let nf = log_mel_spectrogram(&samples, &mut mel);
    Ok((mel, nf))
}

fn asegurar_sesion(
    sesion: &mut Option<Sesion>,
    conf: &Conf,
    fd: u64,
) -> bool {
    if sesion.is_some() {
        return true;
    }
    let nombre = match modelo_efectivo(conf) {
        Some(n) => n,
        None => {
            socket_reply(fd, "sin modelo ASR");
            return false;
        }
    };
    match cargar_sesion(&nombre, conf.gpu) {
        Ok(s) => {
            *sesion = Some(s);
            true
        }
        Err(e) => {
            socket_reply(fd, &format!("error carga {e}"));
            false
        }
    }
}

fn capturar_vad(conf: &Conf, max_ms: u32) -> Result<Vec<i16>, u8> {
    let fmt = AudioFormat {
        sample_rate: SAMPLE_RATE,
        channels: 1,
        bits_per_sample: 16,
        _pad: 0,
    };
    if sys::audio_open(&fmt) < 0 {
        return Err(7);
    }
    let mut pcm = Vec::new();
    let mut silencio_ms = 0u32;
    let inicio = sys::uptime_ms();
    let mut buf = [0u8; 2048];
    let mut ov = 0u32;
    loop {
        let n = sys::audio_read(&mut buf, &mut ov);
        if n <= 0 {
            if sys::uptime_ms().saturating_sub(inicio) > max_ms as i64 {
                break;
            }
            let _ = sys::sleep_ms(10);
            continue;
        }
        for chunk in buf[..n as usize].chunks_exact(2) {
            pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
        }
        let tail = 320.min(pcm.len());
        let rms = rms_pcm16(&pcm[pcm.len().saturating_sub(tail)..]);
        if rms < conf.vad_umbral {
            silencio_ms += 20;
            if silencio_ms > 700 && pcm.len() > SAMPLE_RATE as usize / 4 {
                break;
            }
        } else {
            silencio_ms = 0;
        }
        if sys::uptime_ms().saturating_sub(inicio) > max_ms as i64 {
            break;
        }
    }
    let _ = sys::audio_close();
    Ok(pcm)
}

fn socket_reply(fd: u64, msg: &str) {
    let mut v = msg.as_bytes().to_vec();
    v.push(PROTO_FIN);
    let _ = write_all(fd, &v);
}

fn copiar_respuesta(fd: u64) -> Option<String> {
    let mut out = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = read_timeout(fd, &mut buf, 120_000);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == PROTO_FIN {
                return String::from_utf8(out).ok();
            }
            out.push(b);
        }
    }
    None
}

pub fn run_vozd() -> u8 {
    let conf = leer_conf();
    let listener = match TcpFd::listen(VOZ_PORT) {
        Ok(l) => l,
        Err(_) => return 1,
    };
    println!("vozd: escuchando en {VOZ_ADDR}");
    let mut sesion: Option<Sesion> = None;
    loop {
        let Ok(client) = TcpFd::accept(&listener, 60_000) else {
            continue;
        };
        let mut linea = Vec::new();
        let mut byte = [0u8; 1];
        while linea.len() < LINE_MAX {
            let n = read_timeout(client.fd, &mut byte, 30_000);
            if n <= 0 {
                break;
            }
            if byte[0] == b'\n' {
                break;
            }
            linea.push(byte[0]);
        }
        let cmd = core::str::from_utf8(&linea).unwrap_or("").trim();
        if cmd == ":modelos" {
            socket_reply(client.fd, &modelos().join(" "));
            continue;
        }
        if let Some(m) = cmd.strip_prefix(":modelo ") {
            if !asegurar_sesion(&mut sesion, &conf, client.fd) {
                continue;
            }
            socket_reply(client.fd, &format!("modelo={m}"));
            continue;
        }
        if let Some(resto) = cmd.strip_prefix(":wav ") {
            if !asegurar_sesion(&mut sesion, &conf, client.fd) {
                continue;
            }
            let mut parts = resto.split_whitespace();
            let path = parts.next().unwrap_or("");
            let max_new_tokens = parts.next().and_then(|s| s.parse().ok());
            let (mel, nf) = match mel_desde_wav(path) {
                Ok(v) => v,
                Err(e) => {
                    socket_reply(client.fd, &format!("error wav {e}"));
                    continue;
                }
            };
            print!(".");
            match transcribir(
                sesion.as_mut().unwrap(),
                &mel,
                nf,
                conf.idioma,
                max_new_tokens,
            ) {
                Ok(t) => socket_reply(client.fd, &t),
                Err(_) => socket_reply(client.fd, "error asr"),
            }
            continue;
        }
        if cmd.starts_with(":escucha") {
            let max_ms: u32 = cmd
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(15_000);
            if !asegurar_sesion(&mut sesion, &conf, client.fd) {
                continue;
            }
            let pcm = match capturar_vad(&conf, max_ms) {
                Ok(p) => p,
                Err(e) => {
                    socket_reply(client.fd, &format!("error mic {e}"));
                    continue;
                }
            };
            let (mel, nf) = mel_desde_pcm(&pcm);
            print!(".");
            match transcribir(
                sesion.as_mut().unwrap(),
                &mel,
                nf,
                conf.idioma,
                None,
            ) {
                Ok(t) => socket_reply(client.fd, &t),
                Err(_) => socket_reply(client.fd, "error asr"),
            }
            continue;
        }
        socket_reply(client.fd, "error cmd");
    }
}

pub fn preguntar_via_vozd(cmd: &str) -> Option<String> {
    let addr = parse_sock_addr(VOZ_ADDR)?;
    let mut client = loop {
        match TcpFd::connect(&addr, 2_000) {
            Ok(c) => break c,
            Err(_) => {
                let _ = spawn_vozd();
                let _ = sys::sleep_ms(50);
            }
        }
    };
    for _ in 0..100 {
        if TcpFd::connect(&addr, 100).is_ok() {
            client = TcpFd::connect(&addr, 5_000).ok()?;
            break;
        }
        let _ = sys::sleep_ms(50);
    }
    let mut req = cmd.as_bytes().to_vec();
    req.push(b'\n');
    write_all(client.fd, &req).ok()?;
    copiar_respuesta(client.fd)
}

pub fn spawn_vozd() -> Result<u64, i64> {
    let rc = sys::spawn_io(
        "/bin/soso-voz",
        "vozd",
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
        abi::FD_SERIAL_TTY,
    );
    if rc < 0 { Err(rc) } else { Ok(rc as u64) }
}
