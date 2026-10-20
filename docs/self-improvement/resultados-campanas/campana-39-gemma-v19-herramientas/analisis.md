# Análisis de ejecucion-39-gemma-v19-herramientas

1 aceptadas de 10. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R31 | rechazada | CASO LÍMITE NO CUBIERTO: fallan corrupcion_aleatoria_nunca_entra_en_panico, cualquier_truncado_es_error_y_no_p |
| R32 | rechazada | CASO LÍMITE NO CUBIERTO: fallan la_raiz_es_su_propio_padre, punto_punto_sube_al_padre |
| R33 | rechazada | CASO LÍMITE NO CUBIERTO: fallan data_antes_de_fmt, sin_fmt_o_sin_muestras_es_badfmt, varios_data_se_concatenan |
| R34 | rechazada | CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy |
| R35 | rechazada | CASO LÍMITE NO CUBIERTO: fallan tests::convierte_deepseek2_mla_sintetico |
| R36 | rechazada | CASO LÍMITE NO CUBIERTO: fallan en_v1_a_v3_el_numero_de_capas_tiene_techo |
| R37 | rechazada | CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta, rechaza_esquemas_y_hosts_invalidos |
| R38 | rechazada | NO COMPILA con las pruebas reservadas: cannot find function `decompress_gzip` in this scope; cannot find funct |
| R39 | rechazada | CASO LÍMITE NO CUBIERTO: fallan indices_enormes_no_desbordan |


## R31 — rechazada (107 s, 11 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan corrupcion_aleatoria_nunca_entra_en_panico, cualquier_truncado_es_error_y_no_panico

**Ediciones:** {'aplicada': 3}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/sosomfs/src/catalog.rs:179:49`: range end index 119 out of range for slice of length 116
- `crates/sosomfs/src/catalog.rs:185:47`: range end index 211 out of range for slice of length 209

**Pruebas reservadas que fallan:** corrupcion_aleatoria_nunca_entra_en_panico, cualquier_truncado_es_error_y_no_panico


```diff
--- a/crates/sosomfs/src/catalog.rs
+++ b/crates/sosomfs/src/catalog.rs
@@ -45,12 +45,20 @@
         out
     }
 
-    pub fn parse(data: &[u8]) -> Result<Self, ()> {
+    pub fn parse(data:&[u8]) -> Result<Self, ()> {
         if data.len() < 4 {
             return Err(());
         }
         let nmodels = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
         let mut off = 4usize;
+        
+        // Check if nmodels is reasonable given the data size
+        // Each model is at least NAME_MAX + 1 + 8 + 8 + 8 + 8 + 4 = 133 bytes
+        // But we should check if nmodels * 133 fits in data.len()
+        if nmodels > usize::MAX / 133 || nmodels * 133 > data.len() {
+            return Err(());
+        }
+
         let mut models = Vec::with_capacity(nmodels);
         for _ in 0..nmodels {
             let (m, used) = read_model(&data[off..])?;
@@ -98,7 +106,7 @@
     out.extend_from_slice(&e.stripe_index.to_le_bytes());
 }
 
-fn read_model(data: &[u8]) -> Result<(ModelEntry, usize), ()> {
+fn read_model(data:&[u8]) -> Result<(ModelEntry, usize), ()> {
     if data.len() < NAME_MAX + 32 {
         return Err(());
     }
@@ -115,6 +123,14 @@
     off += 8;
     let nshards = u32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
     off += 4;
+
+    // Check if nshards is reasonable given the data size
+    // Each shard is at least PATH_MAX + 1 + 8 + 2 + 28 + 8 + 4 + 1 + 1 + 2 + 4 = 159 bytes
+    // But we should check if nshards * 159 fits in data.len()
+    if nshards > usize::MAX / 159 || nshards * 159 > data.len() {
+        return Err(());
+    }
+
     let mut shards = Vec::with_capacity(nshards);
     for _ in 0..nshards {
         let (s, used) = read_shard(&data[off..])?;
@@ -123,7 +139,7 @@
     }
     Ok((
         ModelEntry {
-            name: String::from(name),
+            name:String::from(name),
             manifest_lba,
             manifest_blocks,
             index_lba,
@@ -134,7 +150,7 @@
     ))
 }
 
-fn read_shard(data: &[u8]) -> Result<(ShardEntry, usize), ()> {
+fn read_shard(data:&[u8]) -> Result<(ShardEntry, usize), ()> {
     if data.len() < PATH_MAX + 20 {
         return Err(());
     }
@@ -145,6 +161,13 @@
     off += 8;
     let nextents = u16::from_le_bytes(data[off..off + 2].try_into().unwrap()) as usize;
     off += 2;
+
+    // Check if nextents is reasonable given the data size
+    // Each extent is 28 bytes
+    if nextents > usize::MAX / 28 || nextents * 28 > data.len() {
+        return Err(());
+    }
+
     let mut extents = Vec::with_capacity(nextents);
     for _ in 0..nextents {
         let e = read_extent(&data[off..])?;
@@ -161,9 +184,16 @@
     off += 4;
     let shard_crc32c = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
     off += 4;
+
+    // Check if we have enough bytes left for the fixed part (prefetch, policy, flags, crc)
+    // This part is 8 + 4 + 1 + 1 + 2 + 4 = 20 bytes
+    if data.len() < off + 20 {
+        return Err(());
+    }
… (diff recortado)
```


## R32 — rechazada (74 s, 20 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan la_raiz_es_su_propio_padre, punto_punto_sube_al_padre

**Ediciones:** {'aplicada': 1}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/sosofs/tests/aceptacion_R32.rs:40:5`: assertion `left == right` failed
- `crates/sosofs/tests/aceptacion_R32.rs:55:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** la_raiz_es_su_propio_padre, punto_punto_sube_al_padre


```diff
--- a/crates/sosofs/src/fs.rs
+++ b/crates/sosofs/src/fs.rs
@@ -210,14 +210,39 @@
     }
 
     /// Resuelve una ruta absoluta (o relativa a la raíz) a un inode.
-    pub fn resolve(&mut self, path: &str) -> Result<u64, FsError> {
+    pub fn resolve(&mut self, path:&str) -> Result<u64, FsError> {
         let mut ino = ROOT_INODE;
         for comp in path.split('/').filter(|c| !c.is_empty() && *c != ".") {
-            let st = self.stat_inode(ino)?;
-            if st.file_type != FT_DIR {
-                return Err(FsError::NotADir);
+            if comp == ".." {
+                let st = self.stat_inode(ino)?;
+                if st.file_type != FT_DIR {
+                    return Err(FsError::NotADir);
+                }
+                // In a real filesystem, we'd need to know the parent.
+                // But the task says:"la raíz es su propio padre:/.., /../.. y /a/../.. resuelven a ROOT_INODE"
+                // And "un componente .. no tiene tratamiento:se busca como si fuera el nombre de un fichero llamado ..".
+                // Wait, the current behavior is:"se busca como si fuera el nombre de un fichero llamado ..".
+                // The new behavior is:"interprete .. como «subir al directorio padre»".
+                // Since we don't have parent pointers in the inode, how do we know the parent?
+                // The task says:"la raíz es su propio padre".
+                // If we are at /a/b, and we see .., we need to know that the parent of b is a.
+                // But the current `lookup` only takes `dir` and `name`.
+                // If we don't have parent pointers, we can't "go up" unless we know the path.
+                // Let's re-read:"la raíz es su propio padre:/.., /../.. y /a/../.. resuelven a ROOT_INODE".
+                // This implies that if we are at ROOT_INODE, .. stays at ROOT_INODE.
+                // If we are at /a, .. goes to /.
+                // But how do we know /a's parent is /?
+                // Maybe we can't "go up" unless we have the path? No, `resolve` is supposed to work.
+                // Wait, if the filesystem is a tree, and we only have `lookup(dir, name)`,
+                // the only way to know the parent of `dir` is to have it in the inode.
+                // Let's check `InodeItem` and `stat_inode`.
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
```


## R33 — rechazada (76 s, 6 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan data_antes_de_fmt, sin_fmt_o_sin_muestras_es_badfmt, varios_data_se_concatenan_y_los_ajenos_se_saltan

**Ediciones:** {'aplicada': 1}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-audio/tests/aceptacion_R33.rs:104:5`: assertion `left == right` failed
- `crates/soso-audio/tests/aceptacion_R33.rs:122:5`: assertion `left == right` failed
- `crates/soso-audio/tests/aceptacion_R33.rs:117:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** data_antes_de_fmt, sin_fmt_o_sin_muestras_es_badfmt, varios_data_se_concatenan_y_los_ajenos_se_saltan


```diff
--- a/crates/soso-audio/src/wav.rs
+++ b/crates/soso-audio/src/wav.rs
@@ -11,7 +11,7 @@
 }
 
 /// Lee PCM16 mono. Acepta cualquier sample rate (el caller puede remuestrear).
-pub fn parse_pcm16_mono(data: &[u8]) -> Result<(u32, Vec<i16>), WavError> {
+pub fn parse_pcm16_mono(data:&[u8]) -> Result<(u32, Vec<i16>), WavError> {
     if data.len() < 44 {
         return Err(WavError::TooShort);
     }
@@ -23,13 +23,16 @@
     let mut bits = 0u16;
     let mut channels = 0u16;
     let mut pcm = Vec::new();
+    let mut fmt_found = false;
+
     while pos + 8 <= data.len() {
         let id = &data[pos..pos + 4];
         let sz = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
         pos += 8;
-        if pos + sz > data.len() {
-            break;
-        }
+
+        let available = data.len().saturating_sub(pos);
+        let actual_sz = std::cmp::min(sz, available);
+
         if id == b"fmt " && sz >= 16 {
             let audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
             channels = u16::from_le_bytes(data[pos + 2..pos + 4].try_into().unwrap());
@@ -38,21 +41,38 @@
             if audio_format != 1 {
                 return Err(WavError::Unsupported);
             }
+            fmt_found = true;
         } else if id == b"data" {
-            if bits != 16 || channels != 1 {
+            if bits != 16 || channels != 1 && channels != 2 {
                 return Err(WavError::Unsupported);
             }
-            let bytes = &data[pos..pos + sz];
+            let bytes = &data[pos..pos + actual_sz];
             pcm.reserve(bytes.len() / 2);
             for chunk in bytes.chunks_exact(2) {
                 pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
             }
         }
-        pos += sz + (sz & 1);
+        pos += actual_sz;
+    }
+
+    if !fmt_found {
+        return Err(WavError::BadFmt);
     }
     if pcm.is_empty() || sample_rate == 0 {
         return Err(WavError::BadFmt);
     }
+
+    if channels == 2 {
+        let mut mono_pcm = Vec::with_capacity(pcm.len());
+        for i in (0..pcm.len()).step_by(2) {
+            if i + 1 < pcm.len() {
+                let avg = (pcm[i] as i32 + pcm[i + 1] as i32) / 2;
+                mono_pcm.push(avg as i16);
+            }
+        }
+        pcm = mono_pcm;
+    }
+
     Ok((sample_rate, pcm))
 }
 
```


## R34 — rechazada (81 s, 10 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy

**Ediciones:** {'aplicada': 3}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-llm-core/tests/aceptacion_R34.rs:23:5`: assertion `left == right` failed
- `crates/soso-llm-core/tests/aceptacion_R34.rs:60:9`: assertion `left == right` failed

**Pruebas reservadas que fallan:** greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy


```diff
--- a/crates/soso-llm-core/src/sample.rs
+++ b/crates/soso-llm-core/src/sample.rs
@@ -38,16 +38,35 @@
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
+        let mut max_val = f32::NEG_INFINITY;
+        let mut max_idx = 0;
+
+        for (i, &val) in logits.iter().enumerate() {
+            if val.is_nan() {
+                continue;
+            }
+            if val > max_val {
+                max_val = val;
+                max_idx = i as u32;
+            } else if (val - max_val).abs() < f32::EPSILON {
+                if (i as u32) < max_idx {
+                    max_idx = i as u32;
+                }
+            }
+        }
+
+        if max_val == f32::NEG_INFINITY {
+            // Todos son NaN o -inf. Si hay NaN, el resultado es 0.
+            // Si todos son -inf, el resultado es el menor índice (0).
+            // En ambos casos, 0 es el menor índice.
+            0
+        } else {
+            max_idx
+        }
     }
 
-    pub fn sample(&mut self, logits: &[f32]) -> u32 {
+    pub fn sample(&mut self, logits:&[f32]) -> u32 {
         if self.temp <= 0.0 || logits.len() < 2 {
             return Self::argmax(logits);
         }
@@ -55,13 +74,23 @@
         self.probs.clear();
         self.probs.reserve(n);
         let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
+
+        if !max.is_finite() {
+            return Self::argmax(logits);
+        }
+
         let inv_t = 1.0 / self.temp;
         let mut sum = 0.0f32;
         for &l in logits {
-            let p = libm::expf((l - max) * inv_t);
-            self.probs.push(p);
-            sum += p;
+            if l.is_nan() {
+                self.probs.push(0.0);
+            } else {
+                let p = libm::expf((l - max) * inv_t);
+                self.probs.push(p);
+            }
+            sum += self.probs[self.probs.len() - 1];
         }
+
         // índices ordenados por probabilidad descendente
         self.idx.clear();
         self.idx.extend(0..n as u32);
@@ -71,7 +100,8 @@
                 .partial_cmp(&probs[a as usize])
                 .unwrap_or(core::cmp::Ordering::Equal)
         });
-        // núcleo: el prefijo más corto que acumula top_p de la masa
+
+        // núcleo:el prefijo más corto que acumula top_p de la masa
         let cutoff = self.top_p * sum;
         let mut mass = 0.0f32;
         let mut k = 0;
@@ -82,6 +112,7 @@
                 break;
             }
         }
+
         // muestrear dentro del núcleo
         let mut r = self.next_f32() * mass;
… (diff recortado)
```


## R35 — rechazada (262 s, 23 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan tests::convierte_deepseek2_mla_sintetico

**Ediciones:** {'aplicada': 3}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/gguf2som/src/lib.rs:1634:9`: assertion failed: logits.iter().all(|x| x.is_finite())

**Pruebas reservadas que fallan:** tests::convierte_deepseek2_mla_sintetico


```diff
--- a/crates/gguf2som/src/lib.rs
+++ b/crates/gguf2som/src/lib.rs
@@ -1005,7 +1005,7 @@
     }
 }
 
-fn parse_gguf<R: Read + Seek>(file: &mut R) -> Result<GgufFile, String> {
+fn parse_gguf<R:Read + Seek>(file:&mut R) -> Result<GgufFile, String> {
     let mut magic = [0u8; 4];
     file.read_exact(&mut magic)?;
     if &magic != b"GGUF" {
@@ -1030,6 +1030,9 @@
     for _ in 0..tensor_count {
         let name = read_string(file)?;
         let n_dims = read_u32(file)? as usize;
+        if n_dims > 8 {
+            return Err(format!("El tensor {} tiene demasiadas dimensiones (dimensiones)", name));
+        }
         let mut shape = Vec::with_capacity(n_dims);
         for _ in 0..n_dims {
             shape.push(read_u64(file)?);
@@ -1046,17 +1049,10 @@
         );
     }
 
-    let alignment = match meta.get("general.alignment") {
-        Some(MetaValue::Int(v)) if *v > 0 && v.is_power_of_two() => *v,
-        _ => 32,
-    };
-    let pos = file.stream_position()?;
-    let data_offset = pos.next_multiple_of(alignment);
-
     Ok(GgufFile {
         meta,
         tensors,
-        data_offset,
+        data_offset:0,
     })
 }
 
@@ -1227,8 +1223,11 @@
     read_bytes_le(file, 8)
 }
 
-fn read_string<R: Read + Seek>(file: &mut R) -> Result<String, String> {
+fn read_string<R:Read + Seek>(file:&mut R) -> Result<String, String> {
     let len = read_u64(file)? as usize;
+    if len > 16 * 1024 * 1024 {
+        return Err("La longitud de la cadena es demasiado grande (cadena)".into());
+    }
     let mut buf = vec![0u8; len];
     file.read_exact(&mut buf)?;
     String::from_utf8(buf).map_err(|e| e.to_string())
```


## R36 — rechazada (68 s, 9 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan en_v1_a_v3_el_numero_de_capas_tiene_techo

**Ediciones:** {'aplicada': 2}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/sosomodel/tests/aceptacion_R36.rs:52:9`: v1 con 1025 capas

**Pruebas reservadas que fallan:** en_v1_a_v3_el_numero_de_capas_tiene_techo


```diff
--- a/crates/sosomodel/src/manifest.rs
+++ b/crates/sosomodel/src/manifest.rs
@@ -842,6 +842,9 @@
 
     pub fn parse(data: &[u8]) -> Result<Self, ()> {
         let (version, body) = parse_som(data)?;
+if version == 0 || version > 6 {
+    return Err(());
+}
         let mut r = Reader::new(body);
         let name = String::from(r.cstr()?);
         let vocab_size = r.u32()?;
@@ -856,6 +859,10 @@
             (num_heads, 10000.0, 1e-5)
         };
         let (num_experts, num_experts_per_tok, moe_ffn_dim) = if version >= 3 {
+            if version < 4 && num_layers > 1024 {
+                return Err(());
+            }
+
             (r.u32()?, r.u32()?, r.u32()?)
         } else {
             (0, 0, 0)
```


## R37 — rechazada (63 s, 6 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta, rechaza_esquemas_y_hosts_invalidos

**Ediciones:** {'aplicada': 1}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-http/tests/aceptacion_R37.rs:19:5`: assertion `left == right` failed
- `crates/soso-http/tests/aceptacion_R37.rs:38:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** la_query_se_conserva_aunque_no_haya_ruta, rechaza_esquemas_y_hosts_invalidos


```diff
--- a/crates/soso-http/src/lib.rs
+++ b/crates/soso-http/src/lib.rs
@@ -598,18 +598,54 @@
     Ok((status, body))
 }
 
-fn parse_url(url: &str) -> Result<(&str, &str, u16, String), HttpError> {
+pub fn parse_url(url:&str) -> Result<(&str, &str, u16, String), HttpError> {
     let rest = url.strip_prefix("https://").ok_or(HttpError::Parse)?;
-    let (host_port, path) = match rest.split_once('/') {
-        Some((h, p)) => (h, format!("/{p}")),
-        None => (rest, String::from("/")),
+    
+    // Remove fragment
+    let rest = if let Some(idx) = rest.find('#') {
+        &rest[..idx]
+    } else {
+        rest
+    };
+
+    // Split host_port and path
+    let (host_port, path_part) = match rest.find('/') {
+        Some(idx) => (&rest[..idx], &rest[idx..]),
+        None => (rest, "/"),
     };
-    if let Some((host, port)) = host_port.rsplit_once(':') {
-        let port: u16 = port.parse().map_err(|_| HttpError::Parse)?;
-        Ok(("https", host, port, path))
+
+    // Handle path if it starts with '?'
+    let path = if path_part.starts_with('?') {
+        format!("/{}", path_part)
     } else {
-        Ok(("https", host_port, 443, path))
+        path_part.to_string()
+    };
+
+    // Check for '@' in host_port (credentials)
+    if host_port.contains('@') {
+        return Err(HttpError::Parse);
     }
+
+    // Split host and port
+    let (host, port) = if let Some(idx) = host_port.rfind(':') {
+        let h = &host_port[..idx];
+        let p_str = &host_port[idx + 1..];
+        if h.is_empty() || p_str.is_empty() {
+            return Err(HttpError::Parse);
+        }
+        let p = p_str.parse::<u16>().map_err(|_| HttpError::Parse)?;
+        if p == 0 {
+            return Err(HttpError::Parse);
+        }
+        (h, p)
+    } else {
+        if host_port.is_empty() {
+            return Err(HttpError::Parse);
+        }
+        (host_port, 443)
+    };
+
+    Ok(("https", host, port, path) )
 }
 
 fn parse_response_head(head: &[u8]) -> Result<(u16, Vec<(String, String)>), HttpError> {
```


## R38 — rechazada (77 s, None herramientas)

**Causa (orientativa):** NO COMPILA con las pruebas reservadas: cannot find function `decompress_gzip` in this scope; cannot find function `decompress_deflate` in this scope

**Ediciones:** {}; pruebas propias escritas: 0


```diff
--- a/crates/soso-web-core/src/decode.rs
+++ b/crates/soso-web-core/src/decode.rs
@@ -5,13 +5,15 @@
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
+        decompress_gzip(body).map_err(|_| ())
+    } else if enc.contains("deflate") {
+        decompress_deflate(body).map_err(|_| ())
     } else {
         Ok(body.to_vec())
     }
```


## R39 — rechazada (114 s, 12 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan indices_enormes_no_desbordan

**Ediciones:** {'otro error': 3, 'aplicada': 3}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/block-dev/src/lib.rs:102:38`: attempt to add with overflow

**Pruebas reservadas que fallan:** indices_enormes_no_desbordan


```diff
--- a/crates/block-dev/src/lib.rs
+++ b/crates/block-dev/src/lib.rs
@@ -91,17 +91,24 @@
         (self.data.len() / BLOCK_SIZE) as u64
     }
 
-    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
+    fn read_block(&mut self, block:u64, buf:&mut Block) -> Result<(), BlockError> {
         #[cfg(feature = "std")]
         self.reads.set(self.reads.get() + 1);
-        let off = block as usize * BLOCK_SIZE;
+        
+        let off = block.checked_mul(BLOCK_SIZE as u64)
+            .ok_or(BlockError::OutOfRange)?;
+        
+        let off = off as usize;
         let src = self.data.get(off..off + BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
         buf.copy_from_slice(src);
         Ok(())
     }
 
-    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
-        let off = block as usize * BLOCK_SIZE;
+    fn write_block(&mut self, block:u64, buf:&Block) -> Result<(), BlockError> {
+        let off = block.checked_mul(BLOCK_SIZE as u64)
+            .ok_or(BlockError::OutOfRange)?;
+        
+        let off = off as usize;
         let dst = self.data.get_mut(off..off + BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
         dst.copy_from_slice(buf);
         Ok(())
@@ -118,13 +125,17 @@
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
+        
+        let off = start.checked_mul(BLOCK_SIZE as u64)
+            .ok_or(BlockError::OutOfRange)?;
+        
+        let off = off as usize;
         let src = self
             .data
             .get(off..off + buf.len())
```
