//! U5d: la entrada UEFI de rescate. Lo que el shim decide con lo único que
//! tiene delante —el registro de arranque de la ESP— sin sosofs ni red.

use soso_update_core::txn::bootrec::{BootRecord, Decision, BOOTREC_FORMATO};
use soso_update_core::txn::rescate::{es_senal, planear, Plan, Sin, SENAL};
use soso_update_core::txn::TxnId;

fn id(n: u8) -> TxnId {
    TxnId::from_manifest(&[n; 8])
}

fn confirmado() -> BootRecord {
    BootRecord::nuevo(Decision::Confirmado, id(1), "0.3.0", "0.2.0", 7).con_punto(id(1))
}

#[test]
fn la_senal_se_reconoce_venga_como_venga() {
    assert!(es_senal(SENAL));
    assert!(es_senal("rescatar\0"));
    assert!(es_senal("RESCATAR"));
    assert!(es_senal("  rescatar \0\0"));
    assert!(!es_senal(""));
    assert!(!es_senal("rescate"));
    // Nada de coincidencias parciales: una entrada que diga otra cosa no debe
    // disparar una vuelta atrás.
    assert!(!es_senal("norescatar"));
}

#[test]
fn desde_confirmado_se_pide_el_rescate_al_punto_retenido() {
    let Plan::Pedir {
        registro,
        desde,
        hasta,
    } = planear(&confirmado())
    else {
        panic!("debería pedirse");
    };
    assert_eq!(desde, "0.3.0", "se sale de la que está corriendo");
    assert_eq!(hasta, "0.2.0");
    assert_eq!(registro.decision, Decision::Rescatar);
    assert_eq!(registro.punto, Some(id(1)));
    // La secuencia avanza: si no, el registro nuevo no gana al que ya había.
    assert_eq!(registro.seq, 8);
    assert!(registro.ranura() != confirmado().ranura());
}

#[test]
fn sin_punto_no_se_promete_nada() {
    let rec = BootRecord::nuevo(Decision::Confirmado, id(1), "0.3.0", "0.2.0", 3);
    assert_eq!(planear(&rec), Plan::No(Sin::SinPunto));
}

#[test]
fn un_registro_de_formato_1_es_no_consta_no_es_no_hay() {
    let mut rec = confirmado();
    rec.formato = 1;
    rec.punto = None;
    assert_eq!(
        planear(&rec),
        Plan::No(Sin::FormatoAntiguo),
        "un formato viejo no sabe de puntos; decir «no hay» sería inventárselo"
    );
    assert!(BOOTREC_FORMATO >= 2);
}

#[test]
fn pedirlo_dos_veces_no_gasta_otra_secuencia() {
    let rec = BootRecord::nuevo(Decision::Rescatar, id(1), "0.3.0", "0.2.0", 9).con_punto(id(1));
    assert_eq!(
        planear(&rec),
        Plan::YaPedido {
            hasta: "0.2.0".into()
        }
    );
}

#[test]
fn tras_restaurar_no_se_vuelve_a_restaurar() {
    for d in [Decision::Revertido, Decision::RestauradoAPrueba] {
        let rec = BootRecord::nuevo(d, id(1), "0.3.0", "0.2.0", 9).con_punto(id(1));
        assert_eq!(
            planear(&rec),
            Plan::No(Sin::YaRestaurado {
                version: "0.2.0".into()
            }),
            "{d:?}: el punto lleva justo a donde ya estamos"
        );
    }
}

#[test]
fn con_una_operacion_armada_el_rescate_sale_de_la_anterior() {
    // Armada pero sin aplicar: lo que corre sigue siendo la vieja, y el punto
    // es el de esta operación.
    let rec = BootRecord::nuevo(Decision::Armado, id(2), "0.4.0", "0.3.0", 2).con_punto(id(2));
    let Plan::Pedir { desde, hasta, .. } = planear(&rec) else {
        panic!("debería pedirse");
    };
    assert_eq!(desde, "0.3.0");
    assert_eq!(hasta, "0.3.0");
}

#[test]
fn el_registro_pedido_se_relee_igual() {
    let Plan::Pedir { registro, .. } = planear(&confirmado()) else {
        panic!("debería pedirse");
    };
    let bytes = registro.format().expect("formatear");
    let leido = BootRecord::parse(&bytes).expect("releer");
    assert_eq!(leido, registro);
}

// --- cierre del diario tras el rescate ------------------------------------

use soso_update_core::txn::journal::Journal;
use soso_update_core::txn::rescate::cerrar_diario;
use soso_update_core::txn::TxnState;

fn diario(estado: TxnState) -> Journal {
    let mut j = Journal::nuevo(id(1), "0.3.0", "0.2.0");
    j.estado = estado;
    j
}

#[test]
fn el_rescate_cierra_lo_que_llego_a_aplicarse() {
    for estado in [
        TxnState::Armado,
        TxnState::Aplicando,
        TxnState::Probando,
        TxnState::Confirmado,
        TxnState::Revirtiendo,
    ] {
        let mut j = diario(estado);
        assert!(cerrar_diario(&mut j), "{estado:?} tenía que cerrarse");
        assert_eq!(
            j.estado,
            TxnState::Revertido,
            "{estado:?}: el punto puso de vuelta los ficheros de antes"
        );
    }
}

#[test]
fn lo_que_nunca_se_aplico_se_descarta() {
    for estado in [TxnState::Descargando, TxnState::Preparado] {
        let mut j = diario(estado);
        assert!(cerrar_diario(&mut j));
        assert_eq!(
            j.estado,
            TxnState::Descartado,
            "{estado:?}: su preparación ya no sirve para el sistema restaurado"
        );
    }
}

#[test]
fn repetir_el_rescate_no_mueve_un_diario_ya_cerrado() {
    for estado in [TxnState::Revertido, TxnState::Descartado] {
        let mut j = diario(estado);
        let seq = j.seq;
        assert!(!cerrar_diario(&mut j));
        assert_eq!(j.estado, estado);
        assert_eq!(j.seq, seq, "un rescate repetido no gasta secuencia");
    }
}

#[test]
fn tras_cerrarlo_la_pareja_ya_no_es_imposible() {
    use soso_update_core::txn::reconcile::{reconcile, EstadoEsp, EstadoJournal, Recuperacion};
    // La secuencia real: rescate, restauración, y el arranque siguiente ya
    // acreditado. Antes de cerrar el diario esto era `ParejaImposible`.
    let mut j = diario(TxnState::Probando);
    cerrar_diario(&mut j);
    let rec = BootRecord::nuevo(Decision::Revertido, id(1), "0.3.0", "0.2.0", 6).con_punto(id(1));
    assert_eq!(
        reconcile(&EstadoEsp::Registro(rec), &EstadoJournal::Diario(j)),
        Recuperacion::Normal
    );
}
