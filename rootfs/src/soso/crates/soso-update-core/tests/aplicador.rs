//! U5: aplicar y deshacer con cortes en cada paso.
//!
//! El criterio de cierre es que **cualquier corte deje una pareja completa** al
//! arrancar: o la versión anterior entera, o la nueva entera, nunca media. Aquí
//! se corta en todos los puntos posibles —incluido entre escribir un fichero y
//! hacer durable el progreso— y se comprueba que al reanudar se llega a un
//! estado coherente.

use soso_update_core::txn::aplicador::{self, De, Fallo, Sistema};
use soso_update_core::txn::journal::{Accion, Contenido, Entrada, Journal, Progreso};
use soso_update_core::txn::{TxnId, TxnState};
use std::collections::BTreeMap;

fn h(datos: &[u8]) -> Contenido {
    Contenido { size: datos.len() as u64, hash: soso_update_core::hex_sha256(datos) }
}

/// Sistema en memoria con inyección de cortes.
struct Mundo {
    sistema: BTreeMap<String, Vec<u8>>,
    preparado: BTreeMap<String, Vec<u8>>,
    respaldo: BTreeMap<String, Vec<u8>>,
    /// Diario durable: lo único que sobrevive a un corte además del disco.
    diario: Option<Vec<u8>>,
    /// Operaciones que quedan antes de cortar (`None` = no cortar).
    quedan: Option<u32>,
    ops: u32,
}

struct Corte;

impl Mundo {
    fn nuevo() -> Self {
        Self {
            sistema: BTreeMap::new(),
            preparado: BTreeMap::new(),
            respaldo: BTreeMap::new(),
            diario: None,
            quedan: None,
            ops: 0,
        }
    }
    fn gastar(&mut self) -> Result<(), Corte> {
        self.ops += 1;
        match &mut self.quedan {
            Some(0) => Err(Corte),
            Some(n) => {
                *n -= 1;
                Ok(())
            }
            None => Ok(()),
        }
    }
    fn diario_leido(&self) -> Journal {
        Journal::parse(self.diario.as_ref().expect("sin diario")).expect("diario ilegible")
    }
}

impl Sistema for Mundo {
    fn leer(&mut self, de: De, ruta: &str) -> Option<Vec<u8>> {
        match de {
            De::Preparado => self.preparado.get(ruta).cloned(),
            De::Respaldo => self.respaldo.get(ruta).cloned(),
        }
    }
    fn escribir(&mut self, ruta: &str, datos: &[u8]) -> Result<(), ()> {
        self.gastar().map_err(|_| ())?;
        self.sistema.insert(ruta.into(), datos.to_vec());
        Ok(())
    }
    fn borrar(&mut self, ruta: &str) -> Result<(), ()> {
        self.gastar().map_err(|_| ())?;
        self.sistema.remove(ruta);
        Ok(())
    }
    fn guardar_diario(&mut self, j: &Journal) -> Result<(), ()> {
        self.gastar().map_err(|_| ())?;
        self.diario = Some(j.format());
        Ok(())
    }
}

/// Operación típica: uno que se reemplaza, uno nuevo y uno que se borra.
fn escenario() -> (Mundo, Journal) {
    let mut m = Mundo::nuevo();
    m.sistema.insert("bin/sosh".into(), b"sosh viejo".to_vec());
    m.sistema.insert("lib/viejo.so".into(), b"libreria vieja".to_vec());
    m.preparado.insert("bin/sosh".into(), b"sosh NUEVO".to_vec());
    m.preparado.insert("bin/nuevo".into(), b"binario nuevo".to_vec());
    m.respaldo.insert("bin/sosh".into(), b"sosh viejo".to_vec());
    m.respaldo.insert("lib/viejo.so".into(), b"libreria vieja".to_vec());

    let mut j = Journal::nuevo(TxnId::from_manifest(b"m"), "0.3.0", "0.2.9");
    j.estado = TxnState::Armado;
    j.kernel_nuevo = Some(h(b"kernel nuevo"));
    j.kernel_anterior = Some(h(b"kernel viejo"));
    j.entradas = vec![
        Entrada {
            accion: Accion::Reemplazar,
            progreso: Progreso::Respaldado,
            path: "bin/sosh".into(),
            nuevo: Some(h(b"sosh NUEVO")),
            respaldo: Some(h(b"sosh viejo")),
        },
        Entrada {
            accion: Accion::Crear,
            progreso: Progreso::Respaldado,
            path: "bin/nuevo".into(),
            nuevo: Some(h(b"binario nuevo")),
            respaldo: None,
        },
        Entrada {
            accion: Accion::Borrar,
            progreso: Progreso::Respaldado,
            path: "lib/viejo.so".into(),
            nuevo: None,
            respaldo: Some(h(b"libreria vieja")),
        },
    ];
    assert!(j.validate().is_ok());
    // Armar deja el diario durable **antes** de que nadie aplique nada: es lo
    // que permite reanudar un corte en la primera operación.
    m.diario = Some(j.format());
    (m, j)
}

fn sistema_nuevo(m: &Mundo) -> bool {
    m.sistema.get("bin/sosh").map(|v| v.as_slice()) == Some(b"sosh NUEVO")
        && m.sistema.get("bin/nuevo").map(|v| v.as_slice()) == Some(b"binario nuevo")
        && !m.sistema.contains_key("lib/viejo.so")
}

fn sistema_anterior(m: &Mundo) -> bool {
    m.sistema.get("bin/sosh").map(|v| v.as_slice()) == Some(b"sosh viejo")
        && !m.sistema.contains_key("bin/nuevo")
        && m.sistema.get("lib/viejo.so").map(|v| v.as_slice()) == Some(b"libreria vieja")
}

// ── Camino limpio ────────────────────────────────────────────────────────

#[test]
fn aplicar_deja_el_sistema_nuevo_entero() {
    let (mut m, mut j) = escenario();
    aplicador::aplicar(&mut j, &mut m).unwrap();
    assert!(sistema_nuevo(&m), "{:?}", m.sistema);
    assert_eq!(j.estado, TxnState::Probando);
    assert!(j.entradas.iter().all(|e| e.progreso == Progreso::Aplicado));
}

#[test]
fn revertir_deja_el_sistema_anterior_entero() {
    let (mut m, mut j) = escenario();
    aplicador::aplicar(&mut j, &mut m).unwrap();
    aplicador::revertir(&mut j, &mut m).unwrap();
    assert!(sistema_anterior(&m), "{:?}", m.sistema);
    assert_eq!(j.estado, TxnState::Revertido);
    // El criterio nombra los tres casos: programas, borrados y versión.
    assert_eq!(j.version_anterior, "0.2.9");
}

#[test]
fn aplicar_dos_veces_no_repite_trabajo() {
    let (mut m, mut j) = escenario();
    aplicador::aplicar(&mut j, &mut m).unwrap();
    let ops = m.ops;
    // El estado ya es PROBANDO: reaplicar no es una transición válida y no
    // vuelve a escribir nada.
    let mut j2 = m.diario_leido();
    let _ = aplicador::aplicar(&mut j2, &mut m);
    assert!(sistema_nuevo(&m));
    assert!(m.ops - ops <= 2, "no debería reescribir ficheros: {} ops", m.ops - ops);
}

// ── Cortes ───────────────────────────────────────────────────────────────

/// Corta tras `n` operaciones, reanuda desde el diario durable y comprueba que
/// se llega a la pareja **nueva** entera.
fn aplicar_con_corte(n: u32) -> Mundo {
    let (mut m, mut j) = escenario();
    m.quedan = Some(n);
    let _ = aplicador::aplicar(&mut j, &mut m);
    // Reinicio: lo único que se conserva es el disco y el diario.
    m.quedan = None;
    let mut reanudado = m.diario_leido();
    aplicador::aplicar(&mut reanudado, &mut m).expect("reanudar");
    assert_eq!(reanudado.estado, TxnState::Probando);
    m
}

#[test]
fn un_corte_en_cualquier_paso_de_la_aplicacion_se_reanuda() {
    // 12 operaciones cubren de sobra el escenario entero: el primer diario, y
    // por cada entrada su escritura y su diario, más el diario final.
    for n in 0..12 {
        let m = aplicar_con_corte(n);
        assert!(sistema_nuevo(&m), "corte tras {n} operaciones dejó {:?}", m.sistema);
    }
}

/// Corta durante la reversión y comprueba que se vuelve a la pareja anterior.
fn revertir_con_corte(n: u32) -> Mundo {
    let (mut m, mut j) = escenario();
    aplicador::aplicar(&mut j, &mut m).unwrap();
    m.quedan = Some(n);
    let _ = aplicador::revertir(&mut j, &mut m);
    m.quedan = None;
    let mut reanudado = m.diario_leido();
    aplicador::revertir(&mut reanudado, &mut m).expect("reanudar reversión");
    assert_eq!(reanudado.estado, TxnState::Revertido);
    m
}

#[test]
fn un_corte_en_cualquier_paso_de_la_reversion_se_reanuda() {
    for n in 0..12 {
        let m = revertir_con_corte(n);
        assert!(sistema_anterior(&m), "corte tras {n} operaciones dejó {:?}", m.sistema);
    }
}

#[test]
fn un_corte_a_mitad_de_aplicar_se_puede_deshacer_igual() {
    // Es la pareja que importa: cortar aplicando y decidir volver atrás. Lo ya
    // escrito se restaura y lo que no se tocó se deja en paz.
    for n in 0..8 {
        let (mut m, mut j) = escenario();
        m.quedan = Some(n);
        let _ = aplicador::aplicar(&mut j, &mut m);
        m.quedan = None;
        let mut reanudado = m.diario_leido();
        aplicador::revertir(&mut reanudado, &mut m).expect("revertir tras corte");
        assert!(
            sistema_anterior(&m),
            "corte tras {n} y reversión dejó {:?}",
            m.sistema
        );
    }
}

// ── Lo que no se puede hacer ─────────────────────────────────────────────

#[test]
fn sin_respaldo_no_se_arma() {
    // `se_puede_deshacer` es la comprobación previa a publicar el registro de
    // arranque: sin la copia, la operación no tiene vuelta atrás.
    let (mut m, j) = escenario();
    assert_eq!(aplicador::se_puede_deshacer(&j, &mut m), Ok(()));

    m.respaldo.remove("bin/sosh");
    assert_eq!(
        aplicador::se_puede_deshacer(&j, &mut m),
        Err(Fallo::FaltaRespaldo("bin/sosh".into()))
    );
}

#[test]
fn un_respaldo_estropeado_se_detecta_antes_de_usarlo() {
    let (mut m, j) = escenario();
    m.respaldo.insert("bin/sosh".into(), b"basura".to_vec());
    assert_eq!(
        aplicador::se_puede_deshacer(&j, &mut m),
        Err(Fallo::HashDistinto("bin/sosh".into()))
    );
}

#[test]
fn un_preparado_estropeado_no_se_escribe_en_el_sistema() {
    let (mut m, mut j) = escenario();
    m.preparado.insert("bin/sosh".into(), b"corrupto".to_vec());
    assert_eq!(
        aplicador::aplicar(&mut j, &mut m),
        Err(Fallo::HashDistinto("bin/sosh".into()))
    );
    assert_eq!(
        m.sistema.get("bin/sosh").map(|v| v.as_slice()),
        Some(&b"sosh viejo"[..]),
        "el fichero del sistema no se toca si lo preparado no cuadra"
    );
}

#[test]
fn si_falta_lo_preparado_se_dice_cual() {
    let (mut m, mut j) = escenario();
    m.preparado.remove("bin/nuevo");
    assert_eq!(
        aplicador::aplicar(&mut j, &mut m),
        Err(Fallo::FaltaPreparado("bin/nuevo".into()))
    );
}

#[test]
fn borrar_algo_que_ya_no_esta_no_es_un_error() {
    let (mut m, mut j) = escenario();
    m.sistema.remove("lib/viejo.so");
    aplicador::aplicar(&mut j, &mut m).expect("un borrado repetido es idempotente");
    assert!(sistema_nuevo(&m));
}
