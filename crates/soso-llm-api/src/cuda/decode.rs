//! Decodificación de un token **entero** en la GPU (grafo residente): embedding de entrada → todas las capas (Gated DeltaNet o atención con
//! puerta, y FFN MoE con experto compartido) → norma final → cabeza de logits, con las activaciones siempre en VRAM. Por token sólo viajan la
//! fila de embedding (a la tarjeta) y los logits más las filas K/V nuevas (de vuelta). El estado recurrente (GDN) y el KV de las capas de
//! atención viven en el dispositivo durante la sesión de decode.
//!
//! Expertos: las capas cuyos 3 × `n_exp` tensores caben en VRAM (de la última hacia atrás, como `--n-cpu-moe` de llama.cpp pero al revés) se
//! resuelven enteras en la tarjeta con el índice elegido por el propio dispositivo; en las demás el dispositivo calcula el router, el host
//! calcula los expertos elegidos y vuelve a subir la suma (una sincronización por capa de ese tipo).

use std::ffi::{CString, c_int, c_void};

use soso_llm_core::gpu::{DecodePlan, EstadoCapaDec, MatDec, VistaCruda};

use super::{Arg, CuFunction, CudaGpu, ck, codigo_mma};

/// Posiciones de contexto por trozo de la atención de decode partida.
const CHUNK: usize = 1024;
/// Trozos máximos (contexto máximo = `CHUNK * MAX_SPLIT`).
const MAX_SPLIT: usize = 128;

pub(super) struct FnsDec {
    rmsnorm: CuFunction,
    add: CuFunction,
    swiglu: CuFunction,
    gdn_conv: CuFunction,
    gdn_params: CuFunction,
    gdn_rec: CuFunction,
    attn_prep: CuFunction,
    attn_split: CuFunction,
    attn_combine: CuFunction,
    router: CuFunction,
    gateup: CuFunction,
    down: CuFunction,
    pgate: CuFunction,
    combine: CuFunction,
    combine_cpu: CuFunction,
    matvec: CuFunction,
    gateup_w: CuFunction,
    down_w: CuFunction,
}

#[derive(Clone, Copy)]
struct MatDev {
    d: u64,
    code: i32,
    rb: i64,
    rows: usize,
    cols: usize,
}

struct GdnD {
    bw: MatDev,
    aw: MatDev,
    cw: u64,
    dt: u64,
    a: u64,
    nw: u64,
    s: u64,
    cs: u64,
    n_k: usize,
    n_v: usize,
    d: usize,
    kernel: usize,
    qkv_dim: usize,
    v_dim: usize,
}

struct GatedD {
    qn: u64,
    kn: u64,
    heads: usize,
    kv_heads: usize,
    hd: usize,
    rope_n: usize,
    theta: f32,
    kv_dim: usize,
    attn_idx: usize,
}

struct ShD {
    gate: MatDev,
    up: MatDev,
    down: MatDev,
    pg: u64,
    ffn: usize,
}

struct MoeD {
    router: MatDev,
    n_exp: usize,
    top_k: usize,
    ffn: usize,
    en_gpu: bool,
    tg: u64,
    tu: u64,
    td: u64,
    dg: i32,
    du: i32,
    dd: i32,
    rbg: i64,
    rbu: i64,
    rbd: i64,
    sh: Option<ShD>,
}

struct CapaD {
    layer: u32,
    na: u64,
    nf: u64,
    mats: Vec<MatDev>,
    gdn: Option<GdnD>,
    gated: Option<GatedD>,
    moe: MoeD,
}

/// Índices de los búferes de trabajo.
mod b {
    pub const HID: usize = 0;
    pub const XN: usize = 1;
    pub const Y: usize = 2;
    pub const QKV: usize = 3;
    pub const Z: usize = 4;
    pub const BRAW: usize = 5;
    pub const ARAW: usize = 6;
    pub const UP: usize = 7;
    pub const DECAY: usize = 8;
    pub const BETA: usize = 9;
    pub const O: usize = 10;
    pub const QF: usize = 11;
    pub const Q: usize = 12;
    pub const GATE: usize = 13;
    pub const KK: usize = 14;
    pub const VV: usize = 15;
    pub const ATT: usize = 16;
    pub const PO: usize = 17;
    pub const PML: usize = 18;
    pub const RLOG: usize = 19;
    pub const RIDX: usize = 20;
    pub const RWT: usize = 21;
    pub const ACT: usize = 22;
    pub const YS: usize = 23;
    pub const G1: usize = 24;
    pub const U1: usize = 25;
    pub const YSH: usize = 26;
    pub const SG: usize = 27;
    pub const LOGITS: usize = 28;
    pub const DYN: usize = 29;
    pub const KST: usize = 30;
    pub const CPUOUT: usize = 31;
    pub const N: usize = 32;
}

pub(super) struct DecodeDev {
    f: FnsDec,
    capas: Vec<CapaD>,
    h: usize,
    eps: f32,
    nfin: u64,
    lm: MatDev,
    vocab: usize,
    n_attn: usize,
    kv_dim: usize,
    bufs: [(u64, usize); b::N],
    grafos: std::collections::HashMap<(usize, usize, usize), *mut c_void>,
    paradas: std::collections::HashMap<(usize, usize, usize), Option<usize>>,
    kv_gen: u64,
}

impl CudaGpu {
    fn dec_buf(bufs: &mut [(u64, usize); b::N], api: &super::Api, i: usize, bytes: usize) -> Result<u64, String> {
        if bytes > bufs[i].1 {
            if bufs[i].0 != 0 {
                unsafe { (api.mem_free)(bufs[i].0) };
            }
            let mut p = 0u64;
            ck(unsafe { (api.mem_alloc)(&mut p, bytes.max(16)) }, "cuMemAlloc")?;
            bufs[i] = (p, bytes.max(16));
        }
        Ok(bufs[i].0)
    }

    fn f32_a_dev(&mut self, v: &[f32]) -> Result<u64, String> {
        let p = self.alloc(v.len() * 4)?;
        self.subir(p, unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4) })?;
        Ok(p)
    }

    fn vista_f32_a_dev(&mut self, v: &VistaCruda) -> Result<u64, String> {
        let p = self.alloc(v.len)?;
        self.subir(p, unsafe { std::slice::from_raw_parts(v.ptr as *const u8, v.len) })?;
        Ok(p)
    }

    /// Matriz residente en VRAM (se sube si hace falta) con el código del decodificador de los kernels de tensor cores.
    fn dec_mat(&mut self, m: &MatDec) -> Result<Option<MatDev>, String> {
        let view = unsafe { m.v.vista() };
        if !self.pesos.contains_key(&m.clave) {
            self.subiendo_experto = false;
            let p = self.subir_peso(&view, m.rows, m.cols)?;
            self.pesos.insert(m.clave.clone(), p);
        }
        match self.pesos.get(&m.clave) {
            Some(Some(p)) if p.rows == m.rows && p.cols == m.cols => match codigo_mma(p.dtype, m.cols) {
                Some((code, rb)) => Ok(Some(MatDev { d: p.d, code, rb, rows: m.rows, cols: m.cols })),
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    pub(super) fn decode_preparar_impl(&mut self, plan: &DecodePlan) -> Result<bool, String> {
        if self.dec.is_some() {
            return Ok(true);
        }
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        let get = |n: &str| -> Result<CuFunction, String> {
            let c = CString::new(n).unwrap();
            let mut f: CuFunction = std::ptr::null_mut();
            ck(unsafe { (self.api.module_fn)(&mut f, self.modulo_iq, c.as_ptr()) }, n)?;
            Ok(f)
        };
        let f = FnsDec {
            rmsnorm: get("dec_rmsnorm")?,
            add: get("dec_add")?,
            swiglu: get("dec_swiglu")?,
            gdn_conv: get("dec_gdn_conv")?,
            gdn_params: get("gdn_params")?,
            gdn_rec: get("gdn_rec")?,
            attn_prep: get("dec_attn_prep")?,
            attn_split: get("dec_attn_split")?,
            attn_combine: get("dec_attn_combine")?,
            router: get("dec_router_topk")?,
            gateup: get("dec_moe_gateup")?,
            down: get("dec_moe_down")?,
            pgate: get("dec_pgate")?,
            combine: get("dec_moe_combine")?,
            combine_cpu: get("dec_moe_combine_cpu")?,
            matvec: get("dec_matvec")?,
            gateup_w: get("dec_moe_gateup_w")?,
            down_w: get("dec_moe_down_w")?,
        };
        // 1) pesos densos y pequeños, todos residentes (si alguno no cabe o no tiene decodificador, no hay decode en GPU)
        let mut capas: Vec<CapaD> = Vec::with_capacity(plan.capas.len());
        let mut n_attn = 0usize;
        let (mut kv_dim, mut max_qkv, mut max_v, mut max_q, mut max_kv, mut max_ffn, mut max_exp, mut max_top) = (0, 0, 0, 0, 0, 0, 0, 0);
        for c in &plan.capas {
            let mut mats = Vec::new();
            for m in &c.mats {
                match self.dec_mat(m)? {
                    Some(md) => mats.push(md),
                    None => {
                        eprintln!("decode GPU: sin decodificador o sin VRAM para {} (dtype {})", m.clave, m.v.dtype);
                        return Ok(false);
                    }
                }
            }
            let na = self.f32_a_dev(&c.norma_attn)?;
            let nf = self.f32_a_dev(&c.norma_ffn)?;
            let f32m = |s: &mut CudaGpu, v: &VistaCruda, rows: usize, cols: usize| -> Result<MatDev, String> {
                let d = s.vista_f32_a_dev(v)?;
                Ok(MatDev { d, code: 1001, rb: (cols * 4) as i64, rows, cols })
            };
            let (mut gdn, mut gated) = (None, None);
            if let Some(g) = &c.gdn {
                let (qkv_dim, v_dim) = (2 * g.n_k * g.d + g.n_v * g.d, g.n_v * g.d);
                max_qkv = max_qkv.max(qkv_dim);
                max_v = max_v.max(v_dim);
                let bw = f32m(self, &g.beta_w, g.n_v, plan.h)?;
                let aw = f32m(self, &g.alpha_w, g.n_v, plan.h)?;
                let cw = self.vista_f32_a_dev(&g.conv_w)?;
                let (dt, a, nw) = (self.f32_a_dev(&g.dt)?, self.f32_a_dev(&g.a)?, self.f32_a_dev(&g.norma)?);
                let s = self.alloc(g.n_v * g.d * g.d * 4)?;
                let cs = self.alloc((g.kernel - 1) * qkv_dim * 4)?;
                gdn = Some(GdnD { bw, aw, cw, dt, a, nw, s, cs, n_k: g.n_k, n_v: g.n_v, d: g.d, kernel: g.kernel, qkv_dim, v_dim });
            }
            if let Some(g) = &c.gated {
                let q_dim = g.heads * g.hd;
                max_q = max_q.max(q_dim);
                max_kv = max_kv.max(g.kv_heads * g.hd);
                kv_dim = g.kv_heads * g.hd;
                let (qn, kn) = (self.f32_a_dev(&g.q_norm)?, self.f32_a_dev(&g.k_norm)?);
                gated = Some(GatedD { qn, kn, heads: g.heads, kv_heads: g.kv_heads, hd: g.hd, rope_n: g.rope_n, theta: g.theta, kv_dim, attn_idx: n_attn });
                n_attn += 1;
            }
            let router = f32m(self, &c.moe.router, c.moe.n_exp, plan.h)?;
            max_ffn = max_ffn.max(c.moe.ffn);
            max_exp = max_exp.max(c.moe.n_exp);
            max_top = max_top.max(c.moe.top_k);
            let sh = match &c.moe.compartido {
                Some(cp) => {
                    let (g1, u1, d1) = (self.dec_mat(&cp.mats[0])?, self.dec_mat(&cp.mats[1])?, self.dec_mat(&cp.mats[2])?);
                    match (g1, u1, d1) {
                        (Some(g1), Some(u1), Some(d1)) => {
                            let pg = self.vista_f32_a_dev(&cp.pg)?;
                            max_ffn = max_ffn.max(g1.rows);
                            Some(ShD { gate: g1, up: u1, down: d1, pg, ffn: g1.rows })
                        }
                        _ => return Ok(false),
                    }
                }
                None => None,
            };
            capas.push(CapaD {
                layer: c.layer,
                na,
                nf,
                mats,
                gdn,
                gated,
                moe: MoeD {
                    router,
                    n_exp: c.moe.n_exp,
                    top_k: c.moe.top_k,
                    ffn: c.moe.ffn,
                    en_gpu: false,
                    tg: 0,
                    tu: 0,
                    td: 0,
                    dg: 0,
                    du: 0,
                    dd: 0,
                    rbg: 0,
                    rbu: 0,
                    rbd: 0,
                    sh,
                },
            });
        }
        let lm = match self.dec_mat(&plan.lm_head)? {
            Some(m) => m,
            None => {
                eprintln!("decode GPU: la cabeza de logits no cabe o no tiene decodificador");
                return Ok(false);
            }
        };
        let nfin = self.f32_a_dev(&plan.norma_final)?;
        // 2) expertos: capas enteras residentes, de la última hacia atrás, mientras quede sitio (margen para KV espejado y búferes)
        let margen = std::env::var("SOSO_RESERVA_DECODE_MIB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1536) << 20;
        let mut con_gpu = 0usize;
        for ci in (0..plan.capas.len()).rev() {
            let moe = &plan.capas[ci].moe;
            let total: usize = moe.expertos.iter().flat_map(|e| e.iter()).map(|m| m.v.len).sum();
            if total == 0 || self.usado + total + margen > self.presupuesto {
                continue;
            }
            let (mut tg, mut tu, mut td) = (Vec::new(), Vec::new(), Vec::new());
            let (mut ok, mut cod) = (true, [(0i32, 0i64); 3]);
            'exp: for (e, tri) in moe.expertos.iter().enumerate() {
                for k in 0..3 {
                    let m = &tri[k];
                    let view = unsafe { m.v.vista() };
                    if !self.pesos.contains_key(&m.clave) {
                        self.subiendo_experto = false;
                        let p = self.subir_peso(&view, m.rows, m.cols)?;
                        self.pesos.insert(m.clave.clone(), p);
                    }
                    match self.pesos.get(&m.clave) {
                        Some(Some(p)) if p.rows == m.rows && p.cols == m.cols => {
                            let Some(cr) = codigo_mma(p.dtype, m.cols) else { ok = false; break 'exp };
                            if e == 0 {
                                cod[k] = cr;
                            } else if cod[k] != cr {
                                ok = false;
                                break 'exp;
                            }
                            [&mut tg, &mut tu, &mut td][k].push(p.d);
                        }
                        _ => {
                            ok = false;
                            break 'exp;
                        }
                    }
                }
            }
            if !ok {
                continue;
            }
            let (ptg, ptu, ptd) = (self.u64_a_dev(&tg)?, self.u64_a_dev(&tu)?, self.u64_a_dev(&td)?);
            let m = &mut capas[ci].moe;
            m.en_gpu = true;
            (m.tg, m.tu, m.td) = (ptg, ptu, ptd);
            (m.dg, m.rbg, m.du, m.rbu, m.dd, m.rbd) = (cod[0].0, cod[0].1, cod[1].0, cod[1].1, cod[2].0, cod[2].1);
            con_gpu += 1;
        }
        // lo que no quedó residente no se sube más tarde por tensores sueltos
        self.tope_expertos = self.usado;
        eprintln!("decode GPU: {} de {} capas con los expertos en la tarjeta ({} MiB en VRAM)", con_gpu, capas.len(), self.usado >> 20);
        // 3) búferes de trabajo
        let h = plan.h;
        let mut bufs = [(0u64, 0usize); b::N];
        let sizes: [(usize, usize); b::N] = {
            let mut z = [(0usize, 0usize); b::N];
            let fl = |n: usize| n * 4;
            z[b::HID] = (b::HID, fl(h));
            z[b::XN] = (b::XN, fl(h));
            z[b::Y] = (b::Y, fl(h));
            z[b::QKV] = (b::QKV, fl(max_qkv.max(1)));
            z[b::Z] = (b::Z, fl(max_v.max(1)));
            z[b::BRAW] = (b::BRAW, fl(256));
            z[b::ARAW] = (b::ARAW, fl(256));
            z[b::UP] = (b::UP, fl(max_qkv.max(1)));
            z[b::DECAY] = (b::DECAY, fl(256));
            z[b::BETA] = (b::BETA, fl(256));
            z[b::O] = (b::O, fl(max_v.max(1)));
            z[b::QF] = (b::QF, fl((2 * max_q).max(1)));
            z[b::Q] = (b::Q, fl(max_q.max(1)));
            z[b::GATE] = (b::GATE, fl(max_q.max(1)));
            z[b::KK] = (b::KK, fl(max_kv.max(1)));
            z[b::VV] = (b::VV, fl(max_kv.max(1)));
            z[b::ATT] = (b::ATT, fl(max_q.max(1)));
            z[b::PO] = (b::PO, fl(16 * 16 * MAX_SPLIT * 256));
            z[b::PML] = (b::PML, fl(16 * 16 * MAX_SPLIT * 2));
            z[b::RLOG] = (b::RLOG, fl(max_exp.max(1)));
            z[b::RIDX] = (b::RIDX, 4 * max_top.max(1));
            z[b::RWT] = (b::RWT, fl(max_top.max(1)));
            z[b::ACT] = (b::ACT, fl((max_top.max(1)) * max_ffn.max(1)));
            z[b::YS] = (b::YS, fl(max_top.max(1) * h));
            z[b::G1] = (b::G1, fl(max_ffn.max(1)));
            z[b::U1] = (b::U1, fl(max_ffn.max(1)));
            z[b::YSH] = (b::YSH, fl(h));
            z[b::SG] = (b::SG, 16);
            z[b::LOGITS] = (b::LOGITS, fl(plan.vocab));
            z[b::DYN] = (b::DYN, 16);
            z[b::KST] = (b::KST, 2 * 2 * kv_dim.max(1) * n_attn.max(1));
            z[b::CPUOUT] = (b::CPUOUT, fl(h));
            z
        };
        for (i, (_, bytes)) in sizes.iter().enumerate() {
            Self::dec_buf(&mut bufs, &self.api, i, *bytes)?;
        }
        self.dec = Some(DecodeDev { f, capas, h, eps: plan.eps, nfin, lm, vocab: plan.vocab, n_attn, kv_dim, bufs, grafos: Default::default(), paradas: Default::default(), kv_gen: 0 });
        Ok(true)
    }

    fn u64_a_dev(&mut self, v: &[u64]) -> Result<u64, String> {
        let p = self.alloc(v.len() * 8)?;
        self.subir(p, unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 8) })?;
        Ok(p)
    }

    /// Sesión de decode: sube el estado GDN y las filas de KV que falten al dispositivo.
    pub(super) fn decode_subir_estado_impl(&mut self, estados: &mut [EstadoCapaDec<'_>], pos: usize) -> Result<(), String> {
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        let Some(dec) = self.dec.take() else { return Err("decode sin preparar".into()) };
        let r = (|| -> Result<(), String> {
            for (c, e) in dec.capas.iter().zip(estados.iter_mut()) {
                if let Some(g) = &c.gdn {
                    if e.gdn_s.len() != g.n_v * g.d * g.d || e.gdn_conv.len() != (g.kernel - 1) * g.qkv_dim {
                        return Err("estado GDN de tamaño inesperado".into());
                    }
                    self.subir(g.s, unsafe { std::slice::from_raw_parts(e.gdn_s.as_ptr() as *const u8, e.gdn_s.len() * 4) })?;
                    self.subir(g.cs, unsafe { std::slice::from_raw_parts(e.gdn_conv.as_ptr() as *const u8, e.gdn_conv.len() * 4) })?;
                }
                if let Some(g) = &c.gated {
                    self.kv_asegurar(c.layer, pos + 1, *e.filas_validas, g.kv_dim)?;
                    let valid = (*e.filas_validas).min(pos);
                    let (kd, vd, _) = self.kvdev[&c.layer];
                    let (ini, fin) = (valid * g.kv_dim, pos * g.kv_dim);
                    if fin > ini {
                        if e.k.len() < fin || e.v.len() < fin {
                            return Err("KV del host más corto que la posición".into());
                        }
                        self.subir(kd + (ini * 2) as u64, unsafe { std::slice::from_raw_parts(e.k.as_ptr().add(ini) as *const u8, (fin - ini) * 2) })?;
                        self.subir(vd + (ini * 2) as u64, unsafe { std::slice::from_raw_parts(e.v.as_ptr().add(ini) as *const u8, (fin - ini) * 2) })?;
                    }
                    *e.filas_validas = pos;
                }
            }
            Ok(())
        })();
        self.dec = Some(dec);
        r
    }

    /// Capacidad del KV espejado de la capa `layer` (conserva las `validas` primeras filas al crecer).
    fn kv_asegurar(&mut self, layer: u32, filas: usize, validas: usize, kv_dim: usize) -> Result<(), String> {
        let (kd, vd, cap) = self.kvdev.get(&layer).copied().unwrap_or((0, 0, 0));
        if filas <= cap {
            return Ok(());
        }
        let ncap = filas.div_ceil(4096) * 4096 + 4096;
        let (nk, nv) = (self.alloc(ncap * kv_dim * 2)?, self.alloc(ncap * kv_dim * 2)?);
        if kd != 0 && validas > 0 {
            ck(unsafe { (self.api.dtod)(nk, kd, validas * kv_dim * 2) }, "cuMemcpyDtoD")?;
            ck(unsafe { (self.api.dtod)(nv, vd, validas * kv_dim * 2) }, "cuMemcpyDtoD")?;
        }
        if kd != 0 {
            unsafe {
                (self.api.mem_free)(kd);
                (self.api.mem_free)(vd);
            }
        }
        self.kvdev.insert(layer, (nk, nv, ncap));
        self.kv_gen += 1;
        Ok(())
    }

    pub(super) fn decode_bajar_estado_impl(&mut self, estados: &mut [EstadoCapaDec<'_>]) -> Result<(), String> {
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        let Some(dec) = self.dec.as_ref() else { return Ok(()) };
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        for (c, e) in dec.capas.iter().zip(estados.iter_mut()) {
            if let Some(g) = &c.gdn {
                ck(unsafe { (self.api.dtoh)(e.gdn_s.as_mut_ptr() as *mut c_void, g.s, e.gdn_s.len() * 4) }, "cuMemcpyDtoH")?;
                ck(unsafe { (self.api.dtoh)(e.gdn_conv.as_mut_ptr() as *mut c_void, g.cs, e.gdn_conv.len() * 4) }, "cuMemcpyDtoH")?;
            }
        }
        Ok(())
    }

    fn dec_mm1(&self, f: CuFunction, m: &MatDev, x: u64, y: u64) -> Result<(), String> {
        if std::env::var("SOSO_DECODE_MMA").is_ok() {
            return self.lanzar_a(
                self.f_iq_matmul_n,
                ((m.rows as u32).div_ceil(64), 1, 1),
                (128, 1, 1),
                &[Arg::P(m.d), Arg::P(x), Arg::P(y), Arg::I(m.rows as i32), Arg::I(m.cols as i32), Arg::I(1), Arg::I(m.code), Arg::L(m.rb)],
            );
        }
        let mut vals: Vec<u64> = vec![m.d, x, y, m.rows as u32 as u64, m.cols as u32 as u64, m.code as u32 as u64, m.rb as u64];
        let mut ptrs: Vec<*mut c_void> = vals.iter_mut().map(|v| v as *mut u64 as *mut c_void).collect();
        self.lanzar3s(f, ((m.rows as u32).div_ceil(8), 1, 1), (256, 1, 1), (m.cols * 4) as u32, &mut ptrs)
    }

    pub(super) fn decode_token_impl(
        &mut self,
        embed: &[f32],
        pos: usize,
        cpu_moe: &mut dyn FnMut(usize, &[f32], &[u32], &[f32], &mut [f32]) -> Result<(), ()>,
        logits: &mut [f32],
        kv_rows: &mut [u16],
    ) -> Result<bool, String> {
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        let Some(mut dec) = self.dec.take() else { return Ok(false) };
        let r = self.decode_token_dev(&mut dec, embed, pos, cpu_moe, logits, kv_rows);
        self.dec = Some(dec);
        r
    }

    fn decode_token_dev(
        &mut self,
        d: &mut DecodeDev,
        embed: &[f32],
        pos: usize,
        cpu_moe: &mut dyn FnMut(usize, &[f32], &[u32], &[f32], &mut [f32]) -> Result<(), ()>,
        logits: &mut [f32],
        kv_rows: &mut [u16],
    ) -> Result<bool, String> {
        let h = d.h;
        if embed.len() != h || logits.len() != d.vocab || kv_rows.len() != d.n_attn * 2 * d.kv_dim || pos + 1 > CHUNK * MAX_SPLIT {
            return Ok(false);
        }
        let bf = |i: usize| d.bufs[i].0;
        // capacidad del KV de las capas de atención (sin perder las `pos` filas ya subidas)
        let gen_antes = self.kv_gen;
        for c in &d.capas {
            if let Some(g) = &c.gated {
                self.kv_asegurar(c.layer, pos + 1, pos, g.kv_dim)?;
            }
        }
        if self.kv_gen != gen_antes || d.kv_gen != self.kv_gen {
            // las direcciones del KV cambiaron: los grafos capturados ya no valen
            if let Some(g) = &self.grafo {
                for (_, ex) in d.grafos.drain() {
                    unsafe { (g.graph_exec_destroy)(ex) };
                }
            }
            d.kv_gen = self.kv_gen;
        }
        let f32b = |v: &[f32]| unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 4) };
        self.subir(bf(b::HID), f32b(embed))?;
        let dyn_host = [pos as i32, pos as i32 + 1, 0, 0];
        self.subir(bf(b::DYN), unsafe { std::slice::from_raw_parts(dyn_host.as_ptr() as *const u8, 16) })?;
        let nsplit = (pos + 1).div_ceil(CHUNK).min(MAX_SPLIT).next_power_of_two().min(MAX_SPLIT);
        let usar_grafos = self.grafo.is_some() && std::env::var("SOSO_DECODE_SIN_GRAFO").is_err();
        let (mut desde, mut comb) = (0usize, None::<usize>);
        let perfil = std::env::var("SOSO_DECODE_PERFIL").is_ok();
        let (mut t_gpu, mut t_cpu, mut nseg) = (0f64, 0f64, 0u32);
        let t_ini = std::time::Instant::now();
        loop {
            let t_a = std::time::Instant::now();
            let parada = if usar_grafos {
                let clave = (desde, comb.map_or(usize::MAX, |c| c), nsplit);
                let ex = match d.grafos.get(&clave) {
                    Some(&e) => Some(e),
                    None => {
                        let g = self.grafo.as_ref().unwrap();
                        let (mut gr, mut ex) = (std::ptr::null_mut(), std::ptr::null_mut());
                        self.capturando = true;
                        let ini = unsafe { (g.capture_begin)(self.stream, 0) };
                        let r = if ini == 0 { self.dec_emitir(d, desde, comb, nsplit) } else { Err(format!("cuStreamBeginCapture {ini}")) };
                        let fin = if ini == 0 { unsafe { (g.capture_end)(self.stream, &mut gr) } } else { -1 };
                        self.capturando = false;
                        match r {
                            Ok(p) if fin == 0 => {
                                let inst = unsafe { (g.graph_instantiate)(&mut ex, gr, 0) };
                                unsafe { (g.graph_destroy)(gr) };
                                if inst != 0 {
                                    return Err(format!("cuGraphInstantiate {inst}"));
                                }
                                d.grafos.insert(clave, ex);
                                d.paradas.insert(clave, p);
                                Some(ex)
                            }
                            Ok(_) => return Err(format!("captura del grafo de decode: {fin}")),
                            Err(e) => return Err(e),
                        }
                    }
                };
                let ex = ex.unwrap();
                let g = self.grafo.as_ref().unwrap();
                ck(unsafe { (g.graph_launch)(ex, self.stream) }, "cuGraphLaunch")?;
                d.paradas[&clave]
            } else {
                self.dec_emitir(d, desde, comb, nsplit)?
            };
            ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
            t_gpu += t_a.elapsed().as_secs_f64();
            nseg += 1;
            let t_b = std::time::Instant::now();
            match parada {
                None => break,
                Some(ci) => {
                    let m = &d.capas[ci].moe;
                    let mut xn = vec![0f32; h];
                    let mut idx = vec![0i32; m.top_k];
                    let mut wt = vec![0f32; m.top_k];
                    ck(unsafe { (self.api.dtoh)(xn.as_mut_ptr() as *mut c_void, bf(b::XN), h * 4) }, "cuMemcpyDtoH")?;
                    ck(unsafe { (self.api.dtoh)(idx.as_mut_ptr() as *mut c_void, bf(b::RIDX), m.top_k * 4) }, "cuMemcpyDtoH")?;
                    ck(unsafe { (self.api.dtoh)(wt.as_mut_ptr() as *mut c_void, bf(b::RWT), m.top_k * 4) }, "cuMemcpyDtoH")?;
                    let idx_u: Vec<u32> = idx.iter().map(|&i| i as u32).collect();
                    let mut cpu = vec![0f32; h];
                    cpu_moe(ci, &xn, &idx_u, &wt, &mut cpu).map_err(|_| "expertos en CPU: error".to_string())?;
                    self.subir(bf(b::CPUOUT), f32b(&cpu))?;
                    desde = ci + 1;
                    comb = Some(ci);
                    t_cpu += t_b.elapsed().as_secs_f64();
                }
            }
        }
        ck(unsafe { (self.api.dtoh)(logits.as_mut_ptr() as *mut c_void, bf(b::LOGITS), d.vocab * 4) }, "cuMemcpyDtoH")?;
        if d.n_attn > 0 {
            ck(unsafe { (self.api.dtoh)(kv_rows.as_mut_ptr() as *mut c_void, bf(b::KST), kv_rows.len() * 2) }, "cuMemcpyDtoH")?;
        }
        if perfil {
            eprintln!("[dec] pos {pos}: total {:.1} ms, gpu {:.1} ms en {nseg} segmentos, cpu {:.1} ms", t_ini.elapsed().as_secs_f64() * 1e3, t_gpu * 1e3, t_cpu * 1e3);
        }
        self.stats.launches += 1;
        Ok(true)
    }

    fn salta(que: &str) -> bool {
        std::env::var("SOSO_DECODE_SALTAR").is_ok_and(|v| v.split(',').any(|x| x == que))
    }

    /// Encola las operaciones del token desde la capa `desde` (precedidas, si `comb`, de la suma de los expertos de CPU de esa capa) hasta
    /// el router de la siguiente capa con expertos en CPU —devuelve su índice— o hasta los logits (`None`).
    fn dec_emitir(&self, d: &DecodeDev, desde: usize, comb: Option<usize>, nsplit: usize) -> Result<Option<usize>, String> {
        let h = d.h;
        let hh = h as i32;
        let bf = |i: usize| d.bufs[i].0;
        let blocks256 = |n: usize| (n as u32).div_ceil(256);
        let rms = |s: &Self, x: u64, w: u64, y: u64, n: usize| -> Result<(), String> {
            s.lanzar_a(d.f.rmsnorm, (1, 1, 1), (256, 1, 1), &[Arg::P(x), Arg::P(w), Arg::P(y), Arg::I(n as i32), Arg::F(d.eps)])
        };
        let add = |s: &Self, a: u64, bb: u64, n: usize| -> Result<(), String> {
            s.lanzar_a(d.f.add, (blocks256(n), 1, 1), (256, 1, 1), &[Arg::P(a), Arg::P(bb), Arg::I(n as i32)])
        };
        if let Some(ci) = comb {
            let ysh = if d.capas[ci].moe.sh.is_some() { bf(b::YSH) } else { 0 };
            self.lanzar_a(
                d.f.combine_cpu,
                (blocks256(h), 1, 1),
                (256, 1, 1),
                &[Arg::P(bf(b::HID)), Arg::P(bf(b::CPUOUT)), Arg::P(ysh), Arg::P(bf(b::SG)), Arg::I(hh)],
            )?;
        }
        for (ci, c) in d.capas.iter().enumerate().skip(desde) {
            // ---- mezcla de tokens (GDN o atención con puerta) ----
            rms(self, bf(b::HID), c.na, bf(b::XN), h)?;
            if let Some(g) = &c.gdn {
                self.dec_mm1(d.f.matvec, &c.mats[0], bf(b::XN), bf(b::QKV))?;
                self.dec_mm1(d.f.matvec, &c.mats[1], bf(b::XN), bf(b::Z))?;
                self.dec_mm1(d.f.matvec, &g.bw, bf(b::XN), bf(b::BRAW))?;
                self.dec_mm1(d.f.matvec, &g.aw, bf(b::XN), bf(b::ARAW))?;
                self.lanzar_a(
                    d.f.gdn_params,
                    (blocks256(g.n_v), 1, 1),
                    (256, 1, 1),
                    &[Arg::P(bf(b::ARAW)), Arg::P(bf(b::BRAW)), Arg::P(g.dt), Arg::P(g.a), Arg::P(bf(b::DECAY)), Arg::P(bf(b::BETA)), Arg::I(1), Arg::I(g.n_v as i32)],
                )?;
                self.lanzar_a(
                    d.f.gdn_conv,
                    (blocks256(g.qkv_dim), 1, 1),
                    (256, 1, 1),
                    &[Arg::P(g.cs), Arg::P(bf(b::QKV)), Arg::P(g.cw), Arg::P(bf(b::UP)), Arg::I(g.qkv_dim as i32), Arg::I(g.kernel as i32)],
                )?;
                self.lanzar_a(
                    d.f.gdn_rec,
                    (g.n_v as u32, 1, 1),
                    (256, 1, 1),
                    &[Arg::P(bf(b::UP)), Arg::P(bf(b::Z)), Arg::P(bf(b::DECAY)), Arg::P(bf(b::BETA)), Arg::P(g.nw), Arg::P(g.s), Arg::P(bf(b::O)), Arg::I(1), Arg::I(g.n_k as i32), Arg::I(g.n_v as i32), Arg::F(d.eps)],
                )?;
                self.dec_mm1(d.f.matvec, &c.mats[2], bf(b::O), bf(b::Y))?;
                add(self, bf(b::HID), bf(b::Y), h)?;
            } else if let Some(g) = &c.gated {
                let (kd, vd, _) = self.kvdev[&c.layer];
                self.dec_mm1(d.f.matvec, &c.mats[0], bf(b::XN), bf(b::QF))?;
                self.dec_mm1(d.f.matvec, &c.mats[1], bf(b::XN), bf(b::KK))?;
                self.dec_mm1(d.f.matvec, &c.mats[2], bf(b::XN), bf(b::VV))?;
                let kst = bf(b::KST) + (g.attn_idx * 2 * g.kv_dim * 2) as u64;
                let vst = kst + (g.kv_dim * 2) as u64;
                self.lanzar_a(
                    d.f.attn_prep,
                    ((g.heads + g.kv_heads) as u32, 1, 1),
                    (256, 1, 1),
                    &[
                        Arg::P(bf(b::QF)), Arg::P(g.qn), Arg::P(g.kn), Arg::P(bf(b::KK)), Arg::P(bf(b::VV)), Arg::P(bf(b::Q)), Arg::P(bf(b::GATE)), Arg::P(kd), Arg::P(vd),
                        Arg::P(kst), Arg::P(vst), Arg::P(bf(b::DYN)), Arg::I(g.heads as i32), Arg::I(g.kv_heads as i32), Arg::I(g.hd as i32), Arg::I(g.rope_n as i32),
                        Arg::F(g.theta), Arg::F(d.eps),
                    ],
                )?;
                let group = g.heads / g.kv_heads;
                if !Self::salta("attn") {
                self.lanzar_a(
                    d.f.attn_split,
                    (g.kv_heads as u32, nsplit as u32, 1),
                    (256, 1, 1),
                    &[
                        Arg::P(bf(b::Q)), Arg::P(kd), Arg::P(vd), Arg::P(bf(b::PO)), Arg::P(bf(b::PML)), Arg::P(bf(b::DYN)), Arg::I(g.heads as i32), Arg::I(g.kv_heads as i32),
                        Arg::I(g.hd as i32), Arg::I(group as i32), Arg::I(CHUNK as i32), Arg::I(MAX_SPLIT as i32),
                    ],
                )?;
                }
                self.lanzar_a(
                    d.f.attn_combine,
                    (g.heads as u32, 1, 1),
                    (256, 1, 1),
                    &[
                        Arg::P(bf(b::PO)), Arg::P(bf(b::PML)), Arg::P(bf(b::GATE)), Arg::P(bf(b::ATT)), Arg::P(bf(b::DYN)), Arg::I(g.kv_heads as i32), Arg::I(group as i32),
                        Arg::I(g.hd as i32), Arg::I(CHUNK as i32), Arg::I(MAX_SPLIT as i32),
                    ],
                )?;
                self.dec_mm1(d.f.matvec, &c.mats[3], bf(b::ATT), bf(b::Y))?;
                add(self, bf(b::HID), bf(b::Y), h)?;
            }
            // ---- FFN MoE ----
            let m = &c.moe;
            rms(self, bf(b::HID), c.nf, bf(b::XN), h)?;
            self.dec_mm1(d.f.matvec, &m.router, bf(b::XN), bf(b::RLOG))?;
            self.lanzar_a(
                d.f.router,
                (1, 1, 1),
                (256, 1, 1),
                &[Arg::P(bf(b::RLOG)), Arg::P(bf(b::RIDX)), Arg::P(bf(b::RWT)), Arg::I(m.n_exp as i32), Arg::I(m.top_k as i32)],
            )?;
            if let Some(sh) = &m.sh {
                self.lanzar_a(d.f.pgate, (1, 1, 1), (256, 1, 1), &[Arg::P(sh.pg), Arg::P(bf(b::XN)), Arg::P(bf(b::SG)), Arg::I(hh)])?;
                self.dec_mm1(d.f.matvec, &sh.gate, bf(b::XN), bf(b::G1))?;
                self.dec_mm1(d.f.matvec, &sh.up, bf(b::XN), bf(b::U1))?;
                self.lanzar_a(d.f.swiglu, (blocks256(sh.ffn), 1, 1), (256, 1, 1), &[Arg::P(bf(b::U1)), Arg::P(bf(b::G1)), Arg::I(sh.ffn as i32)])?;
                self.dec_mm1(d.f.matvec, &sh.down, bf(b::U1), bf(b::YSH))?;
            }
            let ysh = if m.sh.is_some() { bf(b::YSH) } else { 0 };
            if m.en_gpu && !Self::salta("moe") {
                {
                    let mut v1: Vec<u64> = vec![m.tg, m.tu, bf(b::RIDX), bf(b::XN), bf(b::ACT), m.ffn as u64, h as u64, m.dg as u32 as u64, m.du as u32 as u64, m.rbg as u64, m.rbu as u64];
                    let mut p1: Vec<*mut c_void> = v1.iter_mut().map(|v| v as *mut u64 as *mut c_void).collect();
                    self.lanzar3s(d.f.gateup_w, ((m.ffn as u32).div_ceil(8), 1, m.top_k as u32), (256, 1, 1), (h * 4) as u32, &mut p1)?;
                    let mut v2: Vec<u64> = vec![m.td, bf(b::RIDX), bf(b::ACT), bf(b::YS), h as u64, m.ffn as u64, m.dd as u32 as u64, m.rbd as u64];
                    let mut p2: Vec<*mut c_void> = v2.iter_mut().map(|v| v as *mut u64 as *mut c_void).collect();
                    self.lanzar3s(d.f.down_w, ((h as u32).div_ceil(8), 1, m.top_k as u32), (256, 1, 1), (m.ffn * 4) as u32, &mut p2)?;
                }
                self.lanzar_a(
                    d.f.combine,
                    (blocks256(h), 1, 1),
                    (256, 1, 1),
                    &[Arg::P(bf(b::HID)), Arg::P(bf(b::YS)), Arg::P(bf(b::RWT)), Arg::P(ysh), Arg::P(bf(b::SG)), Arg::I(m.top_k as i32), Arg::I(hh)],
                )?;
            } else if !m.en_gpu {
                // expertos en CPU: el token se corta aquí; el host los calcula y el segmento siguiente suma el resultado
                return Ok(Some(ci));
            }
        }
        // ---- norma final y logits ----
        rms(self, bf(b::HID), d.nfin, bf(b::XN), h)?;
        if !Self::salta("lm") {
            self.dec_mm1(d.f.matvec, &d.lm, bf(b::XN), bf(b::LOGITS))?;
        }
        Ok(None)
    }
}
