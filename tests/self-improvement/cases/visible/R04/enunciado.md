# R04 — resolve_url resuelve los segmentos . y ..

## Qué hay hoy

`soso_web_core::html::resolve_url` pega el enlace relativo al directorio de la
base sin resolver los segmentos `.` y `..`:

```rust
resolve_url("https://host/a/b", "../c") == "https://host/a/../c"
```

La URL apunta al sitio correcto cuando la pide un servidor, pero como cadena no
coincide con su forma canónica: el mismo recurso aparece con dos nombres
distintos en los enlaces de la página.

## Qué se pide

Aplicar la eliminación de segmentos de punto de RFC 3986 §5.2.4:

- `../c/d.html` sobre `https://ejemplo.org/a/b/pagina.html` da
  `https://ejemplo.org/a/c/d.html`;
- `./d.html` y `d.html` dan `https://ejemplo.org/a/b/d.html`;
- los `..` que sobran se descartan sin sacar la ruta de la raíz;
- las URL absolutas, las que empiezan por `/`, el ancla `#` y el enlace vacío
  se comportan como hasta ahora.

La prueba `resolve_url_relativa` del propio crate fija el comportamiento
anterior: forma parte del trabajo actualizarla.

## Alcance

Solo `crates/soso-web-core/src/html.rs`.
