use soso_log_core::sectores::{por_cada_tramo_cambiado, tramos_cambiados, Tramo};

#[test]
fn sin_cambios() {
    let a = [0u8; 1024];
    let b = a;
    assert!(tramos_cambiados(&a, &b, 512).is_empty());
}

#[test]
fn un_sector() {
    let mut a = [0u8; 1024];
    let mut b = a;
    b[600] = 1;
    assert_eq!(
        tramos_cambiados(&a, &b, 512),
        vec![Tramo {
            offset: 512,
            len: 512
        }]
    );
}

#[test]
fn cabecera_y_cuerpo() {
    let mut a = [0u8; 2048];
    let mut b = a;
    b[10] = 1;
    b[1500] = 2;
    let v = tramos_cambiados(&a, &b, 512);
    assert_eq!(
        v,
        vec![
            Tramo {
                offset: 0,
                len: 512
            },
            Tramo {
                offset: 1024,
                len: 512
            }
        ]
    );
}

#[test]
fn callback_agrega() {
    let mut a = [0u8; 512];
    let mut b = a;
    b[0] = 9;
    let mut n = 0;
    por_cada_tramo_cambiado(&a, &b, 512, |_| n += 1);
    assert_eq!(n, 1);
}
