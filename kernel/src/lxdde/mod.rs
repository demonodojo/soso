//! Capa DDE (Driver Development Environment) — emulación lx_emul para drivers Linux.

mod completion;
mod dma;
mod drm;
mod fiber;
mod firmware;
mod irq;
mod mem;
mod net;
mod pci;
mod printk;
mod timer;
mod workqueue;

use spin::Mutex;

static POLL_LOCK: Mutex<()> = Mutex::new(());

unsafe extern "C" {
    fn lx_spike_run();
    fn lx_testdrv_run();
    fn lx_e1000e_init_module() -> i32;
}

/// Modo de la capa lx al arrancar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LxddeMode {
    Off,
    Spike,
    TestDrv,
    E1000e,
}

/// Inicializa la capa lx_emul.
pub fn init(mode: LxddeMode) {
    if mode == LxddeMode::Off {
        return;
    }
    fiber::init();
    timer::init();
    workqueue::init();
    irq::init();
    firmware::init();
    drm::init();
    net::init();

    unsafe {
        match mode {
            LxddeMode::Spike => fiber::spawn_main(|| lx_spike_run()),
            LxddeMode::TestDrv => fiber::spawn_main(|| lx_testdrv_run()),
            LxddeMode::E1000e => {
                let _ = lx_e1000e_init_module();
            }
            LxddeMode::Off => {}
        }
    }

    pci::init();

    if matches!(mode, LxddeMode::Spike | LxddeMode::TestDrv) {
        poll();
    }
}

/// Bomba cooperativa: timers, workqueues, fibras e IRQ threads.
pub fn poll() {
    if crate::arch::percpu::cpu_index() != 0 {
        return;
    }
    let Some(_g) = POLL_LOCK.try_lock() else {
        return;
    };
    timer::tick();
    workqueue::poll();
    irq::poll();
    net::poll_rx();
}

pub fn e1000e_present() -> bool {
    net::lx_netdev_registered()
}

pub fn e1000e_mac() -> Option<[u8; 6]> {
    net::lx_netdev_mac()
}

pub fn e1000e_receive(buf: &mut [u8]) -> Option<usize> {
    net::lx_receive(buf)
}

pub fn e1000e_send(data: &[u8]) -> Result<(), ()> {
    net::lx_send(data)
}

pub fn e1000e_can_send() -> bool {
    net::lx_can_send()
}

pub fn poll_rx() {
    net::poll_rx();
}
