//! Lo poco que se le pide a git, y siempre de forma opcional.
//!
//! La base se describe por su inventario de archivos, no por git: si git no
//! está —como dentro de soso—, la captura sigue siendo exacta y solo pierde la
//! etiqueta legible (commit, rama, estado). Aquí también vive `git apply`, que
//! solo usan los casos de repo para aplicar la solución de referencia.

use soso_improve_core::captura::Git;
use soso_improve_core::entorno::{Orden, Procesos};
use soso_improve_core::{Error, Resultado};

use crate::sistema::Host;

/// git atado a un árbol concreto: `--git-dir` corta el descubrimiento hacia
/// arriba. Sin esto, un git lanzado en un directorio que no es repositorio
/// encuentra el que lo contiene y actúa sobre él — así se desacopló una vez el
/// HEAD del checkout real.
fn argv_git(arbol: &str, resto: &[&str]) -> Vec<String> {
    let mut argv = vec![
        "git".to_string(),
        "-C".to_string(),
        arbol.to_string(),
        "--git-dir".to_string(),
        format!("{}/.git", arbol.trim_end_matches('/')),
        "--work-tree".to_string(),
        arbol.to_string(),
        "--no-optional-locks".to_string(),
        "-c".to_string(),
        "core.quotepath=false".to_string(),
    ];
    argv.extend(resto.iter().map(|s| s.to_string()));
    argv
}

fn correr(arbol: &str, resto: &[&str]) -> Option<String> {
    let orden = Orden {
        argv: argv_git(arbol, resto),
        cwd: arbol.to_string(),
        stdin: Vec::new(),
        timeout_s: Some(60),
        entorno: Vec::new(),
    };
    match Host.ejecutar(&orden) {
        Ok(s) if s.ok() => Some(String::from_utf8_lossy(&s.stdout).into_owned()),
        _ => None,
    }
}

/// Describe la base con git si se puede. Nunca escribe nada en el repositorio.
pub fn describir(arbol: &str) -> Git {
    if !std::path::Path::new(arbol).join(".git").exists() {
        return Git {
            disponible: false,
            nota: Some("el árbol no es un repositorio git; la base se describe solo por su inventario".to_string()),
            ..Default::default()
        };
    }
    let commit = correr(arbol, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string());
    if commit.is_none() {
        return Git {
            disponible: false,
            nota: Some("hay .git pero git no respondió".to_string()),
            ..Default::default()
        };
    }
    Git {
        disponible: true,
        commit,
        rama: correr(arbol, &["rev-parse", "--abbrev-ref", "HEAD"]).map(|s| s.trim().to_string()),
        asunto: correr(arbol, &["log", "-1", "--format=%s"]).map(|s| s.trim().to_string()),
        estado: correr(arbol, &["status", "--porcelain=v1", "--untracked-files=all"]),
        nota: None,
    }
}

/// Aplica o revierte un parche sobre un árbol. `revertir` usa `git apply -R`.
pub fn aplicar_parche(arbol: &str, parche: &str, revertir: bool) -> Resultado<()> {
    let mut resto = vec!["apply", "--whitespace=nowarn"];
    if revertir {
        resto.push("-R");
    }
    // Comprobar antes de tocar nada: si el parche no aplica, el árbol se queda
    // como estaba y se dice por qué.
    let mut comprobar = resto.clone();
    comprobar.push("--check");
    comprobar.push(parche);
    let orden = Orden {
        argv: argv_git(arbol, &comprobar),
        cwd: arbol.to_string(),
        stdin: Vec::new(),
        timeout_s: Some(120),
        entorno: Vec::new(),
    };
    let salida = Host.ejecutar(&orden)?;
    if !salida.ok() {
        return Err(Error::entorno(format!(
            "el parche no aplica: {}",
            String::from_utf8_lossy(&salida.stderr).trim()
        )));
    }
    resto.push(parche);
    let orden = Orden {
        argv: argv_git(arbol, &resto),
        cwd: arbol.to_string(),
        stdin: Vec::new(),
        timeout_s: Some(120),
        entorno: Vec::new(),
    };
    let salida = Host.ejecutar(&orden)?;
    if !salida.ok() {
        return Err(Error::entorno(format!(
            "git apply falló: {}",
            String::from_utf8_lossy(&salida.stderr).trim()
        )));
    }
    Ok(())
}
