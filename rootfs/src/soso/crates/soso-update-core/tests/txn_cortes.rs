//! U0: reconciliación tras un corte. Para cada paso durable del contrato se
//! simula el corte justo después y se comprueba qué decide el recuperador en
//! el arranque siguiente.
//!
//! La pareja se lee **al empezar el arranque**: aplicar dentro de ese mismo
//! arranque no vuelve a entrar aquí. Por eso encontrar PROBANDO ya escrito
//! significa que el arranque anterior no se acreditó.

use soso_update_core::record::SLOT_SIZE;
use soso_update_core::txn::bootrec::{BootRecord, Decision, BOOTREC_SIZE};
use soso_update_core::txn::journal::{Accion, Contenido, Entrada, Journal, JournalError, Progreso};
use soso_update_core::txn::reconcile::{reconcile, EstadoEsp, EstadoJournal, Motivo, Recuperacion};
use soso_update_core::txn::{
    TxnId, TxnState, ORDEN_APLICACION, ORDEN_ARMADO, ORDEN_CONFIRMACION, ORDEN_REVERSION,
};

const NUEVA: &str = "0.3.0";
const ANTERIOR: &str = "0.2.9";

fn id() -> TxnId {
    TxnId::from_manifest(b"manifest 0.3.0")
}

fn contenido(n: u64, c: u8) -> Option<Contenido> {
    Some(Contenido { size: n, hash: (c as char).to_string().repeat(64) })
}

/// Los dos medios durables, con los bytes que realmente se escribirían.
struct Mundo {
    esp: Vec<u8>,
    copias: [Vec<u8>; 2],
}

impl Mundo {
    fn nuevo() -> Self {
        Self { esp: vec![0u8; BOOTREC_SIZE], copias: [Vec::new(), Vec::new()] }
    }

    fn escribe_esp(&mut self, r: &BootRecord) {
        let off = r.ranura() * SLOT_SIZE;
        self.esp[off..off + SLOT_SIZE].copy_from_slice(&r.format().unwrap());
    }

    /// El diario alterna dos ficheros según su secuencia.
    fn escribe_diario(&mut self, j: &Journal) {
        assert!(j.validate().is_ok(), "diario incoherente en el banco: {j:?}");
        self.copias[(j.seq % 2) as usize] = j.format();
    }

    fn estado(&self) -> (EstadoEsp, EstadoJournal) {
        use soso_update_core::txn::bootrec::BootRecError;
        use soso_update_core::RecordError;
        let esp = match BootRecord::pick(&self.esp) {
            Ok(r) => EstadoEsp::Registro(r),
            Err(BootRecError::Registro(RecordError::Vacia)) => EstadoEsp::Ausente,
            Err(_) => EstadoEsp::Roto,
        };
        let copias: Vec<&[u8]> = self.copias.iter().map(|c| c.as_slice()).collect();
        let diario = match Journal::pick(&copias) {
            Ok(j) => EstadoJournal::Diario(j),
            Err(JournalError::Registro(RecordError::Vacia)) => EstadoJournal::Ausente,
            Err(_) => EstadoJournal::Roto,
        };
        (esp, diario)
    }

    fn accion(&self) -> Recuperacion {
        let (esp, diario) = self.estado();
        reconcile(&esp, &diario)
    }
}

fn diario_inicial() -> Journal {
    let mut j = Journal::nuevo(id(), NUEVA, ANTERIOR);
    j.entradas = vec![
        Entrada {
            accion: Accion::Reemplazar,
            progreso: Progreso::Pendiente,
            path: "bin/sosh".into(),
            nuevo: contenido(200, b'c'),
            respaldo: None,
        },
        Entrada {
            accion: Accion::Crear,
            progreso: Progreso::Pendiente,
            path: "bin/soso-update".into(),
            nuevo: contenido(300, b'e'),
            respaldo: None,
        },
    ];
    j
}

/// Deja el mundo tal como queda al terminar el armado.
fn armado() -> (Mundo, Journal, u64) {
    let mut m = Mundo::nuevo();
    let mut j = diario_inicial();
    m.escribe_diario(&j);
    j.seq += 1;
    for e in &mut j.entradas {
        if e.accion.exige_respaldo() {
            e.respaldo = contenido(180, b'd');
        }
        e.progreso = Progreso::Respaldado;
    }
    m.escribe_diario(&j);
    j.seq += 1;
    j.estado = TxnState::Preparado;
    j.kernel_nuevo = contenido(1_000_000, b'a');
    j.kernel_anterior = contenido(900_000, b'b');
    m.escribe_diario(&j);
    let mut seq_esp = 1;
    m.escribe_esp(&BootRecord::nuevo(Decision::Armado, id(), NUEVA, ANTERIOR, seq_esp));
    seq_esp += 1;
    j.seq += 1;
    j.estado = TxnState::Armado;
    m.escribe_diario(&j);
    (m, j, seq_esp)
}

// ── Armado ───────────────────────────────────────────────────────────────

#[test]
fn cortes_del_armado() {
    let mut m = Mundo::nuevo();
    let mut j = diario_inicial();
    let mut vistos = Vec::new();

    // Corte antes de cualquier paso: la operación existe pero no ha hecho nada.
    m.escribe_diario(&j);
    vistos.push(m.accion());

    // 1. Artefactos verificados en el área de preparación.
    j.seq += 1;
    m.escribe_diario(&j);
    vistos.push(m.accion());

    // 2. Respaldo de cada fichero que cambia.
    j.seq += 1;
    for e in &mut j.entradas {
        if e.accion.exige_respaldo() {
            e.respaldo = contenido(180, b'd');
        }
        e.progreso = Progreso::Respaldado;
    }
    m.escribe_diario(&j);
    vistos.push(m.accion());

    // 3. Diario PREPARADO con inventario, hashes y versión anterior.
    j.seq += 1;
    j.estado = TxnState::Preparado;
    j.kernel_nuevo = contenido(1_000_000, b'a');
    j.kernel_anterior = contenido(900_000, b'b');
    m.escribe_diario(&j);
    vistos.push(m.accion());

    // 4. Kernel nuevo al hueco de la ESP (aún sin registro de arranque).
    vistos.push(m.accion());

    // 5. Registro de arranque ARMADO: aquí queda comprometida la operación.
    m.escribe_esp(&BootRecord::nuevo(Decision::Armado, id(), NUEVA, ANTERIOR, 1));
    vistos.push(m.accion());

    // 6. Diario ARMADO.
    j.seq += 1;
    j.estado = TxnState::Armado;
    m.escribe_diario(&j);
    vistos.push(m.accion());

    assert_eq!(vistos.len(), ORDEN_ARMADO.len() + 1, "un corte por paso más el inicial");
    assert_eq!(
        vistos,
        vec![
            Recuperacion::Descartar(id()),
            Recuperacion::Descartar(id()),
            Recuperacion::Descartar(id()),
            Recuperacion::Descartar(id()),
            Recuperacion::Descartar(id()),
            // Publicado el registro, lo que hay en sosofs basta para aplicar.
            Recuperacion::Aplicar(id()),
            Recuperacion::Aplicar(id()),
        ]
    );
    // Antes de publicar el registro, cualquier corte deja arrancar sin más.
    assert!(vistos[..5].iter().all(Recuperacion::arranca_directo));
}

// ── Aplicación ───────────────────────────────────────────────────────────

#[test]
fn cortes_de_la_aplicacion() {
    let (mut m, mut j, mut seq_esp) = armado();
    let mut vistos = vec![m.accion()];

    // 1. Diario APLICANDO.
    j.seq += 1;
    j.estado = TxnState::Aplicando;
    m.escribe_diario(&j);
    vistos.push(m.accion());

    // 2. Acciones del inventario, con progreso durable. Corte a mitad:
    j.seq += 1;
    j.entradas[0].progreso = Progreso::Aplicado;
    m.escribe_diario(&j);
    assert_eq!(m.accion(), Recuperacion::Aplicar(id()), "se continúa, no se reinicia");
    assert_eq!(j.pendientes_aplicar().count(), 1);
    j.seq += 1;
    j.entradas[1].progreso = Progreso::Aplicado;
    m.escribe_diario(&j);
    vistos.push(m.accion());

    // 3. Diario PROBANDO.
    j.seq += 1;
    j.estado = TxnState::Probando;
    m.escribe_diario(&j);
    vistos.push(m.accion());

    // 4. Registro de arranque PROBANDO. El arranque sigue hacia init; el
    //    corte se ve en el arranque *siguiente*, que ya no está acreditado.
    m.escribe_esp(&BootRecord::nuevo(Decision::Probando, id(), NUEVA, ANTERIOR, seq_esp));
    seq_esp += 1;
    vistos.push(m.accion());

    assert_eq!(vistos.len(), ORDEN_APLICACION.len() + 1);
    assert_eq!(
        vistos,
        vec![
            Recuperacion::Aplicar(id()),
            Recuperacion::Aplicar(id()),
            Recuperacion::Aplicar(id()),
            Recuperacion::PublicarProbando(id()),
            Recuperacion::Revertir(id()),
        ]
    );
    assert!(!vistos.iter().any(Recuperacion::arranca_directo), "nunca init con mezcla");
    let _ = seq_esp;
}

// ── Confirmación ─────────────────────────────────────────────────────────

#[test]
fn cortes_de_la_confirmacion() {
    let (mut m, mut j, mut seq_esp) = armado();
    j.seq += 1;
    j.estado = TxnState::Aplicando;
    for e in &mut j.entradas {
        e.progreso = Progreso::Aplicado;
    }
    m.escribe_diario(&j);
    j.seq += 1;
    j.estado = TxnState::Probando;
    m.escribe_diario(&j);
    m.escribe_esp(&BootRecord::nuevo(Decision::Probando, id(), NUEVA, ANTERIOR, seq_esp));
    seq_esp += 1;

    // Corte antes de confirmar nada: el arranque no se acreditó.
    let mut vistos = vec![m.accion()];

    // 1. Diario CONFIRMADO: la evidencia va primero a sosofs.
    j.seq += 1;
    j.estado = TxnState::Confirmado;
    m.escribe_diario(&j);
    vistos.push(m.accion());

    // 2. Registro de arranque CONFIRMADO.
    m.escribe_esp(&BootRecord::nuevo(Decision::Confirmado, id(), NUEVA, ANTERIOR, seq_esp));
    vistos.push(m.accion());

    // 3. `/etc/soso-release` reconciliado: no cambia ninguna decisión.
    vistos.push(m.accion());

    assert_eq!(vistos.len(), ORDEN_CONFIRMACION.len() + 1);
    assert_eq!(
        vistos,
        vec![
            Recuperacion::Revertir(id()),
            // Confirmación interrumpida entre sosofs y la ESP: se completa,
            // no se revierte un sistema que ya acreditó su arranque.
            Recuperacion::CompletarConfirmacion(id()),
            Recuperacion::Normal,
            Recuperacion::Normal,
        ]
    );

    // El fichero de versión se reconcilia con la decisión, no al revés.
    let (esp, _) = m.estado();
    assert_eq!(
        soso_update_core::txn::reconcile::release_tras(&esp, &Recuperacion::Normal).as_deref(),
        Some(NUEVA)
    );
}

// ── Reversión ────────────────────────────────────────────────────────────

#[test]
fn cortes_de_la_reversion() {
    let (mut m, mut j, mut seq_esp) = armado();
    j.seq += 1;
    j.estado = TxnState::Aplicando;
    for e in &mut j.entradas {
        e.progreso = Progreso::Aplicado;
    }
    m.escribe_diario(&j);
    j.seq += 1;
    j.estado = TxnState::Probando;
    m.escribe_diario(&j);
    j.seq += 1;
    j.estado = TxnState::Confirmado;
    m.escribe_diario(&j);
    m.escribe_esp(&BootRecord::nuevo(Decision::Confirmado, id(), NUEVA, ANTERIOR, seq_esp));
    seq_esp += 1;

    let mut vistos = vec![m.accion()];

    // 1. Petición durable en la ESP.
    m.escribe_esp(&BootRecord::nuevo(Decision::Revertir, id(), NUEVA, ANTERIOR, seq_esp));
    seq_esp += 1;
    vistos.push(m.accion());

    // 2. Diario REVIRTIENDO.
    j.seq += 1;
    j.estado = TxnState::Revirtiendo;
    m.escribe_diario(&j);
    vistos.push(m.accion());

    // 3. Restaurar cada acción desde su respaldo, idempotente.
    j.seq += 1;
    j.entradas[0].progreso = Progreso::Restaurado;
    m.escribe_diario(&j);
    assert_eq!(m.accion(), Recuperacion::Revertir(id()), "restauración a medias");
    j.seq += 1;
    j.entradas[1].progreso = Progreso::Restaurado;
    m.escribe_diario(&j);
    vistos.push(m.accion());

    // 4. Kernel anterior restaurado desde el hueco.
    vistos.push(m.accion());

    // 5. Diario REVERTIDO.
    j.seq += 1;
    j.estado = TxnState::Revertido;
    m.escribe_diario(&j);
    vistos.push(m.accion());

    // 6. Registro de arranque REVERTIDO.
    m.escribe_esp(&BootRecord::nuevo(Decision::Revertido, id(), NUEVA, ANTERIOR, seq_esp));
    vistos.push(m.accion());

    assert_eq!(vistos.len(), ORDEN_REVERSION.len() + 1);
    assert_eq!(
        vistos,
        vec![
            Recuperacion::Normal,
            Recuperacion::Revertir(id()),
            Recuperacion::Revertir(id()),
            Recuperacion::Revertir(id()),
            Recuperacion::Revertir(id()),
            Recuperacion::CompletarReversion(id()),
            Recuperacion::Normal,
        ]
    );
    let (esp, _) = m.estado();
    assert_eq!(
        soso_update_core::txn::reconcile::release_tras(&esp, &Recuperacion::Normal).as_deref(),
        Some(ANTERIOR),
        "tras revertir, la versión anunciada es la anterior"
    );
}

// ── Registros rotos y parejas imposibles ─────────────────────────────────

#[test]
fn registro_de_arranque_perdido_con_rootfs_a_medias() {
    let (mut m, mut j, _) = armado();
    j.seq += 1;
    j.estado = TxnState::Aplicando;
    j.entradas[0].progreso = Progreso::Aplicado;
    m.escribe_diario(&j);
    // La ESP se borra (reflash, chapuza manual): el diario manda y deshace.
    m.esp = vec![0u8; BOOTREC_SIZE];
    assert_eq!(m.accion(), Recuperacion::Revertir(id()));
}

#[test]
fn armado_sin_diario_legible_exige_diagnostico() {
    let (mut m, _, _) = armado();
    m.copias = [Vec::new(), Vec::new()];
    assert_eq!(m.accion(), Recuperacion::Diagnostico(Motivo::ArmadoSinDiario));

    let (mut m, _, _) = armado();
    for c in &mut m.copias {
        if !c.is_empty() {
            c[20] ^= 0x01;
        }
    }
    assert_eq!(m.accion(), Recuperacion::Diagnostico(Motivo::ArmadoSinDiario));
}

#[test]
fn ids_distintos_no_son_la_misma_operacion() {
    let (mut m, _, _) = armado();
    let otro = TxnId::from_manifest(b"otra release");
    m.escribe_esp(&BootRecord::nuevo(Decision::Probando, otro, "0.4.0", NUEVA, 9));
    assert_eq!(m.accion(), Recuperacion::Diagnostico(Motivo::IdDiscordante));
}

#[test]
fn ambos_registros_rotos() {
    let (mut m, _, _) = armado();
    for i in 0..4 {
        m.esp[i * SLOT_SIZE + 40] ^= 0x01;
    }
    for c in &mut m.copias {
        if !c.is_empty() {
            c[20] ^= 0x01;
        }
    }
    assert_eq!(m.accion(), Recuperacion::Diagnostico(Motivo::RegistrosRotos));
}

#[test]
fn sistema_sin_operacion_arranca_normal() {
    let m = Mundo::nuevo();
    assert_eq!(m.accion(), Recuperacion::Normal);
}

#[test]
fn una_mezcla_posible_nunca_arranca_directo() {
    // Barrido de todas las parejas: ninguna combinación con el rootfs a medias
    // puede acabar en un arranque normal.
    let estados = [
        TxnState::Descargando,
        TxnState::Preparado,
        TxnState::Armado,
        TxnState::Aplicando,
        TxnState::Probando,
        TxnState::Confirmado,
        TxnState::Revirtiendo,
        TxnState::Revertido,
        TxnState::Descartado,
    ];
    let decisiones = [
        Decision::Idle,
        Decision::Armado,
        Decision::Probando,
        Decision::Confirmado,
        Decision::Revertir,
        Decision::Revertido,
    ];
    for estado in estados {
        let mut j = diario_inicial();
        j.estado = estado;
        j.kernel_nuevo = contenido(10, b'a');
        j.kernel_anterior = contenido(10, b'b');
        for e in &mut j.entradas {
            if e.accion.exige_respaldo() {
                e.respaldo = contenido(9, b'd');
            }
        }
        let jd = EstadoJournal::Diario(j);
        for esp in [EstadoEsp::Ausente, EstadoEsp::Roto]
            .into_iter()
            .chain(decisiones.iter().map(|d| {
                EstadoEsp::Registro(BootRecord::nuevo(*d, id(), NUEVA, ANTERIOR, 1))
            }))
        {
            let accion = reconcile(&esp, &jd);
            if estado.mezcla_posible() {
                assert!(
                    !accion.arranca_directo(),
                    "{esp:?} + {estado:?} arrancaría con el rootfs a medias: {accion:?}"
                );
            }
            if let Recuperacion::Diagnostico(_) = accion {
                continue;
            }
            assert!(
                accion.id() == Some(id()) || accion == Recuperacion::Normal,
                "{esp:?} + {estado:?} → {accion:?}"
            );
        }
    }
}
