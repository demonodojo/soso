//! Aceptación reservada de R34.

use soso_llm_core::sample::Sampler;

const NAN: f32 = f32::NAN;
const INF: f32 = f32::INFINITY;

#[test]
fn greedy_desempata_por_el_menor_indice() {
    let mut s = Sampler::greedy();
    assert_eq!(s.sample(&[1.0, 5.0, 5.0, 2.0]), 1);
    assert_eq!(s.sample(&[3.0, 3.0, 3.0]), 0);
    assert_eq!(s.sample(&[0.0, 1.0, 9.0, 9.0, 9.0]), 2);
    assert_eq!(s.sample(&[-INF, -2.0, -2.0]), 1);
}

#[test]
fn greedy_ignora_los_nan() {
    let mut s = Sampler::greedy();
    assert_eq!(s.sample(&[NAN, 1.0, 3.0, NAN]), 2);
    assert_eq!(s.sample(&[NAN, NAN, 0.5]), 2);
    assert_eq!(s.sample(&[2.0, NAN, 7.0, 7.0]), 2);
    assert_eq!(s.sample(&[NAN, -INF, -INF]), 1);
    assert_eq!(s.sample(&[NAN, NAN, NAN]), 0);
    assert_eq!(s.sample(&[-INF, -INF]), 0);
    assert_eq!(s.sample(&[1.0, INF, 2.0, INF]), 1);
    assert_eq!(s.sample(&[4.0]), 0);
}

#[test]
fn con_temperatura_un_nan_nunca_sale() {
    let mut s = Sampler::new(1.0, 1.0, 99);
    let logits = [NAN, 2.0, NAN, 2.0, -3.0];
    let mut vistos = [0usize; 5];
    for _ in 0..400 {
        let t = s.sample(&logits) as usize;
        assert!(t != 0 && t != 2, "salió un NaN: {t}");
        vistos[t] += 1;
    }
    assert!(vistos[1] > 50 && vistos[3] > 50, "{vistos:?}");
}

#[test]
fn con_temperatura_un_nan_al_final_o_al_principio_tampoco() {
    for logits in [[1.0f32, 1.5, 0.5, NAN], [NAN, 1.0, 1.5, 0.5]] {
        let mut s = Sampler::new(0.7, 0.9, 5);
        for _ in 0..300 {
            let t = s.sample(&logits) as usize;
            assert!(logits[t].is_finite(), "salió {t} de {logits:?}");
        }
    }
}

#[test]
fn sin_distribucion_se_comporta_como_greedy() {
    let mut s = Sampler::new(1.0, 0.9, 3);
    for _ in 0..20 {
        assert_eq!(s.sample(&[NAN, NAN, NAN]), 0);
        assert_eq!(s.sample(&[-INF, -INF, -INF]), 0);
        assert_eq!(s.sample(&[NAN, -INF, -INF]), 1);
        assert_eq!(s.sample(&[1.0, INF, 2.0]), 1);
        assert_eq!(s.sample(&[INF, 3.0, INF]), 0);
    }
}

#[test]
fn menos_infinito_pesa_cero_y_lo_normal_no_cambia() {
    let mut s = Sampler::new(1.0, 1.0, 11);
    for _ in 0..100 {
        assert_eq!(s.sample(&[-INF, 3.0, -INF]), 1);
    }
    let logits: Vec<f32> = (0..64).map(|i| (i % 7) as f32 * 0.4).collect();
    let mut a = Sampler::new(0.9, 0.95, 77);
    let mut b = Sampler::new(0.9, 0.95, 77);
    for _ in 0..50 {
        assert_eq!(a.sample(&logits), b.sample(&logits));
    }
    let mut c = Sampler::new(1.0, 0.01, 8);
    assert_eq!(c.sample(&[0.0, 10.0, 0.0, 1.0]), 1);
}
