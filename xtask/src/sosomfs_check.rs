//! `cargo xtask sosomfs-check [ruta]`
//!
//! Diagnostica el volumen de modelos (sosomfs) sin montarlo: lee los dos slots
//! de superbloque y dice cuál sobrevive y por qué.
//!
//! Existe porque `fs: live sosomfs falló (…)` en el arranque de una placa no
//! dice nada accionable, y para saber si el volumen tiene arreglo hay que mirar
//! los bytes. Acepta el pendrive entero (busca la partición 3), una partición
//! suelta o una imagen (`soso-live.img` con GPT, o una de modelos a pelo).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::exit;

const BLOCK: usize = 4096;
const SECTOR: u64 = 512;
const MAGIC: &[u8; 8] = b"SOSOMFS1";
/// Slots de superbloque A/B (`sosomfs::layout::SUPERBLOCK_SLOTS`).
const SLOTS: u64 = 2;

pub fn run(args: &[String]) {
    let mut ruta: Option<PathBuf> = None;
    for a in args {
        match a.as_str() {
            "-h" | "--help" => usage(),
            s if s.starts_with('-') => {
                eprintln!("sosomfs-check: opción desconocida: {s}");
                usage();
            }
            s => ruta = Some(PathBuf::from(s)),
        }
    }
    let ruta = ruta.unwrap_or_else(|| {
        crate::project_root().join("target/usb-live/soso-live.img")
    });
    if !ruta.exists() {
        eprintln!("sosomfs-check: no existe {}", ruta.display());
        exit(1);
    }

    let base = match localizar_volumen(&ruta) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("sosomfs-check: {e}");
            exit(1);
        }
    };
    println!("sosomfs-check: {} (volumen en el byte {base})", ruta.display());

    let mut f = match File::open(&ruta) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("sosomfs-check: no puedo abrir {}: {e}", ruta.display());
            eprintln!("  ¿es un dispositivo? prueba con sudo");
            exit(1);
        }
    };

    let mut validos = Vec::new();
    for slot in 0..SLOTS {
        match leer_slot(&mut f, base, slot) {
            Ok(sb) => {
                println!(
                    "  slot {slot}: magic OK, crc {}, generación {}, {} bloques, \
                     data_start {}, catálogo {}+{}",
                    if sb.crc_ok { "OK" } else { "MAL" },
                    sb.generation,
                    sb.total_blocks,
                    sb.data_start,
                    sb.catalog_root,
                    sb.catalog_blocks,
                );
                if sb.crc_ok {
                    validos.push(sb);
                }
            }
            Err(e) => println!("  slot {slot}: {e}"),
        }
    }

    println!();
    match validos.iter().max_by_key(|s| s.generation) {
        Some(sb) => {
            println!(
                "✅ montaría con la generación {} ({} bloques)",
                sb.generation, sb.total_blocks
            );
            if validos.len() < SLOTS as usize {
                println!(
                    "⚠  solo queda {} superbloque de {SLOTS}: sin redundancia. Un fallo más y \
                     el volumen se pierde; reflashea cuando puedas.",
                    validos.len()
                );
            }
        }
        None => {
            println!("❌ ningún superbloque válido: el volumen no monta (SinSuperbloque)");
            println!(
                "   Los dos slots se escriben solo al ampliar el volumen (`grow`, primer\n   \
                 arranque de un pendrive recién flasheado). Reflashea: no hay a qué volver."
            );
            exit(2);
        }
    }
}

struct Sb {
    crc_ok: bool,
    generation: u64,
    total_blocks: u64,
    data_start: u64,
    catalog_root: u64,
    catalog_blocks: u64,
}

fn leer_slot(f: &mut File, base: u64, slot: u64) -> Result<Sb, String> {
    let mut buf = [0u8; BLOCK];
    f.seek(SeekFrom::Start(base + slot * BLOCK as u64))
        .map_err(|e| format!("seek: {e}"))?;
    f.read_exact(&mut buf).map_err(|e| format!("lectura: {e}"))?;
    if &buf[0..8] != MAGIC {
        return Err(format!(
            "sin magic (empieza por {:02x?}) — escritura a medias o volumen ajeno",
            &buf[0..8]
        ));
    }
    let crc = u32::from_le_bytes(buf[8..12].try_into().unwrap());
    Ok(Sb {
        crc_ok: crc == crc32c(&buf[16..]),
        generation: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
        total_blocks: u64::from_le_bytes(buf[28..36].try_into().unwrap()),
        data_start: u64::from_le_bytes(buf[48..56].try_into().unwrap()),
        catalog_root: u64::from_le_bytes(buf[40..48].try_into().unwrap()),
        catalog_blocks: u64::from_le_bytes(buf[60..68].try_into().unwrap()),
    })
}

/// Byte en el que empieza el volumen: 0 si ya es una imagen de modelos, o el
/// arranque de la partición 3 si lo que hay delante es una GPT.
fn localizar_volumen(ruta: &Path) -> Result<u64, String> {
    let mut f = File::open(ruta).map_err(|e| format!("no puedo abrir: {e}"))?;
    let mut cab = [0u8; 8];
    if f.read_exact(&mut cab).is_ok() && &cab == MAGIC {
        return Ok(0);
    }
    // GPT: cabecera en LBA 1, entradas en LBA 2 (128 B cada una).
    let mut hdr = [0u8; 512];
    f.seek(SeekFrom::Start(SECTOR))
        .and_then(|_| f.read_exact(&mut hdr))
        .map_err(|e| format!("leyendo la GPT: {e}"))?;
    if &hdr[0..8] != b"EFI PART" {
        return Err("no es ni un volumen sosomfs ni una imagen con GPT".into());
    }
    let entries_lba = u64::from_le_bytes(hdr[72..80].try_into().unwrap());
    let entry_size = u32::from_le_bytes(hdr[84..88].try_into().unwrap()) as u64;
    // Partición 3 = índice 2.
    let mut ent = vec![0u8; entry_size as usize];
    f.seek(SeekFrom::Start(entries_lba * SECTOR + 2 * entry_size))
        .and_then(|_| f.read_exact(&mut ent))
        .map_err(|e| format!("leyendo la entrada 3 de la GPT: {e}"))?;
    if ent[0..16].iter().all(|&b| b == 0) {
        return Err("la GPT no tiene partición 3 (modelos)".into());
    }
    Ok(u64::from_le_bytes(ent[32..40].try_into().unwrap()) * SECTOR)
}

/// CRC32C (Castagnoli), el mismo que usa sosomfs en el superbloque.
fn crc32c(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0x82F6_3B78 & mask);
        }
    }
    !crc
}

fn usage() -> ! {
    eprintln!("uso: cargo xtask sosomfs-check [ruta]");
    eprintln!();
    eprintln!("Lee los dos superbloques del volumen de modelos y dice si monta.");
    eprintln!("Acepta el pendrive (sudo), una partición suelta o una imagen.");
    eprintln!("Sin argumento usa target/usb-live/soso-live.img.");
    eprintln!();
    eprintln!("Ejemplos:");
    eprintln!("  cargo xtask sosomfs-check");
    eprintln!("  sudo cargo xtask sosomfs-check /dev/sda");
    eprintln!("  sudo cargo xtask sosomfs-check /dev/sda3");
    eprintln!("  cargo xtask sosomfs-check target/soso-models-live.img");
    exit(2)
}
