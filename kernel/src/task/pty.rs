//! Pseudo-terminales de userspace (molde de las colas SSH).

use alloc::collections::VecDeque;
use spin::Mutex;

use super::{Console, State, PROCS};
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

fn wake_tty_waiters(console: Console) {
    let mut procs = PROCS.lock();
    for p in procs.iter_mut() {
        if p.console == console && matches!(p.state, State::WaitingTty { .. }) && console.has_input()
        {
            p.state = State::Runnable;
        }
    }
    drop(procs);
    task::kick_if_tty_waiting();
}
