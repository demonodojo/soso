//! Driver GPU: NVIDIA (L6) + Intel iGPU fallback.

use crate::drivers::{nvidia_compute, nvidia_probe, pci};
use crate::println;
use alloc::vec::Vec;
use soso_abi::{self as abi, GpuInfo};
use spin::{Mutex, Once};

const VENDOR_INTEL: u16 = 0x8086;
const VENDOR_NVIDIA: u16 = 0x10de;
const GPU_VENDOR_NVIDIA: u8 = 2;
const GPU_VENDOR_INTEL: u8 = 1;
/// Dispositivo de mentira que calcula en la CPU del kernel. NO es un atajo para
/// producción: existe porque sin él todo el camino de syscalls GPU (alloc, map,
/// submit, read) no se ejecuta ni una vez en QEMU, y el primer sitio donde se
/// probaría sería la tarjeta real — donde un fallo de fontanería es
/// indistinguible de un fallo de la GPU. Se enciende a petición (`SOFTG`).
const GPU_VENDOR_SOFT: u8 = 3;

/// Búfer del dispositivo.
///
/// Se reserva como `Vec<u32>` y no como `Vec<u8>` por una razón concreta: un
/// `Vec<u8>` sólo garantiza alineación 1, así que sus bytes no se pueden releer
/// como `f32` sin copiarlos a otro sitio. Con la base alineada a 4, un búfer de
/// pesos se le pasa al motor de cómputo **sin copia**; antes cada `MATVF` construía
/// un `Vec<f32>` entero de la matriz (y a golpe de `from_le_bytes` de 4 en 4) sólo
/// para poder mirarla.
struct GpuBuffer {
    words: Vec<u32>,
    /// Bytes lógicos: lo que pidió el usuario, que no tiene que ser múltiplo de 4.
    len: usize,
}

impl GpuBuffer {
    fn new(bytes: usize) -> Self {
        Self {
            words: alloc::vec![0u32; bytes.div_ceil(4)],
            len: bytes,
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn bytes(&self) -> &[u8] {
        // La reserva cubre `len` redondeado hacia arriba, así que `len` bytes
        // siempre están dentro.
        unsafe { core::slice::from_raw_parts(self.words.as_ptr().cast::<u8>(), self.len) }
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.words.as_mut_ptr().cast::<u8>(), self.len) }
    }

    /// Vista `&[f32]` sin copia. `None` si no caben `elems`.
    fn f32s(&self, elems: usize) -> Option<&[f32]> {
        if elems * 4 > self.len {
            return None;
        }
        Some(unsafe { core::slice::from_raw_parts(self.words.as_ptr().cast::<f32>(), elems) })
    }
}

struct GpuState {
    present: bool,
    vendor: u8,
    /// `submit` calcula de verdad sobre este dispositivo. Ver `abi::GpuInfo`.
    compute: bool,
    name: [u8; 32],
    vram_total: u64,
    vram_used: u64,
    buffers: Vec<Option<GpuBuffer>>,
}

static GPU: Once<Mutex<GpuState>> = Once::new();

fn nvidia_vram() -> u64 {
    #[cfg(feature = "lxdde")]
    {
        let v = crate::lxdde::nouveau_vram_total();
        if v > 0 {
            return v;
        }
    }
    8_u64 * 1024 * 1024 * 1024
}

pub fn init() {
    let devs = pci::enumerate();
    let nvidia = devs
        .iter()
        .find(|d| d.vendor_id == VENDOR_NVIDIA && d.class == 0x03);
    let intel = devs
        .iter()
        .find(|d| d.vendor_id == VENDOR_INTEL && d.class == 0x03);

    let state = if nvidia.is_some() || nvidia_probe::present() {
        let mut name = [0u8; 32];
        let label = b"NVIDIA GB205 (soso/lxdde)";
        name[..label.len()].copy_from_slice(label);
        let vram = nvidia_vram();
        println!(
            "gpu: NVIDIA detectada (chipset {:?}, GSP={})",
            nvidia_probe::chipset_id(),
            gsp_label()
        );
        GpuState {
            present: true,
            vendor: GPU_VENDOR_NVIDIA,
            // La capa C siempre deja un resultado: con canal y QMD lo calcula la
            // GPU, y si no, su bucle de CPU. En los dos casos el búfer sale
            // bueno, que es lo que este bit promete.
            compute: true,
            name,
            vram_total: vram,
            vram_used: 0,
            buffers: Vec::new(),
        }
    } else if let Some(gpu) = intel {
        if gpu.bar0 != 0 && gpu.bar0_size > 0 {
            crate::mm::ensure_mmio_mapped(gpu.bar0, gpu.bar0_size);
        }
        let vram = gpu.bar0_size.max(64 * 1024 * 1024).min(512 * 1024 * 1024);
        let mut name = [0u8; 32];
        let label = b"Intel iGPU (soso)";
        name[..label.len()].copy_from_slice(label);
        println!("gpu: Intel detectada, VRAM estimada {} MiB", vram / (1024 * 1024));
        GpuState {
            present: true,
            vendor: GPU_VENDOR_INTEL,
            // Se le pueden dar búferes, pero no hay quien lance nada en ella.
            compute: false,
            name,
            vram_total: vram,
            vram_used: 0,
            buffers: Vec::new(),
        }
    } else {
        println!("gpu: sin GPU; modo CPU");
        GpuState {
            present: false,
            vendor: 0,
            compute: false,
            name: [0; 32],
            vram_total: 0,
            vram_used: 0,
            buffers: Vec::new(),
        }
    };
    GPU.call_once(|| Mutex::new(state));
}

/// Deja la GPU en un estado en el que se la puede quitar de debajo.
///
/// Se llama desde `SYS_HALT`, antes de que el proceso desaparezca. Con la
/// tarjeta por VFIO, el host la resetea en cuanto se cierra QEMU, y hacer eso
/// sobre un GSP vivo colgó la máquina entera el 2026-07-25 — sin dejar ni traza
/// de panic, porque fue un lockup, no un `panic()`. Ver `gsp_fini.h`.
///
/// Inocuo si no hay GPU NVIDIA o si nunca se llegó a inicializar: en QEMU sin
/// passthrough no imprime nada ni entra en la capa C.
pub fn shutdown() {
    let Some(g) = GPU.get() else {
        return;
    };
    let is_nvidia = {
        let s = g.lock();
        s.present && s.vendor == GPU_VENDOR_NVIDIA
    };
    if !is_nvidia {
        return;
    }
    if gsp_fini() {
        crate::println!("gpu: GSP apagado, la tarjeta se puede soltar");
    }
}

/// Apagado ordenado de GSP-RM. Sin `lxdde` no hay GSP que apagar, así que la
/// respuesta honesta es "no se hizo nada" y no un éxito de mentira.
fn gsp_fini() -> bool {
    #[cfg(feature = "lxdde")]
    {
        return crate::lxdde::gsp_fini();
    }
    #[cfg(not(feature = "lxdde"))]
    false
}

fn gsp_label() -> &'static str {
    #[cfg(feature = "lxdde")]
    {
        return crate::lxdde::gsp_phase();
    }
    #[cfg(not(feature = "lxdde"))]
    "off"
}

fn gpu() -> &'static Mutex<GpuState> {
    GPU.get().expect("gpu no inicializada")
}

pub fn info() -> GpuInfo {
    let g = gpu().lock();
    GpuInfo {
        present: g.present as u8,
        vendor: g.vendor,
        compute: g.compute as u8,
        _pad: [0; 5],
        vram_total: g.vram_total,
        vram_free: g.vram_total.saturating_sub(g.vram_used),
        name: g.name,
    }
}

pub fn alloc(size: u64) -> Result<u64, i64> {
    if size == 0 {
        return Err(abi::EINVAL);
    }
    let mut g = gpu().lock();
    if !g.present {
        return Err(abi::ENOSYS);
    }
    if g.vram_used + size > g.vram_total {
        return Err(abi::ENOMEM);
    }
    let handle = g.buffers.len() as u64;
    g.buffers.push(Some(GpuBuffer::new(size as usize)));
    g.vram_used += size;
    Ok(handle)
}

/// Libera un búfer del dispositivo y devuelve su tamaño a la cuenta de VRAM.
///
/// Faltaba, y sin esto no hay forma de tener pesos residentes: quien quisiera
/// cachear matrices en el dispositivo sólo podía ir dejando búferes muertos hasta
/// agotar la VRAM contada, y entonces `alloc` empieza a devolver ENOMEM sin que
/// nada haya hecho nada mal.
pub fn free(handle: u64) -> Result<u64, i64> {
    let mut g = gpu().lock();
    if !g.present {
        return Err(abi::ENOSYS);
    }
    let slot = g.buffers.get_mut(handle as usize).ok_or(abi::EINVAL)?;
    let bytes = slot.as_ref().ok_or(abi::EINVAL)?.len() as u64;
    *slot = None;
    g.vram_used = g.vram_used.saturating_sub(bytes);
    Ok(bytes)
}

pub fn map_to_user(handle: u64, user_ptr: u64, len: u64) -> Result<u64, i64> {
    let g = gpu().lock();
    let buf = g
        .buffers
        .get(handle as usize)
        .and_then(|b| b.as_ref())
        .ok_or(abi::EINVAL)?;
    // EINVAL, no recorte. Pedir más de lo que hay en el búfer es un error del
    // llamante, y contestarle con los bytes que había dejaba al userspace con
    // media matriz y ninguna pista: el `min()` de antes convertía un handle
    // equivocado o un búfer que se quedó pequeño en un resultado creíble.
    if len > buf.len() as u64 {
        return Err(abi::EINVAL);
    }
    let n = len as usize;
    crate::task::with_current(|p| {
        let space = p.space.as_ref().ok_or(abi::EFAULT)?;
        space.write(user_ptr, &buf.bytes()[..n]).ok_or(abi::EFAULT)?;
        Ok(0)
    })
}

pub fn upload_from_user(handle: u64, user_ptr: u64, len: u64) -> Result<u64, i64> {
    let mut g = gpu().lock();
    let slot = g
        .buffers
        .get_mut(handle as usize)
        .and_then(|b| b.as_mut())
        .ok_or(abi::EINVAL)?;
    // Igual que en la lectura: subir 16 MiB a un búfer de 4 y que la syscall
    // conteste 0 es la peor variante posible. El caso real que esto caza es el de
    // un búfer reservado para la primera capa y reutilizado para una más grande.
    if len > slot.len() as u64 {
        return Err(abi::EINVAL);
    }
    let n = len as usize;
    crate::task::with_current(|p| {
        let space = p.space.as_ref().ok_or(abi::EFAULT)?;
        space.read(user_ptr, &mut slot.bytes_mut()[..n]).ok_or(abi::EFAULT)?;
        Ok(0)
    })
}

/// Enciende el dispositivo software de pruebas (`SOFTG`). Nunca pisa hardware
/// real: si ya hay GPU, contesta EBUSY en vez de tapar la de verdad.
fn enable_soft() -> Result<u64, i64> {
    let mut g = gpu().lock();
    if g.present && g.vendor != GPU_VENDOR_SOFT {
        return Err(abi::EBUSY);
    }
    if g.present && g.vendor == GPU_VENDOR_SOFT {
        return Ok(0);
    }
    let mut name = [0u8; 32];
    let label = b"soft (CPU del kernel, pruebas)";
    name[..label.len()].copy_from_slice(label);
    g.present = true;
    g.vendor = GPU_VENDOR_SOFT;
    g.compute = true;
    g.name = name;
    g.vram_total = 256 * 1024 * 1024;
    g.vram_used = 0;
    g.buffers.clear();
    println!("gpu: dispositivo software activado (cómputo en CPU, para pruebas)");
    Ok(0)
}

/// Lo apaga y suelta sus búferes. El test que lo encendió lo deja como estaba:
/// dejarlo puesto haría que el siguiente proceso del mismo arranque viera una
/// GPU que no existe.
fn disable_soft() -> Result<u64, i64> {
    let mut g = gpu().lock();
    if !g.present || g.vendor != GPU_VENDOR_SOFT {
        return Err(abi::EINVAL);
    }
    g.present = false;
    g.vendor = 0;
    g.compute = false;
    g.name = [0; 32];
    g.vram_total = 0;
    g.vram_used = 0;
    g.buffers.clear();
    println!("gpu: dispositivo software apagado");
    Ok(0)
}

/// Comandos GPU (userspace):
/// - `b"SAXPY"` + f32 a + u64 x_handle + u64 y_handle + u32 n
/// - `b"MATVF"` + u64 w_handle + u32 rows + u32 cols + u64 x_handle + u64 y_handle
/// - `b"GFINI"` — apaga GSP-RM y deja la tarjeta sin DMA (irreversible)
/// - `b"SOFTG"` / `b"SOFTX"` — enciende/apaga el dispositivo software de pruebas
///
/// El valor de vuelta de SAXPY/MATVF lleva `abi::GPU_SUBMIT_ON_GPU` (lo hizo el
/// silicio) y `abi::GPU_SUBMIT_COMPUTED` (el resultado está en el búfer). Ver el
/// comentario de esas constantes: son dos preguntas distintas.
pub fn submit(cmd: &[u8]) -> Result<u64, i64> {
    // Antes del candado y antes de `present`: encender el dispositivo de pruebas
    // es justo lo que se pide cuando NO hay ninguno.
    if cmd.len() >= 5 && &cmd[..5] == b"SOFTG" {
        return enable_soft();
    }
    if cmd.len() >= 5 && &cmd[..5] == b"SOFTX" {
        return disable_soft();
    }
    let g = gpu().lock();
    if !g.present {
        return Err(abi::ENOSYS);
    }
    if !g.compute {
        // Intel: acepta búferes, no ejecuta. Se contesta 0 —ni ON_GPU ni
        // COMPUTED— para que el llamante calcule él y no se crea el búfer.
        return Ok(0);
    }
    let soft = g.vendor == GPU_VENDOR_SOFT;
    if g.vendor != GPU_VENDOR_NVIDIA && !soft {
        return Ok(0);
    }
    // Antes que nada: apagar no necesita buffers ni handles, y tiene que poder
    // ejecutarse aunque el resto del estado esté hecho un desastre.
    if cmd.len() >= 5 && &cmd[..5] == b"GFINI" {
        drop(g);
        return Ok(gsp_fini() as u64);
    }
    if cmd.len() >= 5 && &cmd[..5] == b"SAXPY" && cmd.len() >= 29 {
        let a = f32::from_le_bytes(cmd[5..9].try_into().unwrap_or([0; 4]));
        let x_h = u64::from_le_bytes(cmd[9..17].try_into().unwrap_or([0; 8]));
        let y_h = u64::from_le_bytes(cmd[17..25].try_into().unwrap_or([0; 8]));
        let n = u32::from_le_bytes(cmd[25..29].try_into().unwrap_or([0; 4])) as usize;
        let x = read_f32_vec(&g, x_h, n)?;
        let mut y = read_f32_vec(&g, y_h, n)?;
        drop(g);
        let on_gpu = if soft {
            for i in 0..n {
                y[i] = a * x[i] + y[i];
            }
            false
        } else {
            nvidia_compute::submit_saxpy(a, &x, &mut y).map_err(|_| abi::EIO)?
        };
        write_f32_buffer(&mut gpu().lock(), y_h, &y)?;
        return Ok(submit_bits(on_gpu) | y[0].to_bits() as u64);
    }
    if cmd.len() >= 5 && &cmd[..5] == b"MATVF" && cmd.len() >= 37 {
        let w_h = u64::from_le_bytes(cmd[5..13].try_into().unwrap_or([0; 8]));
        let rows = u32::from_le_bytes(cmd[13..17].try_into().unwrap_or([0; 4])) as usize;
        let cols = u32::from_le_bytes(cmd[17..21].try_into().unwrap_or([0; 4])) as usize;
        let x_h = u64::from_le_bytes(cmd[21..29].try_into().unwrap_or([0; 8]));
        let y_h = u64::from_le_bytes(cmd[29..37].try_into().unwrap_or([0; 8]));
        // `x` e `y` se copian (son vectores); la MATRIZ no. Antes se copiaba
        // entera a un `Vec<f32>` en cada llamada —16 MiB por matvec en un modelo
        // de verdad, y por token—, que es exactamente el coste que el offload
        // viene a evitar.
        let x = read_f32_vec(&g, x_h, cols)?;
        let mut y = read_f32_vec(&g, y_h, rows)?;
        // El candado del dispositivo se mantiene mientras dura el cálculo: la
        // matriz se le pasa prestada desde el búfer, y además dos submits a la vez
        // sobre un solo canal no tendrían sentido. Antes se soltaba porque los
        // datos ya estaban copiados.
        let on_gpu = if soft {
            let w = f32_view(&g, w_h, rows * cols)?;
            for r in 0..rows {
                let mut sum = 0.0f32;
                for c in 0..cols {
                    sum += w[r * cols + c] * x[c];
                }
                y[r] = sum;
            }
            false
        } else {
            let w = f32_view(&g, w_h, rows * cols)?;
            nvidia_compute::submit_matvec_f32(w, rows, cols, &x, &mut y)
                .map_err(|_| abi::EIO)?
        };
        drop(g);
        write_f32_buffer(&mut gpu().lock(), y_h, &y)?;
        return Ok(submit_bits(on_gpu));
    }
    // Legacy: SAXPY con datos embebidos (tests)
    if cmd.len() >= 8 && &cmd[..5] == b"SAXPY" {
        let a = f32::from_le_bytes(cmd[5..9].try_into().unwrap_or([0; 4]));
        let mut x = [1.0f32, 2.0, 3.0];
        let mut y = [0.0f32; 3];
        if nvidia_compute::submit_saxpy(a, &x, &mut y).is_ok() {
            return Ok(y[0].to_bits() as u64);
        }
    }
    Ok(0)
}

/// `COMPUTED` va siempre que hemos escrito el búfer; `ON_GPU` sólo si lo hizo la
/// tarjeta. Ponerlos juntos en una función evita que un camino se olvide de uno.
fn submit_bits(on_gpu: bool) -> u64 {
    abi::GPU_SUBMIT_COMPUTED | if on_gpu { abi::GPU_SUBMIT_ON_GPU } else { 0 }
}

/// Vista `&[f32]` de un búfer, sin copia. Es lo que se usa para la matriz.
fn f32_view(g: &GpuState, handle: u64, elems: usize) -> Result<&[f32], i64> {
    g.buffers
        .get(handle as usize)
        .and_then(|b| b.as_ref())
        .ok_or(abi::EINVAL)?
        .f32s(elems)
        .ok_or(abi::EINVAL)
}

/// Copia de un búfer a un `Vec<f32>`. Sólo para los VECTORES (x, y): son de
/// `rows`/`cols` elementos, no de `rows*cols`, y el destino tiene que ser
/// `&mut` mientras la matriz sigue prestada del mismo `Vec` de búferes.
fn read_f32_vec(g: &GpuState, handle: u64, elems: usize) -> Result<Vec<f32>, i64> {
    Ok(f32_view(g, handle, elems)?.to_vec())
}

fn write_f32_buffer(g: &mut GpuState, handle: u64, data: &[f32]) -> Result<(), i64> {
    let slot = g.buffers.get_mut(handle as usize).and_then(|b| b.as_mut()).ok_or(abi::EINVAL)?;
    let bytes = data.len() * 4;
    if slot.len() < bytes {
        return Err(abi::EINVAL);
    }
    slot.bytes_mut()[..bytes].copy_from_slice(unsafe {
        core::slice::from_raw_parts(data.as_ptr().cast::<u8>(), bytes)
    });
    Ok(())
}
