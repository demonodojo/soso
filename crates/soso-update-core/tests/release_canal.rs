//! U3: contrato de release — canales y precedencia, versiones, e inventario.
//!
//! El criterio de cierre es que **ninguna release contenga logs, claves,
//! cachés ni estado de una OTA**, y que la URL de la que se baja sea la que el
//! usuario pidió. Las dos cosas se rompen en silencio: una release con el
//! `authorized_key` de la máquina que empaquetó no falla, y una URL mal armada
//! sólo da un 404 raro.

use soso_update_core::canal::{self, Conf, Origen, CANAL_DEV, CANAL_STABLE};
use soso_update_core::pack::PackWriter;
use soso_update_core::semver::{cmp, parse};
use soso_update_core::{por_que_se_excluye, Excluido};

// ── Canales y URL ────────────────────────────────────────────────────────

#[test]
fn el_canal_estable_usa_latest_y_dev_es_una_etiqueta() {
    assert_eq!(
        canal::url_de_canal(CANAL_STABLE),
        "https://github.com/demonodojo/soso/releases/latest/download"
    );
    // La forma anterior metía un `/download` de más y pedía
    // `…/download/dev/download/manifest.txt`, que no existe en ningún sitio.
    assert_eq!(
        canal::url_de_canal(CANAL_DEV),
        "https://github.com/demonodojo/soso/releases/download/dev"
    );
    // Cualquier otro nombre se trata como etiqueta publicada.
    assert_eq!(
        canal::url_de_canal("v0.3.0"),
        "https://github.com/demonodojo/soso/releases/download/v0.3.0"
    );
}

#[test]
fn la_url_del_artefacto_se_arma_sin_barras_de_mas() {
    let o = Origen::Remoto("https://ejemplo/soso/".into());
    assert_eq!(o.artefacto("manifest.txt"), "https://ejemplo/soso/manifest.txt");
    let o = Origen::Remoto("https://ejemplo/soso".into());
    assert_eq!(o.artefacto("rootfs.pack"), "https://ejemplo/soso/rootfs.pack");
    let o = Origen::Local("/var/actualiza-prueba/".into());
    assert_eq!(o.artefacto("kernel-x86_64"), "/var/actualiza-prueba/kernel-x86_64");
}

#[test]
fn precedencia_local_gana_a_todo() {
    let conf = Conf { url: Some("https://espejo/soso".into()), canal: Some("dev".into()) };
    assert_eq!(
        canal::resolver(Some("/var/prueba"), Some("dev"), &conf),
        Origen::Local("/var/prueba".into())
    );
    assert_eq!(canal::motivo(Some("/var/prueba"), Some("dev"), &conf), "--local");
}

#[test]
fn el_channel_de_la_orden_gana_al_url_del_fichero() {
    // Era al revés: con `url=` en el fichero, `--channel dev` se ignoraba en
    // silencio y se bajaba de stable.
    let conf = Conf { url: Some("https://espejo/soso".into()), canal: None };
    assert_eq!(
        canal::resolver(None, Some("dev"), &conf),
        Origen::Remoto(canal::url_de_canal(CANAL_DEV))
    );
    assert_eq!(canal::motivo(None, Some("dev"), &conf), "--channel");
}

#[test]
fn sin_orden_manda_el_fichero_y_el_url_gana_al_canal() {
    let conf = Conf { url: Some("https://espejo/soso".into()), canal: Some("dev".into()) };
    assert_eq!(
        canal::resolver(None, None, &conf),
        Origen::Remoto("https://espejo/soso".into())
    );
    assert_eq!(canal::motivo(None, None, &conf), "url= de /etc/actualiza.conf");

    let conf = Conf { url: None, canal: Some("dev".into()) };
    assert_eq!(
        canal::resolver(None, None, &conf),
        Origen::Remoto(canal::url_de_canal(CANAL_DEV))
    );
}

#[test]
fn sin_nada_configurado_es_stable() {
    let conf = Conf::default();
    assert_eq!(
        canal::resolver(None, None, &conf),
        Origen::Remoto(canal::url_de_canal(CANAL_STABLE))
    );
    assert_eq!(canal::motivo(None, None, &conf), "canal stable por defecto");
}

#[test]
fn la_configuracion_se_lee_con_comentarios_y_espacios() {
    let conf = Conf::parse(
        "# actualizaciones\n\
         \n\
         channel = dev \n\
         url=https://espejo/soso\n\
         basura sin igual\n",
    );
    assert_eq!(conf.canal.as_deref(), Some("dev"));
    assert_eq!(conf.url.as_deref(), Some("https://espejo/soso"));

    // Un valor vacío no cuenta como configurado: si no, `url=` a secas dejaría
    // el cliente pidiendo `/manifest.txt` a la nada.
    let conf = Conf::parse("url=\nchannel=\n");
    assert_eq!(conf, Conf::default());
}

// ── Versiones ────────────────────────────────────────────────────────────

#[test]
fn una_release_mas_antigua_no_es_una_actualizacion() {
    let remoto = parse("0.2.9").unwrap();
    let local = parse("0.3.0").unwrap();
    assert_eq!(cmp(&remoto, &local), core::cmp::Ordering::Less);
    assert_eq!(cmp(&local, &local), core::cmp::Ordering::Equal);
    assert_eq!(
        cmp(&parse("0.10.0").unwrap(), &parse("0.9.9").unwrap()),
        core::cmp::Ordering::Greater,
        "comparación numérica, no de texto"
    );
}

// ── Inventario y exclusiones ─────────────────────────────────────────────

#[test]
fn una_release_no_lleva_logs() {
    for p in [
        "var/log/kernel.log",
        "var/log/aplicaciones.log",
        "var/log/actualizaciones.log.2",
        "var/log/lo-que-sea",
        "etc/algo.log",
    ] {
        assert_eq!(por_que_se_excluye(p), Some(Excluido::Log), "{p}");
        assert!(!PackWriter::should_pack(p), "{p}");
    }
}

#[test]
fn una_release_no_lleva_claves() {
    for p in ["etc/ssh_host_key", "etc/authorized_key", "etc/loquesea.key"] {
        assert_eq!(por_que_se_excluye(p), Some(Excluido::ConfigLocal), "{p}");
    }
}

#[test]
fn una_release_no_lleva_el_estado_de_una_ota() {
    // Es lo que convertiría al destino en heredero de una operación ajena,
    // con el respaldo en otra máquina.
    for p in [
        "var/lib/soso-update/abc123/diario.0",
        "var/lib/soso-update/abc123/respaldo/bin/sosh",
        "var/actualiza-prueba/manifest.txt",
        "etc/actualiza.estado",
    ] {
        assert_eq!(por_que_se_excluye(p), Some(Excluido::EstadoOta), "{p}");
    }
}

#[test]
fn una_release_no_lleva_caches_ni_temporales() {
    assert_eq!(por_que_se_excluye("var/forja-cache/x"), Some(Excluido::Cache));
    assert_eq!(por_que_se_excluye("var/cache/loquesea"), Some(Excluido::Cache));
    assert_eq!(
        por_que_se_excluye("var/self-improvement/tasks/T01/log"),
        Some(Excluido::Cache)
    );
    assert_eq!(por_que_se_excluye("tmp/sosh-ready"), Some(Excluido::Temporal));
    assert_eq!(por_que_se_excluye("models/qwen/shard0.tensor"), Some(Excluido::Volumen));
    assert_eq!(por_que_se_excluye("src/soso/kernel/src/main.rs"), Some(Excluido::Fuente));
}

#[test]
fn lo_que_si_viaja_sigue_viajando() {
    for p in [
        "bin/sosh",
        "bin/soso-update",
        "lib/firmware/iwlwifi-cc-a0-77.ucode",
        "etc/motd",
        "etc/web-prueba.html",
    ] {
        assert_eq!(por_que_se_excluye(p), None, "{p}");
        assert!(PackWriter::should_pack(p), "{p}");
    }
}

#[test]
fn la_barra_inicial_no_deja_colar_nada() {
    // El empaquetador trabaja con rutas relativas, pero un `/` de más no puede
    // ser la diferencia entre publicar y no publicar la clave de una máquina.
    assert_eq!(por_que_se_excluye("/etc/authorized_key"), Some(Excluido::ConfigLocal));
    assert_eq!(por_que_se_excluye("/var/log/kernel.log"), Some(Excluido::Log));
}
