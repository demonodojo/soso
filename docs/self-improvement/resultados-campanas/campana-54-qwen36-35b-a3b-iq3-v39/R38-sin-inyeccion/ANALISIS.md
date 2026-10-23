# Análisis de ejecucion-54b-R38-sin-inyeccion

0 aceptadas de 1. Las demás, una a una:

| tarea | resultado | causa |
|---|---|---|
| R38 | rechazada | CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple |


## R38 — rechazada (321 s, 40 herramientas)

**Causa (orientativa):** CASO LÍMITE NO CUBIERTO: fallan deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple

**Ediciones:** {'otro error': 2, 'aplicada': 2}; pruebas propias escritas: 4

**Panics de las pruebas reservadas:**
- `crates/soso-web-core/tests/aceptacion_R38.rs:53:51`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:90:56`: called `Result::unwrap()` on an `Err` value: ()
- `crates/soso-web-core/tests/aceptacion_R38.rs:42:51`: called `Result::unwrap()` on an `Err` value: ()

**Pruebas reservadas que fallan:** deflate_zlib_y_crudo, gzip_con_campos_opcionales, gzip_simple


```diff
--- a/crates/soso-web-core/src/decode.rs
+++ b/crates/soso-web-core/src/decode.rs
@@ -91,3 +91,4 @@
         assert_eq!(bytes_to_text(&[0xE9], Some("iso-8859-1")), "é");
     }
 }
+
```
