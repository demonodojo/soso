//! Contrato de la transacción de actualización (entrega U0 de
//! `docs/PLAN-ACTUALIZACIONES.md`).
//!
//! Una actualización es **una pareja kernel + rootfs** que se confirma o se
//! deshace entera. Esta es la lógica `no_std` compartida: estados, identidad
//! de la operación, orden de escrituras, reconciliación tras un corte y
//! comprobaciones previas de capacidad. No hace E/S: el kernel, el shim, el
//! cliente y los bancos host le dan los bytes ya leídos.

pub mod aplicador;
pub mod bootrec;
pub mod journal;
pub mod reconcile;

use alloc::string::String;

use crate::hash::{decode_hex_sha256, Hash256};

/// Estados durables de la operación (contrato U0 §2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TxnState {
    /// Bajando artefactos al área de preparación. El sistema activo no cambia.
    #[default]
    Descargando,
    /// Todo descargado y verificado, respaldos hechos, diario durable.
    Preparado,
    /// Registro de arranque publicado: se aplicará en el próximo reinicio.
    Armado,
    /// Aplicación en curso (arranque temprano, antes de firmware e init).
    Aplicando,
    /// Pareja nueva activa, esperando que el arranque se acredite.
    Probando,
    /// Decisión durable de quedarse con la versión nueva.
    Confirmado,
    /// Restauración en curso desde los respaldos del diario.
    Revirtiendo,
    /// Restauración completa: vuelve a estar activa la pareja anterior.
    Revertido,
    /// Cancelada antes de armar: se recoge el área y el sistema no cambió.
    Descartado,
}

/// Sucesos que hacen avanzar la máquina. Cada uno corresponde a una escritura
/// durable, no a un paso en RAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxnEvent {
    DescargaVerificada,
    RespaldoDurable,
    RegistroArranquePublicado,
    AplicacionIniciada,
    AplicacionCompleta,
    PruebaSuperada,
    PruebaFallida,
    ReversionSolicitada,
    ReversionCompleta,
    /// Cancelación del usuario antes de armar.
    Abortada,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransicionInvalida {
    pub desde: TxnState,
    pub evento: TxnEvent,
}

impl TxnState {
    /// Transición única y total: lo que no está aquí no es un estado alcanzable.
    pub fn next(self, ev: TxnEvent) -> Result<TxnState, TransicionInvalida> {
        use TxnEvent::*;
        use TxnState::*;
        let siguiente = match (self, ev) {
            (Descargando, DescargaVerificada) => Preparado,
            // Antes de armar, cancelar sólo recoge el área de preparación.
            (Descargando, Abortada) | (Preparado, Abortada) => Descartado,
            (Preparado, RespaldoDurable) => Preparado,
            (Preparado, RegistroArranquePublicado) => Armado,
            (Armado, AplicacionIniciada) => Aplicando,
            (Aplicando, AplicacionIniciada) => Aplicando,
            (Aplicando, AplicacionCompleta) => Probando,
            (Probando, PruebaSuperada) => Confirmado,
            (Probando, PruebaFallida) => Revirtiendo,
            (Probando, ReversionSolicitada) => Revirtiendo,
            (Confirmado, ReversionSolicitada) => Revirtiendo,
            (Armado, ReversionSolicitada) => Revertido,
            (Aplicando, PruebaFallida) => Revirtiendo,
            (Aplicando, ReversionSolicitada) => Revirtiendo,
            (Revirtiendo, ReversionSolicitada) => Revirtiendo,
            (Revirtiendo, ReversionCompleta) => Revertido,
            _ => return Err(TransicionInvalida { desde: self, evento: ev }),
        };
        Ok(siguiente)
    }

    /// Antes de armar, cortar la operación no deja rastro en el sistema activo.
    pub fn sistema_intacto(self) -> bool {
        matches!(
            self,
            TxnState::Descargando | TxnState::Preparado | TxnState::Armado | TxnState::Descartado
        )
    }

    /// Desde que se arma, el diario tiene que bastar para deshacer: exige
    /// respaldo de todo lo que cambia y de la pareja de kernels.
    pub fn exige_respaldos(self) -> bool {
        matches!(
            self,
            TxnState::Armado
                | TxnState::Aplicando
                | TxnState::Probando
                | TxnState::Confirmado
                | TxnState::Revirtiendo
                | TxnState::Revertido
        )
    }

    /// Estados donde el rootfs puede estar a medias entre dos versiones.
    pub fn mezcla_posible(self) -> bool {
        matches!(self, TxnState::Aplicando | TxnState::Revirtiendo)
    }

    /// El área de la operación se puede recoger. El respaldo de la versión
    /// anterior confirmada **no** se borra como efecto inmediato de esto.
    pub fn recogible(self) -> bool {
        matches!(
            self,
            TxnState::Confirmado | TxnState::Revertido | TxnState::Descartado
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TxnState::Descargando => "descargando",
            TxnState::Preparado => "preparado",
            TxnState::Armado => "armado",
            TxnState::Aplicando => "aplicando",
            TxnState::Probando => "probando",
            TxnState::Confirmado => "confirmado",
            TxnState::Revirtiendo => "revirtiendo",
            TxnState::Revertido => "revertido",
            TxnState::Descartado => "descartado",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "descargando" => TxnState::Descargando,
            "preparado" => TxnState::Preparado,
            "armado" => TxnState::Armado,
            "aplicando" => TxnState::Aplicando,
            "probando" => TxnState::Probando,
            "confirmado" => TxnState::Confirmado,
            "revirtiendo" => TxnState::Revirtiendo,
            "revertido" => TxnState::Revertido,
            "descartado" => TxnState::Descartado,
            _ => return None,
        })
    }
}

/// Identidad de la operación: **hash del manifiesto**, no su versión.
///
/// Dos `latest` distintos comparten número de versión; reconocerlos como la
/// misma operación mezclaría artefactos de releases diferentes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxnId(pub Hash256);

impl TxnId {
    pub fn from_manifest(manifest: &[u8]) -> Self {
        TxnId(crate::hash::sha256(manifest))
    }

    pub fn from_hex(hex: &str) -> Option<Self> {
        decode_hex_sha256(hex).map(TxnId)
    }

    pub fn to_hex(self) -> String {
        crate::hash::hex_of(self.0)
    }

    /// Nombre del directorio en `/var/lib/soso-update/`.
    pub fn dir(self) -> String {
        let hex = self.to_hex();
        hex[..16].into()
    }
}

/// Medio donde cae una escritura del orden de aplicación.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Medio {
    /// Diario, respaldos y área de preparación en sosofs.
    Sosofs,
    /// Registro de arranque y huecos de kernel en la ESP (FAT).
    Esp,
    /// Contenido real del sistema activo (`/bin`, `/lib`, `kernel-x86_64`).
    Sistema,
}

/// Un paso durable del contrato. El corte se define **después** de cada paso.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paso {
    pub medio: Medio,
    pub descripcion: &'static str,
}

/// Armado: nada se publica en la ESP hasta que completar o deshacer es posible
/// sólo con lo que ya está en sosofs, releído tras escribirlo.
pub const ORDEN_ARMADO: &[Paso] = &[
    Paso { medio: Medio::Sosofs, descripcion: "artefactos verificados en el área de preparación" },
    Paso { medio: Medio::Sosofs, descripcion: "respaldo de cada fichero que cambia o se borra" },
    Paso { medio: Medio::Sosofs, descripcion: "diario PREPARADO con inventario, hashes y versión anterior" },
    Paso { medio: Medio::Esp, descripcion: "kernel nuevo al hueco y SOSOKRN.MET staged" },
    Paso { medio: Medio::Esp, descripcion: "registro de arranque ARMADO con el ID" },
    Paso { medio: Medio::Sosofs, descripcion: "diario ARMADO" },
];

/// Aplicación en el arranque temprano, antes de cargar firmware y de `/bin/init`.
pub const ORDEN_APLICACION: &[Paso] = &[
    Paso { medio: Medio::Sosofs, descripcion: "diario APLICANDO" },
    Paso { medio: Medio::Sistema, descripcion: "cada acción del inventario, idempotente, con progreso en el diario" },
    Paso { medio: Medio::Sosofs, descripcion: "diario PROBANDO" },
    Paso { medio: Medio::Esp, descripcion: "registro de arranque PROBANDO" },
];

/// Confirmación: primero la evidencia en sosofs, después la decisión en la ESP.
pub const ORDEN_CONFIRMACION: &[Paso] = &[
    Paso { medio: Medio::Sosofs, descripcion: "diario CONFIRMADO con la evidencia del arranque" },
    Paso { medio: Medio::Esp, descripcion: "registro de arranque CONFIRMADO" },
    Paso { medio: Medio::Sosofs, descripcion: "/etc/soso-release reconciliado con la decisión" },
];

/// Reversión: la petición se hace durable en la ESP antes de tocar nada, para
/// que un reinicio a mitad la continúe en vez de dejar el rootfs mezclado.
/// La reversión automática no necesita el primer paso: su disparador es
/// encontrar PROBANDO en el registro al arrancar.
pub const ORDEN_REVERSION: &[Paso] = &[
    Paso { medio: Medio::Esp, descripcion: "registro de arranque REVERTIR (sólo reversión manual)" },
    Paso { medio: Medio::Sosofs, descripcion: "diario REVIRTIENDO" },
    Paso { medio: Medio::Sistema, descripcion: "restaurar cada acción desde su respaldo, idempotente" },
    Paso { medio: Medio::Esp, descripcion: "kernel anterior restaurado desde SOSOKRN.BIN" },
    Paso { medio: Medio::Sosofs, descripcion: "diario REVERTIDO" },
    Paso { medio: Medio::Esp, descripcion: "registro de arranque REVERTIDO" },
];

/// Espacio que exige una operación antes de empezar a bajar nada.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Necesidad {
    /// Pack y kernel en el área de preparación.
    pub preparacion: u64,
    /// Copias de los ficheros que se van a reemplazar o borrar.
    pub respaldo: u64,
    /// Crecimiento neto del sistema al aplicar (ficheros nuevos o mayores).
    pub crecimiento: u64,
    /// Bytes del kernel nuevo, que van al hueco de la ESP.
    pub kernel: u64,
}

/// Capacidad disponible, tal como la miden el FS y la ESP.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capacidad {
    pub sosofs_libre: u64,
    pub hueco_kernel: u64,
    pub registro_arranque: usize,
    pub meta_kernel: usize,
}

/// Reserva intocable de sosofs: logs nativos (U1: 3 × 4 MiB) más margen para
/// que una recuperación pueda escribir aunque el disco se haya llenado.
pub const RESERVA_LOGS: u64 = 12 * 1024 * 1024;
pub const RESERVA_RECUPERACION: u64 = 8 * 1024 * 1024;

/// Tamaño de `SOSOKRN.MET`. Está aquí y no en `kernel_meta` porque ese módulo
/// no se compila en el kernel; la aserción de abajo impide que se separen.
pub const META_KERNEL_MIN: usize = 512;

#[cfg(feature = "full")]
const _: () = assert!(META_KERNEL_MIN == crate::kernel_meta::KERNEL_META_SIZE);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreflightError {
    SinEspacio { necesita: u64, libre: u64 },
    KernelNoCabe { size: u64, hueco: u64 },
    RegistroNoCabe { size: usize },
}

impl Necesidad {
    /// Bytes de sosofs que hay que tener libres, reservas incluidas.
    pub fn total_sosofs(&self) -> u64 {
        self.preparacion
            .saturating_add(self.respaldo)
            .saturating_add(self.crecimiento)
            .saturating_add(RESERVA_LOGS)
            .saturating_add(RESERVA_RECUPERACION)
    }
}

/// Comprobación previa: sin espacio para preparar **y** deshacer, la operación
/// no empieza. Quedarse sin sitio a mitad es lo que deja una pareja incoherente.
pub fn preflight(n: &Necesidad, c: &Capacidad) -> Result<(), PreflightError> {
    let necesita = n.total_sosofs();
    if necesita > c.sosofs_libre {
        return Err(PreflightError::SinEspacio { necesita, libre: c.sosofs_libre });
    }
    if n.kernel == 0 || n.kernel > c.hueco_kernel {
        return Err(PreflightError::KernelNoCabe { size: n.kernel, hueco: c.hueco_kernel });
    }
    if c.registro_arranque < crate::record::SLOT_SIZE * crate::record::SLOTS {
        return Err(PreflightError::RegistroNoCabe { size: c.registro_arranque });
    }
    if c.meta_kernel < META_KERNEL_MIN {
        return Err(PreflightError::RegistroNoCabe { size: c.meta_kernel });
    }
    Ok(())
}
