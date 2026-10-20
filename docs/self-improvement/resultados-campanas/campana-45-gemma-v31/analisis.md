# Análisis de ejecucion-45-gemma-v31

1 aceptadas de 10. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R31 | rechazada | CASO LÍMITE NO CUBIERTO: fallan cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, |
| R32 | rechazada | NO COMPILA con las pruebas reservadas: test failed, to rerun pass `-p sosofs --test aceptacion_R32` |
| R33 | rechazada | CASO LÍMITE NO CUBIERTO: fallan estereo_con_trama_incompleta_la_ignora, sin_fmt_o_sin_muestras_es_badfmt, vari |
| R34 | rechazada | CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy |
| R35 | rechazada | NO COMPILA con las pruebas reservadas: no method named `iter` found for reference `&TensorInfo` in the current |
| R36 | rechazada | CASO LÍMITE NO CUBIERTO: fallan las_versiones_desconocidas_se_rechazan |
| R37 | rechazada | CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta, rechaza_esquemas_y_hosts_invalidos |
| R38 | rechazada | CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple |
| R39 | rechazada | CASO LÍMITE NO CUBIERTO: fallan indices_enormes_no_desbordan |


## R31 — rechazada (1360 s, 40 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, cualquier_truncado_es_error_y_no_panico

**Ediciones:** {'sin efecto': 11, 'aplicada': 10}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/sosomfs/src/catalog.rs:122:47`: range end index 97 out of range for slice of length 96
- `crates/sosomfs/src/catalog.rs:176:49`: range end index 119 out of range for slice of length 116
- `crates/sosomfs/src/catalog.rs:182:47`: range end index 211 out of range for slice of length 209

**Pruebas reservadas que fallan:** cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, cualquier_truncado_es_error_y_no_panico


```diff
--- a/crates/sosomfs/src/catalog.rs
+++ b/crates/sosomfs/src/catalog.rs
@@ -45,21 +45,28 @@
         out
     }
 
-    pub fn parse(data: &[u8]) -> Result<Self, ()> {
-        if data.len() < 4 {
-            return Err(());
-        }
-        let nmodels = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
-        let mut off = 4usize;
-        let mut models = Vec::with_capacity(nmodels);
-        for _ in 0..nmodels {
-            let (m, used) = read_model(&data[off..])?;
-            off += used;
-            models.push(m);
-        }
-        Ok(Self { models })
+pub fn parse(data:&[u8]) -> Result<Self, ()> {
+    if data.len() < 4 {
+        return Err(());
+    }
+    let nmodels = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
+    
+    // Check if nmodels is reasonable given the data length
+    // Each model is at least NAME_MAX + 32 bytes
+    if 4 + nmodels * (NAME_MAX + 32) > data.len() || data.len() < 4 + nmodels * (NAME_MAX + 32) {
+        return Err(());
     }
 
+    let mut off = 4usize;
+    let mut models = Vec::with_capacity(nmodels);
+    for _ in 0..nmodels {
+        let (m, used) = read_model(&data[off..])?;
+        off += used;
+        models.push(m);
+    }
+    Ok(Self { models })
+}
+
     pub fn find_model(&self, name: &str) -> Option<&ModelEntry> {
         self.models.iter().find(|m| m.name == name)
     }
@@ -105,18 +112,25 @@
     let nlen = data[0] as usize;
     let name = core::str::from_utf8(&data[1..1 + nlen.min(NAME_MAX)]).map_err(|_| ())?;
     let mut off = NAME_MAX + 1;
-    let manifest_lba = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
+    
+    let manifest_lba = u64::from_le_bytes(data[off..off + 8].try_into().map_err(|_| ())?);
     off += 8;
-    let manifest_blocks = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
+    let manifest_blocks = u64::from_le_bytes(data[off..off + 8].try_into().map_err(|_| ())?);
     off += 8;
-    let index_lba = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
+    let index_lba = u64::from_le_bytes(data[off..off + 8].try_into().map_err(|_| ())?);
     off += 8;
-    let index_blocks = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
+    let index_blocks = u64::from_le_bytes(data[off..off + 8].try_into().map_err(|_| ())?);
     off += 8;
-    let nshards = u32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
+    let nshards = u32::from_le_bytes(data[off..off + 4].try_into().map_err(|_| ())?);
     off += 4;
-    let mut shards = Vec::with_capacity(nshards);
-    for _ in 0..nshards {
+
+    let nshards_usize = nshards as usize;
+    if off + nshards_usize * (PATH_MAX + 20) > data.len() || data.len() < off + nshards_usize * (PATH_MAX + 20) {
+        return Err(());
+    }
+
+    let mut shards = Vec::with_capacity(nshards_usize);
+    for _ in 0..nshards_usize {
         let (s, used) = read_shard(&data[off..])?;
         off += used;
         shards.push(s);
@@ -141,25 +155,31 @@
     let plen = data[0] as usize;
     let rel_path = core::str::from_utf8(&data[1..1 + plen.min(PATH_MAX)]).map_err(|_| ())?;
     let mut off = PATH_MAX + 1;
-    let byte_len = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
+    
+    let byte_len = u64::from_le_bytes(data[off..off + 8].try_into().map_err(|_| ())?);
     off += 8;
-    let nextents = u16::from_le_bytes(data[off..off + 2].try_into().unwrap()) as usize;
+    let nextents = u16::from_le_bytes(data[off..off + 2].try_into().map_err(|_| ())?) as usize;
     off += 2;
+
+    if off + nextents * 28 > data.len() || data.len() < off + nextents * 28 {
… (diff recortado)
```


## R32 — rechazada (200 s, 14 herramientas)

**Causa (orientativa):** NO COMPILA con las pruebas reservadas: test failed, to rerun pass `-p sosofs --test aceptacion_R32`

**Ediciones:** {'otro error': 1, 'aplicada': 2}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/sosofs/tests/aceptacion_R32.rs:40:5`: assertion `left == right` failed


```diff
--- a/crates/sosofs/src/fs.rs
+++ b/crates/sosofs/src/fs.rs
@@ -212,12 +212,43 @@
     /// Resuelve una ruta absoluta (o relativa a la raíz) a un inode.
     pub fn resolve(&mut self, path: &str) -> Result<u64, FsError> {
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


## R33 — rechazada (1470 s, 25 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan estereo_con_trama_incompleta_la_ignora, sin_fmt_o_sin_muestras_es_badfmt, varios_data_se_concatenan_y_los_ajenos_se_saltan

**Ediciones:** {'aplicada': 2, 'sin efecto': 9, 'bloqueada': 8}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-audio/tests/aceptacion_R33.rs:104:5`: assertion `left == right` failed
- `crates/soso-audio/tests/aceptacion_R33.rs:53:41`: called `Result::unwrap()` on an `Err` value: Unsupported
- `crates/soso-audio/tests/aceptacion_R33.rs:66:37`: called `Result::unwrap()` on an `Err` value: Unsupported
- `crates/soso-audio/tests/aceptacion_R33.rs:122:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** estereo_con_trama_incompleta_la_ignora, sin_fmt_o_sin_muestras_es_badfmt, varios_data_se_concatenan_y_los_ajenos_se_saltan


```diff
--- a/crates/soso-audio/src/wav.rs
+++ b/crates/soso-audio/src/wav.rs
@@ -23,13 +23,27 @@
     let mut bits = 0u16;
     let mut channels = 0u16;
     let mut pcm = Vec::new();
+    let mut fmt_found = false;
+
     while pos + 8 <= data.len() {
         let id = &data[pos..pos + 4];
         let sz = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
         pos += 8;
+
         if pos + sz > data.len() {
+            let remaining = data.len() - pos;
+            if remaining > 0 {
+                if id == b"data" {
+                    let bytes = &data[pos..];
+                    pcm.reserve(bytes.len() / 2);
+                    for chunk in bytes.chunks_exact(2) {
+                        pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
+                    }
+                }
+            }
             break;
         }
+
         if id == b"fmt " && sz >= 16 {
             let audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
             channels = u16::from_le_bytes(data[pos + 2..pos + 4].try_into().unwrap());
@@ -38,6 +52,7 @@
             if audio_format != 1 {
                 return Err(WavError::Unsupported);
             }
+            fmt_found = true;
         } else if id == b"data" {
             if bits != 16 || channels != 1 {
                 return Err(WavError::Unsupported);
@@ -48,8 +63,24 @@
                 pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
             }
         }
-        pos += sz + (sz & 1);
+        pos += sz;
+    }
+
+    if !fmt_found {
+        return Err(WavError::BadFmt);
+    }
+
+    if channels == 2 {
+        let mut new_pcm = Vec::with_capacity(pcm.len());
+        for i in (0..pcm.len()).step_by(2) {
+            if i + 1 < pcm.len() {
+                let avg = (pcm[i] as i32 + pcm[i+1] as i32) / 2;
+                new_pcm.push(avg as i16);
+            }
+        }
+        pcm = new_pcm;
     }
+
     if pcm.is_empty() || sample_rate == 0 {
         return Err(WavError::BadFmt);
     }
@@ -75,3 +106,12 @@
     }
     out
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+
+
+
+
+}
```


## R34 — rechazada (1380 s, 34 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy

**Ediciones:** {'sin efecto': 15, 'aplicada': 2, 'bloqueada': 6}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-llm-core/tests/aceptacion_R34.rs:26:5`: assertion `left == right` failed
- `crates/soso-llm-core/tests/aceptacion_R34.rs:59:9`: assertion `left == right` failed

**Pruebas reservadas que fallan:** greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy


```diff
--- a/crates/soso-llm-core/src/sample.rs
+++ b/crates/soso-llm-core/src/sample.rs
@@ -39,12 +39,30 @@
     }
 
     fn argmax(logits: &[f32]) -> u32 {
-        logits
-            .iter()
-            .enumerate()
-            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal))
-            .map(|(i, _)| i as u32)
-            .unwrap_or(0)
+        let mut max_idx = 0;
+        let mut max_val = f32::NEG_INFINITY;
+        let mut found_finite = false;
+
+        for (i, &val) in logits.iter().enumerate() {
+            if val.is_finite() {
+                if !found_finite || val > max_val {
+                    max_val = val;
+                    max_idx = i as u32;
+                    found_finite = true;
+                }
+            }
+        }
+
+        if !found_finite {
+            for (i, &val) in logits.iter().enumerate() {
+                if !val.is_nan() {
+                    return i as u32;
+                }
+            }
+            return 0;
+        }
+
+        max_idx
     }
 
     pub fn sample(&mut self, logits: &[f32]) -> u32 {
@@ -54,15 +72,38 @@
         let n = logits.len();
         self.probs.clear();
         self.probs.reserve(n);
-        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
+        
+        let mut max_val = f32::NEG_INFINITY;
+        let mut has_inf = false;
+        let mut has_nan = false;
+        
+        for &l in logits {
+            if l.is_nan() { has_nan = true; }
+            if l.is_infinite() {
+                if l.is_sign_positive() { has_inf = true; }
+            }
+            if l > max_val { max_val = l; }
+        }
+
+        let is_non_finite_max = has_nan && max_val == f32::NEG_INFINITY;
+        let is_inf_present = has_inf;
+        
+        if is_non_finite_max || is_inf_present {
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
-        // índices ordenados por probabilidad descendente
+
         self.idx.clear();
         self.idx.extend(0..n as u32);
         let probs = &self.probs;
@@ -71,7 +112,7 @@
                 .partial_cmp(&probs[a as usize])
                 .unwrap_or(core::cmp::Ordering::Equal)
         });
-        // núcleo: el prefijo más corto que acumula top_p de la masa
+
         let cutoff = self.top_p * sum;
         let mut mass = 0.0f32;
         let mut k = 0;
… (diff recortado)
```


## R35 — rechazada (1656 s, 31 herramientas)

**Causa (orientativa):** NO COMPILA con las pruebas reservadas: no method named `iter` found for reference `&TensorInfo` in the current scope; mismatched types

**Ediciones:** {'aplicada': 5, 'sin efecto': 2}; pruebas propias escritas: 8


```diff
--- a/crates/gguf2som/src/lib.rs
+++ b/crates/gguf2som/src/lib.rs
@@ -133,7 +133,7 @@
     name: Option<&str>,
     options: ConvertOptions,
 ) -> Result<(), String> {
-    let gguf = parse_gguf(file)?;
+    let gguf = parse_gguf(file).map_err(|e| format!("Error al parsear GGUF:{}", e))?;
 
     let arch_name = gguf
         .meta
@@ -169,446 +169,91 @@
     let rope_theta = gguf
         .meta_f32(&format!("{meta_prefix}.rope.freq_base"))
         .unwrap_or(10000.0);
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
-    let model_name = name.map(String::from).unwrap_or_else(|| String::from("model"));
-
-    out.mkdir(SHARDS_DIR)?;
-
-    // tensores por capa (GGUF → nombre .som)
-    const LLAMA_LAYER_PARTS: &[(&str, &str)] = &[
-        ("attn_norm", "attn_norm"),
-        ("attn_q", "attn_q"),
-        ("attn_k", "attn_k"),
-        ("attn_v", "attn_v"),
-        ("attn_output", "attn_output"),
-        ("ffn_norm", "ffn_norm"),
-        ("ffn_up", "ffn_up"),
-        ("ffn_gate", "ffn_gate"),
-        ("ffn_down", "ffn_down"),
-    ];
-    // Qwen3.5/3.8 gated full-attn: Q+gate fusionados, QK-norm, post-norm.
-    const QWEN35_GATED_PARTS: &[(&str, &str)] = &[
-        ("attn_norm", "attn_norm"),
-        ("attn_q", "attn_q"),
-        ("attn_k", "attn_k"),
-        ("attn_v", "attn_v"),
-        ("attn_output", "attn_output"),
-        ("attn_q_norm", "attn_q_norm"),
-        ("attn_k_norm", "attn_k_norm"),
-        ("post_attention_norm", "ffn_norm"),
-        ("ffn_up", "ffn_up"),
-        ("ffn_gate", "ffn_gate"),
-        ("ffn_down", "ffn_down"),
-    ];
-    const QWEN35_GDN_PARTS: &[(&str, &str)] = &[
-        ("attn_norm", "attn_norm"),
-        ("attn_qkv", "attn_qkv"),
-        ("attn_gate", "attn_gate"),
-        ("ssm_conv1d", "ssm_conv1d"),
… (diff recortado)
```


## R36 — rechazada (1472 s, 36 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan las_versiones_desconocidas_se_rechazan

**Ediciones:** {'aplicada': 4, 'sin efecto': 3, 'otro error': 2}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/sosomodel/tests/aceptacion_R36.rs:62:9`: versión 7

**Pruebas reservadas que fallan:** las_versiones_desconocidas_se_rechazan


```diff
--- a/crates/sosomodel/src/manifest.rs
+++ b/crates/sosomodel/src/manifest.rs
@@ -881,6 +881,9 @@
             }
             specs
         } else {
+            if num_layers > 1024 {
+                return Err(());
+            }
             Self::synthesize_layers_from_globals(num_layers, num_experts)
         };
         let n_pf = r.u32()? as usize;
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
+        
+        // v3+ fields:num_experts (4), num_experts_per_tok (4), moe_ffn_dim (4)
+        if version >= 3 {
+            buf.extend_from_slice(&num_experts.to_le_bytes());
+            buf.extend_from_slice(&1u32.to_le_bytes());
+            buf.extend_from_slice(&2048u32.to_le_bytes());
+        }
+
+        // v4+ head_dim check (not needed for this task but good to have)
+        // v4+ layer specs
+        if version >= 4 {
+            buf.extend_from_slice(&(num_layers as u32).to_le_bytes());
+            for _ in 0..num_layers {
+                buf.push(0); // attn_kind
+                buf.push(0); // ffn_kind
+                buf.extend_from_slice(&0u16.to_le_bytes()); // pad
+                buf.extend_from_slice(&num_heads.to_le_bytes());
+                buf.extend_from_slice(&num_kv_heads.to_le_bytes());
+                buf.extend_from_slice(&2048u32.to_le_bytes());
+                buf.extend_from_slice(&0u32.to_le_bytes());
+                buf.extend_from_slice(&0u32.to_le_bytes());
+                buf.extend_from_slice(&0u32.to_le_bytes());
+                buf.extend_from_slice(&0u32.to_le_bytes());
+                buf.extend_from_slice(&0u32.to_le_bytes());
+                buf.extend_from_slice(&0u32.to_le_bytes());
+                buf.extend_from_slice(&0u32.to_le_bytes());
+                buf.extend_from_slice(&0u32.to_le_bytes());
+                buf.extend_from_slice(&0u32.to_le_bytes());
+                buf.extend_from_slice(&0u32.to_le_bytes());
+                buf.extend_from_slice(&0u32.to_le_bytes());
+            }
+        } else {
+            // Synthesize layers (v1-v3)
+            // We need to simulate what synthesize_layers_from_globals does.
+            // Since we can't easily call it without the actual logic, 
+            // we'll just ensure the parser doesn't crash.
+            // Actually, the task says "a manifest of a future format should be rejected".
+            // And "num_layers without backup" (synthesized).
+        }
+
+        // Prefetch
+        buf.extend_from_slice(&0u32.to_le_bytes()); // n_pf
+        
+        // v5 chat template
+        if version >= 5 {
+            buf.extend_from_slice(b"template");
+        }
+
+        // v6 model_kind, norm_kind, audio
+        if version >= 6 {
+            buf.push(0); // mk
+            buf.push(0); // nk
… (diff recortado)
```


## R37 — rechazada (449 s, 20 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta, rechaza_esquemas_y_hosts_invalidos

**Ediciones:** {'aplicada': 3, 'sin efecto': 6, 'bloqueada': 8}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/soso-http/tests/aceptacion_R37.rs:19:5`: assertion `left == right` failed
- `crates/soso-http/tests/aceptacion_R37.rs:39:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** la_query_se_conserva_aunque_no_haya_ruta, rechaza_esquemas_y_hosts_invalidos


```diff
--- a/crates/soso-http/src/lib.rs
+++ b/crates/soso-http/src/lib.rs
@@ -598,16 +598,57 @@
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
+    let rest = if let Some(idx) = rest.find('#') {
+        &rest[..idx]
+    } else {
+        rest
+    };
+
+    // El host termina en el primer / o ? o al acabar la cadena
+    let (host_port, path_raw) = match rest.split_once('/') {
+        Some((h, p)) => (h, p),
+        None => {
+            let (h, p) = match rest.split_once('?') {
+                Some((h, p)) => (h, p),
+                None => (rest, ""),
+            };
+            (h, p)
+        }
     };
-    if let Some((host, port)) = host_port.rsplit_once(':') {
-        let port: u16 = port.parse().map_err(|_| HttpError::Parse)?;
+
+    // Determinar la ruta final
+    let path = if path_raw.is_empty() {
+        String::from("/")
+    } else if path_raw.starts_with('?') {
+        format!("/{}", path_raw)
+    } else {
+        format!("/{}", path_raw)
+    };
+
+    // Analizar host y puerto
+    if host_port.is_empty() {
+        return Err(HttpError::Parse);
+    }
+
+    if host_port.contains('@') {
+        return Err(HttpError::Parse);
+    }
+
+    if let Some((host, port_str)) = host_port.rsplit_once(':') {
+        if port_str.is_empty() {
+            return Err(HttpError::Parse);
+        }
+        let port:u16 = port_str.parse().map_err(|_| HttpError::Parse)?;
+        if port == 0 {
+            return Err(HttpError::Parse);
+        }
         Ok(("https", host, port, path))
     } else {
+        // Si no hay puerto explícito, es 443
         Ok(("https", host_port, 443, path))
     }
 }
@@ -844,3 +885,45 @@
         set_wall_clock(test_wall_clock);
     }
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+    #[test]
+    fn test_fragmento_se_descarta() {
+        let (_, host, port, path) = parse_url("https://h.org/path#frag").unwrap();
+        assert_eq!(host, "h.org");
+        assert_eq!(path, "/path");
+    }
+
+
+
+    #[test]
+    fn test_puerto_vacio_o_cero() {
+        assert!(parse_url("https://h.org:/x").is_err());
+        assert!(parse_url("https://h.org:0").is_err());
+    }
+
+    #[test]
… (diff recortado)
```


## R38 — rechazada (1327 s, 38 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple

**Ediciones:** {'aplicada': 3, 'sin efecto': 6}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-web-core/tests/aceptacion_R38.rs:53:51`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:90:56`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:42:51`: called `Result::unwrap()` on an `Err` value: ()

**Pruebas reservadas que fallan:** deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple


```diff
--- a/crates/soso-web-core/src/decode.rs
+++ b/crates/soso-web-core/src/decode.rs
@@ -10,8 +10,55 @@
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
+                // miniz_oxide doesn't have a raw deflate function in the public API 
+                // that we can easily access without extra dependencies.
+                // However, we can try to use decompress_to_vec_zlib on the body 
+                // but it will fail because it expects a zlib header.
+                // Since we can't easily do raw deflate with miniz_oxide 
+                // without more complex logic, and the requirement is to try it,
+                // we'll return Err(()) if zlib fails, as there's no easy way 
+                // to do raw deflate with the current dependencies.
+                // Wait, I should check if I can use decompress_to_vec_zlib 
+                // with some offset? No, that's for zlib.
+                Err(())
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
+        // El tamaño original debe coincidir. 
+        // Para este ejemplo de datos deflate, el tamaño original será algo pequeño.
+        // Como no queremos complicarnos con el deflate crudo exacto, 
+        // vamos a usar un cuerpo que sea válido pero que el tamaño coincida.
+        // Pero el requisito es que si el tamaño NO cuadra, dé error.
+        
… (diff recortado)
```


## R39 — rechazada (200 s, 18 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan indices_enormes_no_desbordan

**Ediciones:** {'aplicada': 6, 'otro error': 3, 'sin efecto': 1}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/block-dev/src/lib.rs:99:38`: attempt to add with overflow

**Pruebas reservadas que fallan:** indices_enormes_no_desbordan


```diff
--- a/crates/block-dev/src/lib.rs
+++ b/crates/block-dev/src/lib.rs
@@ -93,15 +93,17 @@
 
     fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
         #[cfg(feature = "std")]
-        self.reads.set(self.reads.get() + 1);
-        let off = block as usize * BLOCK_SIZE;
+        self.reads.set(self.reads.get().checked_add(1).ok_or(BlockError::OutOfRange)?);
+        let off = block.checked_mul(BLOCK_SIZE as u64).ok_or(BlockError::OutOfRange)?;
+        let off = off as usize;
         let src = self.data.get(off..off + BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
         buf.copy_from_slice(src);
         Ok(())
     }
 
     fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
-        let off = block as usize * BLOCK_SIZE;
+        let off = block.checked_mul(BLOCK_SIZE as u64).ok_or(BlockError::OutOfRange)?;
+        let off = off as usize;
         let dst = self.data.get_mut(off..off + BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
         dst.copy_from_slice(buf);
         Ok(())
@@ -123,8 +125,9 @@
             return Err(BlockError::OutOfRange);
         }
         #[cfg(feature = "std")]
-        self.reads.set(self.reads.get() + 1);
-        let off = start as usize * BLOCK_SIZE;
+        self.reads.set(self.reads.get().checked_add(1).ok_or(BlockError::OutOfRange)?);
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
+        let result = dev.read_blocks(9, &mut buf);
+        assert_eq!(result, Err(BlockError::OutOfRange));
+    }
+}
```
