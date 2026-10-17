//! Backend de GPU del runtime de soso para el **host Linux** (G9): CUDA a través del driver
//! (`libcuda.so.1`, cargada con `dlopen`; sin crates ni toolkit en tiempo de ejecución).
//!
//! Es el `GpuDispatch` que usa el runtime cuando corre como servidor en una máquina con GPU NVIDIA. No es el
//! camino nativo de soso (su propio driver, L6): aquel mueve la tarjeta desde el kernel de soso; éste usa el
//! driver propietario de Linux para acelerar el **mismo runtime** (`serve_host`), que es lo que hace falta para
//! medir y evaluar modelos grandes sin esperar al camino nativo.
//!
//! Diseño:
//! - **Pesos residentes**: la primera vez que se pide un tensor (`key`) se sube a VRAM y se queda. Q4_0 se
//!   reempaqueta al subir (escalas f16 aparte y nibbles alineados a 16 B) para lecturas coalescidas; Q8_0 va
//!   tal cual. Lo que no cabe en el presupuesto, o tiene otro dtype, devuelve `Ok(false)` y lo hace la CPU.
//! - **Activaciones por operación**: cada `matvec`/`matmul` sube `x`, lanza y baja `y`. Es el diseño simple;
//!   la atención, las normas y el resto siguen en CPU.
//! - Kernels en PTX (`cuda/*.ptx`, generados con `scripts/build-cuda-ptx.sh` desde `lxdde/ports/nouveau/*.cu`):
//!   el driver los compila para la tarjeta que haya.

mod capa;

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_void};

use soso_llm_core::gpu::{GpuDispatch, GpuStats};
use soso_llm_core::layer::TensorView;
use sosomodel::layout::{
    DTYPE_Q4_0, DTYPE_Q4_K, DTYPE_Q8_0, Q4_0_BLOCK_BYTES, Q4_0_BLOCK_ELEMS, Q4_K_BLOCK_BYTES,
    Q8_0_BLOCK_BYTES, Q8_0_BLOCK_ELEMS,
};

const PTX_Q4_0: &str = include_str!("../cuda/matvec_q4_0.ptx");
/// Elementos de un superbloque Q4_K: el kernel exige `cols` múltiplo.
const Q4_K_ELEMS: usize = 256;

/// Tokens que procesa de una vez el kernel por lotes.
const NT: usize = 8;
/// Filas que procesa cada warp en `matmul_q4_0r8` (debe coincidir con `Q40R_ROWS` del kernel).
const FILAS_POR_WARP: u32 = 4;
/// VRAM que no se toca (driver, JIT, buffers de trabajo, escritorio).
/// Los pesos se reparten en losas de este tamaño: `cuMemAlloc` redondea cada reserva a 2 MiB y un experto de Q4_K
/// ocupa 1,2 MiB, así que reservarlos uno a uno casi duplica la VRAM (y llena la tarjeta con 8,5 GB de modelo).
const LOSA: usize = 256 << 20;
const ALINEACION: usize = 256;
const RESERVA_VRAM: usize = 1024 << 20;

type CuResult = c_int;
type CuCtx = *mut c_void;
type CuModule = *mut c_void;
type CuFunction = *mut c_void;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

struct Api {
    init: unsafe extern "C" fn(u32) -> CuResult,
    device_get: unsafe extern "C" fn(*mut c_int, c_int) -> CuResult,
    primary_retain: unsafe extern "C" fn(*mut CuCtx, c_int) -> CuResult,
    ctx_set_current: unsafe extern "C" fn(CuCtx) -> CuResult,
    mem_alloc: unsafe extern "C" fn(*mut u64, usize) -> CuResult,
    mem_free: unsafe extern "C" fn(u64) -> CuResult,
    htod: unsafe extern "C" fn(u64, *const c_void, usize) -> CuResult,
    dtoh: unsafe extern "C" fn(*mut c_void, u64, usize) -> CuResult,
    dtod: unsafe extern "C" fn(u64, u64, usize) -> CuResult,
    ev_create: unsafe extern "C" fn(*mut *mut c_void, u32) -> CuResult,
    ev_record: unsafe extern "C" fn(*mut c_void, *mut c_void) -> CuResult,
    ev_sync: unsafe extern "C" fn(*mut c_void) -> CuResult,
    ev_elapsed: unsafe extern "C" fn(*mut f32, *mut c_void, *mut c_void) -> CuResult,
    module_load: unsafe extern "C" fn(*mut CuModule, *const c_void) -> CuResult,
    module_fn: unsafe extern "C" fn(*mut CuFunction, CuModule, *const c_char) -> CuResult,
    launch: unsafe extern "C" fn(
        CuFunction,
        u32,
        u32,
        u32,
        u32,
        u32,
        u32,
        u32,
        *mut c_void,
        *mut *mut c_void,
        *mut *mut c_void,
    ) -> CuResult,
    mem_info: unsafe extern "C" fn(*mut usize, *mut usize) -> CuResult,
    sync: unsafe extern "C" fn() -> CuResult,
}

/// Grafos CUDA y memoria anclada: opcionales (si el driver no los exporta, la capa se lanza kernel a kernel).
struct GrafoApi {
    stream_create: unsafe extern "C" fn(*mut *mut c_void, u32) -> CuResult,
    capture_begin: unsafe extern "C" fn(*mut c_void, c_int) -> CuResult,
    capture_end: unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> CuResult,
    graph_instantiate: unsafe extern "C" fn(*mut *mut c_void, *mut c_void, u64) -> CuResult,
    graph_launch: unsafe extern "C" fn(*mut c_void, *mut c_void) -> CuResult,
    graph_exec_destroy: unsafe extern "C" fn(*mut c_void) -> CuResult,
    graph_destroy: unsafe extern "C" fn(*mut c_void) -> CuResult,
    host_alloc: unsafe extern "C" fn(*mut *mut c_void, usize) -> CuResult,
    htod_async: unsafe extern "C" fn(u64, *const c_void, usize, *mut c_void) -> CuResult,
}

impl GrafoApi {
    fn cargar() -> Result<GrafoApi, String> {
        let nombre = CString::new("libcuda.so.1").unwrap();
        let lib = unsafe { dlopen(nombre.as_ptr(), 2) };
        if lib.is_null() {
            return Err("libcuda".into());
        }
        Ok(GrafoApi {
            stream_create: simbolo(lib, "cuStreamCreate")?,
            capture_begin: simbolo(lib, "cuStreamBeginCapture_v2")?,
            capture_end: simbolo(lib, "cuStreamEndCapture")?,
            graph_instantiate: simbolo(lib, "cuGraphInstantiateWithFlags")?,
            graph_launch: simbolo(lib, "cuGraphLaunch")?,
            graph_exec_destroy: simbolo(lib, "cuGraphExecDestroy")?,
            graph_destroy: simbolo(lib, "cuGraphDestroy")?,
            host_alloc: simbolo(lib, "cuMemAllocHost_v2")?,
            htod_async: simbolo(lib, "cuMemcpyHtoDAsync_v2")?,
        })
    }
}

fn simbolo<T: Copy>(lib: *mut c_void, nombre: &str) -> Result<T, String> {
    let c = CString::new(nombre).unwrap();
    let p = unsafe { dlsym(lib, c.as_ptr()) };
    if p.is_null() {
        return Err(format!("libcuda no exporta {nombre}"));
    }
    Ok(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&p) })
}

impl Api {
    fn cargar() -> Result<Api, String> {
        let nombre = CString::new("libcuda.so.1").unwrap();
        let lib = unsafe { dlopen(nombre.as_ptr(), 2) }; // RTLD_NOW
        if lib.is_null() {
            return Err("no se pudo cargar libcuda.so.1 (¿driver NVIDIA instalado?)".into());
        }
        Ok(Api {
            init: simbolo(lib, "cuInit")?,
            device_get: simbolo(lib, "cuDeviceGet")?,
            primary_retain: simbolo(lib, "cuDevicePrimaryCtxRetain")?,
            ctx_set_current: simbolo(lib, "cuCtxSetCurrent")?,
            mem_alloc: simbolo(lib, "cuMemAlloc_v2")?,
            mem_free: simbolo(lib, "cuMemFree_v2")?,
            htod: simbolo(lib, "cuMemcpyHtoD_v2")?,
            dtoh: simbolo(lib, "cuMemcpyDtoH_v2")?,
            dtod: simbolo(lib, "cuMemcpyDtoD_v2")?,
            ev_create: simbolo(lib, "cuEventCreate")?,
            ev_record: simbolo(lib, "cuEventRecord")?,
            ev_sync: simbolo(lib, "cuEventSynchronize")?,
            ev_elapsed: simbolo(lib, "cuEventElapsedTime")?,
            module_load: simbolo(lib, "cuModuleLoadData")?,
            module_fn: simbolo(lib, "cuModuleGetFunction")?,
            launch: simbolo(lib, "cuLaunchKernel")?,
            mem_info: simbolo(lib, "cuMemGetInfo_v2")?,
            sync: simbolo(lib, "cuCtxSynchronize")?,
        })
    }
}

fn ck(r: CuResult, que: &str) -> Result<(), String> {
    if r == 0 {
        Ok(())
    } else {
        Err(format!("{que}: error CUDA {r}"))
    }
}

/// Un tensor residente en VRAM.
struct Peso {
    /// Q4_0: escalas (u16) y nibbles (16 B por bloque). Q8_0: `d` es el bloque entero y `qs` vale 0.
    d: u64,
    qs: u64,
    dtype: u8,
    rows: usize,
    cols: usize,
    bytes: usize,
}

pub struct CudaGpu {
    api: Api,
    ctx: CuCtx,
    f_matvec_q4: CuFunction,
    f_matmul_q4: CuFunction,
    /// GEMM con tensor cores para el prefill de Q4_0 (`gemm_q4_0_tc`), si no se desactiva con `SOSO_CUDA_SIN_TC=1`.
    f_gemm_q4: CuFunction,
    /// `None` = ya se intentó y no sirve (otro dtype, no cabe): no se reintenta en cada token.
    pesos: HashMap<String, Option<Peso>>,
    presupuesto: usize,
    usado: usize,
    losa_ptr: u64,
    losa_libre: usize,
    x_dev: u64,
    x_cap: usize,
    y_dev: u64,
    y_cap: usize,
    stats: GpuStats,
    avisado: bool,
    capa: capa::Estado,
    /// Grafos CUDA (`None` = no disponibles o desactivados con `SOSO_CUDA_GRAFOS=0`).
    grafo: Option<GrafoApi>,
    stream: *mut c_void,
    capturando: bool,
}

unsafe impl Send for CudaGpu {}

impl CudaGpu {
    /// Inicializa CUDA en el dispositivo 0. Falla (sin pánico) si no hay driver o tarjeta.
    pub fn nueva() -> Result<CudaGpu, String> {
        let api = Api::cargar()?;
        unsafe {
            ck((api.init)(0), "cuInit")?;
            let mut dev: c_int = 0;
            ck((api.device_get)(&mut dev, 0), "cuDeviceGet")?;
            let mut ctx: CuCtx = std::ptr::null_mut();
            ck(
                (api.primary_retain)(&mut ctx, dev),
                "cuDevicePrimaryCtxRetain",
            )?;
            ck((api.ctx_set_current)(ctx), "cuCtxSetCurrent")?;
            let cargar = |ptx: &str| -> Result<CuModule, String> {
                let c = CString::new(ptx).map_err(|_| "PTX con NUL".to_string())?;
                let mut m: CuModule = std::ptr::null_mut();
                ck(
                    (api.module_load)(&mut m, c.as_ptr() as *const c_void),
                    "cuModuleLoadData",
                )?;
                Ok(m)
            };
            let m4 = cargar(PTX_Q4_0)?;
            let func = |m: CuModule, n: &str| -> Result<CuFunction, String> {
                let c = CString::new(n).unwrap();
                let mut f: CuFunction = std::ptr::null_mut();
                ck((api.module_fn)(&mut f, m, c.as_ptr()), n)?;
                Ok(f)
            };
            let f_matvec_q4 = func(m4, "matvec_q4_0r")?;
            let f_matmul_q4 = func(m4, "matmul_q4_0r8")?;
            let f_gemm_q4 = func(m4, "gemm_q4_0_tc")?;
            let (mut libre, mut total) = (0usize, 0usize);
            ck((api.mem_info)(&mut libre, &mut total), "cuMemGetInfo")?;
            let capa = capa::Estado::nueva(&api)?;
            let mut stream: *mut c_void = std::ptr::null_mut();
            let grafo = if std::env::var("SOSO_CUDA_GRAFOS").as_deref() == Ok("0") {
                None
            } else {
                GrafoApi::cargar()
                    .ok()
                    .filter(|g| (g.stream_create)(&mut stream, 0) == 0)
            };
            Ok(CudaGpu {
                capa,
                api,
                ctx,
                f_matvec_q4,
                f_matmul_q4,
                f_gemm_q4,
                pesos: HashMap::new(),
                presupuesto: libre.saturating_sub(RESERVA_VRAM),
                usado: 0,
                losa_ptr: 0,
                losa_libre: 0,
                x_dev: 0,
                x_cap: 0,
                y_dev: 0,
                y_cap: 0,
                stats: GpuStats::default(),
                avisado: false,
                grafo,
                stream,
                capturando: false,
            })
        }
    }

    /// Diagnóstico: activa o apaga el volcado de las activaciones de cada etapa de la capa entera.
    pub fn depurar_capa(&mut self, activo: bool) {
        self.capa.depurar = activo;
        self.capa.volcado.clear();
    }

    /// Diagnóstico: etapas volcadas desde la última llamada a este método (y las vacía).
    pub fn volcado_capa(&mut self) -> Vec<(&'static str, Vec<f32>)> {
        std::mem::take(&mut self.capa.volcado)
    }

    /// Bytes de VRAM ocupados por pesos residentes.
    pub fn vram_usada(&self) -> usize {
        self.usado
    }

    /// Cuántos tensores están residentes.
    pub fn residentes(&self) -> usize {
        self.pesos.values().filter(|p| p.is_some()).count()
    }

    /// Stream de los lanzamientos: el propio mientras se captura un grafo; si no, el de por defecto.
    fn stream_lanzar(&self) -> *mut c_void {
        if self.capturando {
            self.stream
        } else {
            std::ptr::null_mut()
        }
    }

    fn alloc(&self, bytes: usize) -> Result<u64, String> {
        let mut p = 0u64;
        ck(
            unsafe { (self.api.mem_alloc)(&mut p, bytes.max(16)) },
            "cuMemAlloc",
        )?;
        Ok(p)
    }

    /// Reserva un peso: los pequeños se recortan de una losa compartida (alineados a 256 B), los grandes van aparte.
    fn alloc_peso(&mut self, bytes: usize) -> Result<u64, String> {
        let b = bytes.max(16).div_ceil(ALINEACION) * ALINEACION;
        if b >= LOSA / 4 {
            return self.alloc(bytes);
        }
        if self.losa_libre < b {
            match self.alloc(LOSA) {
                Ok(p) => {
                    self.losa_ptr = p;
                    self.losa_libre = LOSA;
                }
                Err(_) => return self.alloc(bytes),
            }
        }
        let p = self.losa_ptr + (LOSA - self.losa_libre) as u64;
        self.losa_libre -= b;
        Ok(p)
    }

    fn subir(&self, dst: u64, src: &[u8]) -> Result<(), String> {
        ck(
            unsafe { (self.api.htod)(dst, src.as_ptr() as *const c_void, src.len()) },
            "cuMemcpyHtoD",
        )
    }

    fn asegurar_buffers(&mut self, x_floats: usize, y_floats: usize) -> Result<(), String> {
        if x_floats > self.x_cap {
            if self.x_dev != 0 {
                unsafe { (self.api.mem_free)(self.x_dev) };
            }
            self.x_dev = self.alloc(x_floats * 4)?;
            self.x_cap = x_floats;
        }
        if y_floats > self.y_cap {
            if self.y_dev != 0 {
                unsafe { (self.api.mem_free)(self.y_dev) };
            }
            self.y_dev = self.alloc(y_floats * 4)?;
            self.y_cap = y_floats;
        }
        Ok(())
    }

    /// Sube el tensor si cabe; `None` si no (otro dtype, no cabe, forma que no sirve).
    fn subir_peso(
        &mut self,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
    ) -> Result<Option<Peso>, String> {
        match view.dtype {
            DTYPE_Q4_0 if cols % Q4_0_BLOCK_ELEMS == 0 => {
                let nb = cols / Q4_0_BLOCK_ELEMS;
                if view.bytes.len() != rows * nb * Q4_0_BLOCK_BYTES {
                    return Ok(None);
                }
                let total = rows * nb * 2 + rows * nb * 16;
                if self.usado + total + (self.x_cap + self.y_cap) * 4 > self.presupuesto {
                    return Ok(None);
                }
                // Reempaquetado: escalas f16 y nibbles en dos arrays alineados.
                let mut escalas = vec![0u8; rows * nb * 2];
                let mut nibbles = vec![0u8; rows * nb * 16];
                for (i, bloque) in view.bytes.chunks_exact(Q4_0_BLOCK_BYTES).enumerate() {
                    escalas[2 * i] = bloque[0];
                    escalas[2 * i + 1] = bloque[1];
                    nibbles[16 * i..16 * i + 16].copy_from_slice(&bloque[2..18]);
                }
                let d = self.alloc_peso(escalas.len())?;
                let qs = self.alloc_peso(nibbles.len())?;
                self.subir(d, &escalas)?;
                self.subir(qs, &nibbles)?;
                self.usado += total;
                Ok(Some(Peso {
                    d,
                    qs,
                    dtype: DTYPE_Q4_0,
                    rows,
                    cols,
                    bytes: total,
                }))
            }
            DTYPE_Q8_0 if cols % Q8_0_BLOCK_ELEMS == 0 => {
                let nb = cols / Q8_0_BLOCK_ELEMS;
                if view.bytes.len() != rows * nb * Q8_0_BLOCK_BYTES {
                    return Ok(None);
                }
                let total = view.bytes.len();
                if self.usado + total + (self.x_cap + self.y_cap) * 4 > self.presupuesto {
                    return Ok(None);
                }
                let d = self.alloc_peso(total)?;
                self.subir(d, view.bytes)?;
                self.usado += total;
                Ok(Some(Peso {
                    d,
                    qs: 0,
                    dtype: DTYPE_Q8_0,
                    rows,
                    cols,
                    bytes: total,
                }))
            }
            DTYPE_Q4_K if cols % Q4_K_ELEMS == 0 && cols <= 4096 => {
                let total = view.bytes.len();
                if total != rows * (cols / Q4_K_ELEMS) * Q4_K_BLOCK_BYTES {
                    return Ok(None);
                }
                if self.usado + total + (self.x_cap + self.y_cap) * 4 > self.presupuesto {
                    return Ok(None);
                }
                let d = self.alloc_peso(total)?;
                self.subir(d, view.bytes)?;
                self.usado += total;
                Ok(Some(Peso {
                    d,
                    qs: 0,
                    dtype: DTYPE_Q4_K,
                    rows,
                    cols,
                    bytes: total,
                }))
            }
            _ => Ok(None),
        }
    }

    fn lanzar(
        &mut self,
        f: CuFunction,
        rows: usize,
        mut args: Vec<*mut c_void>,
    ) -> Result<(), String> {
        let warps_por_bloque = 8u32;
        // El kernel por lotes procesa `FILAS_POR_WARP` filas por warp.
        let filas_por_bloque = if f == self.f_matmul_q4 {
            warps_por_bloque * FILAS_POR_WARP
        } else {
            warps_por_bloque
        };
        let grid = (rows as u32).div_ceil(filas_por_bloque);
        ck(
            unsafe {
                (self.api.launch)(
                    f,
                    grid,
                    1,
                    1,
                    warps_por_bloque * 32,
                    1,
                    1,
                    0,
                    self.stream_lanzar(),
                    args.as_mut_ptr(),
                    std::ptr::null_mut(),
                )
            },
            "cuLaunchKernel",
        )
    }

    fn calcular(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
        n: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, String> {
        if x.len() != n * cols || out.len() != n * rows || n == 0 {
            return Ok(false);
        }
        // El contexto es del hilo: el servidor atiende cada petición en uno distinto del que lo creó.
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        if !self.pesos.contains_key(key) {
            let p = self.subir_peso(view, rows, cols)?;
            self.pesos.insert(key.to_string(), p);
        }
        let Some(Some(p)) = self.pesos.get(key) else {
            return Ok(false);
        };
        if p.rows != rows || p.cols != cols || p.dtype != view.dtype {
            return Ok(false);
        }
        let (dtype, pd, pq) = (p.dtype, p.d, p.qs);
        let _ = p.bytes;
        self.asegurar_buffers(n * cols, n * rows)?;
        let x_bytes = unsafe { std::slice::from_raw_parts(x.as_ptr() as *const u8, x.len() * 4) };
        self.subir(self.x_dev, x_bytes)?;
        let (r32, c32) = (rows as c_int, cols as c_int);
        let (x_dev, y_dev) = (self.x_dev, self.y_dev);
        match dtype {
            DTYPE_Q4_0 if n == 1 => {
                let (mut a0, mut a1, mut a2, mut a3, mut a4, mut a5) =
                    (pd, pq, x_dev, y_dev, r32, c32);
                let args = vec![
                    &mut a0 as *mut _ as *mut c_void,
                    &mut a1 as *mut _ as *mut c_void,
                    &mut a2 as *mut _ as *mut c_void,
                    &mut a3 as *mut _ as *mut c_void,
                    &mut a4 as *mut _ as *mut c_void,
                    &mut a5 as *mut _ as *mut c_void,
                ];
                self.lanzar(self.f_matvec_q4, rows, args)?;
            }
            DTYPE_Q4_0 => {
                let mut t0 = 0usize;
                while t0 < n {
                    let nt = (n - t0).min(NT) as c_int;
                    let (mut a0, mut a1) = (pd, pq);
                    let mut a2 = x_dev + (t0 * cols * 4) as u64;
                    let mut a3 = y_dev + (t0 * rows * 4) as u64;
                    let (mut a4, mut a5, mut a6) = (r32, c32, nt);
                    let args = vec![
                        &mut a0 as *mut _ as *mut c_void,
                        &mut a1 as *mut _ as *mut c_void,
                        &mut a2 as *mut _ as *mut c_void,
                        &mut a3 as *mut _ as *mut c_void,
                        &mut a4 as *mut _ as *mut c_void,
                        &mut a5 as *mut _ as *mut c_void,
                        &mut a6 as *mut _ as *mut c_void,
                    ];
                    self.lanzar(self.f_matmul_q4, rows, args)?;
                    t0 += NT;
                }
            }
            DTYPE_Q4_K => {
                // Un solo vector por lanzamiento, como Q8_0.
                for t in 0..n {
                    let (mut a0, mut a2) = (pd, x_dev + (t * cols * 4) as u64);
                    let mut a3 = y_dev + (t * rows * 4) as u64;
                    let (mut a4, mut a5) = (r32, c32);
                    let args = vec![
                        &mut a0 as *mut _ as *mut c_void,
                        &mut a2 as *mut _ as *mut c_void,
                        &mut a3 as *mut _ as *mut c_void,
                        &mut a4 as *mut _ as *mut c_void,
                        &mut a5 as *mut _ as *mut c_void,
                    ];
                    self.lanzar(self.capa.fns.q4k, rows, args)?;
                }
            }
            DTYPE_Q8_0 => {
                // 4 filas por warp y varios tokens por lectura de pesos (cabeza de logits incluida).
                let (mut a0, mut a1, mut a2) = (pd, x_dev, y_dev);
                let (mut a3, mut a4, mut a5) = (r32, c32, n as c_int);
                let mut args = [
                    &mut a0 as *mut _ as *mut c_void,
                    &mut a1 as *mut _ as *mut c_void,
                    &mut a2 as *mut _ as *mut c_void,
                    &mut a3 as *mut _ as *mut c_void,
                    &mut a4 as *mut _ as *mut c_void,
                    &mut a5 as *mut _ as *mut c_void,
                ];
                self.lanzar3(self.capa.fns.mm8n, ((rows as u32).div_ceil(32), 1, 1), (256, 1, 1), &mut args)?;
            }
            _ => return Ok(false),
        }
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        let out_bytes =
            unsafe { std::slice::from_raw_parts_mut(out.as_mut_ptr() as *mut u8, out.len() * 4) };
        ck(
            unsafe {
                (self.api.dtoh)(
                    out_bytes.as_mut_ptr() as *mut c_void,
                    self.y_dev,
                    out_bytes.len(),
                )
            },
            "cuMemcpyDtoH",
        )?;
        self.stats.launches += 1;
        self.stats.bytes_up += (x.len() * 4) as u64;
        self.stats.bytes_down += (out.len() * 4) as u64;
        Ok(true)
    }
}

impl GpuDispatch for CudaGpu {
    fn available(&self) -> bool {
        true
    }

    fn stats(&self) -> GpuStats {
        self.stats
    }

    fn dtype_soportado(&self, dtype: u8) -> bool {
        matches!(dtype, DTYPE_Q4_0 | DTYPE_Q8_0 | DTYPE_Q4_K)
    }

    fn gemma_puede(&self, layer: u32) -> bool {
        self.capa_puede(layer)
    }

    fn moe_puede(&self, layer: u32) -> bool {
        self.moe_puede_capa(layer)
    }

    fn moe_flujo_activo(&self) -> bool {
        self.moe_flujo_activo_capa()
    }

    fn moe_modo(&mut self, layer: u32, plan: &soso_llm_core::gpu::MoePlan) -> soso_llm_core::gpu::ModoMoe {
        self.moe_modo_capa(layer, plan)
    }

    fn moe_cabe(&self, pesos: usize, kv: usize) -> bool {
        // Además de los pesos y el KV, la capa necesita los búferes de trabajo y las tablas (unas decenas de MiB).
        self.usado + pesos + kv + (96 << 20) <= self.presupuesto
    }

    fn moe_descartar(&mut self, layer: u32) {
        self.capa.rechazar(layer);
    }

    fn moe_lista(&self, layer: u32) -> bool {
        self.moe_lista_capa(layer)
    }

    fn moe_pendiente(&self, layer: u32, pos0: usize, n: usize) -> bool {
        self.moe_pendiente_capa(layer, pos0, n)
    }

    fn moe_volcar(&mut self, hidden: &mut [f32]) -> Result<(), ()> {
        self.moe_volcar_capa(hidden)
            .map_err(|e| eprintln!("cuda: {e}"))
    }

    fn moe_capa(&mut self, capa: &mut soso_llm_core::gpu::MoeCapa<'_>) -> Result<bool, ()> {
        let t0 = std::time::Instant::now();
        let r = self.moe_ejecutar(capa);
        self.stats.total_ns += t0.elapsed().as_nanos() as u64;
        match r {
            Ok(v) => Ok(v),
            Err(e) => {
                eprintln!("cuda: capa MoE {}: {e}; sigo en CPU", capa.layer);
                self.capa.rechazar(capa.layer);
                Err(())
            }
        }
    }

    fn gemma_residente(&self, key: &str) -> bool {
        self.pesos.get(key).is_some_and(|p| p.is_some())
    }

    fn gemma_subir(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
    ) -> Result<bool, ()> {
        match self.capa_subir(key, view, rows, cols) {
            Ok(v) => Ok(v),
            Err(e) => {
                eprintln!("cuda: {e}; la capa va por CPU");
                Err(())
            }
        }
    }

    fn gemma_capa(&mut self, capa: &mut soso_llm_core::gpu::GemmaCapa<'_>) -> Result<bool, ()> {
        match self.capa_ejecutar(capa) {
            Ok(v) => Ok(v),
            Err(e) => {
                eprintln!("cuda: capa {}: {e}; sigo en CPU", capa.layer);
                self.capa.rechazar(capa.layer);
                Err(())
            }
        }
    }

    fn matvec(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()> {
        match self.calcular(key, view, rows, cols, 1, x, out) {
            Ok(v) => Ok(v),
            Err(e) => {
                if !self.avisado {
                    eprintln!("cuda: {e}; sigo en CPU");
                    self.avisado = true;
                }
                Err(())
            }
        }
    }

    fn matmul(
        &mut self,
        key: &str,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
        n: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()> {
        match self.calcular(key, view, rows, cols, n, x, out) {
            Ok(v) => Ok(v),
            Err(e) => {
                if !self.avisado {
                    eprintln!("cuda: {e}; sigo en CPU");
                    self.avisado = true;
                }
                Err(())
            }
        }
    }
}
