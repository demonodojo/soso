//! Descarga durable y reanudable de una release (entrega U4).
//!
//! La regla que manda: **bajar no cambia el sistema activo**. Todo cae en el
//! área de preparación de la operación, verificado por hash antes de contar
//! como hecho; `/bin`, `/lib` y la versión instalada no se tocan hasta que
//! aplica la transacción. Un corte de red o un reinicio a mitad sólo cuestan lo
//! que faltaba: lo ya verificado se conserva.

use alloc::string::String;
use alloc::vec::Vec;

use crate::manifest::{FileEntry, Manifest};
use crate::record::{self, RecordError};
use crate::txn::{Necesidad, TxnId};

/// Tamaño máximo de un trozo en vuelo.
///
/// Acota la RAM del cliente: antes se reservaba el tramo entero, que con un
/// pack de decenas de MiB es memoria que una máquina pequeña no tiene. También
/// acota lo que se pierde al cortarse la red.
pub const TROZO_MAX: u64 = 1024 * 1024;

pub const ETAPA_MAGIC: &str = "SOSOETAPA";
pub const ETAPA_FORMATO: u16 = 1;

/// Registro durable del área de preparación.
///
/// Ata la etapa a **una** release: el identificador es el hash del manifiesto,
/// así que si `latest` cambia entre dos arranques no se mezclan artefactos de
/// dos releases distintas. Una versión no basta; dos builds la comparten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Etapa {
    pub id: TxnId,
    pub version: String,
    pub pack_size: u64,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EtapaError {
    Registro(RecordError),
    IdInvalido,
    CampoInvalido,
}

impl From<RecordError> for EtapaError {
    fn from(e: RecordError) -> Self {
        EtapaError::Registro(e)
    }
}

impl Etapa {
    pub fn nueva(id: TxnId, version: &str, pack_size: u64) -> Self {
        Self { id, version: version.into(), pack_size, seq: 1 }
    }

    pub fn format(&self) -> Vec<u8> {
        use alloc::format;
        let body = format!(
            "id={}\nversion={}\npack_size={}\n",
            self.id.to_hex(),
            self.version,
            self.pack_size
        );
        record::frame(ETAPA_MAGIC, ETAPA_FORMATO, self.seq, &body)
    }

    pub fn parse(raw: &[u8]) -> Result<Self, EtapaError> {
        let f = record::parse(ETAPA_MAGIC, ETAPA_FORMATO, raw)?;
        Ok(Self {
            id: TxnId::from_hex(f.requerido("id")?).ok_or(EtapaError::IdInvalido)?,
            version: f.requerido("version")?.into(),
            pack_size: f
                .requerido("pack_size")?
                .parse()
                .map_err(|_| EtapaError::CampoInvalido)?,
            seq: f.seq,
        })
    }

    /// ¿La etapa guardada sirve para el manifiesto que acabamos de leer?
    ///
    /// Si no, lo que hay bajado es de otra release y **no** se puede reutilizar:
    /// los offsets del pack son de otro fichero.
    pub fn sirve_para(&self, man_bytes: &[u8]) -> bool {
        self.id == TxnId::from_manifest(man_bytes)
    }
}

/// Qué falta por bajar. `ya_listo` responde si el fichero está en la etapa con
/// su tamaño y su hash correctos: sólo eso cuenta como hecho.
pub fn pendientes<F>(man: &Manifest, ya_listo: F) -> Vec<FileEntry>
where
    F: Fn(&FileEntry) -> bool,
{
    pendientes_de(&man.files, ya_listo)
}

/// Igual, sobre una lista cualquiera: al aplicar sólo interesan los ficheros
/// que cambian, no el manifiesto entero.
pub fn pendientes_de<F>(files: &[FileEntry], ya_listo: F) -> Vec<FileEntry>
where
    F: Fn(&FileEntry) -> bool,
{
    files.iter().filter(|f| !ya_listo(f)).cloned().collect()
}

/// Espacio que exige la operación, para `txn::preflight`.
///
/// `tam_actual` da el tamaño del fichero que hay hoy en el sistema, o `None` si
/// no existe: de ahí sale el respaldo (lo que habrá que copiar para poder
/// deshacer) y el crecimiento neto.
pub fn necesidad<F>(
    man: &Manifest,
    cambian: &[FileEntry],
    pendientes: &[FileEntry],
    tam_actual: F,
) -> Necesidad
where
    F: Fn(&str) -> Option<u64>,
{
    let preparacion = pendientes.iter().map(|f| f.size).sum::<u64>() + man.kernel_size;
    let mut respaldo = 0u64;
    let mut crecimiento = 0u64;
    // **`cambian`, no `man.files`.** El respaldo guarda lo que se va a
    // reemplazar, y un fichero que no cambia no se toca. Sumando el manifiesto
    // entero, una release con 121 MB de firmware invariable exigía 167,7 MB
    // libres para bajar 12,6, y `aplicar` se negaba en cualquier instalación
    // con el rootfs normal (384 MiB). El respaldo real ya se hacía sólo sobre
    // los que cambian: era la estimación la que mentía (2026-09-21).
    for f in cambian {
        match tam_actual(&f.path) {
            Some(actual) => {
                respaldo = respaldo.saturating_add(actual);
                crecimiento = crecimiento.saturating_add(f.size.saturating_sub(actual));
            }
            None => crecimiento = crecimiento.saturating_add(f.size),
        }
    }
    Necesidad { preparacion, respaldo, crecimiento, kernel: man.kernel_size }
}

/// Parte `[start, start+len)` en trozos de como mucho `max`.
///
/// Es lo que acota la RAM y lo que se pierde al cortarse la red: sin esto, un
/// corte al 99 % de un tramo de 8 MiB obliga a repetir los 8 MiB.
pub fn trozos(start: u64, len: u64, max: u64) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    if len == 0 || max == 0 {
        return out;
    }
    let mut off = start;
    let fin = start + len;
    while off < fin {
        let n = max.min(fin - off);
        out.push((off, n));
        off += n;
    }
    out
}

/// Bytes que quedan por bajar de una lista de pendientes.
pub fn bytes_pendientes(pendientes: &[FileEntry]) -> u64 {
    pendientes.iter().map(|f| f.size).sum()
}

// ------------------------------------------------- respuestas a un rango

/// Lo que se pidió en una petición de rango, para poder juzgar la respuesta.
#[derive(Clone, Copy, Debug)]
pub struct Peticion {
    pub desde: u64,
    pub hasta: u64,
    /// Bytes que faltan por recibir del tramo entero (no sólo de esta
    /// petición): un servidor que devuelva de más no puede desbordar el plan.
    pub restante: u64,
    /// El tramo empezaba en el byte 0 del artefacto.
    pub desde_el_principio: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FalloRango {
    /// El servidor contestó algo que no sirve (404, 429, 5xx, un redirect que
    /// nadie siguió…). Se guarda el código porque es lo que hay que enseñar.
    Estado(u16),
    /// 206 con el cuerpo vacío: no avanza y repetirlo sería un bucle.
    Vacia,
    /// 206, pero de **otro** tramo. Aceptarlo colocaría los bytes en el sitio
    /// equivocado; lo detectaría después el hash del fichero, pero diciendo
    /// «fichero corrupto» en vez de «tu servidor no respeta Range».
    RangoAjeno { pedido: u64, recibido: u64 },
}

/// Qué hacer con la respuesta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aceptado {
    /// Bytes del cuerpo que se usan (el resto sobra).
    pub usar: usize,
    /// El servidor ignoró el `Range` y mandó el artefacto entero: con esto ya
    /// está todo, no hay que pedir más tramos.
    pub completo: bool,
}

/// Decide qué hacer con lo que contestó el servidor a una petición de rango.
///
/// Está aquí y no en el cliente porque es **política**, no transporte: son las
/// reglas de qué respuesta vale, y se prueban en el host una a una.
pub fn juzgar_rango(
    p: &Peticion,
    status: u16,
    content_range: Option<&str>,
    cuerpo: usize,
) -> Result<Aceptado, FalloRango> {
    match status {
        206 => {
            if cuerpo == 0 {
                return Err(FalloRango::Vacia);
            }
            if let Some(cr) = content_range {
                match parse_content_range(cr) {
                    Some(inicio) if inicio != p.desde => {
                        return Err(FalloRango::RangoAjeno {
                            pedido: p.desde,
                            recibido: inicio,
                        });
                    }
                    // Un `Content-Range` que no se entiende no se usa para
                    // rechazar: lo que se compara después es el hash, y
                    // plantarse por una cabecera rara dejaría sin actualizar a
                    // quien tiene delante un proxy pintoresco.
                    _ => {}
                }
            }
            Ok(Aceptado {
                usar: (cuerpo as u64).min(p.restante) as usize,
                completo: false,
            })
        }
        // 200 sólo vale si el servidor ignoró el `Range` y nos dio el artefacto
        // entero, y eso únicamente sirve cuando pedíamos desde el principio.
        200 if p.desde_el_principio && p.desde == 0 && cuerpo as u64 >= p.restante => Ok(Aceptado {
            usar: p.restante as usize,
            completo: true,
        }),
        otro => Err(FalloRango::Estado(otro)),
    }
}

/// Primer byte que dice `Content-Range: bytes 100-199/12345`.
pub fn parse_content_range(v: &str) -> Option<u64> {
    let resto = v.trim().strip_prefix("bytes")?.trim_start();
    let inicio = resto.split(['-', '/']).next()?.trim();
    inicio.parse().ok()
}
