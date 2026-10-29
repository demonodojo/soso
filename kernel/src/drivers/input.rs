//! Cola de eventos de entrada (ratón / teclado) hacia userspace.

use alloc::collections::VecDeque;
use soso_abi::{INPUT_KEY, INPUT_MOUSE_BTN, INPUT_MOUSE_MOVE, InputEvent};
use spin::Mutex;
use x86_64::instructions::interrupts::without_interrupts;

static EVENTS: Mutex<VecDeque<InputEvent>> = Mutex::new(VecDeque::new());

/// Encola sin perder el evento más reciente. Se toma el candado con las
/// interrupciones cerradas: desde la IRQ un `try_lock` fallido tiraba el
/// movimiento y el puntero se quedaba atrás.
pub fn push(ev: InputEvent) {
    without_interrupts(|| {
        let mut q = EVENTS.lock();
        if q.len() >= 256 {
            q.pop_front();
        }
        q.push_back(ev);
    });
}

pub fn poll(out: &mut [InputEvent]) -> usize {
    without_interrupts(|| {
        let mut q = EVENTS.lock();
        let n = out.len().min(q.len());
        for i in 0..n {
            if let Some(ev) = q.pop_front() {
                out[i] = ev;
            }
        }
        n
    })
}

pub fn push_mouse_move(x: i32, y: i32, buttons: u32) {
    push(InputEvent {
        kind: INPUT_MOUSE_MOVE,
        x,
        y,
        button: buttons,
        ..InputEvent::default()
    });
}

pub fn push_mouse_btn(buttons: u32, x: i32, y: i32) {
    push(InputEvent {
        kind: INPUT_MOUSE_BTN,
        x,
        y,
        button: buttons,
        pressed: (buttons != 0) as u8,
        ..InputEvent::default()
    });
}

pub fn push_key(key: u32, pressed: bool, mods: u32) {
    push(InputEvent {
        kind: INPUT_KEY,
        key,
        button: mods,
        pressed: pressed as u8,
        ..InputEvent::default()
    });
}
