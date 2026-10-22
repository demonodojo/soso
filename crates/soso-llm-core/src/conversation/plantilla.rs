//! Texto de un prompt con los tramos que **no** escribió la plantilla.
//!
//! La plantilla de chat pone marcadores de control (`<|im_end|>`, `<turn|>`…) que el tokenizer convierte en
//! tokens especiales. El contenido de los mensajes, los resultados de herramienta y los argumentos de
//! llamadas anteriores son de terceros: si traen esa misma cadena (un fichero que se está leyendo, una
//! edición que la cita) no deben convertirse en un fin de turno. `Plantilla` recuerda qué rangos son ajenos
//! para que [`Tokenizer::encode_marcado`](crate::tokenizer::Tokenizer::encode_marcado) los segmente como
//! texto corriente.

use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;

/// Destino de la escritura de una plantilla: un `String` (todo es texto de plantilla) o una [`Plantilla`].
pub trait Salida {
    /// Texto que escribe la plantilla.
    fn push_str(&mut self, s: &str);
    /// Un carácter de la plantilla.
    fn push(&mut self, c: char);
    /// Contenido de terceros.
    fn ajeno(&mut self, s: &str);
}

impl Salida for String {
    fn push_str(&mut self, s: &str) {
        String::push_str(self, s)
    }
    fn push(&mut self, c: char) {
        String::push(self, c)
    }
    fn ajeno(&mut self, s: &str) {
        String::push_str(self, s)
    }
}

/// Texto del prompt más los rangos de bytes que son contenido de terceros.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Plantilla {
    pub texto: String,
    pub ajenos: Vec<Range<usize>>,
}

impl Plantilla {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Salida for Plantilla {
    fn push_str(&mut self, s: &str) {
        self.texto.push_str(s)
    }
    fn push(&mut self, c: char) {
        self.texto.push(c)
    }
    fn ajeno(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let ini = self.texto.len();
        self.texto.push_str(s);
        let fin = self.texto.len();
        match self.ajenos.last_mut() {
            Some(r) if r.end == ini => r.end = fin,
            _ => self.ajenos.push(ini..fin),
        }
    }
}
