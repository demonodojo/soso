//! Plataforma de soso: no hay señales POSIX que registrar.

use std::fmt;

use crate::error::Error as CtrlcError;

/// Error de plataforma. `EEXIST` existe porque `error.rs` lo compara fuera de Windows.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// Ya había un manejador.
    EEXIST,
    /// soso no instala un manejador de Ctrl-C.
    Unsupported,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::EEXIST => f.write_str("handler already exists"),
            Error::Unsupported => f.write_str("signals are not supported on soso"),
        }
    }
}

impl std::error::Error for Error {}

/// Señal de plataforma. No se entrega ninguna.
#[derive(Debug)]
pub struct Signal;

/// No registra un manejador. soso no tiene `SIGINT`.
///
/// # Errors
/// Siempre `Error::Unsupported`.
#[allow(unreachable_pub)]
pub unsafe fn init_os_handler(overwrite: bool) -> Result<(), Error> {
    let _ = overwrite;
    Err(Error::Unsupported)
}

/// No espera un Ctrl-C. `init_os_handler` ya ha fallado.
///
/// # Errors
/// Siempre error: no hay señal que esperar.
#[allow(unreachable_pub)]
pub unsafe fn block_ctrl_c() -> Result<(), CtrlcError> {
    Err(CtrlcError::NoSuchSignal(crate::SignalType::Ctrlc))
}
