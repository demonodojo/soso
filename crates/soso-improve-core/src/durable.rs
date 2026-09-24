//! Referencias durables: publicar sin perder la anterior (T46).
//!
//! El patrón clásico —escribir a temporal, `fsync`, `rename` encima— **no se
//! puede usar en soso**: su `rename` falla si el destino existe
//! (`crates/sosofs/src/write.rs`: `if self.lookup(new_dir, new_name).is_ok() {
//! return Err(FsError::Exists) }`). Y la salida fácil, borrar y renombrar,
//! abre una ventana en la que no existe **ninguna** de las dos referencias: un
//! corte ahí no deja una versión vieja, deja nada.
//!
//! Así que aquí no se sobrescribe nunca. Cada publicación escribe una
//! **generación nueva** con creación exclusiva, y la anterior sigue en su sitio
//! hasta que la nueva está entera en disco. El lector se queda con la
//! generación más alta que sea **íntegra**, y una generación a medias —un corte
//! en mitad de la escritura— no lo es, porque no cuadra su hash.
//!
//! Lo que esto da, y es lo que pide la ficha:
//!
//! - nunca se ve una mezcla: o la referencia vieja o la nueva entera;
//! - la vieja se conserva hasta acreditar la nueva;
//! - dos escritores no se pisan: el segundo choca con la creación exclusiva.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::entorno::Archivos;
use crate::{sha256_hex, unir, Error, Resultado};

/// Lo que hace falta del sistema **además** de leer y escribir: crear sin
/// pisar, y asegurarse de que lo escrito está en disco.
///
/// Va aparte de [`Archivos`] a propósito: un adaptador puede saber leer y
/// escribir y no poder prometer durabilidad, y eso hay que poder decirlo.
pub trait Durable: Archivos {
    /// Crea el fichero **sólo si no existe** y deja el contenido sincronizado.
    /// Si existe, `Error::Uso`: es la señal de que otro escritor ganó.
    fn crear_exclusivo(&mut self, ruta: &str, datos: &[u8]) -> Resultado<()>;

    /// Fuerza a disco lo ya escrito en `ruta`.
    fn sincronizar(&mut self, ruta: &str) -> Resultado<()>;
}

/// Número de generación. Va en el nombre, en hexadecimal de ancho fijo, para
/// que el orden alfabético y el numérico coincidan.
pub type Generacion = u64;

const ANCHO_GEN: usize = 16;

fn nombre_gen(nombre: &str, gen: Generacion) -> String {
    format!("{nombre}.{gen:0ANCHO_GEN$x}")
}

/// Separa `<nombre>.<gen>` y devuelve la generación, si el nombre encaja.
fn gen_de(nombre: &str, archivo: &str) -> Option<Generacion> {
    let resto = archivo.strip_prefix(nombre)?.strip_prefix('.')?;
    if resto.len() != ANCHO_GEN {
        return None;
    }
    Generacion::from_str_radix(resto, 16).ok()
}

/// Cabecera de una generación: el hash de lo que sigue.
///
/// Se guarda **dentro** del fichero para que la integridad no dependa de un
/// segundo fichero que también puede quedarse a medias.
fn empaquetar(datos: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(datos.len() + 66);
    out.extend_from_slice(sha256_hex(datos).as_bytes());
    out.push(b'\n');
    out.extend_from_slice(datos);
    out
}

/// Devuelve el contenido si el hash cuadra. `None` = generación a medias o
/// corrupta, que para el lector es lo mismo: no existe.
fn desempaquetar(crudo: &[u8]) -> Option<Vec<u8>> {
    let corte = crudo.iter().position(|&b| b == b'\n')?;
    let hash = core::str::from_utf8(&crudo[..corte]).ok()?;
    if hash.len() != 64 {
        return None;
    }
    let datos = &crudo[corte + 1..];
    if sha256_hex(datos) != hash {
        return None;
    }
    Some(datos.to_vec())
}

/// Generaciones presentes, de menor a mayor.
fn generaciones<D: Archivos>(d: &D, dir: &str, nombre: &str) -> Resultado<Vec<Generacion>> {
    let mut gens: Vec<Generacion> = d
        .listar(dir)?
        .into_iter()
        .filter_map(|e| gen_de(nombre, &e.ruta))
        .collect();
    gens.sort_unstable();
    Ok(gens)
}

/// La referencia vigente: la generación más alta que además está **íntegra**.
///
/// Se recorre de mayor a menor porque un corte deja la última a medias; la
/// anterior sigue siendo buena, y es la que hay que devolver.
pub fn leer_vigente<D: Archivos>(
    d: &D,
    dir: &str,
    nombre: &str,
) -> Resultado<Option<(Generacion, Vec<u8>)>> {
    let gens = generaciones(d, dir, nombre)?;
    Ok(vigente_entre(d, dir, nombre, &gens))
}

/// La vigente de entre unas generaciones ya listadas.
///
/// Existe aparte para que `publicar` pueda mirar las dos cosas —cuál es la
/// vigente y cuál es la última **presente**— sin listar el directorio dos
/// veces y arriesgarse a que cambie entre una y otra.
fn vigente_entre<D: Archivos>(
    d: &D,
    dir: &str,
    nombre: &str,
    gens: &[Generacion],
) -> Option<(Generacion, Vec<u8>)> {
    for gen in gens.iter().rev() {
        let ruta = unir(dir, &nombre_gen(nombre, *gen));
        let Ok(crudo) = d.leer(&ruta) else { continue };
        if let Some(datos) = desempaquetar(&crudo) {
            return Some((*gen, datos));
        }
    }
    None
}

/// Publica una referencia nueva y devuelve su generación.
///
/// `esperado` es la precondición: el hash que el llamante cree que hay ahora.
/// `None` significa «no debe haber nada». Si no cuadra, no se escribe: es la
/// forma de que dos escritores no se pisen sin inventar un candado.
pub fn publicar<D: Durable>(
    d: &mut D,
    dir: &str,
    nombre: &str,
    datos: &[u8],
    esperado: Option<&str>,
) -> Resultado<Generacion> {
    let gens = generaciones(d, dir, nombre)?;
    let vigente = vigente_entre(d, dir, nombre, &gens);
    let hash_actual = vigente.as_ref().map(|(_, v)| sha256_hex(v));
    if hash_actual.as_deref() != esperado {
        return Err(Error::uso(format!(
            "la referencia «{nombre}» cambió: se esperaba {}, hay {}",
            esperado.unwrap_or("nada"),
            hash_actual.as_deref().unwrap_or("nada")
        )));
    }
    // El número sale de la generación **presente** más alta, esté íntegra o
    // no, y no de la vigente (T63). Con la vigente, un corte dejaba un fichero
    // roto que el lector salta pero que el escritor vuelve a encontrarse en la
    // misma posición una y otra vez: `crear_exclusivo` fallaba con «ya existe»
    // para siempre y la referencia quedaba encallada en el último estado
    // bueno. El síntoma —una colisión de nombres— no se parecía en nada a la
    // causa, que era un apagón de hacía tres arranques.
    let siguiente = gens.last().map(|g| g + 1).unwrap_or(0);
    let ruta = unir(dir, &nombre_gen(nombre, siguiente));
    // Exclusivo: si otro escritor llegó antes con esta misma generación, aquí
    // falla, y falla **antes** de tocar la referencia anterior.
    d.crear_exclusivo(&ruta, &empaquetar(datos))?;
    Ok(siguiente)
}

/// Borra las generaciones anteriores a `conservar_desde`.
///
/// Se llama **después** de publicar, nunca antes: la anterior tiene que seguir
/// ahí hasta que la nueva esté acreditada. Un fallo al borrar no es un fallo de
/// la publicación —sobra un fichero, no falta—, así que se informa aparte.
///
/// Borra también las **rotas** que queden por debajo, y eso es deliberado: una
/// generación a medias es la única huella de que hubo un corte, pero
/// conservarla para siempre hace que el directorio crezca con cada apagón. Para
/// cuando `purgar` corre, la generación nueva ya está publicada y el corte ya
/// no explica nada que no esté en el estado vigente.
pub fn purgar<D: Durable>(
    d: &mut D,
    dir: &str,
    nombre: &str,
    conservar_desde: Generacion,
) -> Resultado<usize> {
    let gens = generaciones(d, dir, nombre)?;
    let mut borradas = 0;
    for gen in gens {
        if gen >= conservar_desde {
            continue;
        }
        let ruta = unir(dir, &nombre_gen(nombre, gen));
        if d.borrar(&ruta).is_ok() {
            borradas += 1;
        }
    }
    Ok(borradas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entorno::{Entrada, Tipo};
    use alloc::collections::BTreeMap;

    /// Árbol en memoria con inyección de fallos: se puede cortar la escritura
    /// a la mitad, que es lo que hace un apagón.
    #[derive(Default)]
    struct Memoria {
        ficheros: BTreeMap<String, Vec<u8>>,
        /// Si está, la próxima escritura guarda sólo estos bytes y falla.
        cortar_en: Option<usize>,
        /// Si está, la próxima escritura falla **sin dejar nada**, como un
        /// disco lleno que rechaza la creación.
        sin_espacio: bool,
    }

    impl Archivos for Memoria {
        fn leer(&self, ruta: &str) -> Resultado<Vec<u8>> {
            self.ficheros
                .get(ruta)
                .cloned()
                .ok_or_else(|| Error::entorno(format!("no existe {ruta}")))
        }
        fn escribir(&mut self, ruta: &str, datos: &[u8], _modo: u32) -> Resultado<()> {
            self.ficheros.insert(ruta.to_string(), datos.to_vec());
            Ok(())
        }
        fn existe(&self, ruta: &str) -> bool {
            self.ficheros.contains_key(ruta)
        }
        fn listar(&self, dir: &str) -> Resultado<Vec<Entrada>> {
            let prefijo = format!("{dir}/");
            Ok(self
                .ficheros
                .keys()
                .filter_map(|k| k.strip_prefix(&prefijo))
                .filter(|r| !r.contains('/'))
                .map(|r| Entrada {
                    ruta: r.to_string(),
                    tipo: Tipo::Archivo,
                    bytes: 0,
                    modo: 0,
                })
                .collect())
        }
        fn metadatos(&self, ruta: &str) -> Resultado<Entrada> {
            let bytes = self.leer(ruta)?.len() as u64;
            Ok(Entrada {
                ruta: ruta.to_string(),
                tipo: Tipo::Archivo,
                bytes,
                modo: 0,
            })
        }
        fn crear_directorio(&mut self, _ruta: &str) -> Resultado<()> {
            Ok(())
        }
        fn borrar(&mut self, ruta: &str) -> Resultado<()> {
            self.ficheros
                .remove(ruta)
                .map(|_| ())
                .ok_or_else(|| Error::entorno(format!("no existe {ruta}")))
        }
    }

    impl Durable for Memoria {
        fn crear_exclusivo(&mut self, ruta: &str, datos: &[u8]) -> Resultado<()> {
            if self.ficheros.contains_key(ruta) {
                return Err(Error::uso(format!("{ruta} ya existe")));
            }
            if self.sin_espacio {
                return Err(Error::entorno("no queda espacio"));
            }
            match self.cortar_en.take() {
                Some(n) => {
                    let corte = n.min(datos.len());
                    self.ficheros.insert(ruta.to_string(), datos[..corte].to_vec());
                    Err(Error::entorno("corte durante la escritura"))
                }
                None => {
                    self.ficheros.insert(ruta.to_string(), datos.to_vec());
                    Ok(())
                }
            }
        }
        fn sincronizar(&mut self, _ruta: &str) -> Resultado<()> {
            Ok(())
        }
    }

    #[test]
    fn publicar_y_leer() {
        let mut m = Memoria::default();
        let g = publicar(&mut m, "/d", "estado", b"uno", None).unwrap();
        assert_eq!(g, 0);
        let (gen, datos) = leer_vigente(&m, "/d", "estado").unwrap().unwrap();
        assert_eq!(gen, 0);
        assert_eq!(datos, b"uno");
    }

    /// La regla central: un corte a mitad de la nueva generación deja la
    /// **anterior entera**, nunca una mezcla.
    #[test]
    fn un_corte_deja_la_anterior_no_una_mezcla() {
        let mut m = Memoria::default();
        publicar(&mut m, "/d", "estado", b"version-uno", None).unwrap();
        let h1 = sha256_hex(b"version-uno");

        m.cortar_en = Some(40); // se queda a mitad de la cabecera + datos
        let e = publicar(&mut m, "/d", "estado", b"version-dos", Some(&h1)).unwrap_err();
        assert!(matches!(e, Error::Entorno(_)), "{e}");

        // El fichero a medias existe…
        assert!(m.existe("/d/estado.0000000000000001"));
        // …pero el lector se queda con la anterior, entera.
        let (gen, datos) = leer_vigente(&m, "/d", "estado").unwrap().unwrap();
        assert_eq!(gen, 0);
        assert_eq!(datos, b"version-uno");
    }

    /// Tras el corte se puede volver a publicar **sin limpiar nada** (T63).
    ///
    /// Esta prueba decía antes lo contrario: que había «que quitar la basura
    /// antes», y borraba el fichero roto a mano para poder seguir. Eso no era
    /// una precondición del contrato, era el defecto: en una máquina real no
    /// hay nadie para borrarlo, así que la referencia se quedaba encallada en
    /// el último estado bueno para siempre.
    #[test]
    fn se_puede_reintentar_tras_un_corte() {
        let mut m = Memoria::default();
        publicar(&mut m, "/d", "estado", b"uno", None).unwrap();
        let h1 = sha256_hex(b"uno");
        m.cortar_en = Some(30);
        let _ = publicar(&mut m, "/d", "estado", b"dos", Some(&h1));
        assert!(m.existe("/d/estado.0000000000000001"), "el corte dejó su resto");

        // La vigente sigue siendo la 0, pero la nueva no vuelve a la 1: salta
        // por encima del resto del corte.
        let g = publicar(&mut m, "/d", "estado", b"dos", Some(&h1)).unwrap();
        assert_eq!(g, 2);
        let (gen, datos) = leer_vigente(&m, "/d", "estado").unwrap().unwrap();
        assert_eq!(gen, 2);
        assert_eq!(datos, b"dos");
    }

    /// Dos cortes seguidos tampoco encallan: cada uno deja su resto y la
    /// numeración sigue avanzando. Con un solo corte el fallo se podía
    /// confundir con una casualidad de numeración.
    #[test]
    fn dos_cortes_seguidos_no_encallan() {
        let mut m = Memoria::default();
        publicar(&mut m, "/d", "estado", b"uno", None).unwrap();
        let h1 = sha256_hex(b"uno");
        for corte in [30, 25] {
            m.cortar_en = Some(corte);
            assert!(publicar(&mut m, "/d", "estado", b"dos", Some(&h1)).is_err());
        }
        let g = publicar(&mut m, "/d", "estado", b"dos", Some(&h1)).unwrap();
        assert_eq!(g, 3);
        assert_eq!(leer_vigente(&m, "/d", "estado").unwrap().unwrap().1, b"dos");
    }

    /// Y la limpieza se lleva los restos: si no, el directorio crecería un
    /// fichero por apagón y nadie los borraría nunca.
    #[test]
    fn purgar_se_lleva_tambien_las_rotas() {
        let mut m = Memoria::default();
        publicar(&mut m, "/d", "estado", b"uno", None).unwrap();
        let h1 = sha256_hex(b"uno");
        m.cortar_en = Some(30);
        let _ = publicar(&mut m, "/d", "estado", b"dos", Some(&h1));
        let g = publicar(&mut m, "/d", "estado", b"dos", Some(&h1)).unwrap();

        assert_eq!(purgar(&mut m, "/d", "estado", g).unwrap(), 2);
        assert!(!m.existe("/d/estado.0000000000000001"), "el resto del corte sobra");
        assert_eq!(leer_vigente(&m, "/d", "estado").unwrap().unwrap().1, b"dos");
    }

    /// La precondición de hash es lo que impide que dos escritores se pisen.
    #[test]
    fn la_precondicion_rechaza_al_segundo_escritor() {
        let mut m = Memoria::default();
        publicar(&mut m, "/d", "estado", b"uno", None).unwrap();
        let h1 = sha256_hex(b"uno");
        publicar(&mut m, "/d", "estado", b"dos", Some(&h1)).unwrap();

        // El segundo escritor cree que sigue en «uno»: se le dice que no.
        let e = publicar(&mut m, "/d", "estado", b"tres", Some(&h1)).unwrap_err();
        assert!(format!("{e}").contains("cambió"), "{e}");
        let (_, datos) = leer_vigente(&m, "/d", "estado").unwrap().unwrap();
        assert_eq!(datos, b"dos");
    }

    #[test]
    fn publicar_sobre_algo_existente_sin_precondicion_falla() {
        let mut m = Memoria::default();
        publicar(&mut m, "/d", "estado", b"uno", None).unwrap();
        let e = publicar(&mut m, "/d", "estado", b"dos", None).unwrap_err();
        assert!(format!("{e}").contains("cambió"), "{e}");
    }

    /// Purgar va **después** de publicar y nunca toca la vigente.
    #[test]
    fn purgar_conserva_la_vigente() {
        let mut m = Memoria::default();
        let mut h = None;
        for v in [b"uno".as_slice(), b"dos", b"tres"] {
            publicar(&mut m, "/d", "estado", v, h.as_deref()).unwrap();
            h = Some(sha256_hex(v));
        }
        let borradas = purgar(&mut m, "/d", "estado", 2).unwrap();
        assert_eq!(borradas, 2);
        let (gen, datos) = leer_vigente(&m, "/d", "estado").unwrap().unwrap();
        assert_eq!(gen, 2);
        assert_eq!(datos, b"tres");
    }

    /// Disco lleno: la publicación falla y la referencia anterior sigue
    /// intacta. No queda ni siquiera un fichero a medias.
    #[test]
    fn sin_espacio_no_toca_la_anterior() {
        let mut m = Memoria::default();
        publicar(&mut m, "/d", "estado", b"uno", None).unwrap();
        let h1 = sha256_hex(b"uno");
        m.sin_espacio = true;
        let e = publicar(&mut m, "/d", "estado", b"dos", Some(&h1)).unwrap_err();
        assert!(format!("{e}").contains("espacio"), "{e}");
        assert!(!m.existe("/d/estado.0000000000000001"));
        let (gen, datos) = leer_vigente(&m, "/d", "estado").unwrap().unwrap();
        assert_eq!(gen, 0);
        assert_eq!(datos, b"uno");
    }

    #[test]
    fn sin_referencia_no_hay_vigente() {
        let m = Memoria::default();
        assert!(leer_vigente(&m, "/d", "estado").unwrap().is_none());
    }

    /// Un nombre parecido no se confunde con una generación.
    #[test]
    fn los_nombres_no_se_confunden() {
        assert_eq!(gen_de("estado", "estado.0000000000000003"), Some(3));
        assert_eq!(gen_de("estado", "estado.3"), None);
        assert_eq!(gen_de("estado", "estado"), None);
        assert_eq!(gen_de("estado", "estadobis.0000000000000003"), None);
        assert_eq!(gen_de("estado", "otro.0000000000000003"), None);
    }
}
