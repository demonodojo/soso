//! TCP loopback userspace: `connect(127.0.0.1:port)` empareja con un
//! `listen(port)` sin pasar por la NIC ni smoltcp.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use spin::Mutex;

const LOOPBACK_BUF: usize = 65536;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoopSide {
    Client,
    Server,
}

pub struct LoopPair {
    /// Cliente → servidor.
    c2s: Vec<u8>,
    c2s_read: usize,
    /// Servidor → cliente.
    s2c: Vec<u8>,
    s2c_read: usize,
    closed_client: bool,
    closed_server: bool,
}

impl LoopPair {
    pub(crate) fn new() -> Self {
        Self {
            c2s: Vec::new(),
            c2s_read: 0,
            s2c: Vec::new(),
            s2c_read: 0,
            closed_client: false,
            closed_server: false,
        }
    }

    fn compact(side: LoopSide, pair: &mut LoopPair) {
        match side {
            LoopSide::Client => {
                if pair.c2s_read > 0 {
                    pair.c2s.drain(0..pair.c2s_read);
                    pair.c2s_read = 0;
                }
            }
            LoopSide::Server => {
                if pair.s2c_read > 0 {
                    pair.s2c.drain(0..pair.s2c_read);
                    pair.s2c_read = 0;
                }
            }
        }
    }
}

struct LoopState {
    pairs: Vec<Option<LoopPair>>,
    /// listener_slot → cola de slots servidor pendientes de `accept`.
    pending: BTreeMap<usize, Vec<usize>>,
}

static LOOP: Mutex<LoopState> = Mutex::new(LoopState {
    pairs: Vec::new(),
    pending: BTreeMap::new(),
});

pub fn is_loopback_addr(addr: &[u8; 4]) -> bool {
    addr[0] == 127
}

pub fn alloc_pair() -> usize {
    let mut st = LOOP.lock();
    if let Some(i) = st.pairs.iter().position(|p| p.is_none()) {
        st.pairs[i] = Some(LoopPair::new());
        return i;
    }
    let id = st.pairs.len();
    st.pairs.push(Some(LoopPair::new()));
    id
}

pub fn register_pending(listener_slot: usize, server_slot: usize) {
    LOOP.lock()
        .pending
        .entry(listener_slot)
        .or_default()
        .push(server_slot);
}

pub fn take_pending(listener_slot: usize) -> Option<usize> {
    LOOP.lock().pending.get_mut(&listener_slot)?.pop()
}

pub fn has_pending(listener_slot: usize) -> bool {
    LOOP.lock()
        .pending
        .get(&listener_slot)
        .is_some_and(|q| !q.is_empty())
}

pub fn try_read(pair_id: usize, side: LoopSide, buf: u64, len: u64) -> Result<u64, i64> {
    let mut st = LOOP.lock();
    let Some(pair) = st.pairs.get_mut(pair_id).and_then(|p| p.as_mut()) else {
        return Err(-soso_abi::EBADF);
    };
    let (data, read_pos, peer_closed) = match side {
        LoopSide::Client => (&pair.s2c, &mut pair.s2c_read, pair.closed_server),
        LoopSide::Server => (&pair.c2s, &mut pair.c2s_read, pair.closed_client),
    };
    if data.len() <= *read_pos {
        if peer_closed {
            return Ok(0);
        }
        return Ok(0);
    }
    let avail = data.len() - *read_pos;
    let n = (len as usize).min(avail).min(4096);
    for i in 0..n {
        unsafe {
            *((buf + i as u64) as *mut u8) = data[*read_pos + i];
        }
    }
    *read_pos += n;
    LoopPair::compact(side, pair);
    Ok(n as u64)
}

pub fn try_write(pair_id: usize, side: LoopSide, buf: u64, len: u64) -> Result<u64, i64> {
    let mut st = LOOP.lock();
    let Some(pair) = st.pairs.get_mut(pair_id).and_then(|p| p.as_mut()) else {
        return Err(-soso_abi::EBADF);
    };
    let closed = match side {
        LoopSide::Client => pair.closed_server,
        LoopSide::Server => pair.closed_client,
    };
    if closed {
        return Err(-soso_abi::EPIPE);
    }
    let dst = match side {
        LoopSide::Client => &mut pair.c2s,
        LoopSide::Server => &mut pair.s2c,
    };
    if dst.len() >= LOOPBACK_BUF {
        return Ok(0);
    }
    let room = LOOPBACK_BUF - dst.len();
    let n = (len as usize).min(room).min(4096);
    for i in 0..n {
        let b = unsafe { *((buf + i as u64) as *const u8) };
        dst.push(b);
    }
    Ok(n as u64)
}

pub fn close_side(pair_id: usize, side: LoopSide) {
    let mut st = LOOP.lock();
    let Some(pair) = st.pairs.get_mut(pair_id).and_then(|p| p.as_mut()) else {
        return;
    };
    match side {
        LoopSide::Client => pair.closed_client = true,
        LoopSide::Server => pair.closed_server = true,
    }
    if pair.closed_client && pair.closed_server {
        st.pairs[pair_id] = None;
    }
}
