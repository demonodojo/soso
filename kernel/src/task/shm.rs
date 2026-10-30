//! Segmentos de memoria compartida entre procesos (ids globales).

use alloc::vec::Vec;
use spin::Mutex;
use x86_64::structures::paging::{FrameAllocator, FrameDeallocator, PhysFrame, Size4KiB};

use super::addrspace::{AddrSpace, UserProt};
use super::mmap::{self, MmapRegion};
use crate::mm;

const SHM_MAX_BYTES: u64 = 64 * 1024 * 1024;
const SHM_MAX_SEGMENTS: usize = 32;

struct Segment {
    frames: Vec<PhysFrame<Size4KiB>>,
    len: u64,
    creator: u64,
    creador_as: u64,
    concedidos: Vec<u64>,
    refs: u32,
}

static SEGMENTS: Mutex<Vec<Option<Segment>>> = Mutex::new(Vec::new());

fn align_len(len: u64) -> Result<u64, i64> {
    if len == 0 || len > SHM_MAX_BYTES {
        return Err(-soso_abi::EINVAL);
    }
    len.checked_next_multiple_of(4096).ok_or(-soso_abi::EINVAL)
}

pub fn create(len: u64) -> Result<u64, i64> {
    let len = align_len(len)?;
    let pages = (len / 4096) as usize;
    let mut frames = Vec::with_capacity(pages);
    let mut fa = mm::FRAME_ALLOC.get().ok_or(-soso_abi::ENOMEM)?.lock();
    for _ in 0..pages {
        let Some(f) = FrameAllocator::<Size4KiB>::allocate_frame(&mut *fa) else {
            return Err(-soso_abi::ENOMEM);
        };
        frames.push(f);
    }
    drop(fa);
    for f in &frames {
        let va = mm::phys_to_virt(f.start_address().as_u64()).as_u64();
        unsafe {
            core::ptr::write_bytes(va as *mut u8, 0, 4096);
        }
    }
    let pid = super::current_pid();
    let creador_as = super::current_space_id().ok_or(-soso_abi::EINVAL)?;
    let mut segs = SEGMENTS.lock();
    if segs.len() >= SHM_MAX_SEGMENTS && !segs.iter().any(|s| s.is_none()) {
        return Err(-soso_abi::EMFILE);
    }
    let seg = Segment {
        frames,
        len,
        creator: pid,
        creador_as,
        concedidos: Vec::new(),
        refs: 1,
    };
    if let Some(hole) = segs.iter().position(|s| s.is_none()) {
        segs[hole] = Some(seg);
        Ok(hole as u64 + 1)
    } else {
        segs.push(Some(seg));
        Ok(segs.len() as u64)
    }
}

fn permitido(seg: &Segment, as_id: u64) -> bool {
    seg.creador_as == as_id || seg.concedidos.iter().any(|&a| a == as_id)
}

pub fn grant(id: u64, pid: u64) -> Result<(), i64> {
    if id == 0 {
        return Err(-soso_abi::EINVAL);
    }
    let caller = super::current_space_id().ok_or(-soso_abi::EINVAL)?;
    let target_as = super::space_id_for_pid(pid).ok_or(-soso_abi::ESRCH)?;
    let idx = (id - 1) as usize;
    let mut segs = SEGMENTS.lock();
    let seg = segs.get_mut(idx).and_then(|s| s.as_mut()).ok_or(-soso_abi::EINVAL)?;
    if seg.creador_as != caller {
        return Err(-soso_abi::EPERM);
    }
    if !seg.concedidos.iter().any(|&a| a == target_as) {
        seg.concedidos.push(target_as);
    }
    Ok(())
}

pub fn map(id: u64, space: AddrSpace) -> Result<u64, i64> {
    if id == 0 {
        return Err(-soso_abi::EINVAL);
    }
    let as_id = space.id();
    let idx = (id - 1) as usize;
    let mut segs = SEGMENTS.lock();
    let Some(seg) = segs.get_mut(idx).and_then(|s| s.as_mut()) else {
        return Err(-soso_abi::EINVAL);
    };
    if !permitido(seg, as_id) {
        return Err(-soso_abi::EPERM);
    }
    seg.refs = seg.refs.saturating_add(1);
    let len = seg.len;
    let frames = seg.frames.clone();
    drop(segs);

    let virt = space.with_mmap_mut(|book| {
        let virt = mmap::next_addr(&book.regions, 0, len).ok_or(-soso_abi::ENOMEM)?;
        book.regions.push(MmapRegion {
            virt_start: virt,
            len,
            inode: 0,
            file_offset: 0,
            file_len: 0,
            writable: true,
            exec: false,
            sin_acceso: false,
            shm_id: id,
        });
        Ok::<u64, i64>(virt)
    })?;

    let mut off = 0u64;
    for f in frames {
        if space
            .map_page(virt + off, f, UserProt::Rw)
            .is_none()
        {
            return Err(-soso_abi::ENOMEM);
        }
        off += 4096;
    }
    Ok(virt)
}

pub fn drop_ref(id: u64) {
    if id == 0 {
        return;
    }
    let idx = (id - 1) as usize;
    let mut segs = SEGMENTS.lock();
    let Some(entry) = segs.get_mut(idx) else {
        return;
    };
    let Some(seg) = entry.as_mut() else {
        return;
    };
    seg.refs = seg.refs.saturating_sub(1);
    if seg.refs == 0 {
        let Some(mut seg) = entry.take() else {
            return;
        };
        free_frames(&mut seg.frames);
    }
}

fn free_frames(frames: &mut Vec<PhysFrame<Size4KiB>>) {
    let mut fa = mm::FRAME_ALLOC.get().unwrap().lock();
    for f in frames.drain(..) {
        unsafe {
            fa.deallocate_frame(f);
        }
    }
}

pub fn free(id: u64, pid: u64) -> Result<(), i64> {
    if id == 0 {
        return Err(-soso_abi::EINVAL);
    }
    let idx = (id - 1) as usize;
    let mut segs = SEGMENTS.lock();
    let Some(entry) = segs.get_mut(idx) else {
        return Err(-soso_abi::EINVAL);
    };
    let Some(seg) = entry.as_mut() else {
        return Err(-soso_abi::EINVAL);
    };
    if seg.creator != pid {
        return Err(-soso_abi::EPERM);
    }
    seg.refs = seg.refs.saturating_sub(1);
    if seg.refs == 0 {
        let Some(mut seg) = entry.take() else {
            return Ok(());
        };
        free_frames(&mut seg.frames);
    }
    Ok(())
}
