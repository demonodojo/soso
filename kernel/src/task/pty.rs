//! Pseudo-terminales de userspace (molde de las colas SSH).

use alloc::collections::VecDeque;
use spin::Mutex;

use super::Console;
use crate::task;

pub const PTY_MAX: usize = 8;

struct Colas {
    rx: Mutex<VecDeque<u8>>,
    tx: Mutex<VecDeque<u8>>,
    master_open: Mutex<bool>,
}

static COLAS: [Colas; PTY_MAX] = [
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
        master_open: Mutex::new(false),
    },
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
        master_open: Mutex::new(false),
    },
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
        master_open: Mutex::new(false),
    },
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
        master_open: Mutex::new(false),
    },
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
        master_open: Mutex::new(false),
    },
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
        master_open: Mutex::new(false),
    },
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
        master_open: Mutex::new(false),
    },
    Colas {
        rx: Mutex::new(VecDeque::new()),
        tx: Mutex::new(VecDeque::new()),
        master_open: Mutex::new(false),
    },
];

static FG_PTY: [core::sync::atomic::AtomicU64; PTY_MAX] =
    [const { core::sync::atomic::AtomicU64::new(0) }; PTY_MAX];

fn colas(slot: usize) -> Option<&'static Colas> {
    COLAS.get(slot)
}

pub fn fg_pgid(slot: u8) -> u64 {
    FG_PTY
        .get(slot as usize)
        .map(|a| a.load(core::sync::atomic::Ordering::Relaxed))
        .unwrap_or(0)
}

pub fn set_fg_pgid(slot: u8, pgid: u64) {
    if let Some(a) = FG_PTY.get(slot as usize) {
        a.store(pgid, core::sync::atomic::Ordering::Relaxed);
    }
}

pub fn rx_has_data(slot: u8) -> bool {
    colas(slot as usize).is_some_and(|c| !c.rx.lock().is_empty())
}

pub fn rx_pop(slot: u8) -> Option<u8> {
    colas(slot as usize)?.rx.lock().pop_front()
}

pub fn tx_push(slot: u8, data: &[u8]) {
    let Some(c) = colas(slot as usize) else {
        return;
    };
    let mut tx = c.tx.lock();
    let mut prev = tx.back().copied();
    for &b in data {
        if b == b'\n' && prev != Some(b'\r') {
            tx.push_back(b'\r');
        }
        tx.push_back(b);
        prev = Some(b);
    }
}

pub fn alloc_slot() -> Result<u8, i64> {
    for slot in 0..PTY_MAX {
        let Some(c) = colas(slot) else {
            continue;
        };
        let mut open = c.master_open.lock();
        if *open {
            continue;
        }
        c.rx.lock().clear();
        c.tx.lock().clear();
        *open = true;
        return Ok(slot as u8);
    }
    Err(-soso_abi::EMFILE)
}

/// Maestros cuyo cierre se aplazó: un bit por ranura.
///
/// `close_master` pide `PROCS` (mata a quien cuelga de esa consola), y
/// `close_all_fds` se llama con `PROCS` ya cogido (salida, `kill`, planificador).
/// Cerrarlo ahí era un autobloqueo con las interrupciones apagadas: al salir
/// `soso-wm` con terminales abiertas la máquina entera se congelaba.
static CIERRES_PENDIENTES: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Anota el cierre de un maestro para hacerlo sin candados (`drenar_cierres`).
pub fn cerrar_master_diferido(slot: u8) {
    CIERRES_PENDIENTES.fetch_or(1 << slot, core::sync::atomic::Ordering::AcqRel);
}

/// Aplica los cierres aplazados. Hay que llamarla **sin** `PROCS` cogido.
pub fn drenar_cierres() {
    if CIERRES_PENDIENTES.load(core::sync::atomic::Ordering::Relaxed) == 0 {
        return;
    }
    let mask = CIERRES_PENDIENTES.swap(0, core::sync::atomic::Ordering::AcqRel);
    for slot in 0..PTY_MAX as u8 {
        if mask & (1 << slot) != 0 {
            close_master(slot);
        }
    }
}

pub fn close_master(slot: u8) {
    let Some(c) = colas(slot as usize) else {
        return;
    };
    *c.master_open.lock() = false;
    c.rx.lock().clear();
    c.tx.lock().clear();
    task::kill_console(Console::Pty(slot));
}

pub fn master_try_read(slot: u8, dst: &mut [u8]) -> usize {
    let Some(c) = colas(slot as usize) else {
        return 0;
    };
    let mut tx = c.tx.lock();
    let n = dst.len().min(tx.len());
    for i in 0..n {
        if let Some(b) = tx.pop_front() {
            dst[i] = b;
        }
    }
    n
}

pub fn master_write(slot: u8, data: &[u8]) {
    let Some(c) = colas(slot as usize) else {
        return;
    };
    let console = Console::Pty(slot);
    for &b in data {
        if b == 0x03 {
            task::signal_console(console, soso_abi::SIGINT as u8);
            continue;
        }
        c.rx.lock().push_back(b);
    }
    wake_tty_waiters(console);
}

/// Avisa al planificador de que hay entrada. **No** se pasa al lector a `Runnable`
/// aquí: el lector de tty bloqueado necesita que alguien le copie los bytes y le
/// ponga `rax`, y eso lo hace el bucle de `WaitingTty` del planificador. Despertarlo
/// a mano lo reanudaba con `rax` sin tocar y nada copiado: `sosh` en un PTY leía EOF
/// con la primera tecla y salía.
fn wake_tty_waiters(_console: Console) {
    task::kick_if_tty_waiting();
}
