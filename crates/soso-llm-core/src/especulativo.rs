//! Política del borrador especulativo: cuántos tokens del borrador vale la pena verificar en cada ronda.
//!
//! Verificar `k` tokens de borrador cuesta un forward de `k + 1` posiciones (`coste(k + 1)`) y rinde, en
//! esperanza, `1 + Σ_j Π_{i≤j} p_i` tokens, con `p_i` la probabilidad de que el borrador acierte su posición
//! `i` habiendo acertado las anteriores. Se elige el `k` que maximiza tokens por milisegundo (la idea de
//! longitud variable por coste de DSpark/DeepInfer, sin su planificador de clúster):
//!
//! ```text
//! k* = argmax_{k ∈ 0..=disponibles} (1 + Σ_{j≤k} Π_{i≤j} p_i) / coste(k + 1)
//! ```
//!
//! - `coste(n)` sale de la **mediana** de las últimas muestras de rondas de `n` posiciones (más robusta que una
//!   media móvil exponencial ante los picos de E/S); las longitudes sin muestra se interpolan entre las que
//!   sí tienen.
//! - `p_i` se estima por posición, con un prior suave para que unas pocas rondas no la fijen en 0 ni en 1.
//! - Hasta tener [`MUESTRAS_MIN`] muestras de coste no se opina: [`PoliticaBorrador::lista`] es falso y quien
//!   la usa conserva el comportamiento anterior (todo el borrador).
//!
//! Sin dependencias de `std`: el reloj lo pone quien la llama (el runtime guarda un `fn() -> u64` en ms).

use alloc::vec::Vec;

/// Posiciones máximas de una ronda de verificación (`k + 1`).
pub const MAX_BLOQUE: usize = 16;
/// Muestras de coste por longitud con las que se calcula la mediana.
const VENTANA: usize = 9;
/// Muestras de coste (en total) a partir de las cuales la política decide.
pub const MUESTRAS_MIN: usize = 5;
/// Una ronda de cada tantas ofrece el borrador entero aunque la política prefiera menos, para que la
/// estimación de `p_i` siga viva cuando está baja (si no, no volvería a medirse nunca).
const EXPLORAR_CADA: u32 = 16;
/// Prior de aceptación y su peso (en rondas equivalentes).
const PRIOR_P: f32 = 0.5;
const PRIOR_PESO: f32 = 2.0;
/// Coste mínimo que se usa (ms): con reloj de milisegundos, una ronda de «0 ms» sólo dice «menos de 1».
const COSTE_MIN: f32 = 0.5;

#[derive(Debug, Clone)]
pub struct PoliticaBorrador {
    /// Últimas muestras de coste (ms) por número de posiciones de la ronda (índice `n`).
    muestras: [Vec<u64>; MAX_BLOQUE + 1],
    /// Por posición del borrador (índice `i` = posición `i + 1`): veces ofrecida y veces acertada.
    ofrecidos: [u32; MAX_BLOQUE],
    aceptados: [u32; MAX_BLOQUE],
    rondas: u32,
}

impl Default for PoliticaBorrador {
    fn default() -> Self {
        Self::new()
    }
}

impl PoliticaBorrador {
    pub fn new() -> Self {
        Self {
            muestras: core::array::from_fn(|_| Vec::new()),
            ofrecidos: [0; MAX_BLOQUE],
            aceptados: [0; MAX_BLOQUE],
            rondas: 0,
        }
    }

    /// Anota lo que costó una ronda de `n` posiciones (`n = 1` es un paso normal sin borrador).
    pub fn registrar_coste(&mut self, n: usize, ms: u64) {
        if n == 0 || n > MAX_BLOQUE {
            return;
        }
        let v = &mut self.muestras[n];
        if v.len() == VENTANA {
            v.remove(0);
        }
        v.push(ms);
    }

    /// Anota una ronda: se ofrecieron `ofrecidos` tokens y se aceptaron los `aceptados` primeros.
    ///
    /// Una posición cuenta como ofrecida sólo si se llegó a comprobar (las anteriores acertaron), de modo que
    /// `p_i` es condicional, que es lo que pide la fórmula de la esperanza.
    pub fn registrar_ronda(&mut self, ofrecidos: usize, aceptados: usize) {
        let aceptados = aceptados.min(ofrecidos);
        let comprobados = (aceptados + 1).min(ofrecidos);
        for i in 0..comprobados.min(MAX_BLOQUE) {
            self.ofrecidos[i] += 1;
            if i < aceptados {
                self.aceptados[i] += 1;
            }
        }
    }

    /// Muestras de coste acumuladas (en todas las longitudes).
    pub fn muestras(&self) -> usize {
        self.muestras.iter().map(Vec::len).sum()
    }

    /// ¿Hay datos suficientes para que `elegir_k` opine?
    pub fn lista(&self) -> bool {
        self.muestras() >= MUESTRAS_MIN
    }

    fn mediana(v: &[u64]) -> Option<f32> {
        if v.is_empty() {
            return None;
        }
        let mut o: Vec<u64> = v.to_vec();
        o.sort_unstable();
        let m = o.len() / 2;
        Some(if o.len() % 2 == 1 { o[m] as f32 } else { (o[m - 1] + o[m]) as f32 / 2.0 })
    }

    /// Coste estimado (ms) de una ronda de `n` posiciones: la mediana de sus muestras o, si no hay,
    /// interpolación lineal entre las longitudes medidas (constante por debajo de la menor; por encima de la
    /// mayor, con la pendiente de los dos últimos puntos, sin bajar nunca del último).
    pub fn coste(&self, n: usize) -> f32 {
        let n = n.clamp(1, MAX_BLOQUE);
        let mut puntos: [(usize, f32); MAX_BLOQUE] = [(0, 0.0); MAX_BLOQUE];
        let mut cuantos = 0;
        for m in 1..=MAX_BLOQUE {
            if let Some(c) = Self::mediana(&self.muestras[m]) {
                puntos[cuantos] = (m, c);
                cuantos += 1;
            }
        }
        let puntos = &puntos[..cuantos];
        let c = match puntos {
            [] => 1.0,
            [(_, c)] => *c,
            _ => {
                if let Some(&(_, c)) = puntos.iter().find(|(m, _)| *m == n) {
                    c
                } else if n < puntos[0].0 {
                    puntos[0].1
                } else if let Some(w) = puntos.windows(2).find(|w| w[0].0 < n && n < w[1].0) {
                    let (m0, c0) = w[0];
                    let (m1, c1) = w[1];
                    c0 + (c1 - c0) * (n - m0) as f32 / (m1 - m0) as f32
                } else {
                    let (m0, c0) = puntos[cuantos - 2];
                    let (m1, c1) = puntos[cuantos - 1];
                    let pendiente = ((c1 - c0) / (m1 - m0) as f32).max(0.0);
                    c1 + pendiente * (n - m1) as f32
                }
            }
        };
        c.max(COSTE_MIN)
    }

    /// Probabilidad estimada de que el borrador acierte su posición `i` (1-based) dado que acertó las anteriores.
    pub fn aceptacion(&self, i: usize) -> f32 {
        if i == 0 || i > MAX_BLOQUE {
            return 0.0;
        }
        let o = self.ofrecidos[i - 1] as f32;
        let a = self.aceptados[i - 1] as f32;
        (a + PRIOR_P * PRIOR_PESO) / (o + PRIOR_PESO)
    }

    /// Tokens esperados por ronda al verificar `k` de borrador (incluye el token propio de la ronda).
    pub fn tokens_esperados(&self, k: usize) -> f32 {
        let mut prod = 1.0f32;
        let mut suma = 1.0f32;
        for i in 1..=k.min(MAX_BLOQUE - 1) {
            prod *= self.aceptacion(i);
            suma += prod;
        }
        suma
    }

    /// Longitud de borrador (`0..=disponibles`) que maximiza tokens por milisegundo.
    ///
    /// Si no está [`lista`](Self::lista) devuelve `disponibles`. Una ronda de cada [`EXPLORAR_CADA`] también.
    /// Determinista: mismas muestras y mismas rondas, misma respuesta; en empate gana el `k` menor.
    pub fn elegir_k(&mut self, disponibles: usize) -> usize {
        let tope = disponibles.min(MAX_BLOQUE - 1);
        self.rondas = self.rondas.wrapping_add(1);
        if !self.lista() || self.rondas % EXPLORAR_CADA == 0 {
            return tope;
        }
        let mut mejor = 0usize;
        let mut mejor_valor = self.tokens_esperados(0) / self.coste(1);
        for k in 1..=tope {
            let valor = self.tokens_esperados(k) / self.coste(k + 1);
            if valor > mejor_valor {
                mejor = k;
                mejor_valor = valor;
            }
        }
        mejor
    }
}
