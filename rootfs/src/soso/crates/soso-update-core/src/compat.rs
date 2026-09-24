//! Contrato de compatibilidad de una release.
//!
//! Un paquete declara para qué sistema sirve; el cliente comprueba que este
//! equipo lo cumple **antes** de bajar nada. Sin esto, una release con otra
//! ABI, otro formato de FS o sin el driver del disco de arranque se instala
//! igual y sólo se descubre al reiniciar, que es el peor momento posible.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Versión del contrato de transacción que implementa este recuperador.
/// Una release que exija más de esto no se puede aplicar aquí: hace falta la
/// release puente de U6.
pub const RECUPERADOR_VERSION: u32 = 1;
/// Versión del shim que entiende el registro de arranque de U0.
pub const SHIM_VERSION: u32 = 1;
/// Formato del sistema de ficheros que monta este kernel.
pub const FS_FORMATO: &str = "sosofs1";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compat {
    pub arch: String,
    /// Perfil de drivers con el que se construyó (`live-usb`, …).
    pub perfil: String,
    /// Drivers que la release **trae** compilados en su kernel.
    pub drivers: Vec<String>,
    /// Versión de la ABI de syscalls.
    pub abi: u32,
    pub fs: String,
    pub min_shim: u32,
    pub min_recuperador: u32,
}

/// Lo que este equipo ofrece, tal como lo conoce el cliente en ejecución.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Equipo {
    pub arch: String,
    pub abi: u32,
    pub fs: String,
    pub shim: u32,
    pub recuperador: u32,
    /// Drivers que esta máquina **necesita** que la release traiga: sin ellos
    /// no vuelve a arrancar (el del disco de arranque, el de la red de la OTA).
    /// No es el inventario de lo que hay: es la lista de lo imprescindible.
    pub drivers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompatError {
    /// El manifiesto no declara compatibilidad: no se acepta a ciegas.
    NoDeclarada,
    Arch { espera: String, hay: String },
    Abi { espera: u32, hay: u32 },
    Fs { espera: String, hay: String },
    /// El shim o el recuperador instalados son anteriores al mínimo: hace
    /// falta la transición de U6, no una sobrescritura de ficheros.
    ShimAntiguo { min: u32, hay: u32 },
    RecuperadorAntiguo { min: u32, hay: u32 },
    /// La release no trae un driver imprescindible para esta máquina.
    DriverAusente(String),
    CampoInvalido(&'static str),
}

impl Compat {
    pub fn format(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("arch={}\n", self.arch));
        s.push_str(&format!("perfil={}\n", self.perfil));
        if !self.drivers.is_empty() {
            s.push_str(&format!("drivers={}\n", self.drivers.join(",")));
        }
        s.push_str(&format!("abi={}\n", self.abi));
        s.push_str(&format!("fs={}\n", self.fs));
        s.push_str(&format!("min_shim={}\n", self.min_shim));
        s.push_str(&format!("min_recuperador={}\n", self.min_recuperador));
        s
    }

    /// Lee las líneas de compatibilidad de un manifiesto. Devuelve `None` si
    /// el manifiesto no declara ninguna (formato anterior a U3).
    pub fn parse(text: &str) -> Option<Result<Self, CompatError>> {
        let mut c = Compat::default();
        let mut visto = false;
        for linea in text.lines() {
            let l = linea.trim();
            let Some((clave, valor)) = l.split_once('=') else {
                continue;
            };
            let num = |campo: &'static str, v: &str| -> Result<u32, CompatError> {
                v.parse().map_err(|_| CompatError::CampoInvalido(campo))
            };
            match clave {
                "arch" => { c.arch = valor.into(); visto = true; }
                "perfil" => { c.perfil = valor.into(); visto = true; }
                "drivers" => {
                    c.drivers = valor
                        .split(',')
                        .map(str::trim)
                        .filter(|d| !d.is_empty())
                        .map(String::from)
                        .collect();
                    visto = true;
                }
                "abi" => { c.abi = match num("abi", valor) { Ok(v) => v, Err(e) => return Some(Err(e)) }; visto = true; }
                "fs" => { c.fs = valor.into(); visto = true; }
                "min_shim" => { c.min_shim = match num("min_shim", valor) { Ok(v) => v, Err(e) => return Some(Err(e)) }; visto = true; }
                "min_recuperador" => { c.min_recuperador = match num("min_recuperador", valor) { Ok(v) => v, Err(e) => return Some(Err(e)) }; visto = true; }
                _ => {}
            }
        }
        visto.then_some(Ok(c))
    }

    /// Comprueba este equipo contra la release. Orden fijo: lo que obliga a
    /// una transición (shim/recuperador) se informa antes que un driver suelto.
    pub fn check(&self, eq: &Equipo) -> Result<(), CompatError> {
        if !self.arch.is_empty() && self.arch != eq.arch {
            return Err(CompatError::Arch { espera: self.arch.clone(), hay: eq.arch.clone() });
        }
        if self.abi != eq.abi {
            return Err(CompatError::Abi { espera: self.abi, hay: eq.abi });
        }
        if !self.fs.is_empty() && self.fs != eq.fs {
            return Err(CompatError::Fs { espera: self.fs.clone(), hay: eq.fs.clone() });
        }
        if self.min_shim > eq.shim {
            return Err(CompatError::ShimAntiguo { min: self.min_shim, hay: eq.shim });
        }
        if self.min_recuperador > eq.recuperador {
            return Err(CompatError::RecuperadorAntiguo {
                min: self.min_recuperador,
                hay: eq.recuperador,
            });
        }
        // El sentido importa y es fácil invertirlo: se comprueba que la release
        // trae **todo lo que la máquina necesita**, no que la máquina tenga
        // todo lo que la release trae. Al revés, cualquier release con un
        // driver de más se rechazaba.
        for necesario in &eq.drivers {
            if !self.drivers.iter().any(|d| d == necesario) {
                return Err(CompatError::DriverAusente(necesario.clone()));
            }
        }
        Ok(())
    }
}

/// Comprobación del cliente: una release sin declaración no se aplica.
pub fn exigir(compat: Option<&Compat>, eq: &Equipo) -> Result<(), CompatError> {
    match compat {
        None => Err(CompatError::NoDeclarada),
        Some(c) => c.check(eq),
    }
}
