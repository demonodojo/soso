//! T24 — copia de tarea y exportación de su parche.
//!
//! El ciclo entero sin Git: capturar un árbol, reconstruirlo en otro sitio,
//! sembrar el enunciado, dejar que un «candidato» cambie cosas y exportar lo
//! que cambió como paquete aplicable al árbol original.
//!
//! Lo que se vigila aquí no es que las rutas se copien —eso lo fija T01— sino
//! los tres sitios donde esto se estropea sin avisar: que el `TASK.md` del
//! coordinador acabe en el parche, que un enlace simbólico desaparezca del
//! informe, y que la copia se plante encima del trabajo de otro.

use std::collections::{BTreeMap, BTreeSet};

use soso_improve_core::captura::{self, Git, Politica};
use soso_improve_core::entorno::{Archivos, Entrada, Tipo};
use soso_improve_core::state::{Comprobacion, Limites, TaskSpec};
use soso_improve_core::workspace::*;
use soso_improve_core::{unir, Error, Resultado, ESQUEMA};

// ---------------------------------------------------------------------------
// Árbol en memoria, con enlaces simbólicos de mentira pero con su tipo real
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Memoria {
    ficheros: BTreeMap<String, (Vec<u8>, u32)>,
    /// Rutas que el sistema de archivos declara como «no es archivo regular».
    otros: BTreeSet<String>,
    directorios: BTreeSet<String>,
}

impl Memoria {
    fn pon(&mut self, ruta: &str, datos: &[u8]) {
        self.ficheros
            .insert(ruta.to_string(), (datos.to_vec(), 0o644));
    }
    /// Un enlace simbólico: existe, se lista, y **no** es un archivo regular.
    fn enlace(&mut self, ruta: &str, destino: &str) {
        self.ficheros
            .insert(ruta.to_string(), (destino.as_bytes().to_vec(), 0o777));
        self.otros.insert(ruta.to_string());
    }
    fn tipo_de(&self, ruta: &str) -> Tipo {
        if self.otros.contains(ruta) {
            Tipo::Otro
        } else {
            Tipo::Archivo
        }
    }
}

impl Archivos for Memoria {
    fn leer(&self, ruta: &str) -> Resultado<Vec<u8>> {
        self.ficheros
            .get(ruta)
            .map(|(d, _)| d.clone())
            .ok_or_else(|| Error::entorno(format!("no existe {ruta}")))
    }
    fn escribir(&mut self, ruta: &str, datos: &[u8], modo: u32) -> Resultado<()> {
        self.ficheros
            .insert(ruta.to_string(), (datos.to_vec(), modo));
        Ok(())
    }
    fn existe(&self, ruta: &str) -> bool {
        self.ficheros.contains_key(ruta)
    }
    fn listar(&self, dir: &str) -> Resultado<Vec<Entrada>> {
        let prefijo = if dir.is_empty() {
            String::new()
        } else {
            format!("{}/", dir.trim_end_matches('/'))
        };
        let mut vistos: BTreeMap<String, Entrada> = BTreeMap::new();
        let mut hay = self.directorios.contains(dir);
        for k in self.ficheros.keys() {
            let Some(resto) = k.strip_prefix(&prefijo) else {
                continue;
            };
            hay = true;
            match resto.split_once('/') {
                Some((dir_hijo, _)) => {
                    vistos.entry(dir_hijo.to_string()).or_insert(Entrada {
                        ruta: dir_hijo.to_string(),
                        tipo: Tipo::Directorio,
                        bytes: 0,
                        modo: 0o755,
                    });
                }
                None => {
                    let (datos, modo) = &self.ficheros[k];
                    vistos.insert(
                        resto.to_string(),
                        Entrada {
                            ruta: resto.to_string(),
                            tipo: self.tipo_de(k),
                            bytes: datos.len() as u64,
                            modo: *modo,
                        },
                    );
                }
            }
        }
        if !hay {
            return Err(Error::entorno(format!("no existe {dir}")));
        }
        Ok(vistos.into_values().collect())
    }
    fn metadatos(&self, ruta: &str) -> Resultado<Entrada> {
        let (datos, modo) = self
            .ficheros
            .get(ruta)
            .ok_or_else(|| Error::entorno(format!("no existe {ruta}")))?;
        Ok(Entrada {
            ruta: ruta.to_string(),
            tipo: self.tipo_de(ruta),
            bytes: datos.len() as u64,
            modo: *modo,
        })
    }
    fn crear_directorio(&mut self, ruta: &str) -> Resultado<()> {
        self.directorios.insert(ruta.to_string());
        Ok(())
    }
    fn borrar(&mut self, ruta: &str) -> Resultado<()> {
        self.directorios.remove(ruta);
        self.ficheros.remove(ruta);
        self.otros.remove(ruta);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Montaje
// ---------------------------------------------------------------------------

/// Un árbol de trabajo **sin `.git`**: la ficha lo pide así porque todo el
/// circuito tiene que funcionar donde Git no existe.
fn arbol_origen() -> Memoria {
    let mut m = Memoria::default();
    m.pon("/repo/src/main.rs", b"fn main() { println!(\"hola\"); }");
    m.pon("/repo/src/lib.rs", b"pub fn suma(a: i32, b: i32) -> i32 { a - b }");
    // Subruta con espacios: se copia y se exporta igual que cualquier otra.
    m.pon("/repo/docs/notas de diseno/uno.md", b"# uno\n");
    m.pon("/repo/sobra.txt", b"esto se borrara\n");
    m.pon("/repo/bin/dato.bin", &[0u8, 159, 146, 150, 0, 255]);
    m
}

fn capturar(origen: &Memoria) -> (Memoria, captura::Manifiesto) {
    let mut alm = Memoria::default();
    let man = captura::capturar(
        origen,
        "/repo",
        &mut alm,
        "/cap",
        &Politica::default(),
        Git::default(),
        BTreeMap::new(),
        Vec::new(),
        Vec::new(),
        "2026-09-24T00:00:00Z",
    )
    .unwrap();
    (alm, man)
}

fn spec(base: &str) -> TaskSpec {
    TaskSpec {
        schema_version: ESQUEMA,
        id: "T99-suma".into(),
        base: base.into(),
        problema: "`suma` resta en vez de sumar".into(),
        rutas_editables: vec!["src/lib.rs".into()],
        comprobaciones: vec![Comprobacion {
            argv: vec!["cargo".into(), "test".into()],
            cwd: ".".into(),
            timeout_s: 600,
        }],
        limites: Limites::default(),
        hashes_entrada: BTreeMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Preparar
// ---------------------------------------------------------------------------

#[test]
fn la_copia_reproduce_la_base_y_no_toca_el_original() {
    let origen = arbol_origen();
    let (alm, man) = capturar(&origen);
    let mut destino = Memoria::default();
    let copia = preparar(
        &alm,
        "/cap",
        &mut destino,
        "/trabajo/r-1",
        &man,
        &spec(&captura::huella(&man.inventario)),
        "r-1",
    )
    .unwrap();

    assert_eq!(copia.archivos, man.inventario.len());
    let v = captura::verificar(&destino, "/trabajo/r-1", &man).unwrap();
    assert!(v.ok, "{:?}", v.problemas);

    // Los binarios y los nombres con espacios sobreviven al viaje.
    assert_eq!(
        destino.leer("/trabajo/r-1/bin/dato.bin").unwrap(),
        vec![0u8, 159, 146, 150, 0, 255]
    );
    assert!(destino.existe("/trabajo/r-1/docs/notas de diseno/uno.md"));

    // Y el árbol activo queda exactamente como estaba: es la razón de copiar.
    assert_eq!(origen.ficheros, arbol_origen().ficheros);
}

#[test]
fn el_enunciado_se_materializa_con_lo_visible_y_nada_mas() {
    let origen = arbol_origen();
    let (alm, man) = capturar(&origen);
    let base = captura::huella(&man.inventario);
    let mut destino = Memoria::default();
    let mut s = spec(&base);
    s.hashes_entrada
        .insert("banco/reservado".into(), "no-debe-salir".into());
    preparar(&alm, "/cap", &mut destino, "/w", &man, &s, "r-1").unwrap();

    let texto = String::from_utf8(destino.leer("/w/TASK.md").unwrap()).unwrap();
    assert!(texto.contains("`suma` resta en vez de sumar"), "{texto}");
    assert!(texto.contains("src/lib.rs"), "{texto}");
    assert!(texto.contains("cargo test"), "{texto}");
    // La contabilidad del coordinador no viaja en el enunciado.
    assert!(!texto.contains(&base), "la huella base no es cosa suya:\n{texto}");
    assert!(!texto.contains("no-debe-salir"), "{texto}");
}

#[test]
fn una_tarea_escrita_contra_otra_base_se_rechaza() {
    let origen = arbol_origen();
    let (alm, man) = capturar(&origen);
    let mut destino = Memoria::default();
    let mut s = spec(&"f".repeat(64));
    s.base = "f".repeat(64);
    let e = preparar(&alm, "/cap", &mut destino, "/w", &man, &s, "r-1").unwrap_err();
    assert!(format!("{e:?}").contains("base"), "{e:?}");
    // Y no ha dejado nada a medias.
    assert!(!destino.existe("/w/TASK.md"));
}

#[test]
fn un_destino_ocupado_no_se_pisa() {
    let origen = arbol_origen();
    let (alm, man) = capturar(&origen);
    let base = captura::huella(&man.inventario);

    // Un directorio con cosas de alguien.
    let mut destino = Memoria::default();
    destino.pon("/w/tesis.tex", b"anos de trabajo\n");
    let e = preparar(&alm, "/cap", &mut destino, "/w", &man, &spec(&base), "r-1").unwrap_err();
    assert!(format!("{e:?}").contains("no lleva marca"), "{e:?}");
    assert_eq!(destino.leer("/w/tesis.tex").unwrap(), b"anos de trabajo\n");

    // La copia de otra ejecución tampoco se reutiliza ni se borra.
    let mut d2 = Memoria::default();
    preparar(&alm, "/cap", &mut d2, "/w", &man, &spec(&base), "r-1").unwrap();
    let e = preparar(&alm, "/cap", &mut d2, "/w", &man, &spec(&base), "r-2").unwrap_err();
    assert!(format!("{e:?}").contains("r-1"), "{e:?}");

    // Repetir la misma ejecución sí vale: es reanudar, no pisar.
    preparar(&alm, "/cap", &mut d2, "/w", &man, &spec(&base), "r-1").unwrap();
}

// ---------------------------------------------------------------------------
// Exportar
// ---------------------------------------------------------------------------

fn copia_lista() -> (Memoria, captura::Manifiesto, Memoria, Copia) {
    let origen = arbol_origen();
    let (alm, man) = capturar(&origen);
    let base = captura::huella(&man.inventario);
    let mut destino = Memoria::default();
    let copia = preparar(&alm, "/cap", &mut destino, "/w", &man, &spec(&base), "r-1").unwrap();
    (alm, man, destino, copia)
}

#[test]
fn el_paquete_lleva_altas_bajas_y_binarios() {
    let (_, man, mut w, copia) = copia_lista();

    // El «candidato» trabaja.
    w.pon("/w/src/lib.rs", b"pub fn suma(a: i32, b: i32) -> i32 { a + b }");
    w.pon("/w/tests/suma.rs", b"#[test] fn va() { assert_eq!(2, 2); }");
    w.borrar("/w/sobra.txt").unwrap();
    w.pon("/w/bin/dato.bin", &[1u8, 2, 3]);

    let exp = exportar(&w, &copia, &man, &Politica::default()).unwrap();
    let rutas: Vec<&str> = exp.paquete.operaciones.iter().map(|o| o.ruta()).collect();
    assert_eq!(
        rutas,
        vec!["bin/dato.bin", "sobra.txt", "src/lib.rs", "tests/suma.rs"],
        "{rutas:?}"
    );
    assert_eq!(exp.paquete.base, copia.base);
    assert!(!exp.vacia());
}

/// La trampa que esta ficha tiene que evitar: el `TASK.md` lo escribió el
/// coordinador. Si saliera en el paquete, se aplicaría al repositorio de verdad
/// como si el candidato lo hubiera creado.
#[test]
fn lo_que_siembra_el_coordinador_no_es_un_cambio_del_intento() {
    let (_, man, mut w, copia) = copia_lista();
    w.pon("/w/src/lib.rs", b"pub fn suma(a: i32, b: i32) -> i32 { a + b }");

    let exp = exportar(&w, &copia, &man, &Politica::default()).unwrap();
    let rutas: Vec<&str> = exp.paquete.operaciones.iter().map(|o| o.ruta()).collect();
    assert_eq!(rutas, vec!["src/lib.rs"], "{rutas:?}");
    assert!(exp.previos.iter().any(|p| p == "TASK.md"));
    assert!(exp.previos.iter().any(|p| p == MARCA));

    // Y tampoco si el candidato lo toca: sigue sin ser del repositorio.
    w.pon("/w/TASK.md", b"# lo he reescrito\n");
    let exp = exportar(&w, &copia, &man, &Politica::default()).unwrap();
    let rutas: Vec<&str> = exp.paquete.operaciones.iter().map(|o| o.ruta()).collect();
    assert_eq!(rutas, vec!["src/lib.rs"], "{rutas:?}");
}

/// Un enlace saliente no se sigue **y no desaparece del informe**. Si se
/// descartara en silencio, el contenido de lo que apunta podría acabar en un
/// parche sin que nadie lo viera.
#[test]
fn un_enlace_saliente_se_informa_y_no_entra_en_el_paquete() {
    let (_, man, mut w, copia) = copia_lista();
    w.enlace("/w/atajo", "/etc/passwd");
    w.enlace("/w/src/otro", "../../../../fuera");

    let exp = exportar(&w, &copia, &man, &Politica::default()).unwrap();
    assert!(exp.vacia(), "un enlace no es un cambio: {:?}", exp.paquete.operaciones);
    let rechazadas: Vec<&str> = exp.rechazados.iter().map(|x| x.ruta.as_str()).collect();
    assert_eq!(rechazadas, vec!["atajo", "src/otro"], "{rechazadas:?}");
    for x in &exp.rechazados {
        assert_eq!(x.motivo, captura::NO_REGULAR);
        assert!(x.sha256.is_none(), "no se ha leído lo que apunta");
    }
}

/// Lo excluido por política —artefactos de build— no es «sospechoso»: si
/// entrara en `rechazados` junto a los enlaces, el informe que tiene que
/// delatar un escape se llenaría de ruido y nadie lo miraría.
#[test]
fn lo_ignorado_por_politica_no_se_confunde_con_un_enlace() {
    let (_, man, mut w, copia) = copia_lista();
    // Un archivo excluido por prefijo — no un directorio podado, que ni
    // siquiera se recorre y por tanto no probaría nada.
    w.pon("/w/models/pesos.gguf", b"dos gigas de mentira");
    w.enlace("/w/atajo", "/etc/passwd");

    let exp = exportar(&w, &copia, &man, &Politica::default()).unwrap();
    let rechazadas: Vec<&str> = exp.rechazados.iter().map(|x| x.ruta.as_str()).collect();
    assert_eq!(rechazadas, vec!["atajo"], "sólo lo que no es archivo regular");
    // Y desde luego el archivo ignorado no viaja en el paquete.
    let rutas: Vec<&str> = exp.paquete.operaciones.iter().map(|o| o.ruta()).collect();
    assert!(!rutas.iter().any(|r| r.starts_with("models/")), "{rutas:?}");
}

#[test]
fn un_intento_que_no_cambio_nada_da_un_paquete_vacio() {
    let (_, man, w, copia) = copia_lista();
    let exp = exportar(&w, &copia, &man, &Politica::default()).unwrap();
    assert!(exp.vacia());
    assert!(exp.rechazados.is_empty());
}

#[test]
fn exportar_contra_otro_manifiesto_se_rechaza() {
    let (_, man, w, mut copia) = copia_lista();
    copia.base = "0".repeat(64);
    let e = exportar(&w, &copia, &man, &Politica::default()).unwrap_err();
    assert!(format!("{e:?}").contains("copia se hizo sobre"), "{e:?}");
}

/// El paquete tiene que aplicarse de verdad sobre el árbol original: si sólo
/// «parece» correcto, no acredita nada.
#[test]
fn el_paquete_se_aplica_al_arbol_original() {
    let origen = arbol_origen();
    let (mut alm, man) = capturar(&origen);
    let base = captura::huella(&man.inventario);
    let mut w = Memoria::default();
    let copia = preparar(&alm, "/cap", &mut w, "/w", &man, &spec(&base), "r-1").unwrap();

    w.pon("/w/src/lib.rs", b"pub fn suma(a: i32, b: i32) -> i32 { a + b }");
    w.borrar("/w/sobra.txt").unwrap();
    let exp = exportar(&w, &copia, &man, &Politica::default()).unwrap();

    // Los contenidos nuevos van al almacén, como haría el coordinador.
    for op in &exp.paquete.operaciones {
        if let Some(h) = op.objeto() {
            let datos = w.leer(&unir("/w", op.ruta())).unwrap();
            alm.escribir(&unir("/cap", &captura::ruta_objeto(h)), &datos, 0o644)
                .unwrap();
        }
    }

    let mut arbol = arbol_origen();
    let inv = soso_improve_core::delta::aplicar(
        &alm,
        "/cap",
        &mut arbol,
        "/repo",
        &man.inventario,
        &exp.paquete,
    )
    .unwrap();

    assert_eq!(
        arbol.leer("/repo/src/lib.rs").unwrap(),
        b"pub fn suma(a: i32, b: i32) -> i32 { a + b }"
    );
    assert!(!arbol.existe("/repo/sobra.txt"));
    assert_eq!(captura::huella(&inv), exp.paquete.destino);
    // Y el TASK.md no se ha colado en el repositorio.
    assert!(!arbol.existe("/repo/TASK.md"));
}

// ---------------------------------------------------------------------------
// Limpieza
// ---------------------------------------------------------------------------

#[test]
fn solo_se_borra_la_copia_propia() {
    let (_, _, mut w, copia) = copia_lista();

    // La de otra ejecución no se toca, aunque la ruta coincida.
    let mut ajena = copia.clone();
    ajena.run_id = "r-9".into();
    let e = limpiar(&mut w, &ajena).unwrap_err();
    assert!(format!("{e:?}").contains("r-1"), "{e:?}");
    assert!(w.existe("/w/TASK.md"));

    // La propia sí, y se lleva subdirectorios y enlaces.
    w.enlace("/w/atajo", "/etc/passwd");
    let n = limpiar(&mut w, &copia).unwrap();
    assert!(n >= 6, "borrados {n}");
    assert!(!w.existe("/w/TASK.md"));
    assert!(!w.existe("/w/docs/notas de diseno/uno.md"));
    // Borrar el enlace no puede borrar su destino.
    assert!(!w.existe("/w/atajo"));
}

#[test]
fn los_intentos_fallidos_se_conservan() {
    use soso_improve_core::state::Estado;
    assert!(conservar(Estado::Rechazada), "es lo único que queda para mirar");
    assert!(conservar(Estado::Bloqueada));
    assert!(conservar(Estado::Verificando));
    assert!(!conservar(Estado::Aceptada), "ya dejó su paquete");
}

#[test]
fn la_marca_se_lee_y_rechaza_un_esquema_desconocido() {
    let (_, _, mut w, copia) = copia_lista();
    assert_eq!(leer_marca(&w, "/w").unwrap(), copia);

    let mut rara = copia.clone();
    rara.schema_version = ESQUEMA + 3;
    w.escribir("/w/.soso-workspace.json", &serde_json::to_vec(&rara).unwrap(), 0o644)
        .unwrap();
    assert!(leer_marca(&w, "/w").is_err());
}
