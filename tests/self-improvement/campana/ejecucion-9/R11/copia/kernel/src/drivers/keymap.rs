//! Mapas de teclado PS/2 set 1 → UTF-8.
//!
//! Por defecto **es** (ISO-105 español). **us** para QEMU / teclado americano.

use core::sync::atomic::{AtomicU8, Ordering};

/// Layout activo: 0 = es, 1 = us.
static LAYOUT: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Es,
    Us,
}

impl Layout {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "es" | "ES" => Some(Self::Es),
            "us" | "US" => Some(Self::Us),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Es => "es",
            Self::Us => "us",
        }
    }
}

pub fn layout() -> Layout {
    match LAYOUT.load(Ordering::Relaxed) {
        1 => Layout::Us,
        _ => Layout::Es,
    }
}

pub fn set_layout(l: Layout) {
    LAYOUT.store(
        match l {
            Layout::Es => 0,
            Layout::Us => 1,
        },
        Ordering::Relaxed,
    );
}

/// Tecla muerta pendiente (acento).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dead {
    Acute,      // ´
    Diaeresis,  // ¨
    Grave,      // `
    Circumflex, // ^
    Tilde,      // ~
}

/// Resultado de traducir un scancode.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeyOutput {
    /// Un byte ASCII (incluye BS, tab, LF).
    Byte(u8),
    /// Carácter Unicode.
    Char(char),
    /// Sin salida (modifier, caps toggle, etc.).
    None,
}

/// Estado del traductor (shift, altgr, ctrl, tecla muerta).
pub struct KeymapState {
    shift: bool,
    caps: bool,
    altgr: bool,
    ctrl: bool,
    dead: Option<Dead>,
    /// Tras un acento no combinable, queda la tecla siguiente por emitir.
    pending: Option<char>,
}

impl KeymapState {
    pub const fn new() -> Self {
        Self {
            shift: false,
            caps: false,
            altgr: false,
            ctrl: false,
            dead: None,
            pending: None,
        }
    }

    pub fn shift_press(&mut self, down: bool) {
        self.shift = down;
    }

    pub fn altgr_press(&mut self, down: bool) {
        self.altgr = down;
    }

    pub fn ctrl_press(&mut self, down: bool) {
        self.ctrl = down;
    }

    pub fn ctrl(&self) -> bool {
        self.ctrl
    }

    pub fn caps_toggle(&mut self) {
        self.caps = !self.caps;
    }

    /// Si quedó un carácter pendiente tras acento suelto, lo devuelve.
    pub fn take_pending(&mut self) -> Option<KeyOutput> {
        self.pending.take().map(KeyOutput::Char)
    }

    /// Traduce con modificadores explícitos (USB: snapshot del informe HID).
    pub fn translate_scancode(&mut self, sc: u8, shift: bool, altgr: bool) -> KeyOutput {
        let saved_shift = self.shift;
        let saved_altgr = self.altgr;
        self.shift = shift;
        self.altgr = altgr;
        let out = self.translate_inner(sc);
        self.shift = saved_shift;
        self.altgr = saved_altgr;
        out
    }

    /// Traduce un scancode make (AltGr ya reflejado en `altgr`).
    pub fn translate(&mut self, sc: u8) -> KeyOutput {
        self.translate_inner(sc)
    }

    fn translate_inner(&mut self, sc: u8) -> KeyOutput {
        match layout() {
            Layout::Us => self.translate_us(sc),
            Layout::Es => self.translate_es(sc),
        }
    }

    fn letter_shift(&self) -> bool {
        self.shift ^ self.caps
    }

    fn row_shift(&self) -> bool {
        self.shift
    }

    fn emit_char(&mut self, ch: char) -> KeyOutput {
        if ch.is_ascii() && (ch as u32) < 0x80 {
            return KeyOutput::Byte(ch as u8);
        }
        if let Some(d) = self.dead.take() {
            if let Some(composed) = compose_dead(d, ch) {
                return KeyOutput::Char(composed);
            }
            self.pending = Some(ch);
            return KeyOutput::Char(dead_char(d));
        }
        KeyOutput::Char(ch)
    }

    fn set_dead(&mut self, d: Dead) -> KeyOutput {
        if let Some(prev) = self.dead.replace(d) {
            self.dead = None;
            return KeyOutput::Char(dead_char(prev));
        }
        KeyOutput::None
    }

    fn translate_us(&mut self, sc: u8) -> KeyOutput {
        if self.altgr {
            return self.translate_us_altgr(sc);
        }
        let shift = self.row_shift();
        let lshift = self.letter_shift();
        match sc {
            0x02..=0x0D => {
                let row = if shift {
                    b"!@#$%^&*()_+"
                } else {
                    b"1234567890-="
                };
                KeyOutput::Byte(row[(sc - 0x02) as usize])
            }
            0x10..=0x1B => {
                let row = if lshift {
                    b"QWERTYUIOP{}"
                } else {
                    b"qwertyuiop[]"
                };
                self.emit_char(row[(sc - 0x10) as usize] as char)
            }
            0x1E..=0x28 => {
                let row = if lshift {
                    b"ASDFGHJKL:\""
                } else {
                    b"asdfghjkl;'"
                };
                self.emit_char(row[(sc - 0x1E) as usize] as char)
            }
            0x29 => self.emit_char(if shift { '~' } else { '`' }),
            0x2B => self.emit_char(if shift { '|' } else { '\\' }),
            0x2C..=0x35 => {
                let row = if shift {
                    b"ZXCVBNM<>?"
                } else {
                    b"zxcvbnm,./"
                };
                self.emit_char(row[(sc - 0x2C) as usize] as char)
            }
            0x39 => KeyOutput::Byte(b' '),
            0x0E => KeyOutput::Byte(0x08),
            0x0F => KeyOutput::Byte(b'\t'),
            0x1C => KeyOutput::Byte(b'\n'),
            // F4 → push-to-talk (PTT) en sosh
            0x3E => KeyOutput::Byte(0x12),
            _ => KeyOutput::None,
        }
    }

    fn translate_us_altgr(&mut self, sc: u8) -> KeyOutput {
        match sc {
            0x02 => self.emit_char('@'),
            0x03 => self.emit_char('#'),
            0x04 => self.emit_char('$'),
            0x05 => self.emit_char('%'),
            0x06 => self.emit_char('^'),
            0x07 => self.emit_char('&'),
            0x08 => self.emit_char('*'),
            0x09 => self.emit_char('('),
            0x0A => self.emit_char(')'),
            0x0B => self.emit_char('_'),
            0x0C => self.emit_char('+'),
            0x0D => self.emit_char('{'),
            0x1A => self.emit_char('}'),
            0x1B => self.emit_char('|'),
            0x27 => self.emit_char(':'),
            0x28 => self.emit_char('"'),
            0x29 => self.emit_char('~'),
            0x2B => self.emit_char('|'),
            0x33 => self.emit_char('<'),
            0x34 => self.emit_char('>'),
            _ => KeyOutput::None,
        }
    }

    fn translate_es(&mut self, sc: u8) -> KeyOutput {
        if self.altgr {
            return self.translate_es_altgr(sc);
        }
        let shift = self.row_shift();
        let lshift = self.letter_shift();
        match sc {
            0x02..=0x0A => {
                const ROW: [char; 9] = ['1', '2', '3', '4', '5', '6', '7', '8', '9'];
                const ROW_S: [char; 9] = ['!', '"', '·', '$', '%', '&', '/', '(', ')'];
                let i = (sc - 0x02) as usize;
                self.emit_char(if shift { ROW_S[i] } else { ROW[i] })
            }
            0x0B => self.emit_char(if shift { '=' } else { '0' }),
            0x0C => self.emit_char(if shift { '?' } else { '\'' }),
            0x0D => self.emit_char(if shift { '¿' } else { '¡' }),
            0x10..=0x19 => {
                const ROW: [char; 10] = [
                    'q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p',
                ];
                const ROW_S: [char; 10] = [
                    'Q', 'W', 'E', 'R', 'T', 'Y', 'U', 'I', 'O', 'P',
                ];
                let i = (sc - 0x10) as usize;
                self.emit_char(if lshift { ROW_S[i] } else { ROW[i] })
            }
            0x1A => {
                if shift {
                    self.set_dead(Dead::Circumflex)
                } else {
                    self.emit_char('`')
                }
            }
            0x1B => self.emit_char(if shift { '*' } else { '+' }),
            0x1E..=0x25 => {
                const ROW: [char; 8] = ['a', 's', 'd', 'f', 'g', 'h', 'j', 'k'];
                const ROW_S: [char; 8] = ['A', 'S', 'D', 'F', 'G', 'H', 'J', 'K'];
                let i = (sc - 0x1E) as usize;
                self.emit_char(if lshift { ROW_S[i] } else { ROW[i] })
            }
            0x26 => self.emit_char(if lshift { 'L' } else { 'l' }),
            0x27 => self.emit_char(if lshift { 'Ñ' } else { 'ñ' }),
            0x28 => {
                if shift {
                    self.set_dead(Dead::Diaeresis)
                } else {
                    self.set_dead(Dead::Acute)
                }
            }
            0x29 => self.emit_char(if shift { 'ª' } else { 'º' }),
            0x2B => self.emit_char(if shift { '|' } else { '\\' }),
            0x2C..=0x32 => {
                const ROW: [char; 7] = ['z', 'x', 'c', 'v', 'b', 'n', 'm'];
                const ROW_S: [char; 7] = ['Z', 'X', 'C', 'V', 'B', 'N', 'M'];
                let i = (sc - 0x2C) as usize;
                self.emit_char(if lshift { ROW_S[i] } else { ROW[i] })
            }
            0x33 => self.emit_char(if shift { ';' } else { ',' }),
            0x34 => self.emit_char(if shift { ':' } else { '.' }),
            0x35 => self.emit_char(if shift { '_' } else { '-' }),
            0x56 => self.emit_char(if shift { '>' } else { '<' }),
            0x39 => KeyOutput::Byte(b' '),
            0x0E => KeyOutput::Byte(0x08),
            0x0F => KeyOutput::Byte(b'\t'),
            0x1C => KeyOutput::Byte(b'\n'),
            0x3E => KeyOutput::Byte(0x12),
            _ => KeyOutput::None,
        }
    }

    fn translate_es_altgr(&mut self, sc: u8) -> KeyOutput {
        match sc {
            0x02 => self.emit_char('|'),
            0x03 => self.emit_char('@'),
            0x04 => self.emit_char('€'),
            0x05 => self.set_dead(Dead::Circumflex),
            0x06 => self.emit_char('¬'),
            0x07 => self.emit_char('{'),
            0x08 => self.emit_char('['),
            0x09 => self.emit_char(']'),
            0x0A => self.emit_char('}'),
            0x0B => self.emit_char('\\'),
            0x0C => self.emit_char('\\'),
            0x1A => self.emit_char('['),
            0x1B => self.emit_char(']'),
            0x27 => self.emit_char('~'),
            0x28 => self.set_dead(Dead::Grave),
            0x29 => self.set_dead(Dead::Tilde),
            0x2B => self.set_dead(Dead::Grave),
            0x33 => self.emit_char('|'),
            0x34 => self.emit_char('@'),
            _ => KeyOutput::None,
        }
    }
}

fn dead_char(d: Dead) -> char {
    match d {
        Dead::Acute => '´',
        Dead::Diaeresis => '¨',
        Dead::Grave => '`',
        Dead::Circumflex => '^',
        Dead::Tilde => '~',
    }
}

fn compose_dead(d: Dead, ch: char) -> Option<char> {
    let lower = ch.to_ascii_lowercase();
    let upper = |c: char| if ch.is_ascii_uppercase() { c.to_ascii_uppercase() } else { c };
    match (d, lower) {
        (Dead::Acute, 'a') => Some(upper('á')),
        (Dead::Acute, 'e') => Some(upper('é')),
        (Dead::Acute, 'i') => Some(upper('í')),
        (Dead::Acute, 'o') => Some(upper('ó')),
        (Dead::Acute, 'u') => Some(upper('ú')),
        (Dead::Acute, 'y') => Some(upper('ý')),
        (Dead::Diaeresis, 'a') => Some(upper('ä')),
        (Dead::Diaeresis, 'e') => Some(upper('ë')),
        (Dead::Diaeresis, 'i') => Some(upper('ï')),
        (Dead::Diaeresis, 'o') => Some(upper('ö')),
        (Dead::Diaeresis, 'u') => Some(upper('ü')),
        (Dead::Diaeresis, 'y') => Some(upper('ÿ')),
        (Dead::Grave, 'a') => Some(upper('à')),
        (Dead::Grave, 'e') => Some(upper('è')),
        (Dead::Grave, 'i') => Some(upper('ì')),
        (Dead::Grave, 'o') => Some(upper('ò')),
        (Dead::Grave, 'u') => Some(upper('ù')),
        (Dead::Circumflex, 'a') => Some(upper('â')),
        (Dead::Circumflex, 'e') => Some(upper('ê')),
        (Dead::Circumflex, 'i') => Some(upper('î')),
        (Dead::Circumflex, 'o') => Some(upper('ô')),
        (Dead::Circumflex, 'u') => Some(upper('û')),
        (Dead::Tilde, 'a') => Some(upper('ã')),
        (Dead::Tilde, 'n') => Some(upper('ñ')),
        (Dead::Tilde, 'o') => Some(upper('õ')),
        // Espacio tras muerta: emitir acento solo
        (_, ' ') => None,
        _ => None,
    }
}

/// Escribe la representación UTF-8 de `out` en `buf`. Devuelve bytes escritos.
pub fn output_bytes(out: KeyOutput, buf: &mut [u8]) -> usize {
    match out {
        KeyOutput::None => 0,
        KeyOutput::Byte(b) => {
            buf[0] = b;
            1
        }
        KeyOutput::Char(c) => {
            let mut tmp = [0u8; 4];
            let s = c.encode_utf8(&mut tmp);
            let n = s.len().min(buf.len());
            buf[..n].copy_from_slice(&tmp[..n]);
            n
        }
    }
}
