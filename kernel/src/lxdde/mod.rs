//! Capa DDE (Driver Development Environment) — emulación lx_emul para drivers Linux.

mod completion;
mod dma;
mod drm;
mod fiber;
mod firmware;
mod gpu;
mod irq;
mod mem;
mod net;
mod pci;
mod printk;
mod timer;
mod workqueue;
#[cfg(feature = "lxdde")]
pub mod wifi;

use spin::Mutex;

static POLL_LOCK: Mutex<()> = Mutex::new(());

unsafe extern "C" {
    fn lx_spike_run();
    fn lx_testdrv_run();
    fn lx_e1000e_init_module() -> i32;
    fn lx_nouveau_init_module() -> i32;
}

/// Modo de la capa lx al arrancar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LxddeMode {
    Off,
    Spike,
    TestDrv,
    E1000e,
    Nouveau,
    Iwlwifi,
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
            LxddeMode::Nouveau => {
                let rc = lx_nouveau_init_module();
                crate::println!("lxdde: nouveau init rc={rc} phase={}", gpu::gsp_phase());
            }
            LxddeMode::Iwlwifi => {
                let rc = wifi::init();
                crate::println!("lxdde: iwlwifi init rc={rc} phase={}", wifi::phase());
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
    #[cfg(feature = "lxdde")]
    wifi::poll();
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

pub fn gsp_ready() -> bool {
    gpu::gsp_ready()
}

pub fn gsp_phase() -> &'static str {
    gpu::gsp_phase()
}

pub fn nouveau_vram_total() -> u64 {
    gpu::vram_total()
}

pub fn device_vram_free() -> u64 {
    gpu::device_vram_free()
}

pub fn device_buf_alloc(size: u64) -> Result<u64, ()> {
    gpu::device_buf_alloc(size)
}

pub fn device_buf_upload(va: u64, data: &[u8]) -> Result<(), ()> {
    gpu::device_buf_upload(va, data)
}

pub fn device_buf_free(va: u64) -> Result<(), ()> {
    gpu::device_buf_free(va)
}

pub fn gsp_fini() -> bool {
    gpu::gsp_fini()
}

pub fn wifi_present() -> bool {
    wifi::wifi_present()
}

pub fn wifi_alive() -> bool {
    wifi::alive()
}

pub fn wifi_mac() -> Option<[u8; 6]> {
    wifi::mac()
}

pub fn wifi_connected() -> bool {
    wifi::connected()
}

pub fn wifi_receive(buf: &mut [u8]) -> Option<usize> {
    wifi::receive(buf)
}

pub fn wifi_send(data: &[u8]) -> Result<(), ()> {
    wifi::send(data)
}

pub fn wifi_can_send() -> bool {
    wifi::can_send()
}

pub fn wifi_scan() -> i32 {
    wifi::scan()
}

pub fn wifi_scan_results() -> alloc::vec::Vec<(alloc::string::String, i8, u8, bool)> {
    wifi::scan_results()
}

pub fn wifi_phase() -> &'static str {
    wifi::phase()
}

pub fn poll_rx() {
    net::poll_rx();
}

pub fn notify_boot0(boot0: u32, device_id: u16) {
    gpu::notify_boot0(boot0, device_id);
}

pub fn submit_saxpy(a: f32, x: &[f32], y: &mut [f32]) -> Result<bool, ()> {
    gpu::submit_saxpy(a, x, y)
}

pub fn submit_matvec_f32(
    w: &[f32],
    rows: usize,
    cols: usize,
    x: &[f32],
    y: &mut [f32],
) -> Result<bool, ()> {
    gpu::submit_matvec_f32(w, rows, cols, x, y)
}

pub fn submit_matvec_resident(
    w_va: u64,
    rows: usize,
    cols: usize,
    x: &[f32],
    y: &mut [f32],
) -> Result<bool, ()> {
    gpu::submit_matvec_resident(w_va, rows, cols, x, y)
}
