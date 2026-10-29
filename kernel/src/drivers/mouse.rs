//! Ratón PS/2 (puerto aux i8042).

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use x86_64::instructions::port::Port;

use super::fb;
use super::input;

const DATA: u16 = 0x60;
const STATUS: u16 = 0x64;
const CMD: u16 = 0x64;

const ST_OUT_FULL: u8 = 1;
const ST_IN_FULL: u8 = 2;

static OK: AtomicBool = AtomicBool::new(false);
static X: AtomicI32 = AtomicI32::new(0);
static Y: AtomicI32 = AtomicI32::new(0);
static BTNS: AtomicU32 = AtomicU32::new(0);
static MAX_W: AtomicI32 = AtomicI32::new(1920);
static MAX_H: AtomicI32 = AtomicI32::new(1080);

static mut PKT: [u8; 3] = [0; 3];
static mut PKT_I: u8 = 0;

fn wait_out_empty() {
    for _ in 0..100_000 {
        if unsafe { Port::<u8>::new(STATUS).read() } & ST_IN_FULL == 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

fn wait_in_full() {
    for _ in 0..100_000 {
        if unsafe { Port::<u8>::new(STATUS).read() } & ST_OUT_FULL != 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

fn write_cmd(v: u8) {
    wait_out_empty();
    unsafe { Port::<u8>::new(CMD).write(v) };
}

fn write_data(v: u8) {
    wait_out_empty();
    unsafe { Port::<u8>::new(DATA).write(v) };
}

fn read_data() -> u8 {
    wait_in_full();
    unsafe { Port::<u8>::new(DATA).read() }
}

fn write_aux(v: u8) {
    write_cmd(0xD4);
    write_data(v);
}

fn read_config() -> Option<u8> {
    write_cmd(0x20);
    wait_in_full();
    Some(unsafe { Port::<u8>::new(DATA).read() })
}

fn write_config(v: u8) {
    write_cmd(0x60);
    write_data(v);
}

fn flush_out() {
    for _ in 0..16 {
        if unsafe { Port::<u8>::new(STATUS).read() } & ST_OUT_FULL == 0 {
            break;
        }
        let _ = read_data();
    }
}

/// Copia el tamaño del framebuffer. No se llama desde la IRQ: `user_info`
/// toma el candado de la consola y un movimiento a mitad de un `present`
/// dejaría el núcleo girando sobre sí mismo.
pub fn refresh_bounds() {
    let mut info = soso_abi::FbInfo::default();
    if fb::user_info(&mut info) && info.present != 0 && info.width > 1 && info.height > 1 {
        MAX_W.store(info.width as i32, Ordering::Relaxed);
        MAX_H.store(info.height as i32, Ordering::Relaxed);
    }
}

fn clamp_xy(nx: i32, ny: i32) -> (i32, i32) {
    let max_w = MAX_W.load(Ordering::Relaxed).max(1);
    let max_h = MAX_H.load(Ordering::Relaxed).max(1);
    (nx.clamp(0, max_w - 1), ny.clamp(0, max_h - 1))
}

/// Máscara de botones USB boot / PS/2 (bit0=izq, bit1=der, bit2=medio).
pub fn buttons_from_hid_byte(flags: u8) -> u32 {
    ((flags & 1) != 0) as u32 | (((flags & 2) != 0) as u32) << 1 | (((flags & 4) != 0) as u32) << 2
}

/// `dx`/`dy` en coordenadas de pantalla (Y positivo hacia abajo). `buttons` es
/// la máscara completa (bit0 izquierdo, bit1 derecho, bit2 central).
///
/// El movimiento sale con esa máscara. Si no, el escritorio ve el botón a
/// cero en cada desplazamiento y ni el arrastre ni el clic sobreviven al
/// evento siguiente. El botón se publica antes, en la posición ya actualizada,
/// para que el mismo paquete que pulsa pueda empezar a arrastrar.
/// Posición absoluta lógica HID (0..max) → píxeles de pantalla.
pub fn apply_absolute_logical(x: i32, y: i32, x_max: u32, y_max: u32, buttons: u32) {
    let mw = MAX_W.load(Ordering::Relaxed).max(1) as i64;
    let mh = MAX_H.load(Ordering::Relaxed).max(1) as i64;
    let xm = x_max.max(1) as i64;
    let ym = y_max.max(1) as i64;
    let sx = ((x as i64).clamp(0, xm) * (mw - 1)) / xm;
    let sy = ((y as i64).clamp(0, ym) * (mh - 1)) / ym;
    apply_absolute(sx as i32, sy as i32, buttons);
}

pub fn apply_absolute(x: i32, y: i32, buttons: u32) {
    let (cx, cy) = clamp_xy(x, y);
    let prev_x = X.load(Ordering::Relaxed);
    let prev_y = Y.load(Ordering::Relaxed);
    X.store(cx, Ordering::Relaxed);
    Y.store(cy, Ordering::Relaxed);
    let prev = BTNS.swap(buttons, Ordering::Relaxed);
    if buttons != prev {
        input::push_mouse_btn(buttons, cx, cy);
    }
    if cx != prev_x || cy != prev_y || buttons != prev {
        input::push_mouse_move(cx, cy, buttons);
    }
}

pub fn apply_hid_pointer(evt: xhci_nostd::PointerEvent) {
    if evt.absolute {
        apply_absolute_logical(evt.dx, evt.dy, evt.abs_x_max, evt.abs_y_max, evt.buttons);
    } else {
        apply_pointer(evt.dx, evt.dy, evt.buttons);
    }
}

pub fn apply_pointer(dx: i32, dy: i32, buttons: u32) {
    let prev_x = X.load(Ordering::Relaxed);
    let prev_y = Y.load(Ordering::Relaxed);
    let (cx, cy) = clamp_xy(prev_x.saturating_add(dx), prev_y.saturating_add(dy));
    X.store(cx, Ordering::Relaxed);
    Y.store(cy, Ordering::Relaxed);
    let prev = BTNS.swap(buttons, Ordering::Relaxed);
    if buttons != prev {
        input::push_mouse_btn(buttons, cx, cy);
    }
    if cx != prev_x || cy != prev_y {
        input::push_mouse_move(cx, cy, buttons);
    }
}

/// Informe boot HID de 3 bytes (botones, dx, dy). El PS/2 da Y hacia arriba.
pub fn on_hid_boot_report(buttons: u8, dx: i8, dy: i8) {
    apply_pointer(dx as i32, -(dy as i32), buttons_from_hid_byte(buttons));
}

pub fn on_byte(b: u8) {
    if !present() {
        return;
    }
    unsafe {
        // Bit 3 del primer byte siempre está a 1. Sin esto, un byte perdido
        // deja el paquete desfasado para siempre. 0xFA/0xFE son ACK/resend.
        if PKT_I == 0 && (b & 0x08 == 0 || b == 0xFA || b == 0xFE) {
            return;
        }
        PKT[PKT_I as usize] = b;
        PKT_I += 1;
        if PKT_I < 3 {
            return;
        }
        PKT_I = 0;
        on_hid_boot_report(PKT[0], PKT[1] as i8, PKT[2] as i8);
    }
}

fn seed_origin() {
    refresh_bounds();
    X.store(MAX_W.load(Ordering::Relaxed) / 2, Ordering::Relaxed);
    Y.store(MAX_H.load(Ordering::Relaxed) / 2, Ordering::Relaxed);
}

pub fn init() {
    if !super::kbd::i8042_ready() {
        return;
    }
    x86_64::instructions::interrupts::without_interrupts(|| {
        flush_out();
        write_cmd(0xA8);
        flush_out();
        if let Some(mut cfg) = read_config() {
            cfg |= 1 << 1;
            cfg &= !(1 << 5);
            write_config(cfg);
            flush_out();
        }
        write_aux(0xF6);
        let _ = read_data();
        write_aux(0xF4);
        let _ = read_data();
        seed_origin();
        OK.store(true, Ordering::Relaxed);
    });
    super::kbd::route_aux_irq();
    crate::println!("mouse: ps2 aux habilitado");
}

pub fn present() -> bool {
    OK.load(Ordering::Relaxed)
}
