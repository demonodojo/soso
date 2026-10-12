//! Aceptación reservada de R12.

use soso_update_core::mailbox::{Mailbox, MailboxCmd};

const H: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn una_linea_kernel_valida_se_lee() {
    let m = Mailbox::parse(&format!("SOSOUPD v1\nKERNEL 1234 {H} 0.3.1\n"));
    assert_eq!(m.cmd, MailboxCmd::Kernel { size: 1234, hash: H.into(), version: "0.3.1".into() });
    let m = Mailbox::parse(&format!("KERNEL 7 {} 1.0.0", H.to_uppercase()));
    assert!(matches!(m.cmd, MailboxCmd::Kernel { size: 7, .. }));
}

#[test]
fn tamano_no_numerico_no_se_convierte_en_cero() {
    for malo in ["KERNEL abc", "KERNEL -5", "KERNEL 12x", "KERNEL 99999999999999999999999"] {
        let m = Mailbox::parse(&format!("{malo} {H} 0.3.1"));
        assert_eq!(m.cmd, MailboxCmd::Idle, "{malo}");
    }
}

#[test]
fn hash_invalido_se_ignora() {
    for hash in ["abc", &H[..63], &format!("{H}0"), &"g".repeat(64), ""] {
        let m = Mailbox::parse(&format!("KERNEL 10 {hash} 0.3.1"));
        assert_eq!(m.cmd, MailboxCmd::Idle, "{hash:?}");
    }
}

#[test]
fn una_linea_mala_deja_la_orden_anterior() {
    let m = Mailbox::parse(&format!("PROBANDO 0.2.0\nKERNEL abc {H} 0.3.1\n"));
    assert_eq!(m.cmd, MailboxCmd::Probando { version: "0.2.0".into() });
    let m = Mailbox::parse(&format!("KERNEL abc {H} 0.3.1\nOK 0.2.0\n"));
    assert_eq!(m.cmd, MailboxCmd::Ok { version: "0.2.0".into() });
    let m = Mailbox::parse(&format!("KERNEL 5 {H} 0.1.0\nKERNEL zz {H} 0.2.0\n"));
    assert!(matches!(m.cmd, MailboxCmd::Kernel { size: 5, .. }), "la última línea válida gana");
}
