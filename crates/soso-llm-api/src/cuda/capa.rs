//! Capa de Gemma 4 entera en la GPU (G9): normas, proyecciones, RoPE, KV residente, atención, GeGLU y
//! residuales sin volver a la CPU entre operaciones. Por llamada sólo viajan el `hidden` (entrada y salida) y las
//! normas, que son pequeñas.
//!
//! El KV de cada capa vive en el dispositivo y sigue las **mismas reglas** que en CPU (`soso-llm-core/src/gemma.rs`):
//! las capas locales conservan su ventana más una holgura y expulsan por delante al pasar de `ventana + 2048`
//! filas; las globales guardan todo. Para que la reutilización de prefijo siga funcionando sin que el runtime
//! hable con el dispositivo, el KV **rebobina** cuando una llamada trae un `pos0` menor que las posiciones que
//! guarda (ver [`GemmaCapa`]).

use std::collections::{HashMap, HashSet};
use std::ffi::{CString, c_int, c_void};

use soso_llm_core::gemma::{EXPULSION_DISPARO, EXPULSION_HOLGURA};
use soso_llm_core::gpu::GemmaCapa;

use super::{Api, CuFunction, CuModule, CudaGpu, NT, Peso, ck};
use sosomodel::layout::{DTYPE_Q4_0, DTYPE_Q4_K, DTYPE_Q8_0};

const PTX_CAPA: &str = include_str!("../../cuda/gemma_capa.ptx");

/// Parámetros dinámicos por capa (pos0, seq_antes, …) en el dispositivo: 16 bytes por capa, hasta 256 capas, y un anillo
/// de ranuras ancladas desde el que se copian en asíncrono.
pub(super) const DYN_BYTES: usize = 16;
pub(super) const DYN_CAPAS: usize = 256;
pub(super) const DYN_SLOTS: usize = 256;

/// Posiciones por trozo de la atención por trozos de KV (debe coincidir con `ATT_CH` del kernel).
pub(super) const ATT_CH: usize = 256;
/// Posiciones por trozo de la atención por trozos para Gemma (debe coincidir con `GA_CH` del kernel).
pub(super) const GA_CH: usize = 128;

/// Con al menos tantos tokens el matmul de Q4_0 usa los tensor cores (a menos de 16 no compensa llenar el CTA).
pub(super) const GEMM_TC_MIN_TOKENS: usize = 16;

/// Tokens por pasada de los kernels agrupados (`TT` en el .cu) y relleno de la `x` en shared (`XPAD`).
pub(super) const GRP_TT: usize = 4;
pub(super) const XPAD_RUST: usize = 36;

/// Tokens máximos por llamada (los bloques del prefill son de 8).
pub(super) const N_MAX: usize = 256;

macro_rules! a {
    ($v:expr) => {
        &mut $v as *mut _ as *mut c_void
    };
}

pub(super) struct KvDev {
    dk: u64,
    dv: u64,
    /// Filas que caben.
    cap: usize,
    kv_dim: usize,
    /// Filas guardadas ahora mismo.
    stored: usize,
    /// Filas expulsadas por delante (posición absoluta de la fila 0).
    dropped: usize,
}

mod moe;

struct NormasDev {
    attn: u64,
    q: u64,
    k: u64,
    post_attn: u64,
    ffn: u64,
    post_ffn: u64,
    rope: u64,
}

pub(super) struct Fns {
    pub(super) rmsnorm: CuFunction,
    rope: CuFunction,
    kv_append: CuFunction,
    scores: CuFunction,
    softmax: CuFunction,
    pv: CuFunction,
    reduce: CuFunction,
    geglu: CuFunction,
    add_scale: CuFunction,
    pub(super) mo_router_logits: CuFunction,
    pub(super) mo_router_topk: CuFunction,
    pub(super) q4k: CuFunction,
    pub(super) q80: CuFunction,
    pub(super) q4k_x3: CuFunction,
    pub(super) att_split: CuFunction,
    pub(super) ga_256_2: CuFunction,
    pub(super) ga_512_16: CuFunction,
    pub(super) ga_combine: CuFunction,
    pub(super) grp: CuFunction,
    pub(super) gateup_grp: CuFunction,
    pub(super) down_grp: CuFunction,
    pub(super) down_q4k: CuFunction,
    pub(super) down_grp_q4k: CuFunction,
    pub(super) mm4n: CuFunction,
    pub(super) mm8n: CuFunction,
    pub(super) att_combine: CuFunction,
    pub(super) mo_gateup: CuFunction,
    pub(super) mo_down: CuFunction,
    pub(super) mo_combine: CuFunction,
}

/// Búfer de trabajo que sólo crece.
#[derive(Default, Clone, Copy)]
struct Buf {
    ptr: u64,
    cap: usize,
}

pub(super) const B_HS: usize = 0;
pub(super) const B_XN: usize = 1;
pub(super) const B_Q: usize = 2;
pub(super) const B_K: usize = 3;
pub(super) const B_V: usize = 4;
pub(super) const B_ATTN: usize = 5;
pub(super) const B_O: usize = 6;
const B_UP: usize = 7;
const B_GATE: usize = 8;
pub(super) const B_SC: usize = 9;
pub(super) const B_PARCIAL: usize = 10;
pub(super) const B_TMP: usize = 11;
pub(super) const B_IDX: usize = 12;
pub(super) const B_WGT: usize = 13;
pub(super) const B_ACT: usize = 14;
pub(super) const B_YE: usize = 15;
pub(super) const B_LG: usize = 16;
pub(super) const B_ML: usize = 17;
pub(super) const B_GS: usize = 18;
pub(super) const B_GO: usize = 19;
const N_BUFS: usize = 20;

pub(super) struct Estado {
    /// Diagnóstico: si está activo, cada llamada guarda aquí copias de las activaciones tras cada etapa.
    pub(super) depurar: bool,
    /// Diagnóstico de tiempos (`SOSO_CUDA_PERFIL=1`): milisegundos de GPU acumulados por etapa.
    pub(super) perfil: Option<(*mut c_void, *mut c_void)>,
    pub(super) perfil_ms: Vec<(&'static str, f64)>,
    marcas: Vec<&'static str>,
    evpool: Vec<*mut c_void>,
    pub(super) volcado: Vec<(&'static str, Vec<f32>)>,
    pub(super) fns: Fns,
    moe: HashMap<u32, moe::MoeDev>,
    /// Grafos instanciados por (capa, n, tramo de contexto) y la generación de búferes con la que se capturaron.
    grafos: HashMap<(u32, usize, usize), *mut c_void>,
    grafos_gen: u64,
    gen_bufs: u64,
    /// Parámetros dinámicos por capa en el dispositivo (`DYN_BYTES` por capa) y su ensayo anclado en el host.
    dyn_dev: u64,
    pinned: *mut c_void,
    slot: usize,
    /// ¿Se permiten los tensor cores? (`SOSO_CUDA_SIN_TC=1` los apaga, para comparar.)
    tensor_cores: bool,
    /// Plan de VRAM de un modelo MoE que no cabe entero (ver `moe::planificar_moe`), modo de cada capa y pool de expertos.
    plan_moe: Option<moe::PlanGpu>,
    modos_moe: HashMap<u32, soso_llm_core::gpu::ModoMoe>,
    pool: Option<moe::Pool>,
    /// Diagnóstico del flujo: ns de CPU en traer expertos, bytes copiados y ns esperando al router.
    pub(super) flujo_diag: [u64; 3],
    /// Salida de capa MoE aún sin descargar: (capa, pos0, n). El `hidden` vive en el búfer `B_HS`.
    pendiente: Option<(u32, usize, usize)>,
    pub(super) kvs: HashMap<u32, KvDev>,
    normas: HashMap<u32, NormasDev>,
    pub(super) rechazadas: HashSet<u32>,
    bufs: [Buf; N_BUFS],
}

impl Estado {
    pub(super) fn rechazar(&mut self, layer: u32) {
        self.rechazadas.insert(layer);
    }

    pub(super) fn nueva(api: &Api) -> Result<Estado, String> {
        unsafe {
            let c = CString::new(PTX_CAPA).map_err(|_| "PTX con NUL".to_string())?;
            let mut m: CuModule = std::ptr::null_mut();
            ck(
                (api.module_load)(&mut m, c.as_ptr() as *const c_void),
                "cuModuleLoadData(gemma_capa)",
            )?;
            let f = |n: &str| -> Result<CuFunction, String> {
                let c = CString::new(n).unwrap();
                let mut f: CuFunction = std::ptr::null_mut();
                ck((api.module_fn)(&mut f, m, c.as_ptr()), n)?;
                Ok(f)
            };
            let perfil = if std::env::var_os("SOSO_CUDA_PERFIL").is_some() {
                let (mut a, mut b): (*mut c_void, *mut c_void) =
                    (std::ptr::null_mut(), std::ptr::null_mut());
                ck((api.ev_create)(&mut a, 0), "cuEventCreate")?;
                ck((api.ev_create)(&mut b, 0), "cuEventCreate")?;
                Some((a, b))
            } else {
                None
            };
            Ok(Estado {
                perfil,
                perfil_ms: Vec::new(),
                marcas: Vec::new(),
                evpool: Vec::new(),
                depurar: false,
                volcado: Vec::new(),
                fns: Fns {
                    rmsnorm: f("gm_rmsnorm_rows")?,
                    rope: f("gm_rope")?,
                    kv_append: f("gm_kv_append")?,
                    scores: f("gm_attn_scores")?,
                    softmax: f("gm_attn_softmax")?,
                    pv: f("gm_attn_pv")?,
                    reduce: f("gm_attn_reduce")?,
                    geglu: f("gm_geglu")?,
                    add_scale: f("gm_add_scale")?,
                    mo_router_logits: f("mo_router_logits")?,
                    mo_router_topk: f("mo_router_topk")?,
                    q4k: f("gm_matvec_q4k")?,
                    q80: f("gm_matvec_q80")?,
                    q4k_x3: f("gm_matvec_q4k_x3")?,
                    att_split: f("mo_attn_split")?,
                    ga_256_2: f("ga_split_256_2")?,
                    ga_512_16: f("ga_split_512_16")?,
                    ga_combine: f("ga_combine")?,
                    grp: f("mo_group")?,
                    gateup_grp: f("mo_gateup_grp")?,
                    down_grp: f("mo_down_grp")?,
                    down_q4k: f("mo_down_q4k")?,
                    down_grp_q4k: f("mo_down_grp_q4k")?,
                    mm4n: f("gm_matmul_q4k_n")?,
                    mm8n: f("gm_matmul_q80_n")?,
                    att_combine: f("mo_attn_combine")?,
                    mo_gateup: f("mo_gateup_q4k")?,
                    mo_down: f("mo_down_q80")?,
                    mo_combine: f("mo_combine")?,
                },
                moe: HashMap::new(),
                pendiente: None,
                plan_moe: None,
                modos_moe: HashMap::new(),
                pool: None,
                flujo_diag: [0; 3],
                tensor_cores: std::env::var_os("SOSO_CUDA_SIN_TC").is_none(),
                grafos: HashMap::new(),
                grafos_gen: 0,
                gen_bufs: 0,
                dyn_dev: 0,
                pinned: std::ptr::null_mut(),
                slot: 0,
                kvs: HashMap::new(),
                normas: HashMap::new(),
                rechazadas: HashSet::new(),
                bufs: [Buf::default(); N_BUFS],
            })
        }
    }
}

type Dim3 = (u32, u32, u32);

impl CudaGpu {
    pub(super) fn capa_puede(&self, layer: u32) -> bool {
        !self.capa.rechazadas.contains(&layer)
    }

    /// Como [`Self::lanzar3`] con memoria compartida dinámica (`smem` bytes).
    pub(super) fn lanzar3s(
        &self,
        f: CuFunction,
        grid: Dim3,
        block: Dim3,
        smem: u32,
        args: &mut [*mut c_void],
    ) -> Result<(), String> {
        if !self.saltar.is_empty() && self.saltar.contains(&(f as usize)) {
            return Ok(());
        }
        ck(
            unsafe {
                (self.api.launch)(
                    f,
                    grid.0,
                    grid.1,
                    grid.2,
                    block.0,
                    block.1,
                    block.2,
                    smem,
                    self.stream_lanzar(),
                    args.as_mut_ptr(),
                    std::ptr::null_mut(),
                )
            },
            "cuLaunchKernel",
        )
    }

    /// `Y[n×rows] = X[n×cols] · W^T` para `n` tokens leyendo los pesos una vez por varios tokens (Q4_K y Q8_0); con
    /// `n == 1` es el matvec de siempre.
    pub(super) fn mm_n(&mut self, key: &str, x: u64, y: u64, n: usize) -> Result<(), String> {
        if n < 2 {
            return self.mm_dev(key, x, y, n);
        }
        let (dtype, pd, rows, cols) = match self.pesos.get(key) {
            Some(Some(p)) => (p.dtype, p.d, p.rows, p.cols),
            _ => return Err(format!("tensor {key} no residente")),
        };
        let (r32, c32, n32) = (rows as c_int, cols as c_int, n as c_int);
        match dtype {
            DTYPE_Q4_K => {
                let xt = cols / 32 * XPAD_RUST;
                let tc = (49152 / (xt * 4)).min(GRP_TT);
                if tc == 0 {
                    return self.mm_dev(key, x, y, n);
                }
                let (mut a0, mut a1, mut a2, mut a3, mut a4, mut a5, mut a6) =
                    (pd, x, y, r32, c32, n32, tc as c_int);
                self.lanzar3s(
                    self.capa.fns.mm4n,
                    ((rows as u32).div_ceil(16), 1, 1),
                    (256, 1, 1),
                    (tc * xt * 4) as u32,
                    &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4), a!(a5), a!(a6)],
                )
            }
            DTYPE_Q8_0 => {
                let (mut a0, mut a1, mut a2, mut a3, mut a4, mut a5) = (pd, x, y, r32, c32, n32);
                self.lanzar3(
                    self.capa.fns.mm8n,
                    ((rows as u32).div_ceil(32), 1, 1),
                    (256, 1, 1),
                    &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4), a!(a5)],
                )
            }
            _ => self.mm_dev(key, x, y, n),
        }
    }

    pub(super) fn lanzar3(
        &self,
        f: CuFunction,
        grid: Dim3,
        block: Dim3,
        args: &mut [*mut c_void],
    ) -> Result<(), String> {
        ck(
            unsafe {
                (self.api.launch)(
                    f,
                    grid.0,
                    grid.1,
                    grid.2,
                    block.0,
                    block.1,
                    block.2,
                    0,
                    self.stream_lanzar(),
                    args.as_mut_ptr(),
                    std::ptr::null_mut(),
                )
            },
            "cuLaunchKernel",
        )
    }

    /// Búfer de trabajo de al menos `floats` floats (4 bytes cada uno); devuelve el puntero de dispositivo.
    pub(super) fn buf(&mut self, quien: usize, floats: usize) -> Result<u64, String> {
        let b = self.capa.bufs[quien];
        if b.cap >= floats {
            return Ok(b.ptr);
        }
        if b.ptr != 0 {
            unsafe { (self.api.mem_free)(b.ptr) };
            self.usado -= b.cap * 4;
        }
        let ptr = self.alloc(floats * 4)?;
        self.usado += floats * 4;
        self.capa.bufs[quien] = Buf { ptr, cap: floats };
        self.capa.gen_bufs += 1;
        Ok(ptr)
    }

    pub(super) fn subir_f32(&self, dst: u64, v: &[f32]) -> Result<(), String> {
        let b = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4) };
        self.subir(dst, b)
    }

    pub(super) fn subir_norma(&mut self, v: &[f32]) -> Result<u64, String> {
        if v.is_empty() {
            return Ok(0);
        }
        let p = self.alloc(v.len() * 4)?;
        self.usado += v.len() * 4;
        self.subir_f32(p, v)?;
        Ok(p)
    }

    /// `Y[n×rows] = X[n×cols] · W^T` con `x` e `y` ya en el dispositivo.
    /// Q, K y V a la vez si las tres son Q4_K (un solo lanzamiento por token); si no, tres `mm_dev`.
    pub(super) fn mm_qkv(
        &mut self,
        kq: &str,
        kk: &str,
        kv: &str,
        x: u64,
        yq: u64,
        yk: u64,
        yv: u64,
        n: usize,
    ) -> Result<(), String> {
        let get = |k: &str| match self.pesos.get(k) {
            Some(Some(p)) if p.dtype == DTYPE_Q4_K => Some((p.d, p.rows, p.cols)),
            _ => None,
        };
        let (Some((w0, r0, c0)), Some((w1, r1, c1)), Some((w2, r2, c2))) =
            (get(kq), get(kk), get(kv))
        else {
            self.mm_dev(kq, x, yq, n)?;
            self.mm_dev(kk, x, yk, n)?;
            return self.mm_dev(kv, x, yv, n);
        };
        if c0 != c1 || c0 != c2 || r0 % 32 != 0 || r1 % 32 != 0 || r2 % 32 != 0 {
            self.mm_dev(kq, x, yq, n)?;
            self.mm_dev(kk, x, yk, n)?;
            return self.mm_dev(kv, x, yv, n);
        }
        let grid = ((r0 + r1 + r2) / 32) as u32;
        for t in 0..n {
            let (mut a0, mut a1, mut a2) = (w0, w1, w2);
            let mut a3 = x + (t * c0 * 4) as u64;
            let (mut a4, mut a5, mut a6) = (
                yq + (t * r0 * 4) as u64,
                yk + (t * r1 * 4) as u64,
                yv + (t * r2 * 4) as u64,
            );
            let (mut a7, mut a8, mut a9, mut a10) =
                (r0 as c_int, r1 as c_int, r2 as c_int, c0 as c_int);
            let mut args = [
                a!(a0),
                a!(a1),
                a!(a2),
                a!(a3),
                a!(a4),
                a!(a5),
                a!(a6),
                a!(a7),
                a!(a8),
                a!(a9),
                a!(a10),
            ];
            self.lanzar3(self.capa.fns.q4k_x3, (grid, 1, 1), (256, 1, 1), &mut args)?;
        }
        Ok(())
    }

    pub(super) fn mm_dev(&mut self, key: &str, x: u64, y: u64, n: usize) -> Result<(), String> {
        let (dtype, pd, pq, rows, cols) = match self.pesos.get(key) {
            Some(Some(p)) => (p.dtype, p.d, p.qs, p.rows, p.cols),
            _ => return Err(format!("tensor {key} no residente")),
        };
        let (r32, c32) = (rows as c_int, cols as c_int);
        match dtype {
            DTYPE_Q4_0 if n == 1 => {
                let (mut a0, mut a1, mut a2, mut a3, mut a4, mut a5) = (pd, pq, x, y, r32, c32);
                let mut args = [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4), a!(a5)];
                self.lanzar(self.f_matvec_q4, rows, args.to_vec())?;
                let _ = &mut args;
            }
            DTYPE_Q4_0 if n >= GEMM_TC_MIN_TOKENS && self.capa.tensor_cores => {
                // Tensor cores: 64 filas x 32 tokens por CTA, los pesos se leen una vez para los 32 tokens.
                let (mut a0, mut a1, mut a2, mut a3, mut a4, mut a5, mut a6) = (pd, pq, x, y, r32, c32, n as c_int);
                let mut args = [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4), a!(a5), a!(a6)];
                self.lanzar3(self.f_gemm_q4, ((rows as u32).div_ceil(64), (n as u32).div_ceil(32), 1), (128, 1, 1), &mut args)?;
            }
            DTYPE_Q4_0 => {
                let mut t0 = 0usize;
                while t0 < n {
                    let nt = (n - t0).min(NT) as c_int;
                    let (mut a0, mut a1) = (pd, pq);
                    let mut a2 = x + (t0 * cols * 4) as u64;
                    let mut a3 = y + (t0 * rows * 4) as u64;
                    let (mut a4, mut a5, mut a6) = (r32, c32, nt);
                    let args = vec![a!(a0), a!(a1), a!(a2), a!(a3), a!(a4), a!(a5), a!(a6)];
                    self.lanzar(self.f_matmul_q4, rows, args)?;
                    t0 += NT;
                }
            }
            DTYPE_Q4_K => {
                for t in 0..n {
                    let (mut a0, mut a2) = (pd, x + (t * cols * 4) as u64);
                    let mut a3 = y + (t * rows * 4) as u64;
                    let (mut a4, mut a5) = (r32, c32);
                    let args = vec![a!(a0), a!(a2), a!(a3), a!(a4), a!(a5)];
                    self.lanzar(self.capa.fns.q4k, rows, args)?;
                }
            }
            DTYPE_Q8_0 => {
                for t in 0..n {
                    let (mut a0, mut a2) = (pd, x + (t * cols * 4) as u64);
                    let mut a3 = y + (t * rows * 4) as u64;
                    let (mut a4, mut a5) = (r32, c32);
                    let args = vec![a!(a0), a!(a2), a!(a3), a!(a4), a!(a5)];
                    self.lanzar(self.capa.fns.q80, rows, args)?;
                }
            }
            _ => return Err("dtype no soportado".into()),
        }
        Ok(())
    }

    /// RoPE NeoX sobre `n` tokens de `nh` cabezas; `factores` (0 = ninguno) divide la frecuencia y `mscale` escala cos/sin.
    pub(super) fn rope_dev(
        &self,
        x: u64,
        nh: usize,
        hd: usize,
        n: usize,
        pos0: usize,
        theta: f32,
        factores: u64,
        mscale: f32,
        dyn_ptr: u64,
    ) -> Result<(), String> {
        let (mut a0, mut a1, mut a2, mut a3, mut a4, mut a5, mut a6, mut a7) = (
            x,
            nh as c_int,
            hd as c_int,
            pos0 as c_int,
            theta,
            factores,
            mscale,
            dyn_ptr,
        );
        self.lanzar3(
            self.capa.fns.rope,
            (nh as u32, n as u32, 1),
            ((hd / 2) as u32, 1, 1),
            &mut [
                a!(a0),
                a!(a1),
                a!(a2),
                a!(a3),
                a!(a4),
                a!(a5),
                a!(a6),
                a!(a7),
            ],
        )
    }

    pub(super) fn rms_rows(
        &self,
        x: u64,
        w: u64,
        y: u64,
        filas: usize,
        h: usize,
        eps: f32,
    ) -> Result<(), String> {
        let (mut a0, mut a1, mut a2, mut a3, mut a4) = (x, w, y, h as c_int, eps);
        let threads = if h >= 256 { 256 } else { 128 };
        self.lanzar3(
            self.capa.fns.rmsnorm,
            (filas as u32, 1, 1),
            (threads, 1, 1),
            &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4)],
        )
    }

    /// KV de la capa: lo crea si falta y lo rebobina según `pos0`. `None` = no cabe o no es coherente.
    fn kv_preparar(
        &mut self,
        c: &GemmaCapa<'_>,
    ) -> Result<Option<(u64, u64, usize, usize)>, String> {
        self.kv_preparar_en(
            c.layer,
            c.kv_heads * c.head_dim,
            c.ventana,
            c.ctx_max,
            c.pos0,
            c.n,
        )
    }

    /// Igual que [`Self::kv_preparar`] con los datos sueltos (lo comparte la capa MoE).
    pub(super) fn kv_preparar_en(
        &mut self,
        layer: u32,
        kv_dim: usize,
        ventana: usize,
        ctx_max: usize,
        pos0: usize,
        n: usize,
    ) -> Result<Option<(u64, u64, usize, usize)>, String> {
        let cap = if ventana > 0 {
            ventana + EXPULSION_DISPARO + N_MAX
        } else {
            ctx_max
        };
        if ventana == 0 && pos0 + n > cap {
            // Una capa global no puede crecer más allá de lo reservado; seguir en CPU leería un KV vacío.
            return Err(format!(
                "contexto de {} posiciones > {cap} reservadas en la capa {}",
                pos0 + n,
                layer
            ));
        }
        if !self.capa.kvs.contains_key(&layer) {
            let bytes = cap * kv_dim * 2;
            if self.usado + 2 * bytes > self.presupuesto {
                return Ok(None);
            }
            let dk = self.alloc(bytes)?;
            let dv = self.alloc(bytes)?;
            self.usado += 2 * bytes;
            self.capa.kvs.insert(
                layer,
                KvDev {
                    dk,
                    dv,
                    cap,
                    kv_dim,
                    stored: 0,
                    dropped: 0,
                },
            );
        }
        let kv = self.capa.kvs.get_mut(&layer).unwrap();
        let abs = kv.dropped + kv.stored;
        if pos0 < abs {
            if pos0 < kv.dropped {
                if pos0 == 0 {
                    kv.dropped = 0;
                    kv.stored = 0;
                } else {
                    return Err(format!(
                        "capa {}: se pide volver a la posición {} y sólo se conserva desde {}",
                        layer, pos0, kv.dropped
                    ));
                }
            } else {
                kv.stored = pos0 - kv.dropped;
            }
        } else if pos0 > abs {
            return Err(format!(
                "capa {}: la posición {} supera las {abs} que tiene el dispositivo",
                layer, pos0
            ));
        }
        // Expulsión por delante: misma regla que `expulsar_si_hace_falta` de la CPU.
        if ventana > 0 && kv.stored >= ventana + EXPULSION_DISPARO {
            let mover = kv.stored - (ventana + EXPULSION_HOLGURA);
            let quedan = kv.stored - mover;
            let (dk, dv, kvd) = (kv.dk, kv.dv, kv.kv_dim);
            kv.dropped += mover;
            kv.stored = quedan;
            let fila = (kvd * 2) as u64;
            let bytes = quedan * kvd * 2;
            // El origen y el destino se solapan: pasa por un búfer intermedio.
            let tmp = self.buf(B_TMP, bytes.div_ceil(4))?;
            for base in [dk, dv] {
                ck(
                    unsafe { (self.api.dtod)(tmp, base + mover as u64 * fila, bytes) },
                    "cuMemcpyDtoD",
                )?;
                ck(unsafe { (self.api.dtod)(base, tmp, bytes) }, "cuMemcpyDtoD")?;
            }
        }
        let kv = &self.capa.kvs[&layer];
        Ok(Some((kv.dk, kv.dv, kv.stored, kv.cap)))
    }

    /// Atención GQA por trozos de KV para formas generales (`ga_split_*` + `ga_combine`): `gp` cabezas por grupo (padding a potencia
    /// de dos), `s` trozos de `GA_CH` posiciones por (token, cabeza KV).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn atencion_ga_dev(&mut self, f: CuFunction, gp: usize, d_q: u64, dk: u64, dv: u64, d_par: u64, d_ml: u64, d_attn: u64, n: usize, heads: usize, kvh: usize, hd: usize, kv_dim: usize, seq_antes: usize, ventana: usize, escala: f32, s: usize) -> Result<(), String> {
        let (mut a0, mut a1, mut a2, mut a3, mut a4) = (d_q, dk, dv, d_par, d_ml);
        let (mut a5, mut a6, mut a7, mut a8, mut a9, mut a10, mut a11, mut a12) = (heads as c_int, kvh as c_int, kv_dim as c_int, seq_antes as c_int, ventana as c_int, escala, s as c_int, 0u64);
        self.lanzar3(f, (s as u32, kvh as u32, n as u32), (256, 1, 1), &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4), a!(a5), a!(a6), a!(a7), a!(a8), a!(a9), a!(a10), a!(a11), a!(a12)])?;
        self.marca("k attn.split")?;
        let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5, mut b6, mut b7) = (d_par, d_ml, d_attn, heads as c_int, kvh as c_int, hd as c_int, s as c_int, gp as c_int);
        self.lanzar3(self.capa.fns.ga_combine, (heads as u32, n as u32, 1), (hd as u32, 1, 1), &mut [a!(b0), a!(b1), a!(b2), a!(b3), a!(b4), a!(b5), a!(b6), a!(b7)])
    }

    /// Atención GQA por trozos de KV (`mo_attn_split` + `mo_attn_combine`): sólo para `hd == 128` y a lo sumo 8 cabezas
    /// de consulta por cabeza KV. `s` = trozos por (token, cabeza KV), de `ATT_CH` posiciones cada uno.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn atencion_split_dev(
        &mut self,
        d_q: u64,
        dk: u64,
        dv: u64,
        d_par: u64,
        d_ml: u64,
        d_attn: u64,
        n: usize,
        heads: usize,
        kvh: usize,
        kv_dim: usize,
        seq_antes: usize,
        ventana: usize,
        escala: f32,
        s: usize,
        dyn_ptr: u64,
    ) -> Result<(), String> {
        let (mut a0, mut a1, mut a2, mut a3, mut a4) = (d_q, dk, dv, d_par, d_ml);
        let (mut a5, mut a6, mut a7, mut a8, mut a9, mut a10, mut a11, mut a12) = (
            heads as c_int,
            kvh as c_int,
            kv_dim as c_int,
            seq_antes as c_int,
            ventana as c_int,
            escala,
            s as c_int,
            dyn_ptr,
        );
        self.lanzar3(
            self.capa.fns.att_split,
            (s as u32, kvh as u32, n as u32),
            (256, 1, 1),
            &mut [
                a!(a0),
                a!(a1),
                a!(a2),
                a!(a3),
                a!(a4),
                a!(a5),
                a!(a6),
                a!(a7),
                a!(a8),
                a!(a9),
                a!(a10),
                a!(a11),
                a!(a12),
            ],
        )?;
        self.marca("k attn.split")?;
        let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5) = (
            d_par,
            d_ml,
            d_attn,
            heads as c_int,
            kvh as c_int,
            s as c_int,
        );
        self.lanzar3(
            self.capa.fns.att_combine,
            (heads as u32, n as u32, 1),
            (128, 1, 1),
            &mut [a!(b0), a!(b1), a!(b2), a!(b3), a!(b4), a!(b5)],
        )
    }

    /// Atención de `n` tokens contra el KV del dispositivo: puntuaciones, softmax, P·V por trozos y su suma.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn atencion_dev(
        &mut self,
        d_q: u64,
        dk: u64,
        dv: u64,
        d_sc: u64,
        d_par: u64,
        d_attn: u64,
        n: usize,
        heads: usize,
        kvh: usize,
        hd: usize,
        kv_dim: usize,
        seq_antes: usize,
        ventana: usize,
        escala: f32,
        cap: usize,
        s_split: usize,
        dyn_ptr: u64,
        total_grid: usize,
    ) -> Result<(), String> {
        let total_max = total_grid;
        self.marca("k a.pre")?;
        {
            let (mut a0, mut a1, mut a2) = (d_q, dk, d_sc);
            let (mut a3, mut a4, mut a5, mut a6, mut a7, mut a8) = (
                heads as c_int,
                kvh as c_int,
                hd as c_int,
                kv_dim as c_int,
                seq_antes as c_int,
                ventana as c_int,
            );
            let (mut a9, mut a10, mut a11) = (escala, cap as c_int, dyn_ptr);
            self.lanzar3(
                self.capa.fns.scores,
                ((total_max as u32).div_ceil(32), heads as u32, n as u32),
                (128, 1, 1),
                &mut [
                    a!(a0),
                    a!(a1),
                    a!(a2),
                    a!(a3),
                    a!(a4),
                    a!(a5),
                    a!(a6),
                    a!(a7),
                    a!(a8),
                    a!(a9),
                    a!(a10),
                    a!(a11),
                ],
            )?;
        }
        self.marca("k a.scores")?;
        {
            let mut a0 = d_sc;
            let (mut a1, mut a2, mut a3, mut a4, mut a5) = (
                heads as c_int,
                seq_antes as c_int,
                ventana as c_int,
                cap as c_int,
                dyn_ptr,
            );
            self.lanzar3(
                self.capa.fns.softmax,
                (heads as u32, n as u32, 1),
                (256, 1, 1),
                &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4), a!(a5)],
            )?;
        }
        self.marca("k a.softmax")?;
        {
            let (mut a0, mut a1, mut a2) = (d_sc, dv, d_par);
            let (mut a3, mut a4, mut a5, mut a6, mut a7, mut a8, mut a9, mut a10, mut a11) = (
                heads as c_int,
                kvh as c_int,
                hd as c_int,
                kv_dim as c_int,
                seq_antes as c_int,
                ventana as c_int,
                cap as c_int,
                s_split as c_int,
                dyn_ptr,
            );
            self.lanzar3(
                self.capa.fns.pv,
                (s_split as u32, heads as u32, n as u32),
                (hd as u32, 1, 1),
                &mut [
                    a!(a0),
                    a!(a1),
                    a!(a2),
                    a!(a3),
                    a!(a4),
                    a!(a5),
                    a!(a6),
                    a!(a7),
                    a!(a8),
                    a!(a9),
                    a!(a10),
                    a!(a11),
                ],
            )?;
            let (mut b0, mut b1) = (d_par, d_attn);
            let (mut b2, mut b3, mut b4) = (heads as c_int, hd as c_int, s_split as c_int);
            self.lanzar3(
                self.capa.fns.reduce,
                (heads as u32, n as u32, 1),
                (hd as u32, 1, 1),
                &mut [a!(b0), a!(b1), a!(b2), a!(b3), a!(b4)],
            )?;
        }
        Ok(())
    }

    pub(super) fn capa_ejecutar(&mut self, c: &mut GemmaCapa<'_>) -> Result<bool, String> {
        let (n, h, heads, kvh, hd, ffn) = (c.n, c.h, c.heads, c.kv_heads, c.head_dim, c.ffn);
        let (q_dim, kv_dim) = (heads * hd, kvh * hd);
        if n == 0
            || n > N_MAX
            || c.hidden.len() != n * h
            || hd > 512
            || hd % 64 != 0
            || heads % kvh != 0
        {
            self.capa.rechazadas.insert(c.layer);
            return Ok(false);
        }
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        let Some((dk, dv, seq_antes, cap)) = self.kv_preparar(c)? else {
            return Ok(false);
        };
        if !self.capa.normas.contains_key(&c.layer) {
            let nd = NormasDev {
                attn: self.subir_norma(c.attn_norm)?,
                q: self.subir_norma(c.q_norm)?,
                k: self.subir_norma(c.k_norm)?,
                post_attn: self.subir_norma(c.post_attn_norm)?,
                ffn: self.subir_norma(c.ffn_norm)?,
                post_ffn: self.subir_norma(c.post_ffn_norm)?,
                rope: match c.rope_freqs {
                    Some(r) => self.subir_norma(r)?,
                    None => 0,
                },
            };
            self.capa.normas.insert(c.layer, nd);
        }
        let (n_attn, n_q, n_k, n_pa, n_ffn, n_pf, n_rope) = {
            let m = &self.capa.normas[&c.layer];
            (m.attn, m.q, m.k, m.post_attn, m.ffn, m.post_ffn, m.rope)
        };
        let d_hs = self.buf(B_HS, n * h)?;
        let d_xn = self.buf(B_XN, n * h)?;
        let d_q = self.buf(B_Q, n * q_dim)?;
        let d_k = self.buf(B_K, n * kv_dim)?;
        let d_v = self.buf(B_V, n * kv_dim)?;
        let d_attn = self.buf(B_ATTN, n * q_dim)?;
        let d_o = self.buf(B_O, n * h)?;
        let d_up = self.buf(B_UP, n * ffn)?;
        let d_gate = self.buf(B_GATE, n * ffn)?;
        // Atención por trozos de KV (`ga_split_*`) para las dos formas de Gemma 4: hd 256 con 2 cabezas por grupo y hd 512 con 16.
        let g_grupo = heads / kvh;
        let variante_ga: Option<(usize, CuFunction)> = if std::env::var_os("SOSO_CUDA_ATT_VIEJA").is_some() {
            None
        } else if hd == 256 && g_grupo == 2 {
            Some((2, self.capa.fns.ga_256_2))
        } else if hd == 512 && g_grupo == 16 {
            Some((16, self.capa.fns.ga_512_16))
        } else {
            None
        };
        let s_ga = {
            let total = seq_antes + n;
            (if c.ventana > 0 { total.min(c.ventana + n) } else { total }).div_ceil(GA_CH).max(1)
        };
        let d_sc = if variante_ga.is_some() { 0 } else { self.buf(B_SC, n * heads * cap)? };
        let s_split = {
            let total = seq_antes + n;
            let len = if c.ventana > 0 {
                total.min(c.ventana)
            } else {
                total
            };
            len.div_ceil(256).clamp(1, 16)
        };
        let (d_par, d_ml) = match variante_ga {
            Some((gp, _)) => (self.buf(B_PARCIAL, n * kvh * s_ga * gp * hd)?, self.buf(B_ML, n * kvh * s_ga * gp * 2)?),
            None => (self.buf(B_PARCIAL, n * heads * s_split * hd)?, 0),
        };
        if let Some((a, _)) = self.capa.perfil {
            ck(
                unsafe { (self.api.ev_record)(a, std::ptr::null_mut()) },
                "cuEventRecord",
            )?;
        }
        self.subir_f32(d_hs, c.hidden)?;
        self.marca("subir hidden")?;

        // ---------------------------------------------------------------- atención
        self.rms_rows(d_hs, n_attn, d_xn, n, h, c.eps)?;
        self.marca("normas")?;
        self.mm_dev(c.clave_q, d_xn, d_q, n)?;
        self.mm_dev(c.clave_k, d_xn, d_k, n)?;
        if c.v_de_k {
            ck(
                unsafe { (self.api.dtod)(d_v, d_k, n * kv_dim * 4) },
                "cuMemcpyDtoD",
            )?;
        } else {
            self.mm_dev(c.clave_v, d_xn, d_v, n)?;
        }
        self.marca("matmul q/k/v")?;
        if c.qk_norm {
            self.rms_rows(d_q, n_q, d_q, n * heads, hd, c.eps)?;
            self.rms_rows(d_k, n_k, d_k, n * kvh, hd, c.eps)?;
        }
        if c.v_norm {
            self.rms_rows(d_v, 0, d_v, n * kvh, hd, c.eps)?;
        }
        self.marca("normas")?;
        self.volcar("1 q (norm)", d_q, n * q_dim)?;
        self.volcar("1 k (norm)", d_k, n * kv_dim)?;
        self.volcar("1 v (norm)", d_v, n * kv_dim)?;
        for (x, nh) in [(d_q, heads), (d_k, kvh)] {
            self.rope_dev(x, nh, hd, n, c.pos0, c.theta, n_rope, 1.0, 0)?;
        }
        {
            let (mut a0, mut a1, mut a2, mut a3, mut a4, mut a5, mut a6, mut a7) = (
                d_k,
                d_v,
                dk,
                dv,
                kv_dim as c_int,
                seq_antes as c_int,
                n as c_int,
                0u64,
            );
            self.lanzar3(
                self.capa.fns.kv_append,
                ((kv_dim as u32).div_ceil(256), n as u32, 1),
                (256, 1, 1),
                &mut [
                    a!(a0),
                    a!(a1),
                    a!(a2),
                    a!(a3),
                    a!(a4),
                    a!(a5),
                    a!(a6),
                    a!(a7),
                ],
            )?;
        }
        self.marca("rope + kv")?;
        self.volcar("2 q (rope)", d_q, n * q_dim)?;
        self.volcar("2 k (rope)", d_k, n * kv_dim)?;
        if let Some((gp, f)) = variante_ga {
            self.atencion_ga_dev(f, gp, d_q, dk, dv, d_par, d_ml, d_attn, n, heads, kvh, hd, kv_dim, seq_antes, c.ventana, c.escala, s_ga)?;
        } else {
            self.atencion_dev(
                d_q,
                dk,
                dv,
                d_sc,
                d_par,
                d_attn,
                n,
                heads,
                kvh,
                hd,
                kv_dim,
                seq_antes,
                c.ventana,
                c.escala,
                cap,
                s_split,
                0,
                seq_antes + n,
            )?;
        }
        self.marca("atención")?;
        self.volcar("3 atención", d_attn, n * q_dim)?;
        self.mm_dev(c.clave_o, d_attn, d_o, n)?;
        self.marca("matmul o")?;
        self.rms_rows(d_o, n_pa, d_o, n, h, c.eps)?;
        self.add_scale(d_hs, d_o, d_hs, n * h, 1.0)?;
        self.marca("normas")?;

        self.volcar("4 hidden tras atención", d_hs, n * h)?;
        // ---------------------------------------------------------------- MLP (GeGLU)
        self.rms_rows(d_hs, n_ffn, d_xn, n, h, c.eps)?;
        self.marca("normas")?;
        self.mm_dev(c.clave_up, d_xn, d_up, n)?;
        self.mm_dev(c.clave_gate, d_xn, d_gate, n)?;
        self.marca("matmul up/gate")?;
        if !c.geglu {
            return Err("sólo GeGLU".into());
        }
        {
            let (mut a0, mut a1, mut a2) = (d_up, d_gate, (n * ffn) as c_int);
            self.lanzar3(
                self.capa.fns.geglu,
                (((n * ffn) as u32).div_ceil(256), 1, 1),
                (256, 1, 1),
                &mut [a!(a0), a!(a1), a!(a2)],
            )?;
        }
        self.marca("geglu")?;
        self.volcar("5 up·gelu(gate)", d_up, n * ffn)?;
        self.mm_dev(c.clave_down, d_up, d_o, n)?;
        self.marca("matmul down")?;
        self.rms_rows(d_o, n_pf, d_o, n, h, c.eps)?;
        self.add_scale(d_hs, d_o, d_hs, n * h, c.layer_scale)?;
        self.marca("normas")?;

        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        self.perfil_cerrar()?;
        let salida =
            unsafe { std::slice::from_raw_parts_mut(c.hidden.as_mut_ptr() as *mut u8, n * h * 4) };
        ck(
            unsafe { (self.api.dtoh)(salida.as_mut_ptr() as *mut c_void, d_hs, salida.len()) },
            "cuMemcpyDtoH",
        )?;
        let kv = self.capa.kvs.get_mut(&c.layer).unwrap();
        kv.stored += n;
        c.expulsadas = kv.dropped;
        self.stats.launches += 1;
        Ok(true)
    }

    /// Diagnóstico: cierra el tramo que empezó en la última marca y lo suma a `etapa` (sólo con `SOSO_CUDA_PERFIL`).
    pub(super) fn marca(&mut self, etapa: &'static str) -> Result<(), String> {
        let Some(_) = self.capa.perfil else {
            return Ok(());
        };
        // Sin sincronizar: cada marca registra un evento propio y los tiempos se leen todos juntos al final.
        let i = self.capa.marcas.len();
        while self.capa.evpool.len() <= i {
            let mut e: *mut c_void = std::ptr::null_mut();
            ck(unsafe { (self.api.ev_create)(&mut e, 0) }, "cuEventCreate")?;
            self.capa.evpool.push(e);
        }
        ck(
            unsafe { (self.api.ev_record)(self.capa.evpool[i], std::ptr::null_mut()) },
            "cuEventRecord",
        )?;
        self.capa.marcas.push(etapa);
        Ok(())
    }

    /// Cierra el tramo de marcas de una capa: espera al último evento y suma el tiempo de cada etapa.
    pub(super) fn perfil_cerrar(&mut self) -> Result<(), String> {
        let Some((a, _)) = self.capa.perfil else {
            return Ok(());
        };
        let marcas = std::mem::take(&mut self.capa.marcas);
        let Some(_) = marcas.last() else {
            return Ok(());
        };
        unsafe {
            ck(
                (self.api.ev_sync)(self.capa.evpool[marcas.len() - 1]),
                "cuEventSynchronize",
            )?;
            let mut previo = a;
            for (i, etapa) in marcas.iter().enumerate() {
                let ev = self.capa.evpool[i];
                let mut ms = 0.0f32;
                ck(
                    (self.api.ev_elapsed)(&mut ms, previo, ev),
                    "cuEventElapsedTime",
                )?;
                match self.capa.perfil_ms.iter_mut().find(|(n, _)| n == etapa) {
                    Some(e) => e.1 += ms as f64,
                    None => self.capa.perfil_ms.push((etapa, ms as f64)),
                }
                previo = ev;
            }
        }
        Ok(())
    }

    /// Diagnóstico: tiempos de GPU acumulados por etapa desde la última llamada (y los vacía).
    pub fn perfil_capa(&mut self) -> Vec<(&'static str, f64)> {
        std::mem::take(&mut self.capa.perfil_ms)
    }

    /// Copia `floats` floats del dispositivo al volcado de diagnóstico (si está activo).
    pub(super) fn volcar(
        &mut self,
        etapa: &'static str,
        ptr: u64,
        floats: usize,
    ) -> Result<(), String> {
        if !self.capa.depurar {
            return Ok(());
        }
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        let mut v = vec![0.0f32; floats];
        ck(
            unsafe { (self.api.dtoh)(v.as_mut_ptr() as *mut c_void, ptr, floats * 4) },
            "cuMemcpyDtoH",
        )?;
        self.capa.volcado.push((etapa, v));
        Ok(())
    }

    pub(super) fn add_scale(&self, a: u64, b: u64, y: u64, n: usize, s: f32) -> Result<(), String> {
        let (mut a0, mut a1, mut a2, mut a3, mut a4) = (a, b, y, n as c_int, s);
        self.lanzar3(
            self.capa.fns.add_scale,
            ((n as u32).div_ceil(256), 1, 1),
            (256, 1, 1),
            &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4)],
        )
    }

    /// Sube un tensor para la capa entera (mismo reempaquetado que los matmul sueltos).
    pub(super) fn capa_subir(
        &mut self,
        key: &str,
        view: &soso_llm_core::layer::TensorView<'_>,
        rows: usize,
        cols: usize,
    ) -> Result<bool, String> {
        if self.pesos.get(key).is_some_and(|p| p.is_some()) {
            return Ok(true);
        }
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        let p = self.subir_peso(view, rows, cols)?;
        let ok = p.is_some();
        self.pesos.insert(key.to_string(), p);
        Ok(ok)
    }
}

#[allow(dead_code)]
fn _usa(_: &Peso) {}
