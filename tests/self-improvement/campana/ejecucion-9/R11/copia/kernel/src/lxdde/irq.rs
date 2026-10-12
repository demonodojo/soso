//! IRQ diferida hacia fibras.

use alloc::vec::Vec;
use core::ffi::c_void;
use spin::Mutex;

type IrqHandlerFn = extern "C" fn(i32, *mut c_void) -> u32;

struct IrqReg {
    vector: u8,
    handler: IrqHandlerFn,
    dev: usize,
    pending: bool,
}

struct IrqList {
    items: Vec<IrqReg>,
}

unsafe impl Sync for IrqList {}

static IRQS: Mutex<IrqList> = Mutex::new(IrqList { items: Vec::new() });

pub fn init() {}

fn top_half(vector: u8) {
    let mut wake = false;
    {
        let mut irqs = IRQS.lock();
        for slot in irqs.items.iter_mut() {
            if slot.vector == vector {
                slot.pending = true;
                wake = true;
            }
        }
    }
    if wake {
        super::fiber::unblock_all();
    }
}

macro_rules! irq_stub {
    ($v:expr, $name:ident) => {
        fn $name() {
            top_half($v);
        }
    };
}

irq_stub!(0x42, stub_42);
irq_stub!(0x43, stub_43);
irq_stub!(0x44, stub_44);
irq_stub!(0x45, stub_45);
irq_stub!(0x46, stub_46);
irq_stub!(0x47, stub_47);
irq_stub!(0x48, stub_48);
irq_stub!(0x49, stub_49);
irq_stub!(0x4a, stub_4a);
irq_stub!(0x4b, stub_4b);
irq_stub!(0x4c, stub_4c);
irq_stub!(0x4d, stub_4d);
irq_stub!(0x4e, stub_4e);
irq_stub!(0x4f, stub_4f);
irq_stub!(0x50, stub_50);
irq_stub!(0x51, stub_51);
irq_stub!(0x52, stub_52);
irq_stub!(0x53, stub_53);
irq_stub!(0x54, stub_54);
irq_stub!(0x55, stub_55);
irq_stub!(0x56, stub_56);
irq_stub!(0x57, stub_57);
irq_stub!(0x58, stub_58);
irq_stub!(0x59, stub_59);
irq_stub!(0x5a, stub_5a);
irq_stub!(0x5b, stub_5b);
irq_stub!(0x5c, stub_5c);
irq_stub!(0x5d, stub_5d);
irq_stub!(0x5e, stub_5e);
irq_stub!(0x5f, stub_5f);
irq_stub!(0x60, stub_60);
irq_stub!(0x61, stub_61);

pub fn stub_for(vector: u8) -> Option<crate::arch::irq::IrqHandler> {
    Some(match vector {
        0x42 => stub_42,
        0x43 => stub_43,
        0x44 => stub_44,
        0x45 => stub_45,
        0x46 => stub_46,
        0x47 => stub_47,
        0x48 => stub_48,
        0x49 => stub_49,
        0x4a => stub_4a,
        0x4b => stub_4b,
        0x4c => stub_4c,
        0x4d => stub_4d,
        0x4e => stub_4e,
        0x4f => stub_4f,
        0x50 => stub_50,
        0x51 => stub_51,
        0x52 => stub_52,
        0x53 => stub_53,
        0x54 => stub_54,
        0x55 => stub_55,
        0x56 => stub_56,
        0x57 => stub_57,
        0x58 => stub_58,
        0x59 => stub_59,
        0x5a => stub_5a,
        0x5b => stub_5b,
        0x5c => stub_5c,
        0x5d => stub_5d,
        0x5e => stub_5e,
        0x5f => stub_5f,
        0x60 => stub_60,
        0x61 => stub_61,
        _ => return None,
    })
}

pub fn poll() {
    let pending: Vec<(IrqHandlerFn, i32, *mut c_void)> = {
        let mut irqs = IRQS.lock();
        let mut list = Vec::new();
        for slot in irqs.items.iter_mut() {
            if slot.pending {
                slot.pending = false;
                list.push((slot.handler, slot.vector as i32, slot.dev as *mut c_void));
            }
        }
        list
    };
    for (handler, irq, dev) in pending {
        let _ = handler(irq, dev);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_request_irq(
    irq: u32,
    handler: IrqHandlerFn,
    _flags: u64,
    _name: *const u8,
    dev: *mut c_void,
) -> i32 {
    let vector = irq as u8;
    IRQS.lock().items.push(IrqReg {
        vector,
        handler,
        dev: dev as usize,
        pending: false,
    });
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_free_irq(irq: u32, _dev: *mut c_void) {
    let v = irq as u8;
    IRQS.lock().items.retain(|s| s.vector != v);
}

#[unsafe(no_mangle)]
pub extern "C" fn lx_irq_wake(irq: u32) {
    top_half(irq as u8);
}
