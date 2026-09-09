//! Teclado PS/2 (i8042) y USB HID — scancode set 1 → UTF-8 vía [`keymap`].
//!
//! IRQ1 vía IOAPIC en placa; respaldo por polling en `read_byte` (QEMU/edge).

use crate::arch::{apic, ioapic, irq};
use crate::drivers::keymap::{self, KeymapState, Layout};
use crate::println;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use spin::Mutex;
use x86_64::instructions::interrupts::without_interrupts;
use x86_64::instructions::port::Port;

const DATA: u16 = 0x60;
const STATUS: u16 = 0x64;
const CMD: u16 = 0x64;

const ST_OUT_FULL: u8 = 1;
const ST_IN_FULL: u8 = 2;
const ST_AUX_DATA: u8 = 1 << 5;

const CTR_KBD_INT: u8 = 1 << 0;
const CTR_AUX_INT: u8 = 1 << 1;
const CTR_KBD_DIS: u8 = 1 << 4;
const CTR_AUX_DIS: u8 = 1 << 5;
const CTR_XLATE: u8 = 1 << 6;

const PS2_ACK: u8 = 0xFA;
const PS2_RESEND: u8 = 0xFE;

const PS2_IO_TIMEOUT_US: u64 = 200_000;
const IO_WAIT_US: u64 = 10_000;

struct RxQueue {
    buf: [u8; 256],
    head: usize,
    tail: usize,
}

impl RxQueue {
    const fn new() -> Self {
        Self {
            buf: [0; 256],
            head: 0,
            tail: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        let next = (self.head + 1) % self.buf.len();
        if next != self.tail {
            self.buf[self.head] = byte;
            self.head = next;
        }
    }

    fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            return None;
        }
        let byte = self.buf[self.tail];
        self.tail = (self.tail + 1) % self.buf.len();
        Some(byte)
    }

    fn has_data(&self) -> bool {
        self.head != self.tail
    }
}

static RX: Mutex<RxQueue> = Mutex::new(RxQueue::new());
static KM: Mutex<KeymapState> = Mutex::new(KeymapState::new());
static I8042_OK: AtomicBool = AtomicBool::new(false);
static EXTENDED: AtomicBool = AtomicBool::new(false);
static SC_LOG: AtomicU8 = AtomicU8::new(0);
static SC_HIST: [AtomicU8; 8] = [const { AtomicU8::new(0) }; 8];

static SC_TOTAL: AtomicU32 = AtomicU32::new(0);
static SC_ULTIMO: AtomicU8 = AtomicU8::new(0);
static CH_ENCOLADOS: AtomicU32 = AtomicU32::new(0);
static CH_ENTREGADOS: AtomicU32 = AtomicU32::new(0);

fn status() -> u8 {
    unsafe { Port::<u8>::new(STATUS).read() }
}

fn read_data() -> u8 {
    unsafe { Port::<u8>::new(DATA).read() }
}

fn write_cmd(b: u8) {
    wait_input_empty();
    unsafe {
        Port::<u8>::new(CMD).write(b);
    }
}

fn write_data(b: u8) {
    wait_input_empty();
    unsafe {
        Port::<u8>::new(DATA).write(b);
    }
}

fn i8042_present() -> bool {
    status() != 0xFF
}

fn deadline_us(us: u64) -> u64 {
    crate::arch::tsc::now_ns().saturating_add(us.saturating_mul(1000))
}

fn wait_input_empty() {
    wait_input_empty_timeout(IO_WAIT_US);
}

fn wait_input_empty_timeout(us: u64) {
    if !i8042_present() {
        return;
    }
    let fin = deadline_us(us);
    loop {
        if status() & ST_IN_FULL == 0 {
            return;
        }
        if crate::arch::tsc::now_ns() >= fin {
            return;
        }
        core::hint::spin_loop();
    }
}

fn wait_output_full_timeout(us: u64) -> bool {
    if !i8042_present() {
        return false;
    }
    let fin = deadline_us(us);
    loop {
        if status() & ST_OUT_FULL != 0 {
            return true;
        }
        if crate::arch::tsc::now_ns() >= fin {
            return false;
        }
        core::hint::spin_loop();
    }
}

fn flush_output() {
    for _ in 0..64 {
        if !i8042_present() || status() & ST_OUT_FULL == 0 {
            break;
        }
        let _ = read_data();
    }
}

fn read_config() -> Option<u8> {
    flush_output();
    wait_input_empty();
    write_cmd(0x20);
    if wait_output_full_timeout(PS2_IO_TIMEOUT_US) {
        Some(read_data())
    } else {
        None
    }
}

fn write_config(b: u8) {
    wait_input_empty();
    write_cmd(0x60);
    wait_input_empty();
    write_data(b);
}

fn ps2_send(b: u8, want_ack: bool) -> bool {
    for _ in 0..3 {
        flush_output();
        wait_input_empty_timeout(IO_WAIT_US);
        write_data(b);
        if !want_ack {
            return true;
        }
        if !wait_output_full_timeout(PS2_IO_TIMEOUT_US) {
            continue;
        }
        match read_data() {
            PS2_ACK => return true,
            PS2_RESEND => continue,
            _ => continue,
        }
    }
    false
}

fn atkbd_probe() {
    flush_output();
    if !ps2_send(0xF2, true) {
        println!("kbd: GETID sin ACK (ok)");
        return;
    }
    if wait_output_full_timeout(PS2_IO_TIMEOUT_US) {
        let id0 = read_data();
        println!("kbd: id0={id0:#04x}");
        if wait_output_full_timeout(PS2_IO_TIMEOUT_US) {
            let id1 = read_data();
            println!("kbd: id1={id1:#04x}");
        }
    } else {
        println!("kbd: GETID sin datos (ok)");
    }
    flush_output();
}

fn atkbd_enable() {
    for attempt in 0..3 {
        if ps2_send(0xF4, true) {
            println!("kbd: enable scan OK");
            return;
        }
        println!("kbd: enable scan intento {} sin ACK", attempt + 1);
    }
    println!("kbd: enable scan sin ACK (ok en muchos portátiles)");
}

fn log_scancode_raw(sc: u8) {
    SC_TOTAL.fetch_add(1, Ordering::Relaxed);
    SC_ULTIMO.store(sc, Ordering::Relaxed);
    let n = SC_LOG.load(Ordering::Relaxed) as usize;
    if n < SC_HIST.len() {
        SC_HIST[n].store(sc, Ordering::Relaxed);
        SC_LOG.store(n as u8 + 1, Ordering::Relaxed);
    }
}

pub fn latido() -> (u32, u8, u32, u32) {
    (
        SC_TOTAL.load(Ordering::Relaxed),
        SC_ULTIMO.load(Ordering::Relaxed),
        CH_ENCOLADOS.load(Ordering::Relaxed),
        CH_ENTREGADOS.load(Ordering::Relaxed),
    )
}

pub fn scancodes_iniciales(out: &mut [u8]) -> usize {
    let n = (SC_LOG.load(Ordering::Relaxed) as usize)
        .min(out.len())
        .min(SC_HIST.len());
    for (i, o) in out.iter_mut().take(n).enumerate() {
        *o = SC_HIST[i].load(Ordering::Relaxed);
    }
    n
}

pub fn layout_name() -> &'static str {
    keymap::layout().name()
}

pub fn set_layout(name: &str) -> bool {
    if let Some(l) = Layout::parse(name) {
        keymap::set_layout(l);
        true
    } else {
        false
    }
}

fn ctrl_byte(out: keymap::KeyOutput) -> keymap::KeyOutput {
    use keymap::KeyOutput;
    match out {
        KeyOutput::Byte(b @ b'a'..=b'z') | KeyOutput::Byte(b @ b'A'..=b'Z') => {
            KeyOutput::Byte(b & 0x1F)
        }
        KeyOutput::Byte(b' ') => KeyOutput::Byte(0),
        KeyOutput::Char(c) if c.is_ascii_alphabetic() => {
            KeyOutput::Byte((c.to_ascii_lowercase() as u8) & 0x1F)
        }
        other => other,
    }
}

fn emit_tty_byte(b: u8) {
    if b == 0x03 {
        crate::task::note_serial_sigint();
        crate::task::kick_scheduler();
        return;
    }
    let mut rx = RX.lock();
    rx.push(b);
    CH_ENCOLADOS.fetch_add(1, Ordering::Relaxed);
}

fn enqueue_output(km: &mut KeymapState, out: keymap::KeyOutput) {
    let out = if km.ctrl() { ctrl_byte(out) } else { out };
    let mut buf = [0u8; 4];
    let n = keymap::output_bytes(out, &mut buf);
    if n == 0 {
        return;
    }
    for &b in &buf[..n] {
        emit_tty_byte(b);
        if crate::drivers::fb::graphics_mode() {
            crate::drivers::input::push_key(b as u32, true);
        }
    }
    while let Some(pending) = km.take_pending() {
        let pending = if km.ctrl() {
            ctrl_byte(pending)
        } else {
            pending
        };
        let n2 = keymap::output_bytes(pending, &mut buf);
        for &b in &buf[..n2] {
            emit_tty_byte(b);
            if crate::drivers::fb::graphics_mode() {
                crate::drivers::input::push_key(b as u32, true);
            }
        }
    }
    crate::task::kick_if_tty_waiting();
}

fn handle_make_scancode(sc: u8) {
    let mut km = KM.lock();
    let out = km.translate(sc);
    enqueue_output(&mut km, out);
}

fn handle_scancode(sc: u8) {
    log_scancode_raw(sc);

    if sc == 0xE0 {
        EXTENDED.store(true, Ordering::Relaxed);
        return;
    }

    let extended = EXTENDED.swap(false, Ordering::Relaxed);

    // Break (bit 7) en set 1 traducido por i8042.
    if sc & 0x80 != 0 {
        let code = sc & 0x7F;
        let mut km = KM.lock();
        match code {
            0x2A | 0x36 => km.shift_press(false),
            0x1D => km.ctrl_press(false),
            0x38 if extended => km.altgr_press(false),
            _ => {}
        }
        return;
    }

    let mut km = KM.lock();
    match sc {
        0x2A | 0x36 => {
            km.shift_press(true);
            return;
        }
        0x1D => {
            km.ctrl_press(true);
            return;
        }
        0x38 if extended => {
            km.altgr_press(true);
            return;
        }
        0x38 => {
            // Left Alt: no activa AltGr en layout ES.
            return;
        }
        0x3A => {
            km.caps_toggle();
            return;
        }
        _ => {}
    }
    drop(km);

    handle_make_scancode(sc);
}

#[cfg(feature = "drv-usb")]
fn handle_usb_event(evt: crate::drivers::usb_storage::UsbKbdEvent) {
    if evt.scancode == 0 && evt.usage_id < 0xE0 {
        return;
    }
    log_scancode_raw(evt.scancode);
    let mut km = KM.lock();

    if evt.usage_id >= 0xE0 && evt.usage_id <= 0xE7 {
        match evt.usage_id {
            0xE1 | 0xE5 => km.shift_press(evt.pressed),
            0xE0 | 0xE4 => km.ctrl_press(evt.pressed),
            0xE6 => km.altgr_press(evt.pressed),
            _ => {}
        }
        return;
    }

    if !evt.pressed || evt.scancode == 0 {
        return;
    }

    let out = km.translate_scancode(evt.scancode, evt.shift, evt.altgr);
    enqueue_output(&mut km, out);
}

fn poll_hw() {
    if I8042_OK.load(Ordering::Relaxed) {
        while i8042_present() && status() & ST_OUT_FULL != 0 {
            let st = status();
            if st & ST_AUX_DATA != 0 {
                let b = read_data();
                crate::drivers::mouse::on_byte(b);
                continue;
            }
            let sc = read_data();
            if sc == PS2_ACK || sc == 0xAA || sc == PS2_RESEND {
                continue;
            }
            handle_scancode(sc);
        }
    }
    #[cfg(feature = "drv-usb")]
    while let Some(evt) = crate::drivers::usb_storage::poll_keyboard_event() {
        handle_usb_event(evt);
    }
}

pub fn handle_irq() {
    if !crate::arch::irq::desde_ring3() {
        return;
    }
    poll_hw();
}

pub fn read_byte() -> Option<u8> {
    let b = without_interrupts(|| {
        poll_hw();
        RX.lock().pop()
    });
    if b.is_some() {
        CH_ENTREGADOS.fetch_add(1, Ordering::Relaxed);
    }
    b
}

pub fn has_input() -> bool {
    without_interrupts(|| {
        poll_hw();
        RX.lock().has_data()
    })
}

fn setup_controller() -> Option<(u8, u8)> {
    flush_output();
    let before = read_config()?;

    let mut quiesce = before;
    quiesce |= CTR_KBD_DIS | CTR_AUX_DIS;
    quiesce &= !(CTR_KBD_INT | CTR_AUX_INT);
    write_config(quiesce);
    flush_output();
    crate::arch::tsc::spin_us(1000);

    let mut cfg = quiesce;
    cfg |= CTR_KBD_INT | CTR_XLATE;
    cfg &= !CTR_KBD_DIS;
    cfg |= CTR_AUX_DIS;
    cfg &= !CTR_AUX_INT;
    cfg &= !0x80;
    write_config(cfg);
    flush_output();

    write_cmd(0xAE);
    flush_output();

    atkbd_probe();
    atkbd_enable();
    flush_output();

    let after = read_config().unwrap_or(cfg);
    Some((before, after))
}

fn route_irq1() {
    if let Some(vec) = irq::allocate(handle_irq) {
        let dest = apic::id();
        match ioapic::route_isa(1, vec, dest) {
            Ok(()) => println!("kbd: ps2 irq1 vector={vec:#x} (ioapic)"),
            Err(e) => println!("kbd: ioapic irq1 fallo ({e}); pic"),
        }
        unmask_pic_irq1();
    } else {
        println!("kbd: sin vector libre; polling");
        unmask_pic_irq1();
    }
}

pub fn init() {
    let st = status();
    println!("kbd: i8042 status={st:#04x}");
    println!("kbd: layout {}", keymap::layout().name());
    if !i8042_present() {
        I8042_OK.store(false, Ordering::Relaxed);
        println!("kbd: sin i8042");
        route_irq1();
        #[cfg(feature = "drv-usb")]
        if crate::drivers::usb_storage::has_usb_keyboard() {
            println!("kbd: usb hid activo");
        }
        return;
    }

    I8042_OK.store(true, Ordering::Relaxed);
    wait_input_empty();

    match setup_controller() {
        Some((before, after)) => {
            println!("kbd: cfg antes={before:#04x} despues={after:#04x}");
        }
        None => {
            println!("kbd: sin respuesta del cfg; dejando BIOS + polling");
        }
    }

    route_irq1();
    println!("kbd: ps2 listo");
    #[cfg(feature = "drv-usb")]
    if crate::drivers::usb_storage::has_usb_keyboard() {
        println!("kbd: usb hid activo");
    }
}

fn unmask_pic_irq1() {
    use crate::arch::interrupts::PICS;
    without_interrupts(|| {
        let mut pics = PICS.lock();
        unsafe {
            pics.write_masks(!0b0001_0111, 0xFF);
        }
    });
}
