//! Test host end-to-end: generate() completo sobre el modelo tiny
//! (hidden_dim != ffn_dim, que es lo que cazaría dimensiones intercambiadas).

#![cfg(feature = "std")]

use soso_llm_core::runtime::Runtime;
use soso_llm_core::source::host::MemFileMapper;
use soso_llm_core::source::MmapTensorSource;
use sosomodel::index::{make_f32_entry, pack_shard, TensorIndex};
use sosomodel::manifest::Manifest;

const BASE: &str = "/models/tiny/shards";

fn f32_shard(elems: usize, seed: u32) -> Vec<u8> {
    let raw: Vec<u8> = (0..elems)
        .flat_map(|i| {
            let v = (((i as u32).wrapping_mul(0x9e37_79b9) ^ seed) % 1000) as f32 * 1e-4;
            v.to_le_bytes()
        })
        .collect();
    pack_shard(&raw)
}

fn tiny_model() -> (Manifest, TensorIndex, MemFileMapper) {
    let manifest = Manifest::tiny("tiny");
    let h = manifest.hidden_dim;
    let ffn = manifest.ffn_dim;
    let vocab = manifest.vocab_size;

    let mut mapper = MemFileMapper::new();
    let mut index = TensorIndex::default();
    let mut id = 0u32;
    let add = |index: &mut TensorIndex,
                   mapper: &mut MemFileMapper,
                   id: &mut u32,
                   name: &str,
                   shape: &[u32]| {
        let elems: usize = shape.iter().map(|&d| d as usize).product();
        let shard = format!("{name}.tensor");
        mapper
            .files
            .insert(format!("{BASE}/{shard}"), f32_shard(elems, *id));
        index
            .entries
            .push(make_f32_entry(*id, name, &shard, 0, shape));
        *id += 1;
    };

    for layer in 0..manifest.num_layers {
        let p = format!("L{layer:02}");
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_norm"), &[h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_q"), &[h, h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_k"), &[h, h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_v"), &[h, h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_output"), &[h, h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_norm"), &[h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_up"), &[ffn, h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_down"), &[h, ffn]);
    }
    add(&mut index, &mut mapper, &mut id, "embed", &[vocab, h]);

    (manifest, index, mapper)
}

#[test]
fn generate_con_pesos_q8_0() {
    use soso_llm_core::quant::quantize_q8_0;
    use sosomodel::index::make_q8_0_entry;

    let manifest = Manifest::tiny("tiny");
    let h = manifest.hidden_dim;
    let ffn = manifest.ffn_dim;
    let vocab = manifest.vocab_size;

    let mut mapper = MemFileMapper::new();
    let mut index = TensorIndex::default();
    let mut id = 0u32;
    let mut add = |index: &mut TensorIndex,
                   mapper: &mut MemFileMapper,
                   id: &mut u32,
                   name: &str,
                   shape: &[u32],
                   q8: bool| {
        let elems: usize = shape.iter().map(|&d| d as usize).product();
        let values: Vec<f32> = (0..elems)
            .map(|i| ((i as u32).wrapping_mul(0x9e37_79b9) ^ *id) as f32 % 100.0 * 1e-3)
            .collect();
        let shard = format!("{name}.tensor");
        if q8 {
            mapper
                .files
                .insert(format!("{BASE}/{shard}"), pack_shard(&quantize_q8_0(&values)));
            index
                .entries
                .push(make_q8_0_entry(*id, name, &shard, 0, shape));
        } else {
            let raw: Vec<u8> = values.iter().flat_map(|f| f.to_le_bytes()).collect();
            mapper.files.insert(format!("{BASE}/{shard}"), pack_shard(&raw));
            index
                .entries
                .push(make_f32_entry(*id, name, &shard, 0, shape));
        }
        *id += 1;
    };

    for layer in 0..manifest.num_layers {
        let p = format!("L{layer:02}");
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_norm"), &[h], false);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_q"), &[h, h], true);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_k"), &[h, h], true);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_v"), &[h, h], true);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.attn_output"), &[h, h], true);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_norm"), &[h], false);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_up"), &[ffn, h], true);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_down"), &[h, ffn], true);
    }
    add(&mut index, &mut mapper, &mut id, "embed", &[vocab, h], true);

    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.validate_shapes().expect("shapes válidas");
    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
    let tokens = rt
        .generate(&mut source, &[5, 10], 3, None)
        .expect("generate con Q8_0 debe funcionar");
    assert!(tokens.len() >= 2);
}

#[test]
fn generate_completo_modelo_tiny() {
    let (manifest, index, mapper) = tiny_model();
    let vocab = manifest.vocab_size;
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.validate_shapes().expect("shapes válidas");

    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
    let prompt = [10u32, 20, 30];
    let tokens = rt
        .generate(&mut source, &prompt, 4, None)
        .expect("generate debe funcionar");
    assert!(tokens.len() >= prompt.len());
    assert!(tokens.len() <= prompt.len() + 4);
    assert!(tokens.iter().all(|&t| t < vocab));
    assert!(tokens.starts_with(&prompt));
}

#[test]
fn validate_shapes_detecta_transposicion() {
    let (manifest, mut index, _mapper) = tiny_model();
    // Transponer ffn_up ([ffn,h] → [h,ffn]) debe fallar la validación.
    for e in &mut index.entries {
        if e.name == "L00.ffn_up" {
            e.shape.reverse();
        }
    }
    let rt = Runtime::new(manifest, index, 0, 0);
    assert!(rt.validate_shapes().is_err());
}

#[test]
fn embed_token_fuera_de_rango_es_error() {
    let (manifest, index, mapper) = tiny_model();
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
    assert!(rt.embed_token(9999, &mut source).is_err());
}

#[test]
fn prompt_mas_largo_que_max_seq_falla() {
    let (manifest, index, mapper) = tiny_model();
    let max_seq = manifest.max_seq as usize;
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
    let prompt: Vec<u32> = (0..max_seq as u32 + 1).map(|i| i % 200).collect();
    assert!(rt.generate(&mut source, &prompt, 1, None).is_err());
}

/// Dispositivo de mentira que hace lo que hace el kernel: calcula el matvec y
/// dice "ya está hecho". Cuenta llamadas por tensor para poder afirmar que el
/// despacho recibe TODAS las proyecciones y con qué clave.
struct FakeDevice {
    llamadas: std::collections::BTreeMap<String, usize>,
}

impl soso_llm_core::gpu::GpuDispatch for FakeDevice {
    fn available(&self) -> bool {
        true
    }

    fn matvec_f32(
        &mut self,
        key: &str,
        view: &soso_llm_core::layer::TensorView<'_>,
        rows: usize,
        cols: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()> {
        let w = view.f32().ok_or(())?;
        if w.len() != rows * cols || x.len() != cols || out.len() != rows {
            return Err(());
        }
        *self.llamadas.entry(String::from(key)).or_insert(0) += 1;
        for r in 0..rows {
            let mut sum = 0.0f32;
            for c in 0..cols {
                sum += w[r * cols + c] * x[c];
            }
            out[r] = sum;
        }
        Ok(true)
    }
}

/// El camino de offload completo, con un dispositivo que sí calcula.
///
/// Esto existe porque en QEMU sin GPU el despacho no se ejercita nunca y el
/// primer sitio donde se probaría es la tarjeta real. Compara contra la ruta de
/// CPU: el dispositivo calcula lo mismo, así que los tokens tienen que ser los
/// mismos, y si no lo son es que el despacho está mandando otra cosa (pesos de
/// otro tensor, dimensiones cambiadas o el vector de entrada equivocado).
#[test]
fn generate_por_dispositivo_igual_que_cpu() {
    use soso_llm_core::gpu::GpuDispatch;

    let esperado = {
        let (manifest, index, mapper) = tiny_model();
        let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
        rt.set_backend(soso_llm_core::runtime::Backend::Cpu);
        let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
        let mut sampler = soso_llm_core::sample::Sampler::greedy();
        let mut sin_gpu: Option<&mut dyn GpuDispatch> = None;
        rt.generate_stream_par(
            &mut source,
            &[10, 20, 30],
            4,
            None,
            &mut sampler,
            |_| {},
            None,
            &mut sin_gpu,
        )
        .expect("la ruta de CPU debe funcionar")
    };

    let (manifest, index, mapper) = tiny_model();
    let num_layers = manifest.num_layers;
    let mut rt = Runtime::new(manifest, index.clone(), 0, 64 * 1024 * 1024);
    rt.set_backend(soso_llm_core::runtime::Backend::Auto);
    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
    let mut sampler = soso_llm_core::sample::Sampler::greedy();
    let mut dev = FakeDevice {
        llamadas: std::collections::BTreeMap::new(),
    };
    let tokens = {
        let mut gpu: Option<&mut dyn GpuDispatch> = Some(&mut dev);
        rt.generate_stream_par(
            &mut source,
            &[10, 20, 30],
            4,
            None,
            &mut sampler,
            |_| {},
            None,
            &mut gpu,
        )
        .expect("la ruta de dispositivo debe funcionar")
    };

    assert_eq!(tokens, esperado, "el dispositivo no da los mismos tokens que la CPU");

    // Todas las proyecciones de todas las capas han pasado por el dispositivo, y
    // la clave es el nombre del tensor (no una dirección).
    for layer in 0..num_layers {
        for t in ["attn_q", "attn_k", "attn_v", "attn_output", "ffn_up", "ffn_down"] {
            let key = format!("L{layer:02}.{t}");
            assert!(
                dev.llamadas.get(&key).copied().unwrap_or(0) > 0,
                "el dispositivo no recibió {key}"
            );
        }
    }
    // Y se le llamó una vez por token y por proyección: 3 del prompt + 4 nuevos.
    let q0 = dev.llamadas["L00.attn_q"];
    assert_eq!(q0, tokens.len(), "L00.attn_q: {q0} llamadas para {} tokens", tokens.len());
}
