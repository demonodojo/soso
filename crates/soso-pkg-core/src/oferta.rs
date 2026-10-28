//! Decisión instalar / actualizar / rechazar según versión.

use core::cmp::Ordering;

use soso_update_core::{parse_semver, semver_cmp, SemVer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccionInstalacion {
    Instalar,
    Actualizar,
    YaInstalado,
    RequiereForzar,
}

/// `local`: versión ya instalada (`None` si no hay).
pub fn decidir(
    remota: &SemVer,
    local: Option<&SemVer>,
    forzar: bool,
) -> AccionInstalacion {
    let Some(loc) = local else {
        return AccionInstalacion::Instalar;
    };
    match semver_cmp(remota, loc) {
        Ordering::Greater => AccionInstalacion::Actualizar,
        Ordering::Less => {
            if forzar {
                AccionInstalacion::Actualizar
            } else {
                AccionInstalacion::RequiereForzar
            }
        }
        Ordering::Equal => {
            if forzar {
                AccionInstalacion::Actualizar
            } else {
                AccionInstalacion::YaInstalado
            }
        }
    }
}

pub fn parse_version(s: &str) -> Option<SemVer> {
    parse_semver(s)
}
