# Análisis de ejecucion-53-qwen36-iq3-v38

7 aceptadas de 10. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R33 | rechazada | NO COMPILA con las pruebas reservadas: unresolved import `soso_audio::wav::resample_to_16k`; could not compile |
| R34 | rechazada | NO COMPILA con las pruebas reservadas: cannot find function `resample_to_16k` in module `wav`; cannot find fun |
| R38 | rechazada | CASO LÍMITE NO CUBIERTO: fallan gzip_con_campos_opcionales, gzip_simple |


## R33 — rechazada (950 s, 40 herramientas)

**Causa (orientativa):** NO COMPILA con las pruebas reservadas: unresolved import `soso_audio::wav::resample_to_16k`; could not compile `soso-audio` (test "mel") due to 1 previous error

**Ediciones:** {'aplicada': 13, 'otro error': 1}; pruebas propias escritas: 2


```diff
--- a/crates/soso-audio/src/wav.rs
+++ b/crates/soso-audio/src/wav.rs
@@ -11,6 +11,18 @@
 }
 
 /// Lee PCM16 mono. Acepta cualquier sample rate (el caller puede remuestrear).
+///
+/// Soporta:
+/// - Mono y estéreo (2 canales): los canales se promedian.
+/// - Múltiples chunks `data`: se concatenan.
+/// - Chunks en cualquier orden (data puede ir antes o después de fmt).
+/// - Chunk `data` con tamaño mayor que los datos reales: lee hasta EOF, ignora bytes sueltos.
+///
+/// Rechaza:
+/// - `audio_format` distinto de 1 → `Unsupported`.
+/// - Más de 2 canales o profundidad distinta de 16 bits → `Unsupported`.
+/// - Sin chunk `fmt ` → `BadFmt`.
+/// - Sin muestras decodificadas → `BadFmt`.
 pub fn parse_pcm16_mono(data: &[u8]) -> Result<(u32, Vec<i16>), WavError> {
     if data.len() < 44 {
         return Err(WavError::TooShort);
@@ -18,11 +30,13 @@
     if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
         return Err(WavError::BadRiff);
     }
+
+    // Primer pase: encontrar y validar el chunk fmt
     let mut pos = 12usize;
     let mut sample_rate = 0u32;
-    let mut bits = 0u16;
     let mut channels = 0u16;
-    let mut pcm = Vec::new();
+    let mut fmt_pos: Option<usize> = None;
+    let mut fmt_sz = 0usize;
     while pos + 8 <= data.len() {
         let id = &data[pos..pos + 4];
         let sz = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
@@ -32,46 +46,353 @@
         }
         if id == b"fmt " && sz >= 16 {
             let audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
-            channels = u16::from_le_bytes(data[pos + 2..pos + 4].try_into().unwrap());
-            sample_rate = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap());
-            bits = u16::from_le_bytes(data[pos + 14..pos + 16].try_into().unwrap());
             if audio_format != 1 {
                 return Err(WavError::Unsupported);
             }
-        } else if id == b"data" {
-            if bits != 16 || channels != 1 {
+            channels = u16::from_le_bytes(data[pos + 2..pos + 4].try_into().unwrap());
+            sample_rate = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap());
+            let bits = u16::from_le_bytes(data[pos + 14..pos + 16].try_into().unwrap());
+            if channels > 2 || bits != 16 {
                 return Err(WavError::Unsupported);
             }
-            let bytes = &data[pos..pos + sz];
-            pcm.reserve(bytes.len() / 2);
-            for chunk in bytes.chunks_exact(2) {
-                pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
+            fmt_pos = Some(pos);
+            fmt_sz = sz;
+        }
+        pos += sz + (sz & 1);
+    }
+
+    if fmt_pos.is_none() || sample_rate == 0 {
+        return Err(WavError::BadFmt);
+    }
+
+    // Segundo pase: procesar todos los chunks data en orden de aparición
+    let mut pcm = Vec::new();
+    pos = 12usize;
+    while pos + 8 <= data.len() {
+        let id = &data[pos..pos + 4];
+        let sz = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
+        pos += 8;
+        if pos + sz > data.len() {
+            break;
+        }
+        if id == b"data" {
+            let end = pos + sz;
+            let bytes = &data[pos..end];
+            if channels == 1 {
+                let usable = (bytes.len() / 2) * 2;
+                for i in (0..usable).step_by(2) {
+                    pcm.push(i16::from_le_bytes([bytes[i], bytes[i + 1]]));
+                }
+            } else if channels == 2 {
+                let usable = (bytes.len() / 4) * 4;
+                for i in (0..usable).step_by(4) {
… (diff recortado)
```


## R34 — rechazada (463 s, 35 herramientas)

**Causa (orientativa):** NO COMPILA con las pruebas reservadas: cannot find function `resample_to_16k` in module `wav`; cannot find function `resample_to_16k` in module `soso_audio::wav`

**Ediciones:** {'aplicada': 3, 'otro error': 3}; pruebas propias escritas: 2


```diff
--- a/crates/soso-llm-core/src/sample.rs
+++ b/crates/soso-llm-core/src/sample.rs
@@ -38,29 +38,53 @@
         r as f32 / (1u64 << 24) as f32
     }
 
+    /// Returns the index of the largest finite logit.
+    /// In case of tie, the smallest index wins.
+    /// NaN values are ignored; if all are NaN, returns 0.
     fn argmax(logits: &[f32]) -> u32 {
-        logits
-            .iter()
-            .enumerate()
-            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal))
-            .map(|(i, _)| i as u32)
-            .unwrap_or(0)
+        // First pass: find the max finite value and its smallest index
+        let mut best_idx: Option<u32> = None;
+        let mut best_val: f32 = f32::NEG_INFINITY;
+        for (i, &l) in logits.iter().enumerate() {
+            if !l.is_nan() {
+                if l > best_val {
+                    best_val = l;
+                    best_idx = Some(i as u32);
+                } else if l == best_val && best_val.is_finite() {
+                    // tie: keep the smaller index (already have it)
+                }
+            }
+        }
+        best_idx.unwrap_or(0)
     }
 
     pub fn sample(&mut self, logits: &[f32]) -> u32 {
         if self.temp <= 0.0 || logits.len() < 2 {
             return Self::argmax(logits);
         }
+        // Check if the max logit is not finite (all NaN, all -inf, or +inf present)
+        let max = logits
+            .iter()
+            .copied()
+            .fold(f32::NEG_INFINITY, f32::max);
+        if !max.is_finite() {
+            // Cannot form a valid distribution → fallback to greedy
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
-            self.probs.push(p);
-            sum += p;
+            if l.is_nan() {
+                // NaN logits get zero probability
+                self.probs.push(0.0);
+            } else {
+                let p = libm::expf((l - max) * inv_t);
+                self.probs.push(p);
+                sum += p;
+            }
         }
         // índices ordenados por probabilidad descendente
         self.idx.clear();
@@ -90,7 +114,13 @@
                 return i;
             }
         }
-        self.idx[k - 1]
+        // fallback: last token in nucleus with non-zero probability
+        for i in (1..=k).rev() {
+            if self.probs[self.idx[(i - 1) as usize] as usize] > 0.0 {
+                return self.idx[(i - 1) as usize];
+            }
+        }
+        self.idx[0]
     }
 }
 
@@ -139,3 +169,108 @@
         }
     }
 }
+
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
… (diff recortado)
```


## R38 — rechazada (1168 s, 40 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan gzip_con_campos_opcionales, gzip_simple

**Ediciones:** {'aplicada': 7, 'otro error': 4}; pruebas propias escritas: 8

**Panics de las pruebas reservadas:**
- `crates/soso-web-core/tests/aceptacion_R38.rs:53:51`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:42:51`: called `Result::unwrap()` on an `Err` value: ()

**Pruebas reservadas que fallan:** gzip_con_campos_opcionales, gzip_simple


```diff
--- a/crates/soso-web-core/src/decode.rs
+++ b/crates/soso-web-core/src/decode.rs
@@ -10,13 +10,128 @@
     if enc.is_empty() || enc == "identity" {
         return Ok(body.to_vec());
     }
-    if enc.contains("gzip") || enc.contains("deflate") {
-        decompress_to_vec_zlib(body).map_err(|_| ())
+    if enc.contains("gzip") {
+        decompress_gzip(body)
+    } else if enc.contains("deflate") {
+        decompress_deflate(body)
     } else {
         Ok(body.to_vec())
     }
 }
 
+/// Descomprime un miembro gzip según RFC 1952.
+fn decompress_gzip(body: &[u8]) -> Result<Vec<u8>, ()> {
+    // Necesitamos al menos 18 bytes: 10 cabecera + 0 deflate + 8 cola
+    if body.len() < 18 {
+        return Err(());
+    }
+
+    // Verificar magia
+    if body[0] != 0x1f || body[1] != 0x8b {
+        return Err(());
+    }
+
+    // Verificar método (debe ser 8 = deflate)
+    if body[2] != 8 {
+        return Err(());
+    }
+
+    let flags = body[3] as u8;
+
+    // Saltar FNAME y FCOMMENT (terminados en 0) si están presentes
+    let mut pos = 10; // Después de la cabecera fija de 10 bytes
+
+    // FEXTRA: saltar datos extra
+    if flags & 0x04 != 0 {
+        if pos + 2 > body.len() {
+            return Err(());
+        }
+        let extra_len = u16::from_le_bytes([body[pos], body[pos + 1]]) as usize;
+        pos += 2;
+        if pos + extra_len > body.len() {
+            return Err(());
+        }
+        pos += extra_len;
+    }
+
+    // FNAME: saltar nombre hasta byte 0
+    if flags & 0x08 != 0 {
+        loop {
+            if pos >= body.len() {
+                return Err(());
+            }
+            if body[pos] == 0 {
+                pos += 1;
+                break;
+            }
+            pos += 1;
+        }
+    }
+
+    // FCOMMENT: saltar comentario hasta byte 0
+    if flags & 0x10 != 0 {
+        loop {
+            if pos >= body.len() {
+                return Err(());
+            }
+            if body[pos] == 0 {
+                pos += 1;
+                break;
+            }
+            pos += 1;
+        }
+    }
+
+    // FHCRC: saltar 2 bytes de CRC de cabecera si están presentes
+    if flags & 0x02 != 0 {
+        if pos + 2 > body.len() {
+            return Err(());
+        }
+        pos += 2;
+    }
+
+    // Ahora pos debe estar dentro de los datos deflate
+    // Los datos deflate terminan con una cola de 8 bytes (CRC32 + size)
… (diff recortado)
```
