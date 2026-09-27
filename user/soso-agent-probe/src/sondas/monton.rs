//! Sonda del montón (T72): asignación, realloc y liberación con libsoso.
//!
//! El bloque pequeño se acredita por la dirección: dos reservas, se libera
//! la primera y la siguiente tiene que ser la misma. El arena sólo rebobina
//! el último bloque, así que esa igualdad sale de la lista. Un bloque grande
//! no mueve `bytes_en_lista` (va a `mmap`) y se puede repetir al liberarlo.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::Layout;

use libsoso::println;
use soso_alloc::{self, MMAP_ALLOC_MIN};

use crate::Caso;

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

fn alinear(align: usize) -> Caso {
    let layout = Layout::from_size_align(32, align).unwrap();
    let p = unsafe { alloc::alloc::alloc(layout) };
    let observado = if p.is_null() {
        String::from("nulo")
    } else {
        let resto = (p as usize) % align;
        unsafe { alloc::alloc::dealloc(p, layout) };
        format!("{resto}")
    };
    Caso::nuevo(&format!("monton-align-{align}"), "0", observado)
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos = Vec::new();

    for align in [16usize, 64, 256, 4096] {
        anotar(&mut casos, alinear(align));
    }

    let layout = Layout::from_size_align(32, 16).unwrap();
    let p = unsafe { alloc::alloc::alloc_zeroed(layout) };
    let ceros = if p.is_null() {
        String::from("nulo")
    } else {
        let ok = unsafe { core::slice::from_raw_parts(p, 32) }
            .iter()
            .all(|&b| b == 0);
        if ok {
            for i in 0..32 {
                unsafe { p.add(i).write(i as u8) };
            }
        }
        let q = unsafe { alloc::alloc::realloc(p, layout, 200) };
        let conserva = !q.is_null()
            && ok
            && (0..32).all(|i| unsafe { q.add(i).read() } == i as u8);
        if !q.is_null() {
            let nuevo = Layout::from_size_align(200, 16).unwrap();
            unsafe { alloc::alloc::dealloc(q, nuevo) };
        }
        if conserva {
            String::from("ceros y prefijo")
        } else {
            String::from("no")
        }
    };
    anotar(
        &mut casos,
        Caso::nuevo("monton-zeroed-realloc", "ceros y prefijo", ceros),
    );

    // Dos bloques vivos: el primero deja de ser «el último» del arena, así que
    // sólo la lista puede devolver su dirección al volver a pedirlo.
    let a_lay = Layout::from_size_align(128, 16).unwrap();
    let b_lay = Layout::from_size_align(64, 16).unwrap();
    let a = unsafe { alloc::alloc::alloc(a_lay) };
    let b = unsafe { alloc::alloc::alloc(b_lay) };
    if !a.is_null() {
        unsafe { alloc::alloc::dealloc(a, a_lay) };
    }
    let c = unsafe { alloc::alloc::alloc(a_lay) };
    let (ocupados, tam) = soso_alloc::estado_lista();
    let libre = if !a.is_null() && !b.is_null() && c == a {
        String::from("devuelto")
    } else {
        format!(
            "a={:?}/{} b={:?}/{} c={:?}/{} lista={ocupados}/{tam}",
            a,
            soso_alloc::en_lista(a),
            b,
            soso_alloc::en_lista(b),
            c,
            soso_alloc::en_lista(c),
        )
    };
    if !c.is_null() {
        unsafe { alloc::alloc::dealloc(c, a_lay) };
    }
    if !b.is_null() {
        unsafe { alloc::alloc::dealloc(b, b_lay) };
    }
    anotar(
        &mut casos,
        Caso::nuevo("monton-liberar-pequeno", "devuelto", libre),
    );

    let grande = Layout::from_size_align(MMAP_ALLOC_MIN, 16).unwrap();
    let lista = soso_alloc::bytes_en_lista();
    let g = unsafe { alloc::alloc::alloc(grande) };
    let en_lista = soso_alloc::bytes_en_lista() == lista;
    if !g.is_null() {
        unsafe {
            g.write(0x11);
            g.add(MMAP_ALLOC_MIN - 1).write(0x22);
            alloc::alloc::dealloc(g, grande);
        }
    }
    let g2 = unsafe { alloc::alloc::alloc(grande) };
    let repetible = !g.is_null() && !g2.is_null() && en_lista;
    if !g2.is_null() {
        unsafe { alloc::alloc::dealloc(g2, grande) };
    }
    anotar(
        &mut casos,
        Caso::nuevo(
            "monton-liberar-grande",
            "mmap repetible",
            if repetible {
                String::from("mmap repetible")
            } else {
                format!("primero={} segundo={} lista={en_lista}", !g.is_null(), !g2.is_null())
            },
        ),
    );

    casos
}
