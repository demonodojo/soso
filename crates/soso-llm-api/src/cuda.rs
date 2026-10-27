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
mod decode;

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_void};

use soso_llm_core::gpu::{GpuDispatch, GpuStats};
use soso_llm_core::layer::TensorView;
use sosomodel::layout::{
    DTYPE_IQ2_S, DTYPE_IQ2_XS, DTYPE_IQ3_S, DTYPE_IQ3_XXS, DTYPE_IQ4_XS, DTYPE_Q4_0, DTYPE_Q4_K, DTYPE_Q5_K, DTYPE_Q8_0, Q4_0_BLOCK_BYTES, Q4_0_BLOCK_ELEMS, Q4_K_BLOCK_BYTES,
    Q8_0_BLOCK_BYTES, Q8_0_BLOCK_ELEMS,
};

const PTX_Q4_0: &str = include_str!("../cuda/matvec_q4_0.ptx");
/// i-quants y Q5_K residentes tal cual (`matvec_iq.cu`, compilado con NVRTC: `tools/cuda-nvrtc/compilar.py`).
const PTX_IQ: &str = include_str!("../cuda/matvec_iq.ptx");

/// Código del decodificador de `iq_matmul_mma` y bytes por fila para un tensor de ese dtype y `cols` columnas.
fn codigo_mma(dtype: u8, cols: usize) -> Option<(i32, i64)> {
    match dtype {
        DTYPE_Q8_0 if cols % 32 == 0 => Some((1000, ((cols / 32) * 36) as i64)),
        DTYPE_Q4_K if cols % 256 == 0 => Some((1002, ((cols / 256) * 144) as i64)),
        d if es_iq_gpu(d) && cols % 256 == 0 => {
            Some((sosomodel::layout::ggml_de_dtype_iq(d)? as i32, sosomodel::dequant::row_bytes(d, cols)? as i64))
        }
        _ => None,
    }
}

/// Dtypes que resuelve `iq_matvec`.
fn soltar_paginas_host(bytes: &[u8]) {
    unsafe extern "C" {
        fn madvise(addr: *mut c_void, len: usize, advice: c_int) -> c_int;
    }
    const MADV_DONTNEED: c_int = 4;
    const PAGINA: usize = 4096;
    let ini = (bytes.as_ptr() as usize).div_ceil(PAGINA) * PAGINA;
    let fin = (bytes.as_ptr() as usize + bytes.len()) / PAGINA * PAGINA;
    // Sólo si es un mapa de fichero: en memoria anónima `MADV_DONTNEED` borra los datos.
    if fin > ini && crate::host_mmap::es_mapa_de_fichero(ini, fin - ini) && std::env::var_os("SOSO_CONSERVAR_PAGINAS").is_none() {
        // Sólo páginas completas del tensor; el mapa es de sólo lectura y privado, así que no se pierde nada (se vuelve a leer del fichero).
        unsafe { madvise(ini as *mut c_void, fin - ini, MADV_DONTNEED) };
    }
}

/// Bytes de VRAM que se guardan para los pesos que no son expertos (`SOSO_RESERVA_DENSA_MIB`, 3072 por defecto).
fn reserva_densa() -> usize {
    std::env::var("SOSO_RESERVA_DENSA_MIB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(3072) << 20
}

/// ¿Es la clave de un tensor de experto (`L12.E034.ffn_gate`)?
fn es_clave_experto(key: &str) -> bool {
    key.find(".E").is_some_and(|i| key[i + 2..].starts_with(|c: char| c.is_ascii_digit()))
}

fn es_iq_gpu(dtype: u8) -> bool {
    matches!(dtype, DTYPE_Q5_K | DTYPE_IQ2_S | DTYPE_IQ2_XS | DTYPE_IQ3_S | DTYPE_IQ3_XXS | DTYPE_IQ4_XS)
}
/// Elementos de un superbloque Q4_K: el kernel exige `cols` múltiplo.
const Q4_K_ELEMS: usize = 256;

/// Un experto del bloque de prefill con sus tres pesos ya en VRAM (puntero y dtype) y su posición en el búfer de activaciones.
struct PlanExp {
    i: usize,
    w: [(u64, u8); 3],
    abase: usize,
}

/// Bytes máximos de expertos que se suben de una vez al búfer de paso del prefill.
const TROZO_PASO: usize = 96 << 20;

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

/// Argumento de un lanzamiento (el valor se pasa por puntero, en los bytes bajos).
#[derive(Clone, Copy)]
enum Arg {
    P(u64),
    I(i32),
    L(i64),
    F(f32),
}

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
    stream_sync: unsafe extern "C" fn(*mut c_void) -> CuResult,
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
            stream_sync: simbolo(lib, "cuStreamSynchronize")?,
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
    f_iq: CuFunction,
    /// `None` = ya se intentó y no sirve (otro dtype, no cabe): no se reintenta en cada token.
    pesos: HashMap<String, Option<Peso>>,
    presupuesto: usize,
    /// Tope de VRAM para los pesos de **expertos** (`presupuesto` menos una reserva para las matrices densas, la cabeza de logits y el
    /// compartido, que se usan en todos los tokens): sin él, los expertos subidos primero agotaban la tarjeta y la cabeza de logits (0,9 GB)
    /// y parte de las matrices densas se quedaban en CPU.
    tope_expertos: usize,
    subiendo_experto: bool,
    usado: usize,
    losa_ptr: u64,
    losa_libre: usize,
    x_dev: u64,
    /// Búferes del prefill por bloques de expertos MoE: vectores del bloque, índices de token, activaciones y salidas.
    blq: [(u64, usize); 14],
    f_gateup_n: CuFunction,
    f_down_n: CuFunction,
    f_iq_matmul_n: CuFunction,
    f_gateup_multi: CuFunction,
    f_combinar: CuFunction,
    f_gdn_conv: CuFunction,
    f_gdn_rec: CuFunction,
    f_gdn_conv2: CuFunction,
    f_gdn_params: CuFunction,
    f_gdn_cola: CuFunction,
    f_sgemm: CuFunction,
    f_topk: CuFunction,
    /// Matrices de router (f32) subidas por clave.
    router_w: HashMap<String, u64>,
    /// Búferes de la capa GDN entera (ver `gdn_capa_gpu`).
    gbuf: [(u64, usize); 17],
    f_down_multi: CuFunction,
    f_attn_prefill: CuFunction,
    /// KV espejado por capa para la atención del prefill: (K, V, filas de capacidad).
    kvdev: HashMap<u32, (u64, u64, usize)>,
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
    modulo_iq: CuModule,
    /// Se incrementa cuando cambian las direcciones del KV espejado (invalida los grafos del decode).
    kv_gen: u64,
    /// Diagnóstico: funciones que no se lanzan (`SOSO_DECODE_SALTAR`).
    saltar: Vec<usize>,
    /// Los expertos que no están residentes se suben de paso, por trozos, a `stg` (no se cachean).
    flujo_expertos: bool,
    stg: (u64, usize),
    /// Doble búfer del paso de expertos: dispositivo, anclado en el host y stream de copia (sin bloqueo con el stream por defecto).
    stg2: [(u64, usize); 2],
    pin: [(usize, usize); 2],
    stream_copia: *mut c_void,
    dec: Option<decode::DecodeDev>,
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
            let mod_iq = cargar(PTX_IQ)?;
            let f_iq = func(mod_iq, "iq_matvec")?;
            let f_gateup_n = func(mod_iq, "iq_gateup_n")?;
            let f_down_n = func(mod_iq, "iq_down_n")?;
            let f_iq_matmul_n = func(mod_iq, "iq_matmul_mma")?;
            let f_attn_prefill = func(mod_iq, "attn_prefill_f16")?;
            let f_gateup_multi = func(mod_iq, "iq_gateup_mma")?;
            let f_combinar = func(mod_iq, "iq_combinar")?;
            let f_gdn_conv = func(mod_iq, "gdn_conv_silu")?;
            let f_gdn_rec = func(mod_iq, "gdn_rec")?;
            let f_gdn_conv2 = func(mod_iq, "gdn_conv_silu2")?;
            let f_gdn_params = func(mod_iq, "gdn_params")?;
            let f_gdn_cola = func(mod_iq, "gdn_cola")?;
            let f_sgemm = func(mod_iq, "sgemm_nt")?;
            let f_topk = func(mod_iq, "dec_router_topk")?;
            let f_down_multi = func(mod_iq, "iq_down_mma")?;
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
                f_iq,
                pesos: HashMap::new(),
                presupuesto: libre.saturating_sub(RESERVA_VRAM),
                tope_expertos: libre.saturating_sub(RESERVA_VRAM).saturating_sub(reserva_densa()),
                subiendo_experto: false,
                usado: 0,
                losa_ptr: 0,
                losa_libre: 0,
                x_dev: 0,
                blq: [(0, 0); 14],
                f_gateup_n,
                f_down_n,
                f_iq_matmul_n,
                f_attn_prefill,
                f_gateup_multi,
                f_combinar,
                f_gdn_conv,
                f_gdn_rec,
                f_gdn_conv2,
                f_gdn_params,
                f_gdn_cola,
                f_sgemm,
                f_topk,
                router_w: HashMap::new(),
                gbuf: [(0, 0); 17],
                f_down_multi,
                kvdev: HashMap::new(),
                modulo_iq: mod_iq,
                kv_gen: 0,
                saltar: Vec::new(),
                flujo_expertos: false,
                stg: (0, 0),
                stg2: [(0, 0); 2],
                pin: [(0, 0); 2],
                stream_copia: std::ptr::null_mut(),
                dec: None,
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

    /// Sube el tensor si cabe; `None` si no. Una vez en VRAM, las páginas del host dejan de hacer falta (salvo si la GPU falla
    /// y la capa se hace en CPU): se sueltan del mapa del proceso para que, bajo presión de memoria, el kernel las recupere antes que
    /// las de los tensores que sólo viven en RAM. Sin esto, el 35B-A3B (14 GB de pesos) mantenía en RAM una copia de lo que ya
    /// estaba en la tarjeta y, con otras cargas en la máquina, la caché de páginas se agitaba (prefill de minutos).
    fn subir_peso(
        &mut self,
        view: &TensorView<'_>,
        rows: usize,
        cols: usize,
    ) -> Result<Option<Peso>, String> {
        let r = self.subir_peso_interno(view, rows, cols)?;
        if r.is_some() {
            soltar_paginas_host(view.bytes);
        }
        Ok(r)
    }

    fn subir_peso_interno(
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
                if self.usado + total + (self.x_cap + self.y_cap) * 4 > if self.subiendo_experto { self.tope_expertos } else { self.presupuesto } {
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
                if self.usado + total + (self.x_cap + self.y_cap) * 4 > if self.subiendo_experto { self.tope_expertos } else { self.presupuesto } {
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
            DTYPE_Q4_K if cols % Q4_K_ELEMS == 0 => {
                let total = view.bytes.len();
                if total != rows * (cols / Q4_K_ELEMS) * Q4_K_BLOCK_BYTES {
                    return Ok(None);
                }
                if self.usado + total + (self.x_cap + self.y_cap) * 4 > if self.subiendo_experto { self.tope_expertos } else { self.presupuesto } {
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
            d if es_iq_gpu(d) && cols % 256 == 0 => {
                let total = view.bytes.len();
                let rb = sosomodel::dequant::row_bytes(d, cols).unwrap_or(0);
                if rb == 0 || total != rows * rb {
                    return Ok(None);
                }
                if self.usado + total + (self.x_cap + self.y_cap) * 4 > if self.subiendo_experto { self.tope_expertos } else { self.presupuesto } {
                    return Ok(None);
                }
                let dd = self.alloc_peso(total)?;
                self.subir(dd, view.bytes)?;
                self.usado += total;
                Ok(Some(Peso { d: dd, qs: 0, dtype: d, rows, cols, bytes: total }))
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

    /// Lanza el kernel del peso `dtype` sobre `n` vectores ya en el dispositivo (`x_dev`, `y_dev`); `false` si no hay kernel.
    fn lanzar_peso(
        &mut self,
        dtype: u8,
        pd: u64,
        pq: u64,
        rows: usize,
        cols: usize,
        n: usize,
        x_dev: u64,
        y_dev: u64,
    ) -> Result<bool, String> {
        let (r32, c32) = (rows as c_int, cols as c_int);
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
            DTYPE_Q4_K if n >= 2 || cols > 4096 => {
                // Bloque de tokens (o filas más anchas que el kernel de un vector): tensor cores con el decodificador Q4_K (tipo 1002).
                let rb = ((cols / 256) * 144) as i64;
                let (mut a0, mut a1, mut a2) = (pd, x_dev, y_dev);
                let (mut a3, mut a4, mut a5, mut a6, mut a7) = (r32, c32, n as c_int, 1002 as c_int, rb);
                let mut args = [
                    &mut a0 as *mut _ as *mut c_void,
                    &mut a1 as *mut _ as *mut c_void,
                    &mut a2 as *mut _ as *mut c_void,
                    &mut a3 as *mut _ as *mut c_void,
                    &mut a4 as *mut _ as *mut c_void,
                    &mut a5 as *mut _ as *mut c_void,
                    &mut a6 as *mut _ as *mut c_void,
                    &mut a7 as *mut _ as *mut c_void,
                ];
                self.lanzar3s(self.f_iq_matmul_n, ((rows as u32).div_ceil(64), (n as u32).div_ceil(64), 1), (128, 1, 1), 0, &mut args)?;
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
            DTYPE_Q8_0 if n >= 4 => {
                // Bloque de tokens: tensor cores (`iq_matmul_mma` con el decodificador de Q8_0 de sosomodel, tipo 1000).
                let rb = ((cols / 32) * 36) as i64;
                let (mut a0, mut a1, mut a2) = (pd, x_dev, y_dev);
                let (mut a3, mut a4, mut a5, mut a6, mut a7) = (r32, c32, n as c_int, 1000 as c_int, rb);
                let mut args = [
                    &mut a0 as *mut _ as *mut c_void,
                    &mut a1 as *mut _ as *mut c_void,
                    &mut a2 as *mut _ as *mut c_void,
                    &mut a3 as *mut _ as *mut c_void,
                    &mut a4 as *mut _ as *mut c_void,
                    &mut a5 as *mut _ as *mut c_void,
                    &mut a6 as *mut _ as *mut c_void,
                    &mut a7 as *mut _ as *mut c_void,
                ];
                self.lanzar3s(self.f_iq_matmul_n, ((rows as u32).div_ceil(64), (n as u32).div_ceil(64), 1), (128, 1, 1), 0, &mut args)?;
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
            d if es_iq_gpu(d) && n == 1 => {
                let rb = sosomodel::dequant::row_bytes(d, cols).unwrap_or(0) as i64;
                let ggml = sosomodel::layout::ggml_de_dtype_iq(d).unwrap_or(0) as c_int;
                let (mut a0, mut a1, mut a2) = (pd, x_dev, y_dev);
                let (mut a3, mut a4, mut a5, mut a6) = (r32, c32, ggml, rb);
                let mut args = [
                    &mut a0 as *mut _ as *mut c_void,
                    &mut a1 as *mut _ as *mut c_void,
                    &mut a2 as *mut _ as *mut c_void,
                    &mut a3 as *mut _ as *mut c_void,
                    &mut a4 as *mut _ as *mut c_void,
                    &mut a5 as *mut _ as *mut c_void,
                    &mut a6 as *mut _ as *mut c_void,
                ];
                self.lanzar3s(self.f_iq, ((rows as u32).div_ceil(8), 1, 1), (256, 1, 1), (cols * 4) as u32, &mut args)?;
            }
            d if es_iq_gpu(d) => {
                // Varios tokens: el kernel descodifica cada bloque de pesos una vez y lo usa para IQ_TILE tokens.
                let rb = sosomodel::dequant::row_bytes(d, cols).unwrap_or(0) as i64;
                let ggml = sosomodel::layout::ggml_de_dtype_iq(d).unwrap_or(0) as c_int;
                let (mut a0, mut a1, mut a2) = (pd, x_dev, y_dev);
                let (mut a3, mut a4, mut a5, mut a6, mut a7) = (r32, c32, n as c_int, ggml, rb);
                let mut args = [
                    &mut a0 as *mut _ as *mut c_void,
                    &mut a1 as *mut _ as *mut c_void,
                    &mut a2 as *mut _ as *mut c_void,
                    &mut a3 as *mut _ as *mut c_void,
                    &mut a4 as *mut _ as *mut c_void,
                    &mut a5 as *mut _ as *mut c_void,
                    &mut a6 as *mut _ as *mut c_void,
                    &mut a7 as *mut _ as *mut c_void,
                ];
                self.lanzar3s(self.f_iq_matmul_n, ((rows as u32).div_ceil(64), (n as u32).div_ceil(64), 1), (128, 1, 1), 0, &mut args)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn lanzar_a(&self, f: CuFunction, grid: (u32, u32, u32), block: (u32, u32, u32), args: &[Arg]) -> Result<(), String> {
        let mut vals: Vec<u64> = args
            .iter()
            .map(|a| match *a {
                Arg::P(v) => v,
                Arg::I(v) => v as u32 as u64,
                Arg::L(v) => v as u64,
                Arg::F(v) => v.to_bits() as u64,
            })
            .collect();
        let mut ptrs: Vec<*mut c_void> = vals.iter_mut().map(|v| v as *mut u64 as *mut c_void).collect();
        self.lanzar3s(f, grid, block, 0, &mut ptrs)
    }

    fn gbuf_asegurar(&mut self, i: usize, bytes: usize) -> Result<u64, String> {
        if bytes > self.gbuf[i].1 {
            if self.gbuf[i].0 != 0 {
                unsafe { (self.api.mem_free)(self.gbuf[i].0) };
            }
            let cap = bytes.max(1 << 16).next_power_of_two();
            self.gbuf[i] = (self.alloc(cap)?, cap);
        }
        Ok(self.gbuf[i].0)
    }

    /// Capa GDN entera de un bloque en el dispositivo: ver [`GpuDispatch::gdn_capa`].
    fn gdn_capa_gpu(
        &mut self,
        c: &soso_llm_core::gpu::GdnCapa<'_>,
        xs: &[f32],
        n: usize,
        h: usize,
        conv_estado: &mut [f32],
        estado: &mut [f32],
        salida: &mut [f32],
    ) -> Result<bool, String> {
        let (n_k, n_v, d, ker) = (c.n_k, c.n_v, c.d, c.kernel);
        if d != 128 || n < ker || ker < 2 || n_k == 0 || n_v % n_k != 0 {
            if std::env::var_os("SOSO_TRAZA_GPU").is_some() {
                eprintln!("[gpu] gdn_capa rechazada: d {d} n {n} ker {ker} n_k {n_k} n_v {n_v}");
            }
            return Ok(false);
        }
        let (qkv_dim, v_dim, prev) = (2 * n_k * d + n_v * d, n_v * d, ker - 1);
        if xs.len() != n * h || salida.len() != n * h || conv_estado.len() != prev * qkv_dim || estado.len() != n_v * d * d
            || c.beta_w.len() != n_v * h || c.alpha_w.len() != n_v * h || c.conv_w.len() != qkv_dim * ker
        {
            return Ok(false);
        }
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        // pesos grandes (Q8_0) residentes
        let dims = [(qkv_dim, h), (v_dim, h), (h, v_dim)];
        let mut w = [0u64; 3];
        let mut cod = [(0i32, 0i64); 3];
        for k in 0..3 {
            let Some(cr) = codigo_mma(c.vistas[k].dtype, dims[k].1) else {
                if std::env::var_os("SOSO_TRAZA_GPU").is_some() {
                    eprintln!("[gpu] gdn_capa: sin código para {} dtype {} cols {}", c.claves[k], c.vistas[k].dtype, dims[k].1);
                }
                return Ok(false);
            };
            cod[k] = cr;
            if !self.pesos.contains_key(c.claves[k]) {
                let p = self.subir_peso(c.vistas[k], dims[k].0, dims[k].1)?;
                self.pesos.insert(c.claves[k].to_string(), p);
            }
            match self.pesos.get(c.claves[k]) {
                Some(Some(p)) if p.dtype == c.vistas[k].dtype && p.rows == dims[k].0 && p.cols == dims[k].1 => w[k] = p.d,
                _ => {
                    if std::env::var_os("SOSO_TRAZA_GPU").is_some() {
                        eprintln!("[gpu] gdn_capa: {} no residente", c.claves[k]);
                    }
                    return Ok(false);
                }
            }
        }
        let f = |v: &[f32]| unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4) };
        let x_dev = self.gbuf_asegurar(0, xs.len() * 4)?;
        let qkv_dev = self.gbuf_asegurar(1, n * qkv_dim * 4)?;
        let z_dev = self.gbuf_asegurar(2, n * v_dim * 4)?;
        let braw_dev = self.gbuf_asegurar(3, n * n_v * 4)?;
        let araw_dev = self.gbuf_asegurar(4, n * n_v * 4)?;
        let up_dev = self.gbuf_asegurar(5, n * qkv_dim * 4)?;
        let dec_dev = self.gbuf_asegurar(6, n * n_v * 4)?;
        let bet_dev = self.gbuf_asegurar(7, n * n_v * 4)?;
        let o_dev = self.gbuf_asegurar(8, n * v_dim * 4)?;
        let y_dev = self.gbuf_asegurar(9, n * h * 4)?;
        let s_dev = self.gbuf_asegurar(10, estado.len() * 4)?;
        let cs_dev = self.gbuf_asegurar(11, conv_estado.len() * 4)?;
        let bw_dev = self.gbuf_asegurar(12, c.beta_w.len() * 4)?;
        let aw_dev = self.gbuf_asegurar(13, c.alpha_w.len() * 4)?;
        let cw_dev = self.gbuf_asegurar(14, c.conv_w.len() * 4)?;
        let par_dev = self.gbuf_asegurar(15, (c.dt.len() + c.a.len() + c.norma.len()) * 4)?;
        let (dt_dev, a_dev, nw_dev) = (par_dev, par_dev + (c.dt.len() * 4) as u64, par_dev + ((c.dt.len() + c.a.len()) * 4) as u64);
        self.subir(x_dev, f(xs))?;
        self.subir(s_dev, f(estado))?;
        self.subir(cs_dev, f(conv_estado))?;
        self.subir(bw_dev, f(c.beta_w))?;
        self.subir(aw_dev, f(c.alpha_w))?;
        self.subir(cw_dev, f(c.conv_w))?;
        self.subir(dt_dev, f(c.dt))?;
        self.subir(a_dev, f(c.a))?;
        self.subir(nw_dev, f(c.norma))?;
        let nn = n as i32;
        let mm = |s: &Self, w: u64, rows: usize, cols: usize, dtype: i32, rb: i64, x: u64, y: u64| {
            s.lanzar_a(s.f_iq_matmul_n, ((rows as u32).div_ceil(64), (n as u32).div_ceil(64), 1), (128, 1, 1),
                &[Arg::P(w), Arg::P(x), Arg::P(y), Arg::I(rows as i32), Arg::I(cols as i32), Arg::I(nn), Arg::I(dtype), Arg::L(rb)])
        };
        mm(self, w[0], qkv_dim, h, cod[0].0, cod[0].1, x_dev, qkv_dev)?;
        mm(self, w[1], v_dim, h, cod[1].0, cod[1].1, x_dev, z_dev)?;
        mm(self, bw_dev, n_v, h, 1001, (h * 4) as i64, x_dev, braw_dev)?;
        mm(self, aw_dev, n_v, h, 1001, (h * 4) as i64, x_dev, araw_dev)?;
        self.lanzar_a(self.f_gdn_params, (((n * n_v) as u32).div_ceil(256), 1, 1), (256, 1, 1),
            &[Arg::P(araw_dev), Arg::P(braw_dev), Arg::P(dt_dev), Arg::P(a_dev), Arg::P(dec_dev), Arg::P(bet_dev), Arg::I(nn), Arg::I(n_v as i32)])?;
        self.lanzar_a(self.f_gdn_conv2, ((qkv_dim as u32).div_ceil(256), n as u32, 1), (256, 1, 1),
            &[Arg::P(cs_dev), Arg::P(qkv_dev), Arg::P(cw_dev), Arg::P(up_dev), Arg::I(nn), Arg::I(qkv_dim as i32), Arg::I(ker as i32)])?;
        self.lanzar_a(self.f_gdn_rec, (n_v as u32, 1, 1), (256, 1, 1),
            &[Arg::P(up_dev), Arg::P(z_dev), Arg::P(dec_dev), Arg::P(bet_dev), Arg::P(nw_dev), Arg::P(s_dev), Arg::P(o_dev), Arg::I(nn), Arg::I(n_k as i32), Arg::I(n_v as i32), Arg::F(c.eps)])?;
        mm(self, w[2], h, v_dim, cod[2].0, cod[2].1, o_dev, y_dev)?;
        // estado de la conv para el bloque siguiente: las `prev` últimas filas de qkv (se baja antes de pisarlo)
        self.lanzar_a(self.f_gdn_cola, (((prev * qkv_dim) as u32).div_ceil(256), 1, 1), (256, 1, 1),
            &[Arg::P(qkv_dev), Arg::P(cs_dev), Arg::I(nn), Arg::I(qkv_dim as i32), Arg::I(prev as i32)])?;
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        ck(unsafe { (self.api.dtoh)(salida.as_mut_ptr() as *mut c_void, y_dev, salida.len() * 4) }, "cuMemcpyDtoH")?;
        ck(unsafe { (self.api.dtoh)(estado.as_mut_ptr() as *mut c_void, s_dev, estado.len() * 4) }, "cuMemcpyDtoH")?;
        ck(unsafe { (self.api.dtoh)(conv_estado.as_mut_ptr() as *mut c_void, cs_dev, conv_estado.len() * 4) }, "cuMemcpyDtoH")?;
        self.stats.launches += 1;
        Ok(true)
    }

    /// Gated DeltaNet de un bloque: ver [`GpuDispatch::gdn_bloque`].
    #[allow(clippy::too_many_arguments)]
    fn gdn_bloque_gpu(
        &mut self,
        hist: &[f32],
        conv: &[f32],
        z: &[f32],
        decay: &[f32],
        beta: &[f32],
        norma: &[f32],
        estado: &mut [f32],
        salida: &mut [f32],
        n: usize,
        n_k: usize,
        n_v: usize,
        d: usize,
        kernel: usize,
        eps: f32,
    ) -> Result<bool, String> {
        if d != 128 || n == 0 || n_k == 0 || n_v % n_k != 0 {
            return Ok(false);
        }
        let qkv_dim = 2 * n_k * d + n_v * d;
        let v_dim = n_v * d;
        if hist.len() != (kernel - 1 + n) * qkv_dim || conv.len() != qkv_dim * kernel || z.len() != n * v_dim || decay.len() != n * n_v
            || beta.len() != n * n_v || norma.len() != d || estado.len() != n_v * d * d || salida.len() != n * v_dim
        {
            return Ok(false);
        }
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        let f = |v: &[f32]| unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4) };
        let h_dev = self.blq_asegurar(8, hist.len() * 4)?;
        let c_dev = self.blq_asegurar(9, conv.len() * 4)?;
        let up_dev = self.blq_asegurar(10, n * qkv_dim * 4)?;
        let z_dev = self.blq_asegurar(11, (z.len() + decay.len() + beta.len() + norma.len()) * 4)?;
        let s_dev = self.blq_asegurar(12, estado.len() * 4)?;
        let o_dev = self.blq_asegurar(13, salida.len() * 4)?;
        let (dec_dev, bet_dev, nw_dev) = (z_dev + (z.len() * 4) as u64, z_dev + ((z.len() + decay.len()) * 4) as u64, z_dev + ((z.len() + decay.len() + beta.len()) * 4) as u64);
        self.subir(h_dev, f(hist))?;
        self.subir(c_dev, f(conv))?;
        self.subir(z_dev, f(z))?;
        self.subir(dec_dev, f(decay))?;
        self.subir(bet_dev, f(beta))?;
        self.subir(nw_dev, f(norma))?;
        self.subir(s_dev, f(estado))?;
        let (mut a0, mut a1, mut a2) = (h_dev, c_dev, up_dev);
        let (mut a3, mut a4, mut a5) = (n as c_int, qkv_dim as c_int, kernel as c_int);
        let mut args = [
            &mut a0 as *mut _ as *mut c_void,
            &mut a1 as *mut _ as *mut c_void,
            &mut a2 as *mut _ as *mut c_void,
            &mut a3 as *mut _ as *mut c_void,
            &mut a4 as *mut _ as *mut c_void,
            &mut a5 as *mut _ as *mut c_void,
        ];
        self.lanzar3s(self.f_gdn_conv, ((qkv_dim as u32).div_ceil(256), n as u32, 1), (256, 1, 1), 0, &mut args)?;
        let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5, mut b6) = (up_dev, z_dev, dec_dev, bet_dev, nw_dev, s_dev, o_dev);
        let (mut b7, mut b8, mut b9, mut b10) = (n as c_int, n_k as c_int, n_v as c_int, eps);
        let mut args = [
            &mut b0 as *mut _ as *mut c_void,
            &mut b1 as *mut _ as *mut c_void,
            &mut b2 as *mut _ as *mut c_void,
            &mut b3 as *mut _ as *mut c_void,
            &mut b4 as *mut _ as *mut c_void,
            &mut b5 as *mut _ as *mut c_void,
            &mut b6 as *mut _ as *mut c_void,
            &mut b7 as *mut _ as *mut c_void,
            &mut b8 as *mut _ as *mut c_void,
            &mut b9 as *mut _ as *mut c_void,
            &mut b10 as *mut _ as *mut c_void,
        ];
        self.lanzar3s(self.f_gdn_rec, (n_v as u32, 1, 1), (256, 1, 1), 0, &mut args)?;
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        ck(unsafe { (self.api.dtoh)(salida.as_mut_ptr() as *mut c_void, o_dev, salida.len() * 4) }, "cuMemcpyDtoH")?;
        ck(unsafe { (self.api.dtoh)(estado.as_mut_ptr() as *mut c_void, s_dev, estado.len() * 4) }, "cuMemcpyDtoH")?;
        self.stats.launches += 1;
        Ok(true)
    }

    /// Atención del prefill de un bloque en la GPU: ver [`GpuDispatch::atencion_prefill`].
    #[allow(clippy::too_many_arguments)]
    fn atencion_prefill_gpu(
        &mut self,
        capa: u32,
        q: &[f32],
        out: &mut [f32],
        n: usize,
        seq_antes: usize,
        heads: usize,
        kv_heads: usize,
        hd: usize,
        k: &[u16],
        v: &[u16],
        filas_validas: &mut usize,
    ) -> Result<bool, String> {
        let group = if kv_heads == 0 { 0 } else { heads / kv_heads };
        if hd > 256 || group == 0 || group > 16 || heads % kv_heads != 0 || n == 0 {
            return Ok(false);
        }
        let kv_dim = kv_heads * hd;
        let total = seq_antes + n;
        if k.len() < total * kv_dim || v.len() < total * kv_dim || q.len() != n * heads * hd || out.len() != n * heads * hd {
            return Ok(false);
        }
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        // 1) KV espejado: capacidad, y subir las filas que el dispositivo no tenga válidas
        let valid = (*filas_validas).min(seq_antes);
        let (mut kd, mut vd, mut cap) = self.kvdev.get(&capa).copied().unwrap_or((0, 0, 0));
        if total > cap {
            let ncap = total.div_ceil(4096) * 4096 + 4096;
            let (nk, nv) = (self.alloc(ncap * kv_dim * 2)?, self.alloc(ncap * kv_dim * 2)?);
            if kd != 0 && valid > 0 {
                ck(unsafe { (self.api.dtod)(nk, kd, valid * kv_dim * 2) }, "cuMemcpyDtoD")?;
                ck(unsafe { (self.api.dtod)(nv, vd, valid * kv_dim * 2) }, "cuMemcpyDtoD")?;
            }
            if kd != 0 {
                unsafe {
                    (self.api.mem_free)(kd);
                    (self.api.mem_free)(vd);
                }
            }
            (kd, vd, cap) = (nk, nv, ncap);
            self.kvdev.insert(capa, (kd, vd, cap));
            self.kv_gen += 1;
        }
        let (ini, fin) = (valid * kv_dim, total * kv_dim);
        self.subir(kd + (ini * 2) as u64, unsafe { std::slice::from_raw_parts(k.as_ptr().add(ini) as *const u8, (fin - ini) * 2) })?;
        self.subir(vd + (ini * 2) as u64, unsafe { std::slice::from_raw_parts(v.as_ptr().add(ini) as *const u8, (fin - ini) * 2) })?;
        *filas_validas = total;
        // 2) consultas, lanzamiento, resultado
        let q_dev = self.blq_asegurar(0, q.len() * 4)?;
        let o_dev = self.blq_asegurar(3, out.len() * 4)?;
        self.subir(q_dev, unsafe { std::slice::from_raw_parts(q.as_ptr() as *const u8, q.len() * 4) })?;
        let tb = 16 / group;
        let (mut a0, mut a1, mut a2, mut a3) = (q_dev, kd, vd, o_dev);
        let (mut a4, mut a5, mut a6, mut a7, mut a8, mut a9) = (n as c_int, seq_antes as c_int, heads as c_int, kv_heads as c_int, hd as c_int, group as c_int);
        let mut args = [
            &mut a0 as *mut _ as *mut c_void,
            &mut a1 as *mut _ as *mut c_void,
            &mut a2 as *mut _ as *mut c_void,
            &mut a3 as *mut _ as *mut c_void,
            &mut a4 as *mut _ as *mut c_void,
            &mut a5 as *mut _ as *mut c_void,
            &mut a6 as *mut _ as *mut c_void,
            &mut a7 as *mut _ as *mut c_void,
            &mut a8 as *mut _ as *mut c_void,
            &mut a9 as *mut _ as *mut c_void,
        ];
        self.lanzar3s(self.f_attn_prefill, (kv_heads as u32, (n as u32).div_ceil(tb as u32), 1), (256, 1, 1), 0, &mut args)?;
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        ck(unsafe { (self.api.dtoh)(out.as_mut_ptr() as *mut c_void, o_dev, out.len() * 4) }, "cuMemcpyDtoH")?;
        self.stats.launches += 1;
        Ok(true)
    }

    /// Asegura que el búfer `i` del prefill por bloques tenga al menos `bytes`.
    fn blq_asegurar(&mut self, i: usize, bytes: usize) -> Result<u64, String> {
        if bytes > self.blq[i].1 {
            if self.blq[i].0 != 0 {
                unsafe { (self.api.mem_free)(self.blq[i].0) };
            }
            let cap = bytes.max(1 << 20).next_power_of_two();
            self.blq[i] = (self.alloc(cap)?, cap);
        }
        Ok(self.blq[i].0)
    }

    /// Router por bloque en la GPU: ver [`GpuDispatch::router_topk`].
    #[allow(clippy::too_many_arguments)]
    fn router_topk_gpu(
        &mut self,
        key: &str,
        w: &[f32],
        xs: &[f32],
        n: usize,
        h: usize,
        n_exp: usize,
        top_k: usize,
        idx: &mut [i32],
        pesos: &mut [f32],
    ) -> Result<bool, String> {
        if w.len() != n_exp * h || xs.len() != n * h || idx.len() != n * top_k || pesos.len() != n * top_k || n_exp > 1024 || n == 0 {
            return Ok(false);
        }
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        let w_dev = match self.router_w.get(key) {
            Some(&d) => d,
            None => {
                let d = self.alloc(w.len() * 4)?;
                self.subir(d, unsafe { std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 4) })?;
                self.router_w.insert(key.to_string(), d);
                d
            }
        };
        let x_dev = self.blq_asegurar(8, n * h * 4)?;
        let l_dev = self.blq_asegurar(9, n * n_exp * 4)?;
        let i_dev = self.blq_asegurar(10, n * top_k * 4)?;
        let p_dev = self.blq_asegurar(11, n * top_k * 4)?;
        self.subir(x_dev, unsafe { std::slice::from_raw_parts(xs.as_ptr() as *const u8, xs.len() * 4) })?;
        self.lanzar_a(
            self.f_sgemm,
            ((n_exp as u32).div_ceil(16), (n as u32).div_ceil(16), 1),
            (16, 16, 1),
            &[Arg::P(w_dev), Arg::P(x_dev), Arg::P(l_dev), Arg::I(n_exp as i32), Arg::I(h as i32), Arg::I(n as i32)],
        )?;
        self.lanzar_a(self.f_topk, (n as u32, 1, 1), (256, 1, 1), &[Arg::P(l_dev), Arg::P(i_dev), Arg::P(p_dev), Arg::I(n_exp as i32), Arg::I(top_k as i32)])?;
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        ck(unsafe { (self.api.dtoh)(idx.as_mut_ptr() as *mut c_void, i_dev, idx.len() * 4) }, "cuMemcpyDtoH")?;
        ck(unsafe { (self.api.dtoh)(pesos.as_mut_ptr() as *mut c_void, p_dev, pesos.len() * 4) }, "cuMemcpyDtoH")?;
        self.stats.launches += 1;
        Ok(true)
    }

    /// Prefill MoE por bloques: ver [`GpuDispatch::expertos_bloque`].
    fn expertos_bloque_gpu(
        &mut self,
        xs: &[f32],
        n: usize,
        h: usize,
        ffn: usize,
        top_k: usize,
        pesos: &[f32],
        exps: &mut [soso_llm_core::gpu::ExpertoBloque<'_>],
        suma: &mut [f32],
        hecho: &mut [bool],
    ) -> Result<(), String> {
        let t_a = std::time::Instant::now();
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        // 1) qué expertos se pueden resolver aquí (pesos residentes o que caben): se anotan sus punteros.
        let mut plan: Vec<PlanExp> = Vec::new();
        let mut stream: Vec<usize> = Vec::new();
        let mut total = 0usize;
        for (i, e) in exps.iter().enumerate() {
            let dims = [(ffn, h), (ffn, h), (h, ffn)];
            let mut w = [(0u64, 0u8); 3];
            let mut ok = true;
            for k in 0..3 {
                let key = e.claves[k];
                if !self.pesos.contains_key(key) {
                    // Con el decode en GPU residente la VRAM es de las capas enteras: los expertos de las demás pasan por el búfer de paso.
                    if self.flujo_expertos && !e.compartido && e.vistas[k].dtype != 0 && !e.tokens.is_empty() {
                        stream.push(i);
                        ok = false;
                        break;
                    }
                    // Vista vacía (`dtype` 0): el llamante sólo la da para lo que ya cree residente; no hay nada que subir.
                    if e.vistas[k].dtype == 0 {
                        ok = false;
                        break;
                    }
                    self.subiendo_experto = !e.compartido;
                    let p = self.subir_peso(e.vistas[k], dims[k].0, dims[k].1);
                    self.subiendo_experto = false;
                    let p = p?;
                    self.pesos.insert(key.to_string(), p);
                }
                match self.pesos.get(key) {
                    Some(Some(p)) if codigo_mma(p.dtype, dims[k].1).is_some() && p.rows == dims[k].0 && p.cols == dims[k].1 && (e.vistas[k].dtype == 0 || p.dtype == e.vistas[k].dtype) => {
                        w[k] = (p.d, p.dtype)
                    }
                    _ => ok = false,
                }
            }
            if ok && !e.tokens.is_empty() {
                plan.push(PlanExp { i, w, abase: total });
                total += e.tokens.len();
            }
        }
        self.stats.flujo_aciertos += plan.len() as u64;
        self.stats.flujo_fallos += (exps.len() - plan.len()) as u64;
        if plan.is_empty() && stream.is_empty() {
            return Ok(());
        }
        if !plan.is_empty() {
            self.correr_plan(xs, n, h, ffn, top_k, pesos, &*exps, &plan, suma, (true, false, stream.is_empty()))?;
            for pl in &plan {
                hecho[pl.i] = true;
            }
        }
        if !stream.is_empty() {
            self.expertos_de_paso(xs, n, h, ffn, top_k, pesos, &*exps, &stream, suma, hecho, !plan.is_empty())?;
        }
        if std::env::var_os("SOSO_TRAZA_GPU").is_some() {
            eprintln!("[gpu] llamada expertos: {:.1} ms total ({} residentes, {} de paso)", t_a.elapsed().as_secs_f64() * 1e3, plan.len(), stream.len());
        }
        Ok(())
    }

    /// Prefill de los expertos no residentes: sus pesos pasan por un doble búfer de paso (sin cachearse), en trozos de `TROZO_PASO`. Tres etapas
    /// solapadas: copia a memoria anclada (hilos), DMA asíncrono y cálculo del trozo anterior.
    #[allow(clippy::too_many_arguments)]
    fn expertos_de_paso(
        &mut self,
        xs: &[f32],
        n: usize,
        h: usize,
        ffn: usize,
        top_k: usize,
        pesos: &[f32],
        exps: &[soso_llm_core::gpu::ExpertoBloque<'_>],
        stream: &[usize],
        suma: &mut [f32],
        hecho: &mut [bool],
        mut escrita: bool,
    ) -> Result<(), String> {
        let dims = [(ffn, h), (ffn, h), (h, ffn)];
        // 1) partir en trozos: (experto, [(desplazamiento, puntero origen, bytes, dtype); 3])
        type Sub = (usize, [(usize, usize, usize, u8); 3]);
        let mut trozos: Vec<(Vec<Sub>, usize)> = Vec::new();
        let (mut cur, mut bytes): (Vec<Sub>, usize) = (Vec::new(), 0);
        for &i in stream {
            let e = &exps[i];
            let mut desp = [(0usize, 0usize, 0usize, 0u8); 3];
            let mut tam = 0usize;
            let mut ok = true;
            for k in 0..3 {
                let v = e.vistas[k];
                match codigo_mma(v.dtype, dims[k].1) {
                    Some((_, rb)) if v.bytes.len() as i64 == dims[k].0 as i64 * rb => {
                        let o = (bytes + tam).div_ceil(ALINEACION) * ALINEACION - bytes;
                        desp[k] = (bytes + o, v.bytes.as_ptr() as usize, v.bytes.len(), v.dtype);
                        tam = o + v.bytes.len();
                    }
                    _ => ok = false,
                }
            }
            if !ok {
                continue;
            }
            if bytes > 0 && bytes + tam > TROZO_PASO {
                trozos.push((std::mem::take(&mut cur), bytes));
                bytes = 0;
                // el experto se vuelve a colocar al principio del trozo nuevo
                let mut desp2 = desp;
                let mut t2 = 0usize;
                for k in 0..3 {
                    let o = t2.div_ceil(ALINEACION) * ALINEACION;
                    desp2[k].0 = o;
                    t2 = o + desp2[k].2;
                }
                cur.push((i, desp2));
                bytes = t2;
                continue;
            }
            bytes += tam;
            cur.push((i, desp));
        }
        if !cur.is_empty() {
            trozos.push((cur, bytes));
        }
        if trozos.is_empty() {
            return Ok(());
        }
        // 2) búferes (dispositivo ×2, anclados ×2, stream de copia)
        let cap = trozos.iter().map(|t| t.1).max().unwrap_or(0).max(TROZO_PASO).div_ceil(2 << 20) * (2 << 20);
        let con_pin = self.grafo.is_some();
        if con_pin && self.stream_copia.is_null() {
            let g = self.grafo.as_ref().unwrap();
            let mut st: *mut c_void = std::ptr::null_mut();
            ck(unsafe { (g.stream_create)(&mut st, 1) }, "cuStreamCreate(copia)")?;
            self.stream_copia = st;
        }
        for b in 0..2 {
            if cap > self.stg2[b].1 {
                if self.stg2[b].0 != 0 {
                    unsafe { (self.api.mem_free)(self.stg2[b].0) };
                }
                self.stg2[b] = (self.alloc(cap)?, cap);
            }
            if con_pin && cap > self.pin[b].1 {
                let g = self.grafo.as_ref().unwrap();
                let mut p: *mut c_void = std::ptr::null_mut();
                ck(unsafe { (g.host_alloc)(&mut p, cap) }, "cuMemAllocHost")?;
                self.pin[b] = (p as usize, cap); // (no se libera: vive lo que el proceso)
            }
        }
        // copia de un trozo a un destino del host (anclado) con varios hilos
        let copiar = |t: &Vec<Sub>, dst: usize| {
            let piezas: Vec<(usize, usize, usize)> = t.iter().flat_map(|(_, d)| d.iter().map(|x| (x.0, x.1, x.2))).collect();
            let nh = 4usize.min(piezas.len().max(1));
            std::thread::scope(|sc| {
                for q in 0..nh {
                    let piezas = &piezas;
                    sc.spawn(move || {
                        for (idx, &(off, p, l)) in piezas.iter().enumerate() {
                            if idx % nh == q {
                                // SAFETY: origen = mapa del modelo (vivo); destino = búfer anclado de `cap` bytes; trozos disjuntos.
                                unsafe { std::ptr::copy_nonoverlapping(p as *const u8, (dst + off) as *mut u8, l) };
                            }
                        }
                    });
                }
            });
        };
        let t_sub = std::time::Instant::now();
        let m = trozos.len();
        let mut total_bytes = 0usize;
        if con_pin {
            let g_htod = self.grafo.as_ref().unwrap().htod_async;
            let g_sync = self.grafo.as_ref().unwrap().stream_sync;
            let (pin, stg, sc) = (self.pin, self.stg2, self.stream_copia);
            let dma = |k: usize| -> Result<(), String> {
                let b = k % 2;
                ck(unsafe { g_htod(stg[b].0, pin[b].0 as *const c_void, trozos[k].1, sc) }, "cuMemcpyHtoDAsync")
            };
            let sincro = || ck(unsafe { g_sync(sc) }, "cuStreamSynchronize");
            // prólogo: trozo 0 en el dispositivo y trozo 1 ya copiado al anclado
            let t_p = std::time::Instant::now();
            copiar(&trozos[0].0, pin[0].0);
            let t_copia0 = t_p.elapsed().as_secs_f64() * 1e3;
            dma(0)?;
            sincro()?;
            let t_dma0 = t_p.elapsed().as_secs_f64() * 1e3 - t_copia0;
            if m > 1 {
                copiar(&trozos[1].0, pin[1].0);
            }
            let (mut t_calc, mut t_join, mut t_sync) = (0f64, 0f64, 0f64);
            let t_prologo = t_p.elapsed().as_secs_f64() * 1e3;
            for k in 0..m {
                if k + 1 < m {
                    dma(k + 1)?;
                }
                let plan: Vec<PlanExp> = {
                    let mut ab = 0usize;
                    trozos[k]
                        .0
                        .iter()
                        .map(|(i, d)| {
                            let w = [(stg[k % 2].0 + d[0].0 as u64, d[0].3), (stg[k % 2].0 + d[1].0 as u64, d[1].3), (stg[k % 2].0 + d[2].0 as u64, d[2].3)];
                            let p = PlanExp { i: *i, w, abase: ab };
                            ab += exps[*i].tokens.len();
                            p
                        })
                        .collect()
                };
                let ultimo = k + 1 == m;
                let (acum, subir_x) = (escrita, !escrita);
                let r = std::thread::scope(|sco| {
                    // la copia del trozo k+2 al anclado del trozo k (ya en el dispositivo) corre mientras se calcula el k
                    let hilo = (k + 2 < m).then(|| {
                        let (trozo, dst) = (&trozos[k + 2].0, pin[k % 2].0);
                        sco.spawn(move || copiar(trozo, dst))
                    });
                    let t_c = std::time::Instant::now();
                    let r = self.correr_plan(xs, n, h, ffn, top_k, pesos, exps, &plan, suma, (subir_x, acum, ultimo));
                    t_calc += t_c.elapsed().as_secs_f64() * 1e3;
                    let t_j = std::time::Instant::now();
                    if let Some(hh) = hilo {
                        let _ = hh.join();
                    }
                    t_join += t_j.elapsed().as_secs_f64() * 1e3;
                    r
                });
                r?;
                if k + 1 < m {
                    let t_s = std::time::Instant::now();
                    sincro()?;
                    t_sync += t_s.elapsed().as_secs_f64() * 1e3;
                }
                escrita = true;
                for pl in &plan {
                    hecho[pl.i] = true;
                }
                total_bytes += trozos[k].1;
            }
            if std::env::var_os("SOSO_TRAZA_GPU").is_some() {
                eprintln!("[gpu] pipeline: prólogo {t_prologo:.1} (copia0 {t_copia0:.1}, dma0 {t_dma0:.1}), cálculo {t_calc:.1}, espera copia {t_join:.1}, espera dma {t_sync:.1} ms");
            }
        } else {
            // sin grafos/anclada: camino síncrono desde memoria paginable
            for k in 0..m {
                let b = self.stg2[0].0;
                for (_, d) in &trozos[k].0 {
                    for x in d {
                        let src = unsafe { std::slice::from_raw_parts(x.1 as *const u8, x.2) };
                        self.subir(b + x.0 as u64, src)?;
                    }
                }
                let plan: Vec<PlanExp> = {
                    let mut ab = 0usize;
                    trozos[k]
                        .0
                        .iter()
                        .map(|(i, d)| {
                            let w = [(b + d[0].0 as u64, d[0].3), (b + d[1].0 as u64, d[1].3), (b + d[2].0 as u64, d[2].3)];
                            let p = PlanExp { i: *i, w, abase: ab };
                            ab += exps[*i].tokens.len();
                            p
                        })
                        .collect()
                };
                self.correr_plan(xs, n, h, ffn, top_k, pesos, exps, &plan, suma, (!escrita, escrita, k + 1 == m))?;
                escrita = true;
                for pl in &plan {
                    hecho[pl.i] = true;
                }
                total_bytes += trozos[k].1;
            }
        }
        if std::env::var_os("SOSO_TRAZA_GPU").is_some() {
            let dt = t_sub.elapsed().as_secs_f64();
            eprintln!("[gpu] de paso: {} exps, {:.0} MiB en {:.1} ms (con cálculo, {} trozos)", stream.len(), total_bytes as f64 / 1048576.0, dt * 1e3, m);
        }
        self.stats.bytes_up += total_bytes as u64;
        Ok(())
    }

    /// Lanza gate/up/down y la suma ponderada para los expertos de `plan` (pesos ya en VRAM) y deja en `suma` (`n × h`) su contribución.
    #[allow(clippy::too_many_arguments)]
    fn correr_plan(
        &mut self,
        xs: &[f32],
        n: usize,
        h: usize,
        ffn: usize,
        top_k: usize,
        pesos: &[f32],
        exps: &[soso_llm_core::gpu::ExpertoBloque<'_>],
        plan: &[PlanExp],
        suma: &mut [f32],
        // (subir `xs`, acumular sobre la suma que ya hay en el dispositivo, bajar la suma al terminar)
        (subir_x, acum, bajar): (bool, bool, bool),
    ) -> Result<(), String> {
        let t_a = std::time::Instant::now();
        let total: usize = plan.iter().map(|p| exps[p.i].tokens.len()).sum();
        let t_b = std::time::Instant::now();
        // 2) datos del bloque a la tarjeta
        let x_dev = self.blq_asegurar(0, xs.len() * 4)?;
        let tok_dev = self.blq_asegurar(1, total * 4)?;
        let act_dev = self.blq_asegurar(2, total * ffn * 4)?;
        let y_dev = self.blq_asegurar(3, n * top_k * h * 4)?;
        let dest_dev = self.blq_asegurar(5, total * 4)?;
        let wd_dev = self.blq_asegurar(6, n * top_k * 4)?;
        let out_dev = self.blq_asegurar(7, n * h * 4)?;
        if subir_x {
            self.subir(x_dev, unsafe { std::slice::from_raw_parts(xs.as_ptr() as *const u8, xs.len() * 4) })?;
        }
        let mut toks: Vec<i32> = Vec::with_capacity(total);
        for pl in plan {
            toks.extend(exps[pl.i].tokens.iter().map(|&t| t as i32));
        }
        self.subir(tok_dev, unsafe { std::slice::from_raw_parts(toks.as_ptr() as *const u8, toks.len() * 4) })?;
        let mut dests: Vec<i32> = Vec::with_capacity(total);
        for pl in plan {
            dests.extend(exps[pl.i].destinos.iter().map(|&d| d as i32));
        }
        self.subir(dest_dev, unsafe { std::slice::from_raw_parts(dests.as_ptr() as *const u8, dests.len() * 4) })?;
        let mut wd = vec![0f32; n * top_k];
        for &d in &dests {
            wd[d as usize] = pesos[d as usize];
        }
        self.subir(wd_dev, unsafe { std::slice::from_raw_parts(wd.as_ptr() as *const u8, wd.len() * 4) })?;
        let t_c = std::time::Instant::now();
        // 3) un descriptor por experto y DOS lanzamientos para todos (gate+up+SwiGLU y down), una sola espera
        let mut desc: Vec<u64> = Vec::with_capacity(plan.len() * 12);
        let mut max_cnt = 0usize;
        for pl in plan {
            let cnt = exps[pl.i].tokens.len();
            max_cnt = max_cnt.max(cnt);
            let (dg, du, dd) = (pl.w[0].1, pl.w[1].1, pl.w[2].1);
            let cm = |d: u8, c: usize| codigo_mma(d, c).unwrap_or((0, 0));
            let (cg, cu, cd) = (cm(dg, h), cm(du, h), cm(dd, ffn));
            desc.extend_from_slice(&[
                pl.w[0].0, pl.w[1].0, pl.w[2].0, pl.abase as u64, cnt as u64, cg.0 as u64, cu.0 as u64, cd.0 as u64, cg.1 as u64, cu.1 as u64, cd.1 as u64, pl.abase as u64,
            ]);
        }
        let d_dev = self.blq_asegurar(4, desc.len() * 8)?;
        self.subir(d_dev, unsafe { std::slice::from_raw_parts(desc.as_ptr() as *const u8, desc.len() * 8) })?;
        let tiles = (max_cnt as u32).div_ceil(64);
        let nexp = plan.len() as u32;
        let (mut a0, mut a1, mut a2, mut a3) = (d_dev, x_dev, tok_dev, act_dev);
        let (mut a4, mut a5) = (ffn as c_int, h as c_int);
        let mut args = [
            &mut a0 as *mut _ as *mut c_void,
            &mut a1 as *mut _ as *mut c_void,
            &mut a2 as *mut _ as *mut c_void,
            &mut a3 as *mut _ as *mut c_void,
            &mut a4 as *mut _ as *mut c_void,
            &mut a5 as *mut _ as *mut c_void,
        ];
        self.lanzar3s(self.f_gateup_multi, (((ffn as u32) + 63) / 64, tiles, nexp), (128, 1, 1), 0, &mut args)?;
        let (mut b0, mut b1, mut b2) = (d_dev, act_dev, y_dev);
        let (mut b3, mut b4, mut b5) = (h as c_int, ffn as c_int, dest_dev);
        let mut args = [
            &mut b0 as *mut _ as *mut c_void,
            &mut b1 as *mut _ as *mut c_void,
            &mut b2 as *mut _ as *mut c_void,
            &mut b3 as *mut _ as *mut c_void,
            &mut b4 as *mut _ as *mut c_void,
            &mut b5 as *mut _ as *mut c_void,
        ];
        self.lanzar3s(self.f_down_multi, (((h as u32) + 63) / 64, tiles, nexp), (128, 1, 1), 0, &mut args)?;
        let (mut c0, mut c1, mut c2, mut c3, mut c4, mut c5) = (y_dev, wd_dev, out_dev, top_k as c_int, h as c_int, acum as c_int);
        let mut args = [
            &mut c0 as *mut _ as *mut c_void,
            &mut c1 as *mut _ as *mut c_void,
            &mut c2 as *mut _ as *mut c_void,
            &mut c3 as *mut _ as *mut c_void,
            &mut c4 as *mut _ as *mut c_void,
            &mut c5 as *mut _ as *mut c_void,
        ];
        self.lanzar3s(self.f_combinar, ((h as u32).div_ceil(256), n as u32, 1), (256, 1, 1), 0, &mut args)?;
        let t_d = std::time::Instant::now();
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        let t_e = std::time::Instant::now();
        if bajar {
            ck(unsafe { (self.api.dtoh)(suma.as_mut_ptr() as *mut c_void, out_dev, n * h * 4) }, "cuMemcpyDtoH")?;
        }
        if std::env::var_os("SOSO_TRAZA_GPU").is_some() {
            let t_f = std::time::Instant::now();
            eprintln!(
                "[gpu] expertos: {} exps, {} asignaciones | plan {:.1} ms, subida {:.1}, lanzar {:.1}, kernels {:.1}, bajada+copia {:.1}",
                plan.len(), total,
                (t_b - t_a).as_secs_f64() * 1e3, (t_c - t_b).as_secs_f64() * 1e3, (t_d - t_c).as_secs_f64() * 1e3,
                (t_e - t_d).as_secs_f64() * 1e3, (t_f - t_e).as_secs_f64() * 1e3
            );
        }
        self.stats.launches += 1;
        self.stats.bytes_up += (xs.len() * 4 + toks.len() * 4) as u64;
        self.stats.bytes_down += (total * h * 4) as u64;
        Ok(())
    }

    /// Varios matvec de un vector con **un solo** envío de activaciones, lanzamientos seguidos y una sola espera y
    /// descarga. Marca en `hecho` los que ha resuelto (los demás —dtype sin kernel, sin VRAM— los hace quien llama).
    fn lote(&mut self, ops: &mut [soso_llm_core::gpu::MatvecOp<'_>], hecho: &mut [bool]) -> Result<(), String> {
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        struct Plan {
            i: usize,
            dtype: u8,
            pd: u64,
            pq: u64,
            x_off: usize,
            y_off: usize,
        }
        let mut plan: Vec<Plan> = Vec::new();
        let mut xs: Vec<(usize, usize, usize)> = Vec::new(); // (ptr, len, offset en floats)
        let (mut x_tot, mut y_tot) = (0usize, 0usize);
        for (i, op) in ops.iter().enumerate() {
            if op.x.len() != op.cols || op.out.len() != op.rows {
                continue;
            }
            if !self.pesos.contains_key(op.key) {
                self.subiendo_experto = es_clave_experto(op.key);
                let p = self.subir_peso(op.view, op.rows, op.cols);
                self.subiendo_experto = false;
                let p = p?;
                self.pesos.insert(op.key.to_string(), p);
            }
            let Some(Some(p)) = self.pesos.get(op.key) else { continue };
            if p.rows != op.rows || p.cols != op.cols || p.dtype != op.view.dtype || !(es_iq_gpu(p.dtype) || p.dtype == DTYPE_Q8_0 || p.dtype == DTYPE_Q4_K) {
                continue;
            }
            let key_x = (op.x.as_ptr() as usize, op.x.len());
            let x_off = match xs.iter().find(|e| (e.0, e.1) == key_x) {
                Some(e) => e.2,
                None => {
                    xs.push((key_x.0, key_x.1, x_tot));
                    x_tot += op.cols;
                    x_tot - op.cols
                }
            };
            plan.push(Plan { i, dtype: p.dtype, pd: p.d, pq: p.qs, x_off, y_off: y_tot });
            y_tot += op.rows;
        }
        if plan.is_empty() {
            return Ok(());
        }
        self.asegurar_buffers(x_tot, y_tot)?;
        for (ptr, len, off) in &xs {
            let bytes = unsafe { std::slice::from_raw_parts(*ptr as *const u8, len * 4) };
            self.subir(self.x_dev + (*off * 4) as u64, bytes)?;
        }
        let mut lanzados: Vec<usize> = Vec::new();
        for pl in &plan {
            let op = &ops[pl.i];
            let (xd, yd) = (self.x_dev + (pl.x_off * 4) as u64, self.y_dev + (pl.y_off * 4) as u64);
            if self.lanzar_peso(pl.dtype, pl.pd, pl.pq, op.rows, op.cols, 1, xd, yd)? {
                lanzados.push(pl.i);
            }
        }
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        let mut host = vec![0f32; y_tot];
        ck(unsafe { (self.api.dtoh)(host.as_mut_ptr() as *mut c_void, self.y_dev, y_tot * 4) }, "cuMemcpyDtoH")?;
        for pl in &plan {
            if lanzados.contains(&pl.i) {
                let op = &mut ops[pl.i];
                op.out.copy_from_slice(&host[pl.y_off..pl.y_off + op.rows]);
                hecho[pl.i] = true;
            }
        }
        self.stats.launches += 1;
        self.stats.bytes_up += (x_tot * 4) as u64;
        self.stats.bytes_down += (y_tot * 4) as u64;
        Ok(())
    }

    /// Producto por bloque con los pesos subidos sólo para esta llamada (no residentes).
    fn calcular_de_paso(&mut self, view: &TensorView<'_>, rows: usize, cols: usize, n: usize, x: &[f32], out: &mut [f32]) -> Result<bool, String> {
        let (dtype_k, rb) = if view.dtype == DTYPE_Q8_0 {
            (1000i32, ((cols / 32) * 36) as i64)
        } else if view.dtype == DTYPE_Q4_K {
            (1002i32, ((cols / 256) * 144) as i64)
        } else {
            let Some(rb) = sosomodel::dequant::row_bytes(view.dtype, cols) else { return Ok(false) };
            (sosomodel::layout::ggml_de_dtype_iq(view.dtype).unwrap_or(0) as i32, rb as i64)
        };
        if view.bytes.len() as i64 != rows as i64 * rb {
            return Ok(false);
        }
        let t_ini = std::time::Instant::now();
        let w_dev = self.gbuf_asegurar(16, view.bytes.len())?;
        self.subir(w_dev, view.bytes)?;
        self.asegurar_buffers(n * cols, n * rows)?;
        let x_bytes = unsafe { std::slice::from_raw_parts(x.as_ptr() as *const u8, x.len() * 4) };
        self.subir(self.x_dev, x_bytes)?;
        self.lanzar_a(
            self.f_iq_matmul_n,
            ((rows as u32).div_ceil(64), (n as u32).div_ceil(64), 1),
            (128, 1, 1),
            &[Arg::P(w_dev), Arg::P(self.x_dev), Arg::P(self.y_dev), Arg::I(rows as i32), Arg::I(cols as i32), Arg::I(n as i32), Arg::I(dtype_k), Arg::L(rb)],
        )?;
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        let out_bytes = unsafe { std::slice::from_raw_parts_mut(out.as_mut_ptr() as *mut u8, out.len() * 4) };
        ck(unsafe { (self.api.dtoh)(out_bytes.as_mut_ptr() as *mut c_void, self.y_dev, out_bytes.len()) }, "cuMemcpyDtoH")?;
        if std::env::var_os("SOSO_TRAZA_GPU").is_some() {
            eprintln!("[gpu] de paso: {}x{} dtype {} n={} → {:.1} ms", rows, cols, view.dtype, n, t_ini.elapsed().as_secs_f64() * 1e3);
        }
        self.stats.launches += 1;
        Ok(true)
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
            self.subiendo_experto = es_clave_experto(key);
            let p = self.subir_peso(view, rows, cols);
            self.subiendo_experto = false;
            let p = p?;
            self.pesos.insert(key.to_string(), p);
        }
        let Some(Some(p)) = self.pesos.get(key) else {
            if std::env::var_os("SOSO_TRAZA_GPU").is_some() {
                eprintln!("[gpu] no residente: {key} dtype {} {}x{} n={n}", view.dtype, rows, cols);
            }
            // Sin sitio en VRAM: con varios tokens por lectura (prefill por bloques) compensa subir los pesos de paso y usar los tensor cores
            // (una matriz de la FFN del 27B son ~34 MB: 6 ms de bus frente a cientos de ms en CPU).
            if n >= 16 && (es_iq_gpu(view.dtype) || view.dtype == DTYPE_Q8_0 || view.dtype == DTYPE_Q4_K) && cols % 256 == 0 {
                return self.calcular_de_paso(view, rows, cols, n, x, out);
            }
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
        let (x_dev, y_dev) = (self.x_dev, self.y_dev);
        if !self.lanzar_peso(dtype, pd, pq, rows, cols, n, x_dev, y_dev)? {
            return Ok(false);
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

    fn router_topk(&mut self, key: &str, w: &[f32], xs: &[f32], n: usize, h: usize, n_exp: usize, top_k: usize, idx: &mut [i32], pesos: &mut [f32]) -> Result<bool, ()> {
        self.router_topk_gpu(key, w, xs, n, h, n_exp, top_k, idx, pesos).map_err(|e| eprintln!("router GPU: {e}"))
    }

    fn bloque_prefill_moe(&self) -> Option<usize> {
        self.flujo_expertos.then_some(soso_llm_core::runtime::Runtime::BLOQUE_PREFILL_STREAM)
    }

    fn decode_preparar(&mut self, plan: &soso_llm_core::gpu::DecodePlan) -> Result<bool, ()> {
        self.decode_preparar_impl(plan).map_err(|e| eprintln!("decode GPU: {e}"))
    }

    fn decode_listo(&self) -> bool {
        self.dec.is_some()
    }

    fn decode_subir_estado(&mut self, capas: &mut [soso_llm_core::gpu::EstadoCapaDec<'_>], pos: usize) -> Result<(), ()> {
        self.decode_subir_estado_impl(capas, pos).map_err(|e| eprintln!("decode GPU: {e}"))
    }

    fn decode_token(
        &mut self,
        embed: &[f32],
        pos: usize,
        cpu_moe: &mut dyn FnMut(usize, &[f32], &[u32], &[f32], &mut [f32]) -> Result<(), ()>,
        logits: &mut [f32],
        kv_rows: &mut [u16],
    ) -> Result<bool, ()> {
        self.decode_token_impl(embed, pos, cpu_moe, logits, kv_rows).map_err(|e| eprintln!("decode GPU: {e}"))
    }

    fn decode_bajar_estado(&mut self, capas: &mut [soso_llm_core::gpu::EstadoCapaDec<'_>]) -> Result<(), ()> {
        self.decode_bajar_estado_impl(capas).map_err(|e| eprintln!("decode GPU: {e}"))
    }

    fn stats(&self) -> GpuStats {
        self.stats
    }

    fn dtype_soportado(&self, dtype: u8) -> bool {
        matches!(dtype, DTYPE_Q4_0 | DTYPE_Q8_0 | DTYPE_Q4_K) || es_iq_gpu(dtype)
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

    fn gdn_capa(
        &mut self,
        capa: &soso_llm_core::gpu::GdnCapa<'_>,
        xs: &[f32],
        n: usize,
        h: usize,
        conv_estado: &mut [f32],
        estado: &mut [f32],
        salida: &mut [f32],
    ) -> Result<bool, ()> {
        match self.gdn_capa_gpu(capa, xs, n, h, conv_estado, estado, salida) {
            Ok(b) => Ok(b),
            Err(e) => {
                if !self.avisado {
                    eprintln!("cuda: {e}; sigo en CPU");
                    self.avisado = true;
                }
                Err(())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn gdn_bloque(
        &mut self,
        hist: &[f32],
        conv: &[f32],
        z: &[f32],
        decay: &[f32],
        beta: &[f32],
        norma: &[f32],
        estado: &mut [f32],
        salida: &mut [f32],
        n: usize,
        n_k: usize,
        n_v: usize,
        d: usize,
        kernel: usize,
        eps: f32,
    ) -> Result<bool, ()> {
        match self.gdn_bloque_gpu(hist, conv, z, decay, beta, norma, estado, salida, n, n_k, n_v, d, kernel, eps) {
            Ok(b) => Ok(b),
            Err(e) => {
                if !self.avisado {
                    eprintln!("cuda: {e}; sigo en CPU");
                    self.avisado = true;
                }
                Err(())
            }
        }
    }

    fn atencion_prefill(
        &mut self,
        capa: u32,
        q: &[f32],
        out: &mut [f32],
        n: usize,
        seq_antes: usize,
        heads: usize,
        kv_heads: usize,
        hd: usize,
        k: &[u16],
        v: &[u16],
        filas_validas: &mut usize,
    ) -> Result<bool, ()> {
        match self.atencion_prefill_gpu(capa, q, out, n, seq_antes, heads, kv_heads, hd, k, v, filas_validas) {
            Ok(b) => Ok(b),
            Err(e) => {
                if !self.avisado {
                    eprintln!("cuda: {e}; sigo en CPU");
                    self.avisado = true;
                }
                Err(())
            }
        }
    }

    fn expertos_bloque(
        &mut self,
        xs: &[f32],
        n: usize,
        h: usize,
        ffn: usize,
        top_k: usize,
        pesos: &[f32],
        exps: &mut [soso_llm_core::gpu::ExpertoBloque<'_>],
        suma: &mut [f32],
        hecho: &mut [bool],
    ) -> Result<(), ()> {
        match self.expertos_bloque_gpu(xs, n, h, ffn, top_k, pesos, exps, suma, hecho) {
            Ok(()) => Ok(()),
            Err(e) => {
                if !self.avisado {
                    eprintln!("cuda: {e}; sigo en CPU");
                    self.avisado = true;
                }
                Err(())
            }
        }
    }

    fn matvec_lote(&mut self, ops: &mut [soso_llm_core::gpu::MatvecOp<'_>], hecho: &mut [bool]) -> Result<(), ()> {
        match self.lote(ops, hecho) {
            Ok(()) => Ok(()),
            Err(e) => {
                if !self.avisado {
                    eprintln!("cuda: {e}; sigo en CPU");
                    self.avisado = true;
                }
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
