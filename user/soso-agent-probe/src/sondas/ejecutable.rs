//! Sonda 2 — páginas ejecutables y W^X (pase 2 de T33, reescrita el 2026-11-01).
//!
//! El pase 2 midió un soso sin `PROT_EXEC`, donde todo era ejecutable (W+X de
//! facto). Desde el 2026-10-30 las páginas de datos son NX y existe `PROT_EXEC`
//! con política W^X: `mprotect` rechaza WRITE|EXEC a la vez. Lo que un JIT
//! necesita ahora es el baile W^X, y eso es lo que se mide: escribir con RW,
//! pasar a R|X, ejecutar, volver a RW, reescribir y volver a R|X.
//!
//! Es **la decisiva** del inventario de
//! [T32](../../../../docs/self-improvement/T32-opencode-inventario.md): Bun
//! trae JavaScriptCore, JavaScriptCore compila a código máquina en tiempo de
//! ejecución, y para eso hace falta escribir en una página y después
//! ejecutarla. Si esto no se puede, el resto de aquel inventario es teórico.
//!
//! **Lo que se mide es ejecutar, no pedir permiso.** Leer en el kernel que las
//! páginas de usuario se mapean sin el bit NX diría que son ejecutables; eso
//! es leer una constante, no una medida. Aquí se escriben unos bytes de código
//! máquina en una página de datos y **se llama**. Si la página no fuera
//! ejecutable, el proceso moriría, y esa muerte también es un resultado: por
//! eso cada caso se imprime **en cuanto se decide**, antes de arriesgarse al
//! siguiente. Un informe que sólo se imprime al final no sobrevive a la sonda
//! que lo mata.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{abi, println, sys};

use crate::{errno, Caso};

/// `mov eax, edi; add eax, 1; ret` — devuelve su argumento más uno.
///
/// Se usa un argumento a propósito: una función que devolviera una constante
/// podría «pasar» si la llamada no llegara a ocurrir y el registro trajera
/// basura con el valor esperado. Con `arg + 1` el resultado depende de haber
/// ejecutado de verdad, y se prueba con dos argumentos distintos.
const CODIGO: &[u8] = &[0x89, 0xf8, 0x83, 0xc0, 0x01, 0xc3];

const PAGINA: u64 = 4096;

/// Imprime el caso en el momento y lo guarda. Si lo siguiente mata al proceso,
/// esto ya está en la consola.
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

/// Llama al código escrito en `addr` con `arg`.
///
/// # Seguridad
/// El llamante tiene que haber escrito [`CODIGO`] ahí. Si la página no es
/// ejecutable, esto no vuelve: el proceso muere con una falta de página, que
/// es precisamente el desenlace que la sonda quiere distinguir.
unsafe fn llamar(addr: u64, arg: u32) -> u32 {
    let f: extern "C" fn(u32) -> u32 = unsafe { core::mem::transmute(addr as usize) };
    f(arg)
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();
    let rx = abi::PROT_READ | abi::PROT_EXEC;
    let rw = abi::PROT_READ | abi::PROT_WRITE;

    // 1. Una página anónima, que nace RW y no ejecutable.
    let region = sys::mmap(0, PAGINA, u64::MAX, 0);
    if region < 0 {
        anotar(
            &mut casos,
            Caso::nuevo("exec/mmap", "página anónima", format!("mmap = {}", errno(region))),
        );
        return casos;
    }
    let addr = region as u64;
    anotar(
        &mut casos,
        Caso::nuevo("exec/mmap", "página anónima", "página anónima"),
    );

    // 2. Se escribe el código con la página RW, **antes** de pedir el bit de
    //    ejecución: es el orden de un JIT con W^X.
    unsafe {
        core::ptr::copy_nonoverlapping(CODIGO.as_ptr(), addr as *mut u8, CODIGO.len());
    }

    // 3. W+X a la vez se rechaza: es la política, no una carencia. Se mide
    //    pidiéndolo, y la página sigue siendo RW (no se ejecuta nada todavía).
    let rc = sys::mprotect(addr, PAGINA, rw | abi::PROT_EXEC);
    anotar(
        &mut casos,
        Caso::nuevo(
            "exec/w-mas-x",
            "EINVAL: W^X rechaza WRITE|EXEC",
            if rc == -(abi::EINVAL as i64) {
                String::from("EINVAL: W^X rechaza WRITE|EXEC")
            } else {
                format!("mprotect(READ|WRITE|EXEC) = {}", errno(rc))
            },
        ),
    );

    // 4. Pasar a R|X y llamar. **Sólo se llama si el cambio de permisos
    //    funcionó**: sin él la página es NX y el proceso moriría, y eso ya no
    //    es un dato, es perder el resto del informe.
    let rc = sys::mprotect(addr, PAGINA, rx);
    anotar(
        &mut casos,
        Caso::nuevo(
            "exec/prot-exec-existe",
            "mprotect(READ|EXEC) acepta",
            if rc == 0 {
                String::from("mprotect(READ|EXEC) acepta")
            } else {
                format!("mprotect(READ|EXEC) = {}", errno(rc))
            },
        ),
    );
    if rc < 0 {
        sys::munmap(addr, PAGINA);
        return casos;
    }
    println!("probe: exec/rx-ejecuta llamando con la página en R|X");
    let r1 = unsafe { llamar(addr, 41) };
    let r2 = unsafe { llamar(addr, 100) };
    anotar(
        &mut casos,
        Caso::nuevo("exec/rx-ejecuta", "42 y 101", format!("{r1} y {r2}")),
    );

    // 5. Volver a RW, reescribir con otro código y volver a R|X: lo que un JIT
    //    hace para recompilar. `mov eax, edi; add eax, 10; ret`.
    let rc = sys::mprotect(addr, PAGINA, rw);
    if rc < 0 {
        anotar(
            &mut casos,
            Caso::nuevo(
                "exec/volver-a-escribir",
                "51",
                format!("mprotect(READ|WRITE) = {}", errno(rc)),
            ),
        );
    } else {
        let otro: &[u8] = &[0x89, 0xf8, 0x83, 0xc0, 0x0a, 0xc3];
        unsafe {
            core::ptr::copy_nonoverlapping(otro.as_ptr(), addr as *mut u8, otro.len());
        }
        let rc = sys::mprotect(addr, PAGINA, rx);
        if rc < 0 {
            anotar(
                &mut casos,
                Caso::nuevo(
                    "exec/volver-a-escribir",
                    "51",
                    format!("mprotect(READ|EXEC) = {}", errno(rc)),
                ),
            );
        } else {
            let r = unsafe { llamar(addr, 41) };
            anotar(
                &mut casos,
                Caso::nuevo("exec/volver-a-escribir", "51", format!("{r}")),
            );
        }
    }
    sys::munmap(addr, PAGINA);

    // 6. Una página **del montón**: un JIT que use el asignador normal también
    //    tiene que poder ejecutar lo que escriba. Se reserva de más para tener
    //    una página entera alineada dentro del bloque —cambiar permisos de una
    //    página compartida con otras reservas rompería al asignador— y se deja
    //    RW otra vez antes de soltarla.
    let mut buf = alloc::vec![0u8; 3 * PAGINA as usize];
    let base = buf.as_mut_ptr() as u64;
    let pagina = (base + PAGINA - 1) & !(PAGINA - 1);
    unsafe {
        core::ptr::copy_nonoverlapping(CODIGO.as_ptr(), pagina as *mut u8, CODIGO.len());
    }
    let rc = sys::mprotect(pagina, PAGINA, rx);
    if rc < 0 {
        anotar(
            &mut casos,
            Caso::nuevo("exec/monton", "8", format!("mprotect(READ|EXEC) = {}", errno(rc))),
        );
    } else {
        println!("probe: exec/montón llamando código escrito en el heap");
        let r = unsafe { llamar(pagina, 7) };
        anotar(&mut casos, Caso::nuevo("exec/monton", "8", format!("{r}")));
        let _ = sys::mprotect(pagina, PAGINA, rw);
    }
    drop(buf);
    casos
}
