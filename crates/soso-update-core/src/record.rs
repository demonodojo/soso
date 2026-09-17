//! Marco común de los registros durables de la transacción de actualización.
//!
//! Tanto el diario en sosofs como el registro de arranque en la ESP son texto
//! con la misma envoltura: magic, versión de formato, secuencia monotónica y
//! suma de integridad. No hay commit atómico entre FAT y sosofs (U0 §2 del
//! contrato), así que cada medio guarda **varias ranuras** del mismo tamaño y
//! se escribe por turnos: una escritura cortada sólo puede romper la ranura en
//! curso, y `pick` sigue devolviendo la anterior íntegra.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crc::{Crc, CRC_32_ISCSI};

const CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

/// Tamaño de cada ranura del registro de arranque. Cuatro ranuras caben en los
/// 4 KiB de un fichero 8.3 pre-creado en la ESP.
pub const SLOT_SIZE: usize = 1024;
/// Ranuras del registro de arranque en la ESP.
pub const SLOTS: usize = 4;
/// Copias del diario en sosofs. Ahí el registro es un fichero normal y crece
/// con el inventario, así que se alternan dos ficheros en vez de rellenar.
pub const COPIAS: usize = 2;
/// Longitud del campo `sum=` (CRC32C en hex).
///
/// Es un CRC y no un hash a propósito: este campo protege contra **escrituras
/// cortadas y sectores dañados**, no contra falsificación —quien pueda
/// reescribir la ESP puede reescribir también el registro entero—. Y la
/// diferencia no es teórica: instanciar SHA-256 en el kernel sólo para esto
/// añadía 466 KB, y con eso la imagen BIOS dejaba de arrancar.
pub const SUM_LEN: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// La ranura está a ceros o sólo contiene relleno: nunca se escribió.
    Vacia,
    NoTexto,
    BadMagic,
    /// Formato más nuevo del que entiende este recuperador.
    FormatoDesconocido(u16),
    CampoAusente(&'static str),
    CampoInvalido(&'static str),
    /// Falta `sum=`: escritura cortada antes de cerrar el registro.
    SinSuma,
    /// `sum=` no cuadra con el contenido: escritura cortada o sector dañado.
    SumaIncorrecta,
    Desbordado,
}

/// Registro ya validado: cabecera separada del cuerpo, que interpreta cada módulo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Framed {
    pub formato: u16,
    pub seq: u64,
    pub body: String,
}

impl Framed {
    /// Valor de una línea `clave=valor` del cuerpo.
    pub fn campo(&self, clave: &str) -> Option<&str> {
        for line in self.body.lines() {
            let line = line.trim();
            if let Some(v) = line.strip_prefix(clave) {
                if let Some(v) = v.strip_prefix('=') {
                    return Some(v);
                }
            }
        }
        None
    }

    pub fn requerido(&self, clave: &'static str) -> Result<&str, RecordError> {
        self.campo(clave).ok_or(RecordError::CampoAusente(clave))
    }

    /// Líneas del cuerpo que empiezan por `prefijo` (inventario del diario).
    pub fn lineas<'a>(&'a self, prefijo: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.body
            .lines()
            .map(str::trim)
            .filter(move |l| l.starts_with(prefijo))
    }
}

fn suma(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let c = CRC32C.checksum(data);
    let mut s = String::with_capacity(SUM_LEN);
    for i in (0..8).rev() {
        s.push(HEX[((c >> (i * 4)) & 0xf) as usize] as char);
    }
    s
}

/// Serializa un registro. `body` son líneas sin cabecera. Sin tope de tamaño:
/// el diario de sosofs crece con el inventario.
pub fn frame(magic: &str, formato: u16, seq: u64, body: &str) -> Vec<u8> {
    let mut text = format!("{magic}\nformato={formato}\nseq={seq}\n");
    if !body.is_empty() {
        text.push_str(body);
        if !body.ends_with('\n') {
            text.push('\n');
        }
    }
    let sum = suma(text.as_bytes());
    text.push_str(&format!("sum={sum}\n"));
    text.into_bytes()
}

/// Igual, relleno a una ranura de tamaño fijo (registro de arranque en la ESP,
/// que se escribe sobre un fichero 8.3 pre-creado y no puede crecer).
pub fn frame_slot(
    magic: &str,
    formato: u16,
    seq: u64,
    body: &str,
    slot: usize,
) -> Result<Vec<u8>, RecordError> {
    let mut out = frame(magic, formato, seq, body);
    if out.len() > slot {
        return Err(RecordError::Desbordado);
    }
    out.resize(slot, b'\n');
    Ok(out)
}

/// Valida una ranura: magic, formato soportado, secuencia e integridad.
pub fn parse(magic: &str, formato_max: u16, raw: &[u8]) -> Result<Framed, RecordError> {
    if raw.iter().all(|&b| b == 0 || b == b'\n' || b == 0xff) {
        return Err(RecordError::Vacia);
    }
    let fin = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let text = core::str::from_utf8(&raw[..fin]).map_err(|_| RecordError::NoTexto)?;

    let mut prefijo = 0usize;
    let mut valor: Option<&str> = None;
    let mut off = 0usize;
    for linea in text.split_inclusive('\n') {
        if let Some(v) = linea.trim().strip_prefix("sum=") {
            prefijo = off;
            valor = Some(v);
            break;
        }
        off += linea.len();
    }
    let valor = valor.ok_or(RecordError::SinSuma)?;
    if valor.len() != SUM_LEN || suma(&raw[..prefijo]) != valor {
        return Err(RecordError::SumaIncorrecta);
    }

    let cabecera = &text[..prefijo];
    let mut lineas = cabecera.lines();
    if lineas.next().map(str::trim) != Some(magic) {
        return Err(RecordError::BadMagic);
    }
    let mut formato = None;
    let mut seq = None;
    let mut body = String::new();
    for linea in lineas {
        let l = linea.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(v) = l.strip_prefix("formato=") {
            formato = Some(v.parse::<u16>().map_err(|_| RecordError::CampoInvalido("formato"))?);
        } else if let Some(v) = l.strip_prefix("seq=") {
            seq = Some(v.parse::<u64>().map_err(|_| RecordError::CampoInvalido("seq"))?);
        } else {
            body.push_str(l);
            body.push('\n');
        }
    }
    let formato = formato.ok_or(RecordError::CampoAusente("formato"))?;
    if formato == 0 || formato > formato_max {
        return Err(RecordError::FormatoDesconocido(formato));
    }
    Ok(Framed {
        formato,
        seq: seq.ok_or(RecordError::CampoAusente("seq"))?,
        body,
    })
}

/// Ranura a escribir para la secuencia `seq`: turno rotatorio, nunca encima de
/// la última válida.
pub fn ranura(seq: u64, ranuras: usize) -> usize {
    (seq % ranuras as u64) as usize
}

/// Elige la ranura válida de mayor secuencia. Devuelve su índice.
///
/// Si ninguna vale, distingue «nunca escrito» (`Vacia`) de «roto»: lo primero
/// es un sistema sin transacción, lo segundo exige diagnóstico.
pub fn pick(magic: &str, formato_max: u16, slots: &[&[u8]]) -> Result<(usize, Framed), RecordError> {
    let mut mejor: Option<(usize, Framed)> = None;
    let mut error = RecordError::Vacia;
    for (i, raw) in slots.iter().enumerate() {
        match parse(magic, formato_max, raw) {
            Ok(f) => {
                if mejor.as_ref().is_none_or(|(_, m)| f.seq > m.seq) {
                    mejor = Some((i, f));
                }
            }
            Err(RecordError::Vacia) => {}
            Err(e) => {
                if error == RecordError::Vacia {
                    error = e;
                }
            }
        }
    }
    mejor.ok_or(error)
}
