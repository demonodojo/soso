# Análisis de ejecucion-50-qwen36-iq3

5 aceptadas de 10. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R31 | rechazada | CASO LÍMITE NO CUBIERTO: fallan cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, |
| R32 | rechazada | NO COMPILA con las pruebas reservadas: test failed, to rerun pass `-p sosofs --test roundtrip` |
| R33 | rechazada | CASO LÍMITE NO CUBIERTO: fallan estereo_con_trama_incompleta_la_ignora, otros_formatos_no_se_admiten, sin_fmt_ |
| R35 | rechazada | CASO LÍMITE NO CUBIERTO: fallan tests::tensor_demasiadas_dimensiones |
| R38 | error | (NO TERMINÓ; estado que dejó en disco) CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_o |


## R31 — rechazada (778 s, 40 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, cualquier_truncado_es_error_y_no_panico

**Ediciones:** {'aplicada': 6, 'otro error': 2}; pruebas propias escritas: 2

**Panics de las pruebas reservadas:**
- `crates/sosomfs/src/catalog.rs:117:47`: range end index 97 out of range for slice of length 96
- `crates/sosomfs/src/catalog.rs:117:47`: range end index 97 out of range for slice of length 96
- `crates/sosomfs/src/catalog.rs:119:42`: range end index 101 out of range for slice of length 100

**Pruebas reservadas que fallan:** cabecera_de_modelo_justa_no_basta, corrupcion_aleatoria_nunca_entra_en_panico, cualquier_truncado_es_error_y_no_panico


```diff
--- a/crates/sosomfs/src/catalog.rs
+++ b/crates/sosomfs/src/catalog.rs
@@ -50,6 +50,9 @@
             return Err(());
         }
         let nmodels = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
+        if nmodels > 1024 {
+            return Err(());
+        }
         let mut off = 4usize;
         let mut models = Vec::with_capacity(nmodels);
         for _ in 0..nmodels {
@@ -115,6 +118,9 @@
     off += 8;
     let nshards = u32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
     off += 4;
+    if nshards > 1024 {
+        return Err(());
+    }
     let mut shards = Vec::with_capacity(nshards);
     for _ in 0..nshards {
         let (s, used) = read_shard(&data[off..])?;
@@ -145,12 +151,20 @@
     off += 8;
     let nextents = u16::from_le_bytes(data[off..off + 2].try_into().unwrap()) as usize;
     off += 2;
+    if nextents > 1024 {
+        return Err(());
+    }
     let mut extents = Vec::with_capacity(nextents);
     for _ in 0..nextents {
         let e = read_extent(&data[off..])?;
         off += 28;
         extents.push(e);
     }
+    // Part final fija: prefetch_next_lba(8) + prefetch_bytes(4) + cache_policy(1)
+    // + align_requirement(1) + flags(2) + shard_crc32c(4) = 20 bytes
+    if data.len() < off + 20 {
+        return Err(());
+    }
     let prefetch_next_lba = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
     off += 8;
     let prefetch_bytes = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
@@ -191,6 +205,263 @@
     })
 }
 
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+    // --- Helpers para construir catálogos válidos ---
+
+    /// Catálogo vacío: solo el contador 0
+    #[test]
+    fn debug_valid_catalog() {
+        let data = valid_catalog_with_shard();
+        eprintln!("catalog size: {}", data.len());
+        match Catalog::parse(&data) {
+            Ok(cat) => eprintln!("OK, {} models", cat.models.len()),
+            Err(()) => eprintln!("Err at data.len()={}", data.len()),
+        }
+    }
+    fn empty_catalog_bytes() -> Vec<u8> {
+        vec![0, 0, 0, 0]
+    }
+
+    /// Un catálogo válido con un modelo y un shard.
+    fn valid_catalog_with_shard() -> Vec<u8> {
+        let mut out = Vec::new();
+        // 1 modelo
+        out.extend_from_slice(&1u32.to_le_bytes());
+        // ModelEntry: nombre "x" + padding, manifest_lba, manifest_blocks, index_lba, index_blocks, 1 shard
+        let name = b"x";
+        out.push(name.len() as u8);
+        out.extend_from_slice(name);
+        out.resize(out.len() + NAME_MAX, 0); // padding del nombre
+        out.extend_from_slice(&0u64.to_le_bytes()); // manifest_lba
+        out.extend_from_slice(&1u64.to_le_bytes()); // manifest_blocks
+        out.extend_from_slice(&0u64.to_le_bytes()); // index_lba
+        out.extend_from_slice(&1u64.to_le_bytes()); // index_blocks
+        out.extend_from_slice(&1u32.to_le_bytes()); // 1 shard
+        // ShardEntry: path "s" + padding, byte_len, 0 extents, parte final
+        let spath = b"s";
+        out.push(spath.len() as u8);
+        out.extend_from_slice(spath);
+        out.resize(out.len() + PATH_MAX, 0); // padding del path
+        out.extend_from_slice(&100u64.to_le_bytes()); // byte_len
+        out.extend_from_slice(&0u16.to_le_bytes()); // 0 extents
+        out.extend_from_slice(&0u64.to_le_bytes()); // prefetch_next_lba
… (diff recortado)
```


## R32 — rechazada (799 s, 40 herramientas)

**Causa (orientativa):** NO COMPILA con las pruebas reservadas: test failed, to rerun pass `-p sosofs --test roundtrip`

**Ediciones:** {'aplicada': 6}; pruebas propias escritas: 3

**Panics de las pruebas reservadas:**
- `crates/sosofs/tests/roundtrip.rs:16:46`: called `Result::unwrap()` on an `Err` value: Os { code: 2, kind: NotFound, message: "No such file or directory" }
- `crates/sosofs/tests/roundtrip.rs:161:39`: called `Result::unwrap()` on an `Err` value: NotFound


```diff
--- a/crates/sosofs/src/fs.rs
+++ b/crates/sosofs/src/fs.rs
@@ -212,13 +212,43 @@
     /// Resuelve una ruta absoluta (o relativa a la raíz) a un inode.
     pub fn resolve(&mut self, path: &str) -> Result<u64, FsError> {
         let mut ino = ROOT_INODE;
-        for comp in path.split('/').filter(|c| !c.is_empty() && *c != ".") {
+        let mut components = path.split('/').filter(|c| !c.is_empty());
+
+        // Usamos un Vec como stack de (componente, inode_padre).
+        // Cada entrada registra el padre para poder subir con `..`.
+        let mut stack: Vec<(String, u64)> = Vec::new();
+
+        while let Some(comp) = components.next() {
+            if comp == "." {
+                continue;
+            }
+            if comp == ".." {
+                // Subir al padre. Si ya estamos en la raíz, nos quedamos ahí.
+                // Primero verificamos que el inode actual sea un directorio.
+                if stack.is_empty() {
+                    // Estamos en la raíz: `..` en la raíz sigue en la raíz.
+                    continue;
+                }
+                // Comprobar que el inode actual es un directorio.
+                let st = self.stat_inode(ino)?;
+                if st.file_type != FT_DIR {
+                    return Err(FsError::NotADir);
+                }
+                // Sacar el último par y volver al padre.
+                let (_nombre, padre) = stack.pop().unwrap();
+                ino = padre;
+                continue;
+            }
+            // Componente normal: hay que buscarlo en el directorio actual.
             let st = self.stat_inode(ino)?;
             if st.file_type != FT_DIR {
                 return Err(FsError::NotADir);
             }
-            ino = self.lookup(ino, comp)?;
+            let child = self.lookup(ino, comp)?;
+            stack.push((comp.to_string(), ino));
+            ino = child;
         }
+
         Ok(ino)
     }
 
```


## R33 — rechazada (360 s, 25 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan estereo_con_trama_incompleta_la_ignora, otros_formatos_no_se_admiten, sin_fmt_o_sin_muestras_es_badfmt

**Ediciones:** {'aplicada': 6}; pruebas propias escritas: 2

**Panics de las pruebas reservadas:**
- `crates/soso-audio/tests/aceptacion_R33.rs:66:5`: assertion `left == right` failed
- `crates/soso-audio/tests/aceptacion_R33.rs:73:5`: assertion `left == right` failed
- `crates/soso-audio/tests/aceptacion_R33.rs:125:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** estereo_con_trama_incompleta_la_ignora, otros_formatos_no_se_admiten, sin_fmt_o_sin_muestras_es_badfmt


```diff
--- a/crates/soso-audio/src/wav.rs
+++ b/crates/soso-audio/src/wav.rs
@@ -22,37 +22,69 @@
     let mut sample_rate = 0u32;
     let mut bits = 0u16;
     let mut channels = 0u16;
-    let mut pcm = Vec::new();
+    let mut audio_format = 0u16;
+    let mut has_fmt = false;
+    // Almacenar chunks data: (offset_in_data, declared_size)
+    let mut data_chunks: Vec<(usize, usize)> = Vec::new();
+
+    // Primera pasada: leer fmt y recolectar data chunks
     while pos + 8 <= data.len() {
         let id = &data[pos..pos + 4];
         let sz = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
         pos += 8;
-        if pos + sz > data.len() {
-            break;
-        }
         if id == b"fmt " && sz >= 16 {
-            let audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
+            audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
             channels = u16::from_le_bytes(data[pos + 2..pos + 4].try_into().unwrap());
             sample_rate = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap());
             bits = u16::from_le_bytes(data[pos + 14..pos + 16].try_into().unwrap());
+            has_fmt = true;
             if audio_format != 1 {
                 return Err(WavError::Unsupported);
             }
         } else if id == b"data" {
-            if bits != 16 || channels != 1 {
-                return Err(WavError::Unsupported);
-            }
-            let bytes = &data[pos..pos + sz];
-            pcm.reserve(bytes.len() / 2);
-            for chunk in bytes.chunks_exact(2) {
-                pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
-            }
+            data_chunks.push((pos, sz));
         }
         pos += sz + (sz & 1);
     }
-    if pcm.is_empty() || sample_rate == 0 {
+
+    // Verificar fmt
+    if !has_fmt {
         return Err(WavError::BadFmt);
     }
+
+    // Verificar formato soportado
+    if audio_format != 1 || bits != 16 || channels > 2 {
+        return Err(WavError::Unsupported);
+    }
+
+    // Segunda pasada: procesar data chunks y concatenar
+    let mut pcm = Vec::new();
+    for (offset, declared_sz) in &data_chunks {
+        let available = data.len().saturating_sub(*offset);
+        let read_sz = declared_sz.min(&available);
+        // Ignorar último byte si es suelto (impar)
+        let actual = read_sz - (read_sz & 1);
+        for chunk in data[*offset..*offset + actual].chunks_exact(2) {
+            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
+            pcm.push(sample);
+        }
+    }
+
+    if channels == 2 {
+        // Convertir estereo a mono: promediar pares
+        let mut mono = Vec::with_capacity(pcm.len() / 2);
+        let mut iter = pcm.into_iter();
+        while let Some(left) = iter.next() {
+            let right = iter.next().unwrap_or(0);
+            mono.push(((left as i32 + right as i32) / 2) as i16);
+        }
+        pcm = mono;
+    }
+
+    if pcm.is_empty() {
+        return Err(WavError::BadFmt);
+    }
+
     Ok((sample_rate, pcm))
 }
 
@@ -75,3 +107,327 @@
     }
     out
 }
… (diff recortado)
```


## R35 — rechazada (1024 s, 39 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan tests::tensor_demasiadas_dimensiones

**Ediciones:** {'aplicada': 6, 'rechazo llaves': 3, 'otro error': 1}; pruebas propias escritas: 5

**Panics de las pruebas reservadas:**
- `crates/gguf2som/src/lib.rs:2054:13`: error debe contener 'dimensiones': meta llama.embedding_length ausente

**Pruebas reservadas que fallan:** tests::tensor_demasiadas_dimensiones


```diff
--- a/crates/gguf2som/src/lib.rs
+++ b/crates/gguf2som/src/lib.rs
@@ -1029,7 +1029,11 @@
     let mut tensors = BTreeMap::new();
     for _ in 0..tensor_count {
         let name = read_string(file)?;
-        let n_dims = read_u32(file)? as usize;
+        let n_dims = read_u32(file)?;
+        if n_dims > 8 {
+            return Err(format!("{name}: tensor con {n_dims} dimensiones excede el límite de 8"));
+        }
+        let n_dims = n_dims as usize;
         let mut shape = Vec::with_capacity(n_dims);
         for _ in 0..n_dims {
             shape.push(read_u64(file)?);
@@ -1228,7 +1232,12 @@
 }
 
 fn read_string<R: Read + Seek>(file: &mut R) -> Result<String, String> {
-    let len = read_u64(file)? as usize;
+    let len = read_u64(file)?;
+    let max_len: usize = 16 * 1024 * 1024; // 16 MiB
+    if len > max_len as u64 {
+        return Err(format!("cadena declarada de {len} bytes excede el límite de {max_len}"));
+    }
+    let len = len as usize;
     let mut buf = vec![0u8; len];
     file.read_exact(&mut buf)?;
     String::from_utf8(buf).map_err(|e| e.to_string())
@@ -1989,4 +1998,115 @@
         assert!(tok.bos().is_none());
         assert_eq!(tok.encode_trozo("hola mundo", false, true), vec![2, 3]);
     }
+
+        // ── pruebas R35 ──────────────────────────────────────────────────
+
+        /// Una cadena declarada de más de 16 MiB debe devolver Err con "cadena".
+        #[test]
+        fn cadena_demasiado_larga() {
+            let dir = std::env::temp_dir().join("convert-gguf-cadena-grande");
+            let _ = fs::remove_dir_all(&dir);
+            fs::create_dir_all(&dir).unwrap();
+            let mut g: Vec<u8> = Vec::new();
+            g.extend_from_slice(b"GGUF");
+            g.extend_from_slice(&3u32.to_le_bytes());
+            g.extend_from_slice(&0u64.to_le_bytes());
+            g.extend_from_slice(&1u64.to_le_bytes());
+            gguf_string(&mut g, "test");
+            g.extend_from_slice(&8u32.to_le_bytes());
+            let big_len: u64 = 20 * 1024 * 1024;
+            g.extend_from_slice(&big_len.to_le_bytes());
+            let gguf_path = dir.join("big.gguf");
+            fs::File::create(&gguf_path).unwrap().write_all(&g).unwrap();
+            let out = dir.join("out");
+            let err = convert_path(gguf_path.to_str().unwrap(), &out, Some("test")).unwrap_err();
+            assert!(err.contains("cadena"), "error debe contener 'cadena': {err}");
+        }
+
+        /// Un tensor con más de 8 dimensiones debe devolver Err con "dimensiones" y el nombre.
+        #[test]
+        fn tensor_demasiadas_dimensiones() {
+            let dir = std::env::temp_dir().join("convert-gguf-many-dims");
+            let _ = fs::remove_dir_all(&dir);
+            fs::create_dir_all(&dir).unwrap();
+            let mut g: Vec<u8> = Vec::new();
+            g.extend_from_slice(b"GGUF");
+            g.extend_from_slice(&3u32.to_le_bytes());
+            g.extend_from_slice(&1u64.to_le_bytes());
+            g.extend_from_slice(&0u64.to_le_bytes());
+            gguf_string(&mut g, "general.architecture");
+            g.extend_from_slice(&8u32.to_le_bytes());
+            gguf_string(&mut g, "llama");
+            gguf_string(&mut g, "bad_tensor");
+            g.extend_from_slice(&9u32.to_le_bytes());
+            for _ in 0..9 {
+                g.extend_from_slice(&1u64.to_le_bytes());
+            }
+            g.extend_from_slice(&GGML_F32.to_le_bytes());
+            g.extend_from_slice(&0u64.to_le_bytes());
+            while g.len() % 32 != 0 {
+                g.push(0);
+            }
+            let gguf_path = dir.join("many_dims.gguf");
+            fs::File::create(&gguf_path).unwrap().write_all(&g).unwrap();
+            let out = dir.join("out");
+            let err = convert_path(gguf_path.to_str().unwrap(), &out, Some("test")).unwrap_err();
+            assert!(err.contains("dimensiones"), "error debe contener 'dimensiones': {err}");
+            assert!(err.contains("bad_tensor"), "error debe contener el nombre: {err}");
+        }
+
… (diff recortado)
```


## R38 — error (991 s, 40 herramientas)

**Causa (orientativa):** (NO TERMINÓ; estado que dejó en disco) CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple

**Ediciones:** {'aplicada': 9, 'otro error': 2}; pruebas propias escritas: 4

**Panics de las pruebas reservadas:**
- `crates/soso-web-core/tests/aceptacion_R38.rs:53:51`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:90:56`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:42:51`: called `Result::unwrap()` on an `Err` value: ()

**Pruebas reservadas que fallan:** deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple


```diff
--- a/crates/soso-web-core/src/decode.rs
+++ b/crates/soso-web-core/src/decode.rs
@@ -17,6 +17,7 @@
     }
 }
 
+
 /// Convierte bytes HTML a `String` (UTF-8 con fallback Latin-1).
 pub fn bytes_to_text(body: &[u8], charset: Option<&str>) -> String {
     let cs = charset.unwrap_or("utf-8").trim().to_ascii_lowercase();
```
