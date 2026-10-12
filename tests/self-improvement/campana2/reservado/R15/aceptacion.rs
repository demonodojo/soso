//! Aceptación reservada de R15.

use soso_update_core::{hex_sha256, FileEntry, PackReader};

fn entrada(path: &str, datos: &[u8], offset: u64) -> FileEntry {
    FileEntry { path: path.into(), offset, size: datos.len() as u64, hash_hex: hex_sha256(datos) }
}

#[test]
fn un_pack_sano_no_devuelve_nada() {
    let blob = b"aaaabbbbcc".to_vec();
    let files = vec![entrada("a", b"aaaa", 0), entrada("b", b"bbbb", 4), entrada("c", b"cc", 8)];
    let r = PackReader::new(&blob);
    assert!(r.verify_all(&files).is_empty());
    assert!(r.verify_all(&[]).is_empty());
}

#[test]
fn devuelve_todas_las_que_fallan_y_en_orden() {
    let blob = b"aaaaXXXXcc".to_vec();
    let mut fuera = entrada("d", b"zz", 0);
    fuera.offset = 9; // 9 + 2 > 10: se sale del pack
    let files = vec![entrada("a", b"aaaa", 0), entrada("b", b"bbbb", 4), entrada("c", b"cc", 8), fuera];
    let r = PackReader::new(&blob);
    let malas = r.verify_all(&files);
    let nombres: Vec<&str> = malas.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(nombres, ["b", "d"]);
}

#[test]
fn coincide_con_verify_entry_una_a_una() {
    let blob = b"hola mundo".to_vec();
    let files = vec![entrada("x", b"hola", 0), entrada("y", b"mundo", 5), entrada("z", b"nope", 0)];
    let r = PackReader::new(&blob);
    let esperadas: Vec<&FileEntry> = files.iter().filter(|f| !r.verify_entry(f)).collect();
    assert_eq!(r.verify_all(&files).len(), esperadas.len());
    assert_eq!(r.verify_all(&files)[0].path, "z");
}
