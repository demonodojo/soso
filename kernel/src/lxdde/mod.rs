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

/// Puertos lxdde activos (compile-time vía `SOSO_LXDDE_MODE`, coma-separado).
#[derive(Clone, Copy, Default)]
pub struct LxddeModes {
    pub spike: bool,
    pub testdrv: bool,
    pub e1000e: bool,
    pub nouveau: bool,
    pub iwlwifi: bool,
}

impl LxddeModes {
    pub fn from_env() -> Self {
        let s = option_env!("SOSO_LXDDE_MODE").unwrap_or("");
        Self {
            spike: s.contains("spike"),
            testdrv: s.contains("testdrv"),
            e1000e: s.contains("e1000e"),
            nouveau: s.contains("nouveau"),
            iwlwifi: s.contains("iwlwifi"),
        }
    }

    pub fn is_off(self) -> bool {
        !self.spike && !self.testdrv && !self.e1000e && !self.nouveau && !self.iwlwifi
    }
}

/// Inicializa la capa lx_emul.
pub fn init(modes: LxddeModes) {
    if modes.is_off() {
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
        if modes.spike {
            fiber::spawn_main(|| lx_spike_run());
        }
        if modes.testdrv {
            fiber::spawn_main(|| lx_testdrv_run());
        }
        if modes.e1000e {
            let _ = lx_e1000e_init_module();
        }
        if modes.nouveau {
            let rc = lx_nouveau_init_module();
            crate::println!("lxdde: nouveau init rc={rc} phase={}", gpu::gsp_phase());
        }
        if modes.iwlwifi {
            let rc = wifi::init();
            crate::println!("lxdde: iwlwifi register rc={rc}");
        }
    }

    pci::init();

    if modes.iwlwifi {
        let rc = wifi::start_firmware();
        crate::println!(
            "lxdde: iwlwifi start rc={rc} phase={} alive={}",
            wifi::phase(),
            wifi::alive()
        );
    }

    if modes.spike || modes.testdrv {
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

pub fn device_bufs_ready() -> bool {
    gpu::device_bufs_ready()
}

pub fn device_buf_alloc(size: u64) -> Result<u64, ()> {
    gpu::device_buf_alloc(size)
}

pub fn device_buf_upload_at(va: u64, offset: u64, data: &[u8]) -> Result<(), ()> {
    gpu::device_buf_upload_at(va, offset, data)
}

pub fn device_buf_upload_dma(
    va: u64,
    offset: u64,
    phys: &[u64],
    src_off: u32,
    size: u64,
) -> Result<(), ()> {
    gpu::device_buf_upload_dma(va, offset, phys, src_off, size)
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

pub fn submit_matvec_q_resident(
    w_va: u64,
    dtype: u8,
    rows: usize,
    cols: usize,
    x: &[f32],
    y: &mut [f32],
) -> Result<bool, ()> {
    gpu::submit_matvec_q_resident(w_va, dtype, rows, cols, x, y)
}

pub fn submit_matmul_resident(
    w_va: u64,
    rows: usize,
    cols: usize,
    n: usize,
    x: &[f32],
    y: &mut [f32],
) -> Result<bool, ()> {
    gpu::submit_matmul_resident(w_va, rows, cols, n, x, y)
}

pub fn submit_softmax_rows(x: &mut [f32], rows: usize, cols: usize) -> Result<bool, ()> {
    gpu::submit_softmax_rows(x, rows, cols)
}

pub fn submit_layernorm_rows(
    x: &mut [f32],
    weight: &[f32],
    bias: &[f32],
    rows: usize,
    cols: usize,
    eps: f32,
) -> Result<bool, ()> {
    gpu::submit_layernorm_rows(x, weight, bias, rows, cols, eps)
}

pub fn wait_fence(sem_slot: u32) -> Result<(), ()> {
    gpu::wait_fence(sem_slot)
}
