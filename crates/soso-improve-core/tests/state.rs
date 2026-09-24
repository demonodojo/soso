//! T23 — formato de tareas y estados del coordinador.
//!
//! Lo que se comprueba aquí no es que los tipos compilen, sino las decisiones
//! que después no se pueden cambiar sin romper estados ya escritos: quién puede
//! aceptar, qué pasa con lo que no se midió, y si un corte a mitad de escritura
//! deja la ejecución legible o la pierde.

use std::collections::BTreeMap;

use soso_improve_core::durable::Durable;
use soso_improve_core::entorno::{Archivos, Entrada, Tipo};
use soso_improve_core::state::*;
use soso_improve_core::tiempo::RelojSimulado;
use soso_improve_core::{Error, Resultado, ESQUEMA};

// ---------------------------------------------------------------------------
// Árbol en memoria con inyección de fallos
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Memoria {
    ficheros: BTreeMap<String, Vec<u8>>,
    /// Si está, la próxima creación guarda sólo estos bytes y falla: un corte
    /// de corriente a media escritura.
    cortar_en: Option<usize>,
    /// Si está, la próxima creación falla sin dejar nada.
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
                self.ficheros
                    .insert(ruta.to_string(), datos[..corte].to_vec());
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

fn spec() -> TaskSpec {
    TaskSpec {
        schema_version: ESQUEMA,
        id: "T99-ejemplo".into(),
        base: "a".repeat(64),
        problema: "el shard llm-moe no arranca".into(),
        rutas_editables: vec!["kernel/src/net/mod.rs".into()],
        comprobaciones: vec![Comprobacion {
            argv: vec!["cargo".into(), "test".into(), "-p".into(), "soso-net".into()],
            cwd: ".".into(),
            timeout_s: 900,
        }],
        limites: Limites::default(),
        hashes_entrada: BTreeMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Formato
// ---------------------------------------------------------------------------

#[test]
fn ida_y_vuelta_conserva_el_estado() {
    let mut m = RunManifest::nuevo("r-1", "T99-ejemplo", 1_000).unwrap();
    m.transicion(Estado::Reproduciendo, Autoridad::Coordinador)
        .unwrap();
    m.abrir_intento(&Limites::default(), 1_100).unwrap();
    m.anotar_artefacto("tasks/T99/intento-0/salida.log").unwrap();
    m.cerrar_intento(
        Estado::Verificando,
        Medida::Valor(12),
        Medida::desconocida("el endpoint no devolvió usage"),
        1_900,
    )
    .unwrap();

    let json = serde_json::to_vec(&m).unwrap();
    let vuelta: RunManifest = serde_json::from_slice(&json).unwrap();
    assert_eq!(m, vuelta);
    vuelta.validar().unwrap();

    // Lo que no se midió tiene que seguir sin medirse al otro lado: si el
    // round-trip lo convirtiera en 0, el informe diría que no gastó tokens.
    let t = &vuelta.intentos[0].tokens;
    assert_eq!(t.valor(), None, "{t:?}");
    assert!(String::from_utf8_lossy(&json).contains("usage"), "el motivo tiene que viajar");
    assert_eq!(vuelta.intentos[0].herramientas.valor(), Some(12));
}

#[test]
fn un_esquema_desconocido_se_rechaza_en_vez_de_interpretarse() {
    let mut m = RunManifest::nuevo("r-1", "T99", 0).unwrap();
    m.schema_version = ESQUEMA + 7;
    let e = m.validar().unwrap_err();
    assert!(format!("{e:?}").contains("esquema"), "{e:?}");

    let mut s = spec();
    s.schema_version = 0;
    assert!(s.validar().is_err());
    // Y el del día sí pasa: la comprobación anterior no vale nada si esto falla.
    spec().validar().unwrap();
}

#[test]
fn el_spec_exige_plazo_rutas_y_comprobaciones() {
    let mut s = spec();
    s.comprobaciones[0].timeout_s = 0;
    assert!(
        format!("{:?}", s.validar().unwrap_err()).contains("plazo cero"),
        "una espera sin tope es como se cuelga una campaña entera"
    );

    let mut s = spec();
    s.rutas_editables = vec!["../fuera".into()];
    assert!(s.validar().is_err(), "una ruta editable no sale del árbol");

    let mut s = spec();
    s.comprobaciones.clear();
    assert!(s.validar().is_err(), "sin comprobaciones no se acepta nada");
}

// ---------------------------------------------------------------------------
// Transiciones
// ---------------------------------------------------------------------------

#[test]
fn solo_el_validador_acepta() {
    let mut m = RunManifest::nuevo("r-1", "T99", 0).unwrap();
    m.transicion(Estado::Reproduciendo, Autoridad::Coordinador)
        .unwrap();
    m.transicion(Estado::Editando, Autoridad::Coordinador)
        .unwrap();
    m.transicion(Estado::Verificando, Autoridad::Coordinador)
        .unwrap();

    let e = m
        .transicion(Estado::Aceptada, Autoridad::Coordinador)
        .unwrap_err();
    assert!(format!("{e:?}").contains("validador"), "{e:?}");
    assert_eq!(m.estado, Estado::Verificando, "el rechazo no deja rastro");

    m.transicion(Estado::Aceptada, Autoridad::Validador).unwrap();
    assert_eq!(m.estado, Estado::Aceptada);
}

#[test]
fn una_transicion_invalida_no_cambia_el_estado() {
    let mut m = RunManifest::nuevo("r-1", "T99", 0).unwrap();
    // Pendiente → Verificando se salta reproducir y editar: no hay nada que
    // verificar todavía.
    let e = m
        .transicion(Estado::Verificando, Autoridad::Coordinador)
        .unwrap_err();
    assert!(format!("{e:?}").contains("no permitida"), "{e:?}");
    assert_eq!(m.estado, Estado::Pendiente);

    // Y de un terminal no se sale ni siendo el validador.
    let mut m = RunManifest::nuevo("r-2", "T99", 0).unwrap();
    m.transicion(Estado::Rechazada, Autoridad::Coordinador)
        .unwrap();
    assert!(m
        .transicion(Estado::Editando, Autoridad::Validador)
        .is_err());
    assert_eq!(m.estado, Estado::Rechazada);
}

#[test]
fn los_intentos_se_agotan() {
    let limites = Limites {
        intentos_max: 2,
        ..Default::default()
    };
    let mut m = RunManifest::nuevo("r-1", "T99", 0).unwrap();
    for i in 0..2 {
        m.abrir_intento(&limites, i * 10).unwrap();
        m.cerrar_intento(
            Estado::Editando,
            Medida::Valor(1),
            Medida::Valor(2),
            i * 10 + 5,
        )
        .unwrap();
    }
    let e = m.abrir_intento(&limites, 100).unwrap_err();
    assert!(format!("{e:?}").contains("agotados"), "{e:?}");
    assert_eq!(m.intentos.len(), 2);
}

#[test]
fn en_el_estado_van_referencias_no_logs() {
    let mut m = RunManifest::nuevo("r-1", "T99", 0).unwrap();
    m.abrir_intento(&Limites::default(), 0).unwrap();
    let log = "línea de salida\n".repeat(400);
    let e = m.anotar_artefacto(&log).unwrap_err();
    assert!(format!("{e:?}").contains("referencias"), "{e:?}");
}

// ---------------------------------------------------------------------------
// Persistencia
// ---------------------------------------------------------------------------

#[test]
fn se_guarda_y_se_recupera() {
    let mut d = Memoria::default();
    let m = RunManifest::nuevo("r-1", "T99", 7).unwrap();
    guardar(&mut d, "/var/estado", &m, None).unwrap();

    let (leido, hash) = cargar(&d, "/var/estado", "r-1").unwrap().unwrap();
    assert_eq!(leido, m);

    // El hash es la precondición del siguiente escritor: sin él, dos
    // coordinadores se pisan sin enterarse.
    let mut avanzado = leido;
    avanzado
        .transicion(Estado::Reproduciendo, Autoridad::Coordinador)
        .unwrap();
    guardar(&mut d, "/var/estado", &avanzado, Some(&hash)).unwrap();
    assert_eq!(
        cargar(&d, "/var/estado", "r-1").unwrap().unwrap().0.estado,
        Estado::Reproduciendo
    );

    // Y un escritor que traiga el hash viejo se queda fuera.
    let mut rezagado = m.clone();
    rezagado
        .transicion(Estado::Bloqueada, Autoridad::Coordinador)
        .unwrap();
    assert!(guardar(&mut d, "/var/estado", &rezagado, Some(&hash)).is_err());

    // Una ejecución que no existe es `None`, no un estado vacío.
    assert!(cargar(&d, "/var/estado", "r-inexistente").unwrap().is_none());
}

#[test]
fn una_escritura_cortada_no_pierde_el_estado_anterior() {
    let mut d = Memoria::default();
    let mut m = RunManifest::nuevo("r-1", "T99", 0).unwrap();
    guardar(&mut d, "/var/estado", &m, None).unwrap();
    let (_, hash) = cargar(&d, "/var/estado", "r-1").unwrap().unwrap();

    m.transicion(Estado::Reproduciendo, Autoridad::Coordinador)
        .unwrap();
    d.cortar_en = Some(20); // se escribe media generación y se corta la luz
    assert!(guardar(&mut d, "/var/estado", &m, Some(&hash)).is_err());

    // Lo importante: el anterior sigue ahí y se lee entero. Si el formato
    // sobrescribiera en sitio, esto devolvería basura o nada.
    let (recuperado, hash2) = cargar(&d, "/var/estado", "r-1").unwrap().unwrap();
    assert_eq!(recuperado.estado, Estado::Pendiente);
    assert_eq!(hash2, hash);

    // Y se puede seguir escribiendo **sin limpiar nada**: T63. Antes de esa
    // ficha, aquí fallaba con «ya existe» y seguía fallando para siempre,
    // porque `publicar` numeraba desde la vigente íntegra y volvía a chocar
    // con el resto del corte. La ejecución quedaba encallada en el último
    // estado bueno, que es exactamente lo que hace imposible reanudar (T27).
    guardar(&mut d, "/var/estado", &m, Some(&hash)).unwrap();
    let (tras_el_corte, _) = cargar(&d, "/var/estado", "r-1").unwrap().unwrap();
    assert_eq!(tras_el_corte.estado, Estado::Reproduciendo);
}

#[test]
fn un_disco_lleno_falla_sin_tocar_lo_publicado() {
    let mut d = Memoria::default();
    let m = RunManifest::nuevo("r-1", "T99", 0).unwrap();
    guardar(&mut d, "/var/estado", &m, None).unwrap();
    let (_, hash) = cargar(&d, "/var/estado", "r-1").unwrap().unwrap();

    let mut avanzado = m.clone();
    avanzado
        .transicion(Estado::Reproduciendo, Autoridad::Coordinador)
        .unwrap();
    d.sin_espacio = true;
    assert!(guardar(&mut d, "/var/estado", &avanzado, Some(&hash)).is_err());
    assert_eq!(
        cargar(&d, "/var/estado", "r-1").unwrap().unwrap().0,
        m,
        "un fallo de escritura no puede costar la ejecución"
    );
}

#[test]
fn dos_ejecuciones_con_el_mismo_id_chocan_en_vez_de_mezclarse() {
    let mut d = Memoria::default();
    let a = RunManifest::nuevo("r-1", "T99", 0).unwrap();
    guardar(&mut d, "/var/estado", &a, None).unwrap();

    // Otro coordinador arranca y cree que ese id es suyo (`esperado: None`).
    let b = RunManifest::nuevo("r-1", "T98-otra", 500).unwrap();
    let e = guardar(&mut d, "/var/estado", &b, None).unwrap_err();
    assert!(format!("{e:?}").contains("cambió"), "{e:?}");

    // El primero sigue intacto, con su tarea y no la del intruso.
    assert_eq!(
        cargar(&d, "/var/estado", "r-1").unwrap().unwrap().0.task_id,
        "T99"
    );
}

#[test]
fn el_reloj_y_los_ids_se_inyectan() {
    let reloj = RelojSimulado::nuevo(0x1234);
    let mut ids = IdsDelReloj {
        reloj: &reloj,
        prefijo: "run",
    };
    let primero = ids.nuevo_run_id();
    assert_eq!(primero, "run-1234");
    reloj.avanzar(1);
    assert_ne!(ids.nuevo_run_id(), primero);
}

#[test]
fn un_id_no_puede_ser_una_ruta_disfrazada() {
    // Los ids acaban siendo nombres de fichero bajo el directorio de estado.
    assert!(RunManifest::nuevo("../../etc/passwd", "T99", 0).is_err());
    assert!(RunManifest::nuevo("r/1", "T99", 0).is_err());
    assert!(RunManifest::nuevo("", "T99", 0).is_err());
    let d = Memoria::default();
    assert!(cargar(&d, "/var/estado", "../otro").is_err());
}

#[test]
fn un_estado_ilegible_no_se_confunde_con_uno_inexistente() {
    let mut d = Memoria::default();
    let m = RunManifest::nuevo("r-1", "T99", 0).unwrap();
    guardar(&mut d, "/var/estado", &m, None).unwrap();

    // Se corrompe el contenido conservando el envoltorio íntegro: el error es
    // del formato, no del almacén. Devolver `None` aquí haría que el
    // coordinador repitiera efectos ya ejecutados.
    let ruta = d
        .ficheros
        .keys()
        .find(|k| k.contains("run-r-1"))
        .cloned()
        .unwrap();
    let crudo = d.ficheros[&ruta].clone();
    let corte = crudo.iter().position(|b| *b == b'\n').unwrap() + 1;
    let mut roto = crudo[..corte].to_vec();
    roto.extend_from_slice(b"{\"esto\": \"no es un manifiesto\"}");
    // Rehacemos el hash del envoltorio para que el fallo sea del JSON.
    let cuerpo = &roto[corte..];
    let nuevo = format!(
        "{}\n{}",
        soso_improve_core::sha256_hex(cuerpo),
        String::from_utf8_lossy(cuerpo)
    );
    d.ficheros.insert(ruta, nuevo.into_bytes());

    let e = cargar(&d, "/var/estado", "r-1").unwrap_err();
    assert!(format!("{e:?}").contains("ilegible"), "{e:?}");
}
