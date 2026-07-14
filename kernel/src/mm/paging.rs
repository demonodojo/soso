//! Acceso a las tablas de páginas vía el mapeo completo de memoria física
//! que nos deja el bootloader (config `physical_memory`).

use x86_64::VirtAddr;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{OffsetPageTable, PageTable};

/// # Safety
/// `phys_offset` debe ser el offset real del mapeo de toda la memoria
/// física, y solo puede llamarse una vez (aliasing de la tabla L4).
pub unsafe fn init(phys_offset: VirtAddr) -> OffsetPageTable<'static> {
    let (l4_frame, _) = Cr3::read();
    let virt = phys_offset + l4_frame.start_address().as_u64();
    let l4: &mut PageTable = unsafe { &mut *virt.as_mut_ptr() };
    unsafe { OffsetPageTable::new(l4, phys_offset) }
}
