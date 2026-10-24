# Análisis de ejecucion-60-banco5-b-denso

4 aceptadas de 7. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R42 | rechazada | CASO LÍMITE NO CUBIERTO: fallan indices_extremos_no_desbordan, relayout_rechaza_particiones_invertidas_sin_toc |
| R49 | rechazada | CASO LÍMITE NO CUBIERTO: fallan prefijo_incompleto_seguido_de_un_no_continuacion |
| R50 | rechazada | CASO LÍMITE NO CUBIERTO: fallan desbordes_de_u64_son_error_y_no_panico |


## R42 — rechazada (1696 s, 38 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan indices_extremos_no_desbordan, relayout_rechaza_particiones_invertidas_sin_tocar_nada

**Ediciones:** {'aplicada': 6, 'otro error': 1}; pruebas propias escritas: 4

**Panics de las pruebas reservadas:**
- `crates/gptdisk/src/lib.rs:247:22`: attempt to add with overflow
- `crates/gptdisk/tests/aceptacion_R42.rs:177:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** indices_extremos_no_desbordan, relayout_rechaza_particiones_invertidas_sin_tocar_nada


```diff
--- a/crates/gptdisk/src/lib.rs
+++ b/crates/gptdisk/src/lib.rs
@@ -159,9 +159,34 @@
         if !(MIN_HEADER_SIZE..=SECTOR as u32).contains(&header_size) {
             return Err(Error::BadHeader);
         }
+        // CRC32: calcular sobre los primeros header_size bytes con el campo CRC a cero
+        let stored_crc = rd32(sec, 16);
+        let mut header_bytes = [0u8; 960];
+        header_bytes[..header_size as usize].copy_from_slice(&sec[..header_size as usize]);
+        header_bytes[16..20].copy_from_slice(&0u32.to_le_bytes());
+        let computed_crc = crc32_ieee(&header_bytes[..header_size as usize]);
+        if stored_crc != computed_crc {
+            return Err(Error::BadHeader);
+        }
         let num_entries = rd32(sec, 80);
         let entry_size = rd32(sec, 84);
-        if num_entries == 0 || num_entries > 512 || entry_size < 128 || entry_size % 8 != 0 {
+        // entry_size debe ser potencia de 2 entre 128 y 4096
+        if num_entries == 0 || num_entries > 512 {
+            return Err(Error::BadHeader);
+        }
+        if !(128..=4096).contains(&entry_size) || entry_size & (entry_size - 1) != 0 {
+            return Err(Error::BadHeader);
+        }
+        let my_lba = rd64(sec, 24);
+        let entries_lba = rd64(sec, 72);
+        let first_usable = rd64(sec, 40);
+        let last_usable = rd64(sec, 48);
+        // my_lba == 0 o entries_lba == 0 dan error
+        if my_lba == 0 || entries_lba == 0 {
+            return Err(Error::BadHeader);
+        }
+        // first_usable > last_usable da error
+        if first_usable > last_usable {
             return Err(Error::BadHeader);
         }
         let mut disk_guid = [0u8; 16];
@@ -169,12 +194,12 @@
         Ok(Header {
             revision: rd32(sec, 8),
             header_size,
-            my_lba: rd64(sec, 24),
+            my_lba,
             alternate_lba: rd64(sec, 32),
-            first_usable: rd64(sec, 40),
-            last_usable: rd64(sec, 48),
+            first_usable,
+            last_usable,
             disk_guid: Guid(disk_guid),
-            entries_lba: rd64(sec, 72),
+            entries_lba,
             num_entries,
             entry_size,
         })
@@ -223,11 +248,15 @@
 }
 
 pub fn set_entry_first_lba(e: &mut [u8], lba: u64) {
-    wr64(e, 32, lba);
+    if e.len() >= 40 {
+        wr64(e, 32, lba);
+    }
 }
 
 pub fn set_entry_last_lba(e: &mut [u8], lba: u64) {
-    wr64(e, 40, lba);
+    if e.len() >= 48 {
+        wr64(e, 40, lba);
+    }
 }
 
 /// Entrada mutable `i` del array GPT.
@@ -238,26 +267,41 @@
 }
 
 pub fn entry_used(e: &[u8]) -> bool {
+    if e.len() < 16 {
+        return false;
+    }
     e[0..16].iter().any(|&b| b != 0)
 }
 
 pub fn entry_type(e: &[u8]) -> Guid {
+    if e.len() < 16 {
+        return Guid::ZERO;
+    }
     let mut g = [0u8; 16];
     g.copy_from_slice(&e[0..16]);
     Guid(g)
… (diff recortado)
```


## R49 — rechazada (1454 s, 40 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan prefijo_incompleto_seguido_de_un_no_continuacion

**Ediciones:** {'aplicada': 7}; pruebas propias escritas: 0

**Panics de las pruebas reservadas:**
- `crates/soso-llm-core/tests/aceptacion_R49.rs:26:5`: assertion `left == right` failed

**Pruebas reservadas que fallan:** prefijo_incompleto_seguido_de_un_no_continuacion


```diff
--- a/crates/soso-llm-core/src/tokenizer.rs
+++ b/crates/soso-llm-core/src/tokenizer.rs
@@ -89,7 +89,10 @@
     pub fn decode(&self, tokens: &[u32]) -> String {
         match self {
             Tokenizer::ByteLevel => {
-                let bytes: Vec<u8> = tokens.iter().map(|&t| t as u8).collect();
+                let bytes: Vec<u8> = tokens
+                    .iter()
+                    .filter_map(|&t| (t < 256).then_some(t as u8))
+                    .collect();
                 String::from_utf8_lossy(&bytes).into_owned()
             }
             Tokenizer::Vocab(v) => v.decode(tokens),
@@ -100,7 +103,11 @@
     /// emiten nada.
     pub fn token_bytes(&self, t: u32, out: &mut Vec<u8>) {
         match self {
-            Tokenizer::ByteLevel => out.push(t as u8),
+            Tokenizer::ByteLevel => {
+                if t < 256 {
+                    out.push(t as u8);
+                }
+            }
             Tokenizer::Vocab(v) => v.token_bytes(t, out),
         }
     }
@@ -250,7 +257,12 @@
             self.token_bytes(t, &mut bytes);
         }
         let text = String::from_utf8_lossy(&bytes).into_owned();
-        String::from(text.strip_prefix(' ').unwrap_or(&text))
+        // Solo quitar espacio inicial en SentencePiece (▁), no en GPT-2 (Ġ)
+        if self.space_mark == SPACE_SP {
+            String::from(text.strip_prefix(' ').unwrap_or(&text))
+        } else {
+            text
+        }
     }
 }
 
@@ -278,18 +290,62 @@
             }
             Err(e) => {
                 let n = e.valid_up_to();
-                let out = String::from_utf8_lossy(&self.pending[..n]).into_owned();
-                self.pending.drain(..n);
-                // secuencia inválida (no solo incompleta): descartarla para
-                // no acumular basura indefinidamente
-                if self.pending.len() > 4 {
-                    self.pending.clear();
-                }
-                out
+                let rest = &self.pending[n..];
+                // rest contiene bytes inválidos + posible prefijo incompleto.
+                // Buscamos el primer byte que podría ser inicio de un
+                // carácter UTF-8 válido (eso marca dónde empieza el
+                // prefijo incompleto).
+                let incomplete_start = self.find_incomplete_start(rest);
+                let invalid_count = incomplete_start;
+                let emitted: String =
+                    String::from_utf8_lossy(&rest[..invalid_count]).into_owned();
+                // Retener solo el prefijo incompleto
+                self.pending.drain(..n + invalid_count);
+                emitted
             }
         }
     }
 
+    /// Busca dónde empieza el prefijo incompleto en `data`.
+    /// Devuelve el índice del primer byte que podría ser inicio de un
+    /// carácter UTF-8 válido. Si ninguno, devuelve data.len() (todo es inválido).
+    fn find_incomplete_start(&self, data: &[u8]) -> usize {
+        for i in 0..data.len() {
+            let b = data[i];
+            if is_utf8_start_byte(b) {
+                return i;
+            }
+        }
+        data.len()
+    }
+
+    /// Longitud del prefijo UTF-8 incompleto al final de `data`.
+    fn valid_prefix_len(&self, data: &[u8]) -> usize {
+        // Buscar el mayor `keep` tal que data[..keep] es UTF-8 válido.
+        // Eso significa que data[keep..] es el prefijo incompleto.
+        for keep in (0..data.len()).rev() {
+            if core::str::from_utf8(&data[..keep]).is_ok() {
+                return data.len() - keep;
+            }
… (diff recortado)
```


## R50 — rechazada (2265 s, 36 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan desbordes_de_u64_son_error_y_no_panico

**Ediciones:** {'aplicada': 7}; pruebas propias escritas: 3

**Panics de las pruebas reservadas:**
- `crates/soso-resize-core/src/lib.rs:144:22`: attempt to add with overflow

**Pruebas reservadas que fallan:** desbordes_de_u64_son_error_y_no_panico


```diff
--- a/crates/soso-resize-core/src/lib.rs
+++ b/crates/soso-resize-core/src/lib.rs
@@ -213,28 +213,56 @@
     disk: &impl Disk512,
     delta_sectors: u64,
 ) -> Result<(), Error> {
+    // 1. delta_sectors es 0
+    if delta_sectors == 0 {
+        return Err(Error::InvalidGeometry);
+    }
+    // 2. delta_sectors no es múltiplo de 8
+    if delta_sectors % 8 != 0 {
+        return Err(Error::InvalidGeometry);
+    }
+
     let p2 = read_part(disk, GPT_ROOT)?;
     let p3 = read_part(disk, GPT_MODELS)?;
     let p4 = read_part(disk, GPT_INSTALL)?;
+
+    // 3. Alguna partición tiene last < first
+    if p2.last < p2.first || p3.last < p3.first || p4.last < p4.first {
+        return Err(Error::InvalidGeometry);
+    }
+
+    // 4. Particiones no están en orden estricto sin solapes
+    if p2.last >= p3.first || p3.last >= p4.first {
+        return Err(Error::InvalidGeometry);
+    }
+
+    // 5. Comprobar desbordamiento de sumas del relayout
+    let new_p2_last = p2.last.checked_add(delta_sectors).ok_or(Error::InvalidGeometry)?;
+    let new_p3_first = p3.first.checked_add(delta_sectors).ok_or(Error::InvalidGeometry)?;
+    let new_p4_first = p4.first.checked_add(delta_sectors).ok_or(Error::InvalidGeometry)?;
+    let new_p4_last = p4.last.checked_add(delta_sectors).ok_or(Error::InvalidGeometry)?;
+
+    let _new_p3_last = p3.last;
+
+    // 6. delta >= sectores de p3
     if p3.sectors <= delta_sectors {
         return Err(Error::InvalidGeometry);
     }
-    let new_p2_last = p2.last + delta_sectors;
-    let new_p3_first = p3.first + delta_sectors;
-    let new_p3_last = p3.last;
-    let new_p4_first = p4.first + delta_sectors;
-    let new_p4_last = p4.last + delta_sectors;
 
     let mut hdr_sec = [0u8; SECTOR];
     disk.read_sector(1, &mut hdr_sec)?;
     let mut hdr = Header::parse(&hdr_sec).map_err(|_| Error::InvalidGeometry)?;
     let bytes = hdr.entries_bytes();
-    let padded = bytes.div_ceil(SECTOR) * SECTOR;
+    let padded = bytes.div_ceil(SECTOR).checked_mul(SECTOR).ok_or(Error::InvalidGeometry)?;
     let mut entries = alloc::vec![0u8; padded];
     for i in 0..hdr.entries_sectors() {
         let mut sec = [0u8; SECTOR];
-        disk.read_sector(hdr.entries_lba + i, &mut sec)?;
-        entries[i as usize * SECTOR..(i as usize + 1) * SECTOR].copy_from_slice(&sec);
+        let read_lba = hdr.entries_lba.checked_add(i).ok_or(Error::InvalidGeometry)?;
+        disk.read_sector(read_lba, &mut sec)?;
+        let start = (i as usize).checked_mul(SECTOR).ok_or(Error::InvalidGeometry)?;
+        let end = start.checked_add(SECTOR).ok_or(Error::InvalidGeometry)?;
+        let slice = entries.get_mut(start..end).ok_or(Error::InvalidGeometry)?;
+        slice.copy_from_slice(&sec);
     }
     entries.truncate(bytes);
 
@@ -247,7 +275,7 @@
     }
     if let Some(e) = entry_mut(&mut entries, &hdr, GPT_MODELS) {
         set_entry_first_lba(e, new_p3_first);
-        set_entry_last_lba(e, new_p3_last);
+        set_entry_last_lba(e, _new_p3_last);
     }
 
     let disk_sectors = disk_sectors(disk)?;
@@ -445,3 +473,183 @@
         self.reloads
     }
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+    // Mock disk para pruebas que solo necesita read_part y leer el header
+    struct MockDisk {
+        p2: PartGeom,
+        p3: PartGeom,
+        p4: PartGeom,
+        disk_sectors: u64,
… (diff recortado)
```
