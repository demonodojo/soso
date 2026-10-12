//! mkfs-soso: crea una imagen sosofs a partir de un directorio.
//!
//! Uso: mkfs-soso <directorio> <imagen> [tamaño_MiB] [clave_pública_autorizada]
//!
//! Antes de construir la imagen inyecta en `<directorio>/etc`:
//! - `ssh_host_key`: semilla ed25519 de 32 bytes (persistente: solo se
//!   genera si no existe, para que la host key no cambie entre mkfs).
//! - `authorized_key`: los 32 bytes de la clave pública ed25519 indicada
//!   (extraídos de su formato OpenSSH), si se pasa una.

use block_dev::{BLOCK_SIZE, FileBlockDevice};
use std::io::Read;
use std::path::Path;
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("uso: mkfs-soso <directorio> <imagen> [tamaño_MiB] [clave_pública]");
        exit(2);
    }
    let src = Path::new(&args[1]);
    let img = Path::new(&args[2]);
    let mib: u64 = args.get(3).map_or(64, |s| s.parse().expect("tamaño inválido"));
    let blocks = mib * 1024 * 1024 / BLOCK_SIZE as u64;

    inyectar_claves(src, args.get(4).map(String::as_str));

    let mut dev = FileBlockDevice::create(img, blocks).unwrap_or_else(|e| {
        eprintln!("mkfs-soso: no se pudo crear {}: {e}", img.display());
        exit(1);
    });
    match sosofs::builder::build_image(src, &mut dev) {
        Ok(stats) => {
            println!(
                "sosofs: {} inodes, {}/{} bloques usados ({} MiB) -> {}",
                stats.inodes,
                stats.blocks_used,
                stats.block_count,
                stats.blocks_used * BLOCK_SIZE as u64 / (1024 * 1024),
                img.display()
            );
        }
        Err(e) => {
            eprintln!("mkfs-soso: {e:?}");
            exit(1);
        }
    }
}

/// Escribe la host key (si falta) y la clave autorizada (si se pasa) en
/// `<src>/etc`, para que `build_image` las incluya en la imagen.
fn inyectar_claves(src: &Path, pubkey_path: Option<&str>) {
    let etc = src.join("etc");
    std::fs::create_dir_all(&etc).expect("no se pudo crear <rootfs>/etc");

    // Host key: 32 bytes aleatorios, persistente.
    let host_key = etc.join("ssh_host_key");
    if !host_key.exists() {
        let mut seed = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut seed))
            .expect("no se pudo leer /dev/urandom");
        std::fs::write(&host_key, seed).expect("no se pudo escribir ssh_host_key");
        println!("mkfs-soso: host key ed25519 generada -> etc/ssh_host_key");
    }

    // Clave autorizada: extraer los 32 bytes de la clave pública OpenSSH.
    if let Some(path) = pubkey_path {
        match leer_pubkey_ed25519(path) {
            Ok(key) => {
                std::fs::write(etc.join("authorized_key"), key)
                    .expect("no se pudo escribir authorized_key");
                println!("mkfs-soso: clave autorizada desde {path} -> etc/authorized_key");
            }
            Err(e) => {
                eprintln!("mkfs-soso: aviso: no pude leer la clave pública {path}: {e}");
            }
        }
    }
}

/// Extrae los 32 bytes de una clave pública ed25519 en formato OpenSSH
/// ("ssh-ed25519 <base64> [comentario]"). El blob base64 es:
/// u32(len) "ssh-ed25519" u32(32) <32 bytes>. Devolvemos esos 32 bytes.
fn leer_pubkey_ed25519(path: &str) -> Result<[u8; 32], String> {
    let contenido = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let campo = contenido
        .split_whitespace()
        .nth(1)
        .ok_or("formato de clave pública inesperado")?;
    let blob = base64_decode(campo)?;
    // Estructura: [4] len="ssh-ed25519"(11) + "ssh-ed25519" + [4] 32 + [32]
    if blob.len() < 4 + 11 + 4 + 32 {
        return Err("blob demasiado corto".into());
    }
    let tipo = &blob[4..4 + 11];
    if tipo != b"ssh-ed25519" {
        return Err("la clave no es ed25519".into());
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&blob[blob.len() - 32..]);
    Ok(key)
}

/// Decodificador base64 estándar (suficiente para claves OpenSSH).
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &c in s.trim().as_bytes() {
        if c == b'=' {
            break;
        }
        let v = val(c).ok_or("carácter base64 inválido")? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}
