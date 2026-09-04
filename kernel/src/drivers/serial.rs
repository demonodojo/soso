//! Puerto serie COM1 (16550): la consola de soso hasta que exista SSH.
//!
//! TX por polling con macros print!/println!; RX por IRQ4 hacia una cola
//! que consumirá la kernel-shell (fase 2).

use core::sync::atomic::{AtomicBool, Ordering};
use spin::{Lazy, Mutex};
use uart_16550::SerialPort;
use x86_64::instructions::interrupts::without_interrupts;
use x86_64::instructions::port::Port;

const COM1: u16 = 0x3F8;

/// ¿Hay un 16550 de verdad en COM1? En una placa sin puerto serie legacy los
/// `inb` a 0x3F8..0x3FF devuelven 0xFF, con lo que el bit DATA_READY del LSR
/// queda siempre a 1 y `try_receive` no devuelve `Err` jamás: drenar la FIFO
/// por polling sería un bucle infinito dentro de `without_interrupts`.
static SERIAL_OK: AtomicBool = AtomicBool::new(false);

/// Tope de bytes a drenar en una pasada. Aunque `probe_com1` se equivocase,
/// el kernel no puede volver a colgarse con las interrupciones deshabilitadas.
const DRAIN_MAX: usize = 512;

/// Detección de UART al estilo `autoconfig()` de Linux (8250_port.c): el
/// driver nunca programa un puerto sin comprobar antes que responde.
///
/// # Safety
/// Toca puertos de E/S; `base` debe ser una base de UART candidata.
unsafe fn probe_com1(base: u16) -> bool {
    // Un LSR a 0xFF es la firma de un puerto que nadie decodifica.
    if unsafe { Port::<u8>::new(base + 5).read() } == 0xFF {
        return false;
    }

    // Test del IER: los cuatro bits bajos deben aceptar 0x00 y 0x0F. Descarta
    // tanto un puerto que flota a 0xFF como uno que lee siempre 0x00.
    // (No se usa el registro scratch: un 8250 original no lo tiene y saldría
    // «ausente» siendo un UART bueno. El tope de DRAIN_MAX cubre el caso
    // contrario, un falso positivo.)
    let mut ier = Port::<u8>::new(base + 1);
    unsafe {
        let ier_saved = ier.read();
        ier.write(0x00);
        let ceros = ier.read() & 0x0F;
        ier.write(0x0F);
        let unos = ier.read() & 0x0F;
        ier.write(ier_saved);
        ceros == 0x00 && unos == 0x0F
    }
}

pub static SERIAL1: Lazy<Mutex<SerialPort>> = Lazy::new(|| {
    let presente = unsafe { probe_com1(COM1) };
    SERIAL_OK.store(presente, Ordering::Relaxed);
    let mut port = unsafe { SerialPort::new(COM1) };
    if presente {
        port.init(); // habilita también la interrupción de datos recibidos
    }
    Mutex::new(port)
});

/// ¿Hay COM1? Fuerza la detección la primera vez que se pregunta.
pub fn present() -> bool {
    Lazy::force(&SERIAL1);
    SERIAL_OK.load(Ordering::Relaxed)
}

// ---- cola de recepción (productor: IRQ4; consumidor: kernel-shell) ----

struct RxQueue {
    buf: [u8; 256],
    head: usize, // siguiente hueco de escritura
    tail: usize, // siguiente byte a leer
}

impl RxQueue {
    const fn new() -> Self {
        Self { buf: [0; 256], head: 0, tail: 0 }
    }

    fn push(&mut self, byte: u8) {
        if byte == 0x03 {
            crate::task::note_serial_sigint();
            return;
        }
        let next = (self.head + 1) % self.buf.len();
        if next != self.tail {
            self.buf[self.head] = byte;
            self.head = next;
        }
        // Cola llena: se descarta el byte (mejor que bloquear en una IRQ).
    }

    fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            return None;
        }
        let byte = self.buf[self.tail];
        self.tail = (self.tail + 1) % self.buf.len();
        Some(byte)
    }
}

static RX_QUEUE: Mutex<RxQueue> = Mutex::new(RxQueue::new());

/// Drena la FIFO del UART hacia la cola. Acotado a `DRAIN_MAX` bytes.
fn drain_fifo() {
    if !present() {
        return;
    }
    let mut port = SERIAL1.lock();
    let mut queue = RX_QUEUE.lock();
    for _ in 0..DRAIN_MAX {
        match port.try_receive() {
            Ok(byte) => queue.push(byte),
            Err(_) => break,
        }
    }
}

/// Llamado desde el handler de IRQ4 (interrupciones ya deshabilitadas).
/// Drena la FIFO completa: pueden llegar varios bytes por interrupción.
pub fn handle_irq() {
    drain_fifo();
}

/// Lee un byte recibido, si lo hay. No bloquea.
///
/// Además de la cola, drena la FIFO por polling: los 8259 son
/// edge-triggered y si la FIFO se llenó con las interrupciones aún
/// deshabilitadas no habrá flanco nuevo; el sondeo rearma la línea.
pub fn read_byte() -> Option<u8> {
    without_interrupts(|| {
        drain_fifo();
        if let Some(b) = RX_QUEUE.lock().pop() {
            return Some(b);
        }
        crate::drivers::kbd::read_byte()
    })
}

/// ¿Hay bytes pendientes de leer? Drena también la FIFO del UART, por si
/// la interrupción se perdió (los 8259 son edge-triggered).
pub fn has_input() -> bool {
    without_interrupts(|| {
        drain_fifo();
        let pendiente = {
            let queue = RX_QUEUE.lock();
            queue.head != queue.tail
        };
        pendiente || crate::drivers::kbd::has_input()
    })
}

/// Bytes crudos hacia la consola (la tty de los procesos de usuario):
/// sin pasar por fmt, que rompería el UTF-8 multibyte.
pub fn write_bytes(data: &[u8]) {
    without_interrupts(|| {
        crate::drivers::logbuf::append(data);
        write_bytes_raw(data);
    });
}

/// Consola sin pasar por el ring buffer (p. ej. volcado de `dmesg`).
pub fn write_bytes_raw(data: &[u8]) {
    if present() {
        let mut port = SERIAL1.lock();
        for &b in data {
            port.send_raw(b);
        }
    }
    crate::drivers::fb::write_bytes(data);
}

struct DualConsole;

impl core::fmt::Write for DualConsole {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        crate::drivers::logbuf::append(s.as_bytes());
        if present() {
            let mut port = SERIAL1.lock();
            port.write_str(s)?;
        }
        crate::drivers::fb::write_bytes(s.as_bytes());
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    // Sin interrupciones mientras se sostiene el lock: el handler de IRQ4
    // también lo toma, y en monocore eso sería un interbloqueo.
    without_interrupts(|| {
        DualConsole
            .write_fmt(args)
            .expect("fallo escribiendo en consola");
    });
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::drivers::serial::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
