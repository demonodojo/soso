//! U0: la tabla de reconciliación del contrato tiene que ser la que produce
//! `reconcile()`. Sin esto, cambiar una celda del código y dejar el documento
//! viejo pasa desapercibido, y ese documento es lo que lee quien implementa el
//! aplicador de U5.

use soso_update_core::txn::bootrec::{BootRecord, Decision};
use soso_update_core::txn::journal::{Accion, Contenido, Entrada, Journal, Progreso};
use soso_update_core::txn::reconcile::{reconcile, EstadoEsp, EstadoJournal, Motivo, Recuperacion};
use soso_update_core::txn::{TxnId, TxnState};

const DOC: &str = include_str!("../../../docs/U0-CONTRATO-ACTUALIZACION.md");

const ESTADOS: [TxnState; 9] = [
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

const DECISIONES: [Decision; 7] = [
    Decision::Idle,
    Decision::Armado,
    Decision::Probando,
    Decision::Confirmado,
    Decision::Revertir,
    Decision::Revertido,
    Decision::Rescatar,
];

fn id() -> TxnId {
    TxnId::from_manifest(b"m")
}

fn contenido(n: u64, c: u8) -> Option<Contenido> {
    Some(Contenido { size: n, hash: (c as char).to_string().repeat(64) })
}

/// Etiqueta de la celda, tal como aparece en el documento.
fn celda(r: &Recuperacion) -> &'static str {
    match r {
        Recuperacion::Normal => "normal",
        Recuperacion::Descartar(_) => "descartar",
        Recuperacion::RetrocederAPreparado(_) => "retroceder",
        Recuperacion::Aplicar(_) => "**aplicar**",
        Recuperacion::PublicarProbando(_) => "publicar probando",
        Recuperacion::Revertir(_) => "**revertir**",
        Recuperacion::CompletarConfirmacion(_) => "completar confirmación",
        Recuperacion::CompletarReversion(_) => "completar reversión",
        Recuperacion::Rescatar(_) => "rescatar punto",
        Recuperacion::Diagnostico(Motivo::ArmadoSinDiario) => "diag. sin diario",
        Recuperacion::Diagnostico(Motivo::MedioSinDecision) => "diag. sin decisión",
        Recuperacion::Diagnostico(Motivo::IdDiscordante) => "diag. ID",
        Recuperacion::Diagnostico(Motivo::ParejaImposible) => "diag. imposible",
        Recuperacion::Diagnostico(Motivo::RegistrosRotos) => "diag. rotos",
        Recuperacion::Diagnostico(Motivo::RescateSinPunto) => "diag. sin punto",
    }
}

fn diario(estado: TxnState) -> EstadoJournal {
    let mut j = Journal::nuevo(id(), "N", "A");
    j.estado = estado;
    j.kernel_nuevo = contenido(10, b'a');
    j.kernel_anterior = contenido(10, b'b');
    j.entradas = vec![Entrada {
        accion: Accion::Reemplazar,
        progreso: Progreso::Pendiente,
        path: "bin/x".into(),
        nuevo: contenido(1, b'c'),
        respaldo: contenido(1, b'd'),
    }];
    EstadoJournal::Diario(j)
}

fn tabla() -> String {
    let mut out = String::from("| registro ESP \\ diario | ausente | roto |");
    for e in ESTADOS {
        out.push_str(&format!(" {} |", e.as_str()));
    }
    out.push_str("\n|---|---|---|");
    for _ in ESTADOS {
        out.push_str("---|");
    }
    out.push('\n');

    let filas = [("ausente", EstadoEsp::Ausente), ("roto", EstadoEsp::Roto)]
        .into_iter()
        .chain(DECISIONES.iter().map(|d| {
            let r = BootRecord::nuevo(*d, id(), "N", "A", 1);
            // El rescate sólo tiene sentido con punto retenido; sin él la tabla
            // diría «diag. sin punto» en toda la fila y no enseñaría nada.
            let r = if *d == Decision::Rescatar { r.con_punto(id()) } else { r };
            (d.as_str(), EstadoEsp::Registro(r))
        }));

    for (nombre, esp) in filas {
        out.push_str(&format!("| **{nombre}** |"));
        for j in [EstadoJournal::Ausente, EstadoJournal::Roto] {
            out.push_str(&format!(" {} |", celda(&reconcile(&esp, &j))));
        }
        for e in ESTADOS {
            out.push_str(&format!(" {} |", celda(&reconcile(&esp, &diario(e)))));
        }
        out.push('\n');
    }
    out
}

#[test]
fn el_documento_tiene_la_tabla_del_codigo() {
    let generada = tabla();
    assert!(
        DOC.contains(&generada),
        "docs/U0-CONTRATO-ACTUALIZACION.md §6 no coincide con reconcile(). \
         Tabla actual:\n\n{generada}"
    );
}

#[test]
fn el_documento_lista_los_pasos_de_cada_orden() {
    use soso_update_core::txn::{
        ORDEN_APLICACION, ORDEN_ARMADO, ORDEN_CONFIRMACION, ORDEN_REVERSION,
    };
    // El documento maqueta los nombres de estado (`preparado`); se compara sin
    // acentos de formato ni mayúsculas.
    let doc = DOC.replace('`', "").to_lowercase();
    // Cada paso durable del código aparece, y en el mismo orden, en la sección 5.
    for orden in [ORDEN_ARMADO, ORDEN_APLICACION, ORDEN_CONFIRMACION, ORDEN_REVERSION] {
        let mut desde = 0;
        for paso in orden {
            // Las frases largas se parten en varias líneas en el documento: se
            // busca el primer tramo distintivo de cada paso.
            let clave = paso.descripcion.split(',').next().unwrap().to_lowercase();
            let pos = doc[desde..]
                .find(&clave)
                .unwrap_or_else(|| panic!("falta en el documento, o desordenado: {clave:?}"));
            desde += pos + clave.len();
        }
    }
}
