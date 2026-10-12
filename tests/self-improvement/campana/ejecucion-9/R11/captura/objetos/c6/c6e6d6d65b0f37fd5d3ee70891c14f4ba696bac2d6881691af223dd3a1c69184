//! Lectura de líneas por la tty, con eco propio.
//!
//! La consola del kernel es cruda: no hay disciplina de línea, así que el eco,
//! el borrado y el troceado en líneas los hace quien lee. Esto vivía suelto
//! dentro de `sosh`; está aquí porque el REPL de `ask` necesita exactamente lo
//! mismo, y porque así el soporte de UTF-8 se arregla en un solo sitio.
//!
//! Tres diferencias con el bucle original de sosh:
//!
//! - **Pasan los bytes ≥ 0x80.** Antes el filtro era `0x20..=0x7e` y todo lo
//!   demás caía en la rama vacía: escribir «¿qué tal?» dejaba «qu tal?» sin que
//!   nada lo dijera. Ahora la tilde llega entera al proceso.
//! - **El borrado va por carácter, no por byte.** Una `é` son dos bytes en
//!   UTF-8; borrar uno solo dejaría media secuencia y el kernel rechazaría los
//!   argumentos con `EINVAL` en el spawn siguiente.
//! - **Se lee byte a byte.** Leer de 64 en 64 dejaba dentro del proceso lo que
//!   viniera detrás de la línea; si acto seguido se lanzaba un hijo que hereda
//!   la tty (el REPL de `ask`), esos bytes ya no estaban en el kernel y el hijo
//!   nunca los veía. Con lecturas de un byte, lo no consumido se queda en la
//!   cola de la tty, que es de quien toque leer.

use alloc::string::String;

use crate::sys;
use soso_abi as abi;

/// Tope de línea. El kernel admite hasta 3000 bytes de argumentos
/// (`spawn_console_io`), así que 1024 caben de sobra incluso con el prefijo que
/// añade el builtin `ask`. Los 256 de antes se quedaban cortos para una
/// pregunta de verdad.
pub const MAX_LINEA: usize = 1024;

pub struct Lector {
    linea: [u8; MAX_LINEA],
    len: usize,
    /// El último byte fue un `\r`: si viene un `\n` detrás es el mismo salto.
    cr_previo: bool,
    eco: bool,
    /// Texto insertado antes de leer (p. ej. tras dictado por voz).
    prefijo: Option<String>,
    prefijo_i: usize,
    /// Push-to-talk: F4 → transcripción insertada en la línea.
    hook_ptt: Option<fn() -> Option<String>>,
}

impl Default for Lector {
    fn default() -> Self {
        Self::new()
    }
}

impl Lector {
    pub fn new() -> Lector {
        Lector {
            linea: [0u8; MAX_LINEA],
            len: 0,
            cr_previo: false,
            eco: true,
            prefijo: None,
            prefijo_i: 0,
            hook_ptt: None,
        }
    }

    pub fn con_hook_ptt(mut self, hook: fn() -> Option<String>) -> Lector {
        self.hook_ptt = Some(hook);
        self
    }

    pub fn con_texto_inicial(mut self, texto: &str) -> Lector {
        self.prefijo = Some(String::from(texto));
        self.prefijo_i = 0;
        self
    }

    fn inyectar_prefijo(&mut self) {
        let Some(ref p) = self.prefijo else { return };
        while self.prefijo_i < p.len() && self.len < self.linea.len() {
            let b = p.as_bytes()[self.prefijo_i];
            self.linea[self.len] = b;
            self.len += 1;
            self.prefijo_i += 1;
            if self.eco {
                let _ = sys::write_all(1, &[b]);
            }
        }
        if self.prefijo_i >= p.len() {
            self.prefijo = None;
        }
    }

    /// Sin eco: para cuando quien llama ya lo hace, o no se quiere ver.
    pub fn sin_eco(mut self) -> Lector {
        self.eco = false;
        self
    }

    /// Bloquea hasta tener una línea completa. Devuelve `None` en fin de
    /// entrada (Ctrl-D o error de lectura), que es la señal de salida de un
    /// REPL. Una línea que no sea UTF-8 válido se descarta con aviso y se
    /// espera a la siguiente.
    pub fn siguiente(&mut self) -> Option<String> {
        self.inyectar_prefijo();
        loop {
            let mut byte = [0u8; 1];
            let n = sys::read(0, &mut byte);
            if n == -(abi::EINTR as i64) {
                self.len = 0;
                self.eco_str("^C\n");
                continue;
            }
            if n < 0 {
                return None;
            }
            if n == 0 {
                continue;
            }
            let c = byte[0];
            let cr = core::mem::replace(&mut self.cr_previo, c == b'\r');
            match c {
                b'\n' if cr => {}
                b'\r' | b'\n' => {
                    self.eco_str("\n");
                    let len = self.len;
                    self.len = 0;
                    match core::str::from_utf8(&self.linea[..len]) {
                        Ok(s) => return Some(String::from(s)),
                        Err(_) => crate::println!("entrada descartada: no es UTF-8 válido"),
                    }
                }
                // Ctrl-D en línea vacía = fin de entrada; con algo escrito se
                // ignora, como en cualquier shell.
                0x04 => {
                    if self.len == 0 {
                        self.eco_str("\n");
                        return None;
                    }
                }
                0x08 | 0x7f => self.borrar_caracter(),
                // F4 (keymap) → push-to-talk
                0x12 => {
                    if let Some(hook) = self.hook_ptt {
                        if let Some(texto) = hook() {
                            for b in texto.bytes() {
                                if self.len < self.linea.len() {
                                    self.linea[self.len] = b;
                                    self.len += 1;
                                    if self.eco {
                                        let _ = sys::write_all(1, &[b]);
                                    }
                                }
                            }
                        }
                    }
                }
                _ if c >= 0x20 => {
                    if self.len < self.linea.len() {
                        self.linea[self.len] = c;
                        self.len += 1;
                        // El eco también por `write_all`: si el pipe está
                        // lleno, un `write` suelto devuelve 0 y el carácter
                        // desaparece de la pantalla sin que nada lo diga.
                        if self.eco {
                            let _ = sys::write_all(1, &[c]);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Borra el último carácter completo, no el último byte: en UTF-8 los
    /// bytes de continuación son `0x80..=0xBF` y hay que retroceder sobre
    /// ellos hasta el byte inicial.
    fn borrar_caracter(&mut self) {
        if self.len == 0 {
            return;
        }
        let mut n = 1;
        while n < self.len && (self.linea[self.len - n] & 0xC0) == 0x80 {
            n += 1;
        }
        self.len -= n;
        // Una celda en pantalla por carácter, aunque ocupe varios bytes.
        self.eco_str("\x08 \x08");
    }

    fn eco_str(&self, s: &str) {
        if self.eco {
            let _ = sys::write_all(1, s.as_bytes());
        }
    }
}
