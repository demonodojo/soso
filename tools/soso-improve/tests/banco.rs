//! Pruebas del banco de casos (ficha T02, port a Rust).
//!
//! Comprueban la estructura y el sellado, la separación de lo reservado y —lo
//! que de verdad importa— que cada verificador distingue una solución correcta
//! de una incorrecta. Los casos de repo necesitan cargo y un árbol del
//! proyecto, así que aquí solo se comprueba su estructura: su discriminación se
//! ejecuta como paso explícito y queda en la evidencia de la ficha.

use std::path::{Path, PathBuf};

use soso_improve::sistema::{Host, Temporal};
use soso_improve_core::caso::{self, CasoCargado};
use soso_improve_core::entorno::Archivos;
use soso_improve_core::{programa, protocolo, unir};

fn raiz_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn banco() -> String {
    raiz_repo()
        .join("tests/self-improvement/cases")
        .to_string_lossy()
        .into_owned()
}

fn reservado() -> String {
    unir(&banco(), "reservado")
}

fn casos() -> Vec<CasoCargado> {
    caso::cargar(&Host, &banco()).expect("el banco debe cargar")
}

fn de_clase(clase: &str) -> Vec<CasoCargado> {
    casos().into_iter().filter(|c| c.caso.clase == clase).collect()
}

fn hay(programa: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p).any(|d| d.join(programa).exists())
        })
        .unwrap_or(false)
}

#[test]
fn la_distribucion_es_10_10_5_con_ids_unicos() {
    let casos = casos();
    assert_eq!(casos.len(), 25);
    assert_eq!(de_clase("programacion").len(), 10);
    assert_eq!(de_clase("protocolo").len(), 10);
    assert_eq!(de_clase("repo").len(), 5);

    let mut ids: Vec<String> = casos.iter().map(|c| c.caso.id.clone()).collect();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), total, "hay identificadores repetidos");
}

#[test]
fn el_banco_no_tiene_problemas() {
    let problemas = caso::comprobar(&Host, &banco(), &reservado(), &casos()).unwrap();
    assert_eq!(problemas, Vec::<String>::new());
}

#[test]
fn la_particion_sale_de_la_regla_y_reparte() {
    for c in casos() {
        assert_eq!(
            c.caso.particion,
            caso::particion_de(&c.caso.id),
            "{}: la partición no sale del identificador",
            c.caso.id
        );
    }
    for clase in caso::CLASES {
        let de_esta = de_clase(clase);
        let reservados = de_esta
            .iter()
            .filter(|c| c.caso.particion == "reservado")
            .count();
        assert!(reservados > 0, "clase {clase} sin casos reservados");
        assert!(reservados < de_esta.len(), "clase {clase} entera reservada");
    }
}

#[test]
fn la_huella_del_manifiesto_corresponde() {
    let manifiesto = caso::leer_manifiesto(&Host, &banco()).unwrap();
    assert_eq!(manifiesto.huella, caso::huella(&casos()));
    // Los umbrales no los fija T02: medir es T14.
    let umbrales = manifiesto.resto.get("umbrales").expect("umbrales declarados");
    assert_eq!(umbrales["estado"], "no fijados");
}

#[test]
fn los_hashes_de_entrada_sellan_lo_visible() {
    for c in casos() {
        let esperados = caso::hashes_entrada(&Host, &banco(), &c.caso).unwrap();
        assert_eq!(c.caso.hashes_entrada, esperados, "{}", c.caso.id);
        assert!(!c.caso.hashes_entrada.is_empty());
    }
}

#[test]
fn el_paquete_visible_no_contiene_material_reservado() {
    for c in casos() {
        let visibles = c.caso.paquete_visible().unwrap();
        assert!(!visibles.is_empty());
        for v in &visibles {
            assert!(v.starts_with("visible/"), "{}: {v}", c.caso.id);
            assert!(Host.existe(&unir(&banco(), v)), "{}: falta {v}", c.caso.id);
            assert!(!c.caso.reservado.contains(v));
        }
        for arg in &c.caso.comprobador.argv {
            assert!(!arg.starts_with("reservado/"), "{}: {arg}", c.caso.id);
        }
    }
}

#[test]
fn lo_visible_no_filtra_la_solucion() {
    for c in casos() {
        let mut visible = String::new();
        for v in c.caso.paquete_visible().unwrap() {
            visible.push_str(&String::from_utf8_lossy(
                &Host.leer(&unir(&banco(), &v)).unwrap(),
            ));
        }
        for r in &c.caso.reservado {
            let nombre = r.rsplit('/').next().unwrap();
            if nombre != "referencia.rs" && nombre != "referencia.patch" {
                continue;
            }
            let solucion = String::from_utf8_lossy(
                &Host
                    .leer(&unir(&reservado(), &format!("{}/{nombre}", c.caso.id)))
                    .unwrap(),
            )
            .into_owned();
            for linea in solucion.lines() {
                let linea = linea.trim();
                if linea.len() <= 25
                    || linea.starts_with("//")
                    || linea.starts_with('-')
                    || linea.starts_with('+')
                    || linea.starts_with('@')
                    || linea.starts_with('#')
                {
                    continue;
                }
                assert!(
                    !visible.contains(linea),
                    "{}: el enunciado repite una línea de la solución: {linea}",
                    c.caso.id
                );
            }
        }
    }
}

/// Una campaña real guarda lo reservado fuera del checkout del agente.
#[test]
fn la_raiz_reservada_se_puede_mover_fuera() {
    let tmp = Temporal::nuevo("banco-reservado").unwrap();
    let destino = unir(&tmp.ruta(), "oculto");
    copiar_arbol(Path::new(&reservado()), Path::new(&destino));

    let esperado: protocolo::Esperado = serde_json::from_slice(
        &Host.leer(&unir(&destino, "Q01/esperado.json")).unwrap(),
    )
    .unwrap();
    let respuesta: serde_json::Value = serde_json::from_slice(
        &Host.leer(&unir(&destino, "Q01/correcta.json")).unwrap(),
    )
    .unwrap();
    let informe = protocolo::verificar("Q01", &esperado, &respuesta).unwrap();
    assert_eq!(informe.estado, "ok", "{informe:?}");
}

fn copiar_arbol(origen: &Path, destino: &Path) {
    std::fs::create_dir_all(destino).unwrap();
    for entrada in std::fs::read_dir(origen).unwrap() {
        let entrada = entrada.unwrap();
        let destino_hijo = destino.join(entrada.file_name());
        if entrada.file_type().unwrap().is_dir() {
            copiar_arbol(&entrada.path(), &destino_hijo);
        } else {
            std::fs::copy(entrada.path(), destino_hijo).unwrap();
        }
    }
}

// --- discriminación ---------------------------------------------------------

fn verificar_programa(id: &str, archivo: &str) -> programa::Informe {
    let vectores =
        programa::leer_vectores(&Host, &unir(&reservado(), &format!("{id}/vectores.json"))).unwrap();
    let fuente = Host.leer(&unir(&reservado(), &format!("{id}/{archivo}"))).unwrap();
    let trabajo = Temporal::nuevo(&format!("banco-{id}")).unwrap();
    let compilador = |fuente: &str, destino: &str| -> Vec<String> {
        vec![
            "rustc".into(),
            "--edition".into(),
            "2021".into(),
            "-O".into(),
            "-o".into(),
            destino.into(),
            fuente.into(),
        ]
    };
    let mut host = Host;
    programa::verificar(
        id,
        &vectores,
        &fuente,
        &mut host,
        &Host,
        &trabajo.ruta(),
        &compilador,
        30,
    )
    .unwrap()
}

#[test]
fn la_referencia_pasa_todos_los_vectores() {
    if !hay("rustc") {
        eprintln!("sin rustc: se omite la compilación de candidatos");
        return;
    }
    for c in de_clase("programacion") {
        let informe = verificar_programa(&c.caso.id, "referencia.rs");
        assert_eq!(informe.estado, "ok", "{}: {informe:?}", c.caso.id);
        assert_eq!(informe.pasados, informe.total);
        assert!(informe.total >= 2, "un caso con un solo vector no mide nada");
    }
}

#[test]
fn la_solucion_incorrecta_se_detecta() {
    if !hay("rustc") {
        eprintln!("sin rustc: se omite la compilación de candidatos");
        return;
    }
    for c in de_clase("programacion") {
        let informe = verificar_programa(&c.caso.id, "incorrecta.rs");
        assert_eq!(informe.estado, "fallo", "{}: {informe:?}", c.caso.id);
        assert_eq!(informe.codigo(), 2);
        assert!(informe.total > informe.pasados);
    }
}

#[test]
fn un_candidato_que_no_compila_es_error_no_fallo() {
    if !hay("rustc") {
        return;
    }
    let vectores =
        programa::leer_vectores(&Host, &unir(&reservado(), "P01/vectores.json")).unwrap();
    let trabajo = Temporal::nuevo("banco-nocompila").unwrap();
    let compilador = |fuente: &str, destino: &str| -> Vec<String> {
        vec![
            "rustc".into(),
            "--edition".into(),
            "2021".into(),
            "-o".into(),
            destino.into(),
            fuente.into(),
        ]
    };
    let mut host = Host;
    let informe = programa::verificar(
        "P01",
        &vectores,
        b"fn main() { esto no es rust }\n",
        &mut host,
        &Host,
        &trabajo.ruta(),
        &compilador,
        30,
    )
    .unwrap();
    assert_eq!(informe.estado, "error");
    assert_eq!(informe.codigo(), 1, "no compilar no es lo mismo que fallar");
    assert!(informe.compilacion.is_some());
}

fn verificar_protocolo(id: &str, archivo: &str) -> protocolo::Informe {
    let esperado: protocolo::Esperado = serde_json::from_slice(
        &Host.leer(&unir(&reservado(), &format!("{id}/esperado.json"))).unwrap(),
    )
    .unwrap();
    let respuesta: serde_json::Value = serde_json::from_slice(
        &Host.leer(&unir(&reservado(), &format!("{id}/{archivo}"))).unwrap(),
    )
    .unwrap();
    protocolo::verificar(id, &esperado, &respuesta).unwrap()
}

#[test]
fn la_respuesta_correcta_cumple_y_la_incorrecta_se_detecta() {
    for c in de_clase("protocolo") {
        let id = &c.caso.id;
        let buena = verificar_protocolo(id, "correcta.json");
        assert_eq!(buena.estado, "ok", "{id}: {buena:?}");
        assert!(buena.total >= 3, "{id}: pocas aserciones");

        let mala = verificar_protocolo(id, "incorrecta.json");
        assert_eq!(mala.estado, "fallo", "{id}: {mala:?}");
        assert!(mala.total > mala.pasadas);
    }
}

#[test]
fn el_unicode_partido_entre_trozos_se_reensambla() {
    let crudo: serde_json::Value =
        serde_json::from_slice(&Host.leer(&unir(&reservado(), "Q06/correcta.json")).unwrap())
            .unwrap();
    let trozos = crudo["trozos_b64"].as_array().unwrap();
    assert!(trozos.len() > 1, "el caso exige más de un trozo");
    let doc = protocolo::Documento::nuevo(&crudo).unwrap();
    assert!(doc.errores_formato.is_empty(), "{:?}", doc.errores_formato);
    assert!(doc.texto().contains('☕'));
    assert!(!doc.texto().contains('\u{fffd}'));
}

// --- casos de repo ----------------------------------------------------------

#[test]
fn cada_caso_de_repo_declara_base_rutas_y_aceptacion() {
    for c in de_clase("repo") {
        let caso = &c.caso;
        let base = caso.base.as_ref().expect("base declarada");
        assert_eq!(base.commit.len(), 40, "{}", caso.id);
        let rutas = caso.rutas_editables.as_ref().expect("rutas editables");
        assert!(!rutas.is_empty());
        for r in rutas {
            assert!(!r.starts_with('/'));
            assert!(!r.starts_with("tests/self-improvement"));
        }
        let aceptacion = caso.aceptacion.as_ref().expect("aceptación");
        assert_eq!(aceptacion.argv[0], "cargo");
        assert!(aceptacion.destino_prueba.ends_with(".rs"));
        assert!(aceptacion.destino_prueba.contains(&caso.id));
        let origen = caso.origen.as_deref().unwrap_or("");
        assert!(origen == "reproduccion" || origen == "mejora_especificada", "{origen}");
    }
}

#[test]
fn el_parche_de_referencia_solo_toca_rutas_editables() {
    for c in de_clase("repo") {
        let caso = &c.caso;
        let parche = String::from_utf8_lossy(
            &Host
                .leer(&unir(&reservado(), &format!("{}/referencia.patch", caso.id)))
                .unwrap(),
        )
        .into_owned();
        assert!(parche.starts_with("diff --git"), "{}", caso.id);
        let mut tocadas: Vec<String> = parche
            .lines()
            .filter(|l| l.starts_with("diff --git"))
            .filter_map(|l| l.split(" b/").nth(1).map(|s| s.trim().to_string()))
            .collect();
        tocadas.sort();
        let mut editables = caso.rutas_editables.clone().unwrap();
        editables.sort();
        assert_eq!(tocadas, editables, "{}", caso.id);

        assert!(Host.existe(&unir(&reservado(), &format!("{}/aceptacion.rs", caso.id))));
    }
}
