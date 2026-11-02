//! Carga de objetos compartidos (T80). Ver `sys/pal/soso/dl.rs`.

#![unstable(feature = "soso_dl", issue = "none")]

use crate::path::Path;
use crate::string::String;

/// Carga `ruta` y devuelve la dirección del símbolo `simbolo`.
///
/// # Safety
/// Ejecuta el código del objeto (su `init_array`) y devuelve una dirección que
/// el llamante interpreta con el tipo que quiera.
pub unsafe fn cargar_simbolo(ruta: &Path, simbolo: &str) -> Result<usize, String> {
    let id = crate::sys::dl::cargar(ruta)?;
    crate::sys::dl::simbolo(id, simbolo)
}
