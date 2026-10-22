//! Política del borrador: cuánto verificar según lo que cuesta cada ronda y lo que acierta el borrador.

use soso_llm_core::especulativo::{PoliticaBorrador, MUESTRAS_MIN};

/// Política con `muestras` por longitud: `(n, ms)` repetido `veces`.
fn con_costes(curva: &[(usize, u64)], veces: usize) -> PoliticaBorrador {
    let mut p = PoliticaBorrador::new();
    for _ in 0..veces {
        for &(n, ms) in curva {
            p.registrar_coste(n, ms);
        }
    }
    p
}

/// Rondas en las que el borrador acierta `aceptados` de `ofrecidos`.
fn con_aciertos(mut p: PoliticaBorrador, ofrecidos: usize, aceptados: usize, rondas: usize) -> PoliticaBorrador {
    for _ in 0..rondas {
        p.registrar_ronda(ofrecidos, aceptados);
    }
    p
}

#[test]
fn sin_datos_suficientes_no_opina() {
    let mut p = PoliticaBorrador::new();
    assert!(!p.lista());
    assert_eq!(p.elegir_k(8), 8);
    for _ in 0..MUESTRAS_MIN - 1 {
        p.registrar_coste(1, 40);
    }
    assert!(!p.lista());
    assert_eq!(p.elegir_k(5), 5);
    p.registrar_coste(1, 40);
    assert!(p.lista());
}

#[test]
fn con_curva_plana_elige_el_borrador_entero() {
    // El forward cuesta lo mismo con 1 que con 8 posiciones (lectura de pesos dominante).
    let curva: Vec<(usize, u64)> = (1..=8).map(|n| (n, 100)).collect();
    let mut p = con_aciertos(con_costes(&curva, 3), 7, 7, 4);
    for _ in 0..10 {
        assert_eq!(p.elegir_k(7), 7);
    }
}

#[test]
fn con_curva_empinada_y_aciertos_bajos_no_especula() {
    // Cada posición extra cuesta casi lo que un paso entero y el borrador casi nunca acierta.
    let curva: Vec<(usize, u64)> = (1..=8).map(|n| (n, 100 * n as u64)).collect();
    let mut p = con_aciertos(con_costes(&curva, 3), 7, 0, 30);
    // Las rondas de exploración (una de cada 16) piden el borrador entero; el resto, ninguno.
    let k: Vec<usize> = (0..15).map(|_| p.elegir_k(7)).collect();
    assert!(k.iter().all(|&k| k == 0), "{k:?}");
}

#[test]
fn con_curva_empinada_y_aciertos_altos_si_especula() {
    // Con coste lineal, cada posición extra sólo compensa si acierta bastante más que el coste relativo.
    let curva: Vec<(usize, u64)> = (1..=8).map(|n| (n, 60 + 40 * n as u64)).collect();
    let mut p = con_aciertos(con_costes(&curva, 3), 7, 7, 60);
    assert!(p.elegir_k(7) > 0);
}

#[test]
fn la_exploracion_mantiene_viva_la_aceptacion() {
    let curva: Vec<(usize, u64)> = (1..=8).map(|n| (n, 100 * n as u64)).collect();
    let mut p = con_aciertos(con_costes(&curva, 3), 7, 0, 30);
    let ks: Vec<usize> = (0..32).map(|_| p.elegir_k(7)).collect();
    assert_eq!(ks.iter().filter(|&&k| k == 7).count(), 2, "{ks:?}");
}

#[test]
fn la_aceptacion_es_condicional_por_posicion() {
    let mut p = PoliticaBorrador::new();
    // 4 rondas: se aceptan 2 de 4 ofrecidos. Se comprobaron las posiciones 1, 2 y 3; la 4 no llegó a probarse.
    for _ in 0..4 {
        p.registrar_ronda(4, 2);
    }
    assert!(p.aceptacion(1) > p.aceptacion(3), "la 3 falló siempre y la 1 acertó siempre");
    // La posición 4 no se probó nunca: queda en el prior.
    assert!((p.aceptacion(4) - 0.5).abs() < 1e-6);
    // Más aciertos seguidos suben la esperanza de tokens por ronda.
    let mut q = PoliticaBorrador::new();
    for _ in 0..4 {
        q.registrar_ronda(4, 4);
    }
    assert!(q.tokens_esperados(4) > p.tokens_esperados(4));
}

#[test]
fn el_coste_interpola_entre_las_longitudes_medidas() {
    let p = con_costes(&[(1, 100), (5, 140)], 3);
    assert!((p.coste(1) - 100.0).abs() < 1e-3);
    assert!((p.coste(3) - 120.0).abs() < 1e-3);
    assert!((p.coste(5) - 140.0).abs() < 1e-3);
    // Más allá de lo medido sigue la pendiente (nunca baja).
    assert!(p.coste(9) > 140.0);
}

#[test]
fn la_mediana_aguanta_un_pico_aislado() {
    let mut p = PoliticaBorrador::new();
    for ms in [100, 100, 5000, 100, 100] {
        p.registrar_coste(2, ms);
    }
    assert!((p.coste(2) - 100.0).abs() < 1e-3);
}

#[test]
fn es_determinista() {
    let curva: Vec<(usize, u64)> = (1..=8).map(|n| (n, 90 + 15 * n as u64)).collect();
    let mut a = con_aciertos(con_costes(&curva, 3), 7, 3, 20);
    let mut b = con_aciertos(con_costes(&curva, 3), 7, 3, 20);
    let sa: Vec<usize> = (0..40).map(|_| a.elegir_k(7)).collect();
    let sb: Vec<usize> = (0..40).map(|_| b.elegir_k(7)).collect();
    assert_eq!(sa, sb);
}

#[test]
fn nunca_propone_mas_de_lo_disponible() {
    let curva: Vec<(usize, u64)> = (1..=8).map(|n| (n, 100)).collect();
    let mut p = con_aciertos(con_costes(&curva, 3), 7, 7, 5);
    for d in 0..=7 {
        assert!(p.elegir_k(d) <= d);
    }
}
