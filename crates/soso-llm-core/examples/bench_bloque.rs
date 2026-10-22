//! T22/T82: coste del prefill por bloques de una capa con las dimensiones de un modelo real
//! (pesos Q4_K sintéticos), desglosado por fases y con 1..N hilos.
//!
//! ```sh
//! RUSTFLAGS="-C target-feature=+avx2,+fma" cargo run --release -p soso-llm-core --features std \
//!   --example bench_bloque -- [14b|7b] [tokens] [hilos] [h2o]
//! ```
//! `h2o` activa el planificador con H2O (la ruta de atención que usa el guest).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use soso_llm_core::generation::{GenerationOptions, NoCancelObserver};
use soso_llm_core::layer::{
    perfil_activar, perfil_fases_ns, TensorView, NOMBRES_FASES, N_FASES,
};
use soso_llm_core::parallel::{RowParallel, Sequential};
use soso_llm_core::plan::{MemSnapshot, ResourcePlanner};
use soso_llm_core::runtime::Runtime;
use soso_llm_core::sample::Sampler;
use soso_llm_core::layer::TensorSource;
use sosomodel::index::{make_f32_entry, make_q4_k_entry, make_q8_0_entry, TensorIndex};
use sosomodel::layout::{
    DTYPE_F32, DTYPE_Q4_K, DTYPE_Q8_0, Q4_K_BLOCK_BYTES, Q4_K_BLOCK_ELEMS, Q8_0_BLOCK_BYTES, Q8_0_BLOCK_ELEMS,
};
use sosomodel::manifest::Manifest;

struct Fuente {
    t: BTreeMap<String, (u8, Vec<u8>)>,
}

impl TensorSource for Fuente {
    fn load_f32(&mut self, name: &str, out: &mut [f32]) -> Result<(), ()> {
        let (d, b) = self.t.get(name).ok_or(())?;
        if *d != DTYPE_F32 || b.len() != out.len() * 4 {
            return Err(());
        }
        for (o, c) in out.iter_mut().zip(b.chunks_exact(4)) {
            *o = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        }
        Ok(())
    }
    fn load_f32_range(&mut self, name: &str, off: usize, out: &mut [f32]) -> Result<(), ()> {
        let (d, b) = self.t.get(name).ok_or(())?;
        if *d != DTYPE_F32 || (off + out.len()) * 4 > b.len() {
            return Err(());
        }
        for (i, o) in out.iter_mut().enumerate() {
            let c = &b[(off + i) * 4..(off + i) * 4 + 4];
            *o = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        }
        Ok(())
    }
    fn tensor_view(&mut self, name: &str) -> Result<TensorView<'_>, ()> {
        let (d, b) = self.t.get(name).ok_or(())?;
        let elems = match *d {
            DTYPE_F32 => b.len() / 4,
            DTYPE_Q8_0 => b.len() / Q8_0_BLOCK_BYTES * Q8_0_BLOCK_ELEMS,
            _ => b.len() / Q4_K_BLOCK_BYTES * Q4_K_BLOCK_ELEMS,
        };
        Ok(TensorView { bytes: b, dtype: *d, elems })
    }
}

fn f32s(n: usize, v: f32) -> Vec<u8> {
    (0..n).flat_map(|i| (v + (i % 7) as f32 * 0.01).to_le_bytes()).collect()
}

fn q8_0(rows: usize, cols: usize, semilla: u32) -> Vec<u8> {
    let mut s = semilla;
    let mut b = vec![0u8; rows * (cols / Q8_0_BLOCK_ELEMS) * Q8_0_BLOCK_BYTES];
    for x in b.iter_mut() {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *x = (s >> 24) as u8;
    }
    for blk in b.chunks_exact_mut(Q8_0_BLOCK_BYTES) {
        blk[..4].copy_from_slice(&0.004f32.to_le_bytes()); // escala f32
    }
    b
}

fn q4k(rows: usize, cols: usize, semilla: u32) -> Vec<u8> {
    let mut s = semilla;
    let mut b = vec![0u8; rows * (cols / Q4_K_BLOCK_ELEMS) * Q4_K_BLOCK_BYTES];
    for x in b.iter_mut() {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *x = (s >> 24) as u8;
    }
    for blk in b.chunks_exact_mut(Q4_K_BLOCK_BYTES) {
        blk[0] = 0x00;
        blk[1] = 0x1c; // d ≈ 0,0039
        blk[2] = 0x00;
        blk[3] = 0x18;
    }
    b
}

/// Pool de hilos con espera activa (equivale a lo que da `ncpu` núcleos, sin latencia de futex).
struct Pool {
    n: usize,
    epoca: Arc<AtomicU32>,
    hecho: Arc<AtomicUsize>,
    filas: Arc<AtomicUsize>,
    f: Arc<AtomicUsize>,
    parar: Arc<AtomicBool>,
}

fn franja(filas: usize, i: usize, n: usize) -> (usize, usize) {
    (filas * i / n, filas * (i + 1) / n)
}

impl Pool {
    fn new(n: usize) -> Self {
        let p = Pool {
            n,
            epoca: Arc::new(AtomicU32::new(0)),
            hecho: Arc::new(AtomicUsize::new(0)),
            filas: Arc::new(AtomicUsize::new(0)),
            f: Arc::new(AtomicUsize::new(0)),
            parar: Arc::new(AtomicBool::new(false)),
        };
        for i in 1..n {
            let (epoca, hecho, filas, f, parar) =
                (p.epoca.clone(), p.hecho.clone(), p.filas.clone(), p.f.clone(), p.parar.clone());
            std::thread::spawn(move || {
                let mut ultimo = 0u32;
                loop {
                    while epoca.load(Ordering::Acquire) == ultimo {
                        if parar.load(Ordering::Acquire) {
                            return;
                        }
                        std::hint::spin_loop();
                    }
                    ultimo = epoca.load(Ordering::Acquire);
                    let (r0, r1) = franja(filas.load(Ordering::Acquire), i, n);
                    let fp = f.load(Ordering::Acquire) as *const &(dyn Fn(usize, usize) + Sync);
                    if r0 < r1 {
                        unsafe { (*fp)(r0, r1) };
                    }
                    hecho.fetch_add(1, Ordering::AcqRel);
                }
            });
        }
        p
    }
}

impl RowParallel for Pool {
    fn for_rows(&self, rows: usize, f: &(dyn Fn(usize, usize) + Sync)) {
        if self.n <= 1 || rows == 0 {
            f(0, rows);
            return;
        }
        let fref: &(dyn Fn(usize, usize) + Sync) = f;
        self.filas.store(rows, Ordering::Release);
        self.f.store(&fref as *const _ as usize, Ordering::Release);
        self.hecho.store(0, Ordering::Release);
        self.epoca.fetch_add(1, Ordering::AcqRel);
        let (r0, r1) = franja(rows, 0, self.n);
        f(r0, r1);
        while self.hecho.load(Ordering::Acquire) < self.n - 1 {
            std::hint::spin_loop();
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.parar.store(true, Ordering::Release);
    }
}

fn reloj_ns() -> u64 {
    use std::sync::OnceLock;
    static T0: OnceLock<Instant> = OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let modelo = a.first().map(String::as_str).unwrap_or("14b");
    let tokens: usize = a.get(1).and_then(|v| v.parse().ok()).unwrap_or(1024);
    let hilos: usize = a.get(2).and_then(|v| v.parse().ok()).unwrap_or(1);
    let con_h2o = a.get(3).is_some_and(|v| v == "h2o");
    // (hidden, cabezas, cabezas KV, head_dim, ffn)
    let (h, heads, kvh, hd, ffn) = match modelo {
        "7b" => (3584usize, 28usize, 4usize, 128usize, 18944usize),
        _ => (5120, 40, 8, 128, 13824),
    };
    let capas = 2u32;
    let vocab = 256u32;

    let mut m = Manifest::tiny("bench");
    m.vocab_size = vocab;
    m.hidden_dim = h as u32;
    m.num_layers = capas;
    m.num_heads = heads as u32;
    m.num_kv_heads = kvh as u32;
    m.ffn_dim = ffn as u32;
    m.max_seq = 131_072;
    m.rope_theta = 1_000_000.0;
    m.prefetch.clear();

    let mut idx = TensorIndex::default();
    let mut t: BTreeMap<String, (u8, Vec<u8>)> = BTreeMap::new();
    let id = std::cell::Cell::new(0u32);
    let f32t = |idx: &mut TensorIndex, t: &mut BTreeMap<_, _>, name: &str, shape: &[u32], v: f32| {
        let n: usize = shape.iter().map(|&d| d as usize).product();
        idx.entries.push(make_f32_entry(id.get(), name, &format!("{name}.tensor"), 0, shape));
        t.insert(name.to_string(), (DTYPE_F32, f32s(n, v)));
        id.set(id.get() + 1);
    };
    f32t(&mut idx, &mut t, "embed", &[vocab, h as u32], 0.01);
    for l in 0..capas {
        let p = format!("L{l:02}");
        f32t(&mut idx, &mut t, &format!("{p}.attn_norm"), &[h as u32], 1.0);
        f32t(&mut idx, &mut t, &format!("{p}.ffn_norm"), &[h as u32], 1.0);
    }
    let q4 = |idx: &mut TensorIndex, t: &mut BTreeMap<_, _>, name: &str, rows: usize, cols: usize| {
        idx.entries.push(make_q4_k_entry(id.get(), name, &format!("{name}.tensor"), 0, &[rows as u32, cols as u32]));
        t.insert(name.to_string(), (DTYPE_Q4_K, q4k(rows, cols, id.get().wrapping_mul(2654435761))));
        id.set(id.get() + 1);
    };
    // Como el 14B real (Q4_K_M convertido): la mitad de las capas llevan `attn_v` y `ffn_down` en
    // Q8_0 (eran Q6_K) y el resto todo en Q4_K. `SOLO_Q4K=1` las deja todas en Q4_K.
    let solo_q4k = std::env::var_os("SOLO_Q4K").is_some();
    for l in 0..capas {
        let p = format!("L{l:02}");
        let q8_capa = l % 2 == 0 && !solo_q4k;
        q4(&mut idx, &mut t, &format!("{p}.attn_q"), heads * hd, h);
        q4(&mut idx, &mut t, &format!("{p}.attn_k"), kvh * hd, h);
        for (nombre, filas, cols) in [("attn_v", kvh * hd, h), ("ffn_down", h, ffn)] {
            let name = format!("{p}.{nombre}");
            if q8_capa {
                idx.entries.push(make_q8_0_entry(id.get(), &name, &format!("{name}.tensor"), 0, &[filas as u32, cols as u32]));
                t.insert(name, (DTYPE_Q8_0, q8_0(filas, cols, id.get().wrapping_mul(2654435761))));
                id.set(id.get() + 1);
            } else {
                q4(&mut idx, &mut t, &name, filas, cols);
            }
        }
        q4(&mut idx, &mut t, &format!("{p}.attn_output"), h, heads * hd);
        q4(&mut idx, &mut t, &format!("{p}.ffn_up"), ffn, h);
        q4(&mut idx, &mut t, &format!("{p}.ffn_gate"), ffn, h);
    }
    let mut fuente = Fuente { t };
    let mut rt = Runtime::new(m.clone(), idx, 0, 0);
    if con_h2o {
        let mem = MemSnapshot { total_frames: 14 * 1024 * 1024 * 1024 / 4096, free_frames: 12 * 1024 * 1024 * 1024 / 4096, reclaimable_frames: 0 };
        let pl = ResourcePlanner::new(&m, &rt.index, mem, 0, false);
        println!("planificador: kv={:?} h2o={} sparse={}", pl.kv_dtype(), pl.use_h2o(), pl.use_sparse_attn(1000));
        rt.set_planner(pl);
    }
    let pool = Pool::new(hilos);
    let par: &dyn RowParallel = if hilos > 1 { &pool } else { &Sequential };
    let prompt: Vec<u32> = (0..tokens).map(|i| (i as u32 * 7 + 3) % vocab).collect();

    perfil_activar(Some(reloj_ns));
    let t0 = Instant::now();
    let mut sampler = Sampler::greedy();
    let mut obs = NoCancelObserver;
    rt.generate_stream_par_observed(
        &mut fuente,
        &prompt,
        GenerationOptions::new(1, vec![]),
        &mut sampler,
        &mut obs,
        Some(par),
        &mut None,
        &mut None,
    )
    .expect("generar");
    let total = t0.elapsed().as_secs_f64();
    let f = perfil_fases_ns();
    perfil_activar(None);
    let suma: u64 = f.iter().sum();
    let flops_mm = 2.0 * tokens as f64 * capas as f64 * ((heads * hd * h) as f64 * 2.0 + (kvh * hd * h) as f64 * 2.0 + 3.0 * (h * ffn) as f64);
    println!(
        "{modelo} {capas} capas · {tokens} tokens · {hilos} hilo(s){} · bloques {} · total {:.2} s ({:.1} tok/s por capa-equivalente {:.0})",
        if con_h2o { " · H2O" } else { "" },
        rt.tokens_en_bloque,
        total,
        tokens as f64 / total,
        tokens as f64 / total * capas as f64
    );
    for i in 0..N_FASES {
        println!("  {:<12} {:>8.1} ms  {:>5.1}%", NOMBRES_FASES[i], f[i] as f64 / 1e6, 100.0 * f[i] as f64 / suma.max(1) as f64);
    }
    println!("  matvec {:.1} GFLOP/s efectivos (qkv+o+ffn)", flops_mm / ((f[0] + f[2] + f[3] + f[5]) as f64 / 1e9) / 1e9);
}
