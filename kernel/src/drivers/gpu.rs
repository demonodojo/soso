//! Driver GPU: NVIDIA (L6) + Intel iGPU fallback.

use crate::drivers::{nvidia_compute, nvidia_probe, pci};
use crate::println;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use soso_abi::{self as abi, GpuInfo};
use spin::{Mutex, Once};

/// Cuántas subidas a VRAM fueron por DMA del CE y cuántas por rebote.
///
/// POR QUÉ EXISTEN: el camino sin copias (`subir_por_dma`) se escribió el
/// 2026-08-17 y **no se disparaba nunca con los pesos de un modelo**. El payload
/// de un shard `.som` empieza en el byte 64 (`SHARD_PAYLOAD_OFF`), la syscall
/// recibía por tanto un puntero con `%4096 == 64`, y la primera comprobación de
/// `subir_por_dma` lo rechazaba. Desde fuera no se veía nada: el rebote da el
/// mismo resultado, sólo cuesta una copia entera del tensor por la CPU. Sin un
/// contador, el único síntoma es «va lento».
static SUBIDAS_DMA: AtomicU32 = AtomicU32::new(0);
static SUBIDAS_REBOTE: AtomicU32 = AtomicU32::new(0);
static BYTES_REBOTE: AtomicU64 = AtomicU64::new(0);

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

/// Búfer del dispositivo: heap (soft/Intel/fallback) o VRAM residente (G6).
enum GpuStorage {
    Heap {
        words: alloc::vec::Vec<u32>,
        len: usize,
    },
    #[cfg_attr(not(feature = "lxdde"), allow(dead_code))]
    Device {
        va: u64,
        len: usize,
    },
}

struct GpuBuffer {
    storage: GpuStorage,
}

impl GpuBuffer {
    fn heap(bytes: usize) -> Self {
        Self {
            storage: GpuStorage::Heap {
                words: alloc::vec![0u32; bytes.div_ceil(4)],
                len: bytes,
            },
        }
    }

    #[cfg_attr(not(feature = "lxdde"), allow(dead_code))]
    fn device(va: u64, bytes: usize) -> Self {
        Self {
            storage: GpuStorage::Device { va, len: bytes },
        }
    }

    fn len(&self) -> usize {
        match &self.storage {
            GpuStorage::Heap { len, .. } | GpuStorage::Device { len, .. } => *len,
        }
    }

    fn device_va(&self) -> Option<u64> {
        match &self.storage {
            GpuStorage::Device { va, .. } => Some(*va),
            GpuStorage::Heap { .. } => None,
        }
    }

    fn bytes(&self) -> Option<&[u8]> {
        match &self.storage {
            GpuStorage::Heap { words, len } => Some(unsafe {
                core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), *len)
            }),
            GpuStorage::Device { .. } => None,
        }
    }

    fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        match &mut self.storage {
            GpuStorage::Heap { words, len } => Some(unsafe {
                core::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), *len)
            }),
            GpuStorage::Device { .. } => None,
        }
    }

    fn f32s(&self, elems: usize) -> Option<&[f32]> {
        match &self.storage {
            GpuStorage::Heap { words, len } => {
                if elems * 4 > *len {
                    return None;
                }
                Some(unsafe {
                    core::slice::from_raw_parts(words.as_ptr().cast::<f32>(), elems)
                })
            }
            GpuStorage::Device { .. } => None,
        }
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
        // El estado del pool va en la línea de arranque a propósito: queda en la
        // consola y en `SOSOLOG.TXT` sin que nadie tenga que lanzar una
        // inferencia para descubrir que el offload no tenía dónde subir nada.
        println!(
            "gpu: NVIDIA detectada (chipset {:?}, GSP={}, pool VRAM={})",
            nvidia_probe::chipset_id(),
            gsp_label(),
            if pool_vram_listo() { "sí" } else { "no" }
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

/// El pool de la línea de arranque, sin `GpuState` todavía construido.
fn pool_vram_listo() -> bool {
    #[cfg(feature = "lxdde")]
    {
        return crate::lxdde::device_bufs_ready();
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

/// ¿`GPU_ALLOC_VRAM` puede darle a este dispositivo memoria que él lea?
///
/// AVERÍA (2026-08-17): esto miraba `gsp_ready()`, que es cierto ya con el GSP
/// arrancado —y hasta con `booted_soft`—. Pero el pool de VRAM lo monta
/// `gsp_buf_init`, al final de la cadena RM → VMM → canal/CE que hoy sólo corre
/// en la rama Blackwell/FMC. En la placa Ampere, entonces, `alloc` pedía al pool,
/// recibía 0 y **caía en silencio a un búfer del heap del kernel**: `gpu_map`
/// contestaba OK y `MATVF`, al no ver `device_va`, multiplicaba con el bucle de
/// CPU del kernel. Todo decía «offload» y no había ni un byte en la tarjeta.
/// Ahora se pregunta por el pool, que es la pregunta de verdad.
fn device_bufs_available(g: &GpuState) -> bool {
    g.vendor == GPU_VENDOR_NVIDIA && g.compute && {
        #[cfg(feature = "lxdde")]
        {
            crate::lxdde::device_bufs_ready()
        }
        #[cfg(not(feature = "lxdde"))]
        {
            false
        }
    }
}

/// Lo que ve userspace en `GpuInfo::vram_bufs`. El de software dice 1: su heap
/// ES su VRAM y su bucle de CPU es exactamente lo que promete `compute`.
fn vram_bufs_usables(g: &GpuState) -> bool {
    match g.vendor {
        GPU_VENDOR_SOFT => g.present,
        GPU_VENDOR_NVIDIA => device_bufs_available(g),
        _ => false,
    }
}

fn vram_free_bytes(g: &GpuState) -> u64 {
    if device_bufs_available(g) {
        #[cfg(feature = "lxdde")]
        {
            let free = crate::lxdde::device_vram_free();
            if free > 0 {
                return free;
            }
        }
    }
    g.vram_total.saturating_sub(g.vram_used)
}

pub fn info() -> GpuInfo {
    let g = gpu().lock();
    let mut phase = [0u8; 16];
    // Sólo la NVIDIA tiene fases de bring-up. Rellenarlo también para el
    // dispositivo de software o la iGPU sería decirle a userspace que un `rm_ce`
    // habla de un dispositivo que no ha visto un GSP en su vida.
    if g.vendor == GPU_VENDOR_NVIDIA {
        let label = gsp_label().as_bytes();
        // Se trunca en vez de fallar: es un dato de diagnóstico, y el nombre más
        // largo de la tabla (`booted_soft`, 11) cabe de sobra en 15 + NUL.
        let n = core::cmp::min(label.len(), phase.len() - 1);
        phase[..n].copy_from_slice(&label[..n]);
    }
    GpuInfo {
        present: g.present as u8,
        vendor: g.vendor,
        compute: g.compute as u8,
        vram_bufs: vram_bufs_usables(&g) as u8,
        _pad: [0; 4],
        vram_total: g.vram_total,
        vram_free: vram_free_bytes(&g),
        name: g.name,
        phase,
        uploads_dma: SUBIDAS_DMA.load(Ordering::Relaxed),
        uploads_bounce: SUBIDAS_REBOTE.load(Ordering::Relaxed),
        bounce_bytes: BYTES_REBOTE.load(Ordering::Relaxed),
    }
}

fn gpu() -> &'static Mutex<GpuState> {
    GPU.get().expect("gpu no inicializada")
}

/// Una vez por arranque, no por tensor: son cientos de reservas por inferencia y
/// llenar la serie con la misma línea taparía lo que venga después.
fn aviso_sin_pool() {
    use core::sync::atomic::{AtomicBool, Ordering};
    static DICHO: AtomicBool = AtomicBool::new(false);
    if !DICHO.swap(true, Ordering::Relaxed) {
        println!(
            "gpu: GPU_ALLOC_VRAM sin pool (fase={}) — el cómputo se queda en CPU",
            gsp_label()
        );
    }
}

pub fn alloc(size: u64, domain: u64) -> Result<u64, i64> {
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
    let want_vram = domain == abi::GPU_ALLOC_VRAM;
    // VRAM pedida a la NVIDIA: o sale del pool o **falla**. El heap del kernel no
    // vale como VRAM (ver `device_bufs_available`), y el llamante sabe qué hacer
    // con un error: `soso-llm` lo cuenta como «no cabe» y calcula esa capa en CPU.
    let buf = if want_vram && g.vendor == GPU_VENDOR_NVIDIA {
        #[cfg(feature = "lxdde")]
        {
            if !device_bufs_available(&g) {
                aviso_sin_pool();
                return Err(abi::ENOTSUP);
            }
            match crate::lxdde::device_buf_alloc(size) {
                Ok(va) => GpuBuffer::device(va, size as usize),
                // Pool lleno o sin slots: es ENOMEM de verdad, no un sitio donde
                // improvisar con memoria del kernel.
                Err(()) => return Err(abi::ENOMEM),
            }
        }
        #[cfg(not(feature = "lxdde"))]
        {
            aviso_sin_pool();
            return Err(abi::ENOTSUP);
        }
    } else {
        GpuBuffer::heap(size as usize)
    };
    g.buffers.push(Some(buf));
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
    let buf = slot.as_ref().ok_or(abi::EINVAL)?;
    let bytes = buf.len() as u64;
    if let Some(va) = buf.device_va() {
        #[cfg(feature = "lxdde")]
        {
            let _ = crate::lxdde::device_buf_free(va);
        }
        #[cfg(not(feature = "lxdde"))]
        let _ = va;
    }
    *slot = None;
    g.vram_used = g.vram_used.saturating_sub(bytes);
    Ok(bytes)
}

/// Bytes del búfer de `handle`. Para que la syscall pueda rechazar un `len`
/// imposible antes de materializar páginas de usuario; el driver lo vuelve a
/// comprobar con el candado tomado.
pub fn buffer_len(handle: u64) -> Result<u64, i64> {
    let g = gpu().lock();
    g.buffers
        .get(handle as usize)
        .and_then(|b| b.as_ref())
        .map(|b| b.len() as u64)
        .ok_or(abi::EINVAL)
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
    if buf.device_va().is_some() {
        return Err(abi::ENOSYS);
    }
    let n = len as usize;
    crate::task::with_current(|p| {
        let space = p.space.as_ref().ok_or(abi::EFAULT)?;
        space.write(user_ptr, &buf.bytes().ok_or(abi::EFAULT)?[..n]).ok_or(abi::EFAULT)?;
        Ok(0)
    })
}

/// Trozo del rebote entre el proceso y la VRAM. Múltiplo del rebote de 1 MiB de
/// `gsp_buf.h`, que es la unidad real de la copia del CE.
#[cfg(feature = "lxdde")]
const TROZO_SUBIDA: usize = 2 * 1024 * 1024;

/// Copia `n` bytes del proceso a la VA del dispositivo en trozos de
/// `TROZO_SUBIDA`.
///
/// AVERÍA (2026-08-17): esto reservaba `vec![0u8; n]` —el tensor ENTERO— antes de
/// llamar a la capa C. Con TinyLlama son 44 MiB de heap del kernel por subida, y
/// el heap son 512 MiB como mucho: pedirlos contiguos en cada peso es una avería
/// esperando. No hacía falta ni entonces: la capa C ya trocea contra su rebote de
/// 1 MiB, sólo le faltaba aceptar un offset dentro del búfer.
#[cfg(feature = "lxdde")]
fn subir_por_trozos(va: u64, user_ptr: u64, n: usize) -> Result<u64, i64> {
    // Primero sin copias: el CE lee directamente de las páginas del proceso y
    // esto no toca un solo byte.
    match subir_por_dma(va, user_ptr, n) {
        Ok(()) => {
            SUBIDAS_DMA.fetch_add(1, Ordering::Relaxed);
            return Ok(0);
        }
        Err(motivo) => aviso_rebote(motivo, user_ptr, n),
    }
    SUBIDAS_REBOTE.fetch_add(1, Ordering::Relaxed);
    BYTES_REBOTE.fetch_add(n as u64, Ordering::Relaxed);
    let mut tmp = alloc::vec![0u8; core::cmp::min(n, TROZO_SUBIDA)];
    let mut off = 0usize;
    while off < n {
        let c = core::cmp::min(TROZO_SUBIDA, n - off);
        crate::task::with_current(|p| -> Result<u64, i64> {
            let space = p.space.as_ref().ok_or(abi::EFAULT)?;
            space
                .read(user_ptr + off as u64, &mut tmp[..c])
                .ok_or(abi::EFAULT)?;
            Ok(0)
        })?;
        crate::lxdde::device_buf_upload_at(va, off as u64, &tmp[..c]).map_err(|_| abi::EIO)?;
        off += c;
    }
    Ok(0)
}

/// Lote de la subida por DMA. Es `G6_SRC_MAX` de `gsp_buf.h` menos una página: la
/// ventana tiene que alojar además el `lead_in` del origen desalineado.
#[cfg(feature = "lxdde")]
const LOTE_DMA: usize = 16 * 1024 * 1024 - 4096;

/// Por qué no se pudo subir sin copia. Eran cuatro causas distintas devolviendo
/// el mismo `Err(())`, y la que estuvo activa tres meses —el origen en `+64`— no
/// se distinguía de un CE roto.
#[cfg(feature = "lxdde")]
#[derive(Clone, Copy)]
enum Motivo {
    Vacio,
    SinEspacio,
    PhysPages,
    CapaC,
}

/// Una línea de serie la primera vez y sólo la primera: son cientos de subidas
/// por inferencia. Mismo criterio que `aviso_sin_pool`.
#[cfg(feature = "lxdde")]
fn aviso_rebote(motivo: Motivo, user_ptr: u64, n: usize) {
    use core::sync::atomic::AtomicBool;
    static DICHO: AtomicBool = AtomicBool::new(false);
    if DICHO.swap(true, Ordering::Relaxed) {
        return;
    }
    let que = match motivo {
        Motivo::Vacio => "longitud cero",
        Motivo::SinEspacio => "el proceso no tiene espacio de direcciones",
        Motivo::PhysPages => "páginas del origen sin materializar",
        Motivo::CapaC => "el CE rechazó el lote",
    };
    println!(
        "gpu: subida a VRAM por rebote — {} (origen en +{}, {} B). El camino DMA sin copia NO se está usando",
        que,
        user_ptr % 4096,
        n
    );
}

/// Sube sin copiar: el CE lee de las páginas del propio proceso.
///
/// `Err(())` es «por aquí no» y el llamante rebota; ninguno de los caminos de
/// salida deja el búfer a medias de forma que el rebote no pueda arreglar
/// (reescribe los mismos bytes).
///
/// **El origen NO tiene que estar alineado a página**, y eso es el arreglo del
/// 2026-08-17 (segunda tanda): lo que se mapea es la ventana, y el CE lee de
/// `G6_SRC_VA + lead_in`. Mientras aquí se exigió `user_ptr % 4096 == 0`, este
/// camino **no se ejecutó ni una vez con pesos de un modelo**: el payload de un
/// shard `.som` empieza en el byte 64, así que todo tensor mapeado llega en +64 y
/// se iba al rebote sin decirlo. `lead_in` es el mismo en todos los lotes porque
/// el destino avanza en múltiplos de página, y con él el origen.
///
/// Lo que sí se comprueba aquí porque es de este lado: las páginas **fijadas**
/// mientras el CE lee. Son mmap RO de pesos, o sea justo las que `mm::reclaim`
/// desaloja bajo presión, y otro core puede devolver sus frames al asignador en
/// mitad del DMA.
#[cfg(feature = "lxdde")]
fn subir_por_dma(va: u64, user_ptr: u64, n: usize) -> Result<(), Motivo> {
    if n == 0 {
        return Err(Motivo::Vacio);
    }
    let lead_in = (user_ptr % 4096) as usize;
    let base = user_ptr - lead_in as u64;
    // El espacio se clona una vez (es un `Arc`) en vez de reentrar en
    // `with_current` por lote: cada entrada toma `PROCS`, y con los cores ociosos
    // sondeando ese candado no es gratis.
    let space = crate::task::with_current(|p| p.space.clone()).ok_or(Motivo::SinEspacio)?;
    // Lo que vaya a hacer falta y no el lote entero: un `gpu_map` de 4 KiB no
    // tiene por qué pedir 32 KiB de lista. El `+1` es la página que puede añadir
    // el `lead_in`.
    let tope = core::cmp::min((lead_in + n).div_ceil(4096), LOTE_DMA / 4096 + 1);
    let mut phys = alloc::vec![0u64; tope];
    let mut off = 0usize;
    while off < n {
        // Los lotes van en múltiplos de página; el rabo que no llega a página va
        // solo, en su propia copia de una línea (lo que el CE sí tiene probado).
        let resto = n - off;
        let c = if resto > LOTE_DMA {
            LOTE_DMA
        } else if resto > 4096 {
            resto & !0xfff
        } else {
            resto
        };
        // La ventana empieza en la página que contiene al origen del lote, así que
        // cubre `lead_in + c`.
        let paginas = (lead_in + c).div_ceil(4096);
        let ventana = (paginas * 4096) as u64;
        let ini = base + off as u64;
        crate::mm::reclaim::pin_range(&space, ini, ventana);
        let r = space
            .phys_pages(ini, ventana, &mut phys[..paginas])
            .ok_or(Motivo::PhysPages)
            .and_then(|k| {
                crate::lxdde::device_buf_upload_dma(
                    va,
                    off as u64,
                    &phys[..k],
                    lead_in as u32,
                    c as u64,
                )
                .map_err(|()| Motivo::CapaC)
            });
        crate::mm::reclaim::unpin_range(&space, ini, ventana);
        r?;
        off += c;
    }
    Ok(())
}

/// Sube `len` bytes del proceso al búfer. **Devuelve 0**, no la cuenta de bytes.
///
/// Lo devolvía, y su hermana `map_to_user` (gpu_read) devolvía 0: la asimetría no
/// estaba escrita en ninguna parte y ningún llamante usaba el número —`soso-llm`
/// sólo mira el signo—, pero `init test` comparaba con 0 y llevaba en rojo desde
/// entonces, tapado por un timeout del arnés que se comía el paso entero. Si algún
/// día hace falta la cuenta, que sea en las DOS y escrito aquí.
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
    if let Some(va) = slot.device_va() {
        drop(g);
        #[cfg(feature = "lxdde")]
        {
            return subir_por_trozos(va, user_ptr, n);
        }
        #[cfg(not(feature = "lxdde"))]
        {
            let _ = va;
            return Err(abi::ENOSYS);
        }
    }
    crate::task::with_current(|p| -> Result<u64, i64> {
        let space = p.space.as_ref().ok_or(abi::EFAULT)?;
        space
            .read(user_ptr, &mut slot.bytes_mut().ok_or(abi::EFAULT)?[..n])
            .ok_or(abi::EFAULT)?;
        Ok(0)
    })?;
    Ok(0)
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
/// - `b"MATVQ"` + lo mismo + u8 dtype — la matriz está en el búfer **cuantizada**
///   (`sosomodel::layout::DTYPE_Q4_K`/`Q8_0`/`MXFP4`) y se multiplica sin
///   expandirla. Es lo que permite subir los bytes del shard tal cual: 8× menos
///   tráfico y 8× menos VRAM que el plano f32.
/// - `b"GFINI"` — apaga GSP-RM y deja la tarjeta sin DMA (irreversible)
/// - `b"SOFTG"` / `b"SOFTX"` — enciende/apaga el dispositivo software de pruebas
///
/// El valor de vuelta de SAXPY/MATVF/MATVQ lleva `abi::GPU_SUBMIT_ON_GPU` (lo hizo
/// el silicio) y `abi::GPU_SUBMIT_COMPUTED` (el resultado está en el búfer). Ver el
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
        let w_va = g
            .buffers
            .get(w_h as usize)
            .and_then(|b| b.as_ref())
            .and_then(|b| b.device_va());
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
        } else if let Some(va) = w_va {
            match nvidia_compute::submit_matvec_resident(va, rows, cols, &x, &mut y) {
                Ok(true) => true,
                _ => {
                    drop(g);
                    return Ok(0);
                }
            }
        } else {
            let w = f32_view(&g, w_h, rows * cols)?;
            nvidia_compute::submit_matvec_f32(w, rows, cols, &x, &mut y)
                .map_err(|_| abi::EIO)?
        };
        drop(g);
        write_f32_buffer(&mut gpu().lock(), y_h, &y)?;
        return Ok(submit_bits(on_gpu));
    }
    if cmd.len() >= 5 && &cmd[..5] == b"MATVQ" && cmd.len() >= 38 {
        let w_h = u64::from_le_bytes(cmd[5..13].try_into().unwrap_or([0; 8]));
        let rows = u32::from_le_bytes(cmd[13..17].try_into().unwrap_or([0; 4])) as usize;
        let cols = u32::from_le_bytes(cmd[17..21].try_into().unwrap_or([0; 4])) as usize;
        let x_h = u64::from_le_bytes(cmd[21..29].try_into().unwrap_or([0; 8]));
        let y_h = u64::from_le_bytes(cmd[29..37].try_into().unwrap_or([0; 8]));
        let dtype = cmd[37];
        // TODOS los guardias, aquí y antes de lanzar nada: en VRAM la GPU no
        // comprueba una sola cosa, así que un dtype equivocado o un `cols` que no
        // sea número entero de bloques leería la fila del tensor de al lado y
        // devolvería un vector perfectamente creíble. Este es el único sitio del
        // sistema donde se puede acotar.
        let Some(row_bytes) = sosomodel::dequant::row_bytes(dtype, cols) else {
            // Dtype no soportado o `cols` que no cuadra: no es un error del
            // llamante, es «esto lo haces tú» — se calcula en CPU.
            return Ok(0);
        };
        let total = rows.checked_mul(row_bytes).ok_or(abi::EINVAL)?;
        let cap = g
            .buffers
            .get(w_h as usize)
            .and_then(|b| b.as_ref())
            .ok_or(abi::EINVAL)?
            .len();
        if total > cap {
            return Err(abi::EINVAL);
        }
        let x = read_f32_vec(&g, x_h, cols)?;
        let mut y = read_f32_vec(&g, y_h, rows)?;
        let w_va = g
            .buffers
            .get(w_h as usize)
            .and_then(|b| b.as_ref())
            .and_then(|b| b.device_va());
        let on_gpu = if soft {
            // El dispositivo software descuantiza fila a fila con un scratch de
            // `cols`, no el tensor entero: es la referencia numérica del kernel
            // SASS y comparten implementación (`sosomodel::dequant`), que es lo que
            // impide que las dos versiones divieran sin que nada se ponga rojo.
            let w = g
                .buffers
                .get(w_h as usize)
                .and_then(|b| b.as_ref())
                .and_then(|b| b.bytes())
                .ok_or(abi::EINVAL)?;
            let mut scratch = alloc::vec![0f32; cols];
            for r in 0..rows {
                let fila = &w[r * row_bytes..(r + 1) * row_bytes];
                y[r] = sosomodel::dequant::matvec_row(dtype, fila, cols, &x, &mut scratch)
                    .ok_or(abi::EINVAL)?;
            }
            false
        } else if let Some(va) = w_va {
            match nvidia_compute::submit_matvec_q_resident(va, dtype, rows, cols, &x, &mut y) {
                Ok(true) => true,
                _ => {
                    drop(g);
                    return Ok(0);
                }
            }
        } else {
            // Asimétrico con MATVF a propósito: sin `device_va` la matriz está en
            // el heap del kernel, y descuantizarla con el bucle escalar de aquí es
            // estrictamente peor que el matvec fusionado con AVX2 que userspace ya
            // tiene sobre el shard mapeado. Que lo haga él.
            drop(g);
            return Ok(0);
        };
        drop(g);
        write_f32_buffer(&mut gpu().lock(), y_h, &y)?;
        return Ok(submit_bits(on_gpu));
    }
    // Legacy: SAXPY con datos embebidos (tests)
    if cmd.len() >= 8 && &cmd[..5] == b"SAXPY" {
        let a = f32::from_le_bytes(cmd[5..9].try_into().unwrap_or([0; 4]));
        let x = [1.0f32, 2.0, 3.0];
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
    let slot = g
        .buffers
        .get_mut(handle as usize)
        .and_then(|b| b.as_mut())
        .ok_or(abi::EINVAL)?;
    let bytes = data.len() * 4;
    if slot.len() < bytes {
        return Err(abi::EINVAL);
    }
    let buf = slot.bytes_mut().ok_or(abi::EINVAL)?;
    buf[..bytes].copy_from_slice(unsafe {
        core::slice::from_raw_parts(data.as_ptr().cast::<u8>(), bytes)
    });
    Ok(())
}
