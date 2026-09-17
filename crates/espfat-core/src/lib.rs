//! Ficheros 8.3 de la raíz de una ESP (FAT16/FAT32), sin montarla.
//!
//! El kernel de soso no tiene un FAT completo: escribe en ficheros que el
//! empaquetado dejó **pre-creados y contiguos** (`SOSOLOG.TXT`, `SOSOKRN.BIN`,
//! …) sobrescribiendo sus sectores. Esto es esa lógica, separada del disco de
//! arranque para poder usarla también sobre el **destino de una instalación**,
//! que es otro disco y no está montado, y para poder probarla en host.
//!
//! Entrega U2 de `docs/PLAN-ACTUALIZACIONES.md`.

#![cfg_attr(not(feature = "std"), no_std)]

pub const SECTOR: usize = 512;
const ENTRADA: usize = 32;
/// Marca de entrada de directorio borrada.
const BORRADA: u8 = 0xE5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatError {
    /// El medio falló al leer o escribir.
    Io,
    /// Sin firma 0x55AA o BPB incoherente.
    SinBpb,
    /// FAT con sector distinto de 512 u otra variante que no manejamos.
    NoSoportado,
    NoEncontrado,
    TamanoDistinto { esperado: u64, real: u64 },
    /// Los clusters no son consecutivos: sobrescribir por LBA destrozaría lo
    /// que haya en medio.
    Fragmentado,
    CadenaRota,
}

/// Acceso por sectores al volumen. El llamante decide si son LBA absolutos o
/// relativos al inicio de la partición; aquí todo es relativo al volumen.
pub trait Sectores {
    fn leer(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), FatError>;
    fn escribir(&mut self, lba: u64, buf: &[u8]) -> Result<(), FatError>;
}

/// Dónde empiezan los datos de un fichero contiguo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    /// LBA (relativo al volumen) del primer sector de datos.
    pub data_lba: u64,
    pub size: u32,
    pub cluster: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub num_fats: u8,
    pub root_entry_count: u16,
    pub sectors_per_fat: u32,
    pub root_cluster: u32,
    pub is_fat32: bool,
}

impl Params {
    pub fn parse(sec: &[u8]) -> Result<Self, FatError> {
        if sec.len() < SECTOR || sec[510] != 0x55 || sec[511] != 0xAA {
            return Err(FatError::SinBpb);
        }
        let bytes_per_sector = u16::from_le_bytes([sec[11], sec[12]]);
        if bytes_per_sector as usize != SECTOR {
            return Err(FatError::NoSoportado);
        }
        let sectors_per_cluster = sec[13];
        if sectors_per_cluster == 0 {
            return Err(FatError::SinBpb);
        }
        let reserved_sectors = u16::from_le_bytes([sec[14], sec[15]]);
        let num_fats = sec[16];
        let root_entry_count = u16::from_le_bytes([sec[17], sec[18]]);
        let fat16_sectors = u16::from_le_bytes([sec[22], sec[23]]) as u32;
        let total_16 = u16::from_le_bytes([sec[19], sec[20]]) as u32;
        let total_32 = u32::from_le_bytes([sec[32], sec[33], sec[34], sec[35]]);
        let total = if total_16 != 0 { total_16 } else { total_32 };

        let (sectors_per_fat, root_cluster, is_fat32) = if root_entry_count == 0 {
            let spf = u32::from_le_bytes([sec[36], sec[37], sec[38], sec[39]]);
            let root = u32::from_le_bytes([sec[44], sec[45], sec[46], sec[47]]);
            if spf == 0 || root < 2 {
                return Err(FatError::SinBpb);
            }
            (spf, root, true)
        } else {
            if fat16_sectors == 0 {
                return Err(FatError::SinBpb);
            }
            (fat16_sectors, 0, false)
        };
        if num_fats == 0 || total == 0 || reserved_sectors == 0 {
            return Err(FatError::SinBpb);
        }
        Ok(Self {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            root_entry_count,
            sectors_per_fat,
            root_cluster,
            is_fat32,
        })
    }

    fn fin_de_cadena(&self, entrada: u32) -> bool {
        if self.is_fat32 {
            entrada >= 0x0FFF_FFF8
        } else {
            entrada >= 0xFFF8
        }
    }

    fn data_start(&self) -> u64 {
        let base = self.reserved_sectors as u64 + self.num_fats as u64 * self.sectors_per_fat as u64;
        if self.is_fat32 {
            base
        } else {
            base + (self.root_entry_count as u64 * ENTRADA as u64).div_ceil(SECTOR as u64)
        }
    }

    pub fn cluster_to_lba(&self, cluster: u32) -> Result<u64, FatError> {
        if cluster < 2 {
            return Err(FatError::CadenaRota);
        }
        Ok(self.data_start() + (cluster - 2) as u64 * self.sectors_per_cluster as u64)
    }

    /// LBA y desplazamiento de la entrada de `cluster` en la copia `n` de la FAT.
    fn fat_pos(&self, copia: u8, cluster: u32) -> (u64, usize) {
        let ancho = if self.is_fat32 { 4u64 } else { 2 };
        let base = self.reserved_sectors as u64 + copia as u64 * self.sectors_per_fat as u64;
        let off = cluster as u64 * ancho;
        (base + off / SECTOR as u64, (off % SECTOR as u64) as usize)
    }
}

/// Dónde vive una entrada de directorio, para poder reescribirla.
#[derive(Clone, Copy, Debug)]
struct Entrada {
    lba: u64,
    off: usize,
    cluster: u32,
    size: u32,
}

pub struct Volumen<S: Sectores> {
    s: S,
    p: Params,
}

impl<S: Sectores> Volumen<S> {
    pub fn abrir(mut s: S) -> Result<Self, FatError> {
        let mut sec = [0u8; SECTOR];
        s.leer(0, &mut sec)?;
        let p = Params::parse(&sec)?;
        Ok(Self { s, p })
    }

    pub fn params(&self) -> &Params {
        &self.p
    }

    pub fn dispositivo(&mut self) -> &mut S {
        &mut self.s
    }

    /// Busca `NOMBRE.EXT` en la raíz y exige tamaño exacto y clusters
    /// consecutivos: se va a sobrescribir por LBA.
    pub fn localizar(
        &mut self,
        nombre: &[u8; 8],
        ext: &[u8; 3],
        tam: usize,
    ) -> Result<Slot, FatError> {
        let e = self.buscar(nombre, ext)?;
        if e.size as u64 != tam as u64 {
            return Err(FatError::TamanoDistinto {
                esperado: tam as u64,
                real: e.size as u64,
            });
        }
        if !self.contiguo(e.cluster, e.size)? {
            return Err(FatError::Fragmentado);
        }
        Ok(Slot {
            data_lba: self.p.cluster_to_lba(e.cluster)?,
            size: e.size,
            cluster: e.cluster,
        })
    }

    /// Igual, pero sin exigir un tamaño concreto.
    pub fn localizar_cualquiera(
        &mut self,
        nombre: &[u8; 8],
        ext: &[u8; 3],
    ) -> Result<Slot, FatError> {
        let e = self.buscar(nombre, ext)?;
        Ok(Slot {
            data_lba: self.p.cluster_to_lba(e.cluster)?,
            size: e.size,
            cluster: e.cluster,
        })
    }

    pub fn existe(&mut self, nombre: &[u8; 8], ext: &[u8; 3]) -> bool {
        self.buscar(nombre, ext).is_ok()
    }

    /// Borra `NOMBRE.EXT` de verdad: marca la entrada de directorio y libera su
    /// cadena en **todas** las copias de la FAT.
    ///
    /// El orden importa y no es el intuitivo. Se marca primero la entrada y
    /// después se libera la cadena: si el corte llega en medio quedan clusters
    /// perdidos, que es un desperdicio que cualquier `fsck` recoge. Al revés
    /// —liberar antes— quedaría una entrada viva apuntando a clusters ya
    /// libres, y la siguiente escritura del firmware los reutilizaría dejando
    /// dos ficheros enlazados sobre los mismos datos.
    ///
    /// Poner ceros en el contenido **no** es borrar: la entrada seguiría ahí y
    /// `SOSOLOG.TXT` seguiría existiendo en la instalación.
    pub fn borrar(&mut self, nombre: &[u8; 8], ext: &[u8; 3]) -> Result<(), FatError> {
        let e = self.buscar(nombre, ext)?;

        let mut sec = [0u8; SECTOR];
        self.s.leer(e.lba, &mut sec)?;
        sec[e.off] = BORRADA;
        self.s.escribir(e.lba, &sec)?;

        if e.cluster >= 2 {
            self.liberar_cadena(e.cluster)?;
        }
        Ok(())
    }

    fn liberar_cadena(&mut self, inicio: u32) -> Result<(), FatError> {
        let mut cluster = inicio;
        // Cota dura: un FAT corrupto con un ciclo no puede dejarnos girando.
        let max = self.p.sectors_per_fat as u64 * SECTOR as u64 / 2;
        let mut vueltas = 0u64;
        loop {
            let siguiente = self.leer_fat(cluster)?;
            self.escribir_fat(cluster, 0)?;
            if self.p.fin_de_cadena(siguiente) || siguiente < 2 {
                return Ok(());
            }
            cluster = siguiente;
            vueltas += 1;
            if vueltas > max {
                return Err(FatError::CadenaRota);
            }
        }
    }

    fn leer_fat(&mut self, cluster: u32) -> Result<u32, FatError> {
        let (lba, off) = self.p.fat_pos(0, cluster);
        let mut sec = [0u8; SECTOR];
        self.s.leer(lba, &mut sec)?;
        Ok(if self.p.is_fat32 {
            u32::from_le_bytes([sec[off], sec[off + 1], sec[off + 2], sec[off + 3]]) & 0x0FFF_FFFF
        } else {
            u16::from_le_bytes([sec[off], sec[off + 1]]) as u32
        })
    }

    /// Escribe la entrada en **todas** las copias de la FAT: el firmware UEFI
    /// puede leer cualquiera de ellas, y dejarlas discordantes es pedirle a la
    /// siguiente máquina que elija cuál se cree.
    fn escribir_fat(&mut self, cluster: u32, valor: u32) -> Result<(), FatError> {
        for copia in 0..self.p.num_fats {
            let (lba, off) = self.p.fat_pos(copia, cluster);
            let mut sec = [0u8; SECTOR];
            self.s.leer(lba, &mut sec)?;
            if self.p.is_fat32 {
                // Los 4 bits altos son reservados: se conservan.
                let previo =
                    u32::from_le_bytes([sec[off], sec[off + 1], sec[off + 2], sec[off + 3]]);
                let nuevo = (previo & 0xF000_0000) | (valor & 0x0FFF_FFFF);
                sec[off..off + 4].copy_from_slice(&nuevo.to_le_bytes());
            } else {
                sec[off..off + 2].copy_from_slice(&(valor as u16).to_le_bytes());
            }
            self.s.escribir(lba, &sec)?;
        }
        Ok(())
    }

    fn contiguo(&mut self, inicio: u32, size: u32) -> Result<bool, FatError> {
        let por_cluster = self.p.bytes_per_sector as u64 * self.p.sectors_per_cluster as u64;
        let necesarios = (size as u64).div_ceil(por_cluster) as u32;
        if necesarios == 0 {
            return Ok(false);
        }
        let mut cluster = inicio;
        for i in 0..necesarios {
            let siguiente = self.leer_fat(cluster)?;
            if i + 1 < necesarios {
                if siguiente != cluster + 1 {
                    return Ok(false);
                }
                cluster = siguiente;
            } else if !self.p.fin_de_cadena(siguiente) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn buscar(&mut self, nombre: &[u8; 8], ext: &[u8; 3]) -> Result<Entrada, FatError> {
        if self.p.is_fat32 {
            let mut cluster = self.p.root_cluster;
            let mut vueltas = 0u32;
            loop {
                let lba = self.p.cluster_to_lba(cluster)?;
                for c in 0..self.p.sectors_per_cluster as u64 {
                    match self.escanear(lba + c, nombre, ext)? {
                        Escaneo::Encontrada(e) => return Ok(e),
                        Escaneo::Fin => return Err(FatError::NoEncontrado),
                        Escaneo::Sigue => {}
                    }
                }
                let siguiente = self.leer_fat(cluster)?;
                if self.p.fin_de_cadena(siguiente) || siguiente < 2 {
                    return Err(FatError::NoEncontrado);
                }
                cluster = siguiente;
                vueltas += 1;
                if vueltas > 1 << 16 {
                    return Err(FatError::CadenaRota);
                }
            }
        } else {
            let sectores = (self.p.root_entry_count as u64 * ENTRADA as u64).div_ceil(SECTOR as u64);
            let root_lba = self.p.reserved_sectors as u64
                + self.p.num_fats as u64 * self.p.sectors_per_fat as u64;
            for s in 0..sectores {
                match self.escanear(root_lba + s, nombre, ext)? {
                    Escaneo::Encontrada(e) => return Ok(e),
                    Escaneo::Fin => return Err(FatError::NoEncontrado),
                    Escaneo::Sigue => {}
                }
            }
            Err(FatError::NoEncontrado)
        }
    }

    fn escanear(
        &mut self,
        lba: u64,
        nombre: &[u8; 8],
        ext: &[u8; 3],
    ) -> Result<Escaneo, FatError> {
        let mut sec = [0u8; SECTOR];
        self.s.leer(lba, &mut sec)?;
        for (i, ent) in sec.chunks(ENTRADA).enumerate() {
            if ent[0] == 0x00 {
                return Ok(Escaneo::Fin);
            }
            // 0x0F = entrada de nombre largo; se salta, igual que las borradas.
            if ent[0] == BORRADA || ent[11] & 0x0F == 0x0F || ent[11] & 0x08 != 0 {
                continue;
            }
            if &ent[0..8] != nombre || &ent[8..11] != ext {
                continue;
            }
            let lo = u16::from_le_bytes([ent[26], ent[27]]) as u32;
            let hi = u16::from_le_bytes([ent[20], ent[21]]) as u32;
            return Ok(Escaneo::Encontrada(Entrada {
                lba,
                off: i * ENTRADA,
                cluster: (hi << 16) | lo,
                size: u32::from_le_bytes([ent[28], ent[29], ent[30], ent[31]]),
            }));
        }
        Ok(Escaneo::Sigue)
    }
}

enum Escaneo {
    Encontrada(Entrada),
    Sigue,
    Fin,
}
