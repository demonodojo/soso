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

/// Un paquete con destino de sistema y el propio sistema (que trae su versión
/// del mismo programa, anotada en su índice local) pueden querer el mismo
/// fichero. Gana el más nuevo: `true` si el del sistema lo es. Con versiones
/// iguales, o si alguna no se entiende, manda el paquete, que es lo que el
/// usuario pidió.
pub fn manda_el_sistema(instalada: &str, del_sistema: Option<&str>) -> bool {
    let (Some(paquete), Some(sistema)) = (
        parse_semver(instalada),
        del_sistema.and_then(parse_semver),
    ) else {
        return false;
    };
    semver_cmp(&sistema, &paquete) == Ordering::Greater
}

#[cfg(test)]
mod tests {
    use super::manda_el_sistema;

    #[test]
    fn gana_la_version_mas_nueva() {
        assert!(manda_el_sistema("0.1.0", Some("0.4.1")));
        assert!(!manda_el_sistema("0.5.0", Some("0.4.1")));
        assert!(!manda_el_sistema("0.4.1", Some("0.4.1")));
        assert!(!manda_el_sistema("0.1.0", None));
        assert!(!manda_el_sistema("0.1.0", Some("rara")));
    }
}
