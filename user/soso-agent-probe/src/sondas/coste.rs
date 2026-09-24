//! Medida del coste de escritura en sosofs — paso 1 de
//! [N-001](../../../../docs/self-improvement/native/N-001.md).
//!
//! N-001 propone cambiar el modelo de ficheros de «por descriptor» a «por
//! inodo», y ofrece dos formas. La segunda —que cada `write` vaya al VFS en
//! vez de acumularse en un búfer del descriptor— es la simple, y su ficha dice
//! que **hay que medir su coste antes de elegirla**, no suponerlo.
//!
//! Esto es esa medida. Hoy, `Fd::StreamWrite` acumula y vuelca **una vez** al
//! cerrar, así que escribir 64 KiB cuesta un `append_file`. Con la forma 2,
//! escribirlos en trozos de 1 KiB costaría 64. Cada `append_file` reescribe el
//! último extent y **cierra una transacción CoW** (`crates/sosofs/src/write.rs`,
//! `append_file_inner` + `finish`), así que la diferencia no es el número de
//! llamadas sino el número de transacciones.
//!
//! **Se mide con `rdtsc`, no con `uptime_ms`.** El único reloj de milisegundos
//! es el PIT y el PIT **subcuenta durante el polling de disco**: medir con él
//! justo un camino de E/S daría de menos por la razón equivocada. `ciclos()`
//! existe en `libsoso` por esto mismo.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{abi, ciclos, println, sys};

use crate::{errno, limpiar_dir, unir, Caso};

const DIR: &str = "/var/self-improvement/probe/coste";
const TOTAL: usize = 64 * 1024;
const TROZO: usize = 1024;

fn escribir_una_vez(ruta: &str, datos: &[u8]) -> Result<u64, String> {
    let t0 = ciclos();
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
    Ok(ciclos() - t0)
}

/// Un `open`/`write`/`close` **por trozo**: cada cierre cierra su transacción,
/// que es lo que costaría la forma 2 de N-001.
///
/// Ojo con lo que esto **no** hace: no construye un fichero de 64 KiB. Cada
/// ciclo **reemplaza** el contenido en vez de añadirlo, que es el truncamiento
/// que T33 midió en su pase 3 y que N-001 viene a arreglar. Lo que se mide
/// aquí es el coste de **64 transacciones frente a una**, no el de dos formas
/// de construir el mismo fichero.
fn escribir_por_trozos(ruta: &str, datos: &[u8]) -> Result<(u64, usize), String> {
    sys::unlink(ruta);
    let t0 = ciclos();
    let mut hechos = 0;
    let mut off = 0;
    while off < datos.len() {
        let fin = (off + TROZO).min(datos.len());
        let fd = sys::open(ruta, abi::O_WRONLY | abi::O_CREAT);
        if fd < 0 {
            return Err(format!("open {hechos} = {}", errno(fd)));
        }
        let fd = fd as u64;
        // Sin `seek`: abrir para escribir en /var deja la posición al final,
        // que es justo el añadido que se quiere medir.
        let mut n = off;
        while n < fin {
            let w = sys::write(fd, &datos[n..fin]);
            if w <= 0 {
                sys::close(fd);
                return Err(format!("write {hechos} = {}", errno(w)));
            }
            n += w as usize;
        }
        sys::fsync(fd);
        sys::close(fd);
        hechos += 1;
        off = fin;
    }
    Ok((ciclos() - t0, hechos))
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();
    sys::mkdir("/var/self-improvement");
    sys::mkdir("/var/self-improvement/probe");
    limpiar_dir(DIR);

    let datos: Vec<u8> = (0..TOTAL).map(|i| (i % 251) as u8).collect();

    let una = escribir_una_vez(&unir(DIR, "una.dat"), &datos);
    let muchas = escribir_por_trozos(&unir(DIR, "muchas.dat"), &datos);

    match (una, muchas) {
        (Err(e), _) | (_, Err(e)) => {
            println!("coste: no se pudo medir: {e}");
            casos.push(Caso::nuevo("coste/medida", "dos tiempos", e));
        }
        (Ok(c1), Ok((c2, n))) => {
            // La razón importa más que los ciclos absolutos: el TSC no se
            // convierte a segundos sin conocer la frecuencia, y para decidir
            // entre dos formas lo que hace falta es cuántas veces más cuesta
            // una que otra.
            let razon = if c1 == 0 { 0 } else { c2 / c1.max(1) };
            println!(
                "coste: {TOTAL} bytes de una vez = {c1} ciclos; en {n} trozos de {TROZO} = {c2} ciclos"
            );
            println!("coste: la forma por transacción cuesta ~{razon}× más");
            // Este caso **no juzga**: informa. Poner un umbral aquí sería
            // inventarme el criterio que N-001 tiene que decidir con el dato
            // delante.
            casos.push(Caso::observacion(
                "coste/transacciones",
                format!("1 transacción {c1} ciclos, {n} transacciones {c2} ciclos, razón ~{razon}x"),
            ));

            // Y el tamaño que quedó por cada camino, que dice **qué** se midió.
            // El de trozos no acaba en 64 KiB: cada ciclo reemplaza en vez de
            // añadir. Es el mismo truncamiento del pase 3 de T33, y verlo aquí
            // otra vez por un camino distinto es una confirmación, no un
            // estorbo.
            let a = leer(&unir(DIR, "una.dat")).len();
            let b = leer(&unir(DIR, "muchas.dat")).len();
            println!("coste: quedaron {a} bytes por el camino de una transacción y {b} por el de {n}");
            casos.push(Caso::nuevo(
                "coste/el-camino-por-trozos-reemplaza",
                format!("{TOTAL} y {TROZO}"),
                format!("{a} y {b}"),
            ));
        }
    }

    for c in &casos {
        if c.paso && c.esperado.is_empty() {
            println!("probe: {} medido: {}", c.id, c.observado);
        } else if c.paso {
            println!("probe: {} ok ({})", c.id, c.observado);
        } else {
            println!(
                "probe: {} FALLO esperado={:?} observado={:?}",
                c.id, c.esperado, c.observado
            );
        }
    }
    casos
}

fn leer(ruta: &str) -> Vec<u8> {
    let fd = sys::open(ruta, abi::O_RDONLY);
    if fd < 0 {
        return Vec::new();
    }
    let fd = fd as u64;
    let mut out = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = sys::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    sys::close(fd);
    out
}
