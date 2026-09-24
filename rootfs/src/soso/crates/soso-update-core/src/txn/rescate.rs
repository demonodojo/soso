//! Qué hacer cuando alguien pide un rescate desde fuera del sistema instalado.
//!
//! La entrada UEFI de rescate (U5d) no sabe leer sosofs ni consultar nada: lo
//! único que tiene delante es el registro de arranque de la ESP. Esta es la
//! decisión que toma con él, aparte para poder probarla en el host: el shim
//! sólo pone la E/S.

use alloc::string::{String, ToString};

use crate::txn::bootrec::{BootRecord, Decision};

/// Lo que el shim lleva en el OptionalData de su entrada de rescate.
pub const SENAL: &str = "rescatar";

/// ¿Nos han arrancado por la entrada de rescate? Se acepta tanto la forma
/// UTF-16 que escribe el instalador como texto suelto: un firmware que entregue
/// los datos de otra manera no debería dejar sin vuelta atrás a quien la pide.
pub fn es_senal(texto: &str) -> bool {
    texto
        .split(|c: char| c.is_whitespace() || c == '\0')
        .any(|t| t.eq_ignore_ascii_case(SENAL))
}

/// Por qué no se registra el rescate. Todos son «no lo hago y te digo por qué»,
/// nunca un intento a ciegas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sin {
    /// El registro es de formato 1: no sabe de puntos retenidos. Eso es «no
    /// consta», no «no hay».
    FormatoAntiguo,
    /// El registro sabe de puntos y no trae ninguno.
    SinPunto,
    /// Esa vuelta atrás ya se hizo; el punto no lleva a ningún otro sitio.
    YaRestaurado { version: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Publicar este registro y pedir también la vuelta atrás del kernel.
    Pedir {
        registro: BootRecord,
        desde: String,
        hasta: String,
    },
    /// Ya estaba pedido: no se gasta otra secuencia repitiéndolo.
    YaPedido { hasta: String },
    /// No procede.
    No(Sin),
}

/// Decide a partir del registro que hay en la ESP.
pub fn planear(rec: &BootRecord) -> Plan {
    if !rec.conoce_puntos() {
        return Plan::No(Sin::FormatoAntiguo);
    }
    let Some(punto) = rec.punto else {
        return Plan::No(Sin::SinPunto);
    };
    if rec.decision == Decision::Rescatar {
        return Plan::YaPedido {
            hasta: rec.version_anterior.clone(),
        };
    }
    if matches!(
        rec.decision,
        Decision::Revertido | Decision::RestauradoAPrueba
    ) {
        return Plan::No(Sin::YaRestaurado {
            version: rec.version_efectiva().to_string(),
        });
    }
    let desde = rec.version_efectiva().to_string();
    let registro = BootRecord::nuevo(
        Decision::Rescatar,
        rec.id,
        &desde,
        &rec.version_anterior,
        rec.seq + 1,
    )
    .con_punto(punto);
    Plan::Pedir {
        registro,
        desde,
        hasta: rec.version_anterior.clone(),
    }
}

/// Cierra el diario de la operación que el rescate acaba de deshacer.
///
/// Un rescate restaura el **punto**, no la operación, y por eso no lleva diario
/// de progreso. Pero dejar el diario abierto diciendo «probando» mientras la
/// ESP ya dice «revertido» es justo la pareja incoherente que el contrato
/// prohíbe: el arranque siguiente no puede saber cuál de los dos manda. Así que
/// el rescate lo cierra con los eventos de siempre, sin inventar estados:
/// lo que llegó a aplicarse se deshace, y lo que no, se descarta.
///
/// Devuelve `true` si el diario cambió y hay que volver a escribirlo.
pub fn cerrar_diario(j: &mut crate::txn::journal::Journal) -> bool {
    use crate::txn::{TxnEvent, TxnState};
    match j.estado {
        // Ya cerrado: repetir el rescate no debe moverlo.
        TxnState::Revertido | TxnState::Descartado => false,
        // Nada se había escrito en el sistema activo; su preparación ya no
        // sirve, porque el sistema de debajo no es el que tenía delante.
        TxnState::Descargando | TxnState::Preparado => j.avanzar(TxnEvent::Abortada).is_ok(),
        _ => {
            let mut cambio = j.avanzar(TxnEvent::ReversionSolicitada).is_ok();
            if j.estado == TxnState::Revirtiendo {
                cambio |= j.avanzar(TxnEvent::ReversionCompleta).is_ok();
            }
            cambio
        }
    }
}
