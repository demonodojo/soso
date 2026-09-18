//! U6: qué le falta a una instalación antigua para poder recibir una
//! actualización recuperable, y qué se le puede prometer mientras tanto.

use soso_update_core::migracion::{
    admite_recuperable, diagnosticar, EstadoHueco, Falta, Inventario, HUECOS,
};

fn al_dia() -> Inventario {
    Inventario {
        huecos: vec![EstadoHueco::Listo; HUECOS.len()],
        formato_registro: Some(soso_update_core::txn::bootrec::BOOTREC_FORMATO),
        entrada_rescate: true,
        log_fat: false,
        es_live: false,
    }
}

#[test]
fn una_instalacion_al_dia_no_necesita_nada() {
    assert!(diagnosticar(&al_dia()).is_empty());
}

#[test]
fn sin_registro_de_arranque_no_hay_actualizacion_recuperable() {
    let mut inv = al_dia();
    inv.huecos[0] = EstadoHueco::Ausente; // SOSOTXN.BIN
    let faltas = diagnosticar(&inv);
    assert!(
        !admite_recuperable(&faltas),
        "sin dónde escribir la decisión no se puede prometer vuelta atrás"
    );
    assert!(faltas[0].descripcion().contains("SOSOTXN.BIN"));
}

#[test]
fn un_hueco_del_tamano_equivocado_cuenta_igual_que_no_tenerlo() {
    // El kernel lo localiza por LBA y exige tamaño exacto y clusters
    // consecutivos: uno «casi bien» no se puede usar, y decir que sí sería
    // descubrirlo en el peor momento.
    let mut inv = al_dia();
    inv.huecos[1] = EstadoHueco::Inservible; // SOSOKRN.BIN
    let faltas = diagnosticar(&inv);
    assert!(!admite_recuperable(&faltas));
    assert!(faltas[0].descripcion().contains("no se puede usar"));
}

#[test]
fn un_inventario_vacio_no_se_toma_por_bueno() {
    // Quien no pudo mirar no debe parecer una máquina al día.
    let inv = Inventario::default();
    let faltas = diagnosticar(&inv);
    assert!(!admite_recuperable(&faltas));
}

#[test]
fn lo_que_falta_pero_no_bloquea_se_dice_sin_impedir_la_actualizacion() {
    let mut inv = al_dia();
    inv.entrada_rescate = false;
    inv.log_fat = true;
    inv.formato_registro = Some(1);
    let faltas = diagnosticar(&inv);
    assert_eq!(faltas.len(), 3);
    assert!(
        admite_recuperable(&faltas),
        "la vuelta atrás manual sigue estando; lo que falta son comodidades y \
         la vía de rescate, no la garantía"
    );
    assert!(faltas.iter().any(|f| matches!(f, Falta::EntradaRescate)));
    assert!(faltas.iter().any(|f| matches!(f, Falta::LogEnFat)));
    assert!(faltas
        .iter()
        .any(|f| matches!(f, Falta::RegistroSinPuntos { formato: 1 })));
}

#[test]
fn a_un_live_no_se_le_exige_lo_que_no_le_toca() {
    // Un USB de arranque no necesita entrada UEFI de rescate —él es el
    // rescate— y su SOSOLOG.TXT no sobra: es lo que se lee cuando la máquina
    // no llega a montar nada. Diagnosticárselo sería ruido que enseña a
    // ignorar los avisos.
    let mut inv = al_dia();
    inv.es_live = true;
    inv.entrada_rescate = false;
    inv.log_fat = true;
    assert!(diagnosticar(&inv).is_empty());

    // En una instalación, los mismos datos sí son dos avisos.
    inv.es_live = false;
    assert_eq!(diagnosticar(&inv).len(), 2);
}

#[test]
fn el_modo_opcional_no_bloquea() {
    let mut inv = al_dia();
    let i = HUECOS.iter().position(|h| h.nombre == "SOSOMODE.TXT").unwrap();
    inv.huecos[i] = EstadoHueco::Ausente;
    assert!(admite_recuperable(&diagnosticar(&inv)));
}

#[test]
fn los_huecos_declaran_el_tamano_que_el_kernel_exige() {
    // Si esta lista se desvía de lo que el kernel localiza, la transición
    // provisionaría huecos que después no sirven.
    let de = |n: &str| HUECOS.iter().find(|h| h.nombre == n).unwrap().tamano;
    assert_eq!(de("SOSOTXN.BIN"), soso_update_core::UPD_BOOTREC_SIZE);
    assert_eq!(de("SOSOKRN.BIN"), soso_update_core::UPD_KERNEL_SLOT_SIZE);
    assert_eq!(de("SOSOMODE.TXT"), soso_update_core::UPD_MODE_SIZE);
    assert_eq!(
        de("SOSOKRN.MET"),
        soso_update_core::kernel_meta::KERNEL_META_SIZE
    );
}
