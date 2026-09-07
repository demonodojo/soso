//! Formato `rootfs.pack`: concatenación de ficheros con offsets en el manifest.

use alloc::string::String;
use alloc::vec::Vec;

use crate::hash::{hex_sha256, sha256};
use crate::manifest::FileEntry;

#[derive(Clone, Debug)]
pub struct PackEntry {
    pub path: String,
    pub data: Vec<u8>,
}

/// Lee entradas desde un pack ya concatenado + manifest.
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

    pub fn verify_entry(&self, entry: &FileEntry) -> bool {
        self.read_entry(entry)
            .map(|d| hex_sha256(d) == entry.hash_hex)
            .unwrap_or(false)
    }
}

/// Construye un pack (sólo en host con std).
#[cfg(feature = "std")]
pub struct PackWriter {
    entries: Vec<PackEntry>,
}

#[cfg(feature = "std")]
impl PackWriter {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn should_pack(rel: &str) -> bool {
        use crate::{PACK_SKIP, PACK_SKIP_DIRS};
        !PACK_SKIP.iter().any(|skip| *skip == rel)
            && !PACK_SKIP_DIRS.iter().any(|dir| rel.starts_with(dir))
    }

    pub fn push(&mut self, path: String, data: Vec<u8>) {
        self.entries.push(PackEntry { path, data });
    }

    pub fn sort_entries(&mut self) {
        self.entries.sort_by(|a, b| a.path.cmp(&b.path));
    }

    pub fn build(self) -> (Vec<u8>, Vec<FileEntry>) {
        let mut blob = Vec::new();
        let mut files = Vec::new();
        for e in self.entries {
            let offset = blob.len() as u64;
            let hash_hex = hex_sha256(&e.data);
            let size = e.data.len() as u64;
            files.push(FileEntry {
                path: e.path,
                offset,
                size,
                hash_hex,
            });
            blob.extend_from_slice(&e.data);
        }
        (blob, files)
    }
}

#[cfg(feature = "std")]
impl Default for PackWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Empaqueta un directorio rootfs (host).
#[cfg(feature = "std")]
pub fn pack_rootfs(root: &std::path::Path) -> std::io::Result<(Vec<u8>, Vec<FileEntry>)> {
    pack_rootfs_con(root, |_| true)
}

/// Como [`pack_rootfs`] pero admitiendo un filtro extra sobre la ruta relativa.
#[cfg(feature = "std")]
pub fn pack_rootfs_con<F: Fn(&str) -> bool>(
    root: &std::path::Path,
    incluir: F,
) -> std::io::Result<(Vec<u8>, Vec<FileEntry>)> {
    use std::fs;
    use std::io;

    let mut writer = PackWriter::new();
    let mut stack: Vec<std::path::PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for ent in fs::read_dir(&dir)? {
            let ent = ent?;
            let path = ent.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|_| io::Error::other("strip_prefix"))?;
                let rel = rel.to_string_lossy().replace('\\', "/");
                if !PackWriter::should_pack(&rel) || !incluir(&rel) {
                    continue;
                }
                let data = fs::read(&path)?;
                writer.push(rel, data);
            }
        }
    }
    writer.sort_entries();
    Ok(writer.build())
}

#[cfg(test)]
mod tests {
    use super::PackWriter;

    #[test]
    fn no_empaqueta_volumenes_de_solo_lectura() {
        // /models es sosomfs: escribir ahí da EIO y rompe `soso-update aplicar`.
        assert!(!PackWriter::should_pack("models/tiny/index.som"));
        assert!(!PackWriter::should_pack("var/actualiza-prueba/rootfs.pack"));
        assert!(!PackWriter::should_pack("etc/soso-release"));
        assert!(PackWriter::should_pack("bin/soso-update"));
        assert!(PackWriter::should_pack("lib/firmware/nvidia/gb205/gsp/fmc-570.144.bin"));
    }
}

pub fn pack_hash(data: &[u8]) -> String {
    hex_sha256(data)
}

pub fn pack_digest(data: &[u8]) -> [u8; 32] {
    sha256(data)
}
