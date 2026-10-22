//! Restricción del muestreo: qué tokens pueden salir a continuación (salidas estructuradas).
//!
//! Una `Restriccion` es un autómata que sabe lo que ya se generó y dice si un token cabe. El muestreador
//! ([`Sampler::sample_restringido`](crate::sample::Sampler::sample_restringido)) la consulta sólo mientras está
//! [`activa`](Restriccion::activa): hasta que el modelo no abre una llamada a herramienta no se toca nada, y
//! cuando la cierra se suelta, de modo que el texto libre sale igual que sin restricción.
//!
//! Los autómatas concretos (una llamada a herramienta de cada familia de plantilla) viven en
//! `conversation::gramatica`; aquí sólo está el rasgo y la tabla de bytes por token que necesitan.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::tokenizer::Tokenizer;

pub trait Restriccion {
    /// ¿Puede ser `token` el siguiente? Sin efectos.
    fn permite(&self, token: u32) -> bool;
    /// El token se aceptó: avanza el estado. Se llama con **todos** los tokens generados, esté o no activa.
    fn avanza(&mut self, token: u32);
    /// ¿Está limitando ahora mismo? Si no, el muestreo es el de siempre.
    fn activa(&self) -> bool;
    /// Se suelta para el resto de la generación (el autómata no tiene salida: peor que sin restricción no se
    /// queda).
    fn abandonar(&mut self);
    /// Copia independiente del estado (verificar un borrador sin tocar el original).
    fn duplicar(&self) -> Box<dyn Restriccion>;
}

/// Bytes de cada token del vocabulario, calculados una vez. Un autómata por bytes los consulta por id sin
/// decodificar nada en el bucle de generación.
#[derive(Debug, Clone, Default)]
pub struct TablaBytes {
    datos: Vec<u8>,
    /// `desde[i]..desde[i + 1]` son los bytes del token `i`.
    desde: Vec<u32>,
}

impl TablaBytes {
    pub fn de(tok: &Tokenizer) -> Self {
        let n = tok.vocab_len();
        let mut datos = Vec::new();
        let mut desde = Vec::with_capacity(n + 1);
        for id in 0..n as u32 {
            desde.push(datos.len() as u32);
            tok.token_bytes(id, &mut datos);
        }
        desde.push(datos.len() as u32);
        Self { datos, desde }
    }

    /// Una tabla a partir de los bytes de cada token (pruebas y vocabularios sintéticos).
    pub fn desde_piezas<'a>(piezas: impl IntoIterator<Item = &'a [u8]>) -> Self {
        let mut datos = Vec::new();
        let mut desde = Vec::new();
        for p in piezas {
            desde.push(datos.len() as u32);
            datos.extend_from_slice(p);
        }
        desde.push(datos.len() as u32);
        Self { datos, desde }
    }

    pub fn len(&self) -> usize {
        self.desde.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bytes del token; vacío si el id no existe o no emite nada (bos/eos).
    pub fn bytes(&self, id: u32) -> &[u8] {
        let i = id as usize;
        match (self.desde.get(i), self.desde.get(i + 1)) {
            (Some(&a), Some(&b)) => &self.datos[a as usize..b as usize],
            _ => &[],
        }
    }
}

impl Tokenizer {
    /// Tabla de bytes por token de este vocabulario ([`TablaBytes`]).
    pub fn tabla_bytes(&self) -> TablaBytes {
        TablaBytes::de(self)
    }

    /// Bytes de un token (los que emite al decodificar). Para consultas sueltas; en el bucle de generación,
    /// [`tabla_bytes`](Self::tabla_bytes).
    pub fn bytes_token(&self, id: u32) -> Vec<u8> {
        let mut v = Vec::new();
        self.token_bytes(id, &mut v);
        v
    }
}
