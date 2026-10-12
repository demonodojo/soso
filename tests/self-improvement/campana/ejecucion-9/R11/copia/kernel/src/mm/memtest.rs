//! Memtest ligero: escribe/lee un patrón en unos pocos MiB de frames libres.

use crate::println;
use x86_64::structures::paging::{FrameAllocator, FrameDeallocator, Size4KiB};

/// Máximo a probar (no alargar el arranque de tests).
const MAX_BYTES: usize = 4 * 1024 * 1024; // 4 MiB
const PATTERN: u64 = 0xA5A5_5A5A_C3C3_3C3C;

/// Recorre hasta `MAX_BYTES` de frames libres. Loguea OK o paniquea.
pub fn run() {
    let Some(fa) = crate::mm::FRAME_ALLOC.get() else {
        return;
    };
    let free = fa.lock().free_frames();
    let want_frames = (MAX_BYTES / 4096).min(free / 4).max(1);
    if want_frames == 0 {
        println!("memtest: sin frames libres");
        return;
    }

    let mut frames = alloc::vec::Vec::with_capacity(want_frames);
    {
        let mut a = fa.lock();
        for _ in 0..want_frames {
            match FrameAllocator::<Size4KiB>::allocate_frame(&mut *a) {
                Some(f) => frames.push(f),
                None => break,
            }
        }
    }

    if frames.is_empty() {
        println!("memtest: no se pudo reservar");
        return;
    }

    let kib = frames.len() * 4;

    for (i, frame) in frames.iter().enumerate() {
        let virt = crate::mm::phys_to_virt(frame.start_address().as_u64());
        let ptr = virt.as_mut_ptr::<u64>();
        let words = 4096 / 8;
        unsafe {
            for w in 0..words {
                ptr.add(w).write_volatile(PATTERN ^ (i as u64) ^ (w as u64));
            }
        }
    }
    for (i, frame) in frames.iter().enumerate() {
        let virt = crate::mm::phys_to_virt(frame.start_address().as_u64());
        let ptr = virt.as_ptr::<u64>();
        let words = 4096 / 8;
        unsafe {
            for w in 0..words {
                let got = ptr.add(w).read_volatile();
                let expect = PATTERN ^ (i as u64) ^ (w as u64);
                if got != expect {
                    panic!(
                        "memtest: fallo en frame {:#x} word {w}: got={got:#x} expect={expect:#x}",
                        frame.start_address().as_u64()
                    );
                }
            }
        }
    }

    {
        let mut a = fa.lock();
        for f in frames.drain(..) {
            unsafe { FrameDeallocator::<Size4KiB>::deallocate_frame(&mut *a, f) };
        }
    }
    println!("memtest: OK ({kib} KiB)");
}
