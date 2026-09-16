//! U0: identidad live/instalado y contrato de compatibilidad de la release.

use soso_update_core::compat::{exigir, Compat, CompatError, Equipo};
use soso_update_core::identity::{resolver, BootMode, Identidad, ModeRecord, MODE_SIZE};
use soso_update_core::manifest::{Manifest, MANIFEST_MAGIC};
use soso_update_core::record::SLOT_SIZE;

const GUID: &str = "1b2c3d4e-0000-4000-8000-aabbccddeeff";

fn fichero(rec: &ModeRecord) -> Vec<u8> {
    let mut f = vec![0u8; MODE_SIZE];
    let off = rec.ranura() * SLOT_SIZE;
    f[off..off + SLOT_SIZE].copy_from_slice(&rec.format().unwrap());
    f
}

// ── Identidad ────────────────────────────────────────────────────────────

#[test]
fn instalacion_declarada_apaga_el_log_fat() {
    let rec = ModeRecord::nuevo(BootMode::Installed, "2026-09-16", GUID, 1);
    let ident = resolver(Some(&fichero(&rec)), GUID);
    assert_eq!(ident.modo(), BootMode::Installed);
    assert!(!ident.modo().usa_fatlog());
    assert!(ident.puede_retirar_fatlog());
}

#[test]
fn live_declarado_conserva_su_log() {
    let rec = ModeRecord::nuevo(BootMode::Live, "", GUID, 1);
    let ident = resolver(Some(&fichero(&rec)), GUID);
    assert_eq!(ident.modo(), BootMode::Live);
    assert!(ident.modo().usa_fatlog());
    assert!(!ident.puede_retirar_fatlog());
}

#[test]
fn sin_registro_se_comporta_como_hasta_ahora() {
    // Sticks e instalaciones anteriores a U2: nada que deducir, se mantiene el
    // comportamiento actual hasta que la migración U6 escriba el registro.
    assert_eq!(resolver(None, GUID), Identidad::Heredada);
    assert_eq!(resolver(None, GUID).modo(), BootMode::Live);
    assert!(!resolver(None, GUID).puede_retirar_fatlog());
    assert_eq!(resolver(Some(&vec![0u8; MODE_SIZE]), GUID), Identidad::Heredada);
}

#[test]
fn registro_roto_no_se_adivina() {
    let rec = ModeRecord::nuevo(BootMode::Installed, "2026-09-16", GUID, 1);
    let mut f = fichero(&rec);
    f[rec.ranura() * SLOT_SIZE + 30] ^= 0x01;
    let ident = resolver(Some(&f), GUID);
    assert!(matches!(ident, Identidad::Rota(_)));
    assert!(!ident.puede_retirar_fatlog());
    assert!(ident.modo().usa_fatlog(), "ante la duda, no se retira el log");
}

#[test]
fn clon_sin_finalizar_se_detecta_por_el_guid() {
    // `soso-install` clona la ESP y genera GUID nuevos. Un registro con el
    // GUID del origen es una copia a la que le falta la finalización de U2.
    let rec = ModeRecord::nuevo(BootMode::Installed, "2026-09-16", GUID, 1);
    let ident = resolver(Some(&fichero(&rec)), "ffffffff-0000-4000-8000-000000000000");
    assert!(matches!(ident, Identidad::Ajena(_)));
    assert!(!ident.puede_retirar_fatlog());
}

#[test]
fn el_guid_no_distingue_mayusculas() {
    let rec = ModeRecord::nuevo(BootMode::Installed, "2026-09-16", &GUID.to_uppercase(), 1);
    assert!(matches!(resolver(Some(&fichero(&rec)), GUID), Identidad::Explicita(_)));
}

// ── Compatibilidad ───────────────────────────────────────────────────────

fn release() -> Compat {
    Compat {
        arch: "x86_64".into(),
        perfil: "live-usb".into(),
        drivers: vec!["nvme".into(), "iwlwifi".into()],
        abi: 3,
        fs: "sosofs1".into(),
        min_shim: 1,
        min_recuperador: 1,
    }
}

fn equipo() -> Equipo {
    Equipo {
        arch: "x86_64".into(),
        abi: 3,
        fs: "sosofs1".into(),
        shim: 1,
        recuperador: 1,
        drivers: vec!["nvme".into(), "iwlwifi".into(), "e1000e".into()],
    }
}

#[test]
fn release_compatible() {
    assert_eq!(exigir(Some(&release()), &equipo()), Ok(()));
}

#[test]
fn sin_declaracion_no_se_aplica() {
    assert_eq!(exigir(None, &equipo()), Err(CompatError::NoDeclarada));
}

#[test]
fn abi_fs_y_arch_distintos_se_rechazan() {
    let mut eq = equipo();
    eq.abi = 2;
    assert_eq!(
        release().check(&eq),
        Err(CompatError::Abi { espera: 3, hay: 2 })
    );
    let mut eq = equipo();
    eq.fs = "sosofs2".into();
    assert!(matches!(release().check(&eq), Err(CompatError::Fs { .. })));
    let mut eq = equipo();
    eq.arch = "aarch64".into();
    assert!(matches!(release().check(&eq), Err(CompatError::Arch { .. })));
}

#[test]
fn shim_o_recuperador_antiguos_exigen_transicion() {
    // Este es el caso de U6: no se arregla sobrescribiendo ficheros.
    let mut c = release();
    c.min_shim = 2;
    assert_eq!(
        c.check(&equipo()),
        Err(CompatError::ShimAntiguo { min: 2, hay: 1 })
    );
    let mut c = release();
    c.min_recuperador = 4;
    assert_eq!(
        c.check(&equipo()),
        Err(CompatError::RecuperadorAntiguo { min: 4, hay: 1 })
    );
}

#[test]
fn falta_el_driver_del_disco_o_de_la_red() {
    let mut eq = equipo();
    eq.drivers.retain(|d| d != "nvme");
    assert_eq!(
        release().check(&eq),
        Err(CompatError::DriverAusente("nvme".into()))
    );
}

#[test]
fn el_manifiesto_lleva_la_declaracion() {
    let texto = format!(
        "{MANIFEST_MAGIC}\nversion=0.3.0\nbuild=abc\nfecha=2026-09-16\n{}\
         kernel {} 100\npack {} 200\nf {} 0 10 bin/init\n",
        release().format(),
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
    );
    let m = Manifest::parse(&texto).unwrap();
    assert_eq!(m.compat.as_ref(), Some(&release()));
    assert!(m.validate().is_ok());
    // Y sobrevive al roundtrip del propio manifiesto.
    let m2 = Manifest::parse(&m.format()).unwrap();
    assert_eq!(m2.compat, m.compat);
}

#[test]
fn un_manifiesto_antiguo_sigue_leyendose_pero_no_pasa_la_comprobacion() {
    let texto = format!(
        "{MANIFEST_MAGIC}\nversion=0.2.9\nbuild=abc\nfecha=2026-01-01\n\
         kernel {} 100\npack {} 200\nf {} 0 10 bin/init\n",
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
    );
    let m = Manifest::parse(&texto).unwrap();
    assert_eq!(m.compat, None);
    assert_eq!(
        exigir(m.compat.as_ref(), &equipo()),
        Err(CompatError::NoDeclarada)
    );
}
