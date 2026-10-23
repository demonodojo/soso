# Análisis de ejecucion-56-b-denso

3 aceptadas de 5. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R33 | error | (NO TERMINÓ; estado que dejó en disco) NO COMPILA con las pruebas reservadas: mismatched types; could not comp |
| R37 | rechazada | CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta |


## R33 — error (3601 s, 24 herramientas)

**Causa (orientativa):** (NO TERMINÓ; estado que dejó en disco) NO COMPILA con las pruebas reservadas: mismatched types; could not compile `soso-audio` (lib) due to 1 previous error

**Ediciones:** {'rechazo llaves': 2, 'aplicada': 4, 'otro error': 1}; pruebas propias escritas: 5


```diff
--- a/crates/soso-audio/src/wav.rs
+++ b/crates/soso-audio/src/wav.rs
@@ -12,45 +12,65 @@
 
 /// Lee PCM16 mono. Acepta cualquier sample rate (el caller puede remuestrear).
 pub fn parse_pcm16_mono(data: &[u8]) -> Result<(u32, Vec<i16>), WavError> {
-    if data.len() < 44 {
+    if data.len() < 4 {
         return Err(WavError::TooShort);
     }
-    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
+    if &data[0..4] != b"RIFF" {
+        return Err(WavError::BadRiff);
+    }
+    if data.len() < 12 || &data[8..12] != b"WAVE" {
         return Err(WavError::BadRiff);
     }
     let mut pos = 12usize;
     let mut sample_rate = 0u32;
     let mut bits = 0u16;
     let mut channels = 0u16;
-    let mut pcm = Vec::new();
+    let mut audio_format = 0u16;
+    // Recopilar data chunks (puede haber antes de fmt)
+    let mut data_chunks: Vec<(usize, usize)> = Vec::new();
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
-            if audio_format != 1 {
-                return Err(WavError::Unsupported);
-            }
         } else if id == b"data" {
-            if bits != 16 || channels != 1 {
-                return Err(WavError::Unsupported);
-            }
-            let bytes = &data[pos..pos + sz];
-            pcm.reserve(bytes.len() / 2);
+            data_chunks.push((pos, sz));
+        }
+        pos += sz + (sz & 1);
+    }
+    if sample_rate == 0 {
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
+    let mut pcm = Vec::new();
+    for &(start, sz) in &data_chunks {
+        let end = data.len().min(start + sz);
+        let bytes = &data[start..end];
+        if channels == 1 {
             for chunk in bytes.chunks_exact(2) {
                 pcm.push(i16::from_le_bytes([chunk[0], chunk[1]]));
             }
+        } else {
+            for chunk in bytes.chunks_exact(4) {
+                let left = i16::from_le_bytes([chunk[0], chunk[1]]);
+                let right = i16::from_le_bytes([chunk[2], chunk[3]]);
+                pcm.push((left as i32 + right as i32) / 2);
+            }
         }
-        pos += sz + (sz & 1);
     }
-    if pcm.is_empty() || sample_rate == 0 {
+    if pcm.is_empty() {
         return Err(WavError::BadFmt);
     }
     Ok((sample_rate, pcm))
@@ -75,3 +95,284 @@
     }
     out
 }
+
… (diff recortado)
```


## R37 — rechazada (825 s, 26 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta

**Ediciones:** {'aplicada': 5, 'rechazo sintaxis': 1, 'otro error': 1}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/soso-http/tests/aceptacion_R37.rs:20:71`: called `Result::unwrap()` on an `Err` value: Parse

**Pruebas reservadas que fallan:** la_query_se_conserva_aunque_no_haya_ruta


```diff
--- a/crates/soso-http/src/lib.rs
+++ b/crates/soso-http/src/lib.rs
@@ -598,18 +598,60 @@
     Ok((status, body))
 }
 
-fn parse_url(url: &str) -> Result<(&str, &str, u16, String), HttpError> {
+pub fn parse_url(url: &str) -> Result<(&str, &str, u16, String), HttpError> {
+    // Solo se admite el esquema https://
     let rest = url.strip_prefix("https://").ok_or(HttpError::Parse)?;
-    let (host_port, path) = match rest.split_once('/') {
-        Some((h, p)) => (h, format!("/{p}")),
-        None => (rest, String::from("/")),
+
+    // Descartar fragmento (desde el primer #)
+    let rest = rest.split_once('#').map(|(s, _)| s).unwrap_or(rest);
+
+    // El host (con puerto opcional) termina en el primer / o ?
+    let (host_port, path_part): (&str, String) = match rest.split_once('/') {
+        Some((h, p)) => (h, p.to_string()),
+        None => {
+            // No hay /; buscar si hay ?
+            match rest.split_once('?') {
+                Some((h, q)) => (h, format!("?{}", q)),
+                None => (rest, String::new()),
+            }
+        }
+    };
+
+    // Construir la ruta
+    let path = if path_part.is_empty() {
+        String::from("/")
+    } else {
+        format!("/{}", path_part)
     };
-    if let Some((host, port)) = host_port.rsplit_once(':') {
-        let port: u16 = port.parse().map_err(|_| HttpError::Parse)?;
-        Ok(("https", host, port, path))
+
+    // Separar host y puerto
+    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
+        if h.is_empty() {
+            return Err(HttpError::Parse);
+        }
+        if p.is_empty() {
+            return Err(HttpError::Parse);
+        }
+        let port_num: u16 = p.parse().map_err(|_| HttpError::Parse)?;
+        if port_num == 0 {
+            return Err(HttpError::Parse);
+        }
+        (h, port_num)
     } else {
-        Ok(("https", host_port, 443, path))
+        (host_port, 443)
+    };
+
+    // Host no puede estar vacío
+    if host.is_empty() {
+        return Err(HttpError::Parse);
+    }
+
+    // Rechazar si hay @ en la parte del host (credenciales)
+    if host.contains('@') {
+        return Err(HttpError::Parse);
     }
+
+    Ok(("https", host, port, path))
 }
 
 fn parse_response_head(head: &[u8]) -> Result<(u16, Vec<(String, String)>), HttpError> {
@@ -844,3 +886,140 @@
         set_wall_clock(test_wall_clock);
     }
 }
+
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+    // Solo se admite https://
+    #[test]
+    fn solo_https() {
+        assert_eq!(parse_url("http://h.org/x").unwrap_err(), HttpError::Parse);
+        assert_eq!(parse_url("ftp://h.org/x").unwrap_err(), HttpError::Parse);
+        assert_eq!(parse_url("h.org/x").unwrap_err(), HttpError::Parse);
+    }
+
+    // Fragmento se descarta
+    #[test]
… (diff recortado)
```
