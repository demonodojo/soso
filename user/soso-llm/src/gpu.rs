//! Despacho GPU vía syscalls del kernel (G5).
//!
//! DOS COSAS QUE ESTABAN MAL Y NO DABAN ERROR:
//!
//! 1. **La matriz se resubía en cada llamada.** `matvec_f32` se llama una vez por
//!    proyección y por token, y los pesos no cambian entre tokens: subir
//!    `rows*cols*4` bytes por syscall en cada una convierte el offload en la ruta
//!    lenta. Ahora los pesos se cachean por NOMBRE de tensor y se suben una vez.
//!    Por nombre y no por dirección: los shards se mapean y se pueden desmapear, y
//!    una dirección reutilizada por otro tensor devolvería pesos ajenos con toda la
//!    pinta de estar bien.
//! 2. **Un búfer reservado para la primera capa se reutilizaba para otra mayor.**
//!    `ensure_buffer` devolvía Ok en cuanto el handle existía, sin mirar el
//!    tamaño, y `SYS_GPU_MAP` recortaba en silencio: media matriz subida y un
//!    resultado creíble. El kernel ahora contesta EINVAL y aquí se guarda el
//!    tamaño reservado para poder crecer.
//!
//! Y una tercera, de rebote: `SysGpu::new` se enganchaba a cualquier dispositivo
//! con `present=1`. En una caja con iGPU Intel eso son dos copias de la matriz por
//! matvec para luego calcular en CPU igual. Ahora exige `compute=1`.
//!
//! Y una cuarta: cada llamada pedía y soltaba búferes de tránsito por `mmap`
//! —siete syscalls por matvec, cuatro de ellas puro trámite— para copiar datos que
//! el kernel podía leer de donde ya estaban. Ver `write_f32`.
//!
//! **Pesos cuantizados (Q8_0/Q4_K).** El dispositivo calcula en f32, así que los
//! pesos se **descuantizan una sola vez, al subirlos**: son residentes, luego el
//! coste es por tensor y no por token. Lo que sube es F32, así que un Q4_K ocupa
//! ~8× en el dispositivo; cuando no cabe en `vram_free` el tensor se queda en CPU y
//! esa capa se calcula ahí. Eso es el offload híbrido, no un fallo.

use alloc::string::String;
use alloc::vec::Vec;
use libsoso::sys;
use soso_abi as abi;
use soso_llm_core::gpu::GpuDispatch;
use soso_llm_core::layer::TensorView;
use soso_llm_core::quant::{dequant_q4_k, dequant_q8_0};
use sosomodel::layout::{DTYPE_F32, DTYPE_Q4_K, DTYPE_Q8_0};

/// Pesos ya residentes en el dispositivo, indexados por el nombre del tensor.
///
/// Por NOMBRE y no por dirección: los shards del modelo están mapeados y se
/// pueden desmapear, así que una dirección reutilizada por otro tensor devolvería
/// pesos ajenos con toda la pinta de estar bien. El nombre lo da el ejecutor de
/// capas, que es quien sabe cuál es.
struct Resident {
    key: String,
    bytes: u64,
    handle: u64,
}

/// Búfer de trabajo del que sí hace falta saber cuánto se reservó.
struct Scratch {
    handle: u64,
    bytes: u64,
}

impl Scratch {
    const NONE: Scratch = Scratch {
        handle: u64::MAX,
        bytes: 0,
    };
}

pub struct SysGpu {
    vram_free: u64,
    /// Nombre que dio el kernel. Se guarda para poder decir de qué dispositivo se
    /// habla: "GPU detectada" sobre el dispositivo software de pruebas sería
    /// mentira, y quien lea el log no tiene otra forma de saberlo.
    name: [u8; 32],
    /// Fase del bring-up del dispositivo (`rm_ce`, `rm_compute`…). Es lo que
    /// convierte un `on_gpu=0` en un diagnóstico: sin ella, saber dónde se quedó
    /// el silicio obliga a abrir el log de serie.
    phase: [u8; 16],
    on_gpu: bool,
    resident: Vec<Resident>,
    x: Scratch,
    y: Scratch,
    uploads: usize,
    calls: usize,
    /// Tensores que no caben en el dispositivo y se quedan en CPU. Es una cifra
    /// del offload híbrido, no un error: sin verla, "va lento" no se distingue de
    /// "no está usando la GPU".
    sin_sitio: usize,
}

/// Tope de matrices residentes. No es por memoria —eso lo controla `vram_free`—
/// sino para que la búsqueda lineal siga siendo barata: un modelo pone del orden
/// de 7 proyecciones por capa, y con esto entran las de varias capas.
const MAX_RESIDENT: usize = 64;

impl SysGpu {
    pub fn new() -> Option<Self> {
        let mut info = abi::GpuInfo::default();
        if sys::gpu_info(&mut info) < 0 || info.present == 0 {
            return None;
        }
        if info.compute == 0 {
            // Presente pero incapaz de lanzar nada (iGPU Intel hoy). Enganchar el
            // despacho aquí sólo añadiría copias.
            return None;
        }
        Some(Self {
            vram_free: info.vram_free,
            name: info.name,
            phase: info.phase,
            on_gpu: false,
            resident: Vec::new(),
            x: Scratch::NONE,
            y: Scratch::NONE,
            uploads: 0,
            calls: 0,
            sin_sitio: 0,
        })
    }

    /// Nombre del dispositivo tal y como lo dio el kernel.
    pub fn device_name(&self) -> &str {
        libsoso::str_hasta_nul(&self.name)
    }

    /// Fase del bring-up tal y como la dio el kernel; vacía si el dispositivo no
    /// tiene fases.
    pub fn phase(&self) -> &str {
        libsoso::str_hasta_nul(&self.phase)
    }

    /// `true` si el último `matvec_f32` lo calculó de verdad el silicio de la GPU
    /// (y no el bucle de CPU del kernel ni el dispositivo software).
    pub fn last_on_gpu(&self) -> bool {
        self.on_gpu
    }

    pub fn stats(&self) -> (usize, usize, usize, usize) {
        (self.calls, self.uploads, self.resident.len(), self.sin_sitio)
    }

    /// Reserva o agranda un búfer de trabajo. Devolver el handle viejo cuando el
    /// nuevo tamaño no cabe es el bug 2 de la cabecera.
    fn ensure_scratch(s: &mut Scratch, bytes: u64, vram_free: &mut u64) -> Result<u64, ()> {
        if s.handle != u64::MAX && s.bytes >= bytes {
            return Ok(s.handle);
        }
        if s.handle != u64::MAX {
            let freed = sys::gpu_free(s.handle);
            if freed > 0 {
                *vram_free = vram_free.saturating_add(freed as u64);
            }
            *s = Scratch::NONE;
        }
        if bytes > *vram_free {
            return Err(());
        }
        let h = sys::gpu_alloc(bytes);
        if h < 0 {
            return Err(());
        }
        *vram_free = vram_free.saturating_sub(bytes);
        s.handle = h as u64;
        s.bytes = bytes;
        Ok(s.handle)
    }

    /// Handle de los pesos, subiéndolos sólo la primera vez que se ven.
    ///
    /// `elems` son los f32 LÓGICOS del tensor: lo que ocupará en el dispositivo,
    /// que con pesos cuantizados no es lo que ocupa en el modelo.
    fn resident_weights(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        elems: usize,
    ) -> Result<u64, ()> {
        let bytes = (elems * 4) as u64;

        if let Some(r) = self
            .resident
            .iter()
            .find(|r| r.key == key && r.bytes == bytes)
        {
            return Ok(r.handle);
        }
        // Sitio: primero por número de entradas, luego por VRAM. Se echa la más
        // antigua, que con un recorrido de capas en orden es la que más tardará
        // en volver a hacer falta.
        while self.resident.len() >= MAX_RESIDENT || bytes > self.vram_free {
            let Some(old) = self.resident.first() else {
                // Ni vaciando el dispositivo cabe este tensor: se queda en CPU. Es
                // el caso normal de un modelo más grande que la VRAM, no un error.
                self.sin_sitio += 1;
                return Err(());
            };
            let freed = sys::gpu_free(old.handle);
            if freed > 0 {
                self.vram_free = self.vram_free.saturating_add(freed as u64);
            }
            self.resident.remove(0);
        }
        let h = sys::gpu_alloc(bytes);
        if h < 0 {
            return Err(());
        }
        let handle = h as u64;
        self.vram_free = self.vram_free.saturating_sub(bytes);
        // Lo que sube es SIEMPRE f32. Descuantizar aquí es lo que hace que un
        // modelo Q4_K pueda usar el dispositivo, y se paga una vez por tensor.
        let subido = match view.dtype {
            DTYPE_F32 => view.f32().ok_or(()).and_then(|w| write_f32(handle, w)),
            DTYPE_Q8_0 | DTYPE_Q4_K => {
                let mut plano = alloc::vec![0f32; elems];
                let ok = if view.dtype == DTYPE_Q8_0 {
                    dequant_q8_0(view.bytes, &mut plano)
                } else {
                    dequant_q4_k(view.bytes, &mut plano)
                };
                ok.and_then(|_| write_f32(handle, &plano))
            }
            _ => Err(()),
        };
        if subido.is_err() {
            sys::gpu_free(handle);
            self.vram_free = self.vram_free.saturating_add(bytes);
            return Err(());
        }
        self.uploads += 1;
        self.resident.push(Resident {
            key: String::from(key),
            bytes,
            handle,
        });
        Ok(handle)
    }

    fn submit_matvf(w_h: u64, rows: u32, cols: u32, x_h: u64, y_h: u64) -> Result<u64, ()> {
        let mut cmd = [0u8; 37];
        cmd[0..5].copy_from_slice(b"MATVF");
        cmd[5..13].copy_from_slice(&w_h.to_le_bytes());
        cmd[13..17].copy_from_slice(&rows.to_le_bytes());
        cmd[17..21].copy_from_slice(&cols.to_le_bytes());
        cmd[21..29].copy_from_slice(&x_h.to_le_bytes());
        cmd[29..37].copy_from_slice(&y_h.to_le_bytes());
        let rc = sys::gpu_submit(&cmd);
        if rc < 0 {
            return Err(());
        }
        Ok(rc as u64)
    }
}

/// Sube `data` al búfer del dispositivo **desde donde está**.
///
/// Antes esto pedía un `mmap`, copiaba los datos ahí, llamaba a `gpu_map` y
/// soltaba el `mmap`: cuatro syscalls y una copia entera de la matriz por subida,
/// más una falta de página por cada página nueva del mapeo. Todo eso existía porque
/// `SYS_GPU_MAP` exigía permiso de escritura en el búfer de origen, y los pesos de
/// un modelo viven en un mapeo de fichero de sólo lectura. Corregido eso en el
/// kernel, la dirección del propio tensor sirve tal cual.
fn write_f32(handle: u64, data: &[f32]) -> Result<(), ()> {
    let bytes = (data.len() * 4) as u64;
    if sys::gpu_map(handle, data.as_ptr() as u64, bytes) < 0 {
        return Err(());
    }
    Ok(())
}

/// Lee el resultado **directamente sobre el destino** del llamante.
fn read_f32_into(handle: u64, out: &mut [f32]) -> Result<(), ()> {
    let bytes = (out.len() * 4) as u64;
    if sys::gpu_read(handle, out.as_mut_ptr() as u64, bytes) < 0 {
        return Err(());
    }
    Ok(())
}

impl GpuDispatch for SysGpu {
    fn available(&self) -> bool {
        true
    }

    /// `Ok(true)` significa **el resultado ya está en `out`**, no "lo hizo la
    /// GPU": eso último es `last_on_gpu()`. Cuando el kernel calcula con su bucle
    /// de CPU (canal no listo, dispositivo software) el vector es igual de bueno y
    /// repetirlo aquí es trabajo tirado — pero decir "GPU" sería falso.
    fn matvec(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()> {
        if view.elems != rows * cols || x.len() != cols || out.len() != rows {
            return Err(());
        }
        // Que no quepa NO es un error: devolver Err aquí abortaría la inferencia
        // entera en vez de calcular esa capa en CPU, que es lo que hay que hacer
        // con un modelo más grande que la VRAM.
        let Ok(w_handle) = self.resident_weights(key, view, rows * cols) else {
            return Ok(false);
        };
        let x_bytes = (cols * 4) as u64;
        let y_bytes = (rows * 4) as u64;
        let mut vram = self.vram_free;
        let x_handle = Self::ensure_scratch(&mut self.x, x_bytes, &mut vram)?;
        let y_handle = Self::ensure_scratch(&mut self.y, y_bytes, &mut vram)?;
        self.vram_free = vram;

        write_f32(x_handle, x)?;
        let bits = Self::submit_matvf(w_handle, rows as u32, cols as u32, x_handle, y_handle)?;
        self.calls += 1;
        self.on_gpu = bits & abi::GPU_SUBMIT_ON_GPU != 0;
        if bits & abi::GPU_SUBMIT_COMPUTED == 0 {
            // El dispositivo no calculó nada: que lo haga la CPU de quien llama.
            return Ok(false);
        }
        read_f32_into(y_handle, out)?;
        Ok(true)
    }
}
