//! Resolución de rutas relativas al cwd del proceso.

use alloc::string::String;
use soso_abi as abi;

pub const PATH_MAX: usize = 256;

/// Une `cwd` con `path` (si es relativa) y normaliza `.` / `..`.
///
/// **Una sola reserva.** La versión anterior hacía cinco —el `cwd.clone()` del
/// llamante, la concatenación intermedia, el `Vec` de componentes, el `join` y
/// la cadena final—, y en soso eso no es un detalle: el asignador del kernel
/// audita su tabla de bloques vivos en cada reserva, así que **cada `alloc`
/// cuesta ~243 000 ciclos**
/// ([N-013](../../../../docs/self-improvement/native/N-013.md)). Esto lo paga
/// **todas** las llamadas al sistema que llevan una ruta, no sólo `stat`.
///
/// La semántica no cambia, incluido el detalle de que una ruta cuya forma
/// **sin normalizar** pasa de `PATH_MAX` se rechaza aunque al normalizarla
/// quedara corta: por eso el largo se comprueba contra la longitud que
/// *tendría* la concatenación, que se calcula sin construirla.
pub fn abs_path(cwd: &str, path: &str) -> Result<String, i64> {
    if path.len() > PATH_MAX {
        return Err(-abi::ENAMETOOLONG);
    }
    let absoluta = path.starts_with('/');
    // Lo que habría medido la concatenación intermedia de antes.
    let largo_crudo = if absoluta {
        path.len()
    } else if cwd == "/" {
        1 + path.len()
    } else {
        cwd.len() + 1 + path.len()
    };
    if largo_crudo > PATH_MAX {
        return Err(-abi::ENAMETOOLONG);
    }

    let mut out = String::with_capacity(largo_crudo + 1);
    if !absoluta {
        for comp in cwd.split('/') {
            empujar(&mut out, comp);
        }
    }
    for comp in path.split('/') {
        empujar(&mut out, comp);
    }
    if out.is_empty() {
        out.push('/');
    }
    if out.len() > PATH_MAX {
        return Err(-abi::ENAMETOOLONG);
    }
    Ok(out)
}

/// Añade un componente ya separado, aplicando `.` y `..`.
///
/// `out` está siempre o vacío o en forma `/a/b`: nunca acaba en `/`. Así
/// `..` es quitar desde la última barra, y subir desde la raíz **no hace
/// nada** — que es la contención que impide salirse de `/` con `../..`.
fn empujar(out: &mut String, comp: &str) {
    match comp {
        "" | "." => {}
        ".." => {
            if let Some(i) = out.rfind('/') {
                out.truncate(i);
            }
        }
        name => {
            out.push('/');
            out.push_str(name);
        }
    }
}
