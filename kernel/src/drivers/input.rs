//! Cola de eventos de entrada (ratón / teclado) hacia userspace.

use alloc::collections::VecDeque;
use soso_abi::{InputEvent, INPUT_KEY, INPUT_MOUSE_BTN, INPUT_MOUSE_MOVE};
use spin::Mutex;

static EVENTS: Mutex<VecDeque<InputEvent>> = Mutex::new(VecDeque::new());

pub fn push(ev: InputEvent) {
    if let Some(mut q) = EVENTS.try_lock() {
        if q.len() < 256 {
            q.push_back(ev);
        }
    }
}

pub fn poll(out: &mut [InputEvent]) -> usize {
    let mut q = EVENTS.lock();
    let n = out.len().min(q.len());
    for i in 0..n {
        if let Some(ev) = q.pop_front() {
            out[i] = ev;
        }
    }
    n
}

pub fn push_mouse_move(x: i32, y: i32) {
    push(InputEvent {
        kind: INPUT_MOUSE_MOVE,
        x,
        y,
        ..InputEvent::default()
    });
}

pub fn push_mouse_btn(button: u32, pressed: bool) {
    push(InputEvent {
        kind: INPUT_MOUSE_BTN,
        button,
        pressed: pressed as u8,
        ..InputEvent::default()
    });
}

pub fn push_key(key: u32, pressed: bool) {
    push(InputEvent {
        kind: INPUT_KEY,
        key,
        pressed: pressed as u8,
        ..InputEvent::default()
    });
}
