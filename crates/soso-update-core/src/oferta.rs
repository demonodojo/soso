//! Qué hacer con la release remota frente a la instalada.
//!
//! El número de versión no basta. Una release puede republicarse sin subir el
//! número, y una placa puede llevar un build de desarrollo flasheado desde el
//! árbol con el mismo número que la última publicada. En el segundo caso la
//! publicada es **más vieja** aunque coincida la versión: instalarla sin
//! preguntar deshace el trabajo que se está probando.
//!
//! `comprobar` y `aplicar` deciden con esta misma función, para que lo que
//! anuncia el primero sea lo que hace el segundo.

use core::cmp::Ordering;

use crate::semver::{cmp, SemVer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Oferta {
    /// Versión mayor que la instalada.
    Nueva,
    /// Misma versión y mismo build, y algo instalado no coincide con el
    /// manifiesto (un kernel reflasheado, un fichero tocado): se repone.
    Reparar,
    /// Misma versión, otro build publicado un día posterior al instalado:
    /// release republicada sin subir el número.
    Republicada,
    /// Lo instalado ya es lo que describe el manifiesto.
    AlDia,
    /// Misma versión y otro build sin fecha posterior. No se puede saber cuál
    /// es más nuevo: los builds son hashes de git y la fecha sólo tiene días.
    BuildDistinto,
    /// Versión menor que la instalada.
    Antigua,
}

impl Oferta {
    /// `aplicar` la instala sin `--forzar`.
    pub fn se_aplica_sola(self) -> bool {
        matches!(self, Self::Nueva | Self::Reparar | Self::Republicada)
    }
}

/// Release instalada (`/etc/soso-release`) o remota (manifiesto).
#[derive(Clone, Copy, Debug)]
pub struct Lado<'a> {
    pub version: SemVer,
    pub build: &'a str,
    /// `AAAA-MM-DD`, como lo escribe `xtask::version::build_date`.
    pub fecha: &'a str,
}

/// `hay_trabajo`: algún fichero o el kernel difieren de lo instalado.
pub fn decidir(remota: &Lado, local: &Lado, hay_trabajo: bool) -> Oferta {
    match cmp(&remota.version, &local.version) {
        Ordering::Greater => return Oferta::Nueva,
        Ordering::Less => return Oferta::Antigua,
        Ordering::Equal => {}
    }
    if !hay_trabajo {
        return Oferta::AlDia;
    }
    if !local.build.is_empty() && remota.build == local.build {
        return Oferta::Reparar;
    }
    if fecha_valida(remota.fecha) && fecha_valida(local.fecha) && remota.fecha > local.fecha {
        return Oferta::Republicada;
    }
    Oferta::BuildDistinto
}

/// `AAAA-MM-DD`: con ese formato, comparar las cadenas es comparar las fechas.
fn fecha_valida(f: &str) -> bool {
    let b = f.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}
