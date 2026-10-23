# Análisis de ejecucion-57-denso-plazo-largo

1 aceptadas de 2. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R37 | rechazada | CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta |


## R37 — rechazada (607 s, 20 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan la_query_se_conserva_aunque_no_haya_ruta

**Ediciones:** {'aplicada': 4}; pruebas propias escritas: 1

**Panics de las pruebas reservadas:**
- `crates/soso-http/tests/aceptacion_R37.rs:20:71`: called `Result::unwrap()` on an `Err` value: Parse

**Pruebas reservadas que fallan:** la_query_se_conserva_aunque_no_haya_ruta


```diff
--- a/crates/soso-http/src/lib.rs
+++ b/crates/soso-http/src/lib.rs
@@ -598,18 +598,54 @@
     Ok((status, body))
 }
 
-fn parse_url(url: &str) -> Result<(&str, &str, u16, String), HttpError> {
+pub fn parse_url(url: &str) -> Result<(&str, &str, u16, String), HttpError> {
+    // Solo se admite https://
     let rest = url.strip_prefix("https://").ok_or(HttpError::Parse)?;
-    let (host_port, path) = match rest.split_once('/') {
-        Some((h, p)) => (h, format!("/{p}")),
-        None => (rest, String::from("/")),
+
+    // Descartar fragmento (desde el primer #)
+    let rest = rest.split_once('#').map(|(s, _)| s).unwrap_or(rest);
+
+    // Host (con puerto opcional) termina en el primer / o ?, o al acabar la cadena.
+    // split_once no conserva el separador, así que buscamos manualmente.
+    let (host_port, path) = if let Some(slash_pos) = rest.find('/') {
+        let h = &rest[..slash_pos];
+        let tail = &rest[slash_pos..];
+        (h, tail.to_string())
+    } else if let Some(q_pos) = rest.find('?') {
+        let h = &rest[..q_pos];
+        // La ruta es / seguida del query (?q=1)
+        let tail = &rest[q_pos..];
+        (h, format!("/{}", tail))
+    } else {
+        (rest, String::from("/"))
     };
-    if let Some((host, port)) = host_port.rsplit_once(':') {
-        let port: u16 = port.parse().map_err(|_| HttpError::Parse)?;
-        Ok(("https", host, port, path))
+
+    // Separar host y puerto
+    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
+        // Puerto presente
+        if p.is_empty() {
+            return Err(HttpError::Parse); // puerto vacío
+        }
+        let port_val: u16 = p.parse().map_err(|_| HttpError::Parse)?;
+        if port_val == 0 {
+            return Err(HttpError::Parse); // puerto 0 no válido
+        }
+        (h, port_val)
     } else {
-        Ok(("https", host_port, 443, path))
+        (host_port, 443)
+    };
+
+    // Host vacío no permitido
+    if host.is_empty() {
+        return Err(HttpError::Parse);
+    }
+
+    // Presencia de @ en la parte del host da error
+    if host.contains('@') {
+        return Err(HttpError::Parse);
     }
+
+    Ok(("https", host, port, path))
 }
 
 fn parse_response_head(head: &[u8]) -> Result<(u16, Vec<(String, String)>), HttpError> {
@@ -844,3 +880,118 @@
         set_wall_clock(test_wall_clock);
     }
 }
+#[cfg(test)]
+mod pruebas_agente {
+    use super::*;
+
+    // Solo se admite https://
+    #[test]
+    fn esquema_no_https() {
+        assert_eq!(parse_url("http://h.org/x"), Err(HttpError::Parse));
+        assert_eq!(parse_url("ftp://h.org/x"), Err(HttpError::Parse));
+        assert_eq!(parse_url("h.org/x"), Err(HttpError::Parse));
+    }
+
+    // Fragmento se descarta
+    #[test]
+    fn fragmento_se_descarta() {
+        let (_, host, port, path) = parse_url("https://h.org/x#frag").unwrap();
+        assert_eq!(host, "h.org");
+        assert_eq!(port, 443);
+        assert_eq!(path, "/x");
+    }
+
… (diff recortado)
```
