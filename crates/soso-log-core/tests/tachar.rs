use soso_log_core::Ring;

#[test]
fn tachar_psk_en_linea() {
    let mut r = Ring::<256>::new();
    r.append(b"wifi connect Rutilo puentelasira\n");
    r.tachar(b"puentelasira");
    let mut out = [0u8; 64];
    let n = r.copy_from(0, &mut out);
    let s = core::str::from_utf8(&out[..n]).unwrap();
    assert!(s.contains("************"));
    assert!(!s.contains("puentelasira"));
}
