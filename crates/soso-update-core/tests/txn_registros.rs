//! U0: registros durables — envoltura, ranuras por turnos y registros rotos.

use soso_update_core::record::{self, RecordError, SLOTS, SLOT_SIZE};
use soso_update_core::txn::bootrec::{BootRecError, BootRecord, Decision, BOOTREC_SIZE};
use soso_update_core::txn::journal::{Accion, Contenido, Entrada, Journal, JournalError, Progreso};
use soso_update_core::txn::{TxnId, TxnState};

const MAGIC: &str = "SOSOTXN arranque";

fn id() -> TxnId {
    TxnId::from_manifest(b"manifest de prueba")
}

fn contenido(n: u64, c: u8) -> Option<Contenido> {
    Some(Contenido { size: n, hash: (c as char).to_string().repeat(64) })
}

fn diario(estado: TxnState) -> Journal {
    let mut j = Journal::nuevo(id(), "0.3.0", "0.2.9");
    j.estado = estado;
    j.seq = 4;
    j.kernel_nuevo = contenido(1_000_000, b'a');
    j.kernel_anterior = contenido(900_000, b'b');
    j.entradas = vec![
        Entrada {
            accion: Accion::Reemplazar,
            progreso: Progreso::Respaldado,
            path: "bin/sosh".into(),
            nuevo: contenido(200, b'c'),
            respaldo: contenido(180, b'd'),
        },
        Entrada {
            accion: Accion::Crear,
            progreso: Progreso::Respaldado,
            path: "bin/soso-update".into(),
            nuevo: contenido(300, b'e'),
            respaldo: None,
        },
        Entrada {
            accion: Accion::Borrar,
            progreso: Progreso::Respaldado,
            path: "lib/viejo.so".into(),
            nuevo: None,
            respaldo: contenido(50, b'f'),
        },
    ];
    j
}

// ── Envoltura ────────────────────────────────────────────────────────────

#[test]
fn ranura_roundtrip() {
    let raw = record::frame_slot(MAGIC, 1, 7, "clave=valor\n", SLOT_SIZE).unwrap();
    assert_eq!(raw.len(), SLOT_SIZE);
    let f = record::parse(MAGIC, 1, &raw).unwrap();
    assert_eq!(f.seq, 7);
    assert_eq!(f.campo("clave"), Some("valor"));
}

#[test]
fn ranura_vacia_no_es_un_registro_roto() {
    assert_eq!(record::parse(MAGIC, 1, &[0u8; SLOT_SIZE]), Err(RecordError::Vacia));
    assert_eq!(record::parse(MAGIC, 1, &[b'\n'; SLOT_SIZE]), Err(RecordError::Vacia));
    // Flash sin borrar: 0xff también es «nunca escrito».
    assert_eq!(record::parse(MAGIC, 1, &[0xffu8; SLOT_SIZE]), Err(RecordError::Vacia));
}

#[test]
fn escritura_cortada_antes_de_la_suma() {
    let raw = record::frame_slot(MAGIC, 1, 3, "clave=valor\n", SLOT_SIZE).unwrap();
    let cortado = &raw[..20];
    assert_eq!(record::parse(MAGIC, 1, cortado), Err(RecordError::SinSuma));
}

#[test]
fn bit_cambiado_invalida_el_registro() {
    let mut raw = record::frame_slot(MAGIC, 1, 3, "clave=valor\n", SLOT_SIZE).unwrap();
    raw[30] ^= 0x20;
    assert_eq!(record::parse(MAGIC, 1, &raw), Err(RecordError::SumaIncorrecta));
}

#[test]
fn suma_de_otro_registro_no_cuela() {
    let a = record::frame_slot(MAGIC, 1, 3, "clave=a\n", SLOT_SIZE).unwrap();
    let b = record::frame_slot(MAGIC, 1, 3, "clave=b\n", SLOT_SIZE).unwrap();
    // Cuerpo de A con la suma de B: un sector mezclado de dos escrituras.
    let pos = a.windows(4).position(|w| w == b"sum=").unwrap();
    let mut mezcla = a.clone();
    mezcla[pos..pos + 4 + record::SUM_LEN].copy_from_slice(&b[pos..pos + 4 + record::SUM_LEN]);
    assert_eq!(record::parse(MAGIC, 1, &mezcla), Err(RecordError::SumaIncorrecta));
}

#[test]
fn magic_y_formato_desconocidos_se_rechazan() {
    let otro = record::frame_slot("SOSOOTRO", 1, 1, "", SLOT_SIZE).unwrap();
    assert_eq!(record::parse(MAGIC, 1, &otro), Err(RecordError::BadMagic));
    // Formato del futuro: este recuperador no lo entiende y no lo adivina.
    let futuro = record::frame_slot(MAGIC, 9, 1, "", SLOT_SIZE).unwrap();
    assert_eq!(
        record::parse(MAGIC, 1, &futuro),
        Err(RecordError::FormatoDesconocido(9))
    );
}

#[test]
fn registro_mas_grande_que_la_ranura_no_se_trunca() {
    let cuerpo = "x=012345678901234567890123456789\n".repeat(200);
    assert_eq!(
        record::frame_slot(MAGIC, 1, 1, &cuerpo, SLOT_SIZE),
        Err(RecordError::Desbordado)
    );
}

#[test]
fn ranuras_por_turnos_conservan_la_anterior() {
    // Escritura cortada en la ranura que tocaba: la anterior sigue íntegra.
    let mut fichero = vec![0u8; BOOTREC_SIZE];
    let viejo = BootRecord::nuevo(Decision::Armado, id(), "0.3.0", "0.2.9", 8);
    let nuevo = BootRecord::nuevo(Decision::Probando, id(), "0.3.0", "0.2.9", 9);
    assert_ne!(viejo.ranura(), nuevo.ranura());
    let off = viejo.ranura() * SLOT_SIZE;
    fichero[off..off + SLOT_SIZE].copy_from_slice(&viejo.format().unwrap());
    let off = nuevo.ranura() * SLOT_SIZE;
    let bytes = nuevo.format().unwrap();
    fichero[off..off + 40].copy_from_slice(&bytes[..40]); // corte a mitad

    let leido = BootRecord::pick(&fichero).unwrap();
    assert_eq!(leido.decision, Decision::Armado);
    assert_eq!(leido.seq, 8);
}

#[test]
fn se_elige_la_secuencia_mayor_aunque_esté_antes() {
    let mut fichero = vec![0u8; BOOTREC_SIZE];
    for (seq, dec) in [(1u64, Decision::Armado), (2, Decision::Probando), (3, Decision::Confirmado)] {
        let r = BootRecord::nuevo(dec, id(), "0.3.0", "0.2.9", seq);
        let off = r.ranura() * SLOT_SIZE;
        fichero[off..off + SLOT_SIZE].copy_from_slice(&r.format().unwrap());
    }
    assert_eq!(BootRecord::pick(&fichero).unwrap().decision, Decision::Confirmado);
}

#[test]
fn todas_las_ranuras_rotas_no_se_confunde_con_ausente() {
    let mut fichero = vec![0u8; BOOTREC_SIZE];
    for i in 0..SLOTS {
        let r = BootRecord::nuevo(Decision::Armado, id(), "0.3.0", "0.2.9", i as u64);
        let mut bytes = r.format().unwrap();
        bytes[50] ^= 0x01;
        fichero[i * SLOT_SIZE..(i + 1) * SLOT_SIZE].copy_from_slice(&bytes);
    }
    assert_eq!(
        BootRecord::pick(&fichero),
        Err(BootRecError::Registro(RecordError::SumaIncorrecta))
    );
    assert_eq!(
        BootRecord::pick(&vec![0u8; BOOTREC_SIZE]),
        Err(BootRecError::Registro(RecordError::Vacia))
    );
}

// ── Registro de arranque ─────────────────────────────────────────────────

#[test]
fn bootrec_roundtrip() {
    let r = BootRecord::nuevo(Decision::Probando, id(), "0.3.0", "0.2.9", 11);
    let leido = BootRecord::parse(&r.format().unwrap()).unwrap();
    assert_eq!(leido, r);
    assert_eq!(leido.dir, id().dir());
}

#[test]
fn la_decision_manda_sobre_la_version_anunciada() {
    let probando = BootRecord::nuevo(Decision::Probando, id(), "0.3.0", "0.2.9", 1);
    assert_eq!(probando.version_efectiva(), "0.3.0");
    let revertido = BootRecord::nuevo(Decision::Revertido, id(), "0.3.0", "0.2.9", 2);
    assert_eq!(revertido.version_efectiva(), "0.2.9");
    let armado = BootRecord::nuevo(Decision::Armado, id(), "0.3.0", "0.2.9", 3);
    assert_eq!(armado.version_efectiva(), "0.2.9");
}

// ── Diario ───────────────────────────────────────────────────────────────

#[test]
fn diario_roundtrip_con_inventario() {
    let j = diario(TxnState::Armado);
    let leido = Journal::parse(&j.format()).unwrap();
    assert_eq!(leido, j);
    assert_eq!(leido.entradas.len(), 3);
    assert!(leido.validate().is_ok());
}

#[test]
fn diario_admite_rutas_con_espacios() {
    let mut j = diario(TxnState::Armado);
    j.entradas[0].path = "etc/con espacio.conf".into();
    let leido = Journal::parse(&j.format()).unwrap();
    assert_eq!(leido.entradas[0].path, "etc/con espacio.conf");
}

#[test]
fn diario_elige_la_copia_buena() {
    let a = diario(TxnState::Aplicando).format();
    let mut b = diario(TxnState::Probando);
    b.seq = 5;
    let mut rota = b.format();
    rota[80] ^= 0x01;
    assert_eq!(
        Journal::pick(&[&a[..], &rota[..]]).unwrap().estado,
        TxnState::Aplicando
    );
}

#[test]
fn diario_armado_exige_respaldo_de_lo_que_reemplaza() {
    let mut j = diario(TxnState::Armado);
    j.entradas[0].respaldo = None;
    assert_eq!(
        j.validate(),
        Err(JournalError::SinRespaldo("bin/sosh".into()))
    );
    // Antes de armar todavía es un diario legítimo: no se ha tocado nada.
    j.estado = TxnState::Descargando;
    assert!(j.validate().is_ok());
}

#[test]
fn diario_armado_exige_la_pareja_de_kernels() {
    let mut j = diario(TxnState::Armado);
    j.kernel_anterior = None;
    assert_eq!(j.validate(), Err(JournalError::SinKernel));
}

#[test]
fn diario_rechaza_rutas_peligrosas_y_duplicadas() {
    for mala in ["/etc/passwd", "../etc/passwd", "bin//sosh", "a\\b", ""] {
        let mut j = diario(TxnState::Armado);
        j.entradas[0].path = mala.into();
        assert_eq!(
            j.validate(),
            Err(JournalError::RutaInvalida(mala.into())),
            "ruta {mala:?}"
        );
    }
    let mut j = diario(TxnState::Armado);
    j.entradas[1].path = j.entradas[0].path.clone();
    j.entradas[1].accion = Accion::Reemplazar;
    j.entradas[1].respaldo = contenido(10, b'a');
    assert!(matches!(j.validate(), Err(JournalError::RutaDuplicada(_))));
}

#[test]
fn progreso_marca_lo_que_falta_por_hacer() {
    let mut j = diario(TxnState::Aplicando);
    j.entradas[0].progreso = Progreso::Aplicado;
    assert_eq!(j.pendientes_aplicar().count(), 2);
    assert_eq!(j.pendientes_restaurar().count(), 1);
    assert_eq!(
        j.pendientes_restaurar().next().unwrap().path,
        "bin/sosh"
    );
}

#[test]
fn diario_roto_se_distingue_de_ausente() {
    let mut bytes = diario(TxnState::Probando).format();
    bytes[10] ^= 0xff;
    assert!(matches!(
        Journal::parse(&bytes),
        Err(JournalError::Registro(_))
    ));
    assert_eq!(
        Journal::pick(&[&[][..]]),
        Err(JournalError::Registro(RecordError::Vacia))
    );
}

#[test]
fn id_es_el_hash_del_manifiesto_no_la_version() {
    let a = TxnId::from_manifest(b"SOSOREL v1\nversion=0.3.0\nbuild=a\n");
    let b = TxnId::from_manifest(b"SOSOREL v1\nversion=0.3.0\nbuild=b\n");
    assert_ne!(a, b, "dos builds de la misma versión no son la misma operación");
    assert_eq!(TxnId::from_hex(&a.to_hex()), Some(a));
    assert_eq!(a.dir().len(), 16);
    assert_eq!(TxnId::from_hex("no-es-hex"), None);
}
