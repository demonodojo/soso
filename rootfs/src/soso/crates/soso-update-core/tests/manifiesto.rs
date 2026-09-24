//! U7, fila «host, paquete»: qué manifiestos se rechazan. Un pack es código que
//! va a sustituir al que está corriendo, así que aquí no vale «casi bien»: lo
//! que no cuadra se rechaza **antes** de descargar nada.

use soso_update_core::manifest::{Manifest, ParseError, ValidateError, MANIFEST_MAGIC};

const H: &str = "ab";

fn hash(c: char) -> String {
    core::iter::repeat_n(c, 64).collect()
}

/// Manifiesto mínimo válido con las entradas que se le pasen.
fn texto(files: &[(&str, u64, u64, char)]) -> String {
    let mut t = format!(
        "{MANIFEST_MAGIC}\nversion=1.0.0\nbuild=b\nfecha=2026-01-01\n\
         kernel {} 100\npack {} 1000\n",
        hash('a'),
        hash('b')
    );
    for (p, off, size, h) in files {
        t.push_str(&format!("f {} {off} {size} {p}\n", hash(*h)));
    }
    t
}

fn valida(files: &[(&str, u64, u64, char)]) -> Result<(), ValidateError> {
    Manifest::parse(&texto(files)).expect("parsea").validate()
}

#[test]
fn un_manifiesto_normal_pasa() {
    assert_eq!(valida(&[("bin/sosh", 0, 10, 'c')]), Ok(()));
}

#[test]
fn sin_la_cabecera_no_se_interpreta() {
    // Sin esto, cualquier fichero de texto parecería una release.
    for t in ["version=1.0.0\n", "SOSOREL v2\n", "sosorel v1\n"] {
        assert_eq!(
            Manifest::parse(t).err(),
            Some(ParseError::BadMagic),
            "{t:?} no es una release"
        );
    }
    // Vacío también se rechaza; ahí da igual si lo llama «sin cabecera» o
    // «faltan campos», mientras no parezca una release.
    assert!(Manifest::parse("").is_err());
}

#[test]
fn una_ruta_absoluta_no_entra() {
    // «/etc/passwd» escribiría fuera del árbol que administra la release.
    assert!(matches!(
        valida(&[("/etc/passwd", 0, 10, 'c')]),
        Err(ValidateError::BadPath { .. })
    ));
}

#[test]
fn subir_de_directorio_no_entra() {
    for ruta in ["../etc/passwd", "bin/../../etc/passwd", ".."] {
        assert!(
            matches!(valida(&[(ruta, 0, 10, 'c')]), Err(ValidateError::BadPath { .. })),
            "{ruta} tenía que rechazarse"
        );
    }
}

#[test]
fn la_barra_invertida_tampoco() {
    // Un alias del mismo sitio por otro camino: lo que se compara después son
    // rutas, y dos formas de escribir la misma dejarían de coincidir.
    assert!(matches!(
        valida(&[("bin\\sosh", 0, 10, 'c')]),
        Err(ValidateError::BadPath { .. })
    ));
}

#[test]
fn dos_veces_el_mismo_fichero_no() {
    // Cuál de los dos gana no lo decide nadie: es un manifiesto ambiguo.
    assert!(matches!(
        valida(&[("bin/sosh", 0, 10, 'c'), ("bin/sosh", 20, 10, 'd')]),
        Err(ValidateError::DuplicatePath { .. })
    ));
}

#[test]
fn un_fichero_vacio_no_tiene_sentido_en_un_pack() {
    assert!(matches!(
        valida(&[("bin/sosh", 0, 0, 'c')]),
        Err(ValidateError::EmptyFile { .. })
    ));
}

#[test]
fn nada_puede_salirse_del_pack() {
    // Offsets que apuntan fuera: al leer el tramo se leería basura o se
    // desbordaría el buffer.
    assert!(matches!(
        valida(&[("bin/sosh", 990, 100, 'c')]),
        Err(ValidateError::FileOutOfPack { .. })
    ));
}

#[test]
fn dos_ficheros_no_pueden_pisarse_en_el_pack() {
    assert_eq!(
        valida(&[("bin/a", 0, 100, 'c'), ("bin/b", 50, 100, 'd')]),
        Err(ValidateError::OverlappingFiles)
    );
}

#[test]
fn los_hashes_tienen_que_ser_hashes() {
    let t = format!(
        "{MANIFEST_MAGIC}\nversion=1.0.0\nbuild=b\nfecha=2026-01-01\n\
         kernel {H} 100\npack {} 1000\n",
        hash('b')
    );
    assert!(matches!(
        Manifest::parse(&t).expect("parsea").validate(),
        Err(ValidateError::BadHash { field: "kernel" })
    ));
}

#[test]
fn un_fichero_gigante_se_rechaza_por_tamano() {
    let t = format!(
        "{MANIFEST_MAGIC}\nversion=1.0.0\nbuild=b\nfecha=2026-01-01\n\
         kernel {} 100\npack {} 1000000000000\nf {} 0 {} bin/sosh\n",
        hash('a'),
        hash('b'),
        hash('c'),
        soso_update_core::manifest::MAX_FILE_SIZE + 1
    );
    assert!(matches!(
        Manifest::parse(&t).expect("parsea").validate(),
        Err(ValidateError::FileTooLarge { .. })
    ));
}

#[test]
fn el_mismo_numero_con_otro_build_es_otra_release() {
    // Dos builds comparten número más a menudo de lo que parece (un rebuild),
    // y la identidad de la operación sale del manifiesto entero, no del número.
    let a = texto(&[("bin/sosh", 0, 10, 'c')]);
    let b = a.replace("build=b", "build=otro");
    let id_a = soso_update_core::txn::TxnId::from_manifest(a.as_bytes());
    let id_b = soso_update_core::txn::TxnId::from_manifest(b.as_bytes());
    assert_ne!(id_a, id_b, "misma versión y otro build son otra operación");
}
