//! Sustituto de [`super::gpu`] cuando el port nouveau no está enlazado.
//!
//! La capa lxdde se usa también en máquinas sin NVIDIA — la Steam Deck, sin ir
//! más lejos, lleva una APU de AMD y su perfil compila sólo el port `ath11k`.
//! Sin este doble, el kernel referencia los símbolos `lx_nouveau_*` y el enlace
//! falla aunque nadie vaya a llamarlos.
//!
//! Misma superficie que el módulo real, con la respuesta que corresponde a «no
//! hay GPU»: los estados dicen que no, y las operaciones fallan en vez de
//! fingir que se han hecho. Es la diferencia entre que `soso-llm` caiga al
//! camino de CPU y que crea haber calculado algo en una tarjeta inexistente.

pub fn notify_boot0(_boot0: u32, _device_id: u16) {}

pub fn gsp_ready() -> bool {
    false
}

pub fn gsp_phase() -> &'static str {
    "sin-nouveau"
}

pub fn vram_total() -> u64 {
    0
}

pub fn device_vram_free() -> u64 {
    0
}

pub fn device_bufs_ready() -> bool {
    false
}

pub fn device_buf_alloc(_size: u64) -> Result<u64, ()> {
    Err(())
}

pub fn device_buf_upload_at(_va: u64, _offset: u64, _data: &[u8]) -> Result<(), ()> {
    Err(())
}

pub fn device_buf_upload_dma(
    _va: u64,
    _offset: u64,
    _phys: &[u64],
    _src_off: u32,
    _size: u64,
) -> Result<(), ()> {
    Err(())
}

pub fn device_buf_free(_va: u64) -> Result<(), ()> {
    Err(())
}

pub fn gsp_fini() -> bool {
    false
}

pub fn submit_saxpy(_a: f32, _x: &[f32], _y: &mut [f32]) -> Result<bool, ()> {
    Err(())
}

pub fn submit_matvec_f32(
    _w: &[f32],
    _rows: usize,
    _cols: usize,
    _x: &[f32],
    _y: &mut [f32],
) -> Result<bool, ()> {
    Err(())
}

pub fn submit_matvec_resident(
    _w_va: u64,
    _rows: usize,
    _cols: usize,
    _x: &[f32],
    _y: &mut [f32],
) -> Result<bool, ()> {
    Err(())
}

pub fn submit_matvec_q_resident(
    _w_va: u64,
    _dtype: u8,
    _rows: usize,
    _cols: usize,
    _x: &[f32],
    _y: &mut [f32],
) -> Result<bool, ()> {
    Err(())
}

pub fn submit_matmul_resident(
    _w_va: u64,
    _rows: usize,
    _cols: usize,
    _n: usize,
    _x: &[f32],
    _y: &mut [f32],
) -> Result<bool, ()> {
    Err(())
}

pub fn submit_softmax_rows(_x: &mut [f32], _rows: usize, _cols: usize) -> Result<bool, ()> {
    Err(())
}

pub fn submit_layernorm_rows(
    _x: &mut [f32],
    _weight: &[f32],
    _bias: &[f32],
    _rows: usize,
    _cols: usize,
    _eps: f32,
) -> Result<bool, ()> {
    Err(())
}

pub fn wait_fence(_sem_slot: u32) -> Result<(), ()> {
    Err(())
}

pub fn init_module() -> i32 {
    -1
}
