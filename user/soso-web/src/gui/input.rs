//! Polling de ratón y teclado.

use libsoso::sys;
use soso_abi::{INPUT_KEY, INPUT_MOUSE_BTN, INPUT_MOUSE_MOVE, InputEvent};

pub struct InputState {
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub btn_left: bool,
    pub btn_left_prev: bool,
    pub key: Option<u32>,
    /// Tras `Redimensionar` del WM hay que recomputar el layout HTML.
    pub layout_dirty: bool,
    key_decoder: KeyDecoder,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            mouse_x: 0,
            mouse_y: 0,
            btn_left: false,
            btn_left_prev: false,
            key: None,
            layout_dirty: false,
            key_decoder: KeyDecoder::default(),
        }
    }
}

impl InputState {
    pub fn end_frame(&mut self) {
        self.btn_left_prev = self.btn_left;
    }

    pub fn click_left(&self) -> bool {
        self.btn_left && !self.btn_left_prev
    }

    pub fn decode_key(&mut self, raw: u32) -> Option<DecodedKey> {
        self.key_decoder.feed(raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodedKey {
    Char(u8),
    Up,
    Down,
    Space,
    Escape,
    Backspace,
    Enter,
}

#[derive(Default)]
struct KeyDecoder {
    stage: u8,
}

impl KeyDecoder {
    fn feed(&mut self, raw: u32) -> Option<DecodedKey> {
        let b = raw as u8;
        match self.stage {
            0 => {
                if b == 0x1b {
                    self.stage = 1;
                    return None;
                }
                return map_plain_key(b);
            }
            1 => {
                if b == b'[' {
                    self.stage = 2;
                    return None;
                }
                self.stage = 0;
                return Some(DecodedKey::Escape);
            }
            2 => {
                self.stage = 0;
                return match b {
                    b'A' => Some(DecodedKey::Up),
                    b'B' => Some(DecodedKey::Down),
                    _ => None,
                };
            }
            _ => {
                self.stage = 0;
                None
            }
        }
    }
}

fn map_plain_key(b: u8) -> Option<DecodedKey> {
    match b {
        8 | 127 => Some(DecodedKey::Backspace),
        b'\n' | b'\r' => Some(DecodedKey::Enter),
        b' ' => Some(DecodedKey::Space),
        0x1b => Some(DecodedKey::Escape),
        c if c.is_ascii_graphic() || c == b'\t' => Some(DecodedKey::Char(c)),
        _ => None,
    }
}

pub fn poll_input_from_wm(st: &mut InputState, fb: &mut super::fb::Framebuffer) -> bool {
    let Ok(Some(ev)) = fb.poll_wm_event() else {
        return false;
    };
    use libsoso::ventana::Evento;
    match ev {
        Evento::Raton { x, y, botones } => {
            st.mouse_x = x;
            st.mouse_y = y;
            st.btn_left = botones & 1 != 0;
        }
        Evento::Tecla { key, .. } => {
            st.key = Some(key);
        }
        Evento::CerrarPedido => {
            st.key = Some(0x01);
        }
        Evento::Redimensionar { w, h, shm_id } => {
            let _ = fb.inner_mut().aplicar_redimension(w, h, shm_id);
            fb.info = fb.inner_mut().fb_info();
            st.layout_dirty = true;
        }
        Evento::Foco => {}
    }
    true
}

pub fn poll_input(st: &mut InputState) {
    let mut buf = [InputEvent::default(); 8];
    let n = sys::input_poll(&mut buf);
    if n <= 0 {
        return;
    }
    st.key = None;
    for ev in &buf[..n as usize] {
        match ev.kind {
            INPUT_MOUSE_MOVE => {
                st.mouse_x = ev.x;
                st.mouse_y = ev.y;
                st.btn_left = ev.button & 1 != 0;
            }
            INPUT_MOUSE_BTN => {
                st.btn_left = ev.button & 1 != 0;
            }
            INPUT_KEY => {
                if ev.pressed != 0 {
                    st.key = Some(ev.key);
                }
            }
            _ => {}
        }
    }
}
