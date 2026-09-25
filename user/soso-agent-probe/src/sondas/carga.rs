//! Sonda 9 — carga de código ajeno: el experimento de
//! [N-005](../../../../docs/self-improvement/native/N-005.md).
//!
//! La ficha es de **experimento con resultado binario**: ¿se puede enlazar
//! estáticamente todo lo nativo que un runtime necesita, o hace falta carga
//! dinámica? Lo que la responde no es una opinión sobre `dlopen` sino dos
//! medidas que van en direcciones contrarias:
//!
//! - **Lo que sí**: código C escrito para la ocasión, compilado hacia el
//!   target de soso y enlazado en este binario, se ejecuta. `soso-http` ya
//!   enlaza el C de `ring`, pero eso llegó con una dependencia; aquí se mide
//!   la capacidad, no la herencia.
//! - **Lo que no**: un ELF que no sea `ET_EXEC` **no arranca**. Y se mide con
//!   un control de **un byte**: el mismo binario que acaba de arrancar,
//!   copiado, con `e_type` cambiado de 2 a 3. Si el que no arranca fallara por
//!   cualquier otra razón —la ruta, los permisos, el tamaño—, el control
//!   habría fallado también.
//!
//! El control es la mitad que suele faltar. Sin él, «el ET_DYN no arrancó» es
//! compatible con «nada de /tmp arranca», que es una conclusión distinta.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{abi, println, sys};

use crate::{errno, unir, Caso};

const DIR: &str = "/tmp";
const YO: &str = "/bin/soso-agent-probe";
/// `e_type` vive en el byte 16 del ELF: 2 = `ET_EXEC`, 3 = `ET_DYN`.
const OFF_E_TYPE: usize = 16;
const ET_EXEC: u8 = 2;
const ET_DYN: u8 = 3;
/// Con qué código sale el hijo de control. Un valor que no es 0 ni 1, para que
/// no se confunda con «salió bien» ni con «falló».
const CODIGO_CONTROL: i64 = 7;

unsafe extern "C" {
    /// FNV-1a de 64 bits, compilada desde `c/ajeno.c`.
    fn soso_probe_mezcla(datos: *const u8, n: usize) -> u64;
    /// Copia `src` al revés en `dst`, pasando por el `memcpy` de
    /// `compiler-builtins-mem`.
    fn soso_probe_invertir(dst: *mut u8, src: *const u8, n: usize);
}

fn anotar(casos: &mut Vec<Caso>, c: Caso) {
    // Una observación —`Caso::observacion`, sin `esperado`— **no es un
    // aprobado**: imprimirla como `ok` la disfraza de veredicto. Se distingue
    // aquí aunque esta sonda no tenga ninguna todavía, porque la trampa la
    // paga quien añada la primera.
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
    casos.push(c);
}

/// La misma mezcla, en Rust. No se compara contra una constante escrita a
/// mano: una constante equivocada haría fallar el caso por culpa de la
/// comprobación, y una constante copiada de la primera ejecución haría que el
/// caso pasara siempre.
fn mezcla_ref(datos: &[u8]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for b in datos {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

/// Lee un fichero entero.
fn leer(ruta: &str) -> Option<Vec<u8>> {
    let fd = sys::open(ruta, abi::O_RDONLY);
    if fd < 0 {
        return None;
    }
    let fd = fd as u64;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sys::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    sys::close(fd);
    Some(out)
}

/// Escribe un fichero entero, creándolo.
fn escribir(ruta: &str, datos: &[u8]) -> Result<(), String> {
    sys::unlink(ruta);
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
    sys::close(fd);
    Ok(())
}

/// Lanza `ruta` para que salga con [`CODIGO_CONTROL`] y devuelve qué pasó.
fn arrancar(ruta: &str) -> String {
    let codigo = format!("{CODIGO_CONTROL}");
    let rc = sys::spawn_io_full(
        ruta,
        &[ruta, "salir", &codigo],
        &[],
        [
            abi::FD_SERIAL_TTY,
            abi::FD_SERIAL_TTY,
            abi::FD_SERIAL_TTY,
            abi::FD_KERNEL_LOG,
        ],
    );
    if rc < 0 {
        return format!("no arrancó: {}", errno(rc));
    }
    match sys::wait() {
        Ok((_, c)) => format!("arrancó y salió con {c}"),
        Err(e) => format!("arrancó; wait = {}", errno(e)),
    }
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();

    // 1. **El C ajeno se ejecuta.** La entrada no es constante y el resultado
    //    depende de cada byte: una función que no se hubiera compilado no
    //    podría acertar por casualidad.
    let datos: Vec<u8> = (0..64u32).map(|i| (i * 37 + 11) as u8).collect();
    let esperado = mezcla_ref(&datos);
    let visto = unsafe { soso_probe_mezcla(datos.as_ptr(), datos.len()) };
    anotar(
        &mut casos,
        Caso::nuevo(
            "carga/c-ajeno-compilado-y-enlazado",
            format!("{esperado:x}"),
            format!("{visto:x}"),
        ),
    );

    // 2. Y **escribe** en memoria del llamante, pasando por el `memcpy` que
    //    pone `compiler-builtins-mem`. Es el otro de los dos únicos símbolos
    //    de libc que el C de `ring` necesita, así que el caso mide justo la
    //    superficie que N-006 tendría que ampliar.
    let mut al_reves = alloc::vec![0u8; datos.len()];
    unsafe { soso_probe_invertir(al_reves.as_mut_ptr(), datos.as_ptr(), datos.len()) };
    let bien: Vec<u8> = datos.iter().rev().copied().collect();
    anotar(
        &mut casos,
        Caso::nuevo(
            "carga/c-ajeno-escribe-en-memoria-del-llamante",
            format!("{:x?}", &bien[..8]),
            format!("{:x?}", &al_reves[..8]),
        ),
    );

    // 3. El **control**: una copia byte a byte de este mismo binario arranca.
    //    Va antes que el caso del ET_DYN a propósito: si esto fallara, el
    //    siguiente no mediría lo que dice medir.
    let Some(mio) = leer(YO) else {
        anotar(
            &mut casos,
            Caso::nuevo("carga/copia-intacta-arranca", "arrancó", format!("no pude leer {YO}")),
        );
        return casos;
    };
    let copia = unir(DIR, "n005-exec");
    let dyn_ = unir(DIR, "n005-dyn");

    let observado = match escribir(&copia, &mio) {
        Ok(()) => arrancar(&copia),
        Err(e) => e,
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "carga/copia-intacta-arranca",
            format!("arrancó y salió con {CODIGO_CONTROL}"),
            observado,
        ),
    );

    // 4. La misma copia con **un byte** cambiado: `ET_EXEC` → `ET_DYN`. Lo que
    //    se mide es que el cargador lo rechace, no que se caiga: un ELF que
    //    arrancara a medias sería peor que uno que no arranca.
    let mut torcido = mio;
    let observado = if torcido.len() <= OFF_E_TYPE || torcido[OFF_E_TYPE] != ET_EXEC {
        format!(
            "el original no era ET_EXEC: byte {OFF_E_TYPE} = {:?}",
            torcido.get(OFF_E_TYPE)
        )
    } else {
        torcido[OFF_E_TYPE] = ET_DYN;
        match escribir(&dyn_, &torcido) {
            Ok(()) => arrancar(&dyn_),
            Err(e) => e,
        }
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "carga/un-et-dyn-no-arranca",
            "no arrancó: argumento inválido (-22)",
            observado,
        ),
    );

    sys::unlink(&copia);
    sys::unlink(&dyn_);
    casos
}
