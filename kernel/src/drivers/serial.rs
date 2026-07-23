//! Puerto serie COM1 (16550): la consola de soso hasta que exista SSH.
//!
//! TX por polling con macros print!/println!; RX por IRQ4 hacia una cola
//! que consumirá la kernel-shell (fase 2).

use spin::{Lazy, Mutex};
use uart_16550::SerialPort;
use x86_64::instructions::interrupts::without_interrupts;

const COM1: u16 = 0x3F8;

pub static SERIAL1: Lazy<Mutex<SerialPort>> = Lazy::new(|| {
    let mut port = unsafe { SerialPort::new(COM1) };
    port.init(); // habilita también la interrupción de datos recibidos
    Mutex::new(port)
});

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

/// Llamado desde el handler de IRQ4 (interrupciones ya deshabilitadas).
/// Drena la FIFO completa: pueden llegar varios bytes por interrupción.
pub fn handle_irq() {
    let mut port = SERIAL1.lock();
    let mut queue = RX_QUEUE.lock();
    while let Ok(byte) = port.try_receive() {
        queue.push(byte);
    }
}

/// Lee un byte recibido, si lo hay. No bloquea.
///
/// Además de la cola, drena la FIFO por polling: los 8259 son
/// edge-triggered y si la FIFO se llenó con las interrupciones aún
/// deshabilitadas no habrá flanco nuevo; el sondeo rearma la línea.
pub fn read_byte() -> Option<u8> {
    without_interrupts(|| {
        let mut port = SERIAL1.lock();
        let mut queue = RX_QUEUE.lock();
        while let Ok(byte) = port.try_receive() {
            queue.push(byte);
        }
        queue.pop()
    })
}

/// ¿Hay bytes pendientes de leer? Drena también la FIFO del UART, por si
/// la interrupción se perdió (los 8259 son edge-triggered).
pub fn has_input() -> bool {
    without_interrupts(|| {
        let mut port = SERIAL1.lock();
        let mut queue = RX_QUEUE.lock();
        while let Ok(byte) = port.try_receive() {
            queue.push(byte);
        }
        queue.head != queue.tail
    })
}

/// Bytes crudos hacia la consola (la tty de los procesos de usuario):
/// sin pasar por fmt, que rompería el UTF-8 multibyte.
pub fn write_bytes(data: &[u8]) {
    without_interrupts(|| {
        let mut port = SERIAL1.lock();
        for &b in data {
            port.send_raw(b);
        }
    });
    crate::drivers::fb::write_bytes(data);
}

struct DualConsole;

impl core::fmt::Write for DualConsole {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        {
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
