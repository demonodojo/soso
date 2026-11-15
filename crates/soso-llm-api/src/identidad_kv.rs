//! Identidad de snapshots en host: incluye contenido de cada shard, no solo
//! la tabla de tensores. Requiere que los ficheros del modelo permanezcan
//! inmutables durante la generación que vaya a exportarse o importarse.

use sha2::{Digest, Sha256};
use sosomodel::index::TensorIndex;
use std::collections::BTreeSet;
use std::io::{self, Read};
use std::path::{Component, Path};

fn campo(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

pub fn hash_fichero(path: &Path) -> io::Result<[u8; 32]> {
    let mut f = std::fs::File::open(path)?;
    let antes = f.metadata()?;
    if !antes.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no es un fichero"));
    }
    let mut hash = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        total = total.checked_add(n as u64).ok_or_else(|| io::Error::other("tamaño desbordado"))?;
        hash.update(&buf[..n]);
    }
    let despues = f.metadata()?;
    if total != antes.len() || antes.len() != despues.len() || antes.modified()? != despues.modified()? {
        return Err(io::Error::other("el fichero cambió durante el cálculo de identidad"));
    }
    Ok(hash.finalize().into())
}

/// Calcula SHA-256 sobre el manifiesto, índice, tokenizador, todos los shards
/// referenciados y la identidad del ejecutable/configuración. El llamante debe
/// incluir en `ejecucion` toda opción que pueda cambiar los valores KV y la
/// versión del motor. Se rechazan rutas fuera del directorio de shards.
pub fn calcular(modelo: &Path, ejecucion: &[u8]) -> io::Result<[u8; 32]> {
    if ejecucion.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "falta identidad de ejecución"));
    }
    let mut hash = Sha256::new();
    campo(&mut hash, b"soso-kv-perfil-v1");
    campo(&mut hash, ejecucion);
    let indice_bytes = std::fs::read(modelo.join("index.som"))?;
    let index = TensorIndex::parse(&indice_bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "índice inválido"))?;
    for nombre in ["manifest.som", "index.som", "tokenizer.som"] {
        campo(&mut hash, nombre.as_bytes());
        let contenido = hash_fichero(&modelo.join(nombre))?;
        if nombre == "index.som" && contenido.as_slice() != Sha256::digest(&indice_bytes).as_slice() {
            return Err(io::Error::other("el índice cambió durante el cálculo de identidad"));
        }
        campo(&mut hash, &contenido);
    }
    let mut shards = BTreeSet::new();
    for e in &index.entries {
        let p = Path::new(&e.shard);
        if p.as_os_str().is_empty() || !p.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "ruta de shard inválida"));
        }
        shards.insert(e.shard.as_str());
    }
    if shards.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "índice sin shards"));
    }
    let raiz = modelo.join("shards").canonicalize()?;
    for nombre in shards {
        let ruta = raiz.join(nombre).canonicalize()?;
        if !ruta.starts_with(&raiz) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "shard fuera del directorio"));
        }
        campo(&mut hash, nombre.as_bytes());
        campo(&mut hash, &hash_fichero(&ruta)?);
    }
    Ok(hash.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sosomodel::index::make_f32_entry;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);
    struct Dir(std::path::PathBuf);
    impl Dir {
        fn nuevo() -> Self {
            let path = std::env::temp_dir().join(format!("soso-kvid-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(path.join("shards")).unwrap();
            for nombre in ["manifest.som", "index.som", "tokenizer.som", "shards/peso"] {
                std::fs::write(path.join(nombre), nombre).unwrap();
            }
            Self(path)
        }
    }
    impl Drop for Dir { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    #[test]
    fn identidad_incluye_contenido_y_configuracion() {
        let d = Dir::nuevo();
        let index = TensorIndex { entries: vec![make_f32_entry(0, "x", "peso", 0, &[1])] };
        std::fs::write(d.0.join("index.som"), index.serialize()).unwrap();
        let base = calcular(&d.0, b"motor-v1").unwrap();
        assert_eq!(base, calcular(&d.0, b"motor-v1").unwrap());
        assert_ne!(base, calcular(&d.0, b"motor-v2").unwrap());
        std::fs::write(d.0.join("shards/peso"), b"otro peso").unwrap();
        assert_ne!(base, calcular(&d.0, b"motor-v1").unwrap());
        std::fs::write(d.0.join("shards/peso"), b"shards/peso").unwrap();
        std::fs::write(d.0.join("tokenizer.som"), b"otro").unwrap();
        assert_ne!(base, calcular(&d.0, b"motor-v1").unwrap());
    }

    #[test]
    fn rechaza_rutas_y_datos_incompletos() {
        let d = Dir::nuevo();
        let mut index = TensorIndex { entries: vec![make_f32_entry(0, "x", "../peso", 0, &[1])] };
        std::fs::write(d.0.join("index.som"), index.serialize()).unwrap();
        assert_eq!(calcular(&d.0, b"motor").unwrap_err().kind(), io::ErrorKind::InvalidData);
        index.entries[0].shard = "peso".into();
        std::fs::write(d.0.join("index.som"), index.serialize()).unwrap();
        assert_eq!(calcular(&d.0, b"").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        std::fs::remove_file(d.0.join("shards/peso")).unwrap();
        assert_eq!(calcular(&d.0, b"motor").unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn rechaza_enlace_fuera_de_shards() {
        let d = Dir::nuevo();
        let index = TensorIndex { entries: vec![make_f32_entry(0, "x", "escape", 0, &[1])] };
        std::fs::write(d.0.join("index.som"), index.serialize()).unwrap();
        std::os::unix::fs::symlink(d.0.join("manifest.som"), d.0.join("shards/escape")).unwrap();
        assert_eq!(calcular(&d.0, b"motor").unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
