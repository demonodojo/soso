//! Base reproducible: qué árbol se está mejorando, exactamente.
//!
//! La primera versión de esto (T01, en Python) describía la base como
//! `HEAD` + parches de git, y reconstruía con `git clone` + `git apply
//! --binary`. Funciona en el host y **no puede funcionar dentro de soso**: el
//! `soso-git` del sistema sabe `status|log|diff|commit`, no clonar ni aplicar
//! parches binarios.
//!
//! Así que la base se describe por lo que es —un conjunto de archivos con su
//! contenido— y no por cómo llegó a serlo:
//!
//! ```text
//! baseline.json          manifiesto: inventario, entorno, herramientas, sonda
//! objetos/<aa>/<hash>    contenido de cada archivo, direccionado por su hash
//! logs/<nombre>.log      argv, cwd, código de salida y salidas de cada comando
//! ```
//!
//! El almacén por contenido hace que dos capturas del mismo árbol compartan
//! los objetos: la segunda cuesta casi nada. Reconstruir es copiar objetos a
//! sus rutas, sin git de por medio.
//!
//! Git sigue usándose —cuando está— para **describir** la base (commit, rama,
//! estado), porque eso ayuda a un humano a situarla. Si no está, se registra
//! que no está y el inventario sigue siendo exacto.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::entorno::{Archivos, Entrada, Orden, Procesos, Tipo};
use crate::ignorar::Reglas;
use crate::{sha256_hex, unir, Error, Resultado, ESQUEMA};

/// Qué no entra en la base. Son artefactos de build, pesos y firmware: nada de
/// eso describe el trabajo, y recorrerlo cuesta gigabytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Politica {
    /// Nombres de directorio que se saltan a cualquier profundidad.
    pub segmentos_excluidos: Vec<String>,
    /// Prefijos de ruta que se saltan.
    pub prefijos_excluidos: Vec<String>,
    /// Por encima de esto se registra ruta y hash, pero no se guarda contenido.
    pub limite_bytes: u64,
    /// Reglas leídas del `.gitignore` del árbol. El repositorio ya declara ahí
    /// qué es desechable —artefactos, fuentes ajenas, imágenes de disco— y
    /// repetirlo a mano aquí envejece mal. Se guardan en el manifiesto: una
    /// base no es reproducible si no se sabe qué se dejó fuera.
    #[serde(default)]
    pub ignorar: Reglas,
}

impl Default for Politica {
    fn default() -> Self {
        Politica {
            segmentos_excluidos: vec![
                ".git".to_string(),
                "target".to_string(),
                "__pycache__".to_string(),
            ],
            prefijos_excluidos: vec![
                "rootfs/lib/firmware/".to_string(),
                "lxdde/firmware/".to_string(),
                "lxdde/linux/".to_string(),
                "models/".to_string(),
                "vendor/".to_string(),
            ],
            limite_bytes: 8 * 1024 * 1024,
            ignorar: Reglas::default(),
        }
    }
}

impl Politica {
    /// Añade las reglas de un `.gitignore`.
    pub fn con_ignorados(mut self, reglas: Reglas) -> Self {
        self.ignorar = reglas;
        self
    }

    pub fn excluye(&self, ruta: &str) -> Option<String> {
        self.excluye_como(ruta, false)
    }

    /// `es_directorio` importa: `nombre/` en un `.gitignore` solo casa con
    /// directorios.
    pub fn excluye_como(&self, ruta: &str, es_directorio: bool) -> Option<String> {
        if self.ignorar.ignora(ruta, es_directorio) {
            return Some("ignorado por .gitignore".to_string());
        }
        self.excluye_por_politica(ruta)
    }

    fn excluye_por_politica(&self, ruta: &str) -> Option<String> {
        for seg in ruta.split('/') {
            if self.segmentos_excluidos.iter().any(|s| s == seg) {
                return Some(format!("segmento excluido: {seg}"));
            }
        }
        for p in &self.prefijos_excluidos {
            if ruta.starts_with(p.as_str()) {
                return Some(format!("prefijo excluido: {p}"));
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchivoBase {
    pub ruta: String,
    pub sha256: String,
    pub bytes: u64,
    pub modo: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Excluido {
    pub ruta: String,
    pub bytes: u64,
    /// `None` cuando ni siquiera se hasheó (artefactos de build).
    pub sha256: Option<String>,
    pub motivo: String,
}

/// Lo que git cuenta de la base. Todo opcional: sin git, la base sigue siendo
/// exacta, solo que menos legible para un humano.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Git {
    pub disponible: bool,
    pub commit: Option<String>,
    pub rama: Option<String>,
    pub asunto: Option<String>,
    /// `git status --porcelain=v1`, tal cual, para leerlo a ojo.
    pub estado: Option<String>,
    pub nota: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Herramienta {
    pub nombre: String,
    pub argv: Vec<String>,
    pub disponible: bool,
    pub exit_code: Option<i32>,
    pub version: Option<String>,
    pub log: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variable {
    pub definida: bool,
    pub valor: Option<String>,
    pub motivo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suite {
    pub nombre: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub estado: String,
    pub exit_code: Option<i32>,
    pub log: Option<String>,
    pub motivo: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Estabilidad {
    pub sondas: u32,
    pub estable: bool,
    /// Hash del inventario completo: identifica la base en una línea.
    pub huella: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifiesto {
    pub schema_version: u32,
    pub herramienta: String,
    pub creado: String,
    pub raiz: String,
    pub git: Git,
    pub politica: Politica,
    pub inventario: Vec<ArchivoBase>,
    pub excluidos: Vec<Excluido>,
    pub variables: BTreeMap<String, Variable>,
    pub herramientas: Vec<Herramienta>,
    pub suites_base: Vec<Suite>,
    pub estabilidad: Estabilidad,
    /// Rutas declaradas cuyo contenido no se guardó (grandes o excluidas).
    pub no_almacenados: Vec<String>,
}

impl Manifiesto {
    pub fn completa(&self) -> bool {
        self.no_almacenados.is_empty()
    }
}

/// Una sonda: el estado observable del árbol en un instante.
///
/// Se toma antes y después de escribir la captura. Si cambia, lo capturado no
/// describe ningún estado real del repositorio y no se escribe el manifiesto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sonda {
    pub archivos: BTreeMap<String, String>,
    pub huella: String,
}

impl Sonda {
    pub fn diferencias(&self, otra: &Sonda) -> Vec<String> {
        let mut fuera = Vec::new();
        for (ruta, hash) in &self.archivos {
            match otra.archivos.get(ruta) {
                None => fuera.push(format!("desapareció {ruta}")),
                Some(h) if h != hash => fuera.push(format!("cambió {ruta}")),
                _ => {}
            }
        }
        for ruta in otra.archivos.keys() {
            if !self.archivos.contains_key(ruta) {
                fuera.push(format!("apareció {ruta}"));
            }
        }
        fuera
    }
}

/// Recorre el árbol aplicando la política. Devuelve (incluidos, excluidos).
/// Motivo con el que se excluye lo que no es un archivo regular: enlaces
/// simbólicos, dispositivos, sockets.
///
/// Es una constante y no un literal suelto porque [`workspace`](crate::workspace)
/// distingue por él lo sospechoso de lo meramente ignorado. Con dos literales
/// iguales, reescribir uno haría desaparecer los enlaces del informe sin que
/// fallara nada.
pub const NO_REGULAR: &str = "no es archivo regular";

pub fn recorrer(
    arbol: &dyn Archivos,
    raiz: &str,
    politica: &Politica,
) -> Resultado<(Vec<Entrada>, Vec<Excluido>)> {
    let mut incluidos = Vec::new();
    let mut excluidos = Vec::new();
    let mut pendientes = vec![String::new()];
    // Un directorio se visita una vez: si el sistema de archivos devuelve
    // entradas repetidas —o hay un ciclo—, el recorrido termina igual en vez
    // de girar para siempre.
    let mut vistos: alloc::collections::BTreeSet<String> = alloc::collections::BTreeSet::new();
    while let Some(relativo) = pendientes.pop() {
        if !vistos.insert(relativo.clone()) {
            continue;
        }
        let absoluto = if relativo.is_empty() {
            raiz.to_string()
        } else {
            unir(raiz, &relativo)
        };
        for entrada in arbol.listar(&absoluto)? {
            let ruta = if relativo.is_empty() {
                entrada.ruta.clone()
            } else {
                format!("{relativo}/{}", entrada.ruta)
            };
            if let Some(motivo) = politica.excluye_como(&ruta, entrada.tipo == Tipo::Directorio) {
                if entrada.tipo == Tipo::Archivo {
                    excluidos.push(Excluido {
                        ruta,
                        bytes: entrada.bytes,
                        sha256: None,
                        motivo,
                    });
                }
                continue;
            }
            match entrada.tipo {
                Tipo::Directorio => pendientes.push(ruta),
                Tipo::Archivo => incluidos.push(Entrada { ruta, ..entrada }),
                Tipo::Otro => excluidos.push(Excluido {
                    ruta,
                    bytes: entrada.bytes,
                    sha256: None,
                    motivo: NO_REGULAR.to_string(),
                }),
            }
        }
    }
    incluidos.sort_by(|a, b| a.ruta.cmp(&b.ruta));
    excluidos.sort_by(|a, b| a.ruta.cmp(&b.ruta));
    Ok((incluidos, excluidos))
}

/// Hash del inventario entero: dos árboles con la misma huella son el mismo.
pub fn huella(inventario: &[ArchivoBase]) -> String {
    let mut acumulado = String::new();
    for a in inventario {
        acumulado.push_str(&a.ruta);
        acumulado.push('\0');
        acumulado.push_str(&a.sha256);
        acumulado.push('\n');
    }
    sha256_hex(acumulado.as_bytes())
}

/// Sonda del árbol: ruta → hash de contenido, más su huella.
pub fn sondar(arbol: &dyn Archivos, raiz: &str, politica: &Politica) -> Resultado<Sonda> {
    let (incluidos, _) = recorrer(arbol, raiz, politica)?;
    let mut archivos = BTreeMap::new();
    let mut inventario = Vec::new();
    for e in incluidos {
        let absoluto = unir(raiz, &e.ruta);
        let hash = if e.bytes > politica.limite_bytes {
            // Un archivo enorme no se lee dos veces por sonda: basta con que su
            // tamaño y su ruta no cambien.
            format!("grande:{}", e.bytes)
        } else {
            sha256_hex(&arbol.leer(&absoluto)?)
        };
        archivos.insert(e.ruta.clone(), hash.clone());
        inventario.push(ArchivoBase {
            ruta: e.ruta,
            sha256: hash,
            bytes: e.bytes,
            modo: e.modo,
        });
    }
    let h = huella(&inventario);
    Ok(Sonda {
        archivos,
        huella: h,
    })
}

/// Ruta del objeto que guarda un contenido dado.
pub fn ruta_objeto(hash: &str) -> String {
    format!("objetos/{}/{}", &hash[..2], hash)
}

/// Escribe el contenido de los archivos en el almacén por contenido.
///
/// Devuelve el inventario y la lista de rutas cuyo contenido no se guardó.
fn guardar_objetos(
    origen: &dyn Archivos,
    raiz: &str,
    destino: &mut dyn Archivos,
    salida: &str,
    incluidos: &[Entrada],
    politica: &Politica,
) -> Resultado<(Vec<ArchivoBase>, Vec<Excluido>, Vec<String>)> {
    let mut inventario = Vec::new();
    let mut grandes = Vec::new();
    let mut no_almacenados = Vec::new();
    for e in incluidos {
        let datos = origen.leer(&unir(raiz, &e.ruta))?;
        let hash = sha256_hex(&datos);
        if e.bytes > politica.limite_bytes {
            grandes.push(Excluido {
                ruta: e.ruta.clone(),
                bytes: e.bytes,
                sha256: Some(hash.clone()),
                motivo: format!("tamaño {} > {}", e.bytes, politica.limite_bytes),
            });
            no_almacenados.push(e.ruta.clone());
        } else {
            let destino_objeto = unir(salida, &ruta_objeto(&hash));
            if !destino.existe(&destino_objeto) {
                destino.escribir(&destino_objeto, &datos, 0o644)?;
            }
        }
        inventario.push(ArchivoBase {
            ruta: e.ruta.clone(),
            sha256: hash,
            bytes: e.bytes,
            modo: e.modo,
        });
    }
    Ok((inventario, grandes, no_almacenados))
}

/// Captura la base del árbol `raiz` en el directorio `salida`.
///
/// `salida` debe estar vacío o no existir. No modifica el árbol de origen: solo
/// lee. Si el árbol cambia entre las dos sondas, devuelve [`Error::Inestable`]
/// y **no** escribe el manifiesto.
#[allow(clippy::too_many_arguments)]
pub fn capturar(
    origen: &dyn Archivos,
    raiz: &str,
    destino: &mut dyn Archivos,
    salida: &str,
    politica: &Politica,
    git: Git,
    variables: BTreeMap<String, Variable>,
    herramientas: Vec<Herramienta>,
    suites_base: Vec<Suite>,
    creado: &str,
) -> Resultado<Manifiesto> {
    let antes = sondar(origen, raiz, politica)?;

    let (incluidos, mut excluidos) = recorrer(origen, raiz, politica)?;
    let (inventario, grandes, no_almacenados) =
        guardar_objetos(origen, raiz, destino, salida, &incluidos, politica)?;
    excluidos.extend(grandes);
    excluidos.sort_by(|a, b| a.ruta.cmp(&b.ruta));

    let manifiesto = Manifiesto {
        schema_version: ESQUEMA,
        herramienta: "soso-improve".to_string(),
        creado: creado.to_string(),
        raiz: raiz.to_string(),
        git,
        politica: politica.clone(),
        estabilidad: Estabilidad {
            sondas: 2,
            estable: true,
            huella: huella(&inventario),
        },
        inventario,
        excluidos,
        variables,
        herramientas,
        suites_base,
        no_almacenados,
    };

    let despues = sondar(origen, raiz, politica)?;
    let diferencias = antes.diferencias(&despues);
    if !diferencias.is_empty() {
        let detalle = diferencias.join("; ");
        destino.escribir(
            &unir(salida, "inestable.json"),
            serde_json::to_string_pretty(&diferencias)
                .map_err(Error::formato)?
                .as_bytes(),
            0o644,
        )?;
        return Err(Error::Inestable(detalle));
    }

    escribir_manifiesto(destino, salida, &manifiesto)?;
    Ok(manifiesto)
}

pub fn escribir_manifiesto(
    destino: &mut dyn Archivos,
    salida: &str,
    manifiesto: &Manifiesto,
) -> Resultado<()> {
    let texto = serde_json::to_string_pretty(manifiesto).map_err(Error::formato)?;
    destino.escribir(&unir(salida, "baseline.json"), texto.as_bytes(), 0o644)
}

pub fn leer_manifiesto(arbol: &dyn Archivos, salida: &str) -> Resultado<Manifiesto> {
    let datos = arbol.leer(&unir(salida, "baseline.json"))?;
    let manifiesto: Manifiesto = serde_json::from_slice(&datos).map_err(Error::formato)?;
    if manifiesto.schema_version != ESQUEMA {
        return Err(Error::formato(format!(
            "schema_version no soportada: {}",
            manifiesto.schema_version
        )));
    }
    Ok(manifiesto)
}

/// Problema encontrado al verificar una reconstrucción.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Problema {
    pub ruta: String,
    pub esperado: String,
    pub obtenido: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verificacion {
    pub ok: bool,
    pub archivos: usize,
    pub problemas: Vec<Problema>,
    pub no_reproducidos: Vec<String>,
}

/// Rehace la base en `destino_raiz` copiando los objetos de la captura.
///
/// Sin git: los contenidos están en la captura, así que reconstruir es escribir
/// archivos. Es lo que permite que esto funcione dentro de soso.
pub fn reconstruir(
    captura: &dyn Archivos,
    salida: &str,
    destino: &mut dyn Archivos,
    destino_raiz: &str,
    manifiesto: &Manifiesto,
) -> Resultado<usize> {
    let no_almacenados: Vec<&String> = manifiesto.no_almacenados.iter().collect();
    let mut escritos = 0;
    for archivo in &manifiesto.inventario {
        crate::ruta_segura(&archivo.ruta)?;
        if no_almacenados.iter().any(|r| **r == archivo.ruta) {
            continue;
        }
        let datos = captura.leer(&unir(salida, &ruta_objeto(&archivo.sha256)))?;
        let real = sha256_hex(&datos);
        if real != archivo.sha256 {
            return Err(Error::formato(format!(
                "el objeto de {} no cuadra con su hash",
                archivo.ruta
            )));
        }
        destino.escribir(&unir(destino_raiz, &archivo.ruta), &datos, archivo.modo)?;
        escritos += 1;
    }
    Ok(escritos)
}

/// Comprueba que un árbol reconstruido es la base declarada.
pub fn verificar(
    arbol: &dyn Archivos,
    raiz: &str,
    manifiesto: &Manifiesto,
) -> Resultado<Verificacion> {
    let mut problemas = Vec::new();
    let mut archivos = 0;
    for a in &manifiesto.inventario {
        if manifiesto.no_almacenados.iter().any(|r| *r == a.ruta) {
            continue;
        }
        archivos += 1;
        let ruta = unir(raiz, &a.ruta);
        match arbol.leer(&ruta) {
            Err(_) => problemas.push(Problema {
                ruta: a.ruta.clone(),
                esperado: a.sha256.clone(),
                obtenido: "ausente".to_string(),
            }),
            Ok(datos) => {
                let real = sha256_hex(&datos);
                if real != a.sha256 {
                    problemas.push(Problema {
                        ruta: a.ruta.clone(),
                        esperado: a.sha256.clone(),
                        obtenido: real,
                    });
                }
            }
        }
    }
    Ok(Verificacion {
        ok: problemas.is_empty(),
        archivos,
        problemas,
        no_reproducidos: manifiesto.no_almacenados.clone(),
    })
}

/// Variables que se versionan, por nombre. Lista explícita: el entorno entero
/// nunca se vuelca, que es donde viven las credenciales.
pub const VARIABLES: &[&str] = &[
    "SOSO_BUILD",
    "SOSO_CHECK_PROFILE",
    "SOSO_DRIVERS",
    "SOSO_FIRMWARE",
    "SOSO_LIVE_MODEL",
    "SOSO_LIVE_OFFLINE",
    "SOSO_LXDDE",
    "SOSO_LXDDE_MODE",
    "SOSO_MODELS_DIR",
    "SOSO_QEMU_ACCEL",
    "SOSO_QEMU_MEM",
    "SOSO_QEMU_SMP",
    "SOSO_RUST_VENDOR",
    "SOSO_TEST_JOBS",
    "SOSO_VERSION",
    "CARGO_TARGET_DIR",
    "RUSTUP_TOOLCHAIN",
    "RUSTC_WRAPPER",
    "CARGO_BUILD_JOBS",
];

/// ¿El nombre huele a secreto? Entonces se registra si está definida, nunca su
/// valor.
pub fn parece_secreto(nombre: &str) -> bool {
    const SOSPECHOSOS: &[&str] = &["TOKEN", "KEY", "SECRET", "PASS", "CRED", "AUTH"];
    let mayus: String = nombre.to_uppercase();
    SOSPECHOSOS.iter().any(|s| mayus.contains(s))
}

/// Herramientas cuya versión se registra. Que falte una no rompe la captura:
/// también es parte de la base saber que no está.
pub const HERRAMIENTAS: &[(&str, &[&str])] = &[
    ("git", &["git", "--version"]),
    ("cargo", &["cargo", "--version"]),
    ("rustc", &["rustc", "--version"]),
    ("rustup", &["rustup", "show", "active-toolchain"]),
    ("clang", &["clang", "--version"]),
    ("lld", &["ld.lld", "--version"]),
    ("qemu", &["qemu-system-x86_64", "--version"]),
    ("opencode", &["opencode", "--version"]),
];

/// Suites base de C6, declaradas pero no ejecutadas por la captura.
pub const SUITES: &[(&str, &[&str], &str)] = &[
    (
        "core-std",
        &["cargo", "test", "-p", "soso-llm-core", "--features", "std"],
        ".",
    ),
    (
        "guest-llm",
        &["cargo", "build", "--release", "-p", "soso-llm"],
        "user",
    ),
    ("xtask-check", &["cargo", "xtask", "check"], "."),
    ("xtask-test", &["cargo", "xtask", "test"], "."),
];

/// Declara las suites sin ejecutarlas: un build largo no se esconde dentro de
/// una captura que debe ser rápida y repetible.
pub fn suites_declaradas() -> Vec<Suite> {
    SUITES
        .iter()
        .map(|(nombre, argv, cwd)| Suite {
            nombre: nombre.to_string(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            cwd: cwd.to_string(),
            estado: "no ejecutada".to_string(),
            exit_code: None,
            log: None,
            motivo: Some(
                "las suites base se ejecutan como paso explícito para no ocultar un build largo"
                    .to_string(),
            ),
        })
        .collect()
}

/// Sondea las herramientas con el ejecutor dado.
pub fn sondar_herramientas(
    procesos: &dyn Procesos,
    cwd: &str,
    guardar_log: &mut dyn FnMut(&str, &str) -> Resultado<String>,
) -> Vec<Herramienta> {
    let mut fuera = Vec::new();
    for (nombre, argv) in HERRAMIENTAS {
        let orden = Orden::nueva(argv, cwd).con_timeout(60);
        let (disponible, exit_code, version, texto) = match procesos.ejecutar(&orden) {
            Ok(s) => {
                let primera = s
                    .texto()
                    .lines()
                    .next()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty());
                (s.ok(), s.codigo, if s.ok() { primera } else { None }, s.texto())
            }
            Err(e) => (false, None, None, format!("{e}")),
        };
        let log = guardar_log(&format!("tool-{nombre}"), &texto).ok();
        fuera.push(Herramienta {
            nombre: nombre.to_string(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            disponible,
            exit_code,
            version,
            log,
        });
    }
    fuera
}
