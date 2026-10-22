# Análisis de ejecucion-48-gemma-v35

2 aceptadas de 10. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R31 | rechazada | CASO LÍMITE NO CUBIERTO: fallan cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, |
| R32 | rechazada | CASO LÍMITE NO CUBIERTO: fallan la_raiz_es_su_propio_padre, punto_punto_sube_al_padre |
| R33 | rechazada | NO COMPILA con las pruebas reservadas: type annotations needed for `Vec<_>`; type annotations needed for `std: |
| R34 | rechazada | CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy |
| R35 | rechazada | CASO LÍMITE NO CUBIERTO: fallan tests::convierte_deepseek2_mla_sintetico, tests::convierte_gguf_qwen35_gdn_y_g |
| R36 | rechazada | CASO LÍMITE NO CUBIERTO: fallan las_versiones_desconocidas_se_rechazan |
| R37 | rechazada | CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta, rechaza_esquemas_y_hosts_invalidos |
| R38 | rechazada | CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple |


## R31 — rechazada (1666 s, 40 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, cualquier_truncado_es_error_y_no_panico

**Ediciones:** {'sin efecto': 17, 'aplicada': 3}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/sosomfs/src/catalog.rs:125:47`: range end index 97 out of range for slice of length 96
- `crates/sosomfs/src/catalog.rs:181:49`: range end index 119 out of range for slice of length 116
- `crates/sosomfs/src/catalog.rs:187:47`: range end index 211 out of range for slice of length 209
- `crates/sosomfs/tests/aceptacion_R31.rs:59:45`: called `Result::unwrap()` on an `Err` value: ()

**Pruebas reservadas que fallan:** cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, cualquier_truncado_es_error_y_no_panico, ida_y_vuelta_sigue_funcionando


```diff
--- a/crates/sosomfs/src/catalog.rs
+++ b/crates/sosomfs/src/catalog.rs
@@ -50,6 +50,12 @@
             return Err(());
         }
         let nmodels = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
+        
+        // Check if nmodels is reasonable:each model is at least NAME_MAX + 32 bytes
+        if nmodels > 0 && data.len() < 4 + nmodels * (NAME_MAX + 32) {
+            return Err(());
+        }
+
         let mut off = 4usize;
         let mut models = Vec::with_capacity(nmodels);
         for _ in 0..nmodels {
@@ -57,6 +63,11 @@
             off += used;
             models.push(m);
         }
+
+        if off != data.len() {
+            return Err(());
+        }
+
         Ok(Self { models })
     }
 
@@ -115,6 +126,13 @@
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
@@ -145,6 +163,13 @@
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
@@ -218,3 +243,74 @@
     }
     Ok(data)
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+
+    #[test]
+    fn test_impossible_model_count_fails() {
+        let mut data = Vec::new();
+        data.extend_from_slice(&u32::MAX.to_le_bytes()); // Impossible number of models
+        assert!(Catalog::parse(&data).is_err());
+    }
+
+
+    #[test]
+    fn test_impossible_extent_count_fails() {
+        let mut data = Vec::new();
+        data.extend_from_slice(&1u32.to_le_bytes());
+        let name = b"m1";
+        data.push(name.len() as u8);
+        data.extend_from_slice(name);
+        data.resize(NAME_MAX + 1, 0);
+        data.extend_from_slice(&0u64.to_le_bytes());
+        data.extend_from_slice(&0u64.to_le_bytes());
+        data.extend_from_slice(&0u64.to_le_bytes());
+        data.extend_from_slice(&0u64.to_le_bytes());
+        data.extend_from_slice(&1u32.to_le_bytes()); // 1 shard
+        let path = b"p1";
+        data.push(path.len() as u8);
+        data.extend_from_slice(path);
+        data.resize(PATH_MAX + 1, 0);
+        data.extend_from_slice(&0u64.to_le_bytes());
… (diff recortado)
```


## R32 — rechazada (959 s, 37 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan la_raiz_es_su_propio_padre, punto_punto_sube_al_padre

**Ediciones:** {'aplicada': 1}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/sosofs/tests/aceptacion_R32.rs:55:5`: assertion `left == right` failed
- `crates/sosofs/tests/aceptacion_R32.rs:40:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** la_raiz_es_su_propio_padre, punto_punto_sube_al_padre


```diff
--- a/crates/sosofs/src/fs.rs
+++ b/crates/sosofs/src/fs.rs
@@ -212,12 +212,33 @@
     /// Resuelve una ruta absoluta (o relativa a la raíz) a un inode.
     pub fn resolve(&mut self, path: &str) -> Result<u64, FsError> {
         let mut ino = ROOT_INODE;
-        for comp in path.split('/').filter(|c| !c.is_empty() && *c != ".") {
-            let st = self.stat_inode(ino)?;
-            if st.file_type != FT_DIR {
-                return Err(FsError::NotADir);
+        let components:Vec<&str> = path.split('/').filter(|c| !c.is_empty() && *c != ".").collect();
+        
+        for comp in components {
+            if comp == ".." {
+                if ino == ROOT_INODE {
+                    continue;
+                }
+                let st = self.stat_inode(ino)?;
+                if st.file_type != FT_DIR {
+                    return Err(FsError::NotADir);
+                }
+                // Since we don't have a parent pointer, we have to find the parent.
+                // But the FS structure doesn't seem to support navigating up.
+                // Wait, the task says:"la raíz es su propio padre:/.., /../.. y /a/../.. resuelven a ROOT_INODE"
+                // This means we need to keep track of the path taken to find the parent?
+                // No, that's too complex. Let's re-read.
+                // "la raíz es su propio padre"
+                // If the FS doesn't have parent pointers, how can we go up?
+                // Maybe we can't go up unless we know the path.
+                // Let's look for any "parent" or "up" in the whole project.
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
@@ -323,3 +344,13 @@
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


## R33 — rechazada (1462 s, 35 herramientas)

**Causa (orientativa):** NO COMPILA con las pruebas reservadas: type annotations needed for `Vec<_>`; type annotations needed for `std::vec::Vec<_>`

**Ediciones:** {'sin efecto': 11, 'aplicada': 7, 'rechazo llaves': 2, 'bloqueada': 1}; pruebas propias escritas: 0


```diff
--- a/crates/soso-audio/src/wav.rs
+++ b/crates/soso-audio/src/wav.rs
@@ -15,21 +15,50 @@
     if data.len() < 44 {
         return Err(WavError::TooShort);
     }
-    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
-        return Err(WavError::BadRiff);
-    }
+    
+    let mut sample_rate = 0u32;
+    let mut bits = 0u16;
+    let mut channels = 0u16;
+    let mut pcm = Vec::new();
+    let mut data_chunks = Vec::new();
+
     let mut pos = 12usize;
     let mut sample_rate = 0u32;
     let mut bits = 0u16;
     let mut channels = 0u16;
     let mut pcm = Vec::new();
+    let mut data_chunks = Vec::new();
+
     while pos + 8 <= data.len() {
         let id = &data[pos..pos + 4];
         let sz = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
         pos += 8;
-        if pos + sz > data.len() {
+
+        if id == b"fmt " && sz >= 16 {
+            let audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
+            channels = u16::from_le_bytes(data[pos + 2..pos + 4].try_into().unwrap());
+            sample_rate = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap());
+            bits = u16::from_le_bytes(data[pos + 14..pos + 16].try_into().unwrap());
+            if audio_format != 1 {
+                return Err(WavError::Unsupported);
+            }
+        } else if id == b"data" {
+            let end = (pos + sz).min(data.len());
+            data_chunks.push(&data[pos..end]);
+            pos = end;
+        } else {
+            pos += sz;
+        }
+
+        if pos + 8 > data.len() {
             break;
         }
+    }
+    while pos + 8 <= data.len() {
+        let id = &data[pos..pos + 4];
+        let sz = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
+        pos += 8;
+
         if id == b"fmt " && sz >= 16 {
             let audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
             channels = u16::from_le_bytes(data[pos + 2..pos + 4].try_into().unwrap());
@@ -39,20 +68,47 @@
                 return Err(WavError::Unsupported);
             }
         } else if id == b"data" {
-            if bits != 16 || channels != 1 {
-                return Err(WavError::Unsupported);
+            let end = (pos + sz).min(data.len());
+            data_chunks.push(&data[pos..end]);
+            pos = end;
+        } else {
+            pos += sz;
+        }
+
+        if pos + 8 > data.len() {
+            break;
+        }
+    }
+
+    if sample_rate == 0 {
+        return Err(WavError::BadFmt);
+    }
+
+    for chunk in data_chunks {
+        if bits != 16 {
+            return Err(WavError::Unsupported);
+        }
+        if channels == 1 {
+            pcm.reserve(chunk.len() / 2);
+            for b in chunk.chunks_exact(2) {
+                pcm.push(i16::from_le_bytes([b[0], b[1]]));
             }
-            let bytes = &data[pos..pos + sz];
-            pcm.reserve(bytes.len() / 2);
… (diff recortado)
```


## R34 — rechazada (451 s, 21 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy

**Ediciones:** {'aplicada': 7, 'sin efecto': 1}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/soso-llm-core/tests/aceptacion_R34.rs:60:9`: assertion `left == right` failed
- `crates/soso-llm-core/tests/aceptacion_R34.rs:23:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy


```diff
--- a/crates/soso-llm-core/src/sample.rs
+++ b/crates/soso-llm-core/src/sample.rs
@@ -39,12 +39,27 @@
     }
 
     fn argmax(logits: &[f32]) -> u32 {
-        logits
-            .iter()
-            .enumerate()
-            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal))
-            .map(|(i, _)| i as u32)
-            .unwrap_or(0)
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
 
     pub fn sample(&mut self, logits: &[f32]) -> u32 {
@@ -54,43 +69,80 @@
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
+        // Si todos son NaN o todos son -inf, o hay un +inf, 
+        // el comportamiento debe ser el de greedy.
+        if all_nan || (max_val == f32::NEG_INFINITY && !has_inf) || has_inf {
+            return Self::argmax(logits);
+        }
+
         let inv_t = 1.0 / self.temp;
         let mut sum = 0.0f32;
         for &l in logits {
-            let p = libm::expf((l - max) * inv_t);
+            if l.is_nan() {
+                self.probs.push(0.0);
+                continue;
+            }
+            let p = libm::expf((l - max_val) * inv_t);
             self.probs.push(p);
             sum += p;
         }
+
         // índices ordenados por probabilidad descendente
         self.idx.clear();
-        self.idx.extend(0..n as u32);
-        let probs = &self.probs;
+        for i in 0..n as u32 {
+            self.idx.push(i);
+        }
+        
         self.idx.sort_unstable_by(|&a, &b| {
-            probs[b as usize]
… (diff recortado)
```


## R35 — rechazada (1315 s, 36 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan tests::convierte_deepseek2_mla_sintetico, tests::convierte_gguf_qwen35_gdn_y_gated, tests::convierte_gguf_sintetico

**Ediciones:** {'aplicada': 4, 'rechazo sintaxis': 1, 'sin efecto': 5}; pruebas propias escritas: 5

**Panics de las pruebas reservadas:**
- `crates/gguf2som/src/lib.rs:1438:71`: called `Result::unwrap()` on an `Err` value: "eof inesperado"
- `crates/gguf2som/src/lib.rs:1862:73`: called `Result::unwrap()` on an `Err` value: "eof inesperado"
- `crates/gguf2som/src/lib.rs:1632:9`: assertion failed: logits.iter().all(|x| x.is_finite())

**Pruebas reservadas que fallan:** tests::convierte_deepseek2_mla_sintetico, tests::convierte_gguf_qwen35_gdn_y_gated, tests::convierte_gguf_sintetico


```diff
--- a/crates/gguf2som/src/lib.rs
+++ b/crates/gguf2som/src/lib.rs
@@ -1030,6 +1030,9 @@
     for _ in 0..tensor_count {
         let name = read_string(file)?;
         let n_dims = read_u32(file)? as usize;
+        if n_dims > 8 {
+            return Err(format!("El tensor {} tiene demasiadas dimensiones (máx 8)", name));
+        }
         let mut shape = Vec::with_capacity(n_dims);
         for _ in 0..n_dims {
             shape.push(read_u64(file)?);
@@ -1046,18 +1049,9 @@
         );
     }
 
-    let alignment = match meta.get("general.alignment") {
-        Some(MetaValue::Int(v)) if *v > 0 && v.is_power_of_two() => *v,
-        _ => 32,
-    };
-    let pos = file.stream_position()?;
-    let data_offset = pos.next_multiple_of(alignment);
-
-    Ok(GgufFile {
-        meta,
-        tensors,
-        data_offset,
-    })
+    let data_offset = read_u64(file)?;
+
+    Ok(GgufFile { meta, tensors, data_offset })
 }
 
 /// Lee un tensor y devuelve (payload .som, dtype). F16 se expande a F32;
@@ -1229,6 +1223,9 @@
 
 fn read_string<R: Read + Seek>(file: &mut R) -> Result<String, String> {
     let len = read_u64(file)? as usize;
+    if len > 16 * 1024 * 1024 {
+        return Err("La cadena declarada es demasiado larga (máx 16 MiB).".into());
+    }
     let mut buf = vec![0u8; len];
     file.read_exact(&mut buf)?;
     String::from_utf8(buf).map_err(|e| e.to_string())
```


## R36 — rechazada (1104 s, 34 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan las_versiones_desconocidas_se_rechazan

**Ediciones:** {'aplicada': 13, 'otro error': 2, 'rechazo llaves': 1}; pruebas propias escritas: 4

**Panics de las pruebas reservadas:**
- `crates/sosomodel/tests/aceptacion_R36.rs:62:9`: versión 7

**Pruebas reservadas que fallan:** las_versiones_desconocidas_se_rechazan


```diff
--- a/crates/sosomodel/src/manifest.rs
+++ b/crates/sosomodel/src/manifest.rs
@@ -881,6 +881,12 @@
             }
             specs
         } else {
+            if version == 0 || version > 6 {
+                return Err(());
+            }
+            if version < 4 && num_layers > 1024 {
+                return Err(());
+            }
             Self::synthesize_layers_from_globals(num_layers, num_experts)
         };
         let n_pf = r.u32()? as usize;
@@ -990,6 +996,13 @@
         format!("{prefix}.mlp_fc1.weight.tensor"),
         format!("{prefix}.mlp_fc1.bias.tensor"),
         format!("{prefix}.mlp_fc2.weight.tensor"),
-        format!("{prefix}.mlp_fc2.bias.tensor"),
+         format!("{prefix}.mlp_fc2.bias.tensor"),
     ]
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+
+
+}
```


## R37 — rechazada (355 s, 23 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta, rechaza_esquemas_y_hosts_invalidos

**Ediciones:** {'aplicada': 6, 'sin efecto': 5, 'rechazo llaves': 3}; pruebas propias escritas: 1

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
+    };
+
+    // Determinar la ruta
+    let path = if path_raw.is_empty() {
+        String::from("/")
+    } else if path_raw.starts_with('?') {
+        format!("/{}", path_raw)
+    } else {
+        format!("/{}", path_raw)
     };
-    if let Some((host, port)) = host_port.rsplit_once(':') {
-        let port: u16 = port.parse().map_err(|_| HttpError::Parse)?;
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


## R38 — rechazada (1761 s, 35 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple

**Ediciones:** {'aplicada': 10, 'sin efecto': 4, 'bloqueada': 1}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-web-core/tests/aceptacion_R38.rs:90:56`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:42:51`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:53:51`: called `Result::unwrap()` on an `Err` value: ()

**Pruebas reservadas que fallan:** deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple


```diff
--- a/crates/soso-web-core/src/decode.rs
+++ b/crates/soso-web-core/src/decode.rs
@@ -10,8 +10,60 @@
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
+                // If it's raw deflate, it doesn't have the 0x78 0x9c header.
+                // We try to decompress it by skipping the first 2 bytes.
+                if body.len() >= 2 {
+                    // Check if it's NOT a zlib header (0x78 0x9c)
+                    if body[0] != 0x78 || body[1] != 0x9c {
+                        match decompress_to_vec_zlib(&body[2..]) {
+                            Ok(res) => Ok(res),
+                            Err(_) => Err(()),
+                        }
+                    } else {
+                        Err(())
+                    }
+                } else {
+                    Err(())
+                }
+            }
+        }
     } else {
         Ok(body.to_vec())
     }
@@ -91,3 +143,56 @@
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
… (diff recortado)
```
