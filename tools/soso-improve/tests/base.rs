//! Pruebas de la base reproducible (ficha T01, port a Rust).
//!
//! Cada fixture es un árbol real en un temporal. Se comprueban las dos
//! propiedades que dan valor a una captura: que el árbol de origen queda igual
//! después de capturarlo, y que la copia reconstruida coincide hash a hash.
//!
//! La captura ya no describe la base como «HEAD + parches» sino como el
//! contenido de sus archivos, así que no hay fixture de índice: lo que se
//! captura es el árbol de trabajo. Un repositorio con cambios preparados y sin
//! preparar sobre el mismo archivo se sigue probando —`indice_y_arbol`—, pero
//! lo que se exige es que se capture el contenido del árbol y que el estado de
//! git quede anotado.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use soso_improve::sistema::Host;
use soso_improve_core::captura::{self as base, Politica};
use soso_improve_core::entorno::Archivos;
use soso_improve_core::ignorar::Reglas;
use soso_improve_core::{sha256_hex, Error};

struct Temporal(PathBuf);

impl Temporal {
    fn nuevo(nombre: &str) -> Temporal {
        let ruta = std::env::temp_dir().join(format!(
            "t01-rust-{nombre}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&ruta).unwrap();
        Temporal(ruta)
    }

    fn sub(&self, nombre: &str) -> String {
        self.0.join(nombre).to_string_lossy().into_owned()
    }
}

impl Drop for Temporal {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn escribir(raiz: &str, relativa: &str, datos: &[u8]) {
    let ruta = Path::new(raiz).join(relativa);
    std::fs::create_dir_all(ruta.parent().unwrap()).unwrap();
    std::fs::write(ruta, datos).unwrap();
}

/// Árbol de prueba con lo que exigen las fixtures de la ficha: texto, binario,
/// ruta con espacios, archivo nuevo y algo ignorado.
fn arbol_de_prueba(raiz: &str) {
    escribir(raiz, "README.md", b"base\n");
    escribir(raiz, "src/lib.rs", b"pub fn uno() -> u32 { 1 }\n");
    escribir(raiz, "datos/blob.bin", &(0u8..=255).cycle().take(4096).collect::<Vec<u8>>());
    escribir(raiz, "un archivo con espacios.txt", "acentuado: café ☕\n".as_bytes());
    escribir(raiz, "notas/nuevo.md", b"nuevo\n");
    escribir(raiz, ".gitignore", b"/artefactos/\n*.tmp\n");
    escribir(raiz, "artefactos/salida.bin", b"no deberia entrar\n");
    escribir(raiz, "borrador.tmp", b"tampoco\n");
}

fn politica(raiz: &str) -> Politica {
    let texto = std::fs::read_to_string(Path::new(raiz).join(".gitignore")).unwrap_or_default();
    Politica::default().con_ignorados(Reglas::parsear(&texto).unwrap())
}

fn capturar(raiz: &str, salida: &str) -> Result<base::Manifiesto, Error> {
    let mut host = Host;
    host.crear_directorio(salida).unwrap();
    base::capturar(
        &Host,
        raiz,
        &mut host,
        salida,
        &politica(raiz),
        base::Git::default(),
        BTreeMap::new(),
        Vec::new(),
        base::suites_declaradas(),
        "2026-09-16T00:00:00Z",
    )
}

/// Huella observable del árbol: ruta → hash de contenido, sin lo ignorado.
fn huella_arbol(raiz: &str) -> BTreeMap<String, String> {
    let (incluidos, _) = base::recorrer(&Host, raiz, &politica(raiz)).unwrap();
    incluidos
        .into_iter()
        .map(|e| {
            let datos = std::fs::read(Path::new(raiz).join(&e.ruta)).unwrap();
            (e.ruta, sha256_hex(&datos))
        })
        .collect()
}

#[test]
fn captura_un_arbol_y_lo_reconstruye_identico() {
    let tmp = Temporal::nuevo("reconstruir");
    let raiz = tmp.sub("arbol");
    std::fs::create_dir_all(&raiz).unwrap();
    arbol_de_prueba(&raiz);

    let salida = tmp.sub("captura");
    let manifiesto = capturar(&raiz, &salida).unwrap();

    // Lo ignorado por .gitignore no entra, y lo demás sí.
    let rutas: Vec<&str> = manifiesto.inventario.iter().map(|a| a.ruta.as_str()).collect();
    assert!(rutas.contains(&"README.md"));
    assert!(rutas.contains(&"un archivo con espacios.txt"), "ruta con espacios: {rutas:?}");
    assert!(rutas.contains(&"datos/blob.bin"));
    assert!(rutas.contains(&"notas/nuevo.md"));
    assert!(!rutas.contains(&"artefactos/salida.bin"), "lo ignorado no entra");
    assert!(!rutas.contains(&"borrador.tmp"));
    assert!(manifiesto.completa());

    let destino = tmp.sub("copia");
    let mut host = Host;
    host.crear_directorio(&destino).unwrap();
    let escritos = base::reconstruir(&Host, &salida, &mut host, &destino, &manifiesto).unwrap();
    assert_eq!(escritos, manifiesto.inventario.len());

    let verificacion = base::verificar(&Host, &destino, &manifiesto).unwrap();
    assert!(verificacion.ok, "{:?}", verificacion.problemas);

    // Comparación independiente del manifiesto: archivo a archivo.
    assert_eq!(huella_arbol(&raiz), huella_arbol(&destino));
}

#[test]
fn el_binario_sobrevive_byte_a_byte() {
    let tmp = Temporal::nuevo("binario");
    let raiz = tmp.sub("arbol");
    std::fs::create_dir_all(&raiz).unwrap();
    let contenido: Vec<u8> = (0..5000u32).map(|i| ((i * 7 + 13) % 256) as u8).collect();
    escribir(&raiz, "datos/blob.bin", &contenido);

    let salida = tmp.sub("captura");
    let manifiesto = capturar(&raiz, &salida).unwrap();
    let destino = tmp.sub("copia");
    let mut host = Host;
    host.crear_directorio(&destino).unwrap();
    base::reconstruir(&Host, &salida, &mut host, &destino, &manifiesto).unwrap();

    let copiado = std::fs::read(Path::new(&destino).join("datos/blob.bin")).unwrap();
    assert_eq!(copiado, contenido);
}

#[test]
fn el_arbol_de_origen_no_se_altera() {
    let tmp = Temporal::nuevo("intacto");
    let raiz = tmp.sub("arbol");
    std::fs::create_dir_all(&raiz).unwrap();
    arbol_de_prueba(&raiz);

    let antes = huella_arbol(&raiz);
    capturar(&raiz, &tmp.sub("captura")).unwrap();
    assert_eq!(huella_arbol(&raiz), antes);
}

#[test]
fn dos_capturas_del_mismo_arbol_comparten_objetos() {
    let tmp = Temporal::nuevo("dedup");
    let raiz = tmp.sub("arbol");
    std::fs::create_dir_all(&raiz).unwrap();
    arbol_de_prueba(&raiz);

    let una = capturar(&raiz, &tmp.sub("a")).unwrap();
    let otra = capturar(&raiz, &tmp.sub("b")).unwrap();
    // La huella identifica la base: dos capturas del mismo árbol dan la misma.
    assert_eq!(una.estabilidad.huella, otra.estabilidad.huella);
    assert_eq!(una.inventario, otra.inventario);
}

/// Un archivo por encima del límite se declara con su hash, pero no se guarda.
#[test]
fn lo_grande_se_declara_y_no_se_guarda() {
    let tmp = Temporal::nuevo("grande");
    let raiz = tmp.sub("arbol");
    std::fs::create_dir_all(&raiz).unwrap();
    escribir(&raiz, "peque.txt", b"hola\n");
    escribir(&raiz, "enorme.bin", &vec![7u8; 2048]);

    let salida = tmp.sub("captura");
    let mut politica = Politica::default();
    politica.limite_bytes = 1024;
    let mut host = Host;
    host.crear_directorio(&salida).unwrap();
    let manifiesto = base::capturar(
        &Host,
        &raiz,
        &mut host,
        &salida,
        &politica,
        base::Git::default(),
        BTreeMap::new(),
        Vec::new(),
        Vec::new(),
        "2026-09-16T00:00:00Z",
    )
    .unwrap();

    assert!(!manifiesto.completa());
    assert_eq!(manifiesto.no_almacenados, vec!["enorme.bin".to_string()]);
    let grande = manifiesto
        .inventario
        .iter()
        .find(|a| a.ruta == "enorme.bin")
        .unwrap();
    assert_eq!(grande.sha256, sha256_hex(&vec![7u8; 2048]));

    let destino = tmp.sub("copia");
    host.crear_directorio(&destino).unwrap();
    base::reconstruir(&Host, &salida, &mut host, &destino, &manifiesto).unwrap();
    assert!(!Path::new(&destino).join("enorme.bin").exists());
    assert!(Path::new(&destino).join("peque.txt").exists());
    let verificacion = base::verificar(&Host, &destino, &manifiesto).unwrap();
    assert!(verificacion.ok, "lo no almacenado no cuenta como problema");
    assert_eq!(verificacion.no_reproducidos, vec!["enorme.bin".to_string()]);
}

/// Árbol que cambia entre las dos sondas: la captura se declara inestable.
///
/// El cambio se provoca desde un `Archivos` entrometido, sin ganchos en el
/// código de producción.
#[test]
fn una_modificacion_concurrente_hace_inestable_la_captura() {
    struct Entrometido {
        raiz: String,
        lecturas: std::cell::Cell<u32>,
    }

    impl Archivos for Entrometido {
        fn leer(&self, ruta: &str) -> soso_improve_core::Resultado<Vec<u8>> {
            let datos = Host.leer(ruta)?;
            let n = self.lecturas.get() + 1;
            self.lecturas.set(n);
            // A media captura, alguien toca el árbol.
            if n == 3 {
                escribir(&self.raiz, "README.md", b"cambiado a mitad\n");
            }
            Ok(datos)
        }
        fn escribir(&mut self, ruta: &str, datos: &[u8], modo: u32) -> soso_improve_core::Resultado<()> {
            Host.escribir(ruta, datos, modo)
        }
        fn existe(&self, ruta: &str) -> bool {
            Host.existe(ruta)
        }
        fn listar(&self, ruta: &str) -> soso_improve_core::Resultado<Vec<soso_improve_core::entorno::Entrada>> {
            Host.listar(ruta)
        }
        fn metadatos(&self, ruta: &str) -> soso_improve_core::Resultado<soso_improve_core::entorno::Entrada> {
            Host.metadatos(ruta)
        }
        fn crear_directorio(&mut self, ruta: &str) -> soso_improve_core::Resultado<()> {
            Host.crear_directorio(ruta)
        }
        fn borrar(&mut self, ruta: &str) -> soso_improve_core::Resultado<()> {
            Host.borrar(ruta)
        }
    }

    let tmp = Temporal::nuevo("inestable");
    let raiz = tmp.sub("arbol");
    std::fs::create_dir_all(&raiz).unwrap();
    arbol_de_prueba(&raiz);

    let salida = tmp.sub("captura");
    let mut host = Host;
    host.crear_directorio(&salida).unwrap();
    let entrometido = Entrometido {
        raiz: raiz.clone(),
        lecturas: std::cell::Cell::new(0),
    };
    let error = base::capturar(
        &entrometido,
        &raiz,
        &mut host,
        &salida,
        &politica(&raiz),
        base::Git::default(),
        BTreeMap::new(),
        Vec::new(),
        Vec::new(),
        "2026-09-16T00:00:00Z",
    )
    .unwrap_err();

    match error {
        Error::Inestable(detalle) => assert!(
            detalle.contains("README.md"),
            "la inestabilidad debe decir qué cambió: {detalle}"
        ),
        otro => panic!("se esperaba una captura inestable, hubo {otro:?}"),
    }
    // No se escribe manifiesto: una base que se movió no se declara.
    assert!(!Path::new(&salida).join("baseline.json").exists());
    assert!(Path::new(&salida).join("inestable.json").exists());
}

#[test]
fn una_ruta_con_dos_puntos_en_el_manifiesto_no_escribe_fuera() {
    let tmp = Temporal::nuevo("escape");
    let raiz = tmp.sub("arbol");
    std::fs::create_dir_all(&raiz).unwrap();
    escribir(&raiz, "a.txt", b"a\n");
    let salida = tmp.sub("captura");
    let mut manifiesto = capturar(&raiz, &salida).unwrap();

    manifiesto.inventario[0].ruta = "../fuera.txt".to_string();
    let destino = tmp.sub("copia");
    let mut host = Host;
    host.crear_directorio(&destino).unwrap();
    let error = base::reconstruir(&Host, &salida, &mut host, &destino, &manifiesto).unwrap_err();
    assert!(matches!(error, Error::Uso(_)), "{error:?}");
    assert!(!tmp.0.join("fuera.txt").exists());
}

/// Fixture de la ficha: cambios preparados y sin preparar sobre el mismo
/// archivo, en un repositorio de verdad.
///
/// La captura describe el **árbol de trabajo**: el índice de git no viaja,
/// porque es un concepto de git y dentro de soso no existe. Lo que sí se
/// conserva es la descripción: commit, rama y el `git status` tal cual, para
/// que un humano sepa de dónde salió la base.
#[test]
fn indice_y_arbol_en_un_repositorio_de_verdad() {
    if !std::process::Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        eprintln!("sin git: se omite la descripción de la base");
        return;
    }
    let tmp = Temporal::nuevo("git");
    let raiz = tmp.sub("arbol");
    std::fs::create_dir_all(&raiz).unwrap();
    escribir(&raiz, "src/lib.rs", b"version 1\n");
    escribir(&raiz, ".gitignore", b"/artefactos/\n");

    let git = |args: &[&str]| {
        let estado = std::process::Command::new("git")
            .args(args)
            .current_dir(&raiz)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t01")
            .env("GIT_AUTHOR_EMAIL", "t01@example.invalid")
            .env("GIT_COMMITTER_NAME", "t01")
            .env("GIT_COMMITTER_EMAIL", "t01@example.invalid")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(estado.success(), "git {args:?}");
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["add", "-A"]);
    git(&["commit", "--quiet", "-m", "base"]);

    // Preparado en el índice…
    escribir(&raiz, "src/lib.rs", b"version 2\n");
    git(&["add", "src/lib.rs"]);
    // …y otra cosa distinta en el árbol.
    escribir(&raiz, "src/lib.rs", b"version 3\n");

    let salida = tmp.sub("captura");
    let mut host = Host;
    host.crear_directorio(&salida).unwrap();
    let descripcion = soso_improve::git::describir(&raiz);
    assert!(descripcion.disponible, "git describe la base");
    assert!(descripcion.commit.is_some());
    assert_eq!(descripcion.rama.as_deref(), Some("main"));
    assert!(
        descripcion.estado.as_deref().unwrap_or("").contains("src/lib.rs"),
        "el estado de git queda anotado: {:?}",
        descripcion.estado
    );

    let manifiesto = base::capturar(
        &Host,
        &raiz,
        &mut host,
        &salida,
        &politica(&raiz),
        descripcion,
        BTreeMap::new(),
        Vec::new(),
        Vec::new(),
        "2026-09-16T00:00:00Z",
    )
    .unwrap();

    // Lo capturado es el árbol de trabajo, no el índice.
    let destino = tmp.sub("copia");
    host.crear_directorio(&destino).unwrap();
    base::reconstruir(&Host, &salida, &mut host, &destino, &manifiesto).unwrap();
    let contenido = std::fs::read(Path::new(&destino).join("src/lib.rs")).unwrap();
    assert_eq!(contenido, b"version 3\n", "se captura el árbol de trabajo");

    // Y `.git` no entra en la base: es del repositorio, no del trabajo.
    assert!(!manifiesto.inventario.iter().any(|a| a.ruta.starts_with(".git/")));
    assert!(!Path::new(&destino).join(".git").exists());
}
