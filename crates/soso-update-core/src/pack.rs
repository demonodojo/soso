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
        crate::por_que_se_excluye(rel).is_none()
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
                // `Manifest::validate` rechaza los ficheros vacíos. `aplicar`
                // nunca borra, así que los que ya hay en disco se quedan.
                if data.is_empty() {
                    continue;
                }
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

    #[cfg(feature = "std")]
    #[test]
    fn los_ficheros_vacios_no_entran_y_el_manifiesto_valida() {
        let root = std::env::temp_dir().join(format!("soso-pack-vacios-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("lib/src")).unwrap();
        std::fs::write(root.join("lib/src/macro.rs"), b"").unwrap();
        std::fs::write(root.join("lib/src/lib.rs"), b"mod macro;").unwrap();
        let (blob, files) = super::pack_rootfs(&root).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        let rutas: alloc::vec::Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(rutas, ["lib/src/lib.rs"]);
        let m = crate::manifest::Manifest {
            version: crate::semver::parse("1.0.0").unwrap(),
            version_raw: "1.0.0".into(),
            build: "b".into(),
            fecha: "2026-10-10".into(),
            kernel_hash: super::pack_hash(b"k"),
            kernel_size: 1,
            pack_hash: super::pack_hash(&blob),
            pack_size: blob.len() as u64,
            compat: None,
            files,
        };
        assert_eq!(m.validate(), Ok(()));
    }
}

pub fn pack_hash(data: &[u8]) -> String {
    hex_sha256(data)
}

pub fn pack_digest(data: &[u8]) -> [u8; 32] {
    sha256(data)
}
