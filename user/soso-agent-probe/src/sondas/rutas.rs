//! Sonda 12 — que normalizar rutas siga significando lo mismo.
//!
//! [N-012](../../../../docs/self-improvement/native/N-012.md) reescribió
//! `abs_path` para que haga **una** reserva en vez de cinco. Es el camino por
//! el que pasa **toda** llamada al sistema que lleva una ruta, y no hay forma
//! de probarlo en el host: el kernel no se compila para él.
//!
//! Así que se prueba donde corre, y se prueba lo que de verdad importa: que
//! `..` **no deje salirse de la raíz**. Lo demás son comodidades; eso es
//! contención, y una reescritura silenciosa de esa parte sería la clase de
//! fallo que nadie ve hasta que alguien lo usa.
//!
//! Cada caso abre el **mismo fichero** por una escritura distinta de su ruta y
//! compara el contenido. Comparar que «abrió» no bastaría: abrir otro fichero
//! también abre.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{abi, println, sys};

use crate::{errno, unir, Caso};

const DIR: &str = "/var/self-improvement/probe/rutas";
const MARCA: &[u8] = b"RUTAS-MARCA-42\n";

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

fn leer(ruta: &str) -> Result<Vec<u8>, i64> {
    let fd = sys::open(ruta, abi::O_RDONLY);
    if fd < 0 {
        return Err(fd);
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

/// Abre `ruta` y dice si tenía la marca. El veredicto va en la misma forma que
/// lo esperado; el detalle, al mensaje.
fn ve_la_marca(ruta: &str) -> String {
    match leer(ruta) {
        Ok(d) if d == MARCA => String::from("la marca"),
        Ok(d) => format!("otra cosa ({} bytes)", d.len()),
        Err(e) => format!("no abrió: {}", errno(e)),
    }
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();
    sys::mkdir("/var/self-improvement");
    sys::mkdir("/var/self-improvement/probe");
    sys::mkdir(DIR);
    sys::mkdir(&unir(DIR, "hondo"));

    let destino = unir(&unir(DIR, "hondo"), "marca.txt");
    sys::unlink(&destino);
    let fd = sys::open(&destino, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
    if fd < 0 {
        anotar(&mut casos, Caso::nuevo("rutas/preparar", "un fichero", errno(fd)));
        return casos;
    }
    let mut n = 0;
    while n < MARCA.len() {
        let w = sys::write(fd as u64, &MARCA[n..]);
        if w <= 0 {
            break;
        }
        n += w as usize;
    }
    sys::close(fd as u64);

    // 0. El control: por su nombre tal cual. Si esto fallara, ningún otro caso
    //    significaría nada.
    anotar(
        &mut casos,
        Caso::nuevo("rutas/el-fichero-esta-donde-se-dejo", "la marca", ve_la_marca(&destino)),
    );

    // 1. `.` y las barras de más no cambian a dónde apunta.
    let con_punto = format!("{DIR}/./hondo/marca.txt");
    anotar(
        &mut casos,
        Caso::nuevo("rutas/el-punto-no-cambia-nada", "la marca", ve_la_marca(&con_punto)),
    );
    let con_dobles = format!("{DIR}//hondo//marca.txt");
    anotar(
        &mut casos,
        Caso::nuevo("rutas/las-barras-de-mas-no-cuentan", "la marca", ve_la_marca(&con_dobles)),
    );

    // 2. `..` sube un nivel de verdad.
    let subiendo = format!("{DIR}/hondo/../hondo/marca.txt");
    anotar(
        &mut casos,
        Caso::nuevo("rutas/punto-punto-sube-un-nivel", "la marca", ve_la_marca(&subiendo)),
    );

    // 3. **El que importa**: subir desde la raíz no saca de la raíz. Se
    //    escriben más `..` que componentes tiene la ruta a propósito: si
    //    alguno se saliera, lo que quedaría no sería esta ruta.
    let desde_arriba = format!("/../../../..{DIR}/hondo/marca.txt");
    anotar(
        &mut casos,
        Caso::nuevo(
            "rutas/subir-desde-la-raiz-no-escapa",
            "la marca",
            ve_la_marca(&desde_arriba),
        ),
    );

    // 4. Una ruta relativa se resuelve contra el cwd **del proceso**.
    let antes = {
        let mut b = [0u8; 256];
        if sys::getcwd(&mut b) < 0 {
            String::from("/")
        } else {
            let fin = b.iter().position(|c| *c == 0).unwrap_or(b.len());
            String::from_utf8_lossy(&b[..fin]).into_owned()
        }
    };
    sys::chdir(&unir(DIR, "hondo"));
    anotar(
        &mut casos,
        Caso::nuevo("rutas/relativa-usa-el-cwd", "la marca", ve_la_marca("marca.txt")),
    );
    anotar(
        &mut casos,
        Caso::nuevo("rutas/relativa-con-punto-punto", "la marca", ve_la_marca("../hondo/marca.txt")),
    );
    sys::chdir(&antes);

    // 5. Y el tope de longitud sigue rechazando. La ruta se pasa de 256 por
    //    mucho, para que no dependa de un byte.
    let mut larga = String::from("/");
    while larga.len() < 400 {
        larga.push_str("xxxxxxxx/");
    }
    larga.push_str("marca.txt");
    let rc = sys::open(&larga, abi::O_RDONLY);
    if rc >= 0 {
        sys::close(rc as u64);
    }
    anotar(
        &mut casos,
        Caso::nuevo(
            "rutas/una-ruta-demasiado-larga-se-rechaza",
            "nombre demasiado largo (-36)",
            if rc < 0 { errno(rc) } else { format!("abrió igual (fd {rc})") },
        ),
    );

    casos
}
