//! De dónde se baja una release: canales, configuración y precedencia.
//!
//! Entrega U3 de `docs/PLAN-ACTUALIZACIONES.md`. Está aquí, y no en el cliente,
//! porque es lógica pura con reglas fáciles de romper sin enterarse: la URL que
//! sale de aquí decide qué sistema operativo se instala en la máquina.

use alloc::format;
use alloc::string::{String, ToString};

/// Repositorio por defecto.
pub const REPO_BASE: &str = "https://github.com/demonodojo/soso/releases";

/// Canal de actualización para toolchain de desarrollo (Hito 3f).
pub const CANAL_DEV: &str = "dev";
/// Canal estable por defecto.
pub const CANAL_STABLE: &str = "stable";

/// `/etc/actualiza.conf`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conf {
    /// URL base explícita (espejo propio, servidor local…).
    pub url: Option<String>,
    pub canal: Option<String>,
}

impl Conf {
    pub fn parse(text: &str) -> Self {
        let mut c = Conf::default();
        for linea in text.lines() {
            let l = linea.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            // Por el primer `=`, recortando los dos lados: `channel = dev` es
            // lo que cualquiera escribe, y con `strip_prefix` no hacía nada —
            // en silencio, que es lo peor que puede hacer un fichero de
            // configuración.
            let Some((clave, valor)) = l.split_once('=') else {
                continue;
            };
            let valor = valor.trim();
            if valor.is_empty() {
                continue;
            }
            match clave.trim() {
                "url" => c.url = Some(valor.to_string()),
                "channel" | "canal" => c.canal = Some(valor.to_string()),
                _ => {}
            }
        }
        c
    }
}

/// De dónde se sirve la release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origen {
    /// Directorio del propio disco: ni red ni canal.
    Local(String),
    /// URL base; los artefactos cuelgan de ella (`<base>/manifest.txt`).
    Remoto(String),
}

impl Origen {
    /// URL o ruta de un artefacto concreto.
    pub fn artefacto(&self, nombre: &str) -> String {
        match self {
            Origen::Local(dir) => format!("{}/{nombre}", dir.trim_end_matches('/')),
            Origen::Remoto(base) => format!("{}/{nombre}", base.trim_end_matches('/')),
        }
    }
    pub fn es_local(&self) -> bool {
        matches!(self, Origen::Local(_))
    }
}

/// URL base de un canal.
///
/// `stable` usa `releases/latest/download`, que GitHub redirige a la última
/// publicada. `dev` es una **etiqueta**, así que su forma es
/// `releases/download/dev`: la que había antes añadía un `/download` de más y
/// pedía `…/download/dev/download/manifest.txt`, que no existe.
pub fn url_de_canal(canal: &str) -> String {
    match canal {
        CANAL_DEV => format!("{REPO_BASE}/download/{CANAL_DEV}"),
        CANAL_STABLE => format!("{REPO_BASE}/latest/download"),
        // Cualquier otro nombre se trata como etiqueta publicada.
        otro => format!("{REPO_BASE}/download/{otro}"),
    }
}

/// Resuelve el origen **una sola vez** por comando.
///
/// Precedencia, de más a menos fuerte:
///
/// 1. `--local DIR` — ni red ni canal.
/// 2. `--channel X` en la línea de órdenes.
/// 3. `url=` de `/etc/actualiza.conf` (espejo propio).
/// 4. `channel=` de `/etc/actualiza.conf`.
/// 5. `stable`.
///
/// Que el `--channel` de la orden gane al `url=` del fichero es deliberado:
/// pedir un canal a mano y que te sirvan otro sitio sin decir nada es peor
/// sorpresa que ignorar el espejo configurado en esa única ejecución.
pub fn resolver(local: Option<&str>, canal_cli: Option<&str>, conf: &Conf) -> Origen {
    if let Some(dir) = local {
        return Origen::Local(dir.to_string());
    }
    if let Some(c) = canal_cli {
        return Origen::Remoto(url_de_canal(c.trim()));
    }
    if let Some(u) = &conf.url {
        return Origen::Remoto(u.clone());
    }
    let canal = conf.canal.as_deref().unwrap_or(CANAL_STABLE);
    Origen::Remoto(url_de_canal(canal.trim()))
}

/// Explica en una línea por qué se eligió ese origen, para que `comprobar` y
/// `aplicar` puedan decirlo antes de tocar nada.
pub fn motivo(local: Option<&str>, canal_cli: Option<&str>, conf: &Conf) -> &'static str {
    if local.is_some() {
        "--local"
    } else if canal_cli.is_some() {
        "--channel"
    } else if conf.url.is_some() {
        "url= de /etc/actualiza.conf"
    } else if conf.canal.is_some() {
        "channel= de /etc/actualiza.conf"
    } else {
        "canal stable por defecto"
    }
}
