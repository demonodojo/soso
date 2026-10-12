//! Mini net-core + puente a smoltcp para drivers lx (e1000e).

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;
use spin::Mutex;

const SKB_DATA: usize = 2048;

#[repr(C)]
pub struct LxNetDevice {
    pub priv_size: i32,
    pub mac: [u8; 6],
    open: Option<extern "C" fn(*mut LxNetDevice) -> i32>,
    stop: Option<extern "C" fn(*mut LxNetDevice) -> i32>,
    xmit: Option<extern "C" fn(*mut LxSkBuff, *mut LxNetDevice) -> i32>,
    queue_stopped: bool,
    carrier: bool,
    registered: bool,
    priv_data: Vec<u8>,
}

#[repr(C)]
pub struct LxSkBuff {
    data: Vec<u8>,
    headroom: usize,
    len: usize,
}

static NETDEV: Mutex<Option<usize>> = Mutex::new(None);
static RXQ: Mutex<VecDeque<Vec<u8>>> = Mutex::new(VecDeque::new());

pub fn init() {}

pub fn lx_netdev_registered() -> bool {
    NETDEV.lock().is_some_and(|p| p != 0)
}

pub fn lx_netdev_mac() -> Option<[u8; 6]> {
    let Some(addr) = *NETDEV.lock() else {
        return None;
    };
    if addr == 0 {
        return None;
    }
    let p = addr as *mut LxNetDevice;
    unsafe { Some((*p).mac) }
}

pub fn poll_rx() {
    unsafe extern "C" {
        fn lx_e1000_poll(adapter: *mut c_void);
        fn lx_e1000e_adapter() -> *mut c_void;
    }
    unsafe {
        let ad = lx_e1000e_adapter();
        if !ad.is_null() {
            lx_e1000_poll(ad);
        }
    }
}

pub fn lx_receive(buf: &mut [u8]) -> Option<usize> {
    let mut q = RXQ.lock();
    let pkt = q.pop_front()?;
    let n = pkt.len().min(buf.len());
    buf[..n].copy_from_slice(&pkt[..n]);
    Some(n)
}

pub fn lx_send(data: &[u8]) -> Result<(), ()> {
    let Some(addr) = *NETDEV.lock() else {
        return Err(());
    };
    if addr == 0 {
        return Err(());
    }
    let p = addr as *mut LxNetDevice;
    unsafe {
        let dev = &mut *p;
        if dev.queue_stopped {
            return Err(());
        }
        let skb = lx_alloc_skb(data.len() as u32, 0);
        if skb.is_null() {
            return Err(());
        }
        let payload = lx_skb_put(skb, data.len() as u32);
        if payload.is_null() {
            lx_kfree_skb(skb);
            return Err(());
        }
        core::ptr::copy_nonoverlapping(data.as_ptr(), payload, data.len());
        (*skb).len = data.len();
        if let Some(xmit) = dev.xmit {
            let rc = xmit(skb, dev);
            if rc == 0 {
                Ok(())
            } else {
                lx_kfree_skb(skb);
                Err(())
            }
        } else {
            lx_kfree_skb(skb);
            Err(())
        }
    }
}

pub fn lx_can_send() -> bool {
    let Some(addr) = *NETDEV.lock() else {
        return false;
    };
    if addr == 0 {
        return false;
    }
    let p = addr as *mut LxNetDevice;
    unsafe { !(*p).queue_stopped }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_alloc_etherdev(priv_size: i32) -> *mut LxNetDevice {
    let size = core::cmp::max(priv_size, 0) as usize;
    let dev = Box::new(LxNetDevice {
        priv_size,
        mac: [0; 6],
        open: None,
        stop: None,
        xmit: None,
        queue_stopped: true,
        carrier: false,
        registered: false,
        priv_data: vec![0u8; size],
    });
    Box::into_raw(dev)
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_netdev_priv(dev: *mut LxNetDevice) -> *mut c_void {
    if dev.is_null() {
        return core::ptr::null_mut();
    }
    unsafe { (*dev).priv_data.as_mut_ptr() as *mut c_void }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_register_netdev(dev: *mut LxNetDevice) -> i32 {
    if dev.is_null() {
        return -1;
    }
    unsafe {
        (*dev).registered = true;
        *NETDEV.lock() = Some(dev as usize);
        if let Some(open) = (*dev).open {
            return open(dev);
        }
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_unregister_netdev(dev: *mut LxNetDevice) {
    if dev.is_null() {
        return;
    }
    unsafe {
        if let Some(stop) = (*dev).stop {
            stop(dev);
        }
        (*dev).registered = false;
    }
    *NETDEV.lock() = None;
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_netif_rx(skb: *mut LxSkBuff) {
    if skb.is_null() {
        return;
    }
    unsafe {
        let data = lx_skb_data(skb);
        let len = lx_skb_len(skb);
        if !data.is_null() && len > 0 {
            let slice = core::slice::from_raw_parts(data, len as usize);
            RXQ.lock().push_back(slice.to_vec());
        }
        lx_kfree_skb(skb);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_alloc_skb(size: u32, _gfp: u32) -> *mut LxSkBuff {
    let skb = Box::new(LxSkBuff {
        data: vec![0u8; size as usize + SKB_DATA],
        headroom: 0,
        len: 0,
    });
    Box::into_raw(skb)
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_kfree_skb(skb: *mut LxSkBuff) {
    if !skb.is_null() {
        unsafe {
            drop(Box::from_raw(skb));
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_skb_put(skb: *mut LxSkBuff, len: u32) -> *mut u8 {
    if skb.is_null() {
        return core::ptr::null_mut();
    }
    unsafe {
        let s = &mut *skb;
        let start = s.headroom + s.len;
        s.len += len as usize;
        s.data.as_mut_ptr().add(start)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_skb_reserve(skb: *mut LxSkBuff, len: i32) {
    if skb.is_null() {
        return;
    }
    unsafe {
        (*skb).headroom = (*skb).headroom.saturating_add(len as usize);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_skb_len(skb: *mut LxSkBuff) -> u32 {
    if skb.is_null() {
        return 0;
    }
    unsafe { (*skb).len as u32 }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_skb_data(skb: *mut LxSkBuff) -> *const u8 {
    if skb.is_null() {
        return core::ptr::null();
    }
    unsafe { (*skb).data.as_ptr().add((*skb).headroom) }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_netif_start_queue(dev: *mut LxNetDevice) {
    if !dev.is_null() {
        unsafe { (*dev).queue_stopped = false; }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_netif_stop_queue(dev: *mut LxNetDevice) {
    if !dev.is_null() {
        unsafe { (*dev).queue_stopped = true; }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_netif_queue_stopped(dev: *mut LxNetDevice) -> bool {
    if dev.is_null() {
        return true;
    }
    unsafe { (*dev).queue_stopped }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_netif_carrier_on(dev: *mut LxNetDevice) {
    if !dev.is_null() {
        unsafe { (*dev).carrier = true; }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_set_netdev_ops(
    dev: *mut LxNetDevice,
    open: extern "C" fn(*mut LxNetDevice) -> i32,
    stop: extern "C" fn(*mut LxNetDevice) -> i32,
    xmit: extern "C" fn(*mut LxSkBuff, *mut LxNetDevice) -> i32,
) {
    if dev.is_null() {
        return;
    }
    unsafe {
        (*dev).open = Some(open);
        (*dev).stop = Some(stop);
        (*dev).xmit = Some(xmit);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_set_netdev_mac(dev: *mut LxNetDevice, mac: *const u8) {
    if dev.is_null() || mac.is_null() {
        return;
    }
    unsafe {
        for i in 0..6 {
            (*dev).mac[i] = *mac.add(i);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_tx_head(_adapter: *mut c_void) -> u32 {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_dev_queue_xmit(_skb: *mut LxSkBuff) -> i32 {
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_eth_hw_addr_random(_dev: *mut LxNetDevice) {}

#[unsafe(no_mangle)]
pub extern "C" fn lx_eth_mac_addr(_dev: *mut LxNetDevice, _addr: *mut c_void) -> i32 {
    0
}
