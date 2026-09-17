//! Ficheros pre-creados en la ESP (FAT16/FAT32) del **disco de arranque**.
//!
//! La lógica FAT vive en `crates/espfat-core`, que es no_std y se prueba en
//! host; aquí sólo está el acceso a sectores del disco live. `soso-install`
//! usa la misma lógica contra el disco **destino**, que es otro volumen y no
//! está montado: por eso el parser está fuera y no duplicado.
//!
//! No sabe crear ficheros ni asignar clusters: encuentra dónde empiezan los
//! datos de un fichero 8.3 de la raíz **que ya existe y es contiguo**, para
//! sobrescribirlo in situ. `xtask package-usb-live` los deja preparados.
//!
//! Lo usan `fatlog` (log de consola), `drvlog` (informe hwscan), `bootreq`
//! (petición de entrada de arranque), `updslot` (huecos OTA) y `modo` (la
//! identidad live/instalado de U0).

use block_dev::BlockError;
use espfat_core::{FatError, Sectores, Volumen};

pub use espfat_core::{Slot, SECTOR};

/// Acceso por sectores a la ESP del disco desde el que arrancamos.
struct EspArranque;

impl Sectores for EspArranque {
    fn leer(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), FatError> {
        read(lba, buf).map_err(|_| FatError::Io)
    }
    fn escribir(&mut self, lba: u64, buf: &[u8]) -> Result<(), FatError> {
        write(lba, buf).map_err(|_| FatError::Io)
    }
}

pub fn read(lba: u64, buf: &mut [u8]) -> Result<(), BlockError> {
    crate::drivers::live_disk::esp_read_sectors(lba, buf)
}

pub fn write(lba: u64, buf: &[u8]) -> Result<(), BlockError> {
    crate::drivers::live_disk::esp_write_sectors(lba, buf)
}

fn volumen() -> Option<Volumen<EspArranque>> {
    Volumen::abrir(EspArranque).ok()
}

/// Busca `NAME.EXT` en la raíz de la ESP y exige que mida exactamente
/// `expect_size` y que sus clusters sean consecutivos: se va a sobrescribir por
/// LBA, así que un fichero fragmentado destrozaría lo que hubiera en medio.
pub fn locate(name: &[u8; 8], ext: &[u8; 3], expect_size: usize) -> Option<Slot> {
    volumen()?.localizar(name, ext, expect_size).ok()
}

/// ¿Existe el fichero, mida lo que mida?
pub fn existe(name: &[u8; 8], ext: &[u8; 3]) -> bool {
    volumen().is_some_and(|mut v| v.existe(name, ext))
}
