//! Lectura de `paquete.pack`.

use crate::manifest::FileEntry;

pub struct PackReader<'a> {
    pub data: &'a [u8],
}

impl<'a> PackReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub fn read_entry(&self, entry: &FileEntry) -> Option<&'a [u8]> {
        let start = entry.offset as usize;
        let end = start.checked_add(entry.size as usize)?;
        self.data.get(start..end)
    }
}
