//! Reloj monotónico y plazos (T48).
//!
//! Un plazo no se puede medir con la hora del día: si el reloj salta, la
//! medida miente. Aquí el tiempo es **monotónico**, en milisegundos, con un
//! origen arbitrario del que nadie debe depender: sólo las diferencias
//! significan algo.
//!
//! El reloj entra por un trait para que las pruebas no tengan que dormir de
//! verdad. Una prueba que espera un segundo para comprobar un vencimiento es
//! una prueba lenta y frágil; [`RelojSimulado`] avanza cuando se le dice.

use core::cell::Cell;

/// Milisegundos monotónicos. No retrocede nunca.
pub trait Reloj {
    fn ahora_ms(&self) -> u64;
}

/// Un instante límite. Se construye desde un reloj y se pregunta con el mismo.
///
/// La suma satura: un `dentro_ms` enorme da un plazo que no vence, en vez de
/// dar la vuelta y vencer inmediatamente, que es como un desbordamiento se
/// convierte en un fallo intermitente imposible de leer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plazo {
    fin_ms: u64,
}

impl Plazo {
    pub fn en(reloj: &dyn Reloj, dentro_ms: u64) -> Plazo {
        Plazo {
            fin_ms: reloj.ahora_ms().saturating_add(dentro_ms),
        }
    }

    /// Sin límite. Existe para poder decirlo explícitamente; C5 exige que toda
    /// espera de producción tenga tope, así que esto es para casos concretos.
    pub fn infinito() -> Plazo {
        Plazo { fin_ms: u64::MAX }
    }

    pub fn es_infinito(&self) -> bool {
        self.fin_ms == u64::MAX
    }

    pub fn vencido(&self, reloj: &dyn Reloj) -> bool {
        !self.es_infinito() && reloj.ahora_ms() >= self.fin_ms
    }

    /// Cuánto queda. Cero si ya venció; `u64::MAX` si es infinito.
    pub fn restante_ms(&self, reloj: &dyn Reloj) -> u64 {
        if self.es_infinito() {
            return u64::MAX;
        }
        self.fin_ms.saturating_sub(reloj.ahora_ms())
    }

    /// Espera que se puede pedir al sistema sin pasarse del plazo, acotada por
    /// `tope_ms` para seguir sondeando cancelaciones.
    pub fn espera_ms(&self, reloj: &dyn Reloj, tope_ms: u64) -> u64 {
        let queda = self.restante_ms(reloj);
        if queda == u64::MAX {
            tope_ms
        } else {
            queda.min(tope_ms)
        }
    }
}

/// Reloj de pruebas: sólo avanza cuando se le dice.
#[derive(Debug, Default)]
pub struct RelojSimulado {
    ms: Cell<u64>,
}

impl RelojSimulado {
    pub fn nuevo(inicio_ms: u64) -> Self {
        RelojSimulado {
            ms: Cell::new(inicio_ms),
        }
    }

    pub fn avanzar(&self, ms: u64) {
        self.ms.set(self.ms.get().saturating_add(ms));
    }
}

impl Reloj for RelojSimulado {
    fn ahora_ms(&self) -> u64 {
        self.ms.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_plazo_vence_cuando_toca() {
        let r = RelojSimulado::nuevo(1_000);
        let p = Plazo::en(&r, 500);
        assert!(!p.vencido(&r));
        assert_eq!(p.restante_ms(&r), 500);
        r.avanzar(499);
        assert!(!p.vencido(&r));
        assert_eq!(p.restante_ms(&r), 1);
        r.avanzar(1);
        assert!(p.vencido(&r));
        assert_eq!(p.restante_ms(&r), 0);
        r.avanzar(10_000);
        assert!(p.vencido(&r));
        assert_eq!(p.restante_ms(&r), 0);
    }

    /// Un `dentro_ms` enorme no puede dar la vuelta y vencer al instante.
    #[test]
    fn la_suma_satura_en_vez_de_desbordar() {
        let r = RelojSimulado::nuevo(u64::MAX - 5);
        let p = Plazo::en(&r, 1_000);
        assert!(!p.vencido(&r));
        assert!(p.es_infinito() || p.restante_ms(&r) > 0);
    }

    #[test]
    fn el_infinito_no_vence() {
        let r = RelojSimulado::nuevo(0);
        let p = Plazo::infinito();
        r.avanzar(u64::MAX / 2);
        assert!(!p.vencido(&r));
        assert_eq!(p.restante_ms(&r), u64::MAX);
    }

    #[test]
    fn la_espera_respeta_el_tope_y_el_plazo() {
        let r = RelojSimulado::nuevo(0);
        let p = Plazo::en(&r, 30);
        assert_eq!(p.espera_ms(&r, 100), 30);
        assert_eq!(p.espera_ms(&r, 10), 10);
        assert_eq!(Plazo::infinito().espera_ms(&r, 25), 25);
        r.avanzar(30);
        assert_eq!(p.espera_ms(&r, 100), 0);
    }
}
