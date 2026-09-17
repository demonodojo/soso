//! U5b: crear puntos y conservarlos entre actualizaciones sucesivas.
//!
//! Dos garantías, y las dos se rompen en silencio si nadie las vigila:
//! **un punto incompleto o sin espacio impide armar**, y **preparar C no puede
//! destruir la vuelta de B a A**.

use soso_update_core::txn::journal::{Accion, Contenido};
use soso_update_core::txn::punto::{
    crear_punto, Almacen, Cambio, CrearError, Punto, Retencion,
};
use soso_update_core::txn::TxnId;
use std::collections::BTreeMap;

const GUID: &str = "1b2c3d4e-0000-4000-8000-aabbccddeeff";

fn id(n: &str) -> TxnId {
    TxnId::from_manifest(n.as_bytes())
}

fn h(d: &[u8]) -> Contenido {
    Contenido { size: d.len() as u64, hash: soso_update_core::hex_sha256(d) }
}

/// Almacén simulado, con las averías que importan: quedarse sin espacio,
/// fallar al copiar y guardar algo que luego no se relee igual.
#[derive(Default)]
struct Disco {
    sistema: BTreeMap<String, Vec<u8>>,
    copias: BTreeMap<(String, String), Vec<u8>>,
    puntos: BTreeMap<String, Vec<u8>>,
    libre: u64,
    fallar_copia_de: Option<String>,
    /// Devuelve basura al releer esta ruta: simula lo que no llegó a disco.
    releer_mal: Option<String>,
    fallar_registro: bool,
}

impl Almacen for Disco {
    fn leer_sistema(&mut self, ruta: &str) -> Option<Vec<u8>> {
        self.sistema.get(ruta).cloned()
    }
    fn guardar_copia(&mut self, punto: TxnId, ruta: &str, datos: &[u8]) -> Result<(), ()> {
        if self.fallar_copia_de.as_deref() == Some(ruta) {
            return Err(());
        }
        self.copias.insert((punto.to_hex(), ruta.into()), datos.to_vec());
        Ok(())
    }
    fn releer_copia(&mut self, punto: TxnId, ruta: &str) -> Option<Vec<u8>> {
        if self.releer_mal.as_deref() == Some(ruta) {
            return Some(b"lo que de verdad quedo en disco".to_vec());
        }
        self.copias.get(&(punto.to_hex(), ruta.into())).cloned()
    }
    fn guardar_punto(&mut self, p: &Punto) -> Result<(), ()> {
        if self.fallar_registro {
            return Err(());
        }
        self.puntos.insert(p.id.to_hex(), p.format());
        Ok(())
    }
    fn borrar_punto(&mut self, punto: TxnId) -> Result<(), ()> {
        self.puntos.remove(&punto.to_hex());
        self.copias.retain(|(p, _), _| *p != punto.to_hex());
        Ok(())
    }
    fn espacio_libre(&mut self) -> u64 {
        self.libre
    }
}

fn disco() -> Disco {
    let mut d = Disco { libre: 1 << 30, ..Default::default() };
    d.sistema.insert("bin/sosh".into(), b"sosh de A".to_vec());
    d.sistema.insert("lib/vieja.so".into(), b"libreria de A".to_vec());
    d
}

/// Lo que trae y retira la versión nueva.
fn cambios() -> Vec<Cambio> {
    vec![
        Cambio::Trae("bin/sosh".into()),       // existe: se reemplaza
        Cambio::Trae("bin/nuevo-de-b".into()), // no existe: lo añade B
        Cambio::Retira("lib/vieja.so".into()), // B ya no la trae
    ]
}

fn crear(d: &mut Disco) -> Result<Punto, CrearError> {
    crear_punto(id("A"), "0.2.9", "abc", GUID, h(b"kernel de A"), &cambios(), d)
}

// ── Creación ─────────────────────────────────────────────────────────────

#[test]
fn el_punto_recoge_lo_que_cambia_y_lo_que_se_añade() {
    let mut d = disco();
    let p = crear(&mut d).expect("crear");

    let por_ruta = |r: &str| p.entradas.iter().find(|e| e.path == r).unwrap().clone();
    // Lo que se reemplaza y lo que se retira llevan copia; lo que añade B, no.
    assert_eq!(por_ruta("bin/sosh").accion, Accion::Reemplazar);
    assert_eq!(por_ruta("bin/sosh").respaldo, Some(h(b"sosh de A")));
    assert_eq!(por_ruta("lib/vieja.so").accion, Accion::Borrar);
    assert_eq!(por_ruta("lib/vieja.so").respaldo, Some(h(b"libreria de A")));
    assert_eq!(por_ruta("bin/nuevo-de-b").accion, Accion::Crear);
    assert_eq!(por_ruta("bin/nuevo-de-b").respaldo, None);

    // Y queda durable, con las copias dentro.
    assert!(d.puntos.contains_key(&id("A").to_hex()));
    assert_eq!(d.copias.len(), 2);
}

#[test]
fn sin_espacio_no_se_arma() {
    let mut d = disco();
    // Justo para las copias, pero no para poder restaurarlas después.
    d.libre = (b"sosh de A".len() + b"libreria de A".len()) as u64;
    match crear(&mut d) {
        Err(CrearError::SinEspacio { necesita, libre }) => {
            assert!(necesita > libre);
        }
        otro => panic!("esperaba falta de espacio, no {otro:?}"),
    }
    // Y no deja nada a medias: no se copió nada.
    assert!(d.copias.is_empty(), "no debe gastar espacio si ya sabe que no cabe");
    assert!(d.puntos.is_empty());
}

#[test]
fn una_copia_que_falla_impide_armar() {
    let mut d = disco();
    d.fallar_copia_de = Some("lib/vieja.so".into());
    assert_eq!(crear(&mut d), Err(CrearError::Copia("lib/vieja.so".into())));
    assert!(d.puntos.is_empty(), "sin punto durable no hay vuelta atrás que prometer");
}

#[test]
fn una_copia_que_no_se_relee_igual_impide_armar() {
    // Es el caso que justifica releer: la escritura «fue bien» pero lo que hay
    // en disco no es lo que creíamos.
    let mut d = disco();
    d.releer_mal = Some("bin/sosh".into());
    assert_eq!(crear(&mut d), Err(CrearError::Incompleto("bin/sosh".into())));
    assert!(d.puntos.is_empty());
}

#[test]
fn una_copia_que_desaparece_impide_armar() {
    let mut d = disco();
    let r = crear_punto(
        id("A"),
        "0.2.9",
        "abc",
        GUID,
        h(b"kernel de A"),
        &cambios(),
        &mut d,
    );
    assert!(r.is_ok());
    // Ahora se pierde una copia y se vuelve a verificar el punto guardado.
    d.copias.remove(&(id("A").to_hex(), "bin/sosh".into()));
    let p = Punto::parse(d.puntos.get(&id("A").to_hex()).unwrap()).unwrap();
    struct Ver<'a>(&'a mut Disco, TxnId);
    impl soso_update_core::txn::aplicador::Sistema for Ver<'_> {
        fn leer(
            &mut self,
            de: soso_update_core::txn::aplicador::De,
            ruta: &str,
        ) -> Option<Vec<u8>> {
            (de == soso_update_core::txn::aplicador::De::Respaldo)
                .then(|| self.0.releer_copia(self.1, ruta))
                .flatten()
        }
        fn escribir(&mut self, _r: &str, _d: &[u8]) -> Result<(), ()> {
            Ok(())
        }
        fn borrar(&mut self, _r: &str) -> Result<(), ()> {
            Ok(())
        }
        fn guardar_diario(
            &mut self,
            _j: &soso_update_core::txn::journal::Journal,
        ) -> Result<(), ()> {
            Ok(())
        }
    }
    let idp = p.id;
    assert!(p.verificar(&mut Ver(&mut d, idp)).is_err());
}

#[test]
fn si_el_registro_no_queda_durable_no_hay_punto() {
    let mut d = disco();
    d.fallar_registro = true;
    assert_eq!(crear(&mut d), Err(CrearError::Registro));
}

// ── Conservación A → B → C ───────────────────────────────────────────────

#[test]
fn preparar_c_no_destruye_la_vuelta_de_b_a_a() {
    // B activa, con el punto de A para volver. Se arma C, que crea el de B.
    let mut r = Retencion { activo: Some(id("A")), armado: None };
    r.armado = Some(id("B"));
    let refs = r.referencias();
    assert!(refs.contains(&id("A")), "el de A sigue referenciado mientras C no confirme");
    assert!(refs.contains(&id("B")));
    assert_eq!(refs.len(), 2, "durante A→B→C conviven dos a propósito");
}

#[test]
fn al_confirmar_c_se_suelta_el_de_a_y_queda_el_de_b() {
    let mut r = Retencion { activo: Some(id("A")), armado: Some(id("B")) };
    let recogibles = r.al_confirmar(true);
    assert_eq!(recogibles, vec![id("A")]);
    assert_eq!(r.activo, Some(id("B")), "ahora se vuelve a B");
    assert_eq!(r.armado, None);
}

#[test]
fn si_el_punto_nuevo_no_esta_verificado_no_se_suelta_el_viejo() {
    // Quedarse sin ninguna copia recuperable por soltar una que no sabíamos si
    // servía es exactamente lo que no puede pasar.
    let mut r = Retencion { activo: Some(id("A")), armado: Some(id("B")) };
    assert!(r.al_confirmar(false).is_empty());
    assert_eq!(r.activo, Some(id("A")));
    assert_eq!(r.armado, Some(id("B")), "se conservan los dos");
}

#[test]
fn si_c_falla_y_se_revierte_sobra_el_punto_de_b() {
    let mut r = Retencion { activo: Some(id("A")), armado: Some(id("B")) };
    assert_eq!(r.al_revertir(), vec![id("B")]);
    assert_eq!(r.activo, Some(id("A")), "se sigue pudiendo volver a A");
    assert_eq!(r.armado, None);
}

#[test]
fn la_primera_actualizacion_no_tiene_nada_que_soltar() {
    let mut r = Retencion { activo: None, armado: Some(id("A")) };
    assert!(r.al_confirmar(true).is_empty());
    assert_eq!(r.activo, Some(id("A")));
}

#[test]
fn confirmar_dos_veces_no_suelta_el_punto_activo() {
    let mut r = Retencion { activo: Some(id("A")), armado: Some(id("B")) };
    assert_eq!(r.al_confirmar(true), vec![id("A")]);
    // Un segundo arranque confirmado no vuelve a mover nada.
    assert!(r.al_confirmar(true).is_empty());
    assert_eq!(r.activo, Some(id("B")));
}

// ── Limpieza y candidata fallida (U5e) ───────────────────────────────────

#[test]
fn solo_se_recoge_lo_que_nadie_referencia() {
    let todos = [id("A"), id("B"), id("C")];
    let recoger = soso_update_core::txn::punto::a_recoger(&todos, &[id("B")]);
    assert_eq!(recoger, vec![id("A"), id("C")]);
}

#[test]
fn sin_referencias_no_se_recoge_nada() {
    // Quedarse sin ninguna copia recuperable por no saber cuál era la buena es
    // peor que ocupar sitio.
    let todos = [id("A"), id("B")];
    assert!(soso_update_core::txn::punto::a_recoger(&todos, &[]).is_empty());
}

#[test]
fn durante_a_b_c_no_se_recoge_ninguno_de_los_dos() {
    let todos = [id("A"), id("B"), id("viejo")];
    let r = Retencion { activo: Some(id("A")), armado: Some(id("B")) };
    let recoger = soso_update_core::txn::punto::a_recoger(&todos, &r.referencias());
    assert_eq!(recoger, vec![id("viejo")], "A y B se conservan a propósito");
}

#[test]
fn una_candidata_que_ya_fallo_se_reconoce() {
    use soso_update_core::txn::bootrec::Decision;
    use soso_update_core::txn::punto::candidata_fallida;
    // El registro dice que esa misma versión se deshizo: reinstalarla a ciegas
    // es entrar en el bucle de aplicar, fallar y deshacer.
    assert!(candidata_fallida(Decision::Revertido, "0.3.0", "0.3.0"));
    assert!(candidata_fallida(Decision::Rescatar, "0.3.0", "0.3.0"));
    // Otra versión, o un registro que no habla de fallo, no bloquean nada.
    assert!(!candidata_fallida(Decision::Revertido, "0.3.0", "0.3.1"));
    assert!(!candidata_fallida(Decision::Confirmado, "0.3.0", "0.3.0"));
    assert!(!candidata_fallida(Decision::Revertido, "", ""));
}
