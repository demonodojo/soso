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
    client_pid: u64,
    server_pid: u64,
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
            client_pid: 0,
            server_pid: 0,
        }
    }

    pub fn set_client_pid(pair: &mut LoopPair, pid: u64) {
        pair.client_pid = pid;
    }

    pub fn set_server_pid(pair: &mut LoopPair, pid: u64) {
        pair.server_pid = pid;
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

pub fn bind_client_pid(pair_id: usize, pid: u64) {
    if let Some(pair) = LOOP.lock().pairs.get_mut(pair_id).and_then(|p| p.as_mut()) {
        LoopPair::set_client_pid(pair, pid);
    }
}

pub fn bind_server_pid(pair_id: usize, pid: u64) {
    if let Some(pair) = LOOP.lock().pairs.get_mut(pair_id).and_then(|p| p.as_mut()) {
        LoopPair::set_server_pid(pair, pid);
    }
}

pub fn peer_pid(pair_id: usize, side: LoopSide) -> u64 {
    let st = LOOP.lock();
    let Some(pair) = st.pairs.get(pair_id).and_then(|p| p.as_ref()) else {
        return 0;
    };
    match side {
        LoopSide::Client => pair.server_pid,
        LoopSide::Server => pair.client_pid,
    }
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
    let (data, read_pos) = match side {
        LoopSide::Client => (&pair.s2c, &mut pair.s2c_read),
        LoopSide::Server => (&pair.c2s, &mut pair.c2s_read),
    };
    if data.len() <= *read_pos {
        return Ok(0);
    }
    let avail = data.len() - *read_pos;
    let n = (len as usize).min(avail).min(4096);
    let slice = &data[*read_pos..*read_pos + n];
    if crate::task::uaccess::copy_to_user(buf, slice).is_err() {
        return Err(-soso_abi::EFAULT);
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
    let mut tmp = [0u8; 4096];
    if crate::task::uaccess::copy_from_user(buf, &mut tmp[..n]).is_err() {
        return Err(-soso_abi::EFAULT);
    }
    dst.extend_from_slice(&tmp[..n]);
    Ok(n as u64)
}

/// ¿El otro extremo ya llamó `close`?
pub fn peer_closed(pair_id: usize, side: LoopSide) -> bool {
    let st = LOOP.lock();
    let Some(pair) = st.pairs.get(pair_id).and_then(|p| p.as_ref()) else {
        return true;
    };
    match side {
        LoopSide::Client => pair.closed_server,
        LoopSide::Server => pair.closed_client,
    }
}

/// ¿Quedan bytes por leer en este lado?
pub fn has_unread(pair_id: usize, side: LoopSide) -> bool {
    let st = LOOP.lock();
    let Some(pair) = st.pairs.get(pair_id).and_then(|p| p.as_ref()) else {
        return false;
    };
    match side {
        LoopSide::Client => pair.s2c.len() > pair.s2c_read,
        LoopSide::Server => pair.c2s.len() > pair.c2s_read,
    }
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
