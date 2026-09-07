//! Cargador dinámico mínimo para proc-macros (Hito 3b).

use core::ffi::c_void;

const ELF_MAGIC: [u8; 4] = *b"\x7fELF";

#[repr(C)]
pub struct Elf64Ehdr {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

pub struct Library {
    base: *mut u8,
    len: usize,
}

impl Library {
    /// Inspecciona la cabecera ELF; carga completa pendiente (relocations/TLS).
    pub unsafe fn open(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() < 64 || bytes[0..4] != ELF_MAGIC {
            return Err("no es ELF64");
        }
        let e_type = u16::from_le_bytes([bytes[0x10], bytes[0x11]]);
        if e_type != 3 && e_type != 2 {
            // ET_DYN or ET_EXEC
            return Err("tipo ELF no soportado aun");
        }
        Ok(Self {
            base: bytes.as_ptr() as *mut u8,
            len: bytes.len(),
        })
    }

    pub unsafe fn symbol<T>(&self, _name: &str) -> Result<T, &'static str> {
        Err("dlopen: sin tabla de simbolos aun")
    }
}

pub unsafe fn close(_lib: Library) {}

pub fn parse_ehdr(bytes: &[u8]) -> Option<Elf64Ehdr> {
    if bytes.len() < core::mem::size_of::<Elf64Ehdr>() || bytes[0..4] != ELF_MAGIC {
        return None;
    }
    Some(Elf64Ehdr {
        e_ident: bytes[0..16].try_into().ok()?,
        e_type: u16::from_le_bytes([bytes[0x10], bytes[0x11]]),
        e_machine: u16::from_le_bytes([bytes[0x12], bytes[0x13]]),
        e_version: u32::from_le_bytes([bytes[0x14], bytes[0x15], bytes[0x16], bytes[0x17]]),
        e_entry: u64::from_le_bytes(bytes[0x18..0x20].try_into().ok()?),
        e_phoff: u64::from_le_bytes(bytes[0x20..0x28].try_into().ok()?),
        e_shoff: u64::from_le_bytes(bytes[0x28..0x30].try_into().ok()?),
        e_flags: u32::from_le_bytes(bytes[0x30..0x34].try_into().ok()?),
        e_ehsize: u16::from_le_bytes([bytes[0x34], bytes[0x35]]),
        e_phentsize: u16::from_le_bytes([bytes[0x36], bytes[0x37]]),
        e_phnum: u16::from_le_bytes([bytes[0x38], bytes[0x39]]),
        e_shentsize: u16::from_le_bytes([bytes[0x3a], bytes[0x3b]]),
        e_shnum: u16::from_le_bytes([bytes[0x3c], bytes[0x3d]]),
        e_shstrndx: u16::from_le_bytes([bytes[0x3e], bytes[0x3f]]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_non_elf() {
        assert!(Library::open(b"not elf").is_err());
    }
}
