//! Workqueues → fibras cooperativas.

use alloc::vec::Vec;
use spin::Mutex;

#[repr(C)]
pub struct LxWork {
    pub fn_ptr: Option<extern "C" fn(*mut LxWork)>,
    pub pending: i32,
}

#[repr(C)]
pub struct LxDelayedWork {
    pub work: LxWork,
    pub deadline_ms: u64,
}

struct WorkQueue {
    items: Vec<usize>,
}

struct DelayedQueue {
    items: Vec<usize>,
}

// Solo BSP, cooperativo — raw pointers en statics.
unsafe impl Sync for WorkQueue {}
unsafe impl Sync for DelayedQueue {}

static QUEUE: Mutex<WorkQueue> = Mutex::new(WorkQueue { items: Vec::new() });
static DELAYED: Mutex<DelayedQueue> = Mutex::new(DelayedQueue { items: Vec::new() });

pub fn init() {}

#[unsafe(no_mangle)]
pub extern "C" fn lx_init_work(work: *mut LxWork, func: extern "C" fn(*mut LxWork)) {
    if work.is_null() {
        return;
    }
    unsafe {
        (*work).fn_ptr = Some(func);
        (*work).pending = 0;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_schedule_work(work: *mut LxWork) -> i32 {
    if work.is_null() {
        return 0;
    }
    unsafe {
        if (*work).pending == 0 {
            (*work).pending = 1;
            QUEUE.lock().items.push(work as usize);
        }
    }
    super::fiber::unblock_all();
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_flush_work(work: *mut LxWork) {
    while unsafe { !work.is_null() && (*work).pending != 0 } {
        super::fiber::yield_now();
        poll();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_init_delayed_work(dwork: *mut LxDelayedWork, func: extern "C" fn(*mut LxWork)) {
    if dwork.is_null() {
        return;
    }
    unsafe {
        lx_init_work(&mut (*dwork).work as *mut LxWork, func);
        (*dwork).deadline_ms = 0;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_schedule_delayed_work(dwork: *mut LxDelayedWork, delay_jiffies: u64) -> i32 {
    if dwork.is_null() {
        return 0;
    }
    let ms = super::timer::lx_jiffies_to_msecs(delay_jiffies) as u64;
    unsafe {
        (*dwork).deadline_ms = crate::arch::pit::uptime_ms() + ms;
        DELAYED.lock().items.push(dwork as usize);
    }
    super::fiber::unblock_all();
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_cancel_delayed_work(dwork: *mut LxDelayedWork) -> bool {
    if dwork.is_null() {
        return false;
    }
    let addr = dwork as usize;
    let mut d = DELAYED.lock();
    let before = d.items.len();
    d.items.retain(|&p| p != addr);
    before != d.items.len()
}

pub fn poll() {
    let now = crate::arch::pit::uptime_ms();
    {
        let mut delayed = DELAYED.lock();
        let ready: Vec<_> = delayed
            .items
            .iter()
            .filter(|&&p| unsafe { (*(p as *mut LxDelayedWork)).deadline_ms <= now })
            .copied()
            .collect();
        delayed.items.retain(|&p| unsafe { (*(p as *mut LxDelayedWork)).deadline_ms > now });
        for p in ready {
            lx_schedule_work(unsafe { &mut (*(p as *mut LxDelayedWork)).work as *mut LxWork });
        }
    }

    let works: Vec<_> = {
        let mut q = QUEUE.lock();
        q.items.drain(..).collect()
    };
    for addr in works {
        let work = addr as *mut LxWork;
        unsafe {
            if let Some(f) = (*work).fn_ptr {
                f(work);
            }
            (*work).pending = 0;
        }
    }
}
