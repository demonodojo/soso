//! Capa MoE (Mellum) entera en la GPU: atención con KV residente, router, expertos elegidos en el dispositivo.
//! Reutiliza los kernels de la capa de Gemma (normas, RoPE, atención) y añade los de `mo_*` (router + top-k,
//! gate/up Q4_K con SiLU, down Q8_0, combinación). Por llamada sólo viaja el `hidden` (entrada y salida).

use std::collections::HashMap;
use std::ffi::{c_int, c_void};

use soso_llm_core::gpu::{ModoMoe, MoeCapa, MoePlan};

use super::{
    ATT_CH, B_ACT, B_ATTN, B_GO, B_GS, B_HS, B_IDX, B_K, B_LG, B_ML, B_O, B_PARCIAL, B_Q, B_SC,
    B_V, B_WGT, B_XN, B_YE, CudaGpu, DYN_BYTES, DYN_CAPAS, DYN_SLOTS, GRP_TT, N_MAX, XPAD_RUST, ck,
};

/// Lo que la GPU guarda de una capa MoE: normas, router, factores de RoPE y las tablas de punteros de expertos.
pub(super) struct MoeDev {
    attn: u64,
    q: u64,
    k: u64,
    ffn: u64,
    router: u64,
    rope: u64,
    pg: u64,
    pu: u64,
    pd: u64,
    /// `ffn_down` de los expertos en Q4_K (si no, Q8_0).
    down_q4k: bool,
    /// Los expertos no viven aquí: se traen a un pool por demanda (`pg`/`pu`/`pd` valen 0).
    flujo: bool,
}

/// Cuántas capas llevan los expertos residentes y cuántos huecos tiene el pool del resto (ver `planificar_moe`).
pub(super) struct PlanGpu {
    residentes_libres: usize,
    pool_huecos: usize,
}

/// Huecos de VRAM con expertos traídos por demanda (gate, up y down seguidos), con expulsión LRU.
pub(super) struct Pool {
    base: u64,
    hueco_bytes: usize,
    offs: [usize; 3],
    caps: [usize; 3],
    claves: Vec<Option<(u32, u32)>>,
    usos: Vec<u64>,
    mapa: HashMap<(u32, u32), usize>,
    reloj: u64,
    /// Tablas de punteros de la capa en curso: gate, up y down, `n_exp` entradas cada una.
    tabla_dev: u64,
}

/// Margen para los búferes de trabajo, las tablas y los grafos de un modelo MoE.
const RESERVA_TRABAJO: usize = 512 << 20;
const ALINEA: usize = 256;

fn alinear(n: usize) -> usize {
    n.div_ceil(ALINEA) * ALINEA
}

impl CudaGpu {
    pub(in crate::cuda) fn moe_puede_capa(&self, layer: u32) -> bool {
        !self.capa.rechazadas.contains(&layer)
    }

    pub(in crate::cuda) fn moe_lista_capa(&self, layer: u32) -> bool {
        self.capa.moe.contains_key(&layer)
    }

    /// Crea las normas, el router y las tablas de expertos de la capa (los tensores ya están residentes).
    fn moe_montar(&mut self, c: &MoeCapa<'_>) -> Result<(), String> {
        let nm = c
            .normas
            .as_ref()
            .ok_or_else(|| format!("capa {}: faltan las normas de la capa MoE", c.layer))?;
        let flujo = self.capa.modos_moe.get(&c.layer) == Some(&ModoMoe::Flujo);
        let mut tablas = [0u64; 3];
        let mut down_dtype = if flujo && c.down_q4k { sosomodel::layout::DTYPE_Q4_K } else { 0u8 };
        for (i, suf) in ["ffn_gate", "ffn_up", "ffn_down"].iter().enumerate() {
            if flujo {
                break; // las tablas de un modo «en flujo» las construye cada llamada (ver `moe_flujo_tabla`)
            }
            let mut ptrs = Vec::with_capacity(c.n_exp);
            for e in 0..c.n_exp {
                let clave = format!("{}.E{e:02}.{suf}", c.prefijo);
                match self.pesos.get(&clave) {
                    Some(Some(p)) => {
                        ptrs.push(p.d);
                        if i == 2 {
                            if down_dtype != 0 && down_dtype != p.dtype {
                                return Err(format!("{clave}: los `down` de la capa mezclan dtypes"));
                            }
                            down_dtype = p.dtype;
                        }
                    }
                    _ => return Err(format!("experto {clave} no residente")),
                }
            }
            let bytes: Vec<u8> = ptrs.iter().flat_map(|p| p.to_le_bytes()).collect();
            let d = self.alloc(bytes.len())?;
            self.subir(d, &bytes)?;
            self.usado += bytes.len();
            tablas[i] = d;
        }
        let md = MoeDev {
            attn: self.subir_norma(nm.attn_norm)?,
            q: self.subir_norma(nm.q_norm)?,
            k: self.subir_norma(nm.k_norm)?,
            ffn: self.subir_norma(nm.ffn_norm)?,
            router: self.subir_norma(nm.router)?,
            rope: match nm.rope_factores {
                Some(r) => self.subir_norma(r)?,
                None => 0,
            },
            pg: tablas[0],
            pu: tablas[1],
            pd: tablas[2],
            down_q4k: down_dtype == sosomodel::layout::DTYPE_Q4_K,
            flujo,
        };
        self.capa.moe.insert(c.layer, md);
        Ok(())
    }

    /// Modo de una capa MoE. La primera vez planifica la VRAM: si el modelo cabe entero, todas residentes; si no, las
    /// primeras `residentes_libres` capas llevan los expertos residentes y el resto los trae por demanda. (`SOSO_CUDA_RESIDENTES=N`
    /// fija ese número, para probar el flujo con un modelo que sí cabe.)
    pub(in crate::cuda) fn moe_modo_capa(&mut self, layer: u32, plan: &MoePlan) -> ModoMoe {
        if let Some(m) = self.capa.modos_moe.get(&layer) {
            return *m;
        }
        if self.capa.plan_moe.is_none() {
            let pl = self.planificar_moe(plan);
            eprintln!(
                "cuda: plan MoE: {} de {} capas con expertos residentes, pool de {} huecos para el resto",
                pl.residentes_libres, plan.n_layers, pl.pool_huecos
            );
            self.capa.plan_moe = Some(pl);
        }
        let pl = self.capa.plan_moe.as_mut().unwrap();
        let modo = if pl.residentes_libres > 0 {
            pl.residentes_libres -= 1;
            ModoMoe::Residente
        } else {
            ModoMoe::Flujo
        };
        self.capa.modos_moe.insert(layer, modo);
        modo
    }

    fn planificar_moe(&self, p: &MoePlan) -> PlanGpu {
        let libre = self.presupuesto.saturating_sub(self.usado);
        let fijo = p.attn_total + p.kv_total + p.fijo + RESERVA_TRABAJO;
        let hueco = p.bytes_experto.iter().map(|b| alinear(*b)).sum::<usize>();
        let forzado = std::env::var("SOSO_CUDA_RESIDENTES").ok().and_then(|v| v.parse::<usize>().ok());
        if forzado.is_none() && libre >= fijo + p.n_layers * p.expertos_capa {
            return PlanGpu { residentes_libres: p.n_layers, pool_huecos: 0 };
        }
        let minimo = p.n_exp + 16;
        let util = libre.saturating_sub(fijo + minimo * hueco);
        let r = forzado.unwrap_or(util / p.expertos_capa.max(1)).min(p.n_layers);
        let sobra = util.saturating_sub(r * p.expertos_capa);
        let extra = (sobra / hueco.max(1)).min(3 * p.n_exp);
        PlanGpu { residentes_libres: r, pool_huecos: minimo + extra }
    }

    /// Crea el pool de expertos (la primera capa «en flujo»).
    fn crear_pool(&mut self, c: &MoeCapa<'_>) -> Result<(), String> {
        let huecos = self.capa.plan_moe.as_ref().map_or(c.n_exp + 16, |p| p.pool_huecos).max(c.n_exp + 1);
        let caps = c.bytes_experto;
        let offs = [0, alinear(caps[0]), alinear(caps[0]) + alinear(caps[1])];
        let hueco_bytes = offs[2] + alinear(caps[2]);
        let base = self.alloc(huecos * hueco_bytes)?;
        let tabla_dev = self.alloc(3 * c.n_exp * 8)?;
        self.usado += huecos * hueco_bytes + 3 * c.n_exp * 8;
        eprintln!("cuda: pool de expertos: {huecos} huecos de {} KiB ({} MiB)", hueco_bytes >> 10, (huecos * hueco_bytes) >> 20);
        self.capa.pool = Some(Pool {
            base,
            hueco_bytes,
            offs,
            caps,
            claves: vec![None; huecos],
            usos: vec![0; huecos],
            mapa: HashMap::new(),
            reloj: 0,
            tabla_dev,
        });
        Ok(())
    }

    /// Capa «en flujo», tras el router: lee qué expertos eligió cada token, trae a un hueco del pool los que falten (los
    /// menos recientes se expulsan) y deja listas las tablas de punteros que usan los kernels de expertos.
    fn moe_flujo_tabla(&mut self, c: &mut MoeCapa<'_>, p: &mut Ptrs, f: &Forma) -> Result<(), String> {
        let (n, k) = (f.n, f.k);
        if self.capa.pool.is_none() {
            self.crear_pool(c)?;
        }
        let t_sync = std::time::Instant::now();
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        self.capa.flujo_diag[2] += t_sync.elapsed().as_nanos() as u64;
        let mut idx = vec![0i32; n * k];
        ck(
            unsafe { (self.api.dtoh)(idx.as_mut_ptr() as *mut c_void, p.idx, idx.len() * 4) },
            "cuMemcpyDtoH",
        )?;
        let mut usados: Vec<usize> = Vec::with_capacity(idx.len());
        for &e in &idx {
            if e < 0 || e as usize >= c.n_exp {
                return Err(format!("el router devolvió el experto {e} (hay {})", c.n_exp));
            }
            usados.push(e as usize);
        }
        usados.sort_unstable();
        usados.dedup();
        let htod = self.api.htod;
        let mut pool = self.capa.pool.take().unwrap();
        pool.reloj += 1;
        let reloj = pool.reloj;
        let n_exp = c.n_exp;
        let mut tabla = vec![0u64; 3 * n_exp];
        let (mut aciertos, mut fallos) = (0u64, 0u64);
        let mut copiados = 0u64;
        let t_copia = std::time::Instant::now();
        let res: Result<(), String> = (|| {
            let prov = c.flujo.as_mut().ok_or_else(|| format!("capa {}: sin proveedor de expertos", c.layer))?;
            for &e in &usados {
                let clave = (c.layer, e as u32);
                let hueco = match pool.mapa.get(&clave) {
                    Some(&h) => {
                        aciertos += 1;
                        h
                    }
                    None => {
                        fallos += 1;
                        // El hueco libre o el menos reciente que no se esté usando en esta llamada.
                        let mut mejor: Option<usize> = None;
                        for h in 0..pool.claves.len() {
                            if pool.claves[h].is_none() {
                                mejor = Some(h);
                                break;
                            }
                            if pool.usos[h] != reloj && mejor.is_none_or(|m| pool.usos[h] < pool.usos[m]) {
                                mejor = Some(h);
                            }
                        }
                        let h = mejor.ok_or("el pool no tiene huecos libres para los expertos de esta llamada")?;
                        if let Some(vieja) = pool.claves[h].take() {
                            pool.mapa.remove(&vieja);
                        }
                        let (base, offs, caps) = (pool.base + (h * pool.hueco_bytes) as u64, pool.offs, pool.caps);
                        prov.experto(e, &mut |i, _dtype, bytes| {
                            if bytes.len() > caps[i] {
                                return Err(());
                            }
                            copiados += bytes.len() as u64;
                            ck(unsafe { htod(base + offs[i] as u64, bytes.as_ptr() as *const c_void, bytes.len()) }, "cuMemcpyHtoD")
                                .map_err(|_| ())
                        })
                        .map_err(|_| format!("no pude traer el experto {e} de la capa {}", c.layer))?;
                        pool.claves[h] = Some(clave);
                        pool.mapa.insert(clave, h);
                        h
                    }
                };
                pool.usos[hueco] = reloj;
                let base = pool.base + (hueco * pool.hueco_bytes) as u64;
                for i in 0..3 {
                    tabla[i * n_exp + e] = base + pool.offs[i] as u64;
                }
            }
            Ok(())
        })();
        let tabla_dev = pool.tabla_dev;
        self.capa.pool = Some(pool);
        res?;
        self.capa.flujo_diag[0] += t_copia.elapsed().as_nanos() as u64;
        self.capa.flujo_diag[1] += copiados;
        self.stats.flujo_aciertos += aciertos;
        self.stats.flujo_fallos += fallos;
        let bytes: Vec<u8> = tabla.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.subir(tabla_dev, &bytes)?;
        p.pg = tabla_dev;
        p.pu = tabla_dev + (n_exp * 8) as u64;
        p.pd = tabla_dev + (2 * n_exp * 8) as u64;
        Ok(())
    }

    /// Diagnóstico del flujo: (ms de CPU trayendo expertos, MiB copiados, ms esperando a la GPU antes de leer el router).
    pub fn flujo_diagnostico(&self) -> (f64, f64, f64) {
        let d = self.capa.flujo_diag;
        (d[0] as f64 / 1e6, d[1] as f64 / 1048576.0, d[2] as f64 / 1e6)
    }

    pub(in crate::cuda) fn moe_flujo_activo_capa(&self) -> bool {
        self.capa.plan_moe.as_ref().is_some_and(|p| p.pool_huecos > 0)
    }

    pub(in crate::cuda) fn moe_pendiente_capa(&self, layer: u32, pos0: usize, n: usize) -> bool {
        self.capa.pendiente == Some((layer, pos0, n))
    }

    /// Descarga la salida pendiente (si la hay) a `hidden`.
    pub(in crate::cuda) fn moe_volcar_capa(&mut self, hidden: &mut [f32]) -> Result<(), String> {
        let Some(_) = self.capa.pendiente.take() else {
            return Ok(());
        };
        let h = hidden.len();
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
        let ptr = self.capa.bufs[B_HS].ptr;
        ck(
            unsafe { (self.api.dtoh)(hidden.as_mut_ptr() as *mut c_void, ptr, h * 4) },
            "cuMemcpyDtoH",
        )
    }

    pub(in crate::cuda) fn moe_ejecutar(&mut self, c: &mut MoeCapa<'_>) -> Result<bool, String> {
        let (n, h, heads, kvh, hd, ffn, k) =
            (c.n, c.h, c.heads, c.kv_heads, c.head_dim, c.ffn, c.top_k);
        let (q_dim, kv_dim) = (heads * hd, kvh * hd);
        if c.continuar && self.capa.pendiente != Some((c.layer.wrapping_sub(1), c.pos0, n)) {
            return Err(format!(
                "capa {}: se pidió continuar pero la salida pendiente no es la de la capa anterior",
                c.layer
            ));
        }
        if n == 0
            || n > N_MAX
            || c.hidden.len() != n * h
            || hd > 512
            || hd % 64 != 0
            || heads % kvh != 0
            || h > 4096
            || c.layer as usize >= DYN_CAPAS
        {
            self.capa.rechazadas.insert(c.layer);
            return Ok(false);
        }
        unsafe { ck((self.api.ctx_set_current)(self.ctx), "cuCtxSetCurrent")? };
        let Some((dk, dv, seq_antes, cap)) =
            self.kv_preparar_en(c.layer, kv_dim, c.ventana, c.ctx_max, c.pos0, n)?
        else {
            return Ok(false);
        };
        if !self.capa.moe.contains_key(&c.layer) {
            self.moe_montar(c)?;
        }
        if self.capa.dyn_dev == 0 {
            self.capa.dyn_dev = self.alloc(DYN_CAPAS * DYN_BYTES)?;
            self.usado += DYN_CAPAS * DYN_BYTES;
            if let Some(g) = &self.grafo {
                let mut p: *mut c_void = std::ptr::null_mut();
                ck(
                    unsafe { (g.host_alloc)(&mut p, DYN_SLOTS * DYN_BYTES) },
                    "cuMemAllocHost",
                )?;
                self.capa.pinned = p;
            }
        }
        let (n_attn, n_q, n_k, n_ffn, d_router, n_rope, pg, pu, pd, down_q4k, flujo) = {
            let m = &self.capa.moe[&c.layer];
            (m.attn, m.q, m.k, m.ffn, m.router, m.rope, m.pg, m.pu, m.pd, m.down_q4k, m.flujo)
        };
        // Tramo de contexto (potencia de dos): fija la malla de las puntuaciones y los trozos de P·V, de modo que el
        // mismo grafo sirve para todos los tokens del tramo (los kernels leen la posición exacta de `dyn`).
        let total_max = seq_antes + n;
        let tramo = total_max
            .next_power_of_two()
            .max(64)
            .min(cap.max(total_max));
        let s_split = {
            let len = if c.ventana > 0 {
                tramo.min(c.ventana)
            } else {
                tramo
            };
            len.div_ceil(256).clamp(1, 16)
        };
        let d_hs = self.buf(B_HS, n * h)?;
        let d_xn = self.buf(B_XN, n * h)?;
        let d_q = self.buf(B_Q, n * q_dim)?;
        let d_k = self.buf(B_K, n * kv_dim)?;
        let d_v = self.buf(B_V, n * kv_dim)?;
        let d_attn = self.buf(B_ATTN, n * q_dim)?;
        let d_o = self.buf(B_O, n * h)?;
        // Atención por trozos de KV (hd = 128, ≤ 8 cabezas de consulta por cabeza KV); si no, el camino por cabeza.
        let usar_split =
            hd == 128 && heads / kvh <= 8 && std::env::var_os("SOSO_CUDA_ATT_VIEJA").is_none();
        let s_att = {
            let len = if c.ventana > 0 {
                tramo.min(c.ventana)
            } else {
                tramo
            };
            len.div_ceil(ATT_CH).max(1)
        };
        let d_sc = if usar_split {
            0
        } else {
            self.buf(B_SC, n * heads * cap)?
        };
        let (d_par, d_ml) = if usar_split {
            (
                self.buf(B_PARCIAL, n * kvh * s_att * 8 * hd)?,
                self.buf(B_ML, n * kvh * s_att * 8 * 2)?,
            )
        } else {
            (self.buf(B_PARCIAL, n * heads * s_split * hd)?, 0)
        };
        let d_lg = self.buf(B_LG, n * c.n_exp)?;
        // Expertos agrupados (varios tokens por lectura de pesos) con bloques de >= 4 tokens y x cabiendo en shared.
        let agrupar = n >= 4
            && GRP_TT * (h / 32 * XPAD_RUST) * 4 <= 49152
            && c.n_exp <= 256
            && std::env::var_os("SOSO_CUDA_SIN_GRUPOS").is_none();
        let d_gs = self.buf(B_GS, c.n_exp + 1)?;
        let d_go = self.buf(B_GO, n * k)?;
        let d_idx = self.buf(B_IDX, n * k)?;
        let d_wgt = self.buf(B_WGT, n * k)?;
        let d_act = self.buf(B_ACT, n * k * ffn)?;
        let d_ye = self.buf(B_YE, n * k * h)?;
        if let Some((a, _)) = self.capa.perfil {
            ck(
                unsafe { (self.api.ev_record)(a, std::ptr::null_mut()) },
                "cuEventRecord",
            )?;
        }
        if !c.continuar {
            self.subir_f32(d_hs, c.hidden)?;
        }
        // Parámetros que cambian por token, en el dispositivo (los lee el grafo).
        let dyn_ptr = self.capa.dyn_dev + (c.layer as usize * DYN_BYTES) as u64;
        let dinamicos = [c.pos0 as i32, seq_antes as i32, 0, 0];
        let bytes =
            unsafe { std::slice::from_raw_parts(dinamicos.as_ptr() as *const u8, DYN_BYTES) };
        let con_grafo = self.grafo.is_some()
            && !flujo
            && self.capa.perfil.is_none()
            && !self.capa.depurar
            && !self.capa.pinned.is_null();
        if con_grafo {
            let slot = self.capa.slot % DYN_SLOTS;
            self.capa.slot += 1;
            let g = self.grafo.as_ref().unwrap();
            unsafe {
                let dst = (self.capa.pinned as *mut u8).add(slot * DYN_BYTES);
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, DYN_BYTES);
                ck(
                    (g.htod_async)(dyn_ptr, dst as *const c_void, DYN_BYTES, self.stream),
                    "cuMemcpyHtoDAsync",
                )?;
            }
        } else {
            self.subir(dyn_ptr, bytes)?;
        }

        let mut p = Ptrs {
            hs: d_hs,
            xn: d_xn,
            q: d_q,
            k: d_k,
            v: d_v,
            attn: d_attn,
            o: d_o,
            sc: d_sc,
            par: d_par,
            lg: d_lg,
            idx: d_idx,
            wgt: d_wgt,
            act: d_act,
            ye: d_ye,
            dk,
            dv,
            n_attn,
            n_q,
            n_k,
            n_ffn,
            router: d_router,
            rope: n_rope,
            pg,
            pu,
            pd,
            dyn_ptr,
            ml: d_ml,
            gs: d_gs,
            go: d_go,
        };
        let forma = Forma {
            n,
            h,
            heads,
            kvh,
            hd,
            ffn,
            k,
            kv_dim,
            cap,
            s_split,
            tramo,
            seq_antes,
            usar_split,
            s_att,
            agrupar,
            down_q4k,
        };
        if flujo {
            // Atención y router → qué expertos hacen falta → traerlos al pool → expertos y combinación.
            self.moe_cuerpo_atencion(c, &p, &forma)?;
            self.moe_flujo_tabla(c, &mut p, &forma)?;
            self.moe_cuerpo_expertos(c, &p, &forma)?;
        } else if con_grafo {
            if self.capa.grafos_gen != self.capa.gen_bufs {
                self.grafos_olvidar();
                self.capa.grafos_gen = self.capa.gen_bufs;
            }
            let clave = (c.layer, n, tramo);
            if !self.capa.grafos.contains_key(&clave) {
                let g = self.grafo.as_ref().unwrap();
                ck(
                    unsafe { (g.capture_begin)(self.stream, 2) },
                    "cuStreamBeginCapture",
                )?;
                self.capturando = true;
                let r = self.moe_cuerpo(c, &p, &forma);
                self.capturando = false;
                let g = self.grafo.as_ref().unwrap();
                let mut grafo: *mut c_void = std::ptr::null_mut();
                let fin = ck(
                    unsafe { (g.capture_end)(self.stream, &mut grafo) },
                    "cuStreamEndCapture",
                );
                r?;
                fin?;
                let mut exec: *mut c_void = std::ptr::null_mut();
                let inst = ck(
                    unsafe { (g.graph_instantiate)(&mut exec, grafo, 0) },
                    "cuGraphInstantiate",
                );
                unsafe { (g.graph_destroy)(grafo) };
                inst?;
                self.capa.grafos.insert(clave, exec);
            }
            let exec = self.capa.grafos[&clave];
            let g = self.grafo.as_ref().unwrap();
            ck(
                unsafe { (g.graph_launch)(exec, self.stream) },
                "cuGraphLaunch",
            )?;
        } else {
            self.moe_cuerpo(c, &p, &forma)?;
        }

        if c.encadenar && self.capa.perfil.is_none() {
            // La capa siguiente sigue desde el dispositivo: sin sincronizar ni descargar.
            self.capa.pendiente = Some((c.layer, c.pos0, n));
        } else {
            ck(unsafe { (self.api.sync)() }, "cuCtxSynchronize")?;
            self.perfil_cerrar()?;
            let salida = unsafe {
                std::slice::from_raw_parts_mut(c.hidden.as_mut_ptr() as *mut u8, n * h * 4)
            };
            ck(
                unsafe { (self.api.dtoh)(salida.as_mut_ptr() as *mut c_void, d_hs, salida.len()) },
                "cuMemcpyDtoH",
            )?;
            self.capa.pendiente = None;
        }
        let kv = self.capa.kvs.get_mut(&c.layer).unwrap();
        kv.stored += n;
        c.expulsadas = kv.dropped;
        self.stats.launches += 1;
        Ok(true)
    }

    /// Destruye todos los grafos capturados (los búferes que referencian cambiaron de sitio).
    fn grafos_olvidar(&mut self) {
        if let Some(g) = &self.grafo {
            for (_, e) in self.capa.grafos.drain() {
                unsafe { (g.graph_exec_destroy)(e) };
            }
        }
    }

    /// Todos los lanzamientos de la capa, del `hidden` de entrada al de salida. Sólo hace lanzamientos y copias
    /// entre búferes ya reservados: es lo que se captura en un grafo CUDA.
    fn moe_cuerpo(&mut self, c: &MoeCapa<'_>, p: &Ptrs, f: &Forma) -> Result<(), String> {
        self.moe_cuerpo_atencion(c, p, f)?;
        self.moe_cuerpo_expertos(c, p, f)
    }

    /// Primera mitad de la capa: atención, residual y router (deja en `idx`/`wgt` los expertos elegidos).
    #[allow(unused_variables)]
    fn moe_cuerpo_atencion(&mut self, c: &MoeCapa<'_>, p: &Ptrs, f: &Forma) -> Result<(), String> {
        let Forma {
            n,
            h,
            heads,
            kvh,
            hd,
            ffn,
            k,
            kv_dim,
            cap,
            s_split,
            tramo,
            seq_antes,
            usar_split,
            s_att,
            agrupar,
            down_q4k,
        } = *f;
        // ---------------------------------------------------------------- atención
        self.rms_rows(p.hs, p.n_attn, p.xn, n, h, c.eps)?;
        self.marca("k rms")?;
        if n == 1 {
            self.mm_qkv(c.clave_q, c.clave_k, c.clave_v, p.xn, p.q, p.k, p.v, n)?;
        } else {
            self.mm_n(c.clave_q, p.xn, p.q, n)?;
            self.mm_n(c.clave_k, p.xn, p.k, n)?;
            self.mm_n(c.clave_v, p.xn, p.v, n)?;
        }
        self.marca("k qkv")?;
        if c.qk_norm {
            self.rms_rows(p.q, p.n_q, p.q, n * heads, hd, c.eps)?;
            self.rms_rows(p.k, p.n_k, p.k, n * kvh, hd, c.eps)?;
        }
        self.marca("k qk-norm")?;
        for (x, nh) in [(p.q, heads), (p.k, kvh)] {
            self.rope_dev(
                x,
                nh,
                hd,
                n,
                c.pos0,
                c.theta,
                p.rope,
                c.rope_mscale,
                p.dyn_ptr,
            )?;
        }
        {
            let (mut a0, mut a1, mut a2, mut a3, mut a4, mut a5, mut a6, mut a7) = (
                p.k,
                p.v,
                p.dk,
                p.dv,
                kv_dim as c_int,
                seq_antes as c_int,
                n as c_int,
                p.dyn_ptr,
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
        self.marca("k rope+kv_append")?;
        if usar_split {
            self.atencion_split_dev(
                p.q, p.dk, p.dv, p.par, p.ml, p.attn, n, heads, kvh, kv_dim, seq_antes, c.ventana,
                c.escala, s_att, p.dyn_ptr,
            )?;
        } else {
            self.atencion_dev(
                p.q, p.dk, p.dv, p.sc, p.par, p.attn, n, heads, kvh, hd, kv_dim, seq_antes,
                c.ventana, c.escala, cap, s_split, p.dyn_ptr, tramo,
            )?;
        }
        self.marca("k atención(4)")?;
        self.mm_n(c.clave_o, p.attn, p.o, n)?;
        self.marca("k o-proj")?;
        self.add_scale(p.hs, p.o, p.hs, n * h, 1.0)?;
        self.marca("k add")?;

        // ---------------------------------------------------------------- MoE
        self.rms_rows(p.hs, p.n_ffn, p.xn, n, h, c.eps)?;
        self.marca("k rms ffn")?;
        {
            let (mut a0, mut a1, mut a2) = (p.xn, p.router, p.lg);
            let (mut a3, mut a4) = (h as c_int, c.n_exp as c_int);
            self.lanzar3(
                self.capa.fns.mo_router_logits,
                ((c.n_exp as u32).div_ceil(8), n as u32, 1),
                (256, 1, 1),
                &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4)],
            )?;
            let (mut b0, mut b1, mut b2) = (p.lg, p.idx, p.wgt);
            let (mut b3, mut b4) = (c.n_exp as c_int, k as c_int);
            self.lanzar3(
                self.capa.fns.mo_router_topk,
                (n as u32, 1, 1),
                (64, 1, 1),
                &mut [a!(b0), a!(b1), a!(b2), a!(b3), a!(b4)],
            )?;
        }
        self.marca("k router(2)")?;
        Ok(())
    }

    /// Segunda mitad de la capa: los expertos elegidos por el router y la combinación con el residual.
    #[allow(unused_variables)]
    fn moe_cuerpo_expertos(&mut self, c: &MoeCapa<'_>, p: &Ptrs, f: &Forma) -> Result<(), String> {
        let Forma {
            n,
            h,
            heads,
            kvh,
            hd,
            ffn,
            k,
            kv_dim,
            cap,
            s_split,
            tramo,
            seq_antes,
            usar_split,
            s_att,
            agrupar,
            down_q4k,
        } = *f;
        let slots = (n * k) as u32;
        if agrupar {
            {
                let (mut a0, mut a1, mut a2, mut a3, mut a4) =
                    (p.idx, p.gs, p.go, (n * k) as c_int, c.n_exp as c_int);
                self.lanzar3(
                    self.capa.fns.grp,
                    (1, 1, 1),
                    (256, 1, 1),
                    &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4)],
                )?;
            }
            {
                let (mut a0, mut a1, mut a2, mut a3, mut a4, mut a5) =
                    (p.xn, p.pg, p.pu, p.gs, p.go, p.act);
                let (mut a6, mut a7, mut a8) = (h as c_int, ffn as c_int, k as c_int);
                self.lanzar3s(
                    self.capa.fns.gateup_grp,
                    ((ffn as u32).div_ceil(16), c.n_exp as u32, 1),
                    (256, 1, 1),
                    (GRP_TT * (h / 32 * XPAD_RUST) * 4) as u32,
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
                    ],
                )?;
            }
            self.marca("k gate/up")?;
            {
                let (mut a0, mut a1, mut a2, mut a3, mut a4) = (p.act, p.pd, p.gs, p.go, p.ye);
                let (mut a5, mut a6) = (h as c_int, ffn as c_int);
                if down_q4k {
                    self.lanzar3s(
                        self.capa.fns.down_grp_q4k,
                        ((h as u32).div_ceil(16), c.n_exp as u32, 1),
                        (256, 1, 1),
                        (GRP_TT * (ffn / 32 * XPAD_RUST) * 4) as u32,
                        &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4), a!(a5), a!(a6)],
                    )?;
                } else {
                    self.lanzar3(
                        self.capa.fns.down_grp,
                        ((h as u32).div_ceil(32), c.n_exp as u32, 1),
                        (256, 1, 1),
                        &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4), a!(a5), a!(a6)],
                    )?;
                }
            }
        } else {
            {
                let (mut a0, mut a1, mut a2, mut a3, mut a4) = (p.xn, p.pg, p.pu, p.idx, p.act);
                let (mut a5, mut a6, mut a7) = (h as c_int, ffn as c_int, k as c_int);
                self.lanzar3(
                    self.capa.fns.mo_gateup,
                    ((ffn as u32).div_ceil(32), slots, 1),
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
            self.marca("k gate/up")?;
            {
                let (mut a0, mut a1, mut a2, mut a3) = (p.act, p.pd, p.idx, p.ye);
                let (mut a4, mut a5) = (h as c_int, ffn as c_int);
                self.lanzar3(
                    if down_q4k { self.capa.fns.down_q4k } else { self.capa.fns.mo_down },
                    ((h as u32).div_ceil(32), slots, 1),
                    (256, 1, 1),
                    &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4), a!(a5)],
                )?;
            }
        }
        self.marca("k down")?;
        {
            let (mut a0, mut a1, mut a2) = (p.hs, p.ye, p.wgt);
            let (mut a3, mut a4) = (h as c_int, k as c_int);
            self.lanzar3(
                self.capa.fns.mo_combine,
                ((h as u32).div_ceil(256), n as u32, 1),
                (256, 1, 1),
                &mut [a!(a0), a!(a1), a!(a2), a!(a3), a!(a4)],
            )?;
        }
        self.marca("k combine")?;
        Ok(())
    }
}

/// Punteros de dispositivo de una llamada (búferes de trabajo, KV, normas y tablas de la capa).
struct Ptrs {
    hs: u64,
    xn: u64,
    q: u64,
    k: u64,
    v: u64,
    attn: u64,
    o: u64,
    sc: u64,
    par: u64,
    lg: u64,
    idx: u64,
    wgt: u64,
    act: u64,
    ye: u64,
    dk: u64,
    dv: u64,
    n_attn: u64,
    n_q: u64,
    n_k: u64,
    n_ffn: u64,
    router: u64,
    rope: u64,
    pg: u64,
    pu: u64,
    pd: u64,
    dyn_ptr: u64,
    ml: u64,
    gs: u64,
    go: u64,
}

#[derive(Clone, Copy)]
struct Forma {
    n: usize,
    h: usize,
    heads: usize,
    kvh: usize,
    hd: usize,
    ffn: usize,
    k: usize,
    kv_dim: usize,
    cap: usize,
    s_split: usize,
    tramo: usize,
    seq_antes: usize,
    usar_split: bool,
    s_att: usize,
    agrupar: bool,
    down_q4k: bool,
}
