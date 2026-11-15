//! E/S acotada y publicación atómica de una instantánea. El llamante controla
//! el directorio privado, la identidad de usuario y la validez de su contenido.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use sha2::{Digest, Sha256};

static SEQ: AtomicU64 = AtomicU64::new(0);
const MAX_IMAGENES: usize = 64;

fn nombre_imagen(nombre: &str) -> bool {
    nombre == "prefijo.kv" || nombre.strip_prefix("kv-")
        .and_then(|s| s.strip_suffix(".snap"))
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
}

/// Lista únicamente ficheros propios, sin seguir enlaces. El límite también
/// acota el trabajo de búsqueda tras un reinicio.
pub fn listar(directorio: &Path) -> io::Result<Vec<std::path::PathBuf>> {
    let mut archivos = Vec::new();
    for entrada in std::fs::read_dir(directorio)? {
        let entrada = entrada?;
        let nombre = entrada.file_name();
        if nombre.to_str().is_some_and(nombre_imagen) && entrada.file_type()?.is_file() {
            archivos.push(entrada.path());
            if archivos.len() > MAX_IMAGENES {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "demasiadas instantáneas KV"));
            }
        }
    }
    archivos.sort();
    Ok(archivos)
}

#[cfg(unix)]
fn bloquear(directorio: &Path) -> io::Result<File> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    unsafe extern "C" { fn flock(fd: i32, operation: i32) -> i32; }
    let ruta = directorio.join(".soso-kv.lock");
    let mut opciones = OpenOptions::new();
    opciones.read(true).write(true).create(true).mode(0o600);
    #[cfg(target_os = "linux")]
    opciones.custom_flags(0o400000); // O_NOFOLLOW
    let archivo = opciones.open(ruta)?;
    if unsafe { flock(archivo.as_raw_fd(), 2 | 4) } != 0 { // LOCK_EX | LOCK_NB
        return Err(io::Error::last_os_error());
    }
    Ok(archivo) // cerrar libera el candado incluso si hay error
}

/// Guarda contenido direccionado por hash. La cuota comprende instantáneas y
/// temporales propios; al llenarse se conserva lo anterior y se rechaza la nueva.
/// Los escritores cooperantes comparten un candado no bloqueante.
#[cfg(unix)]
pub fn guardar_multiple(directorio: &Path, bytes: &[u8], cuota: usize) -> io::Result<()> {
    let _candado = bloquear(directorio)?;
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let destino = directorio.join(format!("kv-{hex}.snap"));
    let reemplazar = destino.exists();
    if reemplazar && leer(&destino, cuota)? == bytes { return Ok(()); }
    let existentes = listar(directorio)?;
    if !reemplazar && existentes.len() >= MAX_IMAGENES {
        return Err(io::Error::other("límite de instantáneas KV"));
    }
    let mut usado = 0u64;
    for entrada in std::fs::read_dir(directorio)? {
        let entrada = entrada?;
        let nombre = entrada.file_name();
        let Some(nombre) = nombre.to_str() else { continue };
        if (nombre_imagen(nombre) || nombre.starts_with("prefijo.kv.tmp-")
            || (nombre.starts_with("kv-") && nombre.contains(".snap.tmp-")))
            && entrada.file_type()?.is_file() {
            usado = usado.checked_add(entrada.metadata()?.len()).ok_or_else(|| io::Error::other("cuota KV desbordada"))?;
        }
    }
    if usado.checked_add(bytes.len() as u64).is_none_or(|n| n > cuota as u64) {
        return Err(io::Error::other("cuota KV agotada"));
    }
    publicar(&destino, bytes, cuota)
}

#[cfg(not(unix))]
pub fn guardar_multiple(_: &Path, _: &[u8], _: usize) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "el almacén KV requiere candado de ficheros"))
}

/// Solo se admite un directorio propiedad del usuario efectivo, sin acceso
/// para grupo ni otros. No crea ni cambia permisos de directorios existentes.
pub fn verificar_directorio_privado(ruta: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        unsafe extern "C" { fn geteuid() -> u32; }
        let meta = std::fs::symlink_metadata(ruta)?;
        if !meta.is_dir() || meta.uid() != unsafe { geteuid() } || meta.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "directorio KV no es privado o no pertenece al usuario"));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = ruta;
        Err(io::Error::new(io::ErrorKind::Unsupported, "no se puede verificar privacidad del directorio"))
    }
}

/// Lee hasta `max_bytes`, también si el fichero crece tras consultar su tamaño.
pub fn leer(ruta: &Path, max_bytes: usize) -> io::Result<Vec<u8>> {
    if std::fs::symlink_metadata(ruta)?.file_type().is_symlink() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "enlace simbólico de KV"));
    }
    let f = File::open(ruta)?;
    let limite = u64::try_from(max_bytes).map_err(|_| io::Error::other("límite inválido"))?;
    if f.metadata()?.len() > limite {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "instantánea demasiado grande"));
    }
    let mut bytes = Vec::new();
    f.take(limite.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "instantánea demasiado grande"));
    }
    Ok(bytes)
}

/// Escribe en el mismo directorio y renombra solo después de sincronizar datos.
/// El directorio debe ser privado y exclusivo de la identidad del llamante.
pub fn publicar(ruta: &Path, bytes: &[u8], max_bytes: usize) -> io::Result<()> {
    if bytes.len() > max_bytes {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "instantánea demasiado grande"));
    }
    let padre = ruta.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let nombre = ruta.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "ruta sin nombre"))?;
    let mut temp = nombre.to_os_string();
    temp.push(format!(".tmp-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    let temporal = padre.join(temp);
    let mut op = OpenOptions::new();
    op.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        op.mode(0o600);
    }
    let mut f = op.open(&temporal)?;
    let result = (|| {
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&temporal, ruta)?;
        File::open(padre)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(&temporal); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publicacion_acotada_preserva_anterior_ante_error() {
        let dir = std::env::temp_dir().join(format!("soso-kvstore-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&dir).unwrap();
        let ruta = dir.join("prefijo.kv");
        publicar(&ruta, b"primero", 32).unwrap();
        assert_eq!(leer(&ruta, 32).unwrap(), b"primero");
        assert!(publicar(&ruta, b"demasiado largo", 3).is_err());
        assert_eq!(leer(&ruta, 32).unwrap(), b"primero");
        assert_eq!(leer(&ruta, 3).unwrap_err().kind(), io::ErrorKind::InvalidData);
        publicar(&ruta, b"segundo", 32).unwrap();
        assert_eq!(leer(&ruta, 32).unwrap(), b"segundo");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn directorio_debe_ser_privado_y_no_un_enlace() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("soso-kvstore-privado-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(verificar_directorio_privado(&dir).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        verificar_directorio_privado(&dir).unwrap();
        let enlace = dir.join("enlace");
        std::os::unix::fs::symlink(&dir, &enlace).unwrap();
        assert_eq!(verificar_directorio_privado(&enlace).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recupera_instantanea_y_detecta_dano_en_disco() {
        use soso_llm_core::kvsnap::{self, Capa, Snapshot};
        let dir = std::env::temp_dir().join(format!("soso-kvstore-e2e-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&dir).unwrap();
        let ruta = dir.join("prefijo.kv");
        let perfil = [42u8; 32];
        let s = Snapshot { tokens: vec![1, 2], hidden: vec![0.5], capas: vec![
            Capa { k: vec![0x3c00; 4], v: vec![0x4000; 4] },
        ] };
        let bytes = kvsnap::codificar(&s, &perfil, &[2], 1024).unwrap();
        publicar(&ruta, &bytes, 1024).unwrap();
        let b = leer(&ruta, 1024).unwrap();
        let restaurado = kvsnap::decodificar(&b, &perfil, &[2], 16, 256, 1, 1024).unwrap();
        assert_eq!(restaurado.tokens, s.tokens);
        assert_eq!(restaurado.capas[0].k, s.capas[0].k);
        let mut b = b;
        let n = b.len();
        b[n - 33] ^= 1;
        std::fs::write(&ruta, b).unwrap();
        let b = leer(&ruta, 1024).unwrap();
        assert_eq!(kvsnap::decodificar(&b, &perfil, &[2], 16, 256, 1, 1024).err(), Some(kvsnap::Error::Integridad));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn varias_imagenes_respetan_cuota_y_conservan_las_previas() {
        let dir = std::env::temp_dir().join(format!("soso-kvstore-cuota-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&dir).unwrap();
        guardar_multiple(&dir, b"uno", 6).unwrap();
        guardar_multiple(&dir, b"uno", 6).unwrap();
        let ruta_uno = listar(&dir).unwrap().pop().unwrap();
        std::fs::write(&ruta_uno, b"mal").unwrap();
        assert!(guardar_multiple(&dir, b"uno", 5).is_err());
        assert_eq!(leer(&ruta_uno, 6).unwrap(), b"mal");
        guardar_multiple(&dir, b"uno", 6).unwrap();
        assert_eq!(leer(&ruta_uno, 6).unwrap(), b"uno");
        guardar_multiple(&dir, b"dos", 6).unwrap();
        assert!(guardar_multiple(&dir, b"tres", 6).is_err());
        let rutas = listar(&dir).unwrap();
        assert_eq!(rutas.len(), 2);
        let mut contenido: Vec<_> = rutas.iter().map(|p| leer(p, 6).unwrap()).collect();
        contenido.sort();
        assert_eq!(contenido, [b"dos".to_vec(), b"uno".to_vec()]);
        let candado = bloquear(&dir).unwrap();
        assert!(guardar_multiple(&dir, b"dos", 6).is_err());
        drop(candado);
        guardar_multiple(&dir, b"dos", 6).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
