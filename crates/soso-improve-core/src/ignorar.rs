//! Un subconjunto de `.gitignore`, para saber qué no es fuente sin llamar a git.
//!
//! La primera versión de la captura preguntaba a git qué archivos hay. Dentro
//! de soso no hay git, así que el árbol se recorre a pelo — y entonces hay que
//! saber qué tirar: `rootfs/var/` son 137 MB de artefactos y
//! `lxdde/reference/` son 6600 archivos de fuentes ajenas. Esa información ya
//! está escrita en el `.gitignore` del repositorio; lo que falta es leerla.
//!
//! **Se implementa un subconjunto, y se dice cuál:**
//!
//! - `nombre` — coincide en cualquier nivel
//! - `/nombre` — anclado a la raíz del `.gitignore`
//! - `nombre/` — solo directorios
//! - `dir/sub` — patrón con `/` interior: anclado
//! - `*` dentro de un segmento (`*.img`, `test-*-serial.log`)
//! - `**/` al principio, que es como no anclarlo
//!
//! **No se implementa**: negaciones (`!patrón`), clases de caracteres (`[a-z]`)
//! ni `**` en medio. Si aparece una negación, [`Reglas::parsear`] devuelve
//! error en vez de ignorarla en silencio: leer mal un `.gitignore` significa
//! meter basura en una base o dejarse fuera un archivo real.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::{Error, Resultado};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Regla {
    /// Segmentos del patrón; `None` en un segmento significa `*` puro.
    patron: String,
    anclada: bool,
    solo_directorios: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reglas {
    reglas: Vec<Regla>,
}

/// ¿Casa un segmento con un patrón con comodines `*`?
fn casa_glob(patron: &str, texto: &str) -> bool {
    // Comparación por trozos separados por `*`: el primero tiene que estar al
    // principio, el último al final y los de en medio en orden.
    let trozos: Vec<&str> = patron.split('*').collect();
    if trozos.len() == 1 {
        return patron == texto;
    }
    let mut resto = texto;
    if !trozos[0].is_empty() {
        match resto.strip_prefix(trozos[0]) {
            Some(r) => resto = r,
            None => return false,
        }
    }
    let ultimo = trozos.len() - 1;
    for (i, trozo) in trozos.iter().enumerate().skip(1) {
        if trozo.is_empty() {
            continue;
        }
        if i == ultimo {
            return resto.len() >= trozo.len() && resto.ends_with(trozo);
        }
        match resto.find(trozo) {
            Some(pos) => resto = &resto[pos + trozo.len()..],
            None => return false,
        }
    }
    true
}

/// ¿Casan los segmentos del patrón con el principio de estos segmentos?
fn casa_segmentos(patron: &str, ruta: &[&str]) -> bool {
    let p: Vec<&str> = patron.split('/').filter(|s| !s.is_empty()).collect();
    if p.len() > ruta.len() {
        return false;
    }
    p.iter().zip(ruta.iter()).all(|(a, b)| casa_glob(a, b))
}

impl Reglas {
    pub fn parsear(texto: &str) -> Resultado<Reglas> {
        let mut reglas = Vec::new();
        for linea in texto.lines() {
            let linea = linea.trim();
            if linea.is_empty() || linea.starts_with('#') {
                continue;
            }
            if let Some(resto) = linea.strip_prefix('!') {
                return Err(Error::formato(format!(
                    "negación no soportada en .gitignore: !{resto}"
                )));
            }
            if linea.contains('[') {
                return Err(Error::formato(format!(
                    "clase de caracteres no soportada en .gitignore: {linea}"
                )));
            }
            let solo_directorios = linea.ends_with('/');
            let mut patron = linea.trim_end_matches('/').to_string();
            let mut anclada = false;
            if let Some(resto) = patron.strip_prefix("**/") {
                patron = resto.to_string();
            } else if let Some(resto) = patron.strip_prefix('/') {
                patron = resto.to_string();
                anclada = true;
            } else if patron.contains('/') {
                anclada = true;
            }
            if patron.contains("**") {
                return Err(Error::formato(format!(
                    "'**' en medio no soportado en .gitignore: {linea}"
                )));
            }
            reglas.push(Regla {
                patron,
                anclada,
                solo_directorios,
            });
        }
        Ok(Reglas { reglas })
    }

    pub fn vacias(&self) -> bool {
        self.reglas.is_empty()
    }

    /// ¿Se ignora esta ruta relativa a la raíz de las reglas?
    ///
    /// Se prueba cada prefijo de la ruta, no solo la ruta entera: en git, un
    /// directorio ignorado se lleva por delante todo lo que hay debajo, y un
    /// ancestro siempre es un directorio (eso resuelve las reglas con `/`
    /// final sin casos especiales).
    pub fn ignora(&self, ruta: &str, es_directorio: bool) -> bool {
        let segmentos: Vec<&str> = ruta.split('/').filter(|s| !s.is_empty()).collect();
        for fin in 1..=segmentos.len() {
            let prefijo = &segmentos[..fin];
            let es_dir = fin < segmentos.len() || es_directorio;
            for regla in &self.reglas {
                if regla.solo_directorios && !es_dir {
                    continue;
                }
                if regla.anclada {
                    if casa_segmentos(&regla.patron, prefijo) {
                        return true;
                    }
                } else {
                    // Sin anclar casa a cualquier profundidad: se prueban todos
                    // los sufijos del prefijo.
                    for inicio in 0..prefijo.len() {
                        if casa_segmentos(&regla.patron, &prefijo[inicio..]) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reglas() -> Reglas {
        Reglas::parsear(
            "# comentario\n\
             /target/\n\
             **/target/\n\
             *.img\n\
             /rootfs/var/\n\
             lxdde/reference/\n\
             board.txt\n\
             /test-*-serial.log\n\
             __pycache__/\n",
        )
        .unwrap()
    }

    #[test]
    fn directorios_anclados() {
        let r = reglas();
        assert!(r.ignora("target", true));
        assert!(r.ignora("rootfs/var", true));
        assert!(r.ignora("rootfs/var/forja-out/rootfs.pack", false));
        assert!(r.ignora("lxdde/reference/algo/x.c", false));
        assert!(!r.ignora("rootfs/bin", true));
        assert!(!r.ignora("crates/soso-improve-core/src/lib.rs", false));
    }

    #[test]
    fn comodines() {
        let r = reglas();
        assert!(r.ignora("disco.img", false));
        assert!(r.ignora("user/algo.img", false));
        assert!(r.ignora("test-0-serial.log", false));
        assert!(!r.ignora("user/test-0-serial.log", false), "está anclado a la raíz");
        assert!(!r.ignora("imagen.imgx", false));
    }

    #[test]
    fn nombres_sin_anclar() {
        let r = reglas();
        assert!(r.ignora("board.txt", false));
        assert!(r.ignora("sub/dir/board.txt", false));
        assert!(r.ignora("a/__pycache__", true));
        assert!(r.ignora("a/__pycache__/x.pyc", false));
    }

    #[test]
    fn target_anidado() {
        let r = reglas();
        assert!(r.ignora("user/target", true));
        assert!(r.ignora("kernel/target/debug/x", false));
    }

    #[test]
    fn lo_no_soportado_se_dice() {
        assert!(Reglas::parsear("!importante.txt\n").is_err());
        assert!(Reglas::parsear("a[0-9].txt\n").is_err());
        assert!(Reglas::parsear("a/**/b\n").is_err());
    }
}
