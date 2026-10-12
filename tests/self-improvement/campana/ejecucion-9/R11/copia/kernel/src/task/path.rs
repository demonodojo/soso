//! Resolución de rutas relativas al cwd del proceso.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use soso_abi as abi;

pub const PATH_MAX: usize = 256;

/// Une `cwd` con `path` (si es relativa) y normaliza `.` / `..`.
pub fn abs_path(cwd: &str, path: &str) -> Result<String, i64> {
    if path.len() > PATH_MAX {
        return Err(-abi::ENAMETOOLONG);
    }
    let raw = if path.starts_with('/') {
        String::from(path)
    } else if cwd == "/" {
        format!("/{path}")
    } else {
        format!("{cwd}/{path}")
    };
    normalize(&raw)
}

fn normalize(path: &str) -> Result<String, i64> {
    if path.len() > PATH_MAX {
        return Err(-abi::ENAMETOOLONG);
    }
    let mut parts: Vec<&str> = Vec::new();
    for comp in path.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            name => parts.push(name),
        }
    }
    let out = if parts.is_empty() {
        String::from("/")
    } else {
        let mut s = String::with_capacity(parts.len() * 8 + 1);
        s.push('/');
        s.push_str(&parts.join("/"));
        s
    };
    if out.len() > PATH_MAX {
        return Err(-abi::ENAMETOOLONG);
    }
    Ok(out)
}
