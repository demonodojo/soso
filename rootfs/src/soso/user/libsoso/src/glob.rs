//! Comodines de nombre de fichero, en un solo sitio.
//!
//! Vive aquí porque lo usan **dos** programas —`grep --include` (N-009) y la
//! expansión de `sosh` (N-010)— y dos copias acaban siendo dos semánticas:
//! alguien añade `[a-z]` en una y la otra sigue sin entenderlo, y lo que estas
//! dos fichas vinieron a arreglar fue precisamente la semántica que nadie
//! había declarado.
//!
//! **Sólo `*` y `?`.** Ni clases `[…]` ni `**` ni llaves. Añadirlas sin un
//! consumidor que las pida sería inventar semántica.
//!
//! El emparejado es sobre el **nombre**, no sobre la ruta: quien quiera
//! comparar rutas que parta por `/` antes. Así `*.rs` casa `a.rs` viniendo de
//! `src/a.rs`, que es lo que espera quien escribe `--include=*.rs`.

/// ¿Casa `nombre` con `patron`?
pub fn casa(nombre: &str, patron: &str) -> bool {
    let n: &[char] = &nombre.chars().collect::<alloc::vec::Vec<char>>();
    let p: &[char] = &patron.chars().collect::<alloc::vec::Vec<char>>();
    // Recorrido con retroceso sólo en `*`: sin recursión, para que un patrón
    // con muchos comodines no se coma la pila.
    let (mut i, mut j) = (0usize, 0usize);
    let (mut marca_n, mut marca_p) = (usize::MAX, 0usize);
    while i < n.len() {
        if j < p.len() && (p[j] == '?' || p[j] == n[i]) {
            i += 1;
            j += 1;
        } else if j < p.len() && p[j] == '*' {
            marca_n = i;
            marca_p = j;
            j += 1;
        } else if marca_n != usize::MAX {
            marca_n += 1;
            i = marca_n;
            j = marca_p + 1;
        } else {
            return false;
        }
    }
    while j < p.len() && p[j] == '*' {
        j += 1;
    }
    j == p.len()
}

/// ¿Lleva algún comodín?
pub fn tiene_comodin(s: &str) -> bool {
    s.contains('*') || s.contains('?')
}

// **Sin tests unitarios aquí, y a propósito.** `libsoso` es `no_std` para un
// target propio: `cargo test -p libsoso` no compila para el host, así que unos
// `#[cfg(test)]` en este fichero **nunca se ejecutarían** y sólo darían la
// impresión de que hay cobertura. Se comprueba donde corre — en el guest, por
// los dos consumidores: los casos de glob de la sonda `busqueda` (N-009) y el
// paso de suite de `sosh` (N-010), incluidos los de retroceso del `*`.
//
// Si algún día hace falta probarlo en el host, el sitio es un crate en
// `crates/` con feature `std`, que es el patrón que el repo ya usa.
