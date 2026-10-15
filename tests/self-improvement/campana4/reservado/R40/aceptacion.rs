//! Aceptación reservada de R40.

use soso_abi::*;

const EINVAL_NEG: i64 = -22;

#[test]
fn las_combinaciones_de_uso_real_son_validas() {
    assert_eq!(open_flags_validos(O_RDONLY), Ok(()));
    assert_eq!(open_flags_validos(O_WRONLY), Ok(()));
    assert_eq!(open_flags_validos(O_WRONLY | O_CREAT | O_TRUNC), Ok(()));
    assert_eq!(open_flags_validos(O_WRONLY | O_APPEND), Ok(()));
    assert_eq!(open_flags_validos(O_WRONLY | O_CREAT | O_APPEND), Ok(()));
    assert_eq!(open_flags_validos(O_WRONLY | O_CREAT | O_EXCL), Ok(()));
    assert_eq!(open_flags_validos(O_WRONLY | O_CREAT | O_EXCL | O_TRUNC), Ok(()));
}

#[test]
fn los_modificadores_exigen_escritura() {
    for m in [O_APPEND, O_CREAT, O_TRUNC, O_CREAT | O_TRUNC, O_CREAT | O_EXCL] {
        assert_eq!(open_flags_validos(m), Err(EINVAL_NEG), "flags {m:#x}");
    }
}

#[test]
fn excl_exige_creat() {
    assert_eq!(open_flags_validos(O_WRONLY | O_EXCL), Err(EINVAL_NEG));
    assert_eq!(open_flags_validos(O_WRONLY | O_EXCL | O_TRUNC), Err(EINVAL_NEG));
}

#[test]
fn append_y_trunc_se_contradicen() {
    assert_eq!(open_flags_validos(O_WRONLY | O_APPEND | O_TRUNC), Err(EINVAL_NEG));
    assert_eq!(open_flags_validos(O_WRONLY | O_CREAT | O_APPEND | O_TRUNC), Err(EINVAL_NEG));
}

#[test]
fn los_bits_desconocidos_se_rechazan() {
    assert_eq!(open_flags_validos(32), Err(EINVAL_NEG));
    assert_eq!(open_flags_validos(O_WRONLY | 64), Err(EINVAL_NEG));
    assert_eq!(open_flags_validos(1 << 63), Err(EINVAL_NEG));
    assert_eq!(open_flags_validos(u64::MAX), Err(EINVAL_NEG));
}

#[test]
fn la_mascara_cubre_exactamente_los_cinco_flags() {
    assert_eq!(O_FLAGS_MASK, O_WRONLY | O_APPEND | O_CREAT | O_TRUNC | O_EXCL);
    assert_eq!(O_FLAGS_MASK, 0b11111);
}

#[test]
fn exhaustivo_sobre_los_32_valores_bajos() {
    for f in 0u64..64 {
        let w = f & O_WRONLY != 0;
        let ok = f & !0b11111 == 0
            && (w || f & (O_APPEND | O_CREAT | O_TRUNC | O_EXCL) == 0)
            && (f & O_EXCL == 0 || f & O_CREAT != 0)
            && (f & O_APPEND == 0 || f & O_TRUNC == 0);
        assert_eq!(open_flags_validos(f).is_ok(), ok, "flags {f:#x}");
    }
}
