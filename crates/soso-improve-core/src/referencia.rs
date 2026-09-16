//! Comparación de fixtures de referencia contra el tokenizer de soso (T03).
//!
//! Un fixture dice qué texto produce la plantilla oficial del modelo y qué
//! token IDs saca **su** tokenizer. Este módulo comprueba si el tokenizer que
//! usa soso saca los mismos, encuentra la **primera** divergencia y la
//! clasifica. Nada más: decidir si el modelo sirve es T14, y arreglar el
//! renderer es T06.
//!
//! La referencia la genera una herramienta externa offline
//! (`tools/tokenizer-ref`). Aquí no se toca: leer el fixture y compararlo es
//! trabajo del proyecto, y tiene que poder hacerse dentro de soso. Por eso el
//! tokenizer entra por un trait y no como dependencia directa: quien compara
//! no necesita saber de dónde salen los IDs.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::entorno::Archivos;
use crate::{unir, Error, Resultado};

/// Lo que se le pide a un tokenizer para poder compararlo.
pub trait Tokeniza {
    fn encode(&self, texto: &str) -> Vec<u32>;
    /// Número de piezas del vocabulario, para comprobar que ningún ID se sale.
    fn vocab_size(&self) -> u32;
    /// Pieza de un id, si se puede saber. Sirve para explicar la divergencia.
    fn pieza(&self, _id: u32) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub schema_version: u32,
    pub nombre: String,
    pub modelo: String,
    pub revision: String,
    pub texto: String,
    pub texto_sha256: String,
    pub ids: Vec<u32>,
    pub ids_sha256: String,
    #[serde(default)]
    pub especiales: alloc::collections::BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntradaIndice {
    pub nombre: String,
    pub archivo: String,
    pub tokens: usize,
    pub texto_sha256: String,
    pub ids_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Indice {
    pub schema_version: u32,
    pub modelo: String,
    pub revision: String,
    pub vocab_size: u32,
    pub fixtures: Vec<EntradaIndice>,
}

/// Cómo de distinta es la primera divergencia. La clasificación la exige la
/// ficha porque cada clase se arregla en un sitio distinto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Clase {
    /// Un token especial (`<|im_start|>`…) que el vocabulario de soso no tiene
    /// o numera distinto.
    TokenEspecial,
    /// Mismo texto, distinta forma de marcar espacios o normalizar.
    Normalizacion,
    /// El texto se parte en piezas distintas.
    Segmentacion,
    /// Las piezas coinciden pero el id no: conversión de vocabulario.
    Conversion,
    /// El id se sale del vocabulario declarado.
    FueraDeVocabulario,
    /// Longitudes distintas sin divergencia previa: falta o sobra cola.
    Longitud,
}

impl Clase {
    pub fn como_texto(&self) -> &'static str {
        match self {
            Clase::TokenEspecial => "token especial",
            Clase::Normalizacion => "normalización",
            Clase::Segmentacion => "segmentación",
            Clase::Conversion => "conversión",
            Clase::FueraDeVocabulario => "fuera de vocabulario",
            Clase::Longitud => "longitud",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Divergencia {
    /// Índice del primer token que no coincide.
    pub posicion: usize,
    pub clase: Clase,
    pub esperado: Option<u32>,
    pub obtenido: Option<u32>,
    pub pieza_esperada: Option<String>,
    pub pieza_obtenida: Option<String>,
    /// Trozo del texto donde ocurre, para poder mirarlo a ojo.
    pub contexto: String,
    pub explicacion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparacion {
    pub fixture: String,
    pub modelo: String,
    pub revision: String,
    pub tokens_referencia: usize,
    pub tokens_soso: usize,
    pub iguales: bool,
    /// IDs de la referencia que no caben en el vocabulario de soso.
    pub fuera_de_vocabulario: Vec<u32>,
    pub divergencia: Option<Divergencia>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Informe {
    pub schema_version: u32,
    pub modelo: String,
    pub revision: String,
    pub vocab_referencia: u32,
    pub vocab_soso: u32,
    pub comparaciones: Vec<Comparacion>,
    pub iguales: usize,
    pub divergentes: usize,
}

impl Informe {
    /// `0` todo coincide, `2` hay divergencias. Que haya divergencias no es un
    /// error del programa: es el resultado que la ficha pide clasificar.
    pub fn codigo(&self) -> i32 {
        if self.divergentes == 0 {
            0
        } else {
            2
        }
    }
}

/// Recorta el texto alrededor de un punto, en fronteras de carácter.
fn contexto(texto: &str, aproximado: usize) -> String {
    let inicio = texto
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i + 40 <= aproximado)
        .last()
        .unwrap_or(0);
    let fin = texto
        .char_indices()
        .map(|(i, _)| i)
        .find(|i| *i >= aproximado + 40)
        .unwrap_or(texto.len());
    texto[inicio..fin].to_string()
}

/// Compara un fixture contra un tokenizer.
pub fn comparar(fixture: &Fixture, tokenizer: &dyn Tokeniza) -> Comparacion {
    let obtenidos = tokenizer.encode(&fixture.texto);
    let vocab = tokenizer.vocab_size();

    let fuera: Vec<u32> = fixture
        .ids
        .iter()
        .copied()
        .filter(|id| *id >= vocab)
        .collect();

    let mut divergencia = None;
    let comunes = fixture.ids.len().min(obtenidos.len());
    for i in 0..comunes {
        if fixture.ids[i] == obtenidos[i] {
            continue;
        }
        let esperado = fixture.ids[i];
        let obtenido = obtenidos[i];
        let pieza_esperada = tokenizer.pieza(esperado);
        let pieza_obtenida = tokenizer.pieza(obtenido);
        let especial = fixture.especiales.values().any(|v| *v == esperado);
        let clase = if esperado >= vocab {
            Clase::FueraDeVocabulario
        } else if especial {
            Clase::TokenEspecial
        } else {
            match (&pieza_esperada, &pieza_obtenida) {
                (Some(a), Some(b)) if a == b => Clase::Conversion,
                (Some(a), Some(b))
                    if a.trim_start_matches(['\u{2581}', 'Ġ', ' '])
                        == b.trim_start_matches(['\u{2581}', 'Ġ', ' ']) =>
                {
                    Clase::Normalizacion
                }
                _ => Clase::Segmentacion,
            }
        };
        // Posición aproximada en bytes: los tokens anteriores ya consumieron
        // texto, y con eso basta para enseñar dónde pasa.
        let consumido: usize = obtenidos[..i]
            .iter()
            .filter_map(|id| tokenizer.pieza(*id).map(|p| p.len()))
            .sum();
        divergencia = Some(Divergencia {
            posicion: i,
            clase,
            esperado: Some(esperado),
            obtenido: Some(obtenido),
            pieza_esperada,
            pieza_obtenida,
            contexto: contexto(&fixture.texto, consumido),
            explicacion: format!(
                "en el token {i} la referencia dice {esperado} y soso dice {obtenido} ({})",
                clase.como_texto()
            ),
        });
        break;
    }

    if divergencia.is_none() && fixture.ids.len() != obtenidos.len() {
        divergencia = Some(Divergencia {
            posicion: comunes,
            clase: Clase::Longitud,
            esperado: fixture.ids.get(comunes).copied(),
            obtenido: obtenidos.get(comunes).copied(),
            pieza_esperada: None,
            pieza_obtenida: None,
            contexto: contexto(&fixture.texto, fixture.texto.len().saturating_sub(40)),
            explicacion: format!(
                "coinciden los primeros {comunes} tokens, pero la referencia tiene {} y soso {}",
                fixture.ids.len(),
                obtenidos.len()
            ),
        });
    }

    Comparacion {
        fixture: fixture.nombre.clone(),
        modelo: fixture.modelo.clone(),
        revision: fixture.revision.clone(),
        tokens_referencia: fixture.ids.len(),
        tokens_soso: obtenidos.len(),
        iguales: divergencia.is_none(),
        fuera_de_vocabulario: fuera,
        divergencia,
    }
}

pub fn leer_indice(arbol: &dyn Archivos, directorio: &str) -> Resultado<Indice> {
    let datos = arbol.leer(&unir(directorio, "index.json"))?;
    let indice: Indice = serde_json::from_slice(&datos).map_err(Error::formato)?;
    if indice.schema_version != crate::ESQUEMA {
        return Err(Error::formato(format!(
            "index.json: schema_version {} no soportada",
            indice.schema_version
        )));
    }
    Ok(indice)
}

pub fn leer_fixture(arbol: &dyn Archivos, directorio: &str, archivo: &str) -> Resultado<Fixture> {
    crate::ruta_segura(archivo)?;
    let datos = arbol.leer(&unir(directorio, archivo))?;
    let fixture: Fixture = serde_json::from_slice(&datos).map_err(Error::formato)?;
    // El fixture trae sus propios hashes: si el archivo se editó a mano, se
    // nota aquí y no tres pasos más allá.
    let texto = crate::sha256_hex(fixture.texto.as_bytes());
    if texto != fixture.texto_sha256 {
        return Err(Error::formato(format!(
            "{archivo}: el texto no cuadra con texto_sha256"
        )));
    }
    let mut crudo = Vec::with_capacity(fixture.ids.len() * 4);
    for id in &fixture.ids {
        crudo.extend_from_slice(&id.to_le_bytes());
    }
    if crate::sha256_hex(&crudo) != fixture.ids_sha256 {
        return Err(Error::formato(format!(
            "{archivo}: los ids no cuadran con ids_sha256"
        )));
    }
    Ok(fixture)
}

/// Compara todos los fixtures de un directorio.
pub fn comparar_todos(
    arbol: &dyn Archivos,
    directorio: &str,
    tokenizer: &dyn Tokeniza,
) -> Resultado<Informe> {
    let indice = leer_indice(arbol, directorio)?;
    let mut comparaciones = Vec::new();
    for entrada in &indice.fixtures {
        let fixture = leer_fixture(arbol, directorio, &entrada.archivo)?;
        if fixture.ids.len() != entrada.tokens {
            return Err(Error::formato(format!(
                "{}: el índice dice {} tokens y el fixture tiene {}",
                entrada.archivo,
                entrada.tokens,
                fixture.ids.len()
            )));
        }
        comparaciones.push(comparar(&fixture, tokenizer));
    }
    let iguales = comparaciones.iter().filter(|c| c.iguales).count();
    Ok(Informe {
        schema_version: crate::ESQUEMA,
        modelo: indice.modelo,
        revision: indice.revision,
        vocab_referencia: indice.vocab_size,
        vocab_soso: tokenizer.vocab_size(),
        divergentes: comparaciones.len() - iguales,
        iguales,
        comparaciones,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    /// Tokenizer de mentira: parte por espacios y numera por orden de aparición.
    struct Falso {
        piezas: Vec<String>,
        desplazamiento: u32,
    }

    impl Tokeniza for Falso {
        fn encode(&self, texto: &str) -> Vec<u32> {
            texto
                .split(' ')
                .filter(|p| !p.is_empty())
                .map(|p| {
                    self.piezas
                        .iter()
                        .position(|q| q == p)
                        .map(|i| i as u32 + self.desplazamiento)
                        .unwrap_or(0)
                })
                .collect()
        }
        fn vocab_size(&self) -> u32 {
            self.piezas.len() as u32 + self.desplazamiento
        }
        fn pieza(&self, id: u32) -> Option<String> {
            let i = id.checked_sub(self.desplazamiento)? as usize;
            self.piezas.get(i).cloned()
        }
    }

    fn fixture(texto: &str, ids: Vec<u32>) -> Fixture {
        let mut crudo = Vec::new();
        for id in &ids {
            crudo.extend_from_slice(&id.to_le_bytes());
        }
        Fixture {
            schema_version: 1,
            nombre: "prueba".into(),
            modelo: "m".into(),
            revision: "r".into(),
            texto_sha256: crate::sha256_hex(texto.as_bytes()),
            ids_sha256: crate::sha256_hex(&crudo),
            texto: texto.into(),
            ids,
            especiales: BTreeMap::new(),
        }
    }

    fn falso() -> Falso {
        Falso {
            piezas: alloc::vec!["hola".into(), "mundo".into(), "adios".into()],
            desplazamiento: 0,
        }
    }

    #[test]
    fn coincidencia_exacta() {
        let c = comparar(&fixture("hola mundo", alloc::vec![0, 1]), &falso());
        assert!(c.iguales, "{c:?}");
        assert!(c.divergencia.is_none());
        assert_eq!(c.tokens_referencia, 2);
    }

    #[test]
    fn divergencia_de_conversion() {
        // Mismas piezas, ids desplazados: es conversión de vocabulario.
        let tok = Falso {
            piezas: alloc::vec!["hola".into(), "mundo".into()],
            desplazamiento: 100,
        };
        let c = comparar(&fixture("hola mundo", alloc::vec![0, 1]), &tok);
        let d = c.divergencia.expect("debe divergir");
        assert_eq!(d.posicion, 0);
        assert_eq!(d.esperado, Some(0));
        assert_eq!(d.obtenido, Some(100));
    }

    #[test]
    fn divergencia_de_longitud() {
        let c = comparar(&fixture("hola mundo adios", alloc::vec![0, 1]), &falso());
        let d = c.divergencia.expect("debe divergir");
        assert_eq!(d.clase, Clase::Longitud);
        assert_eq!(c.tokens_soso, 3);
        assert_eq!(c.tokens_referencia, 2);
    }

    #[test]
    fn ids_fuera_del_vocabulario_se_listan() {
        let c = comparar(&fixture("hola mundo", alloc::vec![0, 9999]), &falso());
        assert_eq!(c.fuera_de_vocabulario, alloc::vec![9999]);
        let d = c.divergencia.expect("debe divergir");
        assert_eq!(d.clase, Clase::FueraDeVocabulario);
    }

    #[test]
    fn un_token_especial_se_clasifica_como_tal() {
        let mut f = fixture("hola mundo", alloc::vec![2, 1]);
        f.especiales.insert("<|im_start|>".into(), 2);
        let c = comparar(&f, &falso());
        let d = c.divergencia.expect("debe divergir");
        assert_eq!(d.clase, Clase::TokenEspecial);
    }
}
