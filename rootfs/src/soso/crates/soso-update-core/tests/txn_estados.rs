//! U0: máquina de estados de la transacción y comprobación previa de capacidad.

use soso_update_core::txn::{
    preflight, Capacidad, Necesidad, PreflightError, TxnEvent, TxnState, RESERVA_LOGS,
    RESERVA_RECUPERACION,
};

fn camino(inicial: TxnState, eventos: &[TxnEvent]) -> TxnState {
    let mut s = inicial;
    for ev in eventos {
        s = s.next(*ev).unwrap_or_else(|e| panic!("transición inválida: {e:?}"));
    }
    s
}

#[test]
fn camino_feliz_hasta_confirmado() {
    let s = camino(
        TxnState::Descargando,
        &[
            TxnEvent::DescargaVerificada,
            TxnEvent::RespaldoDurable,
            TxnEvent::RegistroArranquePublicado,
            TxnEvent::AplicacionIniciada,
            TxnEvent::AplicacionCompleta,
            TxnEvent::PruebaSuperada,
        ],
    );
    assert_eq!(s, TxnState::Confirmado);
    assert!(s.recogible());
}

#[test]
fn prueba_fallida_lleva_a_revertido() {
    let s = camino(
        TxnState::Probando,
        &[TxnEvent::PruebaFallida, TxnEvent::ReversionCompleta],
    );
    assert_eq!(s, TxnState::Revertido);
}

#[test]
fn reversion_manual_desde_confirmado() {
    let s = camino(
        TxnState::Confirmado,
        &[TxnEvent::ReversionSolicitada, TxnEvent::ReversionCompleta],
    );
    assert_eq!(s, TxnState::Revertido);
}

#[test]
fn aplicacion_es_reentrante() {
    // Un corte a mitad de la aplicación vuelve a entrar en APLICANDO sin
    // saltarse la fase: la idempotencia la da el progreso por entrada.
    let s = camino(
        TxnState::Armado,
        &[
            TxnEvent::AplicacionIniciada,
            TxnEvent::AplicacionIniciada,
            TxnEvent::AplicacionIniciada,
        ],
    );
    assert_eq!(s, TxnState::Aplicando);
    assert!(s.mezcla_posible());
    assert!(!s.sistema_intacto());
}

#[test]
fn armado_sin_aplicar_se_deshace_sin_restaurar_nada() {
    // Cancelar una operación armada pero nunca aplicada no toca el sistema.
    assert_eq!(
        TxnState::Armado.next(TxnEvent::ReversionSolicitada).unwrap(),
        TxnState::Revertido
    );
    assert!(TxnState::Armado.sistema_intacto());
}

#[test]
fn abortar_solo_vale_antes_de_armar() {
    assert_eq!(
        TxnState::Descargando.next(TxnEvent::Abortada).unwrap(),
        TxnState::Descartado
    );
    assert_eq!(
        TxnState::Preparado.next(TxnEvent::Abortada).unwrap(),
        TxnState::Descartado
    );
    for s in [
        TxnState::Armado,
        TxnState::Aplicando,
        TxnState::Probando,
        TxnState::Confirmado,
    ] {
        assert!(
            s.next(TxnEvent::Abortada).is_err(),
            "{s:?} no puede abortarse: hay que revertir"
        );
    }
}

#[test]
fn transiciones_prohibidas() {
    // Saltarse el respaldo o la prueba son los dos atajos que rompen la pareja.
    assert!(TxnState::Descargando.next(TxnEvent::RegistroArranquePublicado).is_err());
    assert!(TxnState::Descargando.next(TxnEvent::AplicacionIniciada).is_err());
    assert!(TxnState::Armado.next(TxnEvent::PruebaSuperada).is_err());
    assert!(TxnState::Aplicando.next(TxnEvent::PruebaSuperada).is_err());
    assert!(TxnState::Revertido.next(TxnEvent::AplicacionIniciada).is_err());
    assert!(TxnState::Confirmado.next(TxnEvent::AplicacionCompleta).is_err());
}

#[test]
fn estados_que_exigen_respaldo() {
    for s in [
        TxnState::Armado,
        TxnState::Aplicando,
        TxnState::Probando,
        TxnState::Confirmado,
        TxnState::Revirtiendo,
        TxnState::Revertido,
    ] {
        assert!(s.exige_respaldos(), "{s:?}");
    }
    for s in [TxnState::Descargando, TxnState::Preparado, TxnState::Descartado] {
        assert!(!s.exige_respaldos(), "{s:?}");
    }
}

#[test]
fn estados_roundtrip_texto() {
    for s in [
        TxnState::Descargando,
        TxnState::Preparado,
        TxnState::Armado,
        TxnState::Aplicando,
        TxnState::Probando,
        TxnState::Confirmado,
        TxnState::Revirtiendo,
        TxnState::Revertido,
        TxnState::Descartado,
    ] {
        assert_eq!(TxnState::parse(s.as_str()), Some(s));
    }
    assert_eq!(TxnState::parse("confirmadísimo"), None);
}

fn capacidad_holgada() -> Capacidad {
    Capacidad {
        sosofs_libre: 1024 * 1024 * 1024,
        hueco_kernel: 64 * 1024 * 1024,
        registro_arranque: soso_update_core::UPD_BOOTREC_SIZE,
        meta_kernel: soso_update_core::UPD_KERNEL_META_SIZE,
    }
}

fn necesidad_tipica() -> Necesidad {
    Necesidad {
        preparacion: 40 * 1024 * 1024,
        respaldo: 30 * 1024 * 1024,
        crecimiento: 5 * 1024 * 1024,
        kernel: 12 * 1024 * 1024,
    }
}

#[test]
fn preflight_acepta_caso_normal() {
    assert_eq!(preflight(&necesidad_tipica(), &capacidad_holgada()), Ok(()));
}

#[test]
fn preflight_cuenta_respaldo_y_reservas() {
    let n = necesidad_tipica();
    // Justo el espacio de los artefactos: falta el respaldo y las reservas.
    let mut c = capacidad_holgada();
    c.sosofs_libre = n.preparacion + n.crecimiento;
    let err = preflight(&n, &c).unwrap_err();
    let PreflightError::SinEspacio { necesita, libre } = err else {
        panic!("esperaba falta de espacio, no {err:?}");
    };
    assert_eq!(libre, c.sosofs_libre);
    assert_eq!(
        necesita,
        n.preparacion + n.respaldo + n.crecimiento + RESERVA_LOGS + RESERVA_RECUPERACION
    );

    // Con el respaldo y las reservas cabe.
    c.sosofs_libre = necesita;
    assert_eq!(preflight(&n, &c), Ok(()));
    c.sosofs_libre = necesita - 1;
    assert!(matches!(preflight(&n, &c), Err(PreflightError::SinEspacio { .. })));
}

#[test]
fn preflight_rechaza_kernel_mayor_que_el_hueco() {
    let mut n = necesidad_tipica();
    n.kernel = 64 * 1024 * 1024 + 1;
    assert!(matches!(
        preflight(&n, &capacidad_holgada()),
        Err(PreflightError::KernelNoCabe { .. })
    ));
    // Un kernel de tamaño cero tampoco es una pareja aplicable.
    n.kernel = 0;
    assert!(matches!(
        preflight(&n, &capacidad_holgada()),
        Err(PreflightError::KernelNoCabe { .. })
    ));
}

#[test]
fn preflight_rechaza_huecos_esp_insuficientes() {
    let n = necesidad_tipica();
    let mut c = capacidad_holgada();
    c.registro_arranque = 4;
    assert!(matches!(
        preflight(&n, &c),
        Err(PreflightError::RegistroNoCabe { size: 4 })
    ));
    let mut c = capacidad_holgada();
    c.meta_kernel = 0;
    assert!(matches!(preflight(&n, &c), Err(PreflightError::RegistroNoCabe { .. })));
}

#[test]
fn preflight_no_desborda_con_necesidades_absurdas() {
    let n = Necesidad {
        preparacion: u64::MAX,
        respaldo: u64::MAX,
        crecimiento: u64::MAX,
        kernel: 1,
    };
    assert!(matches!(
        preflight(&n, &capacidad_holgada()),
        Err(PreflightError::SinEspacio { necesita: u64::MAX, .. })
    ));
}
