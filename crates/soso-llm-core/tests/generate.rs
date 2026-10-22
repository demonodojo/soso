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

/// GQA: num_kv_heads < num_heads y attn_k/v con forma [kv_dim, h] (no [h, h]).
fn tiny_model_gqa() -> (Manifest, TensorIndex, MemFileMapper) {
    let mut manifest = Manifest::tiny("tiny-gqa");
    manifest.num_heads = 4;
    manifest.num_kv_heads = 2;
    let h = manifest.hidden_dim;
    let kv_dim = (manifest.num_kv_heads * (h / manifest.num_heads)) as u32;
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
        add(
            &mut index,
            &mut mapper,
            &mut id,
            &format!("{p}.attn_k"),
            &[kv_dim, h],
        );
        add(
            &mut index,
            &mut mapper,
            &mut id,
            &format!("{p}.attn_v"),
            &[kv_dim, h],
        );
        add(
            &mut index,
            &mut mapper,
            &mut id,
            &format!("{p}.attn_output"),
            &[h, h],
        );
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_norm"), &[h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_up"), &[ffn, h]);
        add(&mut index, &mut mapper, &mut id, &format!("{p}.ffn_down"), &[h, ffn]);
    }
    add(&mut index, &mut mapper, &mut id, "embed", &[vocab, h]);

    (manifest, index, mapper)
}

#[test]
fn generate_con_pesos_q8_0() {
    let (manifest, index, mapper) = tiny_model_q8_0();
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.validate_shapes().expect("shapes válidas");
    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
    let tokens = rt
        .generate(&mut source, &[5, 10], 3, None)
        .expect("generate con Q8_0 debe funcionar");
    assert!(tokens.len() >= 2);
}

fn tiny_model_q8_0() -> (Manifest, TensorIndex, MemFileMapper) {
    tiny_model_cuantizado(false)
}

/// Q4_K necesita filas múltiplo de 256 (un superbloque entero por fila), y el
/// modelo `tiny` es de 128 de ancho: con él, el matvec fusionado devuelve `Err` y
/// la inferencia entera se cae. Así que este modelo es más ancho a propósito.
fn tiny_model_q4_k() -> (Manifest, TensorIndex, MemFileMapper) {
    tiny_model_cuantizado(true)
}

fn tiny_model_mxfp4() -> (Manifest, TensorIndex, MemFileMapper) {
    use soso_llm_core::quant::quantize_mxfp4;
    use sosomodel::index::make_mxfp4_entry;

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
                   shape: &[u32],
                   mx: bool| {
        let elems: usize = shape.iter().map(|&d| d as usize).product();
        let values: Vec<f32> = (0..elems)
            .map(|i| ((i as u32).wrapping_mul(0x9e37_79b9) ^ *id) as f32 % 100.0 * 1e-3)
            .collect();
        let shard = format!("{name}.tensor");
        if mx {
            mapper
                .files
                .insert(format!("{BASE}/{shard}"), pack_shard(&quantize_mxfp4(&values)));
            index
                .entries
                .push(make_mxfp4_entry(*id, name, &shard, 0, shape));
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

    (manifest, index, mapper)
}

/// Modelo tiny con los tensores 2D cuantizados: Q4_K si `q4k`, Q8_0 si no.
fn tiny_model_cuantizado(q4k: bool) -> (Manifest, TensorIndex, MemFileMapper) {
    use soso_llm_core::quant::{quantize_q4_k, quantize_q8_0};
    use sosomodel::index::{make_q4_k_entry, make_q8_0_entry};

    let mut manifest = Manifest::tiny("tiny");
    if q4k {
        // 256 de ancho y 512 de FFN: múltiplos del superbloque de Q4_K.
        manifest.hidden_dim = 256;
        manifest.ffn_dim = 512;
    }
    let manifest = manifest;
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
                   shape: &[u32],
                   q8: bool| {
        let elems: usize = shape.iter().map(|&d| d as usize).product();
        let values: Vec<f32> = (0..elems)
            .map(|i| ((i as u32).wrapping_mul(0x9e37_79b9) ^ *id) as f32 % 100.0 * 1e-3)
            .collect();
        let shard = format!("{name}.tensor");
        if q8 && q4k {
            mapper
                .files
                .insert(format!("{BASE}/{shard}"), pack_shard(&quantize_q4_k(&values)));
            index
                .entries
                .push(make_q4_k_entry(*id, name, &shard, 0, shape));
        } else if q8 {
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

    (manifest, index, mapper)
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
fn generate_modelo_gqa() {
    let (manifest, index, mapper) = tiny_model_gqa();
    let vocab = manifest.vocab_size;
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.validate_shapes().expect("shapes GQA válidas");

    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
    let tokens = rt
        .generate(&mut source, &[5, 10], 3, None)
        .expect("generate con GQA debe funcionar");
    assert!(tokens.len() >= 2);
    assert!(tokens.iter().all(|&t| t < vocab));
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

    fn matvec(
        &mut self,
        key: &str,
        view: &soso_llm_core::layer::TensorView<'_>,
        rows: usize,
        cols: usize,
        x: &[f32],
        out: &mut [f32],
    ) -> Result<bool, ()> {
        use sosomodel::layout::{DTYPE_F32, DTYPE_MXFP4, DTYPE_Q4_K, DTYPE_Q8_0};

        if view.elems != rows * cols || x.len() != cols || out.len() != rows {
            return Err(());
        }
        // Igual que el dispositivo de verdad: descuantiza al "subir" los pesos,
        // porque calcula en f32. Si esto no admitiera cuantizados, el test daría
        // verde sin ejercitar el caso que de verdad importa (los modelos que caben
        // en una tarjeta están cuantizados).
        let mut plano = vec![0.0f32; rows * cols];
        match view.dtype {
            DTYPE_F32 => plano.copy_from_slice(view.f32().ok_or(())?),
            DTYPE_Q8_0 => soso_llm_core::quant::dequant_q8_0(view.bytes, &mut plano)?,
            DTYPE_Q4_K => soso_llm_core::quant::dequant_q4_k(view.bytes, &mut plano)?,
            DTYPE_MXFP4 => soso_llm_core::quant::dequant_mxfp4(view.bytes, &mut plano)?,
            _ => return Ok(false),
        }
        *self.llamadas.entry(String::from(key)).or_insert(0) += 1;
        for r in 0..rows {
            let mut sum = 0.0f32;
            for c in 0..cols {
                sum += plano[r * cols + c] * x[c];
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
            &mut |_, _| {},
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
            &mut |_, _| {},
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

/// Igual que el de Q8_0 pero con **Q4_K**, que es el formato de los modelos que de
/// verdad se usan (TinyLlama Q4_K_M). No se podía escribir hasta que existió
/// `quantize_q4_k`: sin cuantizador no había forma de sintetizar un modelo Q4_K y
/// este camino sólo se habría estrenado con un modelo real de gigabytes.
#[test]
fn generate_por_dispositivo_con_pesos_q4_k() {
    use soso_llm_core::gpu::GpuDispatch;

    let esperado = {
        let (manifest, index, mapper) = tiny_model_q4_k();
        let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
        rt.set_backend(soso_llm_core::runtime::Backend::Cpu);
        let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
        let mut sampler = soso_llm_core::sample::Sampler::greedy();
        let mut sin_gpu: Option<&mut dyn GpuDispatch> = None;
        rt.generate_stream_par(
            &mut source, &[5, 10], 4, None, &mut sampler, |_| {}, None, &mut sin_gpu, &mut |_, _| {},
        )
        .expect("la ruta de CPU con Q4_K debe funcionar")
    };

    let (manifest, index, mapper) = tiny_model_q4_k();
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
            &mut source, &[5, 10], 4, None, &mut sampler, |_| {}, None, &mut gpu, &mut |_, _| {},
        )
        .expect("la ruta de dispositivo con Q4_K debe funcionar")
    };

    for layer in 0..num_layers {
        for t in ["attn_q", "attn_k", "attn_v", "attn_output", "ffn_up", "ffn_down"] {
            let key = format!("L{layer:02}.{t}");
            assert!(
                dev.llamadas.get(&key).copied().unwrap_or(0) > 0,
                "el dispositivo no recibió {key} en Q4_K (¿se rindió por el dtype?)"
            );
        }
    }
    assert_eq!(
        tokens, esperado,
        "descuantizar Q4_K al subir no da lo mismo que el kernel fusionado de CPU"
    );
}

/// Pesos **cuantizados** por el dispositivo (Q8_0), contra la ruta de CPU.
///
/// Este es el caso que de verdad importa: los modelos que caben en una tarjeta
/// están cuantizados, y el despacho sólo admitía F32 — con un Q8_0 o un Q4_K se
/// iba a CPU en silencio y el offload no existía. El dispositivo calcula en f32,
/// así que descuantiza al subir; lo que se comprueba aquí es que eso da el mismo
/// resultado que el kernel fusionado de CPU, tensor a tensor y token a token.
#[test]
fn generate_por_dispositivo_con_pesos_cuantizados() {
    use soso_llm_core::gpu::GpuDispatch;

    let esperado = {
        let (manifest, index, mapper) = tiny_model_q8_0();
        let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
        rt.set_backend(soso_llm_core::runtime::Backend::Cpu);
        let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
        let mut sampler = soso_llm_core::sample::Sampler::greedy();
        let mut sin_gpu: Option<&mut dyn GpuDispatch> = None;
        rt.generate_stream_par(
            &mut source,
            &[5, 10],
            4,
            None,
            &mut sampler,
            |_| {},
            None,
            &mut sin_gpu,
            &mut |_, _| {},
        )
        .expect("la ruta de CPU con Q8_0 debe funcionar")
    };

    let (manifest, index, mapper) = tiny_model_q8_0();
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
            &[5, 10],
            4,
            None,
            &mut sampler,
            |_| {},
            None,
            &mut gpu,
            &mut |_, _| {},
        )
        .expect("la ruta de dispositivo con Q8_0 debe funcionar")
    };

    // Las proyecciones cuantizadas TIENEN que haber pasado por el dispositivo: si
    // el despacho se rindiera por el dtype (el bug que esto cierra), el test
    // seguiría generando tokens correctos y no se notaría nada.
    for layer in 0..num_layers {
        for t in ["attn_q", "attn_k", "attn_v", "attn_output", "ffn_up", "ffn_down"] {
            let key = format!("L{layer:02}.{t}");
            assert!(
                dev.llamadas.get(&key).copied().unwrap_or(0) > 0,
                "el dispositivo no recibió {key} (¿se rindió por el dtype?)"
            );
        }
    }
    assert_eq!(
        tokens, esperado,
        "descuantizar al subir no da lo mismo que el kernel fusionado de CPU"
    );
}

/// Pesos **MXFP4** por el dispositivo, contra la ruta de CPU.
#[test]
fn generate_por_dispositivo_con_pesos_mxfp4() {
    use soso_llm_core::gpu::GpuDispatch;

    let esperado = {
        let (manifest, index, mapper) = tiny_model_mxfp4();
        let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
        rt.set_backend(soso_llm_core::runtime::Backend::Cpu);
        let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
        let mut sampler = soso_llm_core::sample::Sampler::greedy();
        let mut sin_gpu: Option<&mut dyn GpuDispatch> = None;
        rt.generate_stream_par(
            &mut source, &[5, 10], 4, None, &mut sampler, |_| {}, None, &mut sin_gpu, &mut |_, _| {},
        )
        .expect("la ruta de CPU con MXFP4 debe funcionar")
    };

    let (manifest, index, mapper) = tiny_model_mxfp4();
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
            &mut source, &[5, 10], 4, None, &mut sampler, |_| {}, None, &mut gpu, &mut |_, _| {},
        )
        .expect("la ruta de dispositivo con MXFP4 debe funcionar")
    };

    for layer in 0..num_layers {
        for t in ["attn_q", "attn_k", "attn_v", "attn_output", "ffn_up", "ffn_down"] {
            let key = format!("L{layer:02}.{t}");
            assert!(
                dev.llamadas.get(&key).copied().unwrap_or(0) > 0,
                "el dispositivo no recibió {key} en MXFP4 (¿se rindió por el dtype?)"
            );
        }
    }
    assert_eq!(
        tokens, esperado,
        "descuantizar MXFP4 al subir no da lo mismo que el kernel fusionado de CPU"
    );
}

// T22: reutilizar el prefijo del KV entre peticiones. Tiene que dar exactamente
// lo mismo que un prefill limpio: por causalidad, el K/V de los primeros `p`
// tokens sólo depende de ellos.
fn generar_en_frio(prompt: &[u32], max_new: usize) -> Vec<u32> {
    let (manifest, index, mapper) = tiny_model_gqa();
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
    rt.generate(&mut source, prompt, max_new, None).expect("frío")
}

#[test]
fn reutilizar_prefijo_da_lo_mismo_que_un_prefill_limpio() {
    let (manifest, index, mapper) = tiny_model_gqa();
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);

    let p1: Vec<u32> = vec![3, 4, 5, 6, 7, 8];
    let t1 = rt.generate(&mut source, &p1, 3, None).expect("primera");
    assert_eq!(t1, generar_en_frio(&p1, 3));

    // Segundo turno de un agente: el historial anterior más lo nuevo.
    let mut p2 = t1.clone();
    p2.extend_from_slice(&[9, 10, 11]);
    let caliente = rt.generate(&mut source, &p2, 4, None).expect("segunda");
    assert_eq!(caliente, generar_en_frio(&p2, 4));

    // Un tercero que diverge a mitad: sólo se reutiliza lo común.
    let mut p3: Vec<u32> = p2[..4].to_vec();
    p3.extend_from_slice(&[20, 21, 22, 23]);
    let caliente = rt.generate(&mut source, &p3, 4, None).expect("tercera");
    assert_eq!(caliente, generar_en_frio(&p3, 4));

    // El mismo prompt otra vez: queda el último token por procesar.
    let caliente = rt.generate(&mut source, &p3, 4, None).expect("cuarta");
    assert_eq!(caliente, generar_en_frio(&p3, 4));
}

#[test]
fn cuanto_prefijo_se_reutiliza() {
    let (manifest, index, mapper) = tiny_model_gqa();
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
    let p1: Vec<u32> = vec![3, 4, 5, 6, 7, 8];
    let t1 = rt.generate(&mut source, &p1, 3, None).expect("primera");

    // Sin nada en común o con una caché vacía no hay reutilización.
    assert_eq!(rt.begin_sequence_reusing(&[99, 4, 5]), 0);
    assert_eq!(rt.pos, 0);

    // Tras otra generación, lo común con el historial anterior.
    let _ = rt.generate(&mut source, &p1, 3, None).expect("otra vez");
    let mut p2 = t1.clone();
    p2.extend_from_slice(&[9, 10]);
    let ya = rt.begin_sequence_reusing(&p2);
    assert!(ya >= p1.len(), "se esperaba reutilizar al menos el prompt, {ya}");
    assert!(ya <= t1.len());
    assert_eq!(rt.pos, ya);

    // El prompt idéntico nunca se reutiliza entero.
    let _ = rt.generate(&mut source, &p1, 2, None).expect("fija");
    let ya = rt.begin_sequence_reusing(&p1);
    assert_eq!(ya, p1.len() - 1);
}

// Lo mismo por la ruta con planificador, que es la del servidor del guest.
fn reloj_cero() -> u64 {
    0
}

fn generar_planificado(
    rt: &mut Runtime,
    source: &mut MmapTensorSource<MemFileMapper>,
    prompt: &[u32],
    max_new: usize,
) -> Vec<u32> {
    use soso_llm_core::generation::{GenerationOptions, NoCancelObserver};
    use soso_llm_core::plan::MemSnapshot;
    let mut sampler = soso_llm_core::sample::Sampler::greedy();
    let mut obs = NoCancelObserver;
    let mut mem = || MemSnapshot {
        total_frames: 100_000,
        free_frames: 50_000,
        reclaimable_frames: 0,
    };
    rt.generate_stream_planned_observed(
        source,
        prompt,
        GenerationOptions::new(max_new, vec![]),
        &mut sampler,
        &mut obs,
        None,
        &mut None,
        reloj_cero,
        &mut mem,
        &mut None,
    )
    .expect("planificado")
    .0
}

fn runtime_planificado() -> (Runtime, MmapTensorSource<MemFileMapper>) {
    use soso_llm_core::plan::{MemSnapshot, ResourcePlanner};
    let (manifest, index, mapper) = tiny_model_gqa();
    let mut rt = Runtime::new(manifest.clone(), index.clone(), 0, 0);
    let mem = MemSnapshot {
        total_frames: 100_000,
        free_frames: 50_000,
        reclaimable_frames: 0,
    };
    let planner = ResourcePlanner::new(&manifest, &rt.index, mem, 0, false);
    rt.set_planner(planner);
    (rt, MmapTensorSource::new(String::from(BASE), index, mapper))
}

#[test]
fn reutilizar_prefijo_con_planificador_da_lo_mismo_que_en_frio() {
    let (mut rt, mut source) = runtime_planificado();
    let p1: Vec<u32> = vec![3, 4, 5, 6, 7, 8];
    let t1 = generar_planificado(&mut rt, &mut source, &p1, 3);
    let (mut frio, mut fsrc) = runtime_planificado();
    assert_eq!(t1, generar_planificado(&mut frio, &mut fsrc, &p1, 3));

    let mut p2 = t1.clone();
    p2.extend_from_slice(&[9, 10, 11]);
    let caliente = generar_planificado(&mut rt, &mut source, &p2, 4);
    let (mut frio, mut fsrc) = runtime_planificado();
    assert_eq!(caliente, generar_planificado(&mut frio, &mut fsrc, &p2, 4));

    // Tras una generación completa queda algo que reutilizar.
    let ya = rt.begin_sequence_reusing(&p2);
    assert!(ya >= p1.len(), "con planificador también se reutiliza: {ya}");
}

// T82: el prefill por bloques es idéntico bit a bit al de un token cada vez.
type Estado = (usize, Vec<u32>, Vec<(Vec<u16>, Vec<u16>, Vec<i8>, Vec<i8>, Vec<u32>, Vec<u32>)>);

fn estado(rt: &Runtime) -> Estado {
    let bits_h: Vec<u32> = rt.hidden.iter().map(|x| x.to_bits()).collect();
    let capas = rt
        .kv
        .iter()
        .map(|l| {
            (
                l.k_f16.clone(),
                l.v_f16.clone(),
                l.k_i8.clone(),
                l.v_i8.clone(),
                l.k_scale.iter().map(|x| x.to_bits()).collect(),
                l.v_scale.iter().map(|x| x.to_bits()).collect(),
            )
        })
        .collect();
    (rt.pos, bits_h, capas)
}

fn prefill(modelo: fn() -> (Manifest, TensorIndex, MemFileMapper), prompt: &[u32], bloques: bool) -> Runtime {
    let (manifest, index, mapper) = modelo();
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.prefill_por_bloques = bloques;
    let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
    rt.prefill_prompt(&mut source, prompt, None, &mut None, None)
        .expect("prefill");
    rt
}

#[test]
fn prefill_por_bloques_identico_al_token_a_token() {
    for modelo in [tiny_model as fn() -> _, tiny_model_gqa as fn() -> _] {
        // Longitudes que no son múltiplo del bloque (8): 1 bloque + resto, y sólo resto.
        for n in [2usize, 7, 8, 9, 17, 20] {
            let prompt: Vec<u32> = (0..n as u32).map(|i| (i * 5 + 3) % 40).collect();
            let a = prefill(modelo, &prompt, false);
            let b = prefill(modelo, &prompt, true);
            assert_eq!(a.tokens_en_bloque, 0);
            // Sin esto la prueba sería trivial: tiene que haber pasado por el camino nuevo.
            assert_eq!(
                b.tokens_en_bloque,
                (n / soso_llm_core::runtime::Runtime::BLOQUE_PREFILL * soso_llm_core::runtime::Runtime::BLOQUE_PREFILL
                    + if n % soso_llm_core::runtime::Runtime::BLOQUE_PREFILL >= 2 { n % soso_llm_core::runtime::Runtime::BLOQUE_PREFILL } else { 0 }) as u64,
                "n={n}: el camino por bloques no se usó como se esperaba"
            );
            assert_eq!(estado(&a), estado(&b), "n={n}");
        }
    }
}

#[test]
fn bloques_dan_los_mismos_tokens_generados() {
    let prompt: Vec<u32> = (0..19u32).map(|i| (i * 7 + 1) % 40).collect();
    let mut salidas = Vec::new();
    for bloques in [false, true] {
        let (manifest, index, mapper) = tiny_model_gqa();
        let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
        rt.prefill_por_bloques = bloques;
        let mut source = MmapTensorSource::new(String::from(BASE), index, mapper);
        salidas.push(rt.generate(&mut source, &prompt, 6, None).expect("generate"));
    }
    assert_eq!(salidas[0], salidas[1]);
}

// T82 con la política H2O del planificador (la que usa el guest): el camino por bloques
// tiene que acumular la misma masa de atención que el de un token.
fn runtime_h2o(bloques: bool, kv_f16: bool) -> Option<(Runtime, MmapTensorSource<MemFileMapper>, usize)> {
    use soso_llm_core::plan::{MemSnapshot, ResourcePlanner};
    let (mut manifest, index, mapper) = tiny_model_gqa();
    // Como el 7B en el guest: los pesos caben, pero la ventana de KV es más corta que
    // `max_seq`, así que el planificador activa H2O.
    manifest.max_seq = 50_000;
    let rt = Runtime::new(manifest.clone(), index.clone(), 0, 0);
    // La ventana más corta con los pesos residentes (la búsqueda sólo construye planificadores).
    for frames in (1u64..400_000).step_by(16) {
        let mem = MemSnapshot {
            total_frames: frames,
            free_frames: frames / 2,
            reclaimable_frames: 0,
        };
        let pl = ResourcePlanner::new(&manifest, &rt.index, mem, 0, false);
        if pl.use_h2o() && pl.keep_weights_mapped() && pl.kv_window_tokens() >= 24 {
            let ventana = pl.kv_window_tokens();
            let mut rt = rt;
            rt.set_planner(pl);
            if kv_f16 {
                // El planificador cuantiza el KV con ventanas tan cortas; se fuerza f16 para ejercitar
                // el kernel rápido y el genérico con masa sobre el mismo KV f16.
                use soso_llm_core::kv::{KvDtype, LayerKv};
                let m = rt.manifest.clone();
                for l in 0..m.num_layers {
                    let kv_dim = m.effective_num_kv_heads(l) as usize * m.effective_head_dim(l) as usize;
                    rt.kv[l as usize] = LayerKv::with_capacity_dtype(256, kv_dim, KvDtype::F16);
                }
            }
            rt.prefill_por_bloques = bloques;
            return Some((rt, MmapTensorSource::new(String::from(BASE), index, mapper), ventana));
        }
    }
    None
}

fn comprobar_bloques_con_h2o(kv_f16: bool) {
    // La masa de H2O sólo se acumula cuando la secuencia pasa de la mitad de la ventana. Un prompt
    // de ¾ de la ventana la cruza a mitad del prefill: los primeros tokens van por el kernel
    // rápido y los últimos por el genérico con masa (el caso mixto), sin llegar a expulsar nada.
    let (Some((mut a, mut sa, ventana)), Some((mut b, mut sb, _))) = (runtime_h2o(false, kv_f16), runtime_h2o(true, kv_f16)) else {
        panic!("no hay una configuración de memoria con H2O y pesos residentes en el modelo tiny");
    };
    let prompt: Vec<u32> = (0..(ventana * 3 / 4) as u32).map(|i| (i * 5 + 3) % 40).collect();
    a.prefill_prompt(&mut sa, &prompt, None, &mut None, None).unwrap();
    b.prefill_prompt(&mut sb, &prompt, None, &mut None, None).unwrap();
    assert_eq!(a.tokens_en_bloque, 0);
    assert!(b.tokens_en_bloque >= 16, "el camino por bloques no se usó con H2O: {}", b.motivo_sin_bloque);
    assert_eq!(estado(&a), estado(&b));
    let dtype = if a.kv[0].k_i8.is_empty() { "f16" } else { "int8" };
    println!("KV del planificador en la prueba H2O: {dtype}");
    for (la, lb) in a.kv.iter().zip(&b.kv) {
        let ma: Vec<u32> = la.mass.iter().map(|x| x.to_bits()).collect();
        let mb: Vec<u32> = lb.mass.iter().map(|x| x.to_bits()).collect();
        assert_eq!(ma, mb, "la masa de atención (H2O) difiere");
        assert!(!ma.is_empty() && ma.iter().any(|&x| f32::from_bits(x) > 0.0), "sin masa acumulada: la prueba no ejercita H2O");
    }
}

#[test]
fn bloques_con_h2o_acumulan_la_misma_masa() {
    comprobar_bloques_con_h2o(false);
}

// Con KV f16: el bloque mezcla el kernel rápido por grupos (tokens cortos) y el genérico con masa.
#[test]
fn bloques_con_h2o_y_kv_f16_mezclan_los_dos_kernels_sin_diferencias() {
    comprobar_bloques_con_h2o(true);
}


// Verificación del borrador PLD con un bloque: la misma secuencia (y el mismo estado final) que token a
// token y que sin borrador, con menos forwards.
fn rt_borrador(borrador: bool, bloques: bool) -> (Runtime, MmapTensorSource<MemFileMapper>) {
    let (manifest, index, mapper) = tiny_model_gqa();
    let mut rt = Runtime::new(manifest, index.clone(), 0, 0);
    rt.borrador_activo = borrador;
    rt.borrador_por_bloques = bloques;
    (rt, MmapTensorSource::new(String::from(BASE), index, mapper))
}

fn generar_con_borrador(
    borrador: bool,
    bloques: bool,
    prompt: &[u32],
    max_new: usize,
    stops: Vec<u32>,
) -> (Vec<u32>, Runtime) {
    use soso_llm_core::generation::GenerationOptions;
    let (mut rt, mut source) = rt_borrador(borrador, bloques);
    let (tokens, _) = rt
        .generate_with_report(&mut source, prompt, GenerationOptions::new(max_new, stops))
        .expect("generate");
    (tokens, rt)
}

// Un prompt que se repite, para que el borrador por prompt-lookup proponga continuaciones.
fn prompt_repetitivo() -> Vec<u32> {
    let ciclo = [3u32, 9, 14, 21, 5, 30];
    (0..24).map(|i| ciclo[i % ciclo.len()]).collect()
}

#[test]
fn borrador_por_bloques_da_los_mismos_tokens_y_el_mismo_estado() {
    let prompt = prompt_repetitivo();
    let (base, rt_base) = generar_con_borrador(false, false, &prompt, 40, vec![]);
    let (uno_a_uno, rt_1) = generar_con_borrador(true, false, &prompt, 40, vec![]);
    let (bloque, rt_b) = generar_con_borrador(true, true, &prompt, 40, vec![]);
    assert_eq!(base, uno_a_uno);
    assert_eq!(base, bloque);
    assert_eq!(estado(&rt_base), estado(&rt_1));
    assert_eq!(estado(&rt_base), estado(&rt_b));
    // Sin esto la prueba sería trivial: el borrador tiene que haberse ofrecido y aceptado.
    assert!(rt_b.borrador_aceptados > 0, "el borrador nunca acertó: la prueba no cubre nada");
    assert_eq!(rt_b.borrador_ofrecidos, rt_1.borrador_ofrecidos);
    assert_eq!(rt_b.borrador_aceptados, rt_1.borrador_aceptados);
    assert!(
        rt_b.forwards_decode < rt_1.forwards_decode,
        "bloques {} forwards, token a token {}",
        rt_b.forwards_decode,
        rt_1.forwards_decode
    );
}

#[test]
fn borrador_por_bloques_respeta_las_paradas_dentro_del_borrador() {
    let prompt = prompt_repetitivo();
    let (completo, _) = generar_con_borrador(false, false, &prompt, 40, vec![]);
    // Cada token generado, como parada: tanto si cae en la cabeza de una ronda como dentro de un borrador.
    for i in prompt.len()..completo.len() {
        let parada = completo[i];
        let (a, rt_a) = generar_con_borrador(false, false, &prompt, 40, vec![parada]);
        let (b, rt_b) = generar_con_borrador(true, true, &prompt, 40, vec![parada]);
        assert_eq!(a, b, "parada {parada} en {i}");
        assert_eq!(estado(&rt_a), estado(&rt_b), "parada {parada} en {i}");
    }
}

#[test]
fn borrador_por_bloques_respeta_el_limite_de_tokens() {
    let prompt = prompt_repetitivo();
    for max_new in [1usize, 2, 3, 5, 9] {
        let (a, _) = generar_con_borrador(false, false, &prompt, max_new, vec![]);
        let (b, _) = generar_con_borrador(true, true, &prompt, max_new, vec![]);
        assert_eq!(a, b, "max_new={max_new}");
        assert_eq!(a.len(), prompt.len() + max_new);
    }
}

#[test]
fn borrador_por_bloques_con_planificador_da_lo_mismo() {
    let prompt = prompt_repetitivo();
    let mut salidas = Vec::new();
    let mut forwards = Vec::new();
    for bloques in [false, true] {
        let (mut rt, mut source) = runtime_planificado();
        rt.borrador_por_bloques = bloques;
        salidas.push(generar_planificado(&mut rt, &mut source, &prompt, 40));
        forwards.push(rt.forwards_decode);
    }
    assert_eq!(salidas[0], salidas[1]);
    assert!(forwards[1] < forwards[0], "{forwards:?}");
}

fn reloj_que_avanza() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static T: AtomicU64 = AtomicU64::new(0);
    T.fetch_add(3, Ordering::Relaxed)
}

#[test]
fn la_politica_del_borrador_no_cambia_lo_generado() {
    let prompt = prompt_repetitivo();
    let (base, _) = generar_con_borrador(false, false, &prompt, 60, vec![]);
    for bloques in [false, true] {
        let (mut rt, mut source) = rt_borrador(true, bloques);
        rt.reloj = Some(reloj_que_avanza);
        let (tokens, _) = rt
            .generate_with_report(&mut source, &prompt, soso_llm_core::generation::GenerationOptions::new(60, vec![]))
            .expect("generate");
        assert_eq!(tokens, base, "bloques={bloques}");
        // La política recibió muestras: pasó por ella.
        assert!(rt.politica.muestras() > 0);
    }
}

// Restricción del muestreo en el bucle de generación: sólo tokens pares. Anota los tokens que se le avisan
// (el original; las copias para verificar un borrador van aparte).
mod restringido {
    use super::*;
    use soso_llm_core::generation::GenerationOptions;
    use soso_llm_core::restriccion::Restriccion;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct SoloPares {
        vistos: Rc<RefCell<Vec<u32>>>,
    }

    impl Restriccion for SoloPares {
        fn permite(&self, token: u32) -> bool {
            token % 2 == 0
        }
        fn avanza(&mut self, token: u32) {
            self.vistos.borrow_mut().push(token);
        }
        fn activa(&self) -> bool {
            true
        }
        fn abandonar(&mut self) {}
        fn duplicar(&self) -> Box<dyn Restriccion> {
            Box::new(SoloPares { vistos: Rc::new(RefCell::new(Vec::new())) })
        }
    }

    fn generar(borrador: bool, bloques: bool, prompt: &[u32], max_new: usize) -> (Vec<u32>, Vec<u32>, Runtime) {
        let (mut rt, mut source) = rt_borrador(borrador, bloques);
        let vistos = Rc::new(RefCell::new(Vec::new()));
        let opciones = GenerationOptions::new(max_new, vec![])
            .con_restriccion(Some(Box::new(SoloPares { vistos: vistos.clone() })));
        let (tokens, _) = rt.generate_with_report(&mut source, prompt, opciones).expect("generate");
        let v = vistos.borrow().clone();
        (tokens, v, rt)
    }

    #[test]
    fn solo_salen_tokens_permitidos_y_se_avisa_de_todos() {
        let prompt = prompt_repetitivo();
        let (base, vistos_base, _) = generar(false, false, &prompt, 40);
        let (uno, vistos_uno, rt_uno) = generar(true, false, &prompt, 40);
        let (bloque, vistos_bloque, rt_bloque) = generar(true, true, &prompt, 40);
        for (nombre, t, v) in [("sin borrador", &base, &vistos_base), ("token a token", &uno, &vistos_uno), ("bloque", &bloque, &vistos_bloque)] {
            let generados = &t[prompt.len()..];
            assert_eq!(generados.len(), 40, "{nombre}");
            assert!(generados.iter().all(|x| x % 2 == 0), "{nombre}: salió un token no permitido: {generados:?}");
            assert_eq!(v, generados, "{nombre}: la restricción debe ver exactamente los tokens generados");
        }
        assert_eq!(base, uno);
        assert_eq!(base, bloque);
        // El borrador sigue funcionando con la restricción (si no, la prueba no cubriría el camino).
        assert!(rt_bloque.borrador_ofrecidos > 0 || rt_uno.borrador_ofrecidos > 0);
    }

    #[test]
    fn sin_restriccion_nada_cambia() {
        let prompt = prompt_repetitivo();
        let (a, _) = generar_con_borrador(true, true, &prompt, 30, vec![]);
        let (b, _) = generar_con_borrador(true, true, &prompt, 30, vec![]);
        assert_eq!(a, b);
        assert!(a[prompt.len()..].iter().any(|x| x % 2 == 1), "sin restringir el modelo sí saca impares");
    }
}
