# Análisis de ejecucion-54-qwen36-iq3-v39

6 aceptadas de 10. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R33 | rechazada | CASO LÍMITE NO CUBIERTO: fallan estereo_se_promedia |
| R34 | rechazada | CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy |
| R35 | rechazada | CASO LÍMITE NO CUBIERTO: fallan tests::cadena_limite_acepta, tests::tensor_ocho_dimensiones_acepta |
| R38 | rechazada | CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple |


## R33 — rechazada (536 s, 39 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan estereo_se_promedia

**Ediciones:** {'aplicada': 12, 'otro error': 2}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-audio/src/wav.rs:77:26`: attempt to add with overflow

**Pruebas reservadas que fallan:** estereo_se_promedia


```diff
--- a/crates/soso-audio/src/wav.rs
+++ b/crates/soso-audio/src/wav.rs
@@ -11,8 +11,14 @@
 }
 
 /// Lee PCM16 mono. Acepta cualquier sample rate (el caller puede remuestrear).
+///
+/// Soporta:
+/// - WAV mono o estéreo (2 canales): para estéreo devuelve la media de los dos canales.
+/// - Múltiples chunks `data`: los concatena en orden de aparición.
+/// - Chunks en cualquier orden (data puede aparecer antes de fmt).
+/// - `data` más corto que su tamaño declarado: lee hasta el final.
 pub fn parse_pcm16_mono(data: &[u8]) -> Result<(u32, Vec<i16>), WavError> {
-    if data.len() < 44 {
+    if data.len() < 12 {
         return Err(WavError::TooShort);
     }
     if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
@@ -22,35 +28,59 @@
     let mut sample_rate = 0u32;
     let mut bits = 0u16;
     let mut channels = 0u16;
-    let mut pcm = Vec::new();
+    let mut audio_format = 0u16;
+    let mut has_fmt = false;
+    let mut raw_data_chunks: Vec<&[u8]> = Vec::new();
     while pos + 8 <= data.len() {
         let id = &data[pos..pos + 4];
         let sz = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
         pos += 8;
-        if pos + sz > data.len() {
-            break;
-        }
-        if id == b"fmt " && sz >= 16 {
-            let audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
+        if id == b"fmt " {
+            if sz < 16 || pos + 16 > data.len() {
+                return Err(WavError::BadFmt);
+            }
+            audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
             channels = u16::from_le_bytes(data[pos + 2..pos + 4].try_into().unwrap());
             sample_rate = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap());
             bits = u16::from_le_bytes(data[pos + 14..pos + 16].try_into().unwrap());
-            if audio_format != 1 {
-                return Err(WavError::Unsupported);
-            }
-        } else if id == b"data" {
-            if bits != 16 || channels != 1 {
-                return Err(WavError::Unsupported);
-            }
-            let bytes = &data[pos..pos + sz];
-            pcm.reserve(bytes.len() / 2);
-            for chunk in bytes.chunks_exact(2) {
+            has_fmt = true;
+            pos += sz;
+            pos += sz & 1; // padding
+            continue;
+        }
+        if id == b"data" {
+            let available = data.len().saturating_sub(pos);
+            let to_read = sz.min(available);
+            raw_data_chunks.push(&data[pos..pos + to_read]);
+        }
+        pos += sz + (sz & 1);
+    }
+    if !has_fmt || sample_rate == 0 {
+        return Err(WavError::BadFmt);
+    }
+    if audio_format != 1 {
+        return Err(WavError::Unsupported);
+    }
+    if bits != 16 {
+        return Err(WavError::Unsupported);
+    }
+    if channels != 1 && channels != 2 {
+        return Err(WavError::Unsupported);
+    }
+    let sample_bytes = if channels == 2 { 4 } else { 2 };
+    let mut pcm = Vec::new();
+    for raw in &raw_data_chunks {
+        for chunk in raw.chunks_exact(sample_bytes) {
+            if channels == 2 {
+                let left = i16::from_le_bytes([chunk[0], chunk[1]]);
+                let right = i16::from_le_bytes([chunk[2], chunk[3]]);
+                pcm.push((left + right) / 2);
+            } else {
                 pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
             }
         }
-        pos += sz + (sz & 1);
… (diff recortado)
```


## R34 — rechazada (409 s, 36 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy

**Ediciones:** {'aplicada': 5}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-llm-core/tests/aceptacion_R34.rs:23:5`: assertion `left == right` failed
- `crates/soso-llm-core/tests/aceptacion_R34.rs:60:9`: assertion `left == right` failed

**Pruebas reservadas que fallan:** greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy


```diff
--- a/crates/soso-llm-core/src/sample.rs
+++ b/crates/soso-llm-core/src/sample.rs
@@ -38,27 +38,49 @@
         r as f32 / (1u64 << 24) as f32
     }
 
+    /// Devuelve el índice del mayor logit.
+    /// - En empate, el **menor** índice.
+    /// - NaN nunca se elige mientras haya algún logit finito.
+    /// - Si todos son NaN, devuelve 0.
     fn argmax(logits: &[f32]) -> u32 {
-        logits
-            .iter()
-            .enumerate()
-            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal))
-            .map(|(i, _)| i as u32)
-            .unwrap_or(0)
+        let mut best_idx: Option<u32> = None;
+        let mut best_val: f32 = f32::NEG_INFINITY;
+        for (i, &l) in logits.iter().enumerate() {
+            if l.is_nan() {
+                continue;
+            }
+            if l > best_val || (l == best_val && best_idx.map(|bi| (i as u32) < bi) == Some(true)) {
+                best_val = l;
+                best_idx = Some(i as u32);
+            }
+        }
+        if let Some(idx) = best_idx {
+            return idx;
+        }
+        0
     }
 
+    #[allow(clippy::arithmetic_side_effects)]
     pub fn sample(&mut self, logits: &[f32]) -> u32 {
         if self.temp <= 0.0 || logits.len() < 2 {
             return Self::argmax(logits);
         }
+        // Verificar si el máximo no es finito: todos NaN, todos -inf, o hay +inf
+        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
+        if !max.is_finite() {
+            return Self::argmax(logits);
+        }
         let n = logits.len();
         self.probs.clear();
         self.probs.reserve(n);
-        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
         let inv_t = 1.0 / self.temp;
         let mut sum = 0.0f32;
         for &l in logits {
-            let p = libm::expf((l - max) * inv_t);
+            let p = if l.is_nan() {
+                0.0
+            } else {
+                libm::expf((l - max) * inv_t)
+            };
             self.probs.push(p);
             sum += p;
         }
@@ -67,30 +89,38 @@
         self.idx.extend(0..n as u32);
         let probs = &self.probs;
         self.idx.sort_unstable_by(|&a, &b| {
-            probs[b as usize]
-                .partial_cmp(&probs[a as usize])
+            probs.get(b as usize)
+                .and_then(|pa| probs.get(a as usize).map(|pb| pb.partial_cmp(pa).unwrap_or(core::cmp::Ordering::Equal)))
                 .unwrap_or(core::cmp::Ordering::Equal)
         });
         // núcleo: el prefijo más corto que acumula top_p de la masa
         let cutoff = self.top_p * sum;
         let mut mass = 0.0f32;
-        let mut k = 0;
+        let mut k: u32 = 0;
         for &i in &self.idx {
-            mass += self.probs[i as usize];
+            mass += self.probs.get(i as usize).copied().unwrap_or(0.0);
             k += 1;
             if mass >= cutoff {
                 break;
             }
         }
+        // k >= 1 porque n >= 2 y mass > 0 (al menos un logit finito dio prob > 0)
         // muestrear dentro del núcleo
         let mut r = self.next_f32() * mass;
-        for &i in &self.idx[..k] {
-            r -= self.probs[i as usize];
-            if r <= 0.0 {
-                return i;
… (diff recortado)
```


## R35 — rechazada (781 s, 44 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan tests::cadena_limite_acepta, tests::tensor_ocho_dimensiones_acepta

**Ediciones:** {'rechazo llaves': 5, 'otro error': 1, 'aplicada': 6}; pruebas propias escritas: 7

**Panics de las pruebas reservadas:**
- `crates/gguf2som/src/lib.rs:2106:63`: called `Result::unwrap()` on an `Err` value: "abrir /tmp/convert-gguf-8dims/eight.gguf: No such file or directory (os error 2)"
- `crates/gguf2som/src/lib.rs:2075:63`: called `Result::unwrap()` on an `Err` value: "el GGUF no contiene token_embd.weight"

**Pruebas reservadas que fallan:** tests::cadena_limite_acepta, tests::tensor_ocho_dimensiones_acepta


```diff
--- a/crates/gguf2som/src/lib.rs
+++ b/crates/gguf2som/src/lib.rs
@@ -1030,6 +1030,9 @@
     for _ in 0..tensor_count {
         let name = read_string(file)?;
         let n_dims = read_u32(file)? as usize;
+        if n_dims > 8 {
+            return Err(format!("tensor '{name}' con {n_dims} dimensiones excede el máximo de 8"));
+        }
         let mut shape = Vec::with_capacity(n_dims);
         for _ in 0..n_dims {
             shape.push(read_u64(file)?);
@@ -1229,6 +1232,10 @@
 
 fn read_string<R: Read + Seek>(file: &mut R) -> Result<String, String> {
     let len = read_u64(file)? as usize;
+    const MAX_STR: usize = 16 * 1024 * 1024; // 16 MiB
+    if len > MAX_STR {
+        return Err(format!("cadena de {len} bytes excede el máximo de {MAX_STR}"));
+    }
     let mut buf = vec![0u8; len];
     file.read_exact(&mut buf)?;
     String::from_utf8(buf).map_err(|e| e.to_string())
@@ -1989,4 +1996,113 @@
         assert!(tok.bos().is_none());
         assert_eq!(tok.encode_trozo("hola mundo", false, true), vec![2, 3]);
     }
+
+    // ── Pruebas R35 ──────────────────────────────────────────────────────
+
+    /// Una cadena con longitud > 16 MiB debe devolver Err con "cadena".
+    #[test]
+    fn cadena_demasiado_larga_rechaza() {
+        let dir = std::env::temp_dir().join("convert-gguf-longstr");
+        let _ = fs::remove_dir_all(&dir);
+        fs::create_dir_all(&dir).unwrap();
+        let gguf_path = dir.join("big.gguf");
+        let mut g: Vec<u8> = Vec::new();
+        g.extend_from_slice(b"GGUF");
+        g.extend_from_slice(&3u32.to_le_bytes());
+        g.extend_from_slice(&0u64.to_le_bytes());
+        g.extend_from_slice(&1u64.to_le_bytes());
+        let big_len: u64 = 17 * 1024 * 1024;
+        g.extend_from_slice(&big_len.to_le_bytes());
+        g.extend_from_slice(&[0u8; 256]);
+        fs::File::create(&gguf_path).unwrap().write_all(&g).unwrap();
+        let out = dir.join("out");
+        let err = convert_path(gguf_path.to_str().unwrap(), &out, None).unwrap_err();
+        assert!(err.contains("cadena"), "error sin 'cadena': {err}");
+    }
+
+    /// Un tensor con n_dims > 8 debe devolver Err con "dimensiones" y el nombre.
+    #[test]
+    fn tensor_demasiadas_dimensiones_rechaza() {
+        let dir = std::env::temp_dir().join("convert-gguf-ndims");
+        let _ = fs::remove_dir_all(&dir);
+        fs::create_dir_all(&dir).unwrap();
+        let gguf_path = dir.join("ndims.gguf");
+        let mut g: Vec<u8> = Vec::new();
+        g.extend_from_slice(b"GGUF");
+        g.extend_from_slice(&3u32.to_le_bytes());
+        g.extend_from_slice(&1u64.to_le_bytes());
+        g.extend_from_slice(&0u64.to_le_bytes());
+        g.extend_from_slice(&(6u64).to_le_bytes());
+        g.extend_from_slice(b"tensor");
+        g.extend_from_slice(&9u32.to_le_bytes());
+        fs::File::create(&gguf_path).unwrap().write_all(&g).unwrap();
+        let out = dir.join("out");
+        let err = convert_path(gguf_path.to_str().unwrap(), &out, None).unwrap_err();
+        assert!(err.contains("dimensiones"), "error sin 'dimensiones': {err}");
+        assert!(err.contains("tensor"), "error sin nombre del tensor: {err}");
+    }
+
+    /// Cadena de exactamente 16 MiB se acepta si los datos están presentes.
+    #[test]
+    fn cadena_limite_acepta() {
+        let dir = std::env::temp_dir().join("convert-gguf-exact16");
+        let _ = fs::remove_dir_all(&dir);
+        fs::create_dir_all(&dir).unwrap();
+        let gguf_path = dir.join("exact16.gguf");
+        let big_val = vec![0u8; 16 * 1024 * 1024];
+        let g = pack_gguf_v3(
+            10,
+            |g| {
+                gguf_kv_str(g, "general.architecture", "llama");
+                gguf_kv_u32(g, "llama.embedding_length", 8);
+                gguf_kv_u32(g, "llama.block_count", 1);
+                gguf_kv_u32(g, "llama.feed_forward_length", 16);
+                gguf_kv_u32(g, "llama.attention.head_count", 2);
+                gguf_kv_u32(g, "llama.attention.head_count_kv", 1);
… (diff recortado)
```


## R38 — rechazada (32 s, None herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple

**Ediciones:** {}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-web-core/tests/aceptacion_R38.rs:53:51`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:42:51`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:90:56`: called `Result::unwrap()` on an `Err` value: ()

**Pruebas reservadas que fallan:** deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple


```diff
--- a/crates/soso-web-core/src/decode.rs
+++ b/crates/soso-web-core/src/decode.rs
@@ -2,7 +2,7 @@
 
 use alloc::string::String;
 use alloc::vec::Vec;
-use miniz_oxide::inflate::decompress_to_vec_zlib;
+use miniz_oxide::inflate::{decompress_to_vec, decompress_to_vec_zlib};
 
 /// Descomprime el cuerpo si `Content-Encoding` indica gzip/deflate.
 pub fn maybe_decompress(body: &[u8], content_encoding: Option<&str>) -> Result<Vec<u8>, ()> {
```
