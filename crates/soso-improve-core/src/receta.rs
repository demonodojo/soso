//! Receta declarativa del bootstrap de libstd para soso (T39).
//!
//! Hoy esto lo hace `config/rust-soso/apply-patches.sh` con `rsync`, `sed -i`
//! y `perl -i`. Dos cosas lo hacen inservible como base del circuito nativo:
//!
//! 1. **Nada de eso existe en soso.** El objetivo del plan es que la
//!    preparación pueda correr dentro, así que la política vive aquí y el
//!    mecanismo lo pone quien hospeda, como el resto del crate.
//! 2. **`sed -i` y `perl -i` salen 0 cuando el ancla no está.** Comprobado:
//!    `sed -i 's/ancla-que-no-existe/x/' f` devuelve 0 y deja el fichero
//!    igual. Así que si un cambio de upstream renombra cualquier ancla, el
//!    script imprime «apply-patches: OK» habiendo parcheado **cero**, y el
//!    fallo aparece mucho después como un error de compilación que no señala
//!    a la causa.
//!
//! Aquí un ancla que no aparece es un **error del paso, con su nombre**.
//!
//! La otra mitad es la idempotencia. El script la consigue con
//! `grep -q '<subcadena>' || parchear`, y la subcadena es floja: `target_os =
//! "soso"` puede aparecer en un fichero por otro motivo y saltarse el parche.
//! Cada paso de esta receta lleva una **marca** distintiva, que es lo que se
//! busca para decidir si ya está aplicado.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::entorno::{Archivos, Entrada};

/// Qué hacer con un fichero del vendor.
#[derive(Debug, Clone)]
pub enum Accion {
    /// Copiar bytes tal cual (lo que hacía `cp`).
    Copiar { desde: String },
    /// Copiar un árbol entero, recursivo, sin borrar lo que sobre.
    ///
    /// El bootstrap de los directorios que el overlay posee enteros usa
    /// [`Accion::Espejar`]: allí un resto sí se retira. Este variante se
    /// conserva para copias que no pueden llevarse por delante ficheros ajenos.
    CopiarArbol { desde: String },
    /// Espejo de un directorio poseído entero (`rsync -a --delete`).
    ///
    /// Borra en el destino lo que el origen ya no tiene. `conservar` son
    /// nombres de primer nivel que otro paso repondrá. No se usa fuera de
    /// esos directorios: un `find` suelto no puede borrar módulos de upstream.
    Espejar { desde: String, conservar: Vec<String> },
    /// Copia cada `*.rs` bajo `desde` que no caiga en `excluir`, sin borrar
    /// nada en el destino. Es el `find` del script: los ficheros viven dentro
    /// de directorios de rust y un `--delete` se llevaría el std original.
    CopiarSueltos { desde: String, excluir: Vec<String> },
    /// Insertar `texto` **después** de la línea que contiene `ancla`.
    InsertarTrasLinea { ancla: String, texto: String },
    /// Insertar `texto` **antes** de la línea que contiene `ancla`.
    InsertarAntesDeLinea { ancla: String, texto: String },
    /// Añadir `texto` al final del fichero.
    Anadir { texto: String },
    /// Sustituye la primera aparición de `busca`. Si no está y la marca
    /// tampoco, el ancla se rompió: es un fallo con ruta y nombre.
    Sustituir { busca: String, reemplazo: String },
    /// Como [`Accion::Sustituir`], pero si ni la marca ni `busca` están no
    /// había un estado viejo que reparar: no es un ancla rota.
    Reparar { busca: String, reemplazo: String },
    /// Quita las líneas cuyo texto es exactamente `linea`. Que no estén es
    /// el estado aplicado: la marca no se busca como presencia.
    QuitarLinea { linea: String },
    /// Dependencia cuyo texto cambia. La marca es la línea exacta. Si el
    /// fichero no contiene `presente`, se añade `bloque`. Si lo contiene pero
    /// la línea que empieza por `prefijo` no es la marca, se reescribe: una
    /// subcadena floja dejaría la versión vieja para siempre.
    AsegurarLinea {
        presente: String,
        prefijo: String,
        linea: String,
        bloque: String,
    },
}

/// Un paso de la receta: qué fichero, qué hacer y cómo saber si ya está hecho.
#[derive(Debug, Clone)]
pub struct Paso {
    /// Nombre para los informes. Es lo que se lee cuando algo falla.
    pub nombre: String,
    /// Ruta relativa a la raíz del vendor.
    pub fichero: String,
    /// Presencia de esta cadena = el paso ya está aplicado.
    ///
    /// Tiene que ser **distintiva**: si se elige algo que el fichero puede
    /// contener por otro motivo, el paso se salta y nadie se entera.
    pub marca: String,
    pub accion: Accion,
}

/// Qué pasó con un paso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resultado {
    /// Se aplicó ahora.
    Aplicado,
    /// Ya estaba: la marca estaba presente.
    YaEstaba,
    /// Sólo en [`comprobar`]: falta por aplicar, y se puede.
    Falta,
    /// No se pudo, y por qué.
    Fallo(String),
}

/// Informe de un paso, para que un fallo diga **cuál**.
#[derive(Debug, Clone)]
pub struct Informe {
    pub nombre: String,
    pub fichero: String,
    pub resultado: Resultado,
}

impl Informe {
    pub fn ok(&self) -> bool {
        !matches!(self.resultado, Resultado::Fallo(_))
    }
}

/// La revisión del vendor tiene que ser la que fija el lock de T38.
///
/// Se compara aquí en vez de mirar `.git` porque este crate no puede hablar
/// con git —ni existirá en soso—: quien hospeda observa la revisión y la
/// pasa. Separa la política (¿coincide?) del mecanismo (¿cómo se averigua?).
pub fn verificar_revision(esperada: &str, observada: &str) -> Result<(), String> {
    if esperada.is_empty() {
        return Err("el lock no fija revisión".to_string());
    }
    if esperada == observada {
        return Ok(());
    }
    Err(format!(
        "el vendor está en {} y el lock fija {}",
        corto(observada),
        corto(esperada)
    ))
}

fn corto(rev: &str) -> &str {
    if rev.len() > 12 { &rev[..12] } else { rev }
}

/// Dice qué pasos faltan **sin escribir nada**.
///
/// Existe porque el vendor puede quedarse a medias sin que nadie lo note: al
/// escribir esta ficha, `library/std/src/os/mod.rs` no tenía ni rastro de soso
/// mientras los otros seis parches sí estaban, y no había forma de verlo salvo
/// mirando fichero por fichero. Un `sed` que falla en silencio —o un script
/// que aborta a la mitad— deja exactamente ese estado.
///
/// Un ancla ausente se informa igual que en [`aplicar`]: es un fallo con
/// nombre, no un «falta».
pub fn comprobar<A: Archivos>(fs: &A, raiz: &str, pasos: &[Paso]) -> Vec<Informe> {
    pasos
        .iter()
        .map(|p| Informe {
            nombre: p.nombre.clone(),
            fichero: p.fichero.clone(),
            resultado: comprobar_uno(fs, raiz, p),
        })
        .collect()
}

fn comprobar_uno<A: Archivos>(fs: &A, raiz: &str, paso: &Paso) -> Resultado {
    let destino = unir(raiz, &paso.fichero);
    match &paso.accion {
        // Para las copias, «aplicado» es que el contenido ya esté.
        Accion::Copiar { desde } => match (fs.leer(desde), fs.leer(&destino)) {
            (Ok(origen), Ok(actual)) if origen == actual => Resultado::YaEstaba,
            (Ok(_), _) => Resultado::Falta,
            (Err(e), _) => Resultado::Fallo(format!("no pude leer la plantilla {desde}: {e:?}")),
        },
        // Un directorio que existe con bytes viejos no está aplicado. Mirar
        // sólo `existe` daba `YaEstaba` y el resto se quedaba para siempre.
        Accion::CopiarArbol { desde } => estado_copia(fs, desde, &destino, false, &[]),
        Accion::Espejar { desde, conservar } => {
            estado_copia(fs, desde, &destino, true, conservar)
        }
        Accion::CopiarSueltos { desde, excluir } => estado_sueltos(fs, desde, raiz, excluir),
        Accion::QuitarLinea { linea } => match leer_texto(fs, &destino) {
            Err(r) => r,
            Ok(texto) => {
                if texto.lines().any(|l| l == linea) {
                    Resultado::Falta
                } else {
                    Resultado::YaEstaba
                }
            }
        },
        Accion::Reparar { busca, .. } => match leer_texto(fs, &destino) {
            Err(r) => r,
            Ok(texto) => {
                if texto.contains(&paso.marca) {
                    Resultado::YaEstaba
                } else if texto.contains(busca) {
                    Resultado::Falta
                } else {
                    Resultado::YaEstaba
                }
            }
        },
        Accion::Sustituir { busca, .. } => match leer_texto(fs, &destino) {
            Err(r) => r,
            Ok(texto) => {
                if texto.contains(&paso.marca) {
                    Resultado::YaEstaba
                } else if texto.contains(busca) {
                    Resultado::Falta
                } else {
                    Resultado::Fallo(format!(
                        "no encontré el ancla {busca:?} en {}",
                        paso.fichero
                    ))
                }
            }
        },
        Accion::AsegurarLinea { presente, prefijo, .. } => match leer_texto(fs, &destino) {
            Err(r) => r,
            Ok(texto) => {
                if texto.contains(&paso.marca) {
                    Resultado::YaEstaba
                } else if texto.contains(presente) {
                    if texto.lines().any(|l| l.starts_with(prefijo.as_str())) {
                        Resultado::Falta
                    } else {
                        Resultado::Fallo(format!(
                            "hay {:?} pero no la línea {prefijo:?} en {}",
                            presente, paso.fichero
                        ))
                    }
                } else {
                    Resultado::Falta
                }
            }
        },
        _ => {
            let Ok(bytes) = fs.leer(&destino) else {
                return Resultado::Fallo(format!("no pude leer {destino}"));
            };
            let Ok(texto) = String::from_utf8(bytes) else {
                return Resultado::Fallo(format!("{destino} no es UTF-8"));
            };
            if texto.contains(&paso.marca) {
                return Resultado::YaEstaba;
            }
            let ancla = match &paso.accion {
                Accion::InsertarTrasLinea { ancla, .. }
                | Accion::InsertarAntesDeLinea { ancla, .. } => Some(ancla),
                _ => None,
            };
            match ancla {
                Some(a) if !texto.contains(a.as_str()) => Resultado::Fallo(format!(
                    "no encontré el ancla {a:?} en {}",
                    paso.fichero
                )),
                _ => Resultado::Falta,
            }
        }
    }
}

/// Aplica la receta entera. Devuelve un informe por paso, **en orden**.
///
/// No se para en el primer fallo: un informe que sólo cuenta el primer
/// problema obliga a repetir el ciclo entero por cada uno. El llamante decide
/// con [`Informe::ok`].
pub fn aplicar<A: Archivos>(fs: &mut A, raiz: &str, pasos: &[Paso]) -> Vec<Informe> {
    pasos
        .iter()
        .map(|p| Informe {
            nombre: p.nombre.clone(),
            fichero: p.fichero.clone(),
            resultado: aplicar_uno(fs, raiz, p),
        })
        .collect()
}

fn unir(raiz: &str, rel: &str) -> String {
    if raiz.is_empty() || raiz == "/" {
        format!("/{}", rel.trim_start_matches('/'))
    } else {
        format!("{}/{}", raiz.trim_end_matches('/'), rel.trim_start_matches('/'))
    }
}

fn aplicar_uno<A: Archivos>(fs: &mut A, raiz: &str, paso: &Paso) -> Resultado {
    let destino = unir(raiz, &paso.fichero);

    // `Copiar` es el único paso que puede crear el fichero; los demás editan
    // uno que tiene que existir, y que no exista es un fallo con nombre y no
    // un silencio.
    if let Accion::Copiar { desde } = &paso.accion {
        let datos = match fs.leer(desde) {
            Ok(d) => d,
            Err(e) => return Resultado::Fallo(format!("no pude leer la plantilla {desde}: {e:?}")),
        };
        if let Ok(actual) = fs.leer(&destino) {
            if actual == datos {
                return Resultado::YaEstaba;
            }
        }
        return match fs.escribir(&destino, &datos, 0o644) {
            Ok(()) => Resultado::Aplicado,
            Err(e) => Resultado::Fallo(format!("no pude escribir {destino}: {e:?}")),
        };
    }

    if let Accion::CopiarArbol { desde } = &paso.accion {
        let mut copiados = 0usize;
        return match copiar_arbol(fs, desde, &destino, &mut copiados) {
            Ok(()) if copiados == 0 => Resultado::YaEstaba,
            Ok(()) => Resultado::Aplicado,
            Err(e) => Resultado::Fallo(e),
        };
    }

    if let Accion::Espejar { desde, conservar } = &paso.accion {
        let mut copiados = 0usize;
        return match espejar(fs, desde, &destino, conservar, &mut copiados) {
            Ok(()) if copiados == 0 => Resultado::YaEstaba,
            Ok(()) => Resultado::Aplicado,
            Err(e) => Resultado::Fallo(e),
        };
    }

    if let Accion::CopiarSueltos { desde, excluir } = &paso.accion {
        let mut copiados = 0usize;
        return match copiar_sueltos(fs, desde, raiz, excluir, &mut copiados) {
            Ok(()) if copiados == 0 => Resultado::YaEstaba,
            Ok(()) => Resultado::Aplicado,
            Err(e) => Resultado::Fallo(e),
        };
    }

    let bytes = match fs.leer(&destino) {
        Ok(d) => d,
        Err(e) => return Resultado::Fallo(format!("no pude leer {destino}: {e:?}")),
    };
    let Ok(texto) = String::from_utf8(bytes) else {
        return Resultado::Fallo(format!("{destino} no es UTF-8"));
    };
    if !matches!(paso.accion, Accion::QuitarLinea { .. }) && texto.contains(&paso.marca) {
        return Resultado::YaEstaba;
    }

    let nuevo = match &paso.accion {
        Accion::Copiar { .. }
        | Accion::CopiarArbol { .. }
        | Accion::Espejar { .. }
        | Accion::CopiarSueltos { .. } => {
            unreachable!("tratados arriba")
        }
        Accion::Anadir { texto: t } => {
            let mut s = texto.clone();
            if !s.ends_with('\n') {
                s.push('\n');
            }
            s.push_str(t);
            s
        }
        Accion::InsertarTrasLinea { ancla, texto: t } => {
            match insertar(&texto, ancla, t, true) {
                Some(s) => s,
                None => {
                    return Resultado::Fallo(format!(
                        "no encontré el ancla {ancla:?} en {}",
                        paso.fichero
                    ));
                }
            }
        }
        Accion::InsertarAntesDeLinea { ancla, texto: t } => {
            match insertar(&texto, ancla, t, false) {
                Some(s) => s,
                None => {
                    return Resultado::Fallo(format!(
                        "no encontré el ancla {ancla:?} en {}",
                        paso.fichero
                    ));
                }
            }
        }
        Accion::Sustituir { busca, reemplazo } => match sustituir_primera(&texto, busca, reemplazo)
        {
            Some(s) => s,
            None => {
                return Resultado::Fallo(format!(
                    "no encontré el ancla {busca:?} en {}",
                    paso.fichero
                ));
            }
        },
        Accion::Reparar { busca, reemplazo } => match sustituir_primera(&texto, busca, reemplazo) {
            Some(s) => s,
            None => return Resultado::YaEstaba,
        },
        Accion::QuitarLinea { linea } => match quitar_linea(&texto, linea) {
            Some(s) => s,
            None => return Resultado::YaEstaba,
        },
        Accion::AsegurarLinea {
            presente,
            prefijo,
            linea,
            bloque,
        } => {
            if texto.contains(presente) {
                match reescribir_prefijo(&texto, prefijo, linea) {
                    Some(s) => s,
                    None => {
                        return Resultado::Fallo(format!(
                            "hay {presente:?} pero no la línea {prefijo:?} en {}",
                            paso.fichero
                        ));
                    }
                }
            } else {
                let mut s = texto.clone();
                s.push_str(bloque);
                s
            }
        }
    };

    match fs.escribir(&destino, nuevo.as_bytes(), 0o644) {
        Ok(()) => Resultado::Aplicado,
        Err(e) => Resultado::Fallo(format!("no pude escribir {destino}: {e:?}")),
    }
}

/// Copia `desde` en `hasta`, recursivo. Cuenta cuántos ficheros **cambiaron**.
///
/// Se compara el contenido antes de escribir para que una segunda preparación
/// no toque nada: la idempotencia de un árbol no es una marca en un fichero,
/// es que los bytes ya estén.
fn copiar_arbol<A: Archivos>(
    fs: &mut A,
    desde: &str,
    hasta: &str,
    copiados: &mut usize,
) -> Result<(), String> {
    let entradas = fs
        .listar(desde)
        .map_err(|e| format!("no pude listar la plantilla {desde}: {e:?}"))?;
    if fs.crear_directorio(hasta).is_err() {
        return Err(format!("no pude crear {hasta}"));
    }
    for e in entradas {
        let nombre = e.ruta.rsplit('/').next().unwrap_or(&e.ruta).to_string();
        let origen = format!("{}/{}", desde.trim_end_matches('/'), nombre);
        let destino = format!("{}/{}", hasta.trim_end_matches('/'), nombre);
        match e.tipo {
            crate::entorno::Tipo::Directorio => copiar_arbol(fs, &origen, &destino, copiados)?,
            _ => {
                let datos = fs
                    .leer(&origen)
                    .map_err(|err| format!("no pude leer {origen}: {err:?}"))?;
                if fs.leer(&destino).map(|a| a == datos).unwrap_or(false) {
                    continue;
                }
                fs.escribir(&destino, &datos, 0o644)
                    .map_err(|err| format!("no pude escribir {destino}: {err:?}"))?;
                *copiados += 1;
            }
        }
    }
    Ok(())
}

fn nombre_de(e: &Entrada) -> String {
    e.ruta.rsplit('/').next().unwrap_or(&e.ruta).to_string()
}

fn excluido(rel: &str, excluir: &[String]) -> bool {
    excluir.iter().any(|p| rel == p || rel.starts_with(&format!("{p}/")))
}

fn conservado(rel: &str, conservar: &[String]) -> bool {
    conservar.iter().any(|c| rel == c)
}

/// Lista ficheros bajo `abs`. `solo_rs` imita el `find -name '*.rs'` del script.
fn recolectar<A: Archivos>(
    fs: &A,
    abs: &str,
    rel: &str,
    excluir: &[String],
    solo_rs: bool,
    out: &mut Vec<String>,
) -> Result<(), String> {
    if !rel.is_empty() && excluido(rel, excluir) {
        return Ok(());
    }
    let entradas = fs
        .listar(abs)
        .map_err(|e| format!("no pude listar {abs}: {e:?}"))?;
    for e in entradas {
        let nombre = nombre_de(&e);
        if nombre == "." || nombre == ".." {
            continue;
        }
        let rel_hijo = if rel.is_empty() {
            nombre.clone()
        } else {
            format!("{rel}/{nombre}")
        };
        if excluido(&rel_hijo, excluir) {
            continue;
        }
        let abs_hijo = format!("{}/{}", abs.trim_end_matches('/'), nombre);
        match e.tipo {
            crate::entorno::Tipo::Directorio => {
                recolectar(fs, &abs_hijo, &rel_hijo, excluir, solo_rs, out)?;
            }
            crate::entorno::Tipo::Archivo if !solo_rs || nombre.ends_with(".rs") => {
                out.push(rel_hijo);
            }
            _ => {}
        }
    }
    Ok(())
}

fn leer_texto<A: Archivos>(fs: &A, ruta: &str) -> Result<String, Resultado> {
    let bytes = match fs.leer(ruta) {
        Ok(b) => b,
        Err(_) => return Err(Resultado::Fallo(format!("no pude leer {ruta}"))),
    };
    String::from_utf8(bytes).map_err(|_| Resultado::Fallo(format!("{ruta} no es UTF-8")))
}

/// `Ok(true)` si el destino ya tiene los bytes del origen.
fn diff_arbol<A: Archivos>(
    fs: &A,
    desde: &str,
    hasta: &str,
    borrar_ajenos: bool,
    conservar: &[String],
) -> Result<bool, String> {
    let mut origen = Vec::new();
    recolectar(fs, desde, "", &[], false, &mut origen)?;
    if fs.listar(hasta).is_err() {
        return Ok(false);
    }
    for rel in &origen {
        let o = format!("{}/{}", desde.trim_end_matches('/'), rel);
        let d = format!("{}/{}", hasta.trim_end_matches('/'), rel);
        match (fs.leer(&o), fs.leer(&d)) {
            (Ok(a), Ok(b)) if a == b => {}
            (Ok(_), _) => return Ok(false),
            (Err(e), _) => return Err(format!("no pude leer {o}: {e:?}")),
        }
    }
    if borrar_ajenos {
        let mut dest = Vec::new();
        recolectar(fs, hasta, "", &[], false, &mut dest)?;
        for rel in dest {
            if conservado(&rel, conservar) || origen.iter().any(|o| o == &rel) {
                continue;
            }
            return Ok(false);
        }
    }
    Ok(true)
}

fn estado_copia<A: Archivos>(
    fs: &A,
    desde: &str,
    hasta: &str,
    borrar_ajenos: bool,
    conservar: &[String],
) -> Resultado {
    match diff_arbol(fs, desde, hasta, borrar_ajenos, conservar) {
        Ok(true) => Resultado::YaEstaba,
        Ok(false) => Resultado::Falta,
        Err(e) => Resultado::Fallo(e),
    }
}

fn estado_sueltos<A: Archivos>(fs: &A, desde: &str, raiz: &str, excluir: &[String]) -> Resultado {
    let mut rels = Vec::new();
    if let Err(e) = recolectar(fs, desde, "", excluir, true, &mut rels) {
        return Resultado::Fallo(e);
    }
    for rel in rels {
        let o = format!("{}/{}", desde.trim_end_matches('/'), rel);
        let d = unir(raiz, &rel);
        match (fs.leer(&o), fs.leer(&d)) {
            (Ok(a), Ok(b)) if a == b => {}
            (Ok(_), _) => return Resultado::Falta,
            (Err(e), _) => return Resultado::Fallo(format!("no pude leer {o}: {e:?}")),
        }
    }
    Resultado::YaEstaba
}

fn espejar<A: Archivos>(
    fs: &mut A,
    desde: &str,
    hasta: &str,
    conservar: &[String],
    cambiados: &mut usize,
) -> Result<(), String> {
    copiar_arbol(fs, desde, hasta, cambiados)?;
    quitar_ajenos(fs, desde, hasta, conservar, cambiados)
}

fn quitar_ajenos<A: Archivos>(
    fs: &mut A,
    desde: &str,
    hasta: &str,
    conservar: &[String],
    cambiados: &mut usize,
) -> Result<(), String> {
    let origen = fs
        .listar(desde)
        .map_err(|e| format!("no pude listar la plantilla {desde}: {e:?}"))?;
    let mut nombres = BTreeSet::new();
    for e in origen {
        nombres.insert(nombre_de(&e));
    }
    let destino = fs
        .listar(hasta)
        .map_err(|e| format!("no pude listar {hasta}: {e:?}"))?;
    for e in destino {
        let nombre = nombre_de(&e);
        if conservar.iter().any(|c| c == &nombre) || nombres.contains(&nombre) {
            if nombres.contains(&nombre) && e.tipo == crate::entorno::Tipo::Directorio {
                let sub_o = format!("{}/{}", desde.trim_end_matches('/'), nombre);
                let sub_d = format!("{}/{}", hasta.trim_end_matches('/'), nombre);
                quitar_ajenos(fs, &sub_o, &sub_d, &[], cambiados)?;
            }
            continue;
        }
        let ruta = format!("{}/{}", hasta.trim_end_matches('/'), nombre);
        fs.borrar(&ruta)
            .map_err(|err| format!("no pude borrar {ruta}: {err:?}"))?;
        *cambiados += 1;
    }
    Ok(())
}

fn copiar_sueltos<A: Archivos>(
    fs: &mut A,
    desde: &str,
    raiz: &str,
    excluir: &[String],
    copiados: &mut usize,
) -> Result<(), String> {
    let mut rels = Vec::new();
    recolectar(fs, desde, "", excluir, true, &mut rels)?;
    for rel in rels {
        let origen = format!("{}/{}", desde.trim_end_matches('/'), rel);
        let destino = unir(raiz, &rel);
        let datos = fs
            .leer(&origen)
            .map_err(|e| format!("no pude leer {origen}: {e:?}"))?;
        if fs.leer(&destino).map(|a| a == datos).unwrap_or(false) {
            continue;
        }
        fs.escribir(&destino, &datos, 0o644)
            .map_err(|e| format!("no pude escribir {destino}: {e:?}"))?;
        *copiados += 1;
    }
    Ok(())
}

fn sustituir_primera(fuente: &str, busca: &str, reemplazo: &str) -> Option<String> {
    let i = fuente.find(busca)?;
    let mut out = String::with_capacity(fuente.len() + reemplazo.len());
    out.push_str(&fuente[..i]);
    out.push_str(reemplazo);
    out.push_str(&fuente[i + busca.len()..]);
    Some(out)
}

fn quitar_linea(fuente: &str, linea: &str) -> Option<String> {
    let mut out = String::with_capacity(fuente.len());
    let mut quitada = false;
    for l in fuente.split_inclusive('\n') {
        if l.trim_end_matches(['\n', '\r']) == linea {
            quitada = true;
            continue;
        }
        out.push_str(l);
    }
    quitada.then_some(out)
}

fn reescribir_prefijo(fuente: &str, prefijo: &str, linea: &str) -> Option<String> {
    let mut out = String::with_capacity(fuente.len() + linea.len());
    let mut hecho = false;
    for l in fuente.split_inclusive('\n') {
        if !hecho && l.trim_end_matches(['\n', '\r']).starts_with(prefijo) {
            hecho = true;
            out.push_str(linea);
            out.push('\n');
        } else {
            out.push_str(l);
        }
    }
    hecho.then_some(out)
}

/// Inserta `texto` junto a la **primera** línea que contiene `ancla`.
///
/// La primera y no todas: los parches de este bootstrap añaden una rama a un
/// `match`, y hacerlo dos veces rompería la compilación de una forma mucho
/// menos evidente que no hacerlo ninguna.
fn insertar(fuente: &str, ancla: &str, texto: &str, despues: bool) -> Option<String> {
    let mut out = String::with_capacity(fuente.len() + texto.len() + 1);
    let mut puesto = false;
    for linea in fuente.split_inclusive('\n') {
        if !puesto && linea.contains(ancla) {
            puesto = true;
            if despues {
                out.push_str(linea);
                empujar_linea(&mut out, texto);
            } else {
                empujar_linea(&mut out, texto);
                out.push_str(linea);
            }
        } else {
            out.push_str(linea);
        }
    }
    puesto.then_some(out)
}

fn empujar_linea(out: &mut String, texto: &str) {
    out.push_str(texto);
    if !texto.ends_with('\n') {
        out.push('\n');
    }
}

/// Texto canónico de la receta, estable, para que T39 fije su huella.
pub fn inventario(pasos: &[Paso]) -> String {
    let mut s = String::new();
    for p in pasos {
        s.push_str(&p.nombre);
        s.push('\t');
        s.push_str(&p.fichero);
        s.push('\t');
        s.push_str(&p.marca);
        s.push('\t');
        s.push_str(&accion_canon(&p.accion));
        s.push('\n');
    }
    s
}

/// SHA-256 del [inventario]. Cambia cuando cambia un ancla, una marca o un texto.
pub fn huella(pasos: &[Paso]) -> String {
    crate::sha256_hex(inventario(pasos).as_bytes())
}

fn accion_canon(a: &Accion) -> String {
    match a {
        Accion::Copiar { desde } => format!("copiar {desde}"),
        Accion::CopiarArbol { desde } => format!("copiar-arbol {desde}"),
        Accion::Espejar { desde, conservar } => {
            format!("espejar {desde} conservar={}", conservar.join(","))
        }
        Accion::CopiarSueltos { desde, excluir } => {
            format!("sueltos {desde} excluir={}", excluir.join(","))
        }
        Accion::InsertarTrasLinea { ancla, texto } => format!("tras {ancla:?} {texto:?}"),
        Accion::InsertarAntesDeLinea { ancla, texto } => format!("antes {ancla:?} {texto:?}"),
        Accion::Anadir { texto } => format!("anadir {texto:?}"),
        Accion::Sustituir { busca, reemplazo } => format!("sustituir {busca:?} {reemplazo:?}"),
        Accion::Reparar { busca, reemplazo } => format!("reparar {busca:?} {reemplazo:?}"),
        Accion::QuitarLinea { linea } => format!("quitar {linea:?}"),
        Accion::AsegurarLinea {
            presente,
            prefijo,
            linea,
            bloque,
        } => format!("asegurar {presente:?} {prefijo:?} {linea:?} {bloque:?}"),
    }
}

/// Bloque largo de `env_consts`: las cinco constantes de DLL/EXE vacías.
const ENV_LARGO: &str = "\
#[cfg(target_os = \"soso\")]
pub mod os {
    pub const FAMILY: &str = \"unix\";
    pub const OS: &str = \"soso\";
    pub const DLL_PREFIX: &str = \"\";
    pub const DLL_SUFFIX: &str = \"\";
    pub const DLL_EXTENSION: &str = \"\";
    pub const EXE_SUFFIX: &str = \"\";
    pub const EXE_EXTENSION: &str = \"\";
}";

const ENV_CORTO: &str = "\
#[cfg(target_os = \"soso\")]
pub mod os {
    pub const FAMILY: &str = \"unix\";
    pub const OS: &str = \"soso\";
}";

/// Distintivo del bloque largo: el corto no tiene DLL_PREFIX.
const ENV_MARCA: &str = "\
pub const OS: &str = \"soso\";
    pub const DLL_PREFIX: &str = \"\";";

const OS_MOD_MALO: &str = "\
#[cfg(target_os = \"hermit\")]
#[cfg(target_os = \"soso\")]
pub mod soso;
pub mod hermit;";

const OS_MOD_BUENO: &str = "\
#[cfg(target_os = \"soso\")]
pub mod soso;
#[cfg(target_os = \"hermit\")]
pub mod hermit;";

fn paso(nombre: &str, fichero: &str, marca: &str, accion: Accion) -> Paso {
    Paso {
        nombre: nombre.into(),
        fichero: fichero.into(),
        marca: marca.into(),
        accion,
    }
}

fn rama_antes(ancla: &str, texto: &str) -> Accion {
    Accion::InsertarAntesDeLinea {
        ancla: ancla.into(),
        texto: texto.into(),
    }
}

/// La receta del bootstrap de libstd, traducida de
/// `config/rust-soso/apply-patches.sh`.
///
/// `plantillas` es `config/rust-soso` del checkout y `soso_rt` la ruta que va
/// en el `Cargo.toml` del vendor. Las rutas de los ficheros son **relativas a
/// la raíz del vendor**.
///
/// La marca de cada edición es el texto vigente, no una subcadena que una
/// versión vieja también satisfaga. Las reparaciones (módulo `os` mal
/// guardado, bloque corto de `env_consts`, dependencia `dep-of-std`) convergen
/// sin dar por bueno un parche obsoleto.
pub fn pasos_libstd(plantillas: &str, soso_rt: &str) -> Vec<Paso> {
    let t = plantillas.trim_end_matches('/');
    let dep = format!(
        "soso-rt = {{ path = \"{soso_rt}\", features = [\"rustc-dep-of-std\"], public = true }}"
    );
    let bloque_dep = format!("\n[target.'cfg(target_os = \"soso\")'.dependencies]\n{dep}\n");
    let excluir = alloc::vec![
        "library/std/src/os/soso".into(),
        "library/std/src/sys/pal/soso".into(),
    ];
    alloc::vec![
        paso(
            "PAL: os/soso",
            "library/std/src/os/soso",
            "",
            Accion::Espejar {
                desde: format!("{t}/tree/library/std/src/os/soso"),
                conservar: Vec::new(),
            },
        ),
        paso(
            "PAL: sys/pal/soso",
            "library/std/src/sys/pal/soso",
            "",
            Accion::Espejar {
                desde: format!("{t}/tree/library/std/src/sys/pal/soso"),
                conservar: Vec::new(),
            },
        ),
        paso(
            "módulos sueltos del árbol",
            "library",
            "",
            Accion::CopiarSueltos {
                desde: format!("{t}/tree"),
                excluir,
            },
        ),
        paso(
            "build.rs: target soso",
            "library/std/build.rs",
            "target_os == \"soso\"",
            Accion::InsertarTrasLinea {
                ancla: "|| target_os == \"vexos\"".into(),
                texto: "        || target_os == \"soso\"".into(),
            },
        ),
        paso(
            "sys/pal/mod.rs: rama soso",
            "library/std/src/sys/pal/mod.rs",
            "mod soso;",
            Accion::InsertarTrasLinea {
                ancla: "pub use self::zkvm::*;".into(),
                texto: "    }\n    target_os = \"soso\" => {\n        mod soso;\n        pub use self::soso::*;"
                    .into(),
            },
        ),
        paso(
            "os/mod.rs: reparar cfg doble",
            "library/std/src/os/mod.rs",
            OS_MOD_BUENO,
            Accion::Reparar {
                busca: OS_MOD_MALO.into(),
                reemplazo: OS_MOD_BUENO.into(),
            },
        ),
        paso(
            "os/mod.rs: pub mod soso",
            "library/std/src/os/mod.rs",
            "pub mod soso;",
            rama_antes(
                "#[cfg(target_os = \"hermit\")]",
                "#[cfg(target_os = \"soso\")]\npub mod soso;",
            ),
        ),
        // El parche viejo metía soso en `pub mod fd`. El script lo retira.
        paso(
            "os/mod.rs: quitar soso de fd",
            "library/std/src/os/mod.rs",
            "    target_os = \"soso\",",
            Accion::QuitarLinea {
                linea: "    target_os = \"soso\",".into(),
            },
        ),
        paso(
            "sys/exit.rs: salida por soso_rt",
            "library/std/src/sys/exit.rs",
            "soso_rt::exit",
            Accion::InsertarTrasLinea {
                ancla: "hermit_abi::exit(code)".into(),
                texto: "        target_os = \"soso\" => soso_rt::exit(code),".into(),
            },
        ),
        paso(
            "Cargo.toml: dependencia soso-rt",
            "library/std/Cargo.toml",
            &dep,
            Accion::AsegurarLinea {
                presente: "target_os = \"soso\"".into(),
                prefijo: "soso-rt = {".into(),
                linea: dep.clone(),
                bloque: bloque_dep,
            },
        ),
        paso(
            "env_consts.rs: reparar bloque corto",
            "library/std/src/sys/env_consts.rs",
            ENV_MARCA,
            Accion::Reparar {
                busca: ENV_CORTO.into(),
                reemplazo: ENV_LARGO.into(),
            },
        ),
        paso(
            "env_consts.rs: constantes de soso",
            "library/std/src/sys/env_consts.rs",
            ENV_MARCA,
            rama_antes(
                "#[cfg(target_os = \"hermit\")]",
                &format!("{ENV_LARGO}\n\n"),
            ),
        ),
        paso(
            "sys/io/error: rama soso",
            "library/std/src/sys/io/error/mod.rs",
            "mod soso;",
            rama_antes(
                "    target_os = \"hermit\" => {",
                "    target_os = \"soso\" => {\n        mod soso;\n        pub use soso::*;\n    }",
            ),
        ),
        paso(
            "sys/alloc: módulo soso",
            "library/std/src/sys/alloc/mod.rs",
            "use soso as imp",
            rama_antes(
                "    target_os = \"hermit\" => {",
                "    target_os = \"soso\" => {\n        mod soso;\n        use soso as imp;\n    }",
            ),
        ),
        paso(
            "sys/alloc: alloc_zeroed",
            "library/std/src/sys/alloc/mod.rs",
            "target_os = \"hermit\", target_os = \"soso\", target_os = \"solid_asp3\"",
            Accion::Sustituir {
                busca: "any(target_os = \"hermit\", target_os = \"solid_asp3\"".into(),
                reemplazo:
                    "any(target_os = \"hermit\", target_os = \"soso\", target_os = \"solid_asp3\""
                        .into(),
            },
        ),
        paso(
            "sys/args: cfg common",
            "library/std/src/sys/args/mod.rs",
            "#[cfg(any(\n    target_os = \"soso\",",
            Accion::InsertarTrasLinea {
                ancla: "#[cfg(any(".into(),
                texto: "    target_os = \"soso\",".into(),
            },
        ),
        paso(
            "sys/args: rama soso",
            "library/std/src/sys/args/mod.rs",
            "target_os = \"soso\" => {\n        mod soso;",
            rama_antes(
                "    _ => {",
                "    target_os = \"soso\" => {\n        mod soso;\n        pub use soso::*;\n    }",
            ),
        ),
        paso(
            "sys/env: cfg common",
            "library/std/src/sys/env/mod.rs",
            "#[cfg(any(\n    target_os = \"soso\",",
            Accion::InsertarTrasLinea {
                ancla: "#[cfg(any(".into(),
                texto: "    target_os = \"soso\",".into(),
            },
        ),
        paso(
            "sys/env: rama soso",
            "library/std/src/sys/env/mod.rs",
            "target_os = \"soso\" => {\n        mod soso;",
            rama_antes(
                "    _ => {",
                "    target_os = \"soso\" => {\n        mod soso;\n        pub use soso::*;\n    }",
            ),
        ),
        paso(
            "sys/random: fill_bytes",
            "library/std/src/sys/random/mod.rs",
            "pub use soso::fill_bytes",
            rama_antes(
                "    target_os = \"hermit\" => {",
                "    target_os = \"soso\" => {\n        mod soso;\n        pub use soso::fill_bytes;\n    }",
            ),
        ),
        paso(
            "sys/thread_local: guard",
            "library/std/src/sys/thread_local/mod.rs",
            "target_os = \"hermit\", target_os = \"soso\", target_os = \"xous\"",
            Accion::Sustituir {
                busca: "any(target_os = \"hermit\", target_os = \"xous\")".into(),
                reemplazo: "any(target_os = \"hermit\", target_os = \"soso\", target_os = \"xous\")"
                    .into(),
            },
        ),
        paso(
            "sys/sync/futex: rama soso",
            "library/std/src/sys/sync/futex/mod.rs",
            "mod soso;",
            rama_antes(
                "    target_os = \"hermit\" => {",
                "    target_os = \"soso\" => {\n        mod soso;\n        pub use soso::*;\n    }",
            ),
        ),
        paso(
            "sys/sync/mutex: futex",
            "library/std/src/sys/sync/mutex/mod.rs",
            "target_os = \"soso\",",
            Accion::InsertarTrasLinea {
                ancla: "        target_os = \"hermit\",".into(),
                texto: "        target_os = \"soso\",".into(),
            },
        ),
        paso(
            "sys/sync/condvar: futex",
            "library/std/src/sys/sync/condvar/mod.rs",
            "target_os = \"soso\",",
            Accion::InsertarTrasLinea {
                ancla: "        target_os = \"hermit\",".into(),
                texto: "        target_os = \"soso\",".into(),
            },
        ),
        paso(
            "sys/sync/once: futex",
            "library/std/src/sys/sync/once/mod.rs",
            "target_os = \"soso\",",
            Accion::InsertarTrasLinea {
                ancla: "        target_os = \"hermit\",".into(),
                texto: "        target_os = \"soso\",".into(),
            },
        ),
        paso(
            "sys/sync/rwlock: futex",
            "library/std/src/sys/sync/rwlock/mod.rs",
            "target_os = \"soso\",",
            Accion::InsertarTrasLinea {
                ancla: "        target_os = \"hermit\",".into(),
                texto: "        target_os = \"soso\",".into(),
            },
        ),
        paso(
            "sys/stdio: rama soso",
            "library/std/src/sys/stdio/mod.rs",
            "target_os = \"soso\" =>",
            rama_antes(
                "    _ => {",
                "    target_os = \"soso\" => {\n        mod soso;\n        pub use soso::*;\n    }\n",
            ),
        ),
    ]
}

/// Fixture de T75: un directorio poseído con un resto y contenido viejo, y un
/// fichero fuera de ese directorio que no se puede tocar.
///
/// Corre igual en el host y en el guest: sólo usa [`Archivos`].
pub fn probar_espejo<A: Archivos>(fs: &mut A, raiz: &str) -> Result<(), String> {
    let base = raiz.trim_end_matches('/');
    let plantilla = format!("{base}/plantilla/propio/nuevo.rs");
    let resto = format!("{base}/vendor/propio/resto.rs");
    let nuevo = format!("{base}/vendor/propio/nuevo.rs");
    let ajeno = format!("{base}/vendor/ajeno/modulo.rs");
    fs.escribir(&plantilla, b"nuevo\n", 0o644)
        .map_err(|e| format!("no pude sembrar la plantilla: {e:?}"))?;
    fs.escribir(&resto, b"viejo\n", 0o644)
        .map_err(|e| format!("no pude sembrar el resto: {e:?}"))?;
    fs.escribir(&nuevo, b"obsoleto\n", 0o644)
        .map_err(|e| format!("no pude sembrar el contenido viejo: {e:?}"))?;
    fs.escribir(&ajeno, b"no tocar\n", 0o644)
        .map_err(|e| format!("no pude sembrar el fichero ajeno: {e:?}"))?;

    let pasos = [paso(
        "espejo propio",
        "propio",
        "",
        Accion::Espejar {
            desde: format!("{base}/plantilla/propio"),
            conservar: Vec::new(),
        },
    )];
    let vendor = format!("{base}/vendor");
    let antes = comprobar(fs, &vendor, &pasos);
    if antes[0].resultado != Resultado::Falta {
        return Err(format!(
            "un directorio con contenido viejo se informó {:?}; se esperaba Falta",
            antes[0].resultado
        ));
    }
    let inf = aplicar(fs, &vendor, &pasos);
    if inf[0].resultado != Resultado::Aplicado {
        return Err(format!("el espejo no se aplicó: {:?}", inf[0].resultado));
    }
    if fs.existe(&resto) {
        return Err("el resto sigue dentro del directorio poseído".into());
    }
    let n = fs
        .leer(&nuevo)
        .map_err(|e| format!("no pude leer el fichero espejado: {e:?}"))?;
    if n != b"nuevo\n" {
        return Err(format!("el contenido viejo se quedó: {n:?}"));
    }
    let a = fs
        .leer(&ajeno)
        .map_err(|e| format!("no pude leer el fichero ajeno: {e:?}"))?;
    if a != b"no tocar\n" {
        return Err("se modificó un fichero fuera del directorio poseído".into());
    }
    let segundo = aplicar(fs, &vendor, &pasos);
    if segundo[0].resultado != Resultado::YaEstaba {
        return Err(format!(
            "la segunda pasada no estaba quieta: {:?}",
            segundo[0].resultado
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entorno::{Entrada, Tipo};
    use crate::{Error, Resultado as Res};
    use alloc::collections::{BTreeMap, BTreeSet};

    #[derive(Default)]
    struct Memoria {
        f: BTreeMap<String, Vec<u8>>,
        dirs: BTreeSet<String>,
    }

    impl Memoria {
        fn con(pares: &[(&str, &str)]) -> Self {
            let mut m = Self::default();
            for (r, c) in pares {
                m.f.insert(r.to_string(), c.as_bytes().to_vec());
            }
            m
        }
        fn texto(&self, ruta: &str) -> String {
            String::from_utf8(self.f.get(ruta).cloned().unwrap_or_default()).unwrap()
        }
    }

    impl Archivos for Memoria {
        fn leer(&self, ruta: &str) -> Res<Vec<u8>> {
            self.f
                .get(ruta)
                .cloned()
                .ok_or_else(|| Error::entorno(format!("no existe {ruta}")))
        }
        fn escribir(&mut self, ruta: &str, datos: &[u8], _m: u32) -> Res<()> {
            self.f.insert(ruta.to_string(), datos.to_vec());
            Ok(())
        }
        fn existe(&self, ruta: &str) -> bool {
            self.f.contains_key(ruta) || self.dirs.contains(ruta)
        }
        fn listar(&self, dir: &str) -> Res<Vec<Entrada>> {
            let dir = dir.trim_end_matches('/');
            let pref = format!("{dir}/");
            let mut hijos: BTreeMap<String, Tipo> = BTreeMap::new();
            let mut visto = self.dirs.iter().any(|d| d == dir || d.starts_with(&pref));
            for k in self.f.keys() {
                if let Some(resto) = k.strip_prefix(&pref) {
                    visto = true;
                    let nombre = resto.split('/').next().unwrap_or(resto);
                    let tipo = if resto.contains('/') {
                        Tipo::Directorio
                    } else {
                        Tipo::Archivo
                    };
                    hijos
                        .entry(nombre.to_string())
                        .and_modify(|t| {
                            if tipo == Tipo::Directorio {
                                *t = Tipo::Directorio;
                            }
                        })
                        .or_insert(tipo);
                }
            }
            if !visto {
                return Err(Error::entorno(format!("no existe {dir}")));
            }
            Ok(hijos
                .into_iter()
                .map(|(ruta, tipo)| Entrada {
                    ruta,
                    tipo,
                    bytes: 0,
                    modo: 0,
                })
                .collect())
        }
        fn metadatos(&self, ruta: &str) -> Res<Entrada> {
            Ok(Entrada {
                ruta: ruta.to_string(),
                tipo: Tipo::Archivo,
                bytes: self.leer(ruta)?.len() as u64,
                modo: 0,
            })
        }
        fn crear_directorio(&mut self, r: &str) -> Res<()> {
            self.dirs.insert(r.trim_end_matches('/').to_string());
            Ok(())
        }
        fn borrar(&mut self, ruta: &str) -> Res<()> {
            let pref = format!("{}/", ruta.trim_end_matches('/'));
            let antes = self.f.len() + self.dirs.len();
            self.f.retain(|k, _| k != ruta && !k.starts_with(&pref));
            self.dirs.retain(|k| k != ruta && !k.starts_with(&pref));
            if self.f.len() + self.dirs.len() == antes {
                return Err(Error::entorno(format!("no existe {ruta}")));
            }
            Ok(())
        }
    }

    fn paso_insertar(ancla: &str, texto: &str, marca: &str) -> Paso {
        Paso {
            nombre: "rama soso en el match".into(),
            fichero: "library/std/src/sys/pal/mod.rs".into(),
            marca: marca.into(),
            accion: Accion::InsertarTrasLinea {
                ancla: ancla.into(),
                texto: texto.into(),
            },
        }
    }

    const MOD_RS: &str = "pub mod pal {\n    target_os = \"zkvm\" => {}\n    _ => {}\n}\n";

    #[test]
    fn inserta_tras_el_ancla_y_no_toca_el_resto() {
        let mut fs = Memoria::con(&[("/v/library/std/src/sys/pal/mod.rs", MOD_RS)]);
        let pasos = [paso_insertar("zkvm", "    target_os = \"soso\" => {}", "\"soso\" =>")];
        let inf = aplicar(&mut fs, "/v", &pasos);
        assert_eq!(inf[0].resultado, Resultado::Aplicado);
        let t = fs.texto("/v/library/std/src/sys/pal/mod.rs");
        assert!(t.contains("target_os = \"soso\" => {}"));
        // El ancla sigue, y sigue **antes**: insertar no sustituye.
        let i_zkvm = t.find("zkvm").unwrap();
        let i_soso = t.find("\"soso\"").unwrap();
        assert!(i_zkvm < i_soso);
        assert!(t.contains("_ => {}"), "el resto del fichero se conserva");
    }

    /// Aplicar dos veces deja el fichero **igual**.
    ///
    /// Es la propiedad que el script consigue con `grep -q … ||`, y la que
    /// hay que conservar: la preparación se ejecuta cada vez que alguien
    /// arranca el bootstrap.
    #[test]
    fn aplicar_dos_veces_no_duplica() {
        let mut fs = Memoria::con(&[("/v/library/std/src/sys/pal/mod.rs", MOD_RS)]);
        let pasos = [paso_insertar("zkvm", "    target_os = \"soso\" => {}", "\"soso\" =>")];
        assert_eq!(aplicar(&mut fs, "/v", &pasos)[0].resultado, Resultado::Aplicado);
        let tras_uno = fs.texto("/v/library/std/src/sys/pal/mod.rs");

        let segundo = aplicar(&mut fs, "/v", &pasos);
        assert_eq!(segundo[0].resultado, Resultado::YaEstaba);
        assert_eq!(fs.texto("/v/library/std/src/sys/pal/mod.rs"), tras_uno);
        assert_eq!(tras_uno.matches("\"soso\"").count(), 1, "una sola vez");
    }

    /// **El caso que motiva la ficha.** Con `sed -i`, un ancla que no aparece
    /// es un exit 0 y un fichero intacto; el script imprime «OK» y no ha
    /// parcheado nada.
    #[test]
    fn un_ancla_que_no_aparece_es_un_fallo_con_nombre() {
        let mut fs = Memoria::con(&[("/v/library/std/src/sys/pal/mod.rs", MOD_RS)]);
        let pasos = [paso_insertar("upstream-renombro-esto", "x", "marca-x")];
        let inf = aplicar(&mut fs, "/v", &pasos);
        assert!(!inf[0].ok());
        match &inf[0].resultado {
            Resultado::Fallo(m) => {
                assert!(m.contains("upstream-renombro-esto"), "dice qué ancla: {m}");
                assert!(m.contains("pal/mod.rs"), "y en qué fichero: {m}");
            }
            otro => panic!("esperaba un fallo, hubo {otro:?}"),
        }
        // Y no se escribió nada a medias.
        assert_eq!(fs.texto("/v/library/std/src/sys/pal/mod.rs"), MOD_RS);
    }

    /// Un fichero que no existe tampoco pasa por bueno.
    #[test]
    fn un_fichero_que_falta_es_un_fallo() {
        let mut fs = Memoria::default();
        let pasos = [paso_insertar("zkvm", "x", "marca-x")];
        let inf = aplicar(&mut fs, "/v", &pasos);
        assert!(!inf[0].ok());
    }

    /// Los demás pasos siguen informando aunque uno falle: si parara en el
    /// primero, cada problema costaría un ciclo entero.
    #[test]
    fn un_fallo_no_esconde_los_pasos_siguientes() {
        let mut fs = Memoria::con(&[("/v/a.rs", "hola\n"), ("/v/b.rs", "ancla\n")]);
        let pasos = [
            Paso {
                nombre: "el que falla".into(),
                fichero: "a.rs".into(),
                marca: "zzz".into(),
                accion: Accion::InsertarTrasLinea { ancla: "no-esta".into(), texto: "x".into() },
            },
            Paso {
                nombre: "el que sí".into(),
                fichero: "b.rs".into(),
                marca: "zzz".into(),
                accion: Accion::InsertarTrasLinea { ancla: "ancla".into(), texto: "zzz".into() },
            },
        ];
        let inf = aplicar(&mut fs, "/v", &pasos);
        assert_eq!(inf.len(), 2);
        assert!(!inf[0].ok());
        assert_eq!(inf[1].resultado, Resultado::Aplicado);
    }

    #[test]
    fn copiar_es_idempotente_por_contenido() {
        let mut fs = Memoria::con(&[("/plantilla/dl.rs", "pub fn dlopen() {}\n")]);
        let pasos = [Paso {
            nombre: "PAL dl.rs".into(),
            fichero: "library/std/src/sys/pal/soso/dl.rs".into(),
            marca: "no se usa en Copiar".into(),
            accion: Accion::Copiar { desde: "/plantilla/dl.rs".into() },
        }];
        assert_eq!(aplicar(&mut fs, "/v", &pasos)[0].resultado, Resultado::Aplicado);
        assert_eq!(aplicar(&mut fs, "/v", &pasos)[0].resultado, Resultado::YaEstaba);
        assert_eq!(fs.texto("/v/library/std/src/sys/pal/soso/dl.rs"), "pub fn dlopen() {}\n");
    }

    #[test]
    fn anadir_pone_al_final_una_sola_vez() {
        let mut fs = Memoria::con(&[("/v/Cargo.toml", "[package]\nname = \"std\"\n")]);
        let pasos = [Paso {
            nombre: "dependencia soso-rt".into(),
            fichero: "Cargo.toml".into(),
            marca: "soso-rt".into(),
            accion: Accion::Anadir { texto: "\n[target.soso.dependencies]\nsoso-rt = { path = \"x\" }\n".into() },
        }];
        assert_eq!(aplicar(&mut fs, "/v", &pasos)[0].resultado, Resultado::Aplicado);
        assert_eq!(aplicar(&mut fs, "/v", &pasos)[0].resultado, Resultado::YaEstaba);
        assert_eq!(fs.texto("/v/Cargo.toml").matches("soso-rt").count(), 1);
    }

    /// `comprobar` dice qué falta **y no escribe**.
    ///
    /// Es el caso que existía de verdad: seis parches aplicados y uno no, sin
    /// forma de verlo salvo abriendo los ficheros.
    #[test]
    fn comprobar_distingue_aplicado_de_falta_sin_tocar_nada() {
        let mut fs = Memoria::con(&[
            ("/v/a.rs", "ancla\nMARCA-A\n"),
            ("/v/b.rs", "ancla\n"),
        ]);
        let pasos = [
            Paso {
                nombre: "el que ya está".into(),
                fichero: "a.rs".into(),
                marca: "MARCA-A".into(),
                accion: Accion::InsertarTrasLinea { ancla: "ancla".into(), texto: "MARCA-A".into() },
            },
            Paso {
                nombre: "el que falta".into(),
                fichero: "b.rs".into(),
                marca: "MARCA-B".into(),
                accion: Accion::InsertarTrasLinea { ancla: "ancla".into(), texto: "MARCA-B".into() },
            },
        ];
        let antes_a = fs.texto("/v/a.rs");
        let antes_b = fs.texto("/v/b.rs");

        let inf = comprobar(&fs, "/v", &pasos);
        assert_eq!(inf[0].resultado, Resultado::YaEstaba);
        assert_eq!(inf[1].resultado, Resultado::Falta);

        // Y no ha tocado nada: es una comprobación, no una preparación.
        assert_eq!(fs.texto("/v/a.rs"), antes_a);
        assert_eq!(fs.texto("/v/b.rs"), antes_b);
        let _ = &mut fs;
    }

    /// Un ancla ausente sigue siendo un **fallo**, no un «falta»: «falta» dice
    /// que se puede aplicar, y eso sería mentira.
    #[test]
    fn comprobar_no_confunde_un_ancla_rota_con_un_parche_pendiente() {
        let fs = Memoria::con(&[("/v/a.rs", "otra cosa\n")]);
        let pasos = [Paso {
            nombre: "x".into(),
            fichero: "a.rs".into(),
            marca: "MARCA".into(),
            accion: Accion::InsertarTrasLinea { ancla: "ancla-que-no-esta".into(), texto: "MARCA".into() },
        }];
        let inf = comprobar(&fs, "/v", &pasos);
        assert!(!inf[0].ok(), "un ancla rota no es «falta»");
    }

    /// La receta declara los parches vigentes. El número de pasos no basta:
    /// una marca floja o un texto viejo colarían el mismo conteo.
    #[test]
    fn la_receta_real_declara_los_parches_del_script() {
        let pasos = pasos_libstd("/repo/config/rust-soso", "/repo/crates/soso-rt");
        for p in &pasos {
            let copia = matches!(
                p.accion,
                Accion::Copiar { .. }
                    | Accion::CopiarArbol { .. }
                    | Accion::Espejar { .. }
                    | Accion::CopiarSueltos { .. }
            );
            assert!(!p.nombre.is_empty(), "todo paso se llama de algo");
            assert!(
                copia || !p.marca.is_empty(),
                "el paso «{}» edita y no tiene marca de idempotencia",
                p.nombre
            );
        }
        assert!(
            pasos.iter().any(|p| matches!(p.accion, Accion::Espejar { .. }) && p.fichero.ends_with("os/soso")),
            "el directorio poseído se espeja, no se copia dejando restos"
        );
        assert!(
            pasos.iter().any(|p| matches!(p.accion, Accion::CopiarSueltos { .. })),
            "los módulos sueltos del árbol tienen paso"
        );
        let cargo = pasos.iter().find(|p| p.fichero.ends_with("Cargo.toml")).unwrap();
        match &cargo.accion {
            Accion::AsegurarLinea { linea, bloque, .. } => {
                assert!(linea.contains("/repo/crates/soso-rt"), "{linea}");
                assert!(linea.contains("rustc-dep-of-std"), "{linea}");
                assert!(linea.contains("public = true"), "{linea}");
                assert!(bloque.contains("rustc-dep-of-std"));
                assert!(!linea.contains("dep-of-std\"]") || linea.contains("rustc-dep-of-std"));
            }
            otro => panic!("la dependencia se reescribe, no se añade a ciegas: {otro:?}"),
        }
        assert!(
            pasos.iter().any(|p| p.marca.contains("DLL_PREFIX")),
            "env_consts exige las constantes de DLL/EXE"
        );
        assert!(
            pasos.iter().any(|p| p.fichero.ends_with("os/mod.rs") && matches!(p.accion, Accion::QuitarLinea { .. })),
            "soso no se deja en la lista de fd"
        );
        assert!(
            pasos.iter().any(|p| p.marca.contains("use soso as imp")),
            "el alocador tiene rama propia, no basta alloc_zeroed"
        );
        let h = huella(&pasos);
        assert_eq!(h.len(), 64, "huella sha256 para T39");
        assert_ne!(h, huella(&pasos_libstd("/otra", "/repo/crates/soso-rt")));
    }

    #[test]
    fn una_dependencia_vieja_no_cuenta_como_aplicada() {
        let mut fs = Memoria::con(&[(
            "/v/library/std/Cargo.toml",
            "[package]\nname = \"std\"\n\n[target.'cfg(target_os = \"soso\")'.dependencies]\nsoso-rt = { path = \"/viejo\", features = [\"dep-of-std\"] }\n",
        )]);
        let pasos = pasos_libstd("/repo/config/rust-soso", "/repo/crates/soso-rt");
        let cargo = pasos.iter().find(|p| p.fichero.ends_with("Cargo.toml")).unwrap();
        let uno = [cargo.clone()];
        let inf = comprobar(&fs, "/v", &uno);
        assert_eq!(inf[0].resultado, Resultado::Falta, "dep-of-std no es la línea vigente");
        let inf = aplicar(&mut fs, "/v", &uno);
        assert_eq!(inf[0].resultado, Resultado::Aplicado);
        let t = fs.texto("/v/library/std/Cargo.toml");
        assert!(t.contains("rustc-dep-of-std"));
        assert!(t.contains("public = true"));
        assert!(!t.contains("features = [\"dep-of-std\"]"));
        assert_eq!(aplicar(&mut fs, "/v", &uno)[0].resultado, Resultado::YaEstaba);
    }

    #[test]
    fn el_bloque_corto_de_env_no_cuenta_como_aplicado() {
        let corto = "\
#[cfg(target_os = \"hermit\")]
pub mod os {
    pub const OS: &str = \"hermit\";
}
#[cfg(target_os = \"soso\")]
pub mod os {
    pub const FAMILY: &str = \"unix\";
    pub const OS: &str = \"soso\";
}
";
        let mut fs = Memoria::con(&[("/v/library/std/src/sys/env_consts.rs", corto)]);
        let pasos = pasos_libstd("/p", "/rt");
        let reparar = pasos.iter().find(|p| p.nombre.contains("reparar bloque")).unwrap().clone();
        let uno = [reparar];
        assert_eq!(comprobar(&fs, "/v", &uno)[0].resultado, Resultado::Falta);
        assert_eq!(aplicar(&mut fs, "/v", &uno)[0].resultado, Resultado::Aplicado);
        let t = fs.texto("/v/library/std/src/sys/env_consts.rs");
        assert!(t.contains("DLL_PREFIX"));
        assert!(t.contains("EXE_EXTENSION"));
    }

    #[test]
    fn el_espejo_retira_un_resto_y_no_toca_un_ajeno() {
        let mut fs = Memoria::default();
        probar_espejo(&mut fs, "/tmp/t75").unwrap();
    }

    #[test]
    fn un_ancla_de_sustitucion_ausente_nombra_fichero() {
        let fs = Memoria::con(&[("/v/library/std/src/sys/alloc/mod.rs", "any(target_os = \"otro\")\n")]);
        let pasos = pasos_libstd("/p", "/rt");
        let p = pasos.iter().find(|p| p.nombre.contains("alloc_zeroed")).unwrap().clone();
        let uno = [p];
        let inf = comprobar(&fs, "/v", &uno);
        match &inf[0].resultado {
            Resultado::Fallo(m) => {
                assert!(m.contains("solid_asp3"), "{m}");
                assert!(m.contains("alloc/mod.rs"), "{m}");
            }
            otro => panic!("esperaba fallo de ancla, hubo {otro:?}"),
        }
    }

    #[test]
    fn la_revision_tiene_que_ser_la_del_lock() {
        assert!(verificar_revision("abc123", "abc123").is_ok());
        let e = verificar_revision("32d94cc9be3f", "otracosa").unwrap_err();
        assert!(e.contains("otracosa") && e.contains("32d94cc9be3f"), "{e}");
        assert!(verificar_revision("", "loquesea").is_err(), "sin lock no se parchea");
    }
}
