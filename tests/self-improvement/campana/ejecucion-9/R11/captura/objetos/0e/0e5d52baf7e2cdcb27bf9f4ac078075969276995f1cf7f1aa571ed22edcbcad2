//! Polling de ratón y teclado.

use libsoso::sys;
use soso_abi::{InputEvent, INPUT_KEY, INPUT_MOUSE_BTN, INPUT_MOUSE_MOVE};

pub struct InputState {
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub btn_left: bool,
    pub key: Option<u32>,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            mouse_x: 0,
            mouse_y: 0,
            btn_left: false,
            key: None,
        }
    }
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
