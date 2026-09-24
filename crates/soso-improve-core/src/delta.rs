//! Paquete de cambios por contenido: aplicar un candidato sin Git (T50).
//!
//! El coordinador tiene que poder coger una captura de [`crate::captura`] y
//! reconstruir **exactamente** el árbol que produjo un candidato. Con `git
//! apply` sería fácil; dentro de soso no hay Git, y el plan dice que no puede
//! haberlo como requisito.
//!
//! Así que un cambio no es un diff: es una lista de operaciones con **ruta,
//! hash antes y hash después**, y el contenido vive en el mismo almacén por
//! hash que ya usa la captura. Eso da tres cosas que un diff de texto no da:
//!
//! - funciona igual con binarios, archivos vacíos y rutas con espacios;
//! - la precondición es explícita: si el árbol no es el que el paquete espera,
//!   se sabe **antes** de escribir, no a mitad;
//! - el resultado es verificable, porque el paquete dice qué huella debe tener
//!   el árbol final.
//!
//! La regla que gobierna el orden de todo lo que sigue: **validar entero antes
//! de escribir nada**. Un paquete que falla a la mitad deja un árbol que no es
//! ni el viejo ni el nuevo, y eso es peor que no aplicarlo.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::captura::{huella, ruta_objeto, ArchivoBase};
use crate::entorno::Archivos;
use crate::{sha256_hex, unir, Error, Resultado, ESQUEMA};

/// Qué le pasa a una ruta.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "tipo", rename_all = "lowercase")]
pub enum Operacion {
    /// La ruta no existía y pasa a existir.
    Alta {
        ruta: String,
        hash_despues: String,
        #[serde(default)]
        modo: u32,
    },
    /// Existía con `hash_antes` y pasa a `hash_despues`.
    Modificacion {
        ruta: String,
        hash_antes: String,
        hash_despues: String,
        #[serde(default)]
        modo: u32,
    },
    /// Existía con `hash_antes` y desaparece.
    Borrado { ruta: String, hash_antes: String },
}

impl Operacion {
    pub fn ruta(&self) -> &str {
        match self {
            Operacion::Alta { ruta, .. }
            | Operacion::Modificacion { ruta, .. }
            | Operacion::Borrado { ruta, .. } => ruta,
        }
    }

    /// Hash del contenido que hay que tener en el almacén, si lo hay.
    pub fn objeto(&self) -> Option<&str> {
        match self {
            Operacion::Alta { hash_despues, .. }
            | Operacion::Modificacion { hash_despues, .. } => Some(hash_despues),
            Operacion::Borrado { .. } => None,
        }
    }
}

/// Un cambio completo y comprobable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Paquete {
    pub schema_version: u32,
    /// Huella del inventario que el paquete espera encontrar.
    pub base: String,
    /// Huella del inventario que debe quedar.
    pub destino: String,
    pub operaciones: Vec<Operacion>,
    /// Metadato, **nunca** requisito: el commit o el diff de Git pueden viajar
    /// aquí para que un humano se sitúe, y aplicar no los mira.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
}

/// Rutas que no se aceptan, y por qué.
///
/// Se comprueba aquí y no al escribir: una ruta con `..` que se descubre a
/// mitad de aplicar ya ha hecho daño.
fn ruta_valida(ruta: &str) -> Resultado<()> {
    if ruta.is_empty() {
        return Err(Error::uso("ruta vacía"));
    }
    if ruta.starts_with('/') {
        return Err(Error::uso(format!("ruta absoluta: {ruta}")));
    }
    if ruta.contains('\\') {
        return Err(Error::uso(format!("separador de Windows: {ruta}")));
    }
    for seg in ruta.split('/') {
        if seg == ".." {
            return Err(Error::uso(format!("la ruta se sale del árbol: {ruta}")));
        }
        if seg.is_empty() || seg == "." {
            return Err(Error::uso(format!("segmento vacío o «.»: {ruta}")));
        }
    }
    Ok(())
}

/// Inventario resultante de aplicar el paquete a `base`, sin tocar nada.
///
/// Sirve para dos cosas: comprobar que la huella destino cuadra **antes** de
/// escribir, y construir el manifiesto del candidato.
pub fn inventario_resultante(
    base: &[ArchivoBase],
    paquete: &Paquete,
) -> Resultado<Vec<ArchivoBase>> {
    let mut fuera: Vec<ArchivoBase> = base.to_vec();
    for op in &paquete.operaciones {
        let ruta = op.ruta();
        let pos = fuera.iter().position(|a| a.ruta == ruta);
        match op {
            Operacion::Alta {
                hash_despues, modo, ..
            } => {
                if pos.is_some() {
                    return Err(Error::uso(format!("alta de algo que ya existe: {ruta}")));
                }
                fuera.push(ArchivoBase {
                    ruta: ruta.to_string(),
                    sha256: hash_despues.clone(),
                    bytes: 0,
                    modo: *modo,
                });
            }
            Operacion::Modificacion {
                hash_antes,
                hash_despues,
                modo,
                ..
            } => {
                let i = pos.ok_or_else(|| {
                    Error::uso(format!("modificación de algo que no existe: {ruta}"))
                })?;
                if &fuera[i].sha256 != hash_antes {
                    return Err(Error::uso(format!(
                        "precondición distinta en {ruta}: el paquete espera {hash_antes}, hay {}",
                        fuera[i].sha256
                    )));
                }
                fuera[i].sha256 = hash_despues.clone();
                fuera[i].modo = *modo;
            }
            Operacion::Borrado { hash_antes, .. } => {
                let i = pos.ok_or_else(|| {
                    Error::uso(format!("borrado de algo que no existe: {ruta}"))
                })?;
                if &fuera[i].sha256 != hash_antes {
                    return Err(Error::uso(format!(
                        "precondición distinta en {ruta}: el paquete espera {hash_antes}, hay {}",
                        fuera[i].sha256
                    )));
                }
                fuera.remove(i);
            }
        }
    }
    fuera.sort_by(|a, b| a.ruta.cmp(&b.ruta));
    Ok(fuera)
}

/// Comprueba el paquete **entero** contra la base y el almacén.
///
/// Devuelve el inventario que quedaría. No escribe nada: si algo falla, el
/// árbol original sigue como estaba porque ni se ha tocado.
pub fn validar(
    almacen: &dyn Archivos,
    raiz_almacen: &str,
    base: &[ArchivoBase],
    paquete: &Paquete,
) -> Resultado<Vec<ArchivoBase>> {
    if paquete.schema_version != ESQUEMA {
        return Err(Error::formato(format!(
            "paquete de esquema {}; esta herramienta habla {ESQUEMA}",
            paquete.schema_version
        )));
    }
    let base_huella = huella(base);
    if base_huella != paquete.base {
        return Err(Error::uso(format!(
            "base distinta: el paquete espera {}, el árbol es {base_huella}",
            paquete.base
        )));
    }
    let mut vistas: BTreeSet<&str> = BTreeSet::new();
    for op in &paquete.operaciones {
        ruta_valida(op.ruta())?;
        if !vistas.insert(op.ruta()) {
            return Err(Error::uso(format!(
                "la ruta {} aparece dos veces en el paquete",
                op.ruta()
            )));
        }
        // El objeto tiene que estar **antes** de empezar: descubrir que falta a
        // mitad de aplicar deja el árbol partido.
        if let Some(hash) = op.objeto() {
            let ruta = unir(raiz_almacen, &ruta_objeto(hash));
            if !almacen.existe(&ruta) {
                return Err(Error::uso(format!(
                    "falta el objeto {hash} de {}",
                    op.ruta()
                )));
            }
        }
    }
    let resultante = inventario_resultante(base, paquete)?;
    let destino = huella(&resultante);
    if destino != paquete.destino {
        return Err(Error::uso(format!(
            "el paquete no produce lo que dice: destino {} vs {destino}",
            paquete.destino
        )));
    }
    Ok(resultante)
}

/// Aplica el paquete sobre `arbol`. Valida entero antes de escribir.
///
/// Devuelve el inventario resultante ya verificado contra la huella destino.
pub fn aplicar(
    almacen: &dyn Archivos,
    raiz_almacen: &str,
    arbol: &mut dyn Archivos,
    raiz_arbol: &str,
    base: &[ArchivoBase],
    paquete: &Paquete,
) -> Resultado<Vec<ArchivoBase>> {
    let resultante = validar(almacen, raiz_almacen, base, paquete)?;

    // Primero las altas y modificaciones, después los borrados: si algo falla
    // a media escritura, lo que se pierde es lo nuevo, no lo viejo.
    for op in &paquete.operaciones {
        let Some(hash) = op.objeto() else { continue };
        let datos = almacen.leer(&unir(raiz_almacen, &ruta_objeto(hash)))?;
        if sha256_hex(&datos) != hash {
            return Err(Error::formato(format!(
                "el objeto {hash} no cuadra con su hash"
            )));
        }
        let modo = match op {
            Operacion::Alta { modo, .. } | Operacion::Modificacion { modo, .. } => *modo,
            Operacion::Borrado { .. } => 0,
        };
        arbol.escribir(&unir(raiz_arbol, op.ruta()), &datos, modo)?;
    }
    for op in &paquete.operaciones {
        if let Operacion::Borrado { ruta, .. } = op {
            arbol.borrar(&unir(raiz_arbol, ruta))?;
        }
    }
    Ok(resultante)
}

/// Construye el paquete que lleva de `base` a `destino`, comparando
/// inventarios. Sin diffs de texto: rutas y hashes.
pub fn exportar(base: &[ArchivoBase], destino: &[ArchivoBase]) -> Paquete {
    let mut operaciones = Vec::new();
    for d in destino {
        match base.iter().find(|b| b.ruta == d.ruta) {
            None => operaciones.push(Operacion::Alta {
                ruta: d.ruta.clone(),
                hash_despues: d.sha256.clone(),
                modo: d.modo,
            }),
            Some(b) if b.sha256 != d.sha256 || b.modo != d.modo => {
                operaciones.push(Operacion::Modificacion {
                    ruta: d.ruta.clone(),
                    hash_antes: b.sha256.clone(),
                    hash_despues: d.sha256.clone(),
                    modo: d.modo,
                })
            }
            Some(_) => {}
        }
    }
    for b in base {
        if !destino.iter().any(|d| d.ruta == b.ruta) {
            operaciones.push(Operacion::Borrado {
                ruta: b.ruta.clone(),
                hash_antes: b.sha256.clone(),
            });
        }
    }
    operaciones.sort_by(|a, b| a.ruta().cmp(b.ruta()));
    Paquete {
        schema_version: ESQUEMA,
        base: huella(base),
        destino: huella(destino),
        operaciones,
        git: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entorno::{Entrada, Tipo};
    use alloc::collections::BTreeMap;

    #[derive(Default)]
    struct Memoria {
        f: BTreeMap<String, Vec<u8>>,
        fallar_en: Option<String>,
    }

    impl Archivos for Memoria {
        fn leer(&self, ruta: &str) -> Resultado<Vec<u8>> {
            self.f
                .get(ruta)
                .cloned()
                .ok_or_else(|| Error::entorno(format!("no existe {ruta}")))
        }
        fn escribir(&mut self, ruta: &str, datos: &[u8], _m: u32) -> Resultado<()> {
            if self.fallar_en.as_deref() == Some(ruta) {
                return Err(Error::entorno(format!("fallo de escritura en {ruta}")));
            }
            self.f.insert(ruta.to_string(), datos.to_vec());
            Ok(())
        }
        fn existe(&self, ruta: &str) -> bool {
            self.f.contains_key(ruta)
        }
        fn listar(&self, _d: &str) -> Resultado<Vec<Entrada>> {
            Ok(Vec::new())
        }
        fn metadatos(&self, ruta: &str) -> Resultado<Entrada> {
            Ok(Entrada {
                ruta: ruta.to_string(),
                tipo: Tipo::Archivo,
                bytes: self.leer(ruta)?.len() as u64,
                modo: 0,
            })
        }
        fn crear_directorio(&mut self, _r: &str) -> Resultado<()> {
            Ok(())
        }
        fn borrar(&mut self, ruta: &str) -> Resultado<()> {
            self.f
                .remove(ruta)
                .map(|_| ())
                .ok_or_else(|| Error::entorno(format!("no existe {ruta}")))
        }
    }

    /// Guarda contenidos en el almacén y devuelve el inventario.
    fn sembrar(m: &mut Memoria, raiz: &str, ficheros: &[(&str, &[u8])]) -> Vec<ArchivoBase> {
        let mut inv = Vec::new();
        for (ruta, datos) in ficheros {
            let h = sha256_hex(datos);
            m.escribir(&unir(raiz, &ruta_objeto(&h)), datos, 0).unwrap();
            inv.push(ArchivoBase {
                ruta: ruta.to_string(),
                sha256: h,
                bytes: datos.len() as u64,
                modo: 0o644,
            });
        }
        inv.sort_by(|a, b| a.ruta.cmp(&b.ruta));
        inv
    }

    /// Ida y vuelta con lo que un diff de texto hace mal: binarios, vacíos,
    /// borrados y rutas con espacios.
    #[test]
    fn roundtrip_con_binarios_vacios_y_espacios() {
        let mut m = Memoria::default();
        let base = sembrar(
            &mut m,
            "/alm",
            &[
                ("src/main.rs", b"fn main() {}"),
                ("datos/con espacio.bin", &[0u8, 159, 146, 150]),
                ("vacio.txt", b""),
                ("sobra.txt", b"me voy"),
            ],
        );
        let destino = sembrar(
            &mut m,
            "/alm",
            &[
                ("src/main.rs", b"fn main() { println!(); }"),
                ("datos/con espacio.bin", &[0u8, 159, 146, 150]),
                ("vacio.txt", b""),
                ("nuevo/hondo.txt", b"alta"),
            ],
        );

        let p = exportar(&base, &destino);
        // Una modificación, un alta y un borrado; el binario y el vacío no se
        // tocan porque su hash no cambió.
        assert_eq!(p.operaciones.len(), 3);

        let mut arbol = Memoria::default();
        for (ruta, datos) in [
            ("src/main.rs", b"fn main() {}".as_slice()),
            ("datos/con espacio.bin", &[0u8, 159, 146, 150]),
            ("vacio.txt", b""),
            ("sobra.txt", b"me voy"),
        ] {
            arbol.escribir(&unir("/w", ruta), datos, 0).unwrap();
        }

        let inv = aplicar(&m, "/alm", &mut arbol, "/w", &base, &p).unwrap();
        assert_eq!(huella(&inv), p.destino);
        assert_eq!(
            arbol.leer("/w/src/main.rs").unwrap(),
            b"fn main() { println!(); }"
        );
        assert_eq!(arbol.leer("/w/nuevo/hondo.txt").unwrap(), b"alta");
        assert_eq!(arbol.leer("/w/vacio.txt").unwrap(), b"");
        assert!(!arbol.existe("/w/sobra.txt"));
    }

    /// Base equivocada: se dice **antes** de escribir, y el árbol no se toca.
    #[test]
    fn base_incorrecta_no_toca_nada() {
        let mut m = Memoria::default();
        let base = sembrar(&mut m, "/alm", &[("a.txt", b"uno")]);
        let destino = sembrar(&mut m, "/alm", &[("a.txt", b"dos")]);
        let mut p = exportar(&base, &destino);
        p.base = String::from("0".repeat(64));

        let mut arbol = Memoria::default();
        arbol.escribir("/w/a.txt", b"uno", 0).unwrap();
        let e = aplicar(&m, "/alm", &mut arbol, "/w", &base, &p).unwrap_err();
        assert!(format!("{e}").contains("base distinta"), "{e}");
        assert_eq!(arbol.leer("/w/a.txt").unwrap(), b"uno");
    }

    /// Objeto ausente: también se descubre antes de empezar.
    #[test]
    fn objeto_ausente_no_toca_nada() {
        let mut m = Memoria::default();
        let base = sembrar(&mut m, "/alm", &[("a.txt", b"uno")]);
        let destino = sembrar(&mut m, "/alm", &[("a.txt", b"dos")]);
        let p = exportar(&base, &destino);
        // Se borra el objeto nuevo del almacén.
        let h = sha256_hex(b"dos");
        m.borrar(&unir("/alm", &ruta_objeto(&h))).unwrap();

        let mut arbol = Memoria::default();
        arbol.escribir("/w/a.txt", b"uno", 0).unwrap();
        let e = aplicar(&m, "/alm", &mut arbol, "/w", &base, &p).unwrap_err();
        assert!(format!("{e}").contains("falta el objeto"), "{e}");
        assert_eq!(arbol.leer("/w/a.txt").unwrap(), b"uno");
    }

    /// Un fallo de escritura a mitad no puede pasar por éxito.
    #[test]
    fn fallo_de_escritura_se_propaga() {
        let mut m = Memoria::default();
        let base = sembrar(&mut m, "/alm", &[("a.txt", b"uno")]);
        let destino = sembrar(&mut m, "/alm", &[("a.txt", b"dos")]);
        let p = exportar(&base, &destino);

        let mut arbol = Memoria::default();
        arbol.escribir("/w/a.txt", b"uno", 0).unwrap();
        arbol.fallar_en = Some(String::from("/w/a.txt"));
        let e = aplicar(&m, "/alm", &mut arbol, "/w", &base, &p).unwrap_err();
        assert!(format!("{e}").contains("fallo de escritura"), "{e}");
    }

    #[test]
    fn rechaza_traversal_y_rutas_absolutas() {
        for mala in ["../fuera.txt", "/etc/passwd", "a/../../b", "a//b", "a/./b"] {
            let p = Paquete {
                schema_version: ESQUEMA,
                base: huella(&[]),
                destino: String::new(),
                operaciones: alloc::vec![Operacion::Alta {
                    ruta: mala.to_string(),
                    hash_despues: sha256_hex(b"x"),
                    modo: 0,
                }],
                git: None,
            };
            let m = Memoria::default();
            let e = validar(&m, "/alm", &[], &p).unwrap_err();
            assert!(
                format!("{e}").contains("ruta") || format!("{e}").contains("segmento"),
                "{mala}: {e}"
            );
        }
    }

    #[test]
    fn rechaza_rutas_duplicadas() {
        let mut m = Memoria::default();
        let h = sha256_hex(b"x");
        m.escribir(&unir("/alm", &ruta_objeto(&h)), b"x", 0).unwrap();
        let p = Paquete {
            schema_version: ESQUEMA,
            base: huella(&[]),
            destino: String::new(),
            operaciones: alloc::vec![
                Operacion::Alta {
                    ruta: String::from("a.txt"),
                    hash_despues: h.clone(),
                    modo: 0
                },
                Operacion::Alta {
                    ruta: String::from("a.txt"),
                    hash_despues: h,
                    modo: 0
                },
            ],
            git: None,
        };
        let e = validar(&m, "/alm", &[], &p).unwrap_err();
        assert!(format!("{e}").contains("dos veces"), "{e}");
    }

    /// Una precondición que no cuadra se ve antes de escribir.
    #[test]
    fn precondicion_distinta_se_rechaza() {
        let mut m = Memoria::default();
        let base = sembrar(&mut m, "/alm", &[("a.txt", b"uno")]);
        let otra = sembrar(&mut m, "/alm", &[("a.txt", b"otra cosa")]);
        let destino = sembrar(&mut m, "/alm", &[("a.txt", b"dos")]);
        // Paquete construido contra `otra`, aplicado sobre `base`.
        let p = exportar(&otra, &destino);
        let e = validar(&m, "/alm", &base, &p).unwrap_err();
        assert!(format!("{e}").contains("base distinta"), "{e}");
    }

    /// El metadato de Git viaja, pero aplicar no lo mira.
    #[test]
    fn git_es_metadato_no_requisito() {
        let mut m = Memoria::default();
        let base = sembrar(&mut m, "/alm", &[("a.txt", b"uno")]);
        let destino = sembrar(&mut m, "/alm", &[("a.txt", b"dos")]);
        let mut p = exportar(&base, &destino);
        p.git = Some(String::from("deadbeef"));
        let mut arbol = Memoria::default();
        arbol.escribir("/w/a.txt", b"uno", 0).unwrap();
        let inv = aplicar(&m, "/alm", &mut arbol, "/w", &base, &p).unwrap();
        assert_eq!(huella(&inv), p.destino);
    }

    /// La huella del fixture compartido queda **fijada** aquí. El guest
    /// imprime la suya al aplicar el mismo paquete; si las dos no coinciden,
    /// el formato no se aplica igual en los dos sitios, que es justo lo que
    /// esta ficha tiene que descartar.
    #[test]
    fn la_huella_del_fixture_esta_fijada() {
        let mut m = Memoria::default();
        let base = sembrar(&mut m, "/alm", fixture::BASE);
        let destino = sembrar(&mut m, "/alm", fixture::DESTINO);
        let p = exportar(&base, &destino);
        assert_eq!(p.destino, huella(&destino));
        assert_eq!(
            p.destino, HUELLA_FIXTURE,
            "si esto cambia, hay que cambiar también lo que comprueba el guest"
        );
    }

    #[test]
    fn el_esquema_se_comprueba() {
        let p = Paquete {
            schema_version: ESQUEMA + 1,
            base: huella(&[]),
            destino: String::new(),
            operaciones: Vec::new(),
            git: None,
        };
        let m = Memoria::default();
        let e = validar(&m, "/alm", &[], &p).unwrap_err();
        assert!(format!("{e}").contains("esquema"), "{e}");
    }
}

/// Huella del árbol destino del fixture. Host y guest tienen que producir
/// **esta misma** cadena; si no, el formato no se aplica igual en los dos
/// sitios y todo lo demás sobra.
pub const HUELLA_FIXTURE: &str =
    "f07a24b7c1e6c9f5941c19c8042eb179857cb1d8dac077a119c3040a973abccd";

/// Fixture compartido host/guest para acreditar que el formato se aplica igual
/// en los dos sitios (T50, paso «comparar hashes con el host»).
///
/// Va en el crate portable a propósito: si el host y el guest usaran datos
/// distintos, comparar sus huellas no diría nada.
pub mod fixture {
    /// Árbol de partida: texto, binario con bytes altos, vacío y ruta con
    /// espacios — lo que un diff de texto maneja mal.
    pub const BASE: &[(&str, &[u8])] = &[
        ("src/main.rs", b"fn main() {}"),
        ("datos/con espacio.bin", &[0u8, 159, 146, 150]),
        ("vacio.txt", b""),
        ("sobra.txt", b"me voy"),
    ];

    /// Árbol al que se quiere llegar: una modificación, un alta honda, un
    /// borrado, y dos que no cambian.
    pub const DESTINO: &[(&str, &[u8])] = &[
        ("src/main.rs", b"fn main() { println!(); }"),
        ("datos/con espacio.bin", &[0u8, 159, 146, 150]),
        ("vacio.txt", b""),
        ("nuevo/hondo.txt", b"alta"),
    ];
}
