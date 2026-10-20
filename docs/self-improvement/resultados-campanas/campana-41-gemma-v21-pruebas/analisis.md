# Análisis de ejecucion-41-gemma-v21-pruebas

0 aceptadas de 10. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R31 | error | INTENTO NO TERMINADO: agotó turnos, herramientas o falló la infraestructura |
| R32 | bloqueada | NO CAMBIÓ NADA: el agente terminó sin editar (o sus ediciones se revirtieron) |
| R33 | error | INTENTO NO TERMINADO: agotó turnos, herramientas o falló la infraestructura |
| R34 | rechazada | CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy |
| R35 | bloqueada | NO CAMBIÓ NADA: el agente terminó sin editar (o sus ediciones se revirtieron) |
| R36 | bloqueada | NO CAMBIÓ NADA: el agente terminó sin editar (o sus ediciones se revirtieron) |
| R37 | rechazada | CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta |
| R38 | error | NO CAMBIÓ NADA: el agente terminó sin editar (o sus ediciones se revirtieron) |
| R39 | rechazada | CASO LÍMITE NO CUBIERTO: fallan indices_enormes_no_desbordan |
| R40 | rechazada | CASO LÍMITE NO CUBIERTO: fallan pruebas_agente::test_excl_without_creat |


## R31 — error (1086 s, 38 herramientas)

**Causa (orientativa):** INTENTO NO TERMINADO: agotó turnos, herramientas o falló la infraestructura

**Ediciones:** {'aplicada': 2, 'otro error': 3, 'sin efecto': 6, 'bloqueada': 2, 'rechazo sintaxis': 11}; pruebas propias escritas: 0


```diff
--- a/crates/sosomfs/src/catalog.rs
+++ b/crates/sosomfs/src/catalog.rs
@@ -50,7 +50,14 @@
             return Err(());
         }
         let nmodels = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
+let mut off = 4usize;
+if nmodels > 0 && (data.len() - off) < nmodels * (NAME_MAX + 32) {
+    return Err(());
+}
         let mut off = 4usize;
+if nmodels > 0 && (data.len() - off) < nmodels * (NAME_MAX + 32) {
+    return Err(());
+}
         let mut models = Vec::with_capacity(nmodels);
         for _ in 0..nmodels {
             let (m, used) = read_model(&data[off..])?;
@@ -218,3 +225,81 @@
     }
     Ok(data)
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+    #[test]
+    fn test_parse_empty_catalog() {
+        let data = vec![0, 0, 0, 0];
+        assert!(Catalog::parse(&data).is_ok());
+    }
+
+    #[test]
+    fn test_parse_truncated_header() {
+        let data = vec![0, 0, 0];
+        assert!(Catalog::parse(&data).is_err());
+    }
+
+    #[test]
+    fn test_parse_truncated_model_name() {
+        let mut data = vec![0, 0, 0, 1]; // 1 model
+        data.push(10); // name len 10
+        data.extend_from_slice(&[0u8; 5]); // only 5 bytes of name
+        assert!(Catalog::parse(&data).is_err());
+    }
+
+    #[test]
+    fn test_parse_impossible_model_count() {
+        let data = vec![255, 255, 255, 255, 0, 0, 0, 0]; // 255 models
+        assert!(Catalog::parse(&data).is_err());
+    }
+
+    #[test]
+    fn test_parse_impossible_shard_count() {
+        let mut data = vec![0, 0, 0, 1]; // 1 model
+        data.push(1); // name len 1
+        data.extend_from_slice(&[0u8; 31]); // name padding
+        data.extend_from_slice(&[0u8; 8]); // manifest_lba
+        data.extend_from_slice(&[0u8; 8]); // manifest_blocks
+        data.extend_from_slice(&[0u8; 8]); // index_lba
+        data.extend_from_slice(&[0u8; 8]); // index_blocks
+        data.extend_from_slice(&[255, 255, 255, 255]); // 255 shards
+        assert!(Catalog::parse(&data).is_err());
+    }
+
+    #[test]
+    fn test_parse_truncated_shard_extents() {
+        let mut data = vec![0, 0, 0, 1]; // 1 model
+        data.push(1); // name len 1
+        data.extend_from_slice(&[0u8; 31]); // name padding
+        data.extend_from_slice(&[0u8; 8]); // manifest_lba
+        data.extend_from_slice(&[0u8; 8]); // manifest_blocks
+        data.extend_from_slice(&[0u8; 8]); // index_lba
+        data.extend_from_slice(&[0u8; 8]); // index_blocks
+        data.extend_from_slice(&[0, 0, 0, 0]); // 0 shards
+        // This is valid, but let's make it 1 shard
+        data[116..120].copy_from_slice(&[1, 0, 0, 0]);
+        // Shard:path len 1, path 1 byte, padding 31, byte_len 8, nextents 1
+        data.push(1); // plen
+        data.push(0); // path
+        data.extend_from_slice(&[0u8; 31]); // padding
+        data.extend_from_slice(&[0u8; 8]); // byte_len
+        data.extend_from_slice(&[1, 0]); // nextents = 1
+        // Now it expects 28 bytes for extent, but we give nothing
+        assert!(Catalog::parse(&data).is_err());
+    }
+
+    #[test]
+    fn test_parse_valid_catalog() {
+        let mut data = vec![0, 0, 0, 1]; // 1 model
+        data.push(1); // name len 1
… (diff recortado)
```


## R32 — bloqueada (63 s, 12 herramientas)

**Causa (orientativa):** NO CAMBIÓ NADA: el agente terminó sin editar (o sus ediciones se revirtieron)

**Ediciones:** {}; pruebas propias escritas: 0


## R33 — error (2152 s, 37 herramientas)

**Causa (orientativa):** INTENTO NO TERMINADO: agotó turnos, herramientas o falló la infraestructura

**Ediciones:** {'aplicada': 1, 'sin efecto': 11, 'rechazo llaves': 4, 'bloqueada': 1}; pruebas propias escritas: 0


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
@@ -27,9 +27,12 @@
         let id = &data[pos..pos + 4];
         let sz = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
         pos += 8;
-        if pos + sz > data.len() {
-            break;
+        
+        let mut current_sz = sz;
+        if pos + current_sz > data.len() {
+            current_sz = data.len() - pos;
         }
+
         if id == b"fmt " && sz >= 16 {
             let audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
             channels = u16::from_le_bytes(data[pos + 2..pos + 4].try_into().unwrap());
@@ -39,16 +42,25 @@
                 return Err(WavError::Unsupported);
             }
         } else if id == b"data" {
-            if bits != 16 || channels != 1 {
+            if bits != 16 || (channels != 1 && channels != 2) {
                 return Err(WavError::Unsupported);
             }
-            let bytes = &data[pos..pos + sz];
+            let bytes = &data[pos..pos + current_sz];
             pcm.reserve(bytes.len() / 2);
             for chunk in bytes.chunks_exact(2) {
-                pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
+                if channels == 1 {
+                    pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
+                } else {
+                    // Stereo:4 bytes per sample
+                    if chunk.len() >= 4 {
+                        let izq = i16::from_le_bytes([chunk[0], chunk[1]]) as i32;
+                        let der = i16::from_le_bytes([chunk[2], chunk[3]]) as i32;
+                        pcm.push(((izq + der) / 2) as i16);
+                    }
+                }
             }
         }
-        pos += sz + (sz & 1);
+        pos += current_sz + (current_sz & 1);
     }
     if pcm.is_empty() || sample_rate == 0 {
         return Err(WavError::BadFmt);
@@ -75,3 +87,103 @@
     }
     out
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+    #[test]
+    fn test_stereo_support() {
+        // WAV estéreo de 16 bits. 
+        // fmt:channels=2, bits=16, audio_format=1
+        // data:2 muestras estéreo (4 bytes por muestra)
+        let mut data = Vec::new();
+        data.extend_from_slice(b"RIFFx00000000WAVEfmt ");
+        data.extend_from_slice(&1u16.to_le_bytes()); // audio_format
+        data.extend_from_slice(&2u16.to_le_bytes()); // channels
+        data.extend_from_slice(&44100u32.to_le_bytes()); // sample_rate
+        data.extend_from_slice(&0u16.to_le_bytes()); // byte_rate
+        data.extend_from_slice(&16u16.to_le_bytes()); // block_align
+        data.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
+        data.extend_from_slice(&0u32.to_le_bytes()); // extra
+        data.extend_from_slice(b"data");
+        data.extend_from_slice(&8u32.to_le_bytes()); // size
+        // Muestra 1:izq=1000, der=2000 -> media=1500
+        data.extend_from_slice(&1000i16.to_le_bytes());
+        data.extend_from_slice(&2000i16.to_le_bytes());
+        // Muestra 2:izq=-1000, der=-2000 -> media=-1500
+        data.extend_from_slice(&(-1000i16).to_le_bytes());
+        data.extend_from_slice(&(-2000i16).to_le_bytes());
+
+        let result = parse_pcm16_mono(&data);
+        assert_eq!(result, Ok((44100, vec![1500, -1500])));
+    }
… (diff recortado)
```


## R34 — rechazada (218 s, 18 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy

**Ediciones:** {'aplicada': 3}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/soso-llm-core/tests/aceptacion_R34.rs:60:9`: assertion `left == right` failed
- `crates/soso-llm-core/tests/aceptacion_R34.rs:23:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** greedy_ignora_los_nan, sin_distribucion_se_comporta_como_greedy


```diff
--- a/crates/soso-llm-core/src/sample.rs
+++ b/crates/soso-llm-core/src/sample.rs
@@ -38,30 +38,79 @@
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
+        for (i, &l) in logits.iter().enumerate() {
+            if l.is_nan() {
+                continue;
+            }
+            if l > max_val {
+                max_val = l;
+                max_idx = i as u32;
+            }
+        }
+
+        if max_val == f32::NEG_INFINITY {
+            // Todos son NaN o -inf. 
+            // Si hay -inf, el pedido dice que greedy devuelva el menor índice.
+            // Si hay NaN, el pedido dice que si todos son NaN, devuelva 0.
+            // En ambos casos, si no hay nada "mejor" que -inf, devolvemos 0.
+            // Pero hay que verificar si hay -inf específicamente.
+            let has_neg_inf = logits.iter().any(|&l| l == f32::NEG_INFINITY);
+            if has_neg_inf {
+                // Si hay -inf, el menor índice es 0.
+                return 0;
+            }
+            return 0;
+        }
+
+        max_idx
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
+        let mut has_nan = false;
+
+        for &l in logits {
+            if l.is_nan() {
+                has_nan = true;
+            } else if l.is_infinite() {
+                if l.is_sign_positive() {
+                    has_inf = true;
+                }
+            } else if l > max_val {
+                max_val = l;
+            }
+        }
+
+        // Si hay +inf o si todos son NaN o -inf (max_val sigue siendo -inf)
+        // y no hay ningún valor finito mayor a -inf.
+        if has_inf || (max_val == f32::NEG_INFINITY && (has_nan || logits.iter().any(|&l| l == f32::NEG_INFINITY))) {
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
+                let p = libm::expf((l - max_val) * inv_t);
+                self.probs.push(p);
+            }
+            sum += self.probs[self.probs.len() - 1];
         }
… (diff recortado)
```


## R35 — bloqueada (114 s, 11 herramientas)

**Causa (orientativa):** NO CAMBIÓ NADA: el agente terminó sin editar (o sus ediciones se revirtieron)

**Ediciones:** {}; pruebas propias escritas: 0


## R36 — bloqueada (72 s, 8 herramientas)

**Causa (orientativa):** NO CAMBIÓ NADA: el agente terminó sin editar (o sus ediciones se revirtieron)

**Ediciones:** {}; pruebas propias escritas: 0


## R37 — rechazada (197 s, 13 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta

**Ediciones:** {'aplicada': 2, 'sin efecto': 2}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/soso-http/tests/aceptacion_R37.rs:20:71`: called `Result::unwrap()` on an `Err` value: Parse

**Pruebas reservadas que fallan:** la_query_se_conserva_aunque_no_haya_ruta


```diff
--- a/crates/soso-http/src/lib.rs
+++ b/crates/soso-http/src/lib.rs
@@ -598,18 +598,64 @@
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
+    let rest = rest.split('#').next().unwrap_or(rest);
+
+    // El host termina en el primer / o ? o al acabar la cadena
+    let (host_port, path_part) = match rest.split_once('/') {
+        Some((h, p)) => (h, p),
+        None => {
+            if let Some(idx) = rest.find('?') {
+                (&rest[..idx], &rest[idx..])
+            } else {
+                (rest, "")
+            }
+        }
     };
-    if let Some((host, port)) = host_port.rsplit_once(':') {
-        let port: u16 = port.parse().map_err(|_| HttpError::Parse)?;
-        Ok(("https", host, port, path))
+
+    // El host_port puede contener el puerto (ej:h.org:80)
+    // Pero si el host_port contiene '?', significa que la ruta empezó con '?'
+    // y el host_port es en realidad el host.
+    let (host, port_part) = if host_port.contains(':') {
+        let parts:Vec<&str> = host_port.split(':').collect();
+        if parts.len() == 2 {
+            (parts[0], Some(parts[1]))
+        } else {
+            // Caso como "host:port:extra" o similar, o si el puerto está vacío
+            (parts[0], None)
+        }
     } else {
-        Ok(("https", host_port, 443, path))
+        (host_port, None)
+    };
+
+    // Validaciones de host y puerto
+    if host.is_empty() || host.contains('@') {
+        return Err(HttpError::Parse);
     }
+
+    let port = match port_part {
+        Some(p) if !p.is_empty() => {
+            let p_num:u16 = p.parse().map_err(|_| HttpError::Parse)?;
+            if p_num == 0 { return Err(HttpError::Parse); }
+            p_num
+        }
+        Some(_) => return Err(HttpError::Parse), // Puerto vacío
+        None => 443,
+    };
+
+    // Construir la ruta
+    let path = if path_part.is_empty() {
+        "/".to_string()
+    } else if path_part.starts_with('/') {
+        path_part.to_string()
+    } else {
+        format!("/{}", path_part)
+    };
+
+    Ok(("https", host, port, path))
 }
 
 fn parse_response_head(head: &[u8]) -> Result<(u16, Vec<(String, String)>), HttpError> {
@@ -844,3 +890,83 @@
         set_wall_clock(test_wall_clock);
     }
 }
+
+    #[cfg(test)]
+    mod pruebas_agente {
+        use super::*;
+
+        #[test]
+        fn test_parse_url_basic() {
+            let (scheme, host, port, path) = parse_url("https://h.org/path").unwrap();
+            assert_eq!(scheme, "https");
+            assert_eq!(host, "h.org");
+            assert_eq!(port, 443);
… (diff recortado)
```


## R38 — error (658 s, 4 herramientas)

**Causa (orientativa):** NO CAMBIÓ NADA: el agente terminó sin editar (o sus ediciones se revirtieron)

**Ediciones:** {}; pruebas propias escritas: 0


## R39 — rechazada (116 s, 11 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan indices_enormes_no_desbordan

**Ediciones:** {'aplicada': 4, 'otro error': 1}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/block-dev/src/lib.rs:98:38`: attempt to add with overflow

**Pruebas reservadas que fallan:** indices_enormes_no_desbordan


```diff
--- a/crates/block-dev/src/lib.rs
+++ b/crates/block-dev/src/lib.rs
@@ -91,17 +91,17 @@
         (self.data.len() / BLOCK_SIZE) as u64
     }
 
-    fn read_block(&mut self, block: u64, buf: &mut Block) -> Result<(), BlockError> {
+    fn read_block(&mut self, block:u64, buf:&mut Block) -> Result<(), BlockError> {
         #[cfg(feature = "std")]
         self.reads.set(self.reads.get() + 1);
-        let off = block as usize * BLOCK_SIZE;
+        let off = block.checked_mul(BLOCK_SIZE as u64).ok_or(BlockError::OutOfRange)? as usize;
         let src = self.data.get(off..off + BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
         buf.copy_from_slice(src);
         Ok(())
     }
 
-    fn write_block(&mut self, block: u64, buf: &Block) -> Result<(), BlockError> {
-        let off = block as usize * BLOCK_SIZE;
+    fn write_block(&mut self, block:u64, buf:&Block) -> Result<(), BlockError> {
+        let off = block.checked_mul(BLOCK_SIZE as u64).ok_or(BlockError::OutOfRange)? as usize;
         let dst = self.data.get_mut(off..off + BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
         dst.copy_from_slice(buf);
         Ok(())
@@ -118,13 +118,13 @@
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
+        let off = start.checked_mul(BLOCK_SIZE as u64).ok_or(BlockError::OutOfRange)? as usize;
         let src = self
             .data
             .get(off..off + buf.len())
@@ -253,3 +253,52 @@
 
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
+        // u64::MAX * BLOCK_SIZE desbordará en el cálculo de off
+        let result = dev.read_block(u64::MAX, &mut buf);
+        assert_eq!(result, Err(BlockError::OutOfRange));
+    }
+
+    #[test]
+    fn test_write_block_out_of_range_large_block() {
+        let mut dev = MemBlockDevice::new(10);
+        let buf = [0u8; BLOCK_SIZE];
+        // u64::MAX * BLOCK_SIZE desbordará en el cálculo de off
+        let result = dev.write_block(u64::MAX, &buf);
+        assert_eq!(result, Err(BlockError::OutOfRange));
+    }
+
+    #[test]
+    fn test_read_blocks_out_of_range_large_start() {
+        let mut dev = MemBlockDevice::new(10);
+        let mut buf = [0u8; BLOCK_SIZE * 2];
+        // start * BLOCK_SIZE desbordará
+        let result = dev.read_blocks(u64::MAX, &mut buf);
+        assert_eq!(result, Err(BlockError::OutOfRange));
+    }
+
+    #[test]
+    fn test_read_blocks_out_of_range_large_len() {
+        let mut dev = MemBlockDevice::new(10);
+        // buf.len() es válido, pero start * BLOCK_SIZE + buf.len() desbordará
+        let mut buf = [0u8; BLOCK_SIZE * 2];
+        let result = dev.read_blocks(u64::MAX - 1, &mut buf);
+        assert_eq!(result, Err(BlockError::OutOfRange));
+    }
+
+    #[test]
+    fn test_read_blocks_out_of_range_valid_start_but_too_large() {
+        let mut dev = MemBlockDevice::new(10);
+        let mut buf = [0u8; BLOCK_SIZE * 2];
+        // start es válido (0), pero el acceso es fuera de la imagen (10 bloques)
+        let result = dev.read_blocks(9, &mut buf);
… (diff recortado)
```


## R40 — rechazada (348 s, 24 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan pruebas_agente::test_excl_without_creat

**Ediciones:** {'aplicada': 2, 'sin efecto': 5, 'bloqueada': 8}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/soso-abi/src/lib.rs:681:9`: assertion failed: open_flags_validos(O_EXCL | O_CREAT).is_ok()

**Pruebas reservadas que fallan:** pruebas_agente::test_excl_without_creat


```diff
--- a/crates/soso-abi/src/lib.rs
+++ b/crates/soso-abi/src/lib.rs
@@ -515,6 +515,33 @@
 /// Con `O_CREAT`: falla si el fichero ya existe.
 pub const O_EXCL: u64 = 16;
 
+pub const O_FLAGS_MASK:u64 = O_RDONLY | O_WRONLY | O_APPEND | O_CREAT | O_TRUNC | O_EXCL;
+
+pub fn open_flags_validos(flags:u64) -> Result<(), i64> {
+    if (flags & !O_FLAGS_MASK) != 0 {
+        return Err(-22);
+    }
+
+    let has_wronly = (flags & O_WRONLY) != 0;
+    let has_append = (flags & O_APPEND) != 0;
+    let has_creat = (flags & O_CREAT) != 0;
+    let has_trunc = (flags & O_TRUNC) != 0;
+    let has_excl = (flags & O_EXCL) != 0;
+
+    if (has_append || has_creat || has_trunc) && !has_wronly {
+        return Err(-22);
+    }
+
+    if has_excl && !has_creat {
+        return Err(-22);
+    }
+
+    if has_append && has_trunc {
+        return Err(-22);
+    }
+
+    Ok(())
+}
 /// Reloj de pared (epoch Unix).
 pub const CLOCK_REALTIME: u64 = 0;
 /// Reloj monótono desde el arranque.
@@ -610,3 +637,56 @@
 pub fn wait_decode(v: i64) -> (u64, u8) {
     ((v as u64) >> 8, v as u8)
 }
+
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+    #[test]
+    fn test_o_flags_mask() {
+        let mask = O_FLAGS_MASK;
+        assert_eq!(mask, O_RDONLY | O_WRONLY | O_APPEND | O_CREAT | O_TRUNC | O_EXCL);
+    }
+
+    #[test]
+    fn test_valid_rdonly() {
+        assert!(open_flags_validos(O_RDONLY).is_ok());
+        assert!(open_flags_validos(0).is_ok());
+    }
+
+    #[test]
+    fn test_invalid_bits() {
+        assert_eq!(open_flags_validos(1 << 40), Err(-22)); // -EINVAL
+        assert_eq!(open_flags_validos(1 << 10), Err(-22));
+    }
+
+    #[test]
+    fn test_missing_wronly() {
+        // O_APPEND, O_CREAT, O_TRUNC sin O_WRONLY
+        assert_eq!(open_flags_validos(O_APPEND), Err(-22));
+        assert_eq!(open_flags_validos(O_CREAT), Err(-22));
+        assert_eq!(open_flags_validos(O_TRUNC), Err(-22));
+        
+        // Con O_WRONLY son válidos
+        assert!(open_flags_validos(O_WRONLY | O_APPEND).is_ok());
+        assert!(open_flags_validos(O_WRONLY | O_CREAT).is_ok());
+        assert!(open_flags_validos(O_WRONLY | O_TRUNC).is_ok());
+    }
+
+    #[test]
+    fn test_excl_without_creat() {
+        // O_EXCL sin O_CREAT
+        assert_eq!(open_flags_validos(O_EXCL), Err(-22));
+        // O_EXCL con O_CREAT es válido
+        assert!(open_flags_validos(O_EXCL | O_CREAT).is_ok());
+    }
+
+    #[test]
+    fn test_append_trunc_contradiction() {
+        // O_APPEND y O_TRUNC a la vez
+        assert_eq!(open_flags_validos(O_WRONLY | O_APPEND | O_TRUNC), Err(-22));
+        // O_APPEND solo o O_TRUNC solo con O_WRONLY son válidos
+        assert!(open_flags_validos(O_WRONLY | O_APPEND).is_ok());
… (diff recortado)
```
