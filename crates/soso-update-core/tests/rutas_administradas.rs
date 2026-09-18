//! U5: qué protege la exclusión de escritores. La lista no puede ser «todo»
//! —dejaría la máquina de solo lectura mientras hay una actualización armada—
//! ni «/bin» a ojo —dejaría fuera lo que el pack sí reemplaza—.

use soso_update_core::es_administrada;

#[test]
fn lo_que_la_actualizacion_reemplaza_esta_protegido() {
    assert!(es_administrada("/bin/sosh"));
    assert!(es_administrada("/lib/firmware/iwlwifi.ucode"));
    assert!(es_administrada("/etc/motd"));
    // Sin barra inicial vale igual: el kernel resuelve rutas absolutas, pero
    // no se va a colar una relativa por una barra.
    assert!(es_administrada("bin/sosh"));
}

#[test]
fn lo_del_usuario_y_el_estado_mutable_no() {
    for r in [
        "/var/log/kernel.log",
        "/var/lib/soso-update/abc/punto.rec",
        "/tmp/sosh-ready",
        "/models/tiny/0000.shard",
        "/mis-cosas/apuntes.txt",
    ] {
        assert!(!es_administrada(r), "{r} no la administra la release");
    }
}

#[test]
fn la_configuracion_local_se_puede_seguir_editando() {
    // No viaja en el pack, así que nadie la va a pisar: bloquearla sólo sería
    // molestar a quien tiene una actualización armada.
    for r in [
        "/etc/wifi.conf",
        "/etc/llm.conf",
        "/etc/ssh_host_key",
        "/etc/authorized_key",
        "/etc/soso-release",
        "/etc/actualiza.estado",
    ] {
        assert!(!es_administrada(r), "{r} es configuración de la máquina");
    }
}

#[test]
fn la_barra_final_de_la_raiz_importa() {
    assert!(!es_administrada("/binario"), "/binario no es /bin");
    assert!(!es_administrada("/libro"), "/libro no es /lib");
    assert!(!es_administrada("/etcetera"));
    assert!(!es_administrada("/bin"), "el directorio en sí no es un fichero");
}

#[test]
fn ninguna_release_viaja_dentro_de_otra() {
    // El banco fabrica sus releases en `rootfs/var/actualiza-*`, y el pack sale
    // de `rootfs/`. Sin excluirlas por prefijo, una release publicada se
    // llevaría dentro las de prueba de quien la empaquetó.
    for r in [
        "var/actualiza-prueba/manifest.txt",
        "var/actualiza-rota/rootfs.pack",
        "var/actualiza-c/kernel-x86_64",
        "var/actualiza-loquesea/x",
    ] {
        assert!(
            soso_update_core::por_que_se_excluye(r).is_some(),
            "{r} no puede viajar en un pack"
        );
    }
}

#[test]
fn coincide_con_lo_que_viaja_en_el_pack() {
    // La exclusión y el pack tienen que decir lo mismo: si protegiéramos algo
    // que el pack no trae, o dejáramos suelto algo que sí, la vuelta atrás
    // restauraría sobre un sistema que alguien cambió por debajo.
    for r in soso_update_core::RAICES_ADMINISTRADAS {
        let dentro = format!("{r}cualquiera");
        assert_eq!(
            es_administrada(&dentro),
            soso_update_core::por_que_se_excluye(&dentro).is_none()
        );
    }
}
