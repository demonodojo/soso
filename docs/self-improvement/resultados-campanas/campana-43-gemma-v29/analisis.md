# Análisis de ejecucion-43-gemma-v29

1 aceptadas de 10. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R31 | rechazada | CASO LÍMITE NO CUBIERTO: fallan cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, |
| R32 | rechazada | NO COMPILA con las pruebas reservadas: test failed, to rerun pass `-p sosofs --test aceptacion_R32` |
| R33 | rechazada | CASO LÍMITE NO CUBIERTO: fallan wav::pruebas_agente::test_data_before_fmt, wav::pruebas_agente::test_incomplet |
| R34 | rechazada | CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy |
| R35 | rechazada | NO COMPILA con las pruebas reservadas: cannot find function `gguf_string` in this scope; no method named `writ |
| R36 | rechazada | CASO LÍMITE NO CUBIERTO: fallan las_versiones_desconocidas_se_rechazan |
| R37 | rechazada | CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta, rechaza_esquemas_y_hosts_invalidos |
| R38 | rechazada | CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple |
| R39 | rechazada | CASO LÍMITE NO CUBIERTO: fallan indices_enormes_no_desbordan |


## R31 — rechazada (1549 s, 40 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, cualquier_truncado_es_error_y_no_panico

**Ediciones:** {'sin efecto': 20, 'aplicada': 3, 'otro error': 1}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/sosomfs/src/catalog.rs:121:47`: range end index 97 out of range for slice of length 96
- `crates/sosomfs/src/catalog.rs:177:49`: range end index 119 out of range for slice of length 116
- `crates/sosomfs/src/catalog.rs:183:47`: range end index 211 out of range for slice of length 209

**Pruebas reservadas que fallan:** cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, cualquier_truncado_es_error_y_no_panico


```diff
--- a/crates/sosomfs/src/catalog.rs
+++ b/crates/sosomfs/src/catalog.rs
@@ -45,11 +45,18 @@
         out
     }
 
-    pub fn parse(data: &[u8]) -> Result<Self, ()> {
+    pub fn parse(data:&[u8]) -> Result<Self, ()> {
         if data.len() < 4 {
             return Err(());
         }
         let nmodels = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
+        
+        // Check if nmodels is reasonable given the data size
+        // Each model is at least NAME_MAX + 32 bytes
+        if nmodels > 0 && data.len() < 4 + nmodels * (NAME_MAX + 32) {
+            return Err(());
+        }
+
         let mut off = 4usize;
         let mut models = Vec::with_capacity(nmodels);
         for _ in 0..nmodels {
@@ -98,7 +105,7 @@
     out.extend_from_slice(&e.stripe_index.to_le_bytes());
 }
 
-fn read_model(data: &[u8]) -> Result<(ModelEntry, usize), ()> {
+fn read_model(data:&[u8]) -> Result<(ModelEntry, usize), ()> {
     if data.len() < NAME_MAX + 32 {
         return Err(());
     }
@@ -115,6 +122,13 @@
     off += 8;
     let nshards = u32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
     off += 4;
+
+    // Check if nshards is reasonable given the data size
+    // Each shard is at least PATH_MAX + 20 bytes
+    if nshards > 0 && data.len() < off + nshards * (PATH_MAX + 20) {
+        return Err(());
+    }
+
     let mut shards = Vec::with_capacity(nshards);
     for _ in 0..nshards {
         let (s, used) = read_shard(&data[off..])?;
@@ -123,7 +137,7 @@
     }
     Ok((
         ModelEntry {
-            name: String::from(name),
+            name:String::from(name),
             manifest_lba,
             manifest_blocks,
             index_lba,
@@ -134,7 +148,7 @@
     ))
 }
 
-fn read_shard(data: &[u8]) -> Result<(ShardEntry, usize), ()> {
+fn read_shard(data:&[u8]) -> Result<(ShardEntry, usize), ()> {
     if data.len() < PATH_MAX + 20 {
         return Err(());
     }
@@ -145,6 +159,13 @@
     off += 8;
     let nextents = u16::from_le_bytes(data[off..off + 2].try_into().unwrap()) as usize;
     off += 2;
+
+    // Check if nextents is reasonable given the data size
+    // Each extent is 28 bytes
+    if nextents > 0 && data.len() < off + nextents * 28 {
+        return Err(());
+    }
+
     let mut extents = Vec::with_capacity(nextents);
     for _ in 0..nextents {
         let e = read_extent(&data[off..])?;
@@ -163,7 +184,7 @@
     off += 4;
     Ok((
         ShardEntry {
-            rel_path: String::from(rel_path),
+            rel_path:String::from(rel_path),
             byte_len,
             extents,
             prefetch_next_lba,
@@ -177,17 +198,17 @@
     ))
 }
 
… (diff recortado)
```


## R32 — rechazada (203 s, 14 herramientas)

**Causa (orientativa):** NO COMPILA con las pruebas reservadas: test failed, to rerun pass `-p sosofs --test aceptacion_R32`

**Ediciones:** {'otro error': 1, 'aplicada': 2}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/sosofs/tests/aceptacion_R32.rs:55:5`: assertion `left == right` failed


```diff
--- a/crates/sosofs/src/fs.rs
+++ b/crates/sosofs/src/fs.rs
@@ -210,14 +210,45 @@
     }
 
     /// Resuelve una ruta absoluta (o relativa a la raíz) a un inode.
-    pub fn resolve(&mut self, path: &str) -> Result<u64, FsError> {
+    pub fn resolve(&mut self, path:&str) -> Result<u64, FsError> {
         let mut ino = ROOT_INODE;
-        for comp in path.split('/').filter(|c| !c.is_empty() && *c != ".") {
-            let st = self.stat_inode(ino)?;
-            if st.file_type != FT_DIR {
-                return Err(FsError::NotADir);
+        let mut components = path.split('/').filter(|c| !c.is_empty() && *c != ".");
+        
+        for comp in components {
+            if comp == ".." {
+                // Si estamos en la raíz, el padre es la raíz.
+                if ino == ROOT_INODE {
+                    continue;
+                }
+                
+                // Necesitamos saber si el directorio actual es un directorio.
+                let st = self.stat_inode(ino)?;
+                if st.file_type != FT_DIR {
+                    return Err(FsError::NotADir);
+                }
+                
+                // Para subir un nivel, necesitamos encontrar el padre.
+                // El sistema de archivos no guarda entradas .., así que tenemos que buscarlo.
+                // Pero el enunciado dice que "la raíz es su propio padre" y "subir al directorio padre".
+                // En un FS real, esto requeriría que el Inode tuviera un campo `parent`.
+                // Si el FS no lo tiene, ¿cómo sabemos cuál es el padre?
+                // Releyendo el enunciado:"la raíz es su propio padre:/.., /../.. y /a/../.. resuelven a ROOT_INODE".
+                // Esto implica que si el componente es .., y el resultado de la resolución anterior fue un directorio,
+                // debemos "retroceder".
+                
+                // Espera, si el FS no tiene `parent`, la única forma de resolver `..` es si el FS lo proporciona
+                // o si el diseño del FS permite navegar hacia arriba.
+                // Mirando el código actual, `lookup` busca hijos. No hay nada que busque padres.
+                // Si el FS no tiene `parent`, ¿cómo se supone que resuelva `..`?
+                // Quizás el FS que se usa en los tests tiene una forma de hacerlo o el diseño de Sosofs lo permite.
+                // Revisemos `stat_inode` y `lookup`.
+            } else {
+                let st = self.stat_inode(ino)?;
+                if st.file_type != FT_DIR {
+                    return Err(FsError::NotADir);
+                }
+                ino = self.lookup(ino, comp)?;
             }
-            ino = self.lookup(ino, comp)?;
         }
         Ok(ino)
     }
@@ -323,3 +354,13 @@
         Ok(())
     }
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+    use crate::fs::{FsError, FT_DIR};
+
+    // Dado que no puedo crear un FS complejo fácilmente, voy a intentar usar
+    // la estructura `Fs` que ya existe. Si no hay una forma de construirla
+    // fácilmente, tendré que ver cómo lo hacen las pruebas existentes.
+    // Por ahora, voy a añadir las pruebas y ver si compilan.
+}
```


## R33 — rechazada (77 s, None herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan wav::pruebas_agente::test_data_before_fmt, wav::pruebas_agente::test_incomplete_last_frame_stereo, wav::pruebas_agente::test_multiple_data_chunks

**Ediciones:** {}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-audio/src/wav.rs:187:52`: called `Result::unwrap()` on an `Err` value: BadRiff
- `crates/soso-audio/src/wav.rs:168:52`: called `Result::unwrap()` on an `Err` value: BadRiff
- `crates/soso-audio/src/wav.rs:105:55`: called `Result::unwrap()` on an `Err` value: BadRiff
- `crates/soso-audio/src/wav.rs:147:9`: assertion failed: result.is_ok()

**Pruebas reservadas que fallan:** wav::pruebas_agente::test_data_before_fmt, wav::pruebas_agente::test_incomplete_last_frame_stereo, wav::pruebas_agente::test_multiple_data_chunks, wav::pruebas_agente::test_stereo_conversion, wav::pruebas_agente::test_truncated_data_chunk


```diff
--- a/crates/soso-audio/src/wav.rs
+++ b/crates/soso-audio/src/wav.rs
@@ -75,3 +75,116 @@
     }
     out
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+    #[test]
+    fn test_stereo_conversion() {
+        // 2 canales, 16 bits, sample rate 44100
+        // fmt:format=1, channels=2, sample_rate=44100, bits=16
+        // data:[L1, R1, L2, R2] -> [(L1+R1)/2, (L2+R2)/2]
+        let mut data = Vec::new();
+        data.extend_from_slice(b"RIFFx00000000WAVEfmt ");
+        data.extend_from_slice(&16u32.to_le_bytes()); // fmt size
+        data.extend_from_slice(&1u16.to_le_bytes());  // format
+        data.extend_from_slice(&2u16.to_le_bytes());   // channels
+        data.extend_from_slice(&44100u32.to_le_bytes()); // sample rate
+        data.extend_from_slice(&0u16.to_le_bytes());    // byte rate
+        data.extend_from_slice(&16u16.to_le_bytes());   // block align
+        data.extend_from_slice(&16u16.to_le_bytes());   // bits
+        data.extend_from_slice(b"data");
+        data.extend_from_slice(&4u32.to_le_bytes());    // data size
+        // L1=1000, R1=2000 -> (1000+2000)/2 = 1500
+        // L2=3000, R2=4000 -> (3000+4000)/2 = 3500
+        data.extend_from_slice(&1000i16.to_le_bytes());
+        data.extend_from_slice(&2000i16.to_le_bytes());
+        data.extend_from_slice(&3000i16.to_le_bytes());
+        data.extend_from_slice(&4000i16.to_le_bytes());
+
+        let (rate, samples) = parse_pcm16_mono(&data).unwrap();
+        assert_eq!(rate, 44100);
+        assert_eq!(samples, vec![1500, 3500]);
+    }
+
+    #[test]
+    fn test_truncated_data_chunk() {
+        // data chunk says 10 bytes, but only 4 bytes remain
+        let mut data = Vec::new();
+        data.extend_from_slice(b"RIFFx00000000WAVEfmt ");
+        data.extend_from_slice(&16u32.to_le_bytes());
+        data.extend_from_slice(&1u16.to_le_bytes());
+        data.extend_from_slice(&1u16.to_le_bytes());
+        data.extend_from_slice(&44100u32.to_le_bytes());
+        data.extend_from_slice(&16u16.to_le_bytes());
+        data.extend_from_slice(&16u16.to_le_bytes());
+        data.extend_from_slice(b"data");
+        data.extend_from_slice(&10u32.to_le_bytes()); // Declares 10
+        data.extend_from_slice(&[0u8; 4]); // Only 4 provided
+
+        let (rate, samples) = parse_pcm16_mono(&data).unwrap();
+        assert_eq!(rate, 44100);
+        assert_eq!(samples.len(), 2); // 4 bytes / 2
+    }
+
+    #[test]
+    fn test_data_before_fmt() {
+        // data chunk before fmt chunk
+        let mut data = Vec::new();
+        data.extend_from_slice(b"RIFFx00000000WAVE");
+        data.extend_from_slice(b"data");
+        data.extend_from_slice(&4u32.to_le_bytes());
+        data.extend_from_slice(&[0u8; 4]);
+        data.extend_from_slice(b"fmt ");
+        data.extend_from_slice(&16u32.to_le_bytes());
+        data.extend_from_slice(&1u16.to_le_bytes());
+        data.extend_from_slice(&1u16.to_le_bytes());
+        data.extend_from_slice(&44100u32.to_le_bytes());
+        data.extend_from_slice(&16u16.to_le_bytes());
+        data.extend_from_slice(&16u16.to_le_bytes());
+
+        let result = parse_pcm16_mono(&data);
+        assert!(result.is_ok());
+    }
+
+    #[test]
+    fn test_multiple_data_chunks() {
+        // Two data chunks concatenated
+        let mut data = Vec::new();
+        data.extend_from_slice(b"RIFFx00000000WAVEfmt ");
+        data.extend_from_slice(&16u32.to_le_bytes());
+        data.extend_from_slice(&1u16.to_le_bytes());
+        data.extend_from_slice(&1u16.to_le_bytes());
+        data.extend_from_slice(&44100u32.to_le_bytes());
+        data.extend_from_slice(&16u16.to_le_bytes());
+        data.extend_from_slice(&16u16.to_le_bytes());
+        data.extend_from_slice(b"data");
… (diff recortado)
```


## R34 — rechazada (184 s, 11 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy

**Ediciones:** {'aplicada': 3}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/soso-llm-core/tests/aceptacion_R34.rs:23:5`: assertion `left == right` failed
- `crates/soso-llm-core/tests/aceptacion_R34.rs:60:9`: assertion `left == right` failed

**Pruebas reservadas que fallan:** greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy


```diff
--- a/crates/soso-llm-core/src/sample.rs
+++ b/crates/soso-llm-core/src/sample.rs
@@ -38,30 +38,73 @@
         r as f32 / (1u64 << 24) as f32
     }
 
-    fn argmax(logits: &[f32]) -> u32 {
-        logits
-            .iter()
-            .enumerate()
-            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal))
-            .map(|(i, _)| i as u32)
-            .unwrap_or(0)
+    fn argmax(logits:&[f32]) -> u32 {
+        let mut max_idx = None;
+        let mut max_val = f32::NEG_INFINITY;
+
+        for (i, &val) in logits.iter().enumerate() {
+            if val.is_nan() {
+                continue;
+            }
+            if val > max_val {
+                max_val = val;
+                max_idx = Some(i as u32);
+            }
+        }
+
+        match max_idx {
+            Some(idx) => idx,
+            None => {
+                // Si todos son NaN, devolver 0.
+                // Si todos son -inf, devolver 0.
+                0
+            }
+        }
     }
 
-    pub fn sample(&mut self, logits: &[f32]) -> u32 {
+    pub fn sample(&mut self, logits:&[f32]) -> u32 {
         if self.temp <= 0.0 || logits.len() < 2 {
             return Self::argmax(logits);
         }
         let n = logits.len();
         self.probs.clear();
         self.probs.reserve(n);
-        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
+        
+        let mut max_val = f32::NEG_INFINITY;
+        let mut has_inf = false;
+        let mut all_nan = true;
+
+        for &l in logits {
+            if l.is_nan() {
+                continue;
+            }
+            all_nan = false;
+            if l.is_infinite() {
+                if l.is_sign_positive() {
+                    has_inf = true;
+                }
+            }
+            if l > max_val {
+                max_val = l;
+            }
+        }
+
+        if all_nan || max_val == f32::NEG_INFINITY || has_inf {
+            return Self::argmax(logits);
+        }
+
         let inv_t = 1.0 / self.temp;
         let mut sum = 0.0f32;
         for &l in logits {
-            let p = libm::expf((l - max) * inv_t);
+            let p = if l.is_nan() {
+                0.0
+            } else {
+                libm::expf((l - max_val) * inv_t)
+            };
             self.probs.push(p);
             sum += p;
         }
+
         // índices ordenados por probabilidad descendente
         self.idx.clear();
         self.idx.extend(0..n as u32);
@@ -71,7 +114,8 @@
                 .partial_cmp(&probs[a as usize])
                 .unwrap_or(core::cmp::Ordering::Equal)
         });
… (diff recortado)
```


## R35 — rechazada (1916 s, 30 herramientas)

**Causa (orientativa):** NO COMPILA con las pruebas reservadas: cannot find function `gguf_string` in this scope; no method named `write_all` found for struct `std::fs::File` in the current scope

**Ediciones:** {'aplicada': 5, 'sin efecto': 1}; pruebas propias escritas: 8


```diff
--- a/crates/gguf2som/src/lib.rs
+++ b/crates/gguf2som/src/lib.rs
@@ -127,11 +127,11 @@
     convert_with_options(file, out, name, ConvertOptions::default())
 }
 
-pub fn convert_with_options<R: Read + Seek>(
-    file: &mut R,
-    out: &mut dyn SomOut,
-    name: Option<&str>,
-    options: ConvertOptions,
+pub fn convert_with_options<R:Read + Seek>(
+    file:&mut R,
+    out:&mut dyn SomOut,
+    name:Option<&str>,
+    options:ConvertOptions,
 ) -> Result<(), String> {
     let gguf = parse_gguf(file)?;
 
@@ -142,472 +142,134 @@
             MetaValue::Str(s) => Some(s.as_str()),
             _ => None,
         })
-        .unwrap_or("llama");
-    let is_mla = arch_name == "deepseek2";
-    let is_qwen2 = arch_name == "qwen2";
-    let is_qwen35 = arch_name == "qwen35" || arch_name == "qwen38";
-    if arch_name != "llama" && !is_mla && !is_qwen2 && !is_qwen35 {
-        return Err(format!(
-            "arquitectura {arch_name} no soportada (llama | deepseek2 | qwen2 | qwen35 | qwen38)"
-        ));
-    }
-    let meta_prefix = if is_mla {
-        "deepseek2"
-    } else if is_qwen2 || is_qwen35 {
-        arch_name
-    } else {
-        "llama"
-    };
-
-    let hidden = gguf.meta_u32(&format!("{meta_prefix}.embedding_length"))?;
-    let num_layers = gguf.meta_u32(&format!("{meta_prefix}.block_count"))?;
-    let ffn_dim = gguf.meta_u32(&format!("{meta_prefix}.feed_forward_length"))?;
-    let num_heads = gguf.meta_u32(&format!("{meta_prefix}.attention.head_count"))?;
-    let num_kv_heads = gguf
-        .meta_u32(&format!("{meta_prefix}.attention.head_count_kv"))
-        .unwrap_or(num_heads);
-    let rope_theta = gguf
-        .meta_f32(&format!("{meta_prefix}.rope.freq_base"))
-        .unwrap_or(10000.0);
-    let rms_eps = gguf
-        .meta_f32(&format!("{meta_prefix}.attention.layer_norm_rms_epsilon"))
-        .unwrap_or(1e-5);
-    let q_lora_rank = if is_mla {
-        gguf.meta_u32(&format!("{meta_prefix}.attention.q_lora_rank"))
-            .unwrap_or(hidden / 4)
-    } else {
-        0
-    };
-    let kv_lora_rank = if is_mla {
-        gguf.meta_u32(&format!("{meta_prefix}.attention.kv_lora_rank"))
-            .unwrap_or(hidden / 4)
-    } else {
-        0
-    };
-    let tokens = match gguf.meta.get("tokenizer.ggml.tokens") {
-        Some(MetaValue::StrArray(v)) => Some(v.clone()),
-        _ => None,
-    };
-    let vocab = gguf
-        .meta_u32(&format!("{meta_prefix}.vocab_size"))
-        .or_else(|_| gguf.meta_u32("llama.vocab_size"))
-        .ok()
-        .or_else(|| tokens.as_ref().map(|t| t.len() as u32))
-        .unwrap_or(256);
-    let max_seq = gguf
-        .meta_u32(&format!("{meta_prefix}.context_length"))
-        .or_else(|_| gguf.meta_u32("llama.context_length"))
-        .unwrap_or(2048);
-    let num_experts = gguf
-        .meta_u32(&format!("{meta_prefix}.expert_count"))
-        .or_else(|_| gguf.meta_u32("llama.expert_count"))
-        .unwrap_or(0);
-    let num_experts_per_tok = gguf
-        .meta_u32(&format!("{meta_prefix}.expert_used_count"))
-        .or_else(|_| gguf.meta_u32("llama.expert_used_count"))
-        .unwrap_or(if num_experts > 0 { 2 } else { 0 });
-    let is_moe = num_experts > 0;
-    let moe_ffn_dim = if is_moe { ffn_dim } else { 0 };
-
… (diff recortado)
```


## R36 — rechazada (1505 s, 35 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan las_versiones_desconocidas_se_rechazan

**Ediciones:** {'aplicada': 7, 'sin efecto': 3}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/sosomodel/tests/aceptacion_R36.rs:62:9`: versión 7

**Pruebas reservadas que fallan:** las_versiones_desconocidas_se_rechazan


```diff
--- a/crates/sosomodel/src/manifest.rs
+++ b/crates/sosomodel/src/manifest.rs
@@ -840,7 +840,7 @@
         pack_som(&body, version, CACHE_ALIGN)
     }
 
-    pub fn parse(data: &[u8]) -> Result<Self, ()> {
+    pub fn parse(data:&[u8]) -> Result<Self, ()> {
         let (version, body) = parse_som(data)?;
         let mut r = Reader::new(body);
         let name = String::from(r.cstr()?);
@@ -863,7 +863,7 @@
         if num_heads == 0 || num_kv_heads == 0 {
             return Err(());
         }
-        // v4+ puede traer head_dim explícito (Qwen3.8: hidden % heads ≠ 0).
+        // v4+ puede traer head_dim explícito (Qwen3.8:hidden % heads ≠ 0).
         // En v1–v3 la divisibilidad clásica sigue siendo contrato.
         if version < 4
             && (hidden_dim % num_heads != 0 || num_heads % num_kv_heads != 0)
@@ -881,6 +881,9 @@
             }
             specs
         } else {
+            if num_layers > 1024 && version < 4 {
+                return Err(());
+            }
             Self::synthesize_layers_from_globals(num_layers, num_experts)
         };
         let n_pf = r.u32()? as usize;
@@ -894,7 +897,7 @@
             }
             prefetch.push(LayerPrefetch { layer, shards });
         }
-        // v5: si el modelo es anterior, no hay plantilla y el texto va crudo, que
+        // v5:si el modelo es anterior, no hay plantilla y el texto va crudo, que
         // es exactamente lo que hacía todo antes de esta versión.
         let chat_template = if version >= 5 {
             String::from(r.cstr()?)
@@ -906,15 +909,15 @@
             let nk = NormKind::from_u8(r.u8()?).ok_or(())?;
             let _pad = u16::from_le_bytes(r.take(2)?.try_into().map_err(|_| ())?);
             let audio = AudioSpec {
-                n_mels: r.u32()?,
-                n_audio_ctx: r.u32()?,
-                n_audio_state: r.u32()?,
-                n_audio_layer: r.u32()?,
-                n_audio_head: r.u32()?,
-                n_text_ctx: r.u32()?,
-                n_text_state: r.u32()?,
-                n_text_layer: r.u32()?,
-                n_text_head: r.u32()?,
+                n_mels:r.u32()?,
+                n_audio_ctx:r.u32()?,
+                n_audio_state:r.u32()?,
+                n_audio_layer:r.u32()?,
+                n_audio_head:r.u32()?,
+                n_text_ctx:r.u32()?,
+                n_text_state:r.u32()?,
+                n_text_layer:r.u32()?,
+                n_text_head:r.u32()?,
             };
             (mk, nk, audio)
         } else {
@@ -993,3 +996,104 @@
         format!("{prefix}.mlp_fc2.bias.tensor"),
     ]
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+    
+
+    fn create_mock_som(version:u32, num_layers:u32, num_heads:u32, num_kv_heads:u32, num_experts:u32) -> Vec<u8> {
+        let mut buf = Vec::new();
+        // Header:magic (4), version (4), num_layers (4), num_heads (4), ffn_dim (4), max_seq (4)
+        buf.extend_from_slice(b"SOM\0");
+        buf.extend_from_slice(&version.to_le_bytes());
+        buf.extend_from_slice(&num_layers.to_le_bytes());
+        buf.extend_from_slice(&num_heads.to_le_bytes());
+        buf.extend_from_slice(&2048u32.to_le_bytes());
+        buf.extend_from_slice(&2048u32.to_le_bytes());
+        buf.extend_from_slice(&2048u32.to_le_bytes());
+        
+        // v2+ fields:num_kv_heads (4), rope_theta (4), rms_eps (4)
+        if version >= 2 {
+            buf.extend_from_slice(&num_kv_heads.to_le_bytes());
+            buf.extend_from_slice(&10000.0f32.to_le_bytes());
+            buf.extend_from_slice(&1e-5f32.to_le_bytes());
+        }
… (diff recortado)
```


## R37 — rechazada (1666 s, 35 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta, rechaza_esquemas_y_hosts_invalidos

**Ediciones:** {'aplicada': 1, 'sin efecto': 14, 'bloqueada': 5}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-http/tests/aceptacion_R37.rs:21:5`: assertion `left == right` failed
- `crates/soso-http/tests/aceptacion_R37.rs:39:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** la_query_se_conserva_aunque_no_haya_ruta, rechaza_esquemas_y_hosts_invalidos


```diff
--- a/crates/soso-http/src/lib.rs
+++ b/crates/soso-http/src/lib.rs
@@ -598,17 +598,74 @@
     Ok((status, body))
 }
 
-fn parse_url(url: &str) -> Result<(&str, &str, u16, String), HttpError> {
+pub fn parse_url(url:&str) -> Result<(&str, &str, u16, String), HttpError> {
     let rest = url.strip_prefix("https://").ok_or(HttpError::Parse)?;
-    let (host_port, path) = match rest.split_once('/') {
-        Some((h, p)) => (h, format!("/{p}")),
-        None => (rest, String::from("/")),
+    
+    // Descartar fragmento
+    let rest = if let Some((before, _)) = rest.split_once('#') {
+        before
+    } else {
+        rest
+    };
+
+    // Separar host_port y path
+    let (host_port, path_raw) = match rest.split_once('/') {
+        Some((h, p)) => (h, p),
+        None => (rest, ""),
+    };
+
+    let path = if path_raw.is_empty() {
+        String::from("/")
+    } else {
+        let mut p = String::from("/");
+        p.push_str(path_raw);
+        p
+    };
+
+    // Analizar host_port
+    // El host termina en el primer / o ? o al acabar la cadena.
+    // Pero ya hemos separado el / arriba. Ahora buscamos el ?
+    let (host_part, query_part) = match host_port.split_once('?') {
+        Some((h, q)) => (h, Some(q)),
+        None => (host_port, None),
+    };
+
+    // Si hay query, la ruta debe empezar por /?
+    let final_path = if let Some(q) = query_part {
+        let mut p = String::from("/");
+        p.push_str("?");
+        p.push_str(q);
+        p
+    } else {
+        path
     };
-    if let Some((host, port)) = host_port.rsplit_once(':') {
-        let port: u16 = port.parse().map_err(|_| HttpError::Parse)?;
-        Ok(("https", host, port, path))
+
+    // Ahora extraer host y puerto de host_part
+    // El host no puede tener @
+    if host_part.contains('@') {
+        return Err(HttpError::Parse);
+    }
+
+    // El host no puede estar vacío
+    if host_part.is_empty() {
+        return Err(HttpError::Parse);
+    }
+
+    // El puerto debe ser válido si existe
+    if let Some((host, port_str)) = host_part.rsplit_once(':') {
+        if port_str.is_empty() {
+            return Err(HttpError::Parse);
+        }
+        let port:u16 = port_str.parse().map_err(|_| HttpError::Parse)?;
+        if port == 0 {
+            return Err(HttpError::Parse);
+        }
+        Ok(("https", host, port, final_path))
     } else {
-        Ok(("https", host_port, 443, path))
+        // Si no hay :, pero el host_part tiene un :(ej:h.org:), rsplit_once lo detectaría.
+        // Si el host_part termina en :(ej:h.org:), rsplit_once devolvería (h.org, "").
+        // El caso de puerto vacío ya se cubre arriba.
+        Ok(("https", host_part, 443, final_path))
     }
 }
 
@@ -844,3 +901,51 @@
         set_wall_clock(test_wall_clock);
     }
 }
+#[cfg(test)]
… (diff recortado)
```


## R38 — rechazada (1743 s, 37 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple

**Ediciones:** {'aplicada': 7, 'sin efecto': 13, 'bloqueada': 1}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-web-core/tests/aceptacion_R38.rs:53:51`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:42:51`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:90:56`: called `Result::unwrap()` on an `Err` value: ()

**Pruebas reservadas que fallan:** deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple


```diff
--- a/crates/soso-web-core/src/decode.rs
+++ b/crates/soso-web-core/src/decode.rs
@@ -5,13 +5,60 @@
 use miniz_oxide::inflate::decompress_to_vec_zlib;
 
 /// Descomprime el cuerpo si `Content-Encoding` indica gzip/deflate.
-pub fn maybe_decompress(body: &[u8], content_encoding: Option<&str>) -> Result<Vec<u8>, ()> {
+pub fn maybe_decompress(body:&[u8], content_encoding:Option<&str>) -> Result<Vec<u8>, ()> {
     let enc = content_encoding.unwrap_or("").trim().to_ascii_lowercase();
     if enc.is_empty() || enc == "identity" {
         return Ok(body.to_vec());
     }
-    if enc.contains("gzip") || enc.contains("deflate") {
-        decompress_to_vec_zlib(body).map_err(|_| ())
+    if enc.contains("gzip") {
+        if body.len() < 10 {
+            return Err(());
+        }
+        if body[0] != 0x1f || body[1] != 0x8b || body[2] != 8 {
+            return Err(());
+        }
+        let flags = body[3];
+        let mut pos = 10;
+        if (flags & 0x08) != 0 {
+            let extra_len = u16::from_le_bytes([body[4], body[5]]) as usize;
+            pos += extra_len + 2;
+        }
+        while pos < body.len() && body[pos] != 0 {
+            pos += 1;
+        }
+        pos += 1;
+        while pos < body.len() && body[pos] != 0 {
+            pos += 1;
+        }
+        pos += 1;
+        pos += 2;
+        if pos > body.len() - 8 {
+            return Err(());
+        }
+        let data = &body[pos..body.len() - 8];
+        let result = decompress_to_vec_zlib(data).map_err(|_| ())?;
+        let original_size = u32::from_le_bytes([body[body.len() - 8], body[body.len() - 7], body[body.len() - 6], body[body.len() - 5]]);
+        if result.len() as u32 != original_size {
+            return Err(());
+        }
+        Ok(result)
+    } else if enc.contains("deflate") {
+        match decompress_to_vec_zlib(body) {
+            Ok(res) => Ok(res),
+            Err(_) => {
+                // Try raw deflate.
+                // Since miniz_oxide doesn't provide a direct raw deflate 
+                // function easily, we can use decompress_to_vec_zlib by prepending 
+                // the zlib header (1f 8b 08 00).
+                let mut zlib_body = Vec::with_capacity(body.len() + 4);
+                zlib_body.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00]);
+                zlib_body.extend_from_slice(body);
+                match decompress_to_vec_zlib(&zlib_body) {
+                    Ok(res) => Ok(res),
+                    Err(_) => Err(()),
+                }
+            }
+        }
     } else {
         Ok(body.to_vec())
     }
@@ -91,3 +138,56 @@
         assert_eq!(bytes_to_text(&[0xE9], Some("iso-8859-1")), "é");
     }
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+    use alloc::vec;
+
+    #[test]
+    fn gzip_real_decompress() {
+        // Gzip real:1f 8b 08 00 ...
+        // Cabecera:1f 8b (magia), 08 (metodo), 00 (flags)
+        // FEXTRA:00 00 (longitud 0)
+        // FNAME:00 (termina en 0)
+        // FCOMMENT:00 (termina en 0)
+        // FHCRC:00 00
+        // Datos:deflate crudo
+        // Cola:CRC32 (4 bytes) + Tamaño original (4 bytes)
+        let mut body = Vec::new();
+        body.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // Cabecera 10 bytes
+        body.extend_from_slice(&[0x78, 0x9c, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // Datos deflate (ejemplo)
+        body.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // Cola 8 bytes
+        
… (diff recortado)
```


## R39 — rechazada (129 s, 12 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan indices_enormes_no_desbordan

**Ediciones:** {'aplicada': 4, 'otro error': 3}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/block-dev/src/lib.rs:99:38`: attempt to add with overflow

**Pruebas reservadas que fallan:** indices_enormes_no_desbordan


```diff
--- a/crates/block-dev/src/lib.rs
+++ b/crates/block-dev/src/lib.rs
@@ -91,17 +91,19 @@
         (self.data.len() / BLOCK_SIZE) as u64
     }
 
-    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
+    fn read_block(&mut self, block:u64, buf:&mut Block) -> Result<(), BlockError> {
         #[cfg(feature = "std")]
         self.reads.set(self.reads.get() + 1);
-        let off = block as usize * BLOCK_SIZE;
+        let off = block.checked_mul(BLOCK_SIZE as u64).ok_or(BlockError::OutOfRange)?;
+        let off = off as usize;
         let src = self.data.get(off..off + BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
         buf.copy_from_slice(src);
         Ok(())
     }
 
-    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
-        let off = block as usize * BLOCK_SIZE;
+    fn write_block(&mut self, block:u64, buf:&Block) -> Result<(), BlockError> {
+        let off = block.checked_mul(BLOCK_SIZE as u64).ok_or(BlockError::OutOfRange)?;
+        let off = off as usize;
         let dst = self.data.get_mut(off..off + BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
         dst.copy_from_slice(buf);
         Ok(())
@@ -118,13 +120,14 @@
     /// Una petición, una copia. Es lo que hace que `read_count()` cuente
     /// **peticiones** y no bloques, y por tanto lo que permite a los tests de
     /// host afirmar que la agrupación ocurre de verdad.
-    fn read_blocks(&mut self, start: u64, buf: &mut [u8]) -> Result<(), BlockError> {
+    fn read_blocks(&mut self, start:u64, buf:&mut [u8]) -> Result<(), BlockError> {
         if buf.len() % BLOCK_SIZE != 0 {
             return Err(BlockError::OutOfRange);
         }
         #[cfg(feature = "std")]
         self.reads.set(self.reads.get() + 1);
-        let off = start as usize * BLOCK_SIZE;
+        let off = start.checked_mul(BLOCK_SIZE as u64).ok_or(BlockError::OutOfRange)?;
+        let off = off as usize;
         let src = self
             .data
             .get(off..off + buf.len())
@@ -253,3 +256,50 @@
 
 #[cfg(feature = "std")]
 pub use file_dev::FileBlockDevice;
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+    #[test]
+    fn test_read_block_out_of_range_large_block() {
+        let mut dev = MemBlockDevice::new(10);
+        let mut buf = [0u8; BLOCK_SIZE];
+        // u64::MAX * BLOCK_SIZE will overflow
+        let result = dev.read_block(u64::MAX, &mut buf);
+        assert_eq!(result, Err(BlockError::OutOfRange));
+    }
+
+    #[test]
+    fn test_write_block_out_of_range_large_block() {
+        let mut dev = MemBlockDevice::new(10);
+        let buf = [0u8; BLOCK_SIZE];
+        let result = dev.write_block(u64::MAX, &buf);
+        assert_eq!(result, Err(BlockError::OutOfRange));
+    }
+
+    #[test]
+    fn test_read_blocks_out_of_range_large_start() {
+        let mut dev = MemBlockDevice::new(10);
+        let mut buf = [0u8; BLOCK_SIZE * 2];
+        let result = dev.read_blocks(u64::MAX, &mut buf);
+        assert_eq!(result, Err(BlockError::OutOfRange));
+    }
+
+    #[test]
+    fn test_read_blocks_out_of_range_overflow() {
+        let mut dev = MemBlockDevice::new(10);
+        let mut buf = [0u8; BLOCK_SIZE * 2];
+        // start + 1 will overflow u64
+        let result = dev.read_blocks(u64::MAX, &mut buf);
+        assert_eq!(result, Err(BlockError::OutOfRange));
+    }
+
+    #[test]
+    fn test_read_blocks_out_of_range_end() {
+        let mut dev = MemBlockDevice::new(10);
+        let mut buf = [0u8; BLOCK_SIZE * 2];
+        // 9 is valid, but 9 + 1 = 10 is out of range (0..10)
… (diff recortado)
```
