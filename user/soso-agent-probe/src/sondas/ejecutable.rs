//! Sonda 2 — páginas ejecutables y W+X (pase 2 de T33).
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

    // 1. ¿Existe siquiera PROT_EXEC? No: `mprotect` acepta READ y WRITE y
    //    rechaza cualquier otro bit con EINVAL (kernel/src/task/syscall.rs).
    //    Se mide en vez de leerlo: se pide el bit 4 y se mira qué contesta.
    const PROT_EXEC_SUPUESTO: u64 = 4;
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

    let rc = sys::mprotect(addr, PAGINA, abi::PROT_READ | PROT_EXEC_SUPUESTO);
    anotar(
        &mut casos,
        Caso::nuevo(
            "exec/prot-exec-existe",
            "EINVAL: no hay PROT_EXEC",
            if rc == -(abi::EINVAL as i64) {
                String::from("EINVAL: no hay PROT_EXEC")
            } else {
                format!("mprotect(READ|4) = {}", errno(rc))
            },
        ),
    );

    // 2. Escribir código máquina en una página **de datos** y llamarlo. Esto
    //    es W+X de verdad: la página sigue siendo escribible mientras se
    //    ejecuta, que es lo que un JIT sin W^X necesita.
    unsafe {
        core::ptr::copy_nonoverlapping(CODIGO.as_ptr(), addr as *mut u8, CODIGO.len());
    }
    println!("probe: exec/w-mas-x escribiendo y llamando (si muere aquí, no hay W+X)");
    let r1 = unsafe { llamar(addr, 41) };
    let r2 = unsafe { llamar(addr, 100) };
    anotar(
        &mut casos,
        Caso::nuevo(
            "exec/w-mas-x",
            "42 y 101",
            format!("{r1} y {r2}"),
        ),
    );

    // 3. El baile W^X que hace un JIT moderno: escribir con RW, pasar a sólo
    //    lectura y ejecutar. Aquí sólo se puede quitar la escritura, porque no
    //    hay bit de ejecución que poner; lo que se comprueba es que quitarla
    //    **no** deja la página inejecutable.
    let rc = sys::mprotect(addr, PAGINA, abi::PROT_READ);
    if rc < 0 {
        anotar(
            &mut casos,
            Caso::nuevo(
                "exec/tras-quitar-escritura",
                "43",
                format!("mprotect(READ) = {}", errno(rc)),
            ),
        );
    } else {
        println!("probe: exec/tras-quitar-escritura llamando con la página de sólo lectura");
        let r = unsafe { llamar(addr, 42) };
        anotar(
            &mut casos,
            Caso::nuevo("exec/tras-quitar-escritura", "43", format!("{r}")),
        );
    }

    // 4. Y volver a hacerla escribible, que es lo que un JIT necesita para
    //    recompilar. Si el camino de vuelta no funciona, se puede generar
    //    código una vez y nunca más.
    let rc = sys::mprotect(addr, PAGINA, abi::PROT_READ | abi::PROT_WRITE);
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
        // Reescribir con otro código: `mov eax, edi; add eax, 10; ret`.
        let otro: &[u8] = &[0x89, 0xf8, 0x83, 0xc0, 0x0a, 0xc3];
        unsafe {
            core::ptr::copy_nonoverlapping(otro.as_ptr(), addr as *mut u8, otro.len());
        }
        let r = unsafe { llamar(addr, 41) };
        anotar(
            &mut casos,
            Caso::nuevo("exec/volver-a-escribir", "51", format!("{r}")),
        );
    }

    // 5. Una página **del montón**, no de mmap: un JIT que use el asignador
    //    normal también tiene que poder ejecutar lo que escriba.
    let mut buf = alloc::vec![0u8; 64];
    // Alinear a 16 por costumbre; no hace falta para ejecutar, pero un
    // desalineado confundiría un fallo de permisos con uno de alineación.
    let base = buf.as_mut_ptr() as u64;
    let alineada = (base + 15) & !15;
    unsafe {
        core::ptr::copy_nonoverlapping(CODIGO.as_ptr(), alineada as *mut u8, CODIGO.len());
    }
    println!("probe: exec/montón llamando código escrito en el heap");
    let r = unsafe { llamar(alineada, 7) };
    anotar(&mut casos, Caso::nuevo("exec/monton", "8", format!("{r}")));

    sys::munmap(addr, PAGINA);
    casos
}
