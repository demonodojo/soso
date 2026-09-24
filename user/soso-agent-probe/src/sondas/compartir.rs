//! Sonda 3 — varios descriptores sobre el mismo fichero (pase 3 de T33).
//!
//! Es lo que necesita SQLite, y por tanto lo que necesita el `bun:sqlite` que
//! [T32](../../../../docs/self-improvement/T32-opencode-inventario.md) sitúa
//! dentro de Bun: dos conexiones abren el mismo `.db`, leen por posición y
//! escriben sin pisarse. T32 anotó que en el ABI no hay `pread` ni
//! `flock`/`fcntl`; lo que falta es saber **qué pasa de verdad** cuando dos
//! descriptores miran el mismo fichero.
//!
//! No se mide la existencia de una syscall —eso ya se sabe leyendo el ABI—
//! sino la **consecuencia**: si un escritor se ve desde el otro descriptor, si
//! los offsets son independientes, y quién gana cuando los dos escriben.
//! Cualquiera de esas respuestas cambia lo que habría que enseñarle a SQLite.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{abi, println, sys};

use crate::{errno, limpiar_dir, unir, Caso};

const DIR: &str = "/var/self-improvement/probe/compartir";

fn anotar(casos: &mut Vec<Caso>, c: Caso) {
    if c.paso {
        println!("probe: {} ok ({})", c.id, c.observado);
    } else {
        println!(
            "probe: {} FALLO esperado={:?} observado={:?}",
            c.id, c.esperado, c.observado
        );
    }
    casos.push(c);
}

fn escribir_nuevo(ruta: &str, datos: &[u8]) -> Result<(), String> {
    let fd = sys::open(ruta, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
    if fd < 0 {
        return Err(format!("open = {}", errno(fd)));
    }
    let fd = fd as u64;
    let mut n = 0;
    while n < datos.len() {
        let w = sys::write(fd, &datos[n..]);
        if w <= 0 {
            sys::close(fd);
            return Err(format!("write = {}", errno(w)));
        }
        n += w as usize;
    }
    sys::fsync(fd);
    sys::close(fd);
    Ok(())
}

fn leer_todo(ruta: &str) -> Result<Vec<u8>, String> {
    let fd = sys::open(ruta, abi::O_RDONLY);
    if fd < 0 {
        return Err(format!("open = {}", errno(fd)));
    }
    let fd = fd as u64;
    let mut out = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = sys::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    sys::close(fd);
    Ok(out)
}

/// Lee `n` bytes desde `off` usando `seek`+`read`, que es el sustituto de
/// `pread` cuando no hay lectura posicional.
fn leer_en(fd: u64, off: u64, n: usize) -> Result<Vec<u8>, String> {
    let rc = sys::seek(fd, off as i64, abi::SEEK_SET);
    if rc < 0 {
        return Err(format!("seek = {}", errno(rc)));
    }
    let mut buf = alloc::vec![0u8; n];
    let leidos = sys::read(fd, &mut buf);
    if leidos < 0 {
        return Err(format!("read = {}", errno(leidos)));
    }
    buf.truncate(leidos as usize);
    Ok(buf)
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();
    sys::mkdir("/var/self-improvement");
    sys::mkdir("/var/self-improvement/probe");
    limpiar_dir(DIR);

    // Un fichero con posiciones reconocibles: cada bloque de 10 bytes empieza
    // por su dígito, así que leer en el offset 20 tiene que dar «2».
    let mut base = Vec::new();
    for d in 0..6u8 {
        base.extend_from_slice(&[b'0' + d; 10]);
    }
    let ruta = unir(DIR, "base.dat");
    if let Err(e) = escribir_nuevo(&ruta, &base) {
        anotar(&mut casos, Caso::nuevo("compartir/sembrar", "escrito", e));
        return casos;
    }
    anotar(
        &mut casos,
        Caso::nuevo("compartir/sembrar", "escrito", "escrito"),
    );

    // 1. Dos descriptores, offsets independientes. Si el `pos` viviera en el
    //    inodo y no en el descriptor, el `seek` de uno movería al otro y las
    //    dos lecturas darían lo mismo.
    let a = sys::open(&ruta, abi::O_RDONLY);
    let b = sys::open(&ruta, abi::O_RDONLY);
    if a < 0 || b < 0 {
        anotar(
            &mut casos,
            Caso::nuevo(
                "compartir/dos-descriptores",
                "dos abiertos",
                format!("a={} b={}", errno(a), errno(b)),
            ),
        );
        return casos;
    }
    let (a, b) = (a as u64, b as u64);
    let la = leer_en(a, 0, 5).unwrap_or_default();
    let lb = leer_en(b, 30, 5).unwrap_or_default();
    // Y ahora otra vez A, para ver si el seek de B lo movió.
    let la2 = leer_en(a, 10, 5).unwrap_or_default();
    anotar(
        &mut casos,
        Caso::nuevo(
            "compartir/offsets-independientes",
            "00000 | 33333 | 11111",
            format!(
                "{} | {} | {}",
                String::from_utf8_lossy(&la),
                String::from_utf8_lossy(&lb),
                String::from_utf8_lossy(&la2)
            ),
        ),
    );

    // 2. **La pregunta de SQLite.** Un tercer descriptor escribe y sincroniza;
    //    ¿lo ve el que ya estaba abierto? Si cada `open` se lleva su copia del
    //    contenido, no lo verá — y dos conexiones a la misma base trabajarían
    //    sobre datos distintos sin enterarse.
    let escritor = sys::open(&ruta, abi::O_WRONLY);
    let observado = if escritor < 0 {
        format!("open(W) = {}", errno(escritor))
    } else {
        let e = escritor as u64;
        sys::seek(e, 0, abi::SEEK_SET);
        let w = sys::write(e, b"XXXXX");
        sys::fsync(e);
        sys::close(e);
        if w < 0 {
            format!("write = {}", errno(w))
        } else {
            let visto = leer_en(a, 0, 5).unwrap_or_default();
            String::from_utf8_lossy(&visto).into_owned()
        }
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "compartir/visibilidad-entre-descriptores",
            "XXXXX",
            observado,
        ),
    );
    sys::close(a);
    sys::close(b);

    // Y en disco, tras cerrar todo: ¿quedó la escritura **y el resto**?
    //
    // Comparar sólo los cinco primeros bytes dejaba pasar un fichero
    // destrozado: el prefijo puede ser el correcto y el resto haber
    // desaparecido. Se informa la longitud y la cola.
    let en_disco = leer_todo(&ruta).unwrap_or_default();
    let mut esperado_completo = base.clone();
    esperado_completo[..5].copy_from_slice(b"XXXXX");
    anotar(
        &mut casos,
        Caso::nuevo(
            "compartir/escritura-conserva-el-resto",
            format!(
                "{} bytes, empieza {} y acaba {}",
                esperado_completo.len(),
                String::from_utf8_lossy(&esperado_completo[..5]),
                String::from_utf8_lossy(&esperado_completo[esperado_completo.len() - 5..])
            ),
            format!(
                "{} bytes, empieza {} y acaba {}",
                en_disco.len(),
                String::from_utf8_lossy(&en_disco[..5.min(en_disco.len())]),
                String::from_utf8_lossy(&en_disco[en_disco.len().saturating_sub(5)..])
            ),
        ),
    );

    // 3. Dos escritores a la vez sobre zonas **distintas**: si cada descriptor
    //    tiene su copia entera del fichero, el segundo en cerrar borra lo del
    //    primero aunque no se solapen. Es la corrupción silenciosa que hace
    //    falta conocer antes de poner una base de datos encima.
    let ruta2 = unir(DIR, "dos.dat");
    let _ = escribir_nuevo(&ruta2, &base);
    let w1 = sys::open(&ruta2, abi::O_WRONLY);
    let w2 = sys::open(&ruta2, abi::O_WRONLY);
    let observado = if w1 < 0 || w2 < 0 {
        format!("w1={} w2={}", errno(w1), errno(w2))
    } else {
        let (w1, w2) = (w1 as u64, w2 as u64);
        sys::seek(w1, 0, abi::SEEK_SET);
        sys::write(w1, b"AAAAA");
        sys::seek(w2, 30, abi::SEEK_SET);
        sys::write(w2, b"BBBBB");
        sys::fsync(w1);
        sys::close(w1);
        sys::fsync(w2);
        sys::close(w2);
        let v = leer_todo(&ruta2).unwrap_or_default();
        let ini = String::from_utf8_lossy(&v[..5.min(v.len())]).into_owned();
        let med = if v.len() >= 35 {
            String::from_utf8_lossy(&v[30..35]).into_owned()
        } else {
            format!("no llega (sólo {} bytes)", v.len())
        };
        format!("{} bytes: {ini} y {med}", v.len())
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "compartir/dos-escritores-zonas-distintas",
            format!("{} bytes: AAAAA y BBBBB", base.len()),
            observado,
        ),
    );

    // 4. `pwrite` sobre un fichero: escribe en el offset y, según POSIX, **no**
    //    mueve la posición del descriptor. Lo segundo es semántica que el
    //    nombre no garantiza.
    let ruta3 = unir(DIR, "pw.dat");
    let _ = escribir_nuevo(&ruta3, &base);
    let fd = sys::open(&ruta3, abi::O_WRONLY);
    if fd < 0 {
        anotar(
            &mut casos,
            Caso::nuevo("compartir/pwrite", "PPPPP", format!("open = {}", errno(fd))),
        );
    } else {
        let fd = fd as u64;
        sys::seek(fd, 7, abi::SEEK_SET);
        let rc = sys::pwrite(fd, b"PPPPP", 20);
        let pos_tras = sys::seek(fd, 0, abi::SEEK_CUR);
        sys::fsync(fd);
        sys::close(fd);
        let v = leer_todo(&ruta3).unwrap_or_default();
        let en20 = if v.len() >= 25 {
            String::from_utf8_lossy(&v[20..25]).into_owned()
        } else {
            format!("corto ({} bytes)", v.len())
        };
        anotar(
            &mut casos,
            Caso::nuevo(
                "compartir/pwrite-en-offset",
                "PPPPP",
                if rc < 0 {
                    format!("pwrite = {}", errno(rc))
                } else {
                    en20
                },
            ),
        );
        anotar(
            &mut casos,
            Caso::nuevo(
                "compartir/pwrite-no-mueve-offset",
                "7",
                format!("{pos_tras}"),
            ),
        );
    }

    // 5. Exclusión mutua. No hay `flock` ni `fcntl`, así que lo único que
    //    queda es `O_EXCL` — el mismo primitivo sobre el que T46 construyó las
    //    referencias durables. Aquí se comprueba que de verdad excluye.
    let ruta4 = unir(DIR, "lock");
    let p1 = sys::open(&ruta4, abi::O_WRONLY | abi::O_CREAT | abi::O_EXCL);
    let p2 = sys::open(&ruta4, abi::O_WRONLY | abi::O_CREAT | abi::O_EXCL);
    if p1 >= 0 {
        sys::close(p1 as u64);
    }
    if p2 >= 0 {
        sys::close(p2 as u64);
    }
    anotar(
        &mut casos,
        Caso::nuevo(
            "compartir/o-excl-excluye",
            "gana uno",
            if p1 >= 0 && p2 == -(abi::EEXIST as i64) {
                String::from("gana uno")
            } else {
                format!("p1={} p2={}", errno(p1), errno(p2))
            },
        ),
    );

    casos
}
