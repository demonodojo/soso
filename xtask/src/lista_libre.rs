//! El page fault de placa es el `malloc` siguiente: `IterMut::next` carga
//! el `next` de un hueco libre. El test deja ese qword como en la foto
//! (`0x3250000000`) y llama a `malloc`. Si el proceso muere, el panic
//! sigue ahí.

use std::alloc::Layout;
use talc::{ErrOnOom, Span, Talc};

unsafe extern "C" {
    fn fork() -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
}

fn layout(size: usize) -> Layout {
    Layout::from_size_align(size, 8).unwrap()
}

/// Libera el bloque de menor dirección. Ahí queda el `LlistNode` cuyo
/// `next` leerá el `malloc` siguiente.
fn hueco_delante(talc: &mut Talc<ErrOnOom>) -> *mut u8 {
    let pedido = layout(64);
    let primero = unsafe { talc.malloc(pedido).unwrap() };
    let segundo = unsafe { talc.malloc(pedido).unwrap() };
    let antes = if primero.as_ptr() < segundo.as_ptr() {
        primero
    } else {
        segundo
    };
    unsafe { talc.free(antes, pedido) };
    antes.as_ptr()
}

fn hijo_hace_el_malloc_siguiente() -> ! {
    const CR2: u64 = 0x3250_0000_0000;
    let mut mem = vec![0u8; 64 * 1024];
    let mut talc = Talc::new(ErrOnOom);
    unsafe {
        talc.claim(Span::from_base_size(mem.as_mut_ptr(), mem.len()))
            .unwrap();
    }
    let hueco = hueco_delante(&mut talc);
    unsafe { hueco.cast::<u64>().write(CR2) };
    let pedido = layout(64);
    let _ = unsafe { talc.malloc(pedido) };
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_malloc_siguiente_no_revienta() {
        let pid = unsafe { fork() };
        assert!(pid >= 0, "fork");
        if pid == 0 {
            hijo_hace_el_malloc_siguiente();
        }
        let mut status = 0;
        let w = unsafe { waitpid(pid, &mut status, 0) };
        assert_eq!(w, pid);
        let senal = status & 0x7f;
        let salio = senal == 0;
        assert!(
            salio && (status >> 8) == 0,
            "el malloc siguiente murió (status {status:#x}, señal {senal}): \
             es el page fault de IterMut::next"
        );
    }
}
