# Análisis de ejecucion-51-qwen36-iq3-razonamiento

5 aceptadas de 10. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R32 | rechazada | CASO LÍMITE NO CUBIERTO: fallan resolve_trailing_dots |
| R34 | rechazada | NO COMPILA con las pruebas reservadas: no method named `copied` found for enum `Option<&[u32]>` in the current |
| R35 | rechazada | HABRÍA PASADO la prueba reservada |
| R36 | rechazada | CASO LÍMITE NO CUBIERTO: fallan parse_v4_num_layers_large_ok, parse_v6_num_layers_large_ok |
| R38 | rechazada | NO COMPILA con las pruebas reservadas: mismatched types; no method named `ok` found for enum `Option<T>` in th |


## R32 — rechazada (1080 s, 41 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan resolve_trailing_dots

**Ediciones:** {'otro error': 4, 'aplicada': 7}; pruebas propias escritas: 8

**Panics de las pruebas reservadas:**
- `crates/sosofs/tests/roundtrip.rs:189:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** resolve_trailing_dots


```diff
--- a/crates/sosofs/src/fs.rs
+++ b/crates/sosofs/src/fs.rs
@@ -210,16 +210,31 @@
     }
 
     /// Resuelve una ruta absoluta (o relativa a la raíz) a un inode.
+    ///
+    /// Soporta `..` para subir un nivel: si estamos en un directorio, sube al
+    /// padre; si estamos en la raíz, es un no-op. Si el camino actual no es un
+    /// directorio, `..` falla con `NotADir`.
     pub fn resolve(&mut self, path: &str) -> Result<u64, FsError> {
-        let mut ino = ROOT_INODE;
+        let mut stack: Vec<u64> = vec![ROOT_INODE];
+
         for comp in path.split('/').filter(|c| !c.is_empty() && *c != ".") {
+            let ino = *stack.last().unwrap();
             let st = self.stat_inode(ino)?;
             if st.file_type != FT_DIR {
                 return Err(FsError::NotADir);
             }
-            ino = self.lookup(ino, comp)?;
+
+            if comp == ".." {
+                if stack.len() > 1 {
+                    stack.pop();
+                }
+            } else {
+                let child = self.lookup(ino, comp)?;
+                stack.push(child);
+            }
         }
-        Ok(ino)
+
+        Ok(*stack.last().unwrap())
     }
 
     pub fn read_dir(&mut self, dir: u64) -> Result<Vec<(String, u64)>, FsError> {
```


## R34 — rechazada (713 s, 36 herramientas)

**Causa (orientativa):** NO COMPILA con las pruebas reservadas: no method named `copied` found for enum `Option<&[u32]>` in the current scope; no method named `copied` found for enum `Option<u32>` in the current scope

**Ediciones:** {'aplicada': 7, 'otro error': 1}; pruebas propias escritas: 2


```diff
--- a/crates/soso-llm-core/src/sample.rs
+++ b/crates/soso-llm-core/src/sample.rs
@@ -39,12 +39,17 @@
     }
 
     fn argmax(logits: &[f32]) -> u32 {
-        logits
-            .iter()
-            .enumerate()
-            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal))
-            .map(|(i, _)| i as u32)
-            .unwrap_or(0)
+        // Buscar el mayor logit finito; en empate devolver el menor índice.
+        // Si todos son NaN, devolver 0.
+        let mut best_idx: Option<u32> = None;
+        let mut best_val: f32 = f32::NEG_INFINITY;
+        for (i, &l) in logits.iter().enumerate() {
+            if !l.is_nan() && l > best_val {
+                best_val = l;
+                best_idx = Some(i as u32);
+            }
+        }
+        best_idx.unwrap_or(0)
     }
 
     pub fn sample(&mut self, logits: &[f32]) -> u32 {
@@ -57,26 +62,40 @@
         let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
         let inv_t = 1.0 / self.temp;
         let mut sum = 0.0f32;
+        // Verificar si hay algún +inf: si es así, fallback a greedy
+        let has_pos_inf = logits.iter().any(|&l| l == f32::INFINITY);
+        if has_pos_inf {
+            return Self::argmax(logits);
+        }
         for &l in logits {
-            let p = libm::expf((l - max) * inv_t);
+            let p = if l.is_nan() || l == f32::NEG_INFINITY {
+                0.0
+            } else {
+                libm::expf((l - max) * inv_t)
+            };
             self.probs.push(p);
             sum += p;
         }
+        // Si la suma es 0 o NaN, fallback a greedy
+        if sum <= 0.0 || sum.is_nan() {
+            return Self::argmax(logits);
+        }
         // índices ordenados por probabilidad descendente
         self.idx.clear();
         self.idx.extend(0..n as u32);
         let probs = &self.probs;
         self.idx.sort_unstable_by(|&a, &b| {
-            probs[b as usize]
-                .partial_cmp(&probs[a as usize])
+            probs.get(b as usize)
+                .copied()
+                .partial_cmp(probs.get(a as usize).copied())
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
@@ -84,19 +103,26 @@
         }
         // muestrear dentro del núcleo
         let mut r = self.next_f32() * mass;
-        for &i in &self.idx[..k] {
-            r -= self.probs[i as usize];
+        let idx_ref = self.idx.as_slice();
+        let k_us = k as usize;
+        for &i in idx_ref.get(..k_us).copied().copied().unwrap_or(&[][..]) {
+            r -= self.probs.get(i as usize).copied().unwrap_or(0.0);
             if r <= 0.0 {
                 return i;
             }
         }
-        self.idx[k - 1]
+        // fallback: último índice del núcleo
+        if k == 0 {
+            return Self::argmax(logits);
… (diff recortado)
```


## R35 — rechazada (1406 s, 45 herramientas)

**Causa (orientativa):** HABRÍA PASADO la prueba reservada

**Ediciones:** {'aplicada': 6, 'otro error': 1, 'rechazo llaves': 1}; pruebas propias escritas: 5


```diff
--- a/crates/gguf2som/src/lib.rs
+++ b/crates/gguf2som/src/lib.rs
@@ -1030,6 +1030,9 @@
     for _ in 0..tensor_count {
         let name = read_string(file)?;
         let n_dims = read_u32(file)? as usize;
+        if n_dims > 8 {
+            return Err(format!("tensor '{name}' tiene {n_dims} dimensiones (máximo 8)"));
+        }
         let mut shape = Vec::with_capacity(n_dims);
         for _ in 0..n_dims {
             shape.push(read_u64(file)?);
@@ -1228,7 +1231,11 @@
 }
 
 fn read_string<R: Read + Seek>(file: &mut R) -> Result<String, String> {
-    let len = read_u64(file)? as usize;
+    let len = read_u64(file)?;
+    if len > 16 * 1024 * 1024 {
+        return Err("cadena de metadatos excede 16 MiB".into());
+    }
+    let len = len as usize;
     let mut buf = vec![0u8; len];
     file.read_exact(&mut buf)?;
     String::from_utf8(buf).map_err(|e| e.to_string())
@@ -1990,3 +1997,61 @@
         assert_eq!(tok.encode_trozo("hola mundo", false, true), vec![2, 3]);
     }
 }
+
+    #[cfg(test)]
+    mod pruebas_agente {
+        use super::*;
+        use crate::io::std_file::File as IoFile;
+
+        fn build_gguf(
+            kv_count: u64,
+            tensor_count: u64,
+            kv_entries: &mut Vec<u8>,
+            tensor_entries: &mut Vec<u8>,
+        ) -> Vec<u8> {
+            let mut buf = Vec::new();
+            buf.extend_from_slice(b"GGUF");
+            buf.extend_from_slice(&3u32.to_le_bytes());
+            buf.extend_from_slice(&kv_count.to_le_bytes());
+            buf.extend_from_slice(&tensor_count.to_le_bytes());
+            buf.extend_from_slice(kv_entries);
+            buf.extend_from_slice(tensor_entries);
+            buf
+        }
+
+        fn gguf_string(out: &mut Vec<u8>, s: &str) {
+            out.extend_from_slice(&(s.len() as u64).to_le_bytes());
+            out.extend_from_slice(s.as_bytes());
+        }
+
+        fn gguf_kv_u32(out: &mut Vec<u8>, key: &str, v: u32) {
+            gguf_string(out, key);
+            out.extend_from_slice(&4u32.to_le_bytes());
+            out.extend_from_slice(&v.to_le_bytes());
+        }
+
+        fn gguf_kv_str(out: &mut Vec<u8>, key: &str, v: &str) {
+            gguf_string(out, key);
+            out.extend_from_slice(&8u32.to_le_bytes());
+            gguf_string(out, v);
+        }
+
+
+        #[test]
+        fn cadena_u64_max_no_panic() {
+            let mut kv = Vec::new();
+            gguf_string(&mut kv, "general.architecture");
+            kv.extend_from_slice(&8u32.to_le_bytes());
+            kv.extend_from_slice(&u64::MAX.to_le_bytes());
+
+            let gguf_data = build_gguf(1, 0, &mut kv, &mut Vec::new());
+            let tmp = std::env::temp_dir().join("gguf-test-u64max");
+            std::fs::write(&tmp, gguf_data).unwrap();
+            let mut f = IoFile::open(&tmp).unwrap();
+            let result = parse_gguf(&mut f);
+            let _ = std::fs::remove_file(&tmp);
+            assert!(result.is_err(), "debería devolver Err con u64::MAX");
+        }
+
+
+    }
```


## R36 — rechazada (900 s, 40 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan parse_v4_num_layers_large_ok, parse_v6_num_layers_large_ok

**Ediciones:** {'otro error': 2, 'aplicada': 9}; pruebas propias escritas: 6

**Panics de las pruebas reservadas:**
- `crates/sosomodel/tests/roundtrip.rs:366:42`: v6 num_layers=1025 should parse: ()
- `crates/sosomodel/tests/roundtrip.rs:357:42`: v4 num_layers=1025 should parse: ()

**Pruebas reservadas que fallan:** parse_v4_num_layers_large_ok, parse_v6_num_layers_large_ok


```diff
--- a/crates/sosomodel/src/manifest.rs
+++ b/crates/sosomodel/src/manifest.rs
@@ -842,11 +842,19 @@
 
     pub fn parse(data: &[u8]) -> Result<Self, ()> {
         let (version, body) = parse_som(data)?;
+        // v0 no se admite; v7+ no existe todavía.
+        if version == 0 || version > 6 {
+            return Err(());
+        }
         let mut r = Reader::new(body);
         let name = String::from(r.cstr()?);
         let vocab_size = r.u32()?;
         let hidden_dim = r.u32()?;
         let num_layers = r.u32()?;
+        // v1–v3: num_layers no puede superar 1024.
+        if version <= 3 && num_layers > 1024 {
+            return Err(());
+        }
         let num_heads = r.u32()?;
         let ffn_dim = r.u32()?;
         let max_seq = r.u32()?;
```


## R38 — rechazada (1636 s, 37 herramientas)

**Causa (orientativa):** NO COMPILA con las pruebas reservadas: mismatched types; no method named `ok` found for enum `Option<T>` in the current scope

**Ediciones:** {'otro error': 5, 'aplicada': 8}; pruebas propias escritas: 5


```diff
--- a/crates/soso-web-core/src/decode.rs
+++ b/crates/soso-web-core/src/decode.rs
@@ -10,84 +10,166 @@
     if enc.is_empty() || enc == "identity" {
         return Ok(body.to_vec());
     }
-    if enc.contains("gzip") || enc.contains("deflate") {
-        decompress_to_vec_zlib(body).map_err(|_| ())
-    } else {
-        Ok(body.to_vec())
-    }
-}
-
-/// Convierte bytes HTML a `String` (UTF-8 con fallback Latin-1).
-pub fn bytes_to_text(body: &[u8], charset: Option<&str>) -> String {
-    let cs = charset.unwrap_or("utf-8").trim().to_ascii_lowercase();
-    if cs.contains("utf-8") || cs.contains("utf8") {
-        if let Ok(s) = core::str::from_utf8(body) {
-            return String::from(s);
+    if enc.contains("deflate") && !enc.contains("gzip") {
+        if let Ok(data) = decompress_to_vec_zlib(body) {
+            return Ok(data);
         }
-    }
-    body.iter().map(|&b| b as char).collect()
-}
-
-/// Resuelve entidades HTML básicas en texto.
-pub fn decode_entities(input: &str) -> String {
-    let mut out = String::with_capacity(input.len());
-    let bytes = input.as_bytes();
-    let mut i = 0;
-    while i < bytes.len() {
-        if bytes[i] == b'&' {
-            if let Some((ch, consumed)) = parse_entity_char(&bytes[i..]) {
-                if let Some(c) = ch {
-                    out.push(c);
-                }
-                i += consumed;
-                continue;
-            }
+        let data = miniz_oxide::inflate::decompress_to_vec(body);
+        if !data.is_empty() {
+            return Ok(data);
         }
-        out.push(bytes[i] as char);
-        i += 1;
+        return Err(());
     }
-    out
+    if enc.contains("gzip") {
+        return decompress_gzip(body);
+    }
+    Ok(body.to_vec())
 }
 
-fn parse_entity_char(rest: &[u8]) -> Option<(Option<char>, usize)> {
-    let end = rest.iter().position(|&b| b == b';')?;
-    let raw = core::str::from_utf8(&rest[1..end]).ok()?;
-    let total = end + 1;
-    match raw {
-        "amp" => Some((Some('&'), total)),
-        "lt" => Some((Some('<'), total)),
-        "gt" => Some((Some('>'), total)),
-        "quot" => Some((Some('"'), total)),
-        "apos" => Some((Some('\''), total)),
-        "nbsp" => Some((Some('\u{00a0}'), total)),
-        _ if raw.starts_with("#x") || raw.starts_with("#X") => {
-            let n = u32::from_str_radix(&raw[2..], 16).ok()?;
-            Some((char::from_u32(n), total))
+/// Descomprime un cuerpo gzip (RFC 7692).
+fn decompress_gzip(body: &[u8]) -> Result<Vec<u8>, ()> {
+    if body.len() < 10 {
+        return Err(());
+    }
+    if body[0] != 0x1f || body[1] != 0x8b {
+        return Err(());
+    }
+    if body[2] != 8 {
+        return Err(());
+    }
+    let flags = body[3];
+    let mut pos = 10;
+    if flags & 0x02 != 0 {
+        if pos + 2 > body.len() {
+            return Err(());
+        }
+        let extra_len = u16::from_le_bytes([body[pos], body[pos + 1]]) as usize;
+        pos += 2;
+        pos += extra_len;
… (diff recortado)
```
