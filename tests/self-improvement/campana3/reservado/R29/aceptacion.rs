//! Aceptación reservada de R29.

use soso_llm_core::tier::{Tier, TierManager};

fn v(n: usize, b: u8) -> Vec<u8> {
    vec![b; n]
}

#[test]
fn reemplazar_no_expulsa_a_los_ajenos_si_cabe() {
    let mut t = TierManager::new(120, 0);
    t.promote("b", v(30, 1), Tier::Ram).unwrap();
    t.promote("a", v(60, 2), Tier::Ram).unwrap();
    // a pasa de 60 a 80: 30 + 80 = 110 <= 120, b debe sobrevivir
    t.promote("a", v(80, 3), Tier::Ram).unwrap();
    assert_eq!(t.ram_used, 110);
    assert_eq!(t.get("b"), Some(&v(30, 1)[..]));
    assert_eq!(t.get("a"), Some(&v(80, 3)[..]));
}

#[test]
fn reemplazar_expulsa_lru_solo_si_hace_falta() {
    let mut t = TierManager::new(100, 0);
    t.promote("viejo", v(30, 1), Tier::Ram).unwrap();
    t.promote("medio", v(30, 2), Tier::Ram).unwrap();
    t.promote("a", v(30, 3), Tier::Ram).unwrap();
    // a pasa a 50: sin la copia anterior quedan 60 usados; 60 + 50 > 100, sale el LRU (viejo)
    t.promote("a", v(50, 4), Tier::Ram).unwrap();
    assert_eq!(t.get("viejo"), None);
    assert_eq!(t.get("medio").map(|d| d.len()), Some(30));
    assert_eq!(t.get("a").map(|d| d.len()), Some(50));
    assert_eq!(t.ram_used, 80);
}

#[test]
fn el_mismo_tamano_cabe_exacto_sin_expulsar() {
    let mut t = TierManager::new(100, 0);
    t.promote("x", v(100, 1), Tier::Ram).unwrap();
    t.promote("x", v(100, 2), Tier::Ram).unwrap();
    assert_eq!(t.ram_used, 100);
    assert_eq!(t.get("x"), Some(&v(100, 2)[..]));
}

#[test]
fn lo_que_no_cabe_nunca_falla_sin_tocar_nada() {
    let mut t = TierManager::new(100, 50);
    t.promote("a", v(40, 1), Tier::Ram).unwrap();
    t.promote("b", v(40, 2), Tier::Ram).unwrap();
    t.promote("g", v(20, 3), Tier::Vram).unwrap();
    assert!(t.promote("enorme", v(101, 9), Tier::Ram).is_err());
    assert!(t.promote("enorme", v(51, 9), Tier::Vram).is_err());
    // reemplazar uno existente por algo imposible tampoco lo destruye
    assert!(t.promote("a", v(101, 9), Tier::Ram).is_err());
    assert_eq!(t.ram_used, 80);
    assert_eq!(t.vram_used, 20);
    assert_eq!(t.get("a"), Some(&v(40, 1)[..]));
    assert_eq!(t.get("b"), Some(&v(40, 2)[..]));
    assert_eq!(t.get("g"), Some(&v(20, 3)[..]));
    assert_eq!(t.get("enorme"), None);
}

#[test]
fn disk_no_es_destino_y_no_toca_nada() {
    let mut t = TierManager::new(100, 100);
    t.promote("a", v(10, 1), Tier::Ram).unwrap();
    assert!(t.promote("a", v(10, 2), Tier::Disk).is_err());
    assert_eq!(t.ram_used, 10);
    assert_eq!(t.get("a"), Some(&v(10, 1)[..]));
}

#[test]
fn cambiar_de_nivel_descuenta_del_anterior() {
    let mut t = TierManager::new(100, 100);
    t.promote("a", v(70, 1), Tier::Vram).unwrap();
    t.promote("a", v(60, 2), Tier::Ram).unwrap();
    assert_eq!(t.vram_used, 0);
    assert_eq!(t.ram_used, 60);
    // y de vuelta, con otro ocupante que debe seguir ahí
    t.promote("o", v(90, 3), Tier::Vram).unwrap();
    t.promote("a", v(10, 4), Tier::Vram).unwrap();
    assert_eq!(t.ram_used, 0);
    assert_eq!(t.vram_used, 100);
    assert_eq!(t.get("o").map(|d| d.len()), Some(90));
}

#[test]
fn lru_normal_sigue_funcionando() {
    let mut t = TierManager::new(100, 0);
    t.promote("a", v(40, 1), Tier::Ram).unwrap();
    t.promote("b", v(40, 2), Tier::Ram).unwrap();
    t.touch("a"); // b pasa a ser el menos usado
    t.promote("c", v(40, 3), Tier::Ram).unwrap();
    assert_eq!(t.get("b"), None);
    assert!(t.get("a").is_some() && t.get("c").is_some());
    assert_eq!(t.ram_used, 80);
}
