//! Ratón PS/2 (puerto aux i8042).

use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use x86_64::instructions::port::Port;

use super::input;

const DATA: u16 = 0x60;
const STATUS: u16 = 0x64;
const CMD: u16 = 0x64;

const ST_OUT_FULL: u8 = 1;
const ST_IN_FULL: u8 = 2;

static OK: AtomicBool = AtomicBool::new(false);
static X: AtomicI32 = AtomicI32::new(0);
static Y: AtomicI32 = AtomicI32::new(0);

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

pub fn on_byte(b: u8) {
    if !present() {
        return;
    }
    unsafe {
        PKT[PKT_I as usize] = b;
        PKT_I += 1;
        if PKT_I < 3 {
            return;
        }
        PKT_I = 0;
        let flags = PKT[0];
        let dx = PKT[1] as i8 as i32;
        let dy = -(PKT[2] as i8 as i32);
        let nx = X.load(Ordering::Relaxed) + dx;
        let ny = Y.load(Ordering::Relaxed) + dy;
        X.store(nx.max(0), Ordering::Relaxed);
        Y.store(ny.max(0), Ordering::Relaxed);
        input::push_mouse_move(nx.max(0), ny.max(0));
        let left = flags & 1 != 0;
        input::push_mouse_btn(1, left);
    }
}

pub fn init() {
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
    OK.store(true, Ordering::Relaxed);
    crate::println!("mouse: ps2 aux habilitado");
}

pub fn present() -> bool {
    OK.load(Ordering::Relaxed)
}
