//! Fibras cooperativas de kernel.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::asm;
use spin::Mutex;

const STACK_SIZE: usize = 64 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FiberCtx {
    rsp: u64,
    rbx: u64,
    rbp: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
}

enum FiberState {
    Runnable,
    Blocked,
    Dead,
}

struct Fiber {
    ctx: FiberCtx,
    _stack: Box<[u8; STACK_SIZE]>,
    state: FiberState,
}

struct Runtime {
    fibers: Vec<Fiber>,
    current: usize,
}

static RUNTIME: Mutex<Option<Runtime>> = Mutex::new(None);

struct SwitchRequest {
    from_idx: usize,
    to_idx: usize,
    from_ctx: usize,
    to_ctx: usize,
}

unsafe impl Sync for SwitchRequest {}

static SWITCH: Mutex<Option<SwitchRequest>> = Mutex::new(None);

pub fn init() {
    *RUNTIME.lock() = Some(Runtime {
        fibers: vec![Fiber {
            ctx: FiberCtx::default(),
            _stack: Box::new([0u8; STACK_SIZE]),
            state: FiberState::Runnable,
        }],
        current: 0,
    });
}

pub fn spawn_main(entry: fn()) {
    let mut rt = RUNTIME.lock();
    let rt = rt.as_mut().unwrap();
    let mut stack = Box::new([0u8; STACK_SIZE]);
    let top = stack.as_ptr() as u64 + STACK_SIZE as u64;
    let mut sp = top & !15;
    sp -= 8;
    unsafe {
        *(sp as *mut u64) = fiber_entry as u64;
        sp -= 8;
        *(sp as *mut u64) = entry as u64;
        for _ in 0..6 {
            sp -= 8;
            *(sp as *mut u64) = 0;
        }
    }
    rt.fibers.push(Fiber {
        ctx: FiberCtx { rsp: sp, ..FiberCtx::default() },
        _stack: stack,
        state: FiberState::Runnable,
    });
}

pub fn poll() {
    loop {
        let (from_idx, to_idx, from_ctx, to_ctx) = {
            let mut rt = RUNTIME.lock();
            let rt = rt.as_mut().unwrap();
            let n = rt.fibers.len();
            if n <= 1 {
                return;
            }
            let mut found = None;
            for i in 1..n {
                let idx = (rt.current + i) % n;
                if matches!(rt.fibers[idx].state, FiberState::Runnable) {
                    found = Some(idx);
                    break;
                }
            }
            let to_idx = match found {
                Some(i) => i,
                None => return,
            };
            let from_idx = rt.current;
            rt.current = to_idx;
            let from_ctx = &mut rt.fibers[from_idx].ctx as *mut FiberCtx as usize;
            let to_ctx = &rt.fibers[to_idx].ctx as *const FiberCtx as usize;
            (from_idx, to_idx, from_ctx, to_ctx)
        };
        *SWITCH.lock() = Some(SwitchRequest {
            from_idx,
            to_idx,
            from_ctx,
            to_ctx,
        });
        unsafe {
            fiber_switch(from_ctx as *mut FiberCtx, to_ctx as *const FiberCtx);
        }
        *SWITCH.lock() = None;
        let dead = {
            let rt = RUNTIME.lock();
            matches!(rt.as_ref().unwrap().fibers[to_idx].state, FiberState::Dead)
        };
        if dead {
            return;
        }
    }
}

pub fn yield_now() {
    let (from_ctx, to_ctx) = {
        let mut rt = RUNTIME.lock();
        let rt = rt.as_mut().unwrap();
        let n = rt.fibers.len();
        if n <= 1 {
            return;
        }
        let to_idx = (rt.current + 1) % n;
        if !matches!(rt.fibers[to_idx].state, FiberState::Runnable) {
            return;
        }
        let from_idx = rt.current;
        rt.current = to_idx;
        (
            &mut rt.fibers[from_idx].ctx as *mut FiberCtx,
            &rt.fibers[to_idx].ctx as *const FiberCtx,
        )
    };
    unsafe { fiber_switch(from_ctx, to_ctx) };
}

pub fn block_current() {
    {
        let mut rt = RUNTIME.lock();
        let rt = rt.as_mut().unwrap();
        rt.fibers[rt.current].state = FiberState::Blocked;
    }
    yield_now();
}

pub fn unblock_all() {
    let mut rt = RUNTIME.lock();
    let rt = rt.as_mut().unwrap();
    for f in &mut rt.fibers {
        if matches!(f.state, FiberState::Blocked) {
            f.state = FiberState::Runnable;
        }
    }
}

extern "C" fn fiber_entry(entry: fn()) -> ! {
    entry();
    {
        let mut rt = RUNTIME.lock();
        let rt = rt.as_mut().unwrap();
        rt.fibers[rt.current].state = FiberState::Dead;
    }
    loop {
        yield_now();
    }
}

unsafe fn fiber_switch(from: *mut FiberCtx, to: *const FiberCtx) {
    unsafe {
        asm!(
            "push rbx",
            "push rbp",
            "push r12",
            "push r13",
            "push r14",
            "push r15",
            "mov [rdi], rsp",
            "mov rsp, [rsi]",
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop rbp",
            "pop rbx",
            "ret",
            in("rdi") from,
            in("rsi") to,
            clobber_abi("C"),
        );
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_schedule() {
    yield_now();
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_yield() {
    yield_now();
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_msleep(ms: u32) {
    super::timer::sleep_ms(ms);
}
