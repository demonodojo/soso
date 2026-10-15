//! Aceptación reservada de R30.

use soso_abi::{exit_by_signal, signal_de_salida, SIGINT, SIGKILL, SIGTERM};

const _: u8 = exit_by_signal(9); // tiene que seguir siendo const fn

#[test]
fn exit_by_signal_normal_no_cambia() {
    assert_eq!(exit_by_signal(0), 128);
    assert_eq!(exit_by_signal(SIGINT as u8), 130);
    assert_eq!(exit_by_signal(SIGKILL as u8), 137);
    assert_eq!(exit_by_signal(SIGTERM as u8), 143);
    assert_eq!(exit_by_signal(127), 255);
}

#[test]
fn exit_by_signal_satura_en_vez_de_desbordar() {
    assert_eq!(exit_by_signal(128), 255);
    assert_eq!(exit_by_signal(200), 255);
    assert_eq!(exit_by_signal(255), 255);
}

#[test]
fn signal_de_salida_salidas_normales_son_none() {
    for c in 0u8..=128 {
        assert_eq!(signal_de_salida(c), None, "{c}");
    }
}

#[test]
fn signal_de_salida_resta_128() {
    assert_eq!(signal_de_salida(129), Some(1));
    assert_eq!(signal_de_salida(130), Some(2));
    assert_eq!(signal_de_salida(137), Some(9));
    assert_eq!(signal_de_salida(255), Some(127));
}

#[test]
fn ida_y_vuelta() {
    for sig in 1u8..=127 {
        assert_eq!(signal_de_salida(exit_by_signal(sig)), Some(sig), "{sig}");
    }
}
