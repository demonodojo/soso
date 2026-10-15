//! La capa de Gemma entera en la GPU (G9) frente a la de CPU, con un modelo diminuto sintético (sin pesos
//! reales): una capa local (ventana corta, QK-norm, V-norm) y una global (V = K, factores de RoPE). Se compara
//! el `hidden` tras cada token y tras cada bloque, pasando de la ventana, de la expulsión de filas y de un
//! rebobinado (reutilización de prefijo). Sin CUDA se salta.
#![cfg(feature = "std")]

use std::collections::HashMap;

use soso_llm_api::cuda::CudaGpu;
use soso_llm_core::gemma::forward_layer_gemma;
use soso_llm_core::gpu::GpuDispatch;
use soso_llm_core::kv::LayerKv;
use soso_llm_core::layer::{LayerExecutor, LayerScratch, TensorSource, TensorView};
use sosomodel::layout::{DTYPE_F32, DTYPE_Q4_0};
use sosomodel::manifest::{
    FLAG_GEGLU, FLAG_LAYER_SCALE, FLAG_QK_NORM, FLAG_ROPE_FACTORS, FLAG_ROPE_NEOX,
    FLAG_SANDWICH_NORM, FLAG_V_FROM_K, FLAG_V_NORM, LayerSpec, Manifest, ManifestExt,
};

fn azar(e: &mut u64) -> u32 {
    *e = e
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*e >> 33) as u32
}

/// Matriz Q4_0 aleatoria con escalas pequeñas (para que las activaciones no exploten).
fn q4_0(rows: usize, cols: usize, e: &mut u64) -> Vec<u8> {
    const ESCALAS: [u16; 3] = [0x1c00, 0x2000, 0x2400]; // ~0.0039, 0.0078, 0.0156
    let mut v = Vec::new();
    for _ in 0..rows * cols / 32 {
        v.extend_from_slice(&ESCALAS[azar(e) as usize % 3].to_le_bytes());
        for _ in 0..16 {
            v.push(azar(e) as u8);
        }
    }
    v
}

#[derive(Default)]
struct Fuente {
    q4: HashMap<String, Vec<u8>>,
    f32s: HashMap<String, Vec<f32>>,
    vistas: HashMap<String, (usize, u8)>,
}

impl TensorSource for Fuente {
    fn load_f32(&mut self, name: &str, out: &mut [f32]) -> Result<(), ()> {
        let v = self.f32s.get(name).ok_or(())?;
        if v.len() != out.len() {
            return Err(());
        }
        out.copy_from_slice(v);
        Ok(())
    }
    fn load_f32_range(&mut self, name: &str, off: usize, out: &mut [f32]) -> Result<(), ()> {
        let v = self.f32s.get(name).ok_or(())?;
        out.copy_from_slice(&v[off..off + out.len()]);
        Ok(())
    }
    fn tensor_view(&mut self, name: &str) -> Result<TensorView<'_>, ()> {
        let b = self.q4.get(name).ok_or(())?;
        let (elems, dtype) = self.vistas[name];
        Ok(TensorView {
            bytes: b,
            dtype,
            elems,
        })
    }
}

const H: usize = 256;
const FFN: usize = 512;

fn construir() -> (Manifest, Fuente) {
    let mut m = Manifest::tiny("capa-sintetica");
    m.hidden_dim = H as u32;
    m.num_layers = 2;
    m.num_heads = 4;
    m.num_kv_heads = 2;
    m.ffn_dim = FFN as u32;
    m.max_seq = 4096;
    m.rms_eps = 1e-6;
    let comunes = FLAG_SANDWICH_NORM | FLAG_GEGLU | FLAG_LAYER_SCALE | FLAG_QK_NORM | FLAG_V_NORM;
    m.layers = vec![
        LayerSpec {
            num_heads: 4,
            num_kv_heads: 2,
            v_head_dim: 64,
            flags: comunes | FLAG_ROPE_NEOX,
            ..LayerSpec::default()
        },
        LayerSpec {
            num_heads: 4,
            num_kv_heads: 1,
            v_head_dim: 128,
            flags: comunes | FLAG_ROPE_NEOX | FLAG_V_FROM_K | FLAG_ROPE_FACTORS,
            ..LayerSpec::default()
        },
    ];
    m.ext = ManifestExt {
        attn_scale: 1.0,
        window: vec![24, 0],
        rope_theta: vec![10_000.0, 1_000_000.0],
        ..ManifestExt::default()
    };
    let mut f = Fuente::default();
    let mut e = 99u64;
    for (l, (heads, kvh, hd)) in [(4usize, 2usize, 64usize), (4, 1, 128)]
        .into_iter()
        .enumerate()
    {
        let p = format!("L{l:02}");
        let mut q4 = |n: &str, rows: usize, cols: usize, f: &mut Fuente, e: &mut u64| {
            f.q4.insert(format!("{p}.{n}"), q4_0(rows, cols, e));
            f.vistas
                .insert(format!("{p}.{n}"), (rows * cols, DTYPE_Q4_0));
        };
        q4("attn_q", heads * hd, H, &mut f, &mut e);
        q4("attn_k", kvh * hd, H, &mut f, &mut e);
        if l == 0 {
            q4("attn_v", kvh * hd, H, &mut f, &mut e);
        }
        q4("attn_output", H, heads * hd, &mut f, &mut e);
        q4("ffn_up", FFN, H, &mut f, &mut e);
        q4("ffn_gate", FFN, H, &mut f, &mut e);
        q4("ffn_down", H, FFN, &mut f, &mut e);
        let mut vec = |n: &str, len: usize, f: &mut Fuente, e: &mut u64| {
            let v: Vec<f32> = (0..len)
                .map(|_| 0.5 + (azar(e) % 1000) as f32 / 1000.0)
                .collect();
            f.f32s.insert(format!("{p}.{n}"), v);
        };
        vec("attn_norm", H, &mut f, &mut e);
        vec("post_attn_norm", H, &mut f, &mut e);
        vec("ffn_norm", H, &mut f, &mut e);
        vec("post_ffn_norm", H, &mut f, &mut e);
        vec("attn_q_norm", hd, &mut f, &mut e);
        vec("attn_k_norm", hd, &mut f, &mut e);
        f.f32s
            .insert(format!("{p}.layer_scale"), vec![0.8 + 0.1 * l as f32]);
    }
    f.f32s.insert(
        "rope_freqs".into(),
        (0..64).map(|i| 1.0 + i as f32 / 16.0).collect(),
    );
    let _ = DTYPE_F32;
    (m, f)
}

fn cerca(a: &[f32], b: &[f32], que: &str) {
    // Error relativo L2 del vector: el KV es f16, así que una diferencia de 1 ulp en f32 puede cambiar un
    // redondeo (5e-4 relativo) y el máximo elemento a elemento no es una medida estable.
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        num += ((x - y) as f64).powi(2);
        den += (*y as f64).powi(2);
    }
    let rel = (num / den.max(1e-30)).sqrt();
    if std::env::var("CAPA_VERBOSO").is_ok() {
        eprintln!("{que}: {rel:.2e}");
    }
    assert!(rel < 2e-3, "{que}: error relativo L2 {rel}");
}

struct Pista {
    kv: Vec<LayerKv>,
    scratch: LayerScratch,
}

fn pista(m: &Manifest) -> Pista {
    Pista {
        kv: (0..m.num_layers).map(|_| LayerKv::new()).collect(),
        scratch: LayerScratch::new(m),
    }
}

/// Una pasada de las dos capas para `n` tokens desde la posición `pos0`; devuelve el `hidden` de salida.
fn pasar(
    m: &Manifest,
    f: &mut Fuente,
    p: &mut Pista,
    gpu: &mut Option<&mut dyn GpuDispatch>,
    usar: bool,
    pos0: usize,
    x: &[f32],
    n: usize,
) -> Vec<f32> {
    let exec = LayerExecutor {
        manifest: m,
        has_gate: true,
        parallel: None,
    };
    let mut hs = x.to_vec();
    for l in 0..2u32 {
        if n == 1 {
            forward_layer_gemma(
                &exec,
                l,
                pos0,
                &mut hs,
                &mut p.scratch,
                &mut p.kv[l as usize],
                f,
                gpu,
                usar,
                None,
            )
            .expect("capa");
        } else {
            let mut bs = soso_llm_core::layer::BlockScratch::new(m, 8);
            soso_llm_core::gemma::forward_layer_gemma_block(
                &exec,
                l,
                pos0,
                n,
                &mut hs,
                &mut bs,
                &mut p.scratch,
                &mut p.kv[l as usize],
                f,
                gpu,
                usar,
            )
            .expect("capa por bloques");
        }
    }
    hs
}

#[test]
fn la_capa_entera_en_gpu_iguala_a_la_cpu() {
    let mut cuda = match CudaGpu::nueva() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("cuda_capa: SALTADA, no hay CUDA ({e})");
            return;
        }
    };
    let (m, mut fuente) = construir();
    let mut cpu = pista(&m);
    let mut dev = pista(&m);
    let mut e = 5u64;
    let mut ninguna: Option<&mut dyn GpuDispatch> = None;
    let mut con_gpu: Option<&mut dyn GpuDispatch> = Some(&mut cuda);
    let mut pos = 0usize;
    let total = 2200usize; // pasa de ventana + 2048: hay expulsión en la capa local
    while pos < total {
        let nb: usize = std::env::var("CAPA_N")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8);
        let n = if pos % 3 == 0 && pos + nb <= total {
            nb
        } else {
            1
        };
        let x: Vec<f32> = (0..n * H)
            .map(|_| (azar(&mut e) % 2001) as f32 / 1000.0 - 1.0)
            .collect();
        // La referencia es siempre la CPU **token a token**: es la definición.
        let mut a = Vec::new();
        for i in 0..n {
            a.extend(pasar(
                &m,
                &mut fuente,
                &mut cpu,
                &mut ninguna,
                false,
                pos + i,
                &x[i * H..(i + 1) * H],
                1,
            ));
        }
        let b = pasar(&m, &mut fuente, &mut dev, &mut con_gpu, true, pos, &x, n);
        cerca(&b, &a, &format!("pos {pos} n {n}"));
        pos += n;
    }
    assert!(
        dev.kv[0].en_gpu && dev.kv[1].en_gpu,
        "las dos capas debían ir por la GPU"
    );
    assert!(
        dev.kv[0].dropped > 0,
        "la capa local debía haber expulsado filas"
    );
    // Cuántas filas se expulsan depende de la granularidad de las llamadas (bloques de 8 o tokens sueltos); lo que
    // importa es que la ventana siga entera, y eso lo comprueba la comparación de cada paso.

    // Rebobinado: se vuelve a una posición reciente (dentro de lo conservado) y se sigue.
    let vuelta = total - 40;
    cpu.kv[0].truncate_tokens(vuelta, 2 * 64);
    cpu.kv[1].truncate_tokens(vuelta, 128);
    dev.kv[0].truncate_tokens(vuelta, 2 * 64);
    dev.kv[1].truncate_tokens(vuelta, 128);
    for k in 0..6 {
        let x: Vec<f32> = (0..H)
            .map(|_| (azar(&mut e) % 2001) as f32 / 1000.0 - 1.0)
            .collect();
        let a = pasar(
            &m,
            &mut fuente,
            &mut cpu,
            &mut ninguna,
            false,
            vuelta + k,
            &x,
            1,
        );
        let b = pasar(
            &m,
            &mut fuente,
            &mut dev,
            &mut con_gpu,
            true,
            vuelta + k,
            &x,
            1,
        );
        cerca(&b, &a, &format!("tras rebobinar, paso {k}"));
    }
}

/// Diagnóstico: la GPU por bloques frente a la GPU token a token, etapa por etapa (capa 0).
#[test]
#[ignore = "diagnóstico"]
fn diagnostico_bloque_frente_a_token_en_gpu() {
    let Ok(mut cuda) = CudaGpu::nueva() else {
        return;
    };
    let (m, mut fuente) = construir();
    let mut a = pista(&m);
    let mut b = pista(&m);
    let l: u32 = std::env::var("CAPA_L")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut e = 5u64;
    let exec = LayerExecutor {
        manifest: &m,
        has_gate: true,
        parallel: None,
    };
    // Calienta con 10 tokens de uno en uno en las dos pistas.
    let mut g: Option<&mut dyn GpuDispatch> = Some(&mut cuda);
    for pos in 0..10usize {
        let x: Vec<f32> = (0..H)
            .map(|_| (azar(&mut e) % 2001) as f32 / 1000.0 - 1.0)
            .collect();
        for p in [&mut a, &mut b] {
            let mut hs = x.clone();
            forward_layer_gemma(
                &exec,
                l,
                pos,
                &mut hs,
                &mut p.scratch,
                &mut p.kv[l as usize],
                &mut fuente,
                &mut g,
                true,
                None,
            )
            .unwrap();
        }
    }
    drop(g);
    // Pista `a`: 8 tokens de uno en uno; pista `b`: un bloque de 8. Con volcado.
    let xs: Vec<f32> = (0..8 * H)
        .map(|_| (azar(&mut e) % 2001) as f32 / 1000.0 - 1.0)
        .collect();
    cuda.depurar_capa(true);
    let mut g: Option<&mut dyn GpuDispatch> = Some(&mut cuda);
    for i in 0..8usize {
        let mut hs = xs[i * H..(i + 1) * H].to_vec();
        forward_layer_gemma(
            &exec,
            l,
            10 + i,
            &mut hs,
            &mut a.scratch,
            &mut a.kv[l as usize],
            &mut fuente,
            &mut g,
            true,
            None,
        )
        .unwrap();
    }
    drop(g);
    let por_token = cuda.volcado_capa();
    cuda.depurar_capa(true);
    let mut g: Option<&mut dyn GpuDispatch> = Some(&mut cuda);
    let mut hs = xs.clone();
    let mut bs = soso_llm_core::layer::BlockScratch::new(&m, 8);
    soso_llm_core::gemma::forward_layer_gemma_block(
        &exec,
        l,
        10,
        8,
        &mut hs,
        &mut bs,
        &mut b.scratch,
        &mut b.kv[l as usize],
        &mut fuente,
        &mut g,
        true,
    )
    .unwrap();
    drop(g);
    let en_bloque = cuda.volcado_capa();
    let etapas: Vec<&str> = en_bloque.iter().map(|(n, _)| *n).collect();
    for (k, nombre) in etapas.iter().enumerate() {
        // El bloque vuelca una vez la etapa con 8 filas; el token a token, 8 veces con 1 fila.
        let bloque = &en_bloque[k].1;
        let mut tokens = Vec::new();
        for i in 0..8 {
            tokens.extend_from_slice(&por_token[i * etapas.len() + k].1);
        }
        let mut peor = 0.0f32;
        for (x, y) in bloque.iter().zip(&tokens) {
            peor = peor.max((x - y).abs() / (0.05 + y.abs()));
        }
        eprintln!("etapa {nombre}: {peor:.2e}");
    }
}
