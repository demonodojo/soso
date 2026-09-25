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

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::entorno::Archivos;

/// Qué hacer con un fichero del vendor.
#[derive(Debug, Clone)]
pub enum Accion {
    /// Copiar bytes tal cual (lo que hacía `cp`).
    Copiar { desde: String },
    /// Copiar un árbol entero, recursivo (lo que hacía `rsync -a`).
    ///
    /// No borra lo que sobre en el destino, igual que `rsync` **sin**
    /// `--delete`: si se cambiara eso, un fichero que ya no está en la
    /// plantilla desaparecería del vendor sin avisar.
    CopiarArbol { desde: String },
    /// Insertar `texto` **después** de la línea que contiene `ancla`.
    InsertarTrasLinea { ancla: String, texto: String },
    /// Insertar `texto` **antes** de la línea que contiene `ancla`.
    InsertarAntesDeLinea { ancla: String, texto: String },
    /// Añadir `texto` al final del fichero.
    Anadir { texto: String },
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
        // Un árbol no se compara aquí entero: lo que interesa de la
        // comprobación es el estado de los parches, y decir «puede que falte
        // algún fichero» sin mirar sería peor que no decir nada.
        Accion::CopiarArbol { .. } => {
            if fs.existe(&destino) {
                Resultado::YaEstaba
            } else {
                Resultado::Falta
            }
        }
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

    let bytes = match fs.leer(&destino) {
        Ok(d) => d,
        Err(e) => return Resultado::Fallo(format!("no pude leer {destino}: {e:?}")),
    };
    let Ok(texto) = String::from_utf8(bytes) else {
        return Resultado::Fallo(format!("{destino} no es UTF-8"));
    };
    if texto.contains(&paso.marca) {
        return Resultado::YaEstaba;
    }

    let nuevo = match &paso.accion {
        Accion::Copiar { .. } | Accion::CopiarArbol { .. } => {
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

/// La receta del bootstrap de libstd, traducida de
/// `config/rust-soso/apply-patches.sh`.
///
/// `plantillas` es `config/rust-soso` del checkout y `soso_rt` la ruta que va
/// en el `Cargo.toml` del vendor. Las rutas de los ficheros son **relativas a
/// la raíz del vendor**.
///
/// Es una traducción **fiel**, no una mejora: si el script hace algo raro,
/// aquí hace lo mismo y queda anotado. Cambiar la conducta sin poder compilar
/// la libstd —hoy no se puede, falta el enlazador— sería decidir a ciegas.
pub fn pasos_libstd(plantillas: &str, soso_rt: &str) -> Vec<Paso> {
    let t = plantillas.trim_end_matches('/');
    alloc::vec![
        Paso {
            nombre: "PAL: os/soso".into(),
            fichero: "library/std/src/os/soso".into(),
            marca: String::new(),
            accion: Accion::CopiarArbol {
                desde: format!("{t}/tree/library/std/src/os/soso"),
            },
        },
        Paso {
            nombre: "PAL: sys/pal/soso".into(),
            fichero: "library/std/src/sys/pal/soso".into(),
            marca: String::new(),
            accion: Accion::CopiarArbol {
                desde: format!("{t}/tree/library/std/src/sys/pal/soso"),
            },
        },
        // **Anotado**: el `mod.rs` que copia el paso anterior no declara
        // `mod dl`, así que este fichero queda en el vendor sin que nada lo
        // compile. Comprobado en el vendor real. Se conserva porque quitarlo
        // es una decisión que necesita un build para validarse; está en T69.
        Paso {
            nombre: "PAL: dl.rs (hoy no lo declara nadie — T69)".into(),
            fichero: "library/std/src/sys/pal/soso/dl.rs".into(),
            marca: String::new(),
            accion: Accion::Copiar {
                desde: format!("{t}/sys/pal/soso/dl.rs"),
            },
        },
        Paso {
            nombre: "build.rs: target soso".into(),
            fichero: "library/std/build.rs".into(),
            marca: "target_os == \"soso\"".into(),
            accion: Accion::InsertarTrasLinea {
                ancla: "|| target_os == \"vexos\"".into(),
                texto: "        || target_os == \"soso\"".into(),
            },
        },
        Paso {
            nombre: "sys/pal/mod.rs: rama soso".into(),
            fichero: "library/std/src/sys/pal/mod.rs".into(),
            marca: "mod soso;".into(),
            accion: Accion::InsertarTrasLinea {
                ancla: "pub use self::zkvm::*;".into(),
                texto: "    }\n    target_os = \"soso\" => {\n        mod soso;\n        pub use self::soso::*;".into(),
            },
        },
        Paso {
            nombre: "os/mod.rs: pub mod soso".into(),
            fichero: "library/std/src/os/mod.rs".into(),
            marca: "pub mod soso;".into(),
            accion: Accion::InsertarTrasLinea {
                ancla: "#[cfg(target_os = \"hermit\")]".into(),
                texto: "#[cfg(target_os = \"soso\")]\npub mod soso;".into(),
            },
        },
        Paso {
            nombre: "os/mod.rs: soso en la lista de targets".into(),
            fichero: "library/std/src/os/mod.rs".into(),
            marca: "target_os = \"soso\",".into(),
            accion: Accion::InsertarTrasLinea {
                ancla: "target_os = \"hermit\",".into(),
                texto: "    target_os = \"soso\",".into(),
            },
        },
        Paso {
            nombre: "sys/exit.rs: salida por soso_rt".into(),
            fichero: "library/std/src/sys/exit.rs".into(),
            marca: "soso_rt::exit".into(),
            accion: Accion::InsertarTrasLinea {
                ancla: "hermit_abi::exit(code)".into(),
                texto: "        target_os = \"soso\" => soso_rt::exit(code),".into(),
            },
        },
        Paso {
            nombre: "Cargo.toml: dependencia soso-rt".into(),
            fichero: "library/std/Cargo.toml".into(),
            marca: "soso-rt".into(),
            accion: Accion::Anadir {
                texto: format!(
                    "\n[target.'cfg(target_os = \"soso\")'.dependencies]\nsoso-rt = {{ path = \"{soso_rt}\", features = [\"dep-of-std\"] }}\n"
                ),
            },
        },
        Paso {
            nombre: "env_consts.rs: constantes de soso".into(),
            fichero: "library/std/src/sys/env_consts.rs".into(),
            marca: "OS: &str = \"soso\"".into(),
            accion: Accion::InsertarAntesDeLinea {
                ancla: "#[cfg(target_os = \"hermit\")]".into(),
                texto: "#[cfg(target_os = \"soso\")]\npub mod os {\n    pub const FAMILY: &str = \"unix\";\n    pub const OS: &str = \"soso\";\n    pub const ARCH: &str = env!(\"STD_ENV_ARCH\");\n}\n".into(),
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entorno::{Entrada, Tipo};
    use crate::{Error, Resultado as Res};
    use alloc::collections::BTreeMap;

    #[derive(Default)]
    struct Memoria {
        f: BTreeMap<String, Vec<u8>>,
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
            self.f.contains_key(ruta)
        }
        fn listar(&self, _d: &str) -> Res<Vec<Entrada>> {
            Ok(Vec::new())
        }
        fn metadatos(&self, ruta: &str) -> Res<Entrada> {
            Ok(Entrada {
                ruta: ruta.to_string(),
                tipo: Tipo::Archivo,
                bytes: self.leer(ruta)?.len() as u64,
                modo: 0,
            })
        }
        fn crear_directorio(&mut self, _r: &str) -> Res<()> {
            Ok(())
        }
        fn borrar(&mut self, ruta: &str) -> Res<()> {
            self.f.remove(ruta).map(|_| ()).ok_or_else(|| Error::entorno("no existe"))
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

    /// La receta real tiene un paso por parche del script, y ninguno con
    /// marca vacía salvo las copias (que se comparan por contenido).
    #[test]
    fn la_receta_real_declara_los_parches_del_script() {
        let pasos = pasos_libstd("/repo/config/rust-soso", "/repo/crates/soso-rt");
        assert_eq!(pasos.len(), 10, "tres copias y siete ediciones");
        for p in &pasos {
            let copia = matches!(p.accion, Accion::Copiar { .. } | Accion::CopiarArbol { .. });
            assert!(!p.nombre.is_empty(), "todo paso se llama de algo");
            assert!(
                copia || !p.marca.is_empty(),
                "el paso «{}» edita y no tiene marca de idempotencia",
                p.nombre
            );
        }
        // La ruta de soso-rt entra en el Cargo.toml, no se adivina.
        let cargo = pasos.iter().find(|p| p.fichero.ends_with("Cargo.toml")).unwrap();
        match &cargo.accion {
            Accion::Anadir { texto } => assert!(texto.contains("/repo/crates/soso-rt")),
            otro => panic!("esperaba Anadir, hay {otro:?}"),
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
