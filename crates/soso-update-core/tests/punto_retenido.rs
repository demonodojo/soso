//! U5a: extensión versionada del contrato — punto retenido, decisión de
//! rescate y reserva efectiva.
//!
//! Dos cosas que este banco vigila y que se rompen en silencio: que un registro
//! del formato viejo **siga leyéndose** en vez de darse por inválido, y que un
//! punto que se da por bueno se haya releído de verdad.

use soso_update_core::record::{RecordError, SLOT_SIZE};
use soso_update_core::txn::aplicador::{De, Fallo, Sistema};
use soso_update_core::txn::bootrec::{BootRecord, Decision, BOOTREC_FORMATO};
use soso_update_core::txn::journal::{Accion, Contenido, Entrada, Journal, Progreso};
use soso_update_core::txn::punto::{self, Punto, PuntoError};
use soso_update_core::txn::reconcile::{reconcile, EstadoEsp, EstadoJournal, Motivo, Recuperacion};
use soso_update_core::txn::{TxnId, RESERVA_LOGS, RESERVA_RECUPERACION};
use std::collections::BTreeMap;

const GUID: &str = "1b2c3d4e-0000-4000-8000-aabbccddeeff";

fn id() -> TxnId {
    TxnId::from_manifest(b"version A")
}

fn h(d: &[u8]) -> Contenido {
    Contenido { size: d.len() as u64, hash: soso_update_core::hex_sha256(d) }
}

struct Copias(BTreeMap<String, Vec<u8>>);

impl Sistema for Copias {
    fn leer(&mut self, de: De, ruta: &str) -> Option<Vec<u8>> {
        (de == De::Respaldo).then(|| self.0.get(ruta).cloned()).flatten()
    }
    fn escribir(&mut self, _r: &str, _d: &[u8]) -> Result<(), ()> {
        Ok(())
    }
    fn borrar(&mut self, _r: &str) -> Result<(), ()> {
        Ok(())
    }
    fn guardar_diario(&mut self, _j: &Journal) -> Result<(), ()> {
        Ok(())
    }
}

fn punto_y_copias() -> (Punto, Copias) {
    let mut c = Copias(BTreeMap::new());
    c.0.insert("bin/sosh".into(), b"sosh de A".to_vec());
    c.0.insert("lib/vieja.so".into(), b"libreria de A".to_vec());
    let entradas = vec![
        Entrada {
            accion: Accion::Reemplazar,
            progreso: Progreso::Respaldado,
            path: "bin/sosh".into(),
            nuevo: None,
            respaldo: Some(h(b"sosh de A")),
        },
        // B lo añadió: volver a A significa quitarlo, y por eso no tiene copia.
        Entrada {
            accion: Accion::Crear,
            progreso: Progreso::Respaldado,
            path: "bin/nuevo-de-b".into(),
            nuevo: None,
            respaldo: None,
        },
        Entrada {
            accion: Accion::Borrar,
            progreso: Progreso::Respaldado,
            path: "lib/vieja.so".into(),
            nuevo: None,
            respaldo: Some(h(b"libreria de A")),
        },
    ];
    let p = Punto::nuevo(id(), "0.2.9", "abc123", GUID, h(b"kernel de A"), entradas);
    (p, c)
}

// ── Formato: la extensión no invalida lo anterior ────────────────────────

#[test]
fn un_registro_del_formato_viejo_sigue_leyendose() {
    // Lo escribe una versión anterior: formato 1, sin punto. No puede
    // convertirse en «registro inválido» de un día para otro.
    let cuerpo = format!(
        "decision=probando\nid={}\nversion_nueva=0.3.0\nversion_anterior=0.2.9\ndir={}\n",
        id().to_hex(),
        id().dir()
    );
    let viejo =
        soso_update_core::record::frame_slot("SOSOTXN arranque", 1, 7, &cuerpo, SLOT_SIZE).unwrap();

    let r = BootRecord::parse(&viejo).expect("un formato 1 tiene que seguir leyéndose");
    assert_eq!(r.decision, Decision::Probando);
    assert_eq!(r.formato, 1);
    assert_eq!(r.punto, None);
    assert!(
        !r.conoce_puntos(),
        "un registro viejo no sabe de puntos: «no consta», que no es «no hay»"
    );
}

#[test]
fn un_registro_del_futuro_no_se_acepta_a_medias() {
    let cuerpo = format!("decision=idle\nid={}\n", id().to_hex());
    let futuro = soso_update_core::record::frame_slot(
        "SOSOTXN arranque",
        BOOTREC_FORMATO + 1,
        1,
        &cuerpo,
        SLOT_SIZE,
    )
    .unwrap();
    assert!(matches!(
        BootRecord::parse(&futuro),
        Err(soso_update_core::txn::bootrec::BootRecError::Registro(
            RecordError::FormatoDesconocido(_)
        ))
    ));
}

#[test]
fn el_formato_nuevo_lleva_el_punto_y_sobrevive_al_roundtrip() {
    let r = BootRecord::nuevo(Decision::Confirmado, id(), "0.3.0", "0.2.9", 3).con_punto(id());
    let leido = BootRecord::parse(&r.format().unwrap()).unwrap();
    assert_eq!(leido, r);
    assert_eq!(leido.formato, BOOTREC_FORMATO);
    assert_eq!(leido.punto, Some(id()));
    assert!(leido.conoce_puntos());
}

// ── Decisión de rescate ──────────────────────────────────────────────────

#[test]
fn el_rescate_manda_sobre_lo_que_diga_el_diario() {
    // Quien pide rescate o no puede arrancar o ya confirmó: lo que se restaura
    // es el punto retenido, no la operación en curso.
    let r = BootRecord::nuevo(Decision::Rescatar, id(), "0.3.0", "0.2.9", 9).con_punto(id());
    let esp = EstadoEsp::Registro(r);
    for diario in [EstadoJournal::Ausente, EstadoJournal::Roto] {
        assert_eq!(reconcile(&esp, &diario), Recuperacion::Rescatar(id()));
    }
    let mut j = Journal::nuevo(id(), "0.3.0", "0.2.9");
    j.estado = soso_update_core::txn::TxnState::Confirmado;
    j.kernel_nuevo = Some(h(b"k"));
    j.kernel_anterior = Some(h(b"k0"));
    assert_eq!(
        reconcile(&esp, &EstadoJournal::Diario(j)),
        Recuperacion::Rescatar(id()),
        "también después de confirmar"
    );
}

#[test]
fn pedir_rescate_sin_punto_es_diagnostico() {
    let r = BootRecord::nuevo(Decision::Rescatar, id(), "0.3.0", "0.2.9", 9);
    assert_eq!(
        reconcile(&EstadoEsp::Registro(r), &EstadoJournal::Ausente),
        Recuperacion::Diagnostico(Motivo::RescateSinPunto)
    );
}

// ── El punto ─────────────────────────────────────────────────────────────

#[test]
fn el_punto_sobrevive_al_roundtrip() {
    let (p, _) = punto_y_copias();
    let leido = Punto::parse(&p.format()).unwrap();
    assert_eq!(leido.version, "0.2.9");
    assert_eq!(leido.build, "abc123");
    assert_eq!(leido.guid_destino, GUID);
    assert_eq!(leido.kernel, h(b"kernel de A"));
    assert_eq!(leido.entradas.len(), 3);
    // Lo que añadió B se anota para **quitarlo** al volver, sin copia.
    let quitar = leido.entradas.iter().find(|e| e.path == "bin/nuevo-de-b").unwrap();
    assert_eq!(quitar.accion, Accion::Crear);
    assert_eq!(quitar.respaldo, None);
}

#[test]
fn un_punto_de_otra_instalacion_no_se_aplica() {
    let (p, _) = punto_y_copias();
    assert_eq!(p.comprobar_destino(GUID), Ok(()));
    assert!(matches!(
        p.comprobar_destino("ffffffff-0000-4000-8000-000000000000"),
        Err(PuntoError::OtroDestino { .. })
    ));
    // El GUID no distingue mayúsculas: lo escriben herramientas distintas.
    assert_eq!(p.comprobar_destino(&GUID.to_uppercase()), Ok(()));
}

#[test]
fn verificar_relee_de_verdad() {
    let (p, mut c) = punto_y_copias();
    assert_eq!(p.verificar(&mut c), Ok(()));

    // Falta una pieza.
    let (p, mut c) = punto_y_copias();
    c.0.remove("bin/sosh");
    assert_eq!(p.verificar(&mut c), Err(Fallo::FaltaRespaldo("bin/sosh".into())));

    // Está, pero cambiada: el día que haga falta no serviría.
    let (p, mut c) = punto_y_copias();
    c.0.insert("lib/vieja.so".into(), b"otra cosa".to_vec());
    assert_eq!(p.verificar(&mut c), Err(Fallo::HashDistinto("lib/vieja.so".into())));
}

// ── Reserva efectiva y retención ─────────────────────────────────────────

#[test]
fn la_reserva_efectiva_cuenta_la_copia_en_escritura() {
    let (p, _) = punto_y_copias();
    let datos = p.bytes_a_restaurar();
    assert_eq!(
        datos,
        b"sosh de A".len() as u64 + b"libreria de A".len() as u64 + b"kernel de A".len() as u64
    );
    // La reserva fija de U0 es un mínimo; restaurar con CoW necesita el bloque
    // nuevo antes de soltar el viejo, así que los datos cuentan dos veces.
    assert_eq!(
        punto::reserva_efectiva(&p),
        datos * 2 + RESERVA_LOGS + RESERVA_RECUPERACION
    );
    assert!(punto::reserva_efectiva(&p) > RESERVA_LOGS + RESERVA_RECUPERACION);
}

#[test]
fn un_punto_referenciado_no_se_recoge() {
    let (p, _) = punto_y_copias();
    let otro = TxnId::from_manifest(b"otra");
    assert!(punto::recogible(&p, &[otro]));
    assert!(!punto::recogible(&p, &[otro, id()]));
    // Durante A→B→C conviven dos puntos: mientras algo referencie al de A, se
    // queda, aunque C ya esté en marcha.
    assert!(!punto::recogible(&p, &[id()]));
}

#[test]
fn un_punto_roto_no_pasa_por_bueno() {
    let (p, _) = punto_y_copias();
    let mut bytes = p.format();
    bytes[30] ^= 0x01;
    assert!(matches!(
        Punto::parse(&bytes),
        Err(PuntoError::Registro(RecordError::SumaIncorrecta))
    ));
}
