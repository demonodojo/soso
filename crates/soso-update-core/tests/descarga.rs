//! U4: descarga durable y reanudable. El criterio es que un corte de red y un
//! reinicio reanuden **sólo lo pendiente verificado**, con RAM acotada y sin
//! haber tocado el sistema activo.

use soso_update_core::descarga::{self, Etapa, TROZO_MAX};
use soso_update_core::manifest::{Manifest, MANIFEST_MAGIC};
use soso_update_core::txn::{preflight, Capacidad, PreflightError, TxnId};
use std::collections::BTreeSet;

fn manifiesto(files: &[(&str, u64)]) -> (Manifest, String) {
    let mut texto = format!(
        "{MANIFEST_MAGIC}\nversion=0.3.0\nbuild=abc\nfecha=2026-09-17\n\
         arch=x86_64\nperfil=live-usb\nabi=1\nfs=sosofs1\nmin_shim=1\nmin_recuperador=1\n\
         kernel {} 1000\n",
        "a".repeat(64)
    );
    let total: u64 = files.iter().map(|(_, n)| *n).sum();
    texto.push_str(&format!("pack {} {total}\n", "b".repeat(64)));
    let mut off = 0u64;
    for (i, (p, n)) in files.iter().enumerate() {
        texto.push_str(&format!("f {} {off} {n} {p}\n", format!("{i:02}").repeat(32)));
        off += n;
    }
    (Manifest::parse(&texto).expect("manifiesto"), texto)
}

// ── Reanudación ──────────────────────────────────────────────────────────

#[test]
fn solo_se_baja_lo_que_falta() {
    let (man, _) = manifiesto(&[("bin/a", 100), ("bin/b", 200), ("bin/c", 50)]);
    let listos: BTreeSet<&str> = ["bin/a"].into_iter().collect();
    let p = descarga::pendientes(&man, |f| listos.contains(f.path.as_str()));
    assert_eq!(p.len(), 2);
    assert_eq!(descarga::bytes_pendientes(&p), 250);

    // Y una segunda pasada con todo verificado no baja nada.
    let p = descarga::pendientes(&man, |_| true);
    assert!(p.is_empty());
    assert_eq!(descarga::bytes_pendientes(&p), 0);
}

#[test]
fn un_fichero_a_medias_no_cuenta_como_hecho() {
    // `ya_listo` sólo dice que sí con tamaño **y** hash correctos: un fichero
    // truncado por un corte tiene que volver a bajarse entero.
    let (man, _) = manifiesto(&[("bin/a", 100), ("bin/b", 200)]);
    let mut en_disco = std::collections::BTreeMap::new();
    en_disco.insert("bin/a", 100u64);
    en_disco.insert("bin/b", 120u64); // se cortó a mitad
    let p = descarga::pendientes(&man, |f| en_disco.get(f.path.as_str()) == Some(&f.size));
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].path, "bin/b");
}

// ── Identidad de la release ──────────────────────────────────────────────

#[test]
fn la_etapa_esta_atada_a_una_release_concreta() {
    let (man, texto) = manifiesto(&[("bin/a", 100)]);
    let etapa = Etapa::nueva(TxnId::from_manifest(texto.as_bytes()), &man.version_raw, man.pack_size);
    assert!(etapa.sirve_para(texto.as_bytes()));

    // Otro build de la **misma versión** no sirve: los offsets del pack son de
    // otro fichero, y reutilizar lo bajado mezclaría dos releases.
    let otro = texto.replace("build=abc", "build=def");
    assert_eq!(
        Manifest::parse(&otro).unwrap().version_raw,
        man.version_raw,
        "misma versión, distinto build"
    );
    assert!(!etapa.sirve_para(otro.as_bytes()));
}

#[test]
fn la_etapa_sobrevive_al_reinicio() {
    let (man, texto) = manifiesto(&[("bin/a", 100)]);
    let etapa = Etapa::nueva(TxnId::from_manifest(texto.as_bytes()), "0.3.0", man.pack_size);
    let bytes = etapa.format();
    // Lo que se leería tras reiniciar.
    let leida = Etapa::parse(&bytes).unwrap();
    assert_eq!(leida, etapa);
    assert!(leida.sirve_para(texto.as_bytes()));

    // Y un registro roto no se toma por bueno.
    let mut roto = bytes.clone();
    roto[20] ^= 0x01;
    assert!(Etapa::parse(&roto).is_err());
}

// ── RAM acotada ──────────────────────────────────────────────────────────

#[test]
fn los_trozos_acotan_lo_que_hay_en_memoria() {
    let t = descarga::trozos(0, 10 * 1024 * 1024, TROZO_MAX);
    assert_eq!(t.len(), 10);
    assert!(t.iter().all(|(_, n)| *n <= TROZO_MAX));
    assert_eq!(t.iter().map(|(_, n)| n).sum::<u64>(), 10 * 1024 * 1024);
    // Contiguos y en orden: un hueco sería un fichero corrupto silencioso.
    for par in t.windows(2) {
        assert_eq!(par[0].0 + par[0].1, par[1].0);
    }
}

#[test]
fn los_trozos_respetan_el_principio_y_el_final() {
    assert_eq!(descarga::trozos(500, 300, 128), vec![(500, 128), (628, 128), (756, 44)]);
    assert!(descarga::trozos(0, 0, 128).is_empty());
    assert!(descarga::trozos(0, 100, 0).is_empty(), "un máximo de 0 no puede girar");
}

// ── Comprobación previa ──────────────────────────────────────────────────

#[test]
fn la_necesidad_cuenta_respaldo_y_crecimiento() {
    let (man, _) = manifiesto(&[("bin/a", 100), ("bin/b", 200)]);
    let pend = descarga::pendientes(&man, |_| false);
    // `bin/a` ya existe midiendo 60; `bin/b` es nuevo.
    let n = descarga::necesidad(&man, &pend, &pend, |p| (p == "bin/a").then_some(60));
    assert_eq!(n.preparacion, 300 + 1000, "los dos ficheros y el kernel");
    assert_eq!(n.respaldo, 60, "hay que poder deshacer lo que se reemplaza");
    assert_eq!(n.crecimiento, 40 + 200);
    assert_eq!(n.kernel, 1000);
}

#[test]
fn un_fichero_que_encoge_no_cuenta_como_crecimiento() {
    let (man, _) = manifiesto(&[("bin/a", 100)]);
    let pend = descarga::pendientes(&man, |_| false);
    let n = descarga::necesidad(&man, &pend, &pend, |_| Some(5_000));
    assert_eq!(n.crecimiento, 0, "no se resta espacio que no se libera hasta aplicar");
    assert_eq!(n.respaldo, 5_000);
}

#[test]
fn sin_espacio_la_operacion_no_empieza() {
    let (man, _) = manifiesto(&[("bin/a", 8 * 1024 * 1024)]);
    let pend = descarga::pendientes(&man, |_| false);
    let n = descarga::necesidad(&man, &pend, &pend, |_| None);
    let cap = Capacidad {
        sosofs_libre: 4 * 1024 * 1024,
        hueco_kernel: 64 * 1024 * 1024,
        registro_arranque: soso_update_core::UPD_BOOTREC_SIZE,
        meta_kernel: soso_update_core::UPD_KERNEL_META_SIZE,
    };
    assert!(matches!(
        preflight(&n, &cap),
        Err(PreflightError::SinEspacio { .. })
    ));

    // Con sitio de sobra, adelante.
    let cap = Capacidad { sosofs_libre: 1 << 30, ..cap };
    assert_eq!(preflight(&n, &cap), Ok(()));
}

#[test]
fn reanudar_pide_menos_espacio_que_empezar() {
    // Lo ya bajado y verificado no se vuelve a contar: si no, una operación
    // reanudada podría fallar el preflight por espacio que ya está ocupado.
    let (man, _) = manifiesto(&[("bin/a", 1_000_000), ("bin/b", 1_000_000)]);
    // `cambian` es el mismo conjunto en los dos casos —lo que va a cambiar no
    // depende de lo ya bajado—; lo que encoge al reanudar es `pendientes`.
    let todos = descarga::pendientes(&man, |_| false);
    let todo = descarga::necesidad(&man, &todos, &todos, |_| None);
    let listos: BTreeSet<&str> = ["bin/a"].into_iter().collect();
    let medio = descarga::necesidad(
        &man,
        &todos,
        &descarga::pendientes(&man, |f| listos.contains(f.path.as_str())),
        |_| None,
    );
    assert!(medio.preparacion < todo.preparacion);
    assert_eq!(todo.preparacion - medio.preparacion, 1_000_000);
}

#[test]
fn un_manifiesto_sin_ficheros_sigue_necesitando_el_kernel() {
    let (man, _) = manifiesto(&[]);
    let n = descarga::necesidad(&man, &[], &[], |_| None);
    assert_eq!(n.kernel, 1000);
    assert_eq!(n.preparacion, 1000);
    assert_eq!(n.respaldo, 0);
}

// ── Lo que no se toca ────────────────────────────────────────────────────

#[test]
fn el_plan_de_descarga_no_conoce_rutas_del_sistema() {
    // Comprobación de forma, no de comportamiento: `descarga` decide qué bajar
    // y cuánto ocupa, y no tiene forma de escribir en `/bin` ni en `/lib`. Que
    // bajar no cambie el sistema activo es una propiedad de este reparto.
    let fuente = include_str!("../src/descarga.rs");
    for prohibido in ["/bin/", "/lib/", "/etc/"] {
        assert!(!fuente.contains(prohibido), "descarga.rs menciona {prohibido}");
    }
}

#[test]
fn lo_que_no_cambia_no_se_respalda() {
    // Una release trae 120 MB de firmware que no cambia y 1 MB de binarios que
    // sí. El respaldo sólo tiene que cubrir lo que se reemplaza: contando el
    // manifiesto entero, `aplicar` exigía ~168 MB libres para escribir ~20 y se
    // negaba en cualquier instalación con el rootfs de 384 MiB (2026-09-21).
    let (man, _) = manifiesto(&[("lib/firmware/gsp.bin", 120_000_000), ("bin/init", 1_000_000)]);
    let cambian: Vec<_> = man
        .files
        .iter()
        .filter(|f| f.path == "bin/init")
        .cloned()
        .collect();
    let n = descarga::necesidad(&man, &cambian, &cambian, |_| Some(1_000_000));
    assert_eq!(n.respaldo, 1_000_000, "sólo se respalda lo que se reemplaza");
    assert_eq!(n.preparacion, 1_000_000 + 1000, "y sólo se baja eso más el kernel");
}
