//! Ficheros 8.3 de la raíz de una ESP (FAT16/FAT32), sin montarla.
//!
//! El kernel de soso no monta la ESP. Sobrescribe por LBA los ficheros 8.3 que
//! ya miden lo justo y son contiguos (`SOSOLOG.TXT`, `SOSOKRN.BIN`, …). Si uno
//! de esos huecos falta, mide otra cosa o está fragmentado, `asegurar_contiguo`
//! lo repone conservando el prefijo: es lo que permite actualizar una máquina
//! ya instalada sin el live. La misma lógica vale sobre el disco de arranque y
//! sobre el destino de una instalación, y se prueba en host.
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
    /// No hay un run libre del tamaño pedido.
    SinEspacio,
    /// La raíz no tiene una entrada libre.
    RaizLlena,
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
    pub total_sectors: u32,
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
            total_sectors: total,
        })
    }

    fn eoc(&self) -> u32 {
        if self.is_fat32 { 0x0FFF_FFFF } else { 0xFFFF }
    }

    fn bytes_por_cluster(&self) -> u32 {
        self.bytes_per_sector as u32 * self.sectors_per_cluster as u32
    }

    fn clusters_para(&self, size: u32) -> u32 {
        let b = self.bytes_por_cluster();
        if size == 0 || b == 0 { 0 } else { size.div_ceil(b) }
    }

    /// Clusters de datos direccionables. El primero es el 2.
    fn clusters_de_datos(&self) -> u32 {
        let data = self.data_start();
        if self.total_sectors as u64 <= data {
            return 0;
        }
        ((self.total_sectors as u64 - data) / self.sectors_per_cluster as u64) as u32
    }

    fn ultimo_cluster(&self) -> u32 {
        self.clusters_de_datos().saturating_add(1)
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

    /// Deja `NOMBRE.EXT` en la raíz con tamaño exacto y clusters consecutivos.
    ///
    /// Si ya está así, no lo toca. Si existe, conserva el prefijo (la copia del
    /// kernel, cuando el hueco se había recortado) y crece in situ si los
    /// clusters siguientes están libres; si no, lo reubica a un run contiguo.
    /// Si no existe, lo crea a ceros. El resto del hueco nuevo queda a ceros,
    /// para que una restauración que mire el último byte distinto no se lleve
    /// basura de detrás.
    pub fn asegurar_contiguo(
        &mut self,
        nombre: &[u8; 8],
        ext: &[u8; 3],
        tam: u32,
    ) -> Result<Slot, FatError> {
        if tam == 0 {
            return Err(FatError::NoSoportado);
        }
        let necesarios = self.p.clusters_para(tam);
        if necesarios == 0 || self.p.ultimo_cluster() < 2 + necesarios - 1 {
            return Err(FatError::SinEspacio);
        }

        if let Ok(e) = self.buscar(nombre, ext) {
            let (n, contiguo) = self.resumen_cadena(e.cluster)?;
            if e.size == tam && contiguo && n == necesarios {
                return self.localizar(nombre, ext, tam as usize);
            }
            if contiguo && n >= necesarios {
                if n > necesarios {
                    self.cortar_cadena(e.cluster, necesarios)?;
                }
                if e.size < tam {
                    self.rellenar(e.cluster, e.size, tam)?;
                }
                self.fijar_entrada(&e, e.cluster, tam)?;
                return self.localizar(nombre, ext, tam as usize);
            }
            if contiguo && n < necesarios && self.siguientes_libres(e.cluster + n, necesarios - n)? {
                self.marcar_cadena(e.cluster, necesarios)?;
                self.rellenar(e.cluster, e.size, tam)?;
                self.fijar_entrada(&e, e.cluster, tam)?;
                return self.localizar(nombre, ext, tam as usize);
            }
            let destino = self.buscar_run(necesarios)?.ok_or(FatError::SinEspacio)?;
            self.marcar_cadena(destino, necesarios)?;
            let copiar = e.size.min(tam);
            if copiar > 0 && e.cluster >= 2 {
                self.copiar(e.cluster, destino, copiar)?;
            }
            if copiar < tam {
                self.rellenar(destino, copiar, tam)?;
            }
            if e.cluster >= 2 {
                self.liberar_cadena(e.cluster)?;
            }
            self.fijar_entrada(&e, destino, tam)?;
            return self.localizar(nombre, ext, tam as usize);
        }

        let destino = self.buscar_run(necesarios)?.ok_or(FatError::SinEspacio)?;
        let (lba, off) = self.hueco_directorio()?;
        self.marcar_cadena(destino, necesarios)?;
        self.rellenar(destino, 0, tam)?;
        self.crear_entrada(lba, off, nombre, ext, destino, tam)?;
        self.localizar(nombre, ext, tam as usize)
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

    fn resumen_cadena(&mut self, inicio: u32) -> Result<(u32, bool), FatError> {
        if inicio < 2 {
            return Err(FatError::CadenaRota);
        }
        let mut c = inicio;
        let mut n = 0u32;
        let mut contiguo = true;
        let tope = self.p.clusters_de_datos().saturating_add(2);
        loop {
            n += 1;
            if n > tope {
                return Err(FatError::CadenaRota);
            }
            let sig = self.leer_fat(c)?;
            if self.p.fin_de_cadena(sig) {
                return Ok((n, contiguo));
            }
            if sig != c + 1 {
                contiguo = false;
            }
            if sig < 2 {
                return Err(FatError::CadenaRota);
            }
            c = sig;
        }
    }

    fn siguientes_libres(&mut self, inicio: u32, n: u32) -> Result<bool, FatError> {
        if n == 0 {
            return Ok(true);
        }
        let mut cache = FatCache::vacia();
        for i in 0..n {
            let c = inicio + i;
            if c < 2 || c > self.p.ultimo_cluster() {
                return Ok(false);
            }
            if self.leer_fat_cache(&mut cache, c)? != 0 {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn buscar_run(&mut self, n: u32) -> Result<Option<u32>, FatError> {
        if n == 0 {
            return Ok(None);
        }
        let ultimo = self.p.ultimo_cluster();
        if ultimo < 2 || n > ultimo - 1 {
            return Ok(None);
        }
        let mut cache = FatCache::vacia();
        let mut c = 2u32;
        loop {
            if c > ultimo || ultimo - c + 1 < n {
                return Ok(None);
            }
            let mut libre = true;
            for i in 0..n {
                if self.leer_fat_cache(&mut cache, c + i)? != 0 {
                    libre = false;
                    c = c.saturating_add(i + 1);
                    break;
                }
            }
            if libre {
                return Ok(Some(c));
            }
        }
    }

    fn marcar_cadena(&mut self, inicio: u32, n: u32) -> Result<(), FatError> {
        if n == 0 {
            return Err(FatError::CadenaRota);
        }
        for i in 0..n {
            let sig = if i + 1 == n { self.p.eoc() } else { inicio + i + 1 };
            self.escribir_fat(inicio + i, sig)?;
        }
        Ok(())
    }

    fn cortar_cadena(&mut self, inicio: u32, quedar: u32) -> Result<(), FatError> {
        if quedar == 0 {
            return Err(FatError::CadenaRota);
        }
        let mut c = inicio;
        for i in 0..quedar {
            let sig = self.leer_fat(c)?;
            if i + 1 == quedar {
                self.escribir_fat(c, self.p.eoc())?;
                if sig >= 2 && !self.p.fin_de_cadena(sig) {
                    self.liberar_cadena(sig)?;
                }
                return Ok(());
            }
            if sig < 2 || self.p.fin_de_cadena(sig) {
                return Err(FatError::CadenaRota);
            }
            c = sig;
        }
        Ok(())
    }

    /// Ceros en un run contiguo que empieza en `cluster`, del byte `desde` al
    /// `hasta` (exclusivo). Los tramos alineados van en lotes: un sector por
    /// llamada haría del hueco de 64 MiB cien mil escrituras en el arranque.
    fn rellenar(&mut self, cluster: u32, desde: u32, hasta: u32) -> Result<(), FatError> {
        if desde >= hasta {
            return Ok(());
        }
        const LOTE: usize = 16 * 1024;
        let ceros = [0u8; LOTE];
        let base = self.p.cluster_to_lba(cluster)?;
        let mut off = desde;
        if off % SECTOR as u32 != 0 {
            let sec_off = (off as usize) % SECTOR;
            let lba = base + off as u64 / SECTOR as u64;
            let n = ((hasta - off) as usize).min(SECTOR - sec_off);
            let mut sec = [0u8; SECTOR];
            self.s.leer(lba, &mut sec)?;
            sec[sec_off..sec_off + n].fill(0);
            self.s.escribir(lba, &sec)?;
            off += n as u32;
        }
        while hasta.saturating_sub(off) >= SECTOR as u32 {
            let n = ((hasta - off) as usize).min(LOTE) / SECTOR * SECTOR;
            self.s.escribir(base + off as u64 / SECTOR as u64, &ceros[..n])?;
            off += n as u32;
        }
        if off < hasta {
            let lba = base + off as u64 / SECTOR as u64;
            let n = (hasta - off) as usize;
            let mut sec = [0u8; SECTOR];
            self.s.leer(lba, &mut sec)?;
            sec[..n].fill(0);
            self.s.escribir(lba, &sec)?;
        }
        Ok(())
    }

    fn copiar(&mut self, origen: u32, destino: u32, nbytes: u32) -> Result<(), FatError> {
        let mut quedan = nbytes;
        let mut src = origen;
        let mut dst_off = 0u32;
        let nsec = self.p.sectors_per_cluster as u64;
        while quedan > 0 {
            let lba = self.p.cluster_to_lba(src)?;
            for s in 0..nsec {
                if quedan == 0 {
                    break;
                }
                let mut sec = [0u8; SECTOR];
                self.s.leer(lba + s, &mut sec)?;
                let n = quedan.min(SECTOR as u32);
                self.escribir_en_run(destino, dst_off, &sec[..n as usize])?;
                dst_off += n;
                quedan -= n;
            }
            if quedan == 0 {
                break;
            }
            let sig = self.leer_fat(src)?;
            if sig < 2 || self.p.fin_de_cadena(sig) {
                return Err(FatError::CadenaRota);
            }
            src = sig;
        }
        Ok(())
    }

    fn escribir_en_run(&mut self, cluster: u32, offset: u32, data: &[u8]) -> Result<(), FatError> {
        let bpc = self.p.bytes_por_cluster();
        let mut escrito = 0usize;
        while escrito < data.len() {
            let pos = offset + escrito as u32;
            let cual = cluster + pos / bpc;
            let dentro = pos % bpc;
            let lba = self.p.cluster_to_lba(cual)? + (dentro / SECTOR as u32) as u64;
            let sec_off = (dentro % SECTOR as u32) as usize;
            let n = (data.len() - escrito).min(SECTOR - sec_off);
            let mut sec = [0u8; SECTOR];
            if sec_off != 0 || n < SECTOR {
                self.s.leer(lba, &mut sec)?;
            }
            sec[sec_off..sec_off + n].copy_from_slice(&data[escrito..escrito + n]);
            self.s.escribir(lba, &sec)?;
            escrito += n;
        }
        Ok(())
    }

    fn fijar_entrada(&mut self, e: &Entrada, cluster: u32, size: u32) -> Result<(), FatError> {
        let mut sec = [0u8; SECTOR];
        self.s.leer(e.lba, &mut sec)?;
        let ent = &mut sec[e.off..e.off + ENTRADA];
        ent[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
        ent[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
        ent[28..32].copy_from_slice(&size.to_le_bytes());
        self.s.escribir(e.lba, &sec)
    }

    fn crear_entrada(
        &mut self,
        lba: u64,
        off: usize,
        nombre: &[u8; 8],
        ext: &[u8; 3],
        cluster: u32,
        size: u32,
    ) -> Result<(), FatError> {
        let mut sec = [0u8; SECTOR];
        self.s.leer(lba, &mut sec)?;
        let ent = &mut sec[off..off + ENTRADA];
        ent.fill(0);
        ent[0..8].copy_from_slice(nombre);
        ent[8..11].copy_from_slice(ext);
        ent[11] = 0x20;
        ent[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
        ent[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
        ent[28..32].copy_from_slice(&size.to_le_bytes());
        self.s.escribir(lba, &sec)
    }

    fn hueco_directorio(&mut self) -> Result<(u64, usize), FatError> {
        if self.p.is_fat32 {
            let mut cluster = self.p.root_cluster;
            let mut vueltas = 0u32;
            loop {
                let lba = self.p.cluster_to_lba(cluster)?;
                for c in 0..self.p.sectors_per_cluster as u64 {
                    if let Some(off) = self.entrada_libre(lba + c)? {
                        return Ok((lba + c, off));
                    }
                }
                let siguiente = self.leer_fat(cluster)?;
                if self.p.fin_de_cadena(siguiente) || siguiente < 2 {
                    return Err(FatError::RaizLlena);
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
                if let Some(off) = self.entrada_libre(root_lba + s)? {
                    return Ok((root_lba + s, off));
                }
            }
            Err(FatError::RaizLlena)
        }
    }

    /// Primera entrada borrada o el fin del directorio (`0x00`) en este sector.
    /// `None` si el sector está lleno de entradas vivas y hay que seguir.
    fn entrada_libre(&mut self, lba: u64) -> Result<Option<usize>, FatError> {
        let mut sec = [0u8; SECTOR];
        self.s.leer(lba, &mut sec)?;
        for (i, ent) in sec.chunks(ENTRADA).enumerate() {
            if ent[0] == 0x00 || ent[0] == BORRADA {
                return Ok(Some(i * ENTRADA));
            }
        }
        Ok(None)
    }

    fn leer_fat_cache(&mut self, cache: &mut FatCache, cluster: u32) -> Result<u32, FatError> {
        let (lba, off) = self.p.fat_pos(0, cluster);
        if !cache.valida || cache.lba != lba {
            self.s.leer(lba, &mut cache.sec)?;
            cache.lba = lba;
            cache.valida = true;
        }
        Ok(if self.p.is_fat32 {
            u32::from_le_bytes([
                cache.sec[off],
                cache.sec[off + 1],
                cache.sec[off + 2],
                cache.sec[off + 3],
            ]) & 0x0FFF_FFFF
        } else {
            u16::from_le_bytes([cache.sec[off], cache.sec[off + 1]]) as u32
        })
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

struct FatCache {
    lba: u64,
    sec: [u8; SECTOR],
    valida: bool,
}

impl FatCache {
    fn vacia() -> Self {
        Self { lba: 0, sec: [0u8; SECTOR], valida: false }
    }
}

enum Escaneo {
    Encontrada(Entrada),
    Sigue,
    Fin,
}
