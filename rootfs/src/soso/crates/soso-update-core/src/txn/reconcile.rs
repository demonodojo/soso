//! Tabla de reconciliación: qué hacer al arrancar con lo que hay escrito en
//! los dos medios. Es el corazón del contrato U0.
//!
//! No existe commit atómico entre FAT y sosofs, así que **cualquier** pareja
//! (registro de arranque, diario) es alcanzable con el corte adecuado. Aquí
//! se decide una y sólo una acción para cada pareja, y las que no se pueden
//! resolver sin riesgo acaban en `Diagnostico`: nunca en un arranque normal
//! con el rootfs a medias.

use alloc::string::String;

use crate::txn::bootrec::{BootRecord, Decision};
use crate::txn::journal::Journal;
use crate::txn::{TxnId, TxnState};

/// Lo que el recuperador temprano encuentra en la ESP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EstadoEsp {
    /// Ninguna ranura escrita: no hay operación.
    Ausente,
    /// Ranuras presentes pero ilegibles (magic, formato o suma).
    Roto,
    Registro(BootRecord),
}

/// Lo que encuentra en `/var/lib/soso-update/<dir>/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EstadoJournal {
    Ausente,
    Roto,
    Diario(Journal),
}

/// Motivo por el que no se puede decidir sin intervención.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motivo {
    /// El registro de arranque arma una operación cuyo diario no se puede leer.
    ArmadoSinDiario,
    /// El diario está a medias y el registro de arranque no dice qué hacer.
    MedioSinDecision,
    /// ID distinto en cada medio: son operaciones diferentes.
    IdDiscordante,
    /// Pareja imposible en el orden de escrituras del contrato.
    ParejaImposible,
    /// Registros ilegibles en ambos medios.
    RegistrosRotos,
    /// La versión restaurada tampoco llegó a acreditar su arranque. No se
    /// repite la restauración: no hay nada más que el sistema pueda intentar.
    RestauradoNoArranca,
    /// Se pide rescate pero el registro no dice a qué punto volver. Sin eso no
    /// hay nada que restaurar y adivinarlo sería peor.
    RescateSinPunto,
}

/// Acción del recuperador, antes de cargar firmware y antes de `/bin/init`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recuperacion {
    /// Arrancar con lo que hay: la pareja activa es coherente.
    Normal,
    /// Recoger el área de preparación; el sistema activo no se toca.
    Descartar(TxnId),
    /// El armado no llegó a publicarse: devolver el diario a PREPARADO.
    RetrocederAPreparado(TxnId),
    /// Aplicar o continuar la aplicación de forma idempotente.
    Aplicar(TxnId),
    /// Aplicación terminada pero el registro de arranque no lo refleja.
    PublicarProbando(TxnId),
    /// Restaurar la pareja anterior desde los respaldos del diario.
    Revertir(TxnId),
    /// La evidencia existe en sosofs; falta cerrar la decisión en la ESP.
    CompletarConfirmacion(TxnId),
    /// La restauración terminó; falta cerrarla en la ESP.
    CompletarReversion(TxnId),
    /// Restaurar el **punto retenido** (U5a): la vuelta atrás pedida desde
    /// fuera del sistema actualizado, que no depende de la operación en curso.
    Rescatar(TxnId),
    /// Diagnóstico local y recuperación desde live. No es arranque normal.
    Diagnostico(Motivo),
}

impl Recuperacion {
    /// ¿Puede el arranque seguir hacia init sin tocar nada más?
    pub fn arranca_directo(&self) -> bool {
        matches!(self, Recuperacion::Normal | Recuperacion::Descartar(_))
    }
    pub fn id(&self) -> Option<TxnId> {
        match self {
            Recuperacion::Descartar(i)
            | Recuperacion::RetrocederAPreparado(i)
            | Recuperacion::Aplicar(i)
            | Recuperacion::PublicarProbando(i)
            | Recuperacion::Revertir(i)
            | Recuperacion::CompletarConfirmacion(i)
            | Recuperacion::CompletarReversion(i)
            | Recuperacion::Rescatar(i) => Some(*i),
            _ => None,
        }
    }
}

/// Decide la acción de arranque a partir de los dos registros **tal como
/// estaban al empezar el arranque**. Lo que el recuperador escriba después no
/// vuelve a entrar aquí hasta el arranque siguiente: por eso encontrar
/// `Probando` ya publicado significa que el arranque anterior no se acreditó.
pub fn reconcile(esp: &EstadoEsp, journal: &EstadoJournal) -> Recuperacion {
    use Decision as D;
    use EstadoJournal as J;
    use Recuperacion as R;
    use TxnState as S;

    let diario = match journal {
        J::Diario(j) => Some(j),
        _ => None,
    };

    match esp {
        // ── Sin operación armada en la ESP ────────────────────────────────
        // Nada se ha podido aplicar: el sistema activo está intacto salvo que
        // el diario diga lo contrario, y entonces manda el diario.
        EstadoEsp::Ausente => match journal {
            J::Ausente | J::Roto => R::Normal,
            J::Diario(j) => match j.estado {
                S::Descargando | S::Preparado | S::Descartado => R::Descartar(j.id),
                // Se publicó el diario pero no el registro: no llegó a armarse.
                S::Armado => R::RetrocederAPreparado(j.id),
                // El registro desapareció con el rootfs a medias: deshacer.
                S::Aplicando | S::Probando | S::Revirtiendo => R::Revertir(j.id),
                S::Confirmado => R::CompletarConfirmacion(j.id),
                S::Revertido => R::Normal,
            },
        },

        // Ranuras ilegibles: no se sabe si hay decisión pendiente.
        EstadoEsp::Roto => match journal {
            J::Ausente => R::Normal,
            J::Roto => R::Diagnostico(Motivo::RegistrosRotos),
            J::Diario(j) => {
                if j.estado.sistema_intacto() {
                    R::Descartar(j.id)
                } else if j.estado == S::Revertido {
                    R::CompletarReversion(j.id)
                } else {
                    R::Diagnostico(Motivo::MedioSinDecision)
                }
            }
        },

        EstadoEsp::Registro(r) => {
            if let Some(j) = diario {
                if j.id != r.id {
                    return R::Diagnostico(Motivo::IdDiscordante);
                }
            }
            match (r.decision, diario) {
                // ── Idle ─────────────────────────────────────────────────
                (D::Idle, None) => R::Normal,
                (D::Idle, Some(j)) => {
                    if j.estado.sistema_intacto() {
                        R::Descartar(j.id)
                    } else if j.estado == S::Confirmado {
                        R::CompletarConfirmacion(j.id)
                    } else if j.estado == S::Revertido {
                        R::Normal
                    } else {
                        R::Revertir(j.id)
                    }
                }

                // ── Armado ───────────────────────────────────────────────
                // El registro sólo se publica cuando completar o deshacer ya
                // es posible con lo que hay en sosofs; sin diario legible esa
                // garantía no se puede comprobar.
                (D::Armado, None) => R::Diagnostico(Motivo::ArmadoSinDiario),
                (D::Armado, Some(j)) => match j.estado {
                    S::Preparado | S::Armado | S::Aplicando => R::Aplicar(j.id),
                    S::Probando => R::PublicarProbando(j.id),
                    S::Revirtiendo => R::Revertir(j.id),
                    S::Revertido => R::CompletarReversion(j.id),
                    S::Descargando | S::Descartado | S::Confirmado => {
                        R::Diagnostico(Motivo::ParejaImposible)
                    }
                },

                // ── Probando ─────────────────────────────────────────────
                // Verlo al arrancar es la señal de que el arranque anterior
                // no se acreditó, salvo que el diario ya tenga la evidencia.
                (D::Probando, None) => R::Diagnostico(Motivo::ArmadoSinDiario),
                (D::Probando, Some(j)) => match j.estado {
                    S::Confirmado => R::CompletarConfirmacion(j.id),
                    S::Aplicando | S::Probando | S::Revirtiendo => R::Revertir(j.id),
                    S::Revertido => R::CompletarReversion(j.id),
                    _ => R::Diagnostico(Motivo::ParejaImposible),
                },

                // ── Confirmado ───────────────────────────────────────────
                // Decisión durable tomada. El diario puede haberse recogido.
                (D::Confirmado, None) => R::Normal,
                (D::Confirmado, Some(j)) => match j.estado {
                    S::Confirmado => R::Normal,
                    // La ESP cerró la decisión y el diario se quedó atrás.
                    S::Probando => R::CompletarConfirmacion(j.id),
                    // El rootfs volvió atrás después de confirmar: la decisión
                    // de la ESP se queda vieja, no manda sobre un rootfs ya
                    // restaurado.
                    S::Revirtiendo => R::Revertir(j.id),
                    S::Revertido => R::CompletarReversion(j.id),
                    _ => R::Diagnostico(Motivo::ParejaImposible),
                },

                // ── Revertir ─────────────────────────────────────────────
                (D::Revertir, None) => R::Diagnostico(Motivo::ArmadoSinDiario),
                (D::Revertir, Some(j)) => match j.estado {
                    S::Armado | S::Aplicando | S::Probando | S::Confirmado | S::Revirtiendo => {
                        R::Revertir(j.id)
                    }
                    S::Revertido => R::CompletarReversion(j.id),
                    S::Preparado | S::Descargando | S::Descartado => R::Descartar(j.id),
                },

                // ── Restaurado a prueba ──────────────────────────────────
                // Verlo al arrancar significa que la versión restaurada
                // **tampoco** se acreditó. Repetir la restauración sería entrar
                // en bucle: ya está puesta, y volver a ponerla no cambia nada.
                (D::RestauradoAPrueba, _) => R::Diagnostico(Motivo::RestauradoNoArranca),

                // ── Rescatar ─────────────────────────────────────────────
                // Petición de fuera del sistema actualizado (entrada UEFI o
                // live). Manda sobre lo que diga el diario: quien la hace no
                // puede arrancar, o ya confirmó y aun así quiere volver. Lo que
                // se restaura es el **punto retenido**, no la operación.
                (D::Rescatar, _) => match r.punto {
                    Some(p) => R::Rescatar(p),
                    None => R::Diagnostico(Motivo::RescateSinPunto),
                },

                // ── Revertido ────────────────────────────────────────────
                (D::Revertido, None) => R::Normal,
                (D::Revertido, Some(j)) => match j.estado {
                    S::Revertido | S::Descartado => R::Normal,
                    S::Revirtiendo => R::Revertir(j.id),
                    _ => R::Diagnostico(Motivo::ParejaImposible),
                },
            }
        }
    }
}

/// Versión que debe quedar en `/etc/soso-release` tras la acción. `None` si la
/// acción no cierra ninguna pareja y el fichero se deja como está.
pub fn release_tras(esp: &EstadoEsp, accion: &Recuperacion) -> Option<String> {
    let EstadoEsp::Registro(r) = esp else {
        return None;
    };
    match accion {
        Recuperacion::Normal | Recuperacion::CompletarConfirmacion(_) => {
            Some(r.version_efectiva().into())
        }
        Recuperacion::Revertir(_) | Recuperacion::CompletarReversion(_) => {
            Some(r.version_anterior.clone())
        }
        _ => None,
    }
}
