//! Teclado PS/2 (i8042) — scancode set 1 → ASCII US.
//!
//! IRQ1 vía IOAPIC en placa; respaldo por polling en `read_byte` (QEMU/edge).
//! Init al estilo Linux: quiesce del controlador (`i8042_controller_init`),
//! capa ps2 con ACK/reintentos (`libps2`) y enable del dispositivo (`atkbd`).

use crate::arch::{apic, ioapic, irq};
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
/// Bit 5: dato del puerto AUX (ratón/touchpad), no del teclado.
const ST_AUX_DATA: u8 = 1 << 5;

/// Byte de configuración del i8042 (CTR).
const CTR_KBD_INT: u8 = 1 << 0;
const CTR_AUX_INT: u8 = 1 << 1;
const CTR_KBD_DIS: u8 = 1 << 4;
const CTR_AUX_DIS: u8 = 1 << 5;
const CTR_XLATE: u8 = 1 << 6;

const PS2_ACK: u8 = 0xFA;
const PS2_RESEND: u8 = 0xFE;

/// Espera por byte en salida del dispositivo (Linux libps2: 200 ms).
const PS2_IO_TIMEOUT_US: u64 = 200_000;
/// Espera corta de buffer de entrada del controlador.
const IO_WAIT_US: u64 = 10_000;

/// Fila numérica sin shift / con shift (scancodes 0x02..=0x0D).
const ROW1: [u8; 12] = *b"1234567890-=";
const ROW1_SHIFT: [u8; 12] = *b"!@#$%^&*()_+";

/// 0x10..=0x1B
const ROW_Q: [u8; 12] = *b"qwertyuiop[]";
const ROW_Q_SHIFT: [u8; 12] = *b"QWERTYUIOP{}";

/// 0x1E..=0x28 (hueco 0x27 = ;)
const ROW_A: [u8; 11] = *b"asdfghjkl;'";
const ROW_A_SHIFT: [u8; 11] = *b"ASDFGHJKL:\"";

/// 0x2C..=0x35
const ROW_Z: [u8; 10] = *b"zxcvbnm,./";
const ROW_Z_SHIFT: [u8; 10] = *b"ZXCVBNM<>?";

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
static SHIFT: AtomicBool = AtomicBool::new(false);
static CAPS: AtomicBool = AtomicBool::new(false);
static I8042_OK: AtomicBool = AtomicBool::new(false);
static SC_LOG: AtomicU8 = AtomicU8::new(0);
static SC_HIST: [AtomicU8; 8] = [const { AtomicU8::new(0) }; 8];

// Latido del camino teclado→tty, para que un cuelgue deje rastro. Son atómicos
// sueltos a propósito: los lee `fatlog` en cada volcado a SOSOLOG.TXT y el
// comando `kbd` del kshell, y ninguno de los dos puede permitirse un candado.
/// Scancodes vistos (todos, no solo los 8 del histórico).
static SC_TOTAL: AtomicU32 = AtomicU32::new(0);
/// Último scancode visto.
static SC_ULTIMO: AtomicU8 = AtomicU8::new(0);
/// Caracteres encolados hacia la tty.
static CH_ENCOLADOS: AtomicU32 = AtomicU32::new(0);
/// Caracteres entregados a quien leía.
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

/// Espera dato en el buffer de salida (bounded, resolución µs vía TSC).
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

/// Envía un byte al dispositivo teclado y espera ACK (libps2, 3 reintentos).
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

/// GETID (0xF2): tolerante a timeout — muchos ECs no responden.
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

/// Enable scanning (0xF4) con reintentos (atkbd).
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

/// Guarda los primeros scancodes para diagnóstico. **No imprime**: esto corre
/// dentro del handler de la IRQ 1, y `println!` toma el candado de la consola
/// (serie o framebuffer) que el proceso interrumpido puede tener cogido —
/// justo lo que pasa mientras la shell hace eco de lo que escribes. Interbloqueo
/// duro en la primera tecla, y encima con el mensaje ya pintado, que despista.
/// Se leen con `kbd` desde la kernel-shell.
fn log_scancode_raw(sc: u8) {
    SC_TOTAL.fetch_add(1, Ordering::Relaxed);
    SC_ULTIMO.store(sc, Ordering::Relaxed);
    let n = SC_LOG.load(Ordering::Relaxed) as usize;
    if n < SC_HIST.len() {
        SC_HIST[n].store(sc, Ordering::Relaxed);
        SC_LOG.store(n as u8 + 1, Ordering::Relaxed);
    }
}

/// Latido del camino teclado→tty: `(scancodes, último, encolados, entregados)`.
///
/// Si tras un cuelgue los scancodes siguen subiendo pero los entregados no, el
/// atasco está del lado de la tty/consola; si no sube ninguno, del lado del
/// sondeo o de la propia IRQ.
pub fn latido() -> (u32, u8, u32, u32) {
    (
        SC_TOTAL.load(Ordering::Relaxed),
        SC_ULTIMO.load(Ordering::Relaxed),
        CH_ENCOLADOS.load(Ordering::Relaxed),
        CH_ENTREGADOS.load(Ordering::Relaxed),
    )
}

/// Los primeros scancodes vistos, para `kbd` en la kernel-shell.
pub fn scancodes_iniciales(out: &mut [u8]) -> usize {
    let n = (SC_LOG.load(Ordering::Relaxed) as usize).min(out.len()).min(SC_HIST.len());
    for (i, o) in out.iter_mut().take(n).enumerate() {
        *o = SC_HIST[i].load(Ordering::Relaxed);
    }
    n
}

fn scancode_ascii(sc: u8) -> Option<u8> {
    let shift = SHIFT.load(Ordering::Relaxed) ^ CAPS.load(Ordering::Relaxed);
    match sc {
        0x02..=0x0D => {
            let i = (sc - 0x02) as usize;
            Some(if shift { ROW1_SHIFT[i] } else { ROW1[i] })
        }
        0x10..=0x1B => {
            let i = (sc - 0x10) as usize;
            Some(if shift { ROW_Q_SHIFT[i] } else { ROW_Q[i] })
        }
        0x1E..=0x28 => {
            let i = (sc - 0x1E) as usize;
            Some(if shift { ROW_A_SHIFT[i] } else { ROW_A[i] })
        }
        0x29 => Some(if shift { b'~' } else { b'`' }),
        0x2B => Some(if shift { b'|' } else { b'\\' }),
        0x2C..=0x35 => {
            let i = (sc - 0x2C) as usize;
            Some(if shift { ROW_Z_SHIFT[i] } else { ROW_Z[i] })
        }
        0x39 => Some(b' '),
        0x0E => Some(0x08),
        0x0F => Some(b'\t'),
        0x1C => Some(b'\n'),
        _ => None,
    }
}

fn handle_scancode(sc: u8) {
    log_scancode_raw(sc);
    if sc & 0x80 != 0 {
        match sc {
            0xAA | 0xB6 => SHIFT.store(false, Ordering::Relaxed),
            _ => {}
        }
        return;
    }
    match sc {
        0x2A | 0x36 => SHIFT.store(true, Ordering::Relaxed),
        0x3A => CAPS.store(!CAPS.load(Ordering::Relaxed), Ordering::Relaxed),
        _ => {
            if let Some(ch) = scancode_ascii(sc) {
                RX.lock().push(ch);
                CH_ENCOLADOS.fetch_add(1, Ordering::Relaxed);
                crate::task::kick_if_tty_waiting();
            }
        }
    }
}

fn poll_hw() {
    if I8042_OK.load(Ordering::Relaxed) {
        while i8042_present() && status() & ST_OUT_FULL != 0 {
            let st = status();
            if st & ST_AUX_DATA != 0 {
                let _ = read_data();
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
    while let Some(sc) = crate::drivers::usb_storage::poll_keyboard_scancode() {
        if sc != 0 {
            handle_scancode(sc);
        }
    }
}

/// Entrada de la IRQ del teclado, **común a los dos caminos**: IOAPIC (vector
/// 0x42 vía `irq::dispatch`, que es el que se usa en placa) y PIC legacy
/// (`kbd_pic_handler`, el de QEMU sin IOAPIC).
///
/// Sondear solo si veníamos de ring 3, la misma regla que `timer_tick`: desde
/// ring 0 el kernel puede tener cogido `PROCS` (una syscall), `HOSTS` (una
/// lectura del disco live) o el candado de la consola, y el sondeo vuelve a
/// pedirlos — mismo core, spinlock no reentrante, máquina clavada.
///
/// AVERÍA: esta guarda estaba puesta solo en `kbd_pic_handler`, que en placa
/// **no se ejecuta nunca** porque la IRQ 1 va por el IOAPIC (`kbd: ps2 irq1
/// vector=0x42 (ioapic)` en el log de arranque). Con eso el arreglo no llegaba
/// a la máquina que fallaba.
///
/// No se pierde la tecla: el scancode se queda en el búfer del i8042 y lo
/// recoge el siguiente sondeo, que hacen `has_input` y `read_byte`.
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

/// Init del i8042 como Linux: quiesce → CTR final → puerto KBD → atkbd.
/// No self-test del controlador ni reset 0xFF (rompe ECs en portátiles).
fn setup_controller() -> Option<(u8, u8)> {
    flush_output();
    let before = read_config()?;

    // Quiesce: silenciar interfaces e IRQs antes de reconfigurar.
    let mut quiesce = before;
    quiesce |= CTR_KBD_DIS | CTR_AUX_DIS;
    quiesce &= !(CTR_KBD_INT | CTR_AUX_INT);
    write_config(quiesce);
    flush_output();
    crate::arch::tsc::spin_us(1000);

    // CTR final: IRQ1 + traducción set1 + teclado on + ratón off.
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

/// Inicializa i8042 y enruta IRQ1. Seguro llamar sin IOAPIC (solo polling).
pub fn init() {
    let st = status();
    println!("kbd: i8042 status={st:#04x}");
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
