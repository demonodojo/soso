//! U2: localizar y **borrar de verdad** un fichero 8.3 de la raíz de una ESP.
//!
//! Los volúmenes se fabrican aquí a mano, sin `mkfs.vfat` ni loop devices: el
//! banco tiene que correr en cualquier sitio, y controlar el disco byte a byte
//! es lo que permite comprobar la FAT y el directorio por separado.

use espfat_core::{FatError, Sectores, Volumen, SECTOR};

/// Disco en memoria.
struct Disco {
    datos: Vec<u8>,
    fallar_desde: Option<u64>,
}

impl Disco {
    fn nuevo(sectores: usize) -> Self {
        Self { datos: vec![0u8; sectores * SECTOR], fallar_desde: None }
    }
    fn sector(&self, lba: u64) -> &[u8] {
        &self.datos[lba as usize * SECTOR..(lba as usize + 1) * SECTOR]
    }
    fn sector_mut(&mut self, lba: u64) -> &mut [u8] {
        &mut self.datos[lba as usize * SECTOR..(lba as usize + 1) * SECTOR]
    }
}

impl Sectores for &mut Disco {
    fn leer(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), FatError> {
        if self.fallar_desde.is_some_and(|f| lba >= f) {
            return Err(FatError::Io);
        }
        let off = lba as usize * SECTOR;
        buf.copy_from_slice(&self.datos[off..off + buf.len()]);
        Ok(())
    }
    fn escribir(&mut self, lba: u64, buf: &[u8]) -> Result<(), FatError> {
        if self.fallar_desde.is_some_and(|f| lba >= f) {
            return Err(FatError::Io);
        }
        let off = lba as usize * SECTOR;
        self.datos[off..off + buf.len()].copy_from_slice(buf);
        Ok(())
    }
}

struct Formato {
    fat32: bool,
    reservados: u16,
    fats: u8,
    sectores_por_fat: u32,
    entradas_raiz: u16,
    sectores_por_cluster: u8,
}

impl Formato {
    fn fat32() -> Self {
        Self { fat32: true, reservados: 32, fats: 2, sectores_por_fat: 8, entradas_raiz: 0, sectores_por_cluster: 1 }
    }
    fn fat16() -> Self {
        Self { fat32: false, reservados: 1, fats: 2, sectores_por_fat: 8, entradas_raiz: 512, sectores_por_cluster: 1 }
    }
    fn fat_lba(&self, copia: u8) -> u64 {
        self.reservados as u64 + copia as u64 * self.sectores_por_fat as u64
    }
    fn raiz_lba(&self) -> u64 {
        self.reservados as u64 + self.fats as u64 * self.sectores_por_fat as u64
    }
    fn datos_lba(&self) -> u64 {
        if self.fat32 {
            self.raiz_lba()
        } else {
            self.raiz_lba() + (self.entradas_raiz as u64 * 32).div_ceil(SECTOR as u64)
        }
    }
    fn cluster_lba(&self, cluster: u32) -> u64 {
        self.datos_lba() + (cluster as u64 - 2) * self.sectores_por_cluster as u64
    }
}

/// Un fichero del volumen de prueba.
struct Fichero {
    nombre: [u8; 8],
    ext: [u8; 3],
    size: u32,
    relleno: u8,
    /// Cluster inicial; si `fragmentado`, la cadena salta en vez de seguir.
    fragmentado: bool,
}

fn n8(s: &str) -> [u8; 8] {
    let mut o = [b' '; 8];
    o[..s.len()].copy_from_slice(s.as_bytes());
    o
}
fn n3(s: &str) -> [u8; 3] {
    let mut o = [b' '; 3];
    o[..s.len()].copy_from_slice(s.as_bytes());
    o
}

fn fichero(nombre: &str, ext: &str, size: u32) -> Fichero {
    Fichero { nombre: n8(nombre), ext: n3(ext), size, relleno: b'A', fragmentado: false }
}

/// Fabrica un volumen con los ficheros dados en la raíz.
fn volumen(f: &Formato, ficheros: &[Fichero]) -> Disco {
    let mut d = Disco::nuevo(4096);
    let total: u32 = 4096;

    // ── BPB ──
    {
        let sec = d.sector_mut(0);
        sec[11..13].copy_from_slice(&512u16.to_le_bytes());
        sec[13] = f.sectores_por_cluster;
        sec[14..16].copy_from_slice(&f.reservados.to_le_bytes());
        sec[16] = f.fats;
        sec[17..19].copy_from_slice(&f.entradas_raiz.to_le_bytes());
        sec[32..36].copy_from_slice(&total.to_le_bytes());
        if f.fat32 {
            sec[36..40].copy_from_slice(&f.sectores_por_fat.to_le_bytes());
            sec[44..48].copy_from_slice(&2u32.to_le_bytes()); // raíz en cluster 2
        } else {
            sec[22..24].copy_from_slice(&(f.sectores_por_fat as u16).to_le_bytes());
        }
        sec[510] = 0x55;
        sec[511] = 0xAA;
    }

    let eoc: u32 = if f.fat32 { 0x0FFF_FFFF } else { 0xFFFF };
    let poner_fat = |d: &mut Disco, cluster: u32, valor: u32| {
        let ancho = if f.fat32 { 4u64 } else { 2 };
        for copia in 0..f.fats {
            let off = cluster as u64 * ancho;
            let lba = f.fat_lba(copia) + off / SECTOR as u64;
            let pos = (off % SECTOR as u64) as usize;
            let sec = d.sector_mut(lba);
            if f.fat32 {
                sec[pos..pos + 4].copy_from_slice(&valor.to_le_bytes());
            } else {
                sec[pos..pos + 2].copy_from_slice(&(valor as u16).to_le_bytes());
            }
        }
    };

    poner_fat(&mut d, 0, if f.fat32 { 0x0FFF_FFF8 } else { 0xFFF8 });
    poner_fat(&mut d, 1, eoc);

    // Cluster 2 = directorio raíz en FAT32; en FAT16 la raíz es fija.
    let mut siguiente_cluster = if f.fat32 {
        poner_fat(&mut d, 2, eoc);
        3
    } else {
        2
    };

    let raiz = if f.fat32 { f.cluster_lba(2) } else { f.raiz_lba() };
    let por_cluster = SECTOR as u32 * f.sectores_por_cluster as u32;

    for (i, fich) in ficheros.iter().enumerate() {
        let clusters = fich.size.div_ceil(por_cluster).max(1);
        let inicio = siguiente_cluster;
        for c in 0..clusters {
            let actual = if fich.fragmentado && c > 0 {
                inicio + c * 2 // deja huecos: la cadena no es consecutiva
            } else {
                inicio + c
            };
            let siguiente = if c + 1 == clusters {
                eoc
            } else if fich.fragmentado {
                inicio + (c + 1) * 2
            } else {
                actual + 1
            };
            poner_fat(&mut d, actual, siguiente);
            for s in 0..f.sectores_por_cluster as u64 {
                let lba = f.cluster_lba(actual) + s;
                d.sector_mut(lba).fill(fich.relleno);
            }
        }
        siguiente_cluster = inicio + clusters * if fich.fragmentado { 2 } else { 1 };

        // Entrada de directorio
        let ent_lba = raiz + (i as u64 * 32) / SECTOR as u64;
        let ent_off = (i * 32) % SECTOR;
        let sec = d.sector_mut(ent_lba);
        sec[ent_off..ent_off + 8].copy_from_slice(&fich.nombre);
        sec[ent_off + 8..ent_off + 11].copy_from_slice(&fich.ext);
        sec[ent_off + 11] = 0x20; // archivo
        sec[ent_off + 20..ent_off + 22].copy_from_slice(&((inicio >> 16) as u16).to_le_bytes());
        sec[ent_off + 26..ent_off + 28].copy_from_slice(&(inicio as u16).to_le_bytes());
        sec[ent_off + 28..ent_off + 32].copy_from_slice(&fich.size.to_le_bytes());
    }
    d
}

/// Valor crudo de la FAT en una copia concreta, para comprobar que se
/// actualizan **todas**.
fn fat_en(d: &Disco, f: &Formato, copia: u8, cluster: u32) -> u32 {
    let ancho = if f.fat32 { 4u64 } else { 2 };
    let off = cluster as u64 * ancho;
    let lba = f.fat_lba(copia) + off / SECTOR as u64;
    let pos = (off % SECTOR as u64) as usize;
    let sec = d.sector(lba);
    if f.fat32 {
        u32::from_le_bytes([sec[pos], sec[pos + 1], sec[pos + 2], sec[pos + 3]]) & 0x0FFF_FFFF
    } else {
        u16::from_le_bytes([sec[pos], sec[pos + 1]]) as u32
    }
}

// ── Localizar ────────────────────────────────────────────────────────────

#[test]
fn localiza_por_nombre_y_tamano() {
    for f in [Formato::fat32(), Formato::fat16()] {
        let mut d = volumen(&f, &[fichero("SOSOLOG", "TXT", 2048)]);
        let mut v = Volumen::abrir(&mut d).unwrap();
        let slot = v.localizar(&n8("SOSOLOG"), &n3("TXT"), 2048).unwrap();
        assert_eq!(slot.size, 2048);
        assert_eq!(slot.data_lba, f.cluster_lba(slot.cluster));
    }
}

#[test]
fn un_tamano_distinto_no_se_acepta() {
    // El hueco se sobrescribe por LBA: si mide otra cosa, no es el que
    // reservó el empaquetado y escribir ahí pisaría lo que haya detrás.
    let f = Formato::fat32();
    let mut d = volumen(&f, &[fichero("SOSOLOG", "TXT", 2048)]);
    let mut v = Volumen::abrir(&mut d).unwrap();
    assert_eq!(
        v.localizar(&n8("SOSOLOG"), &n3("TXT"), 4096),
        Err(FatError::TamanoDistinto { esperado: 4096, real: 2048 })
    );
}

#[test]
fn un_fichero_fragmentado_se_rechaza() {
    let f = Formato::fat32();
    let mut fich = fichero("SOSOKRN", "BIN", 4096);
    fich.fragmentado = true;
    let mut d = volumen(&f, &[fich]);
    let mut v = Volumen::abrir(&mut d).unwrap();
    assert_eq!(
        v.localizar(&n8("SOSOKRN"), &n3("BIN"), 4096),
        Err(FatError::Fragmentado)
    );
}

#[test]
fn sin_bpb_no_se_abre() {
    let mut d = Disco::nuevo(64);
    assert!(matches!(Volumen::abrir(&mut d).err(), Some(FatError::SinBpb)));
}

// ── Borrar ───────────────────────────────────────────────────────────────

#[test]
fn borrar_quita_la_entrada_y_libera_la_cadena() {
    for f in [Formato::fat32(), Formato::fat16()] {
        let mut d = volumen(
            &f,
            &[fichero("SOSOLOG", "TXT", 2048), fichero("SOSODRV", "TXT", 1024)],
        );
        let cluster = {
            let mut v = Volumen::abrir(&mut d).unwrap();
            v.localizar(&n8("SOSOLOG"), &n3("TXT"), 2048).unwrap().cluster
        };

        let mut v = Volumen::abrir(&mut d).unwrap();
        v.borrar(&n8("SOSOLOG"), &n3("TXT")).unwrap();
        assert!(!v.existe(&n8("SOSOLOG"), &n3("TXT")), "sigue estando");
        assert_eq!(
            v.localizar(&n8("SOSOLOG"), &n3("TXT"), 2048),
            Err(FatError::NoEncontrado)
        );
        // Sus cuatro clusters quedan libres, en las dos copias de la FAT.
        for c in cluster..cluster + 4 {
            for copia in 0..f.fats {
                assert_eq!(fat_en(&d, &f, copia, c), 0, "cluster {c} copia {copia}");
            }
        }
        // Y el vecino no se toca.
        let mut v = Volumen::abrir(&mut d).unwrap();
        assert!(v.existe(&n8("SOSODRV"), &n3("TXT")));
        assert_ne!(fat_en(&d, &f, 0, cluster + 4), 0, "liberó de más");
    }
}

#[test]
fn poner_ceros_en_el_contenido_no_es_borrar() {
    // Es el error que el plan U2 señala: dejar el fichero a ceros deja la
    // entrada viva, y la instalación seguiría teniendo su SOSOLOG.TXT.
    let f = Formato::fat32();
    let mut d = volumen(&f, &[fichero("SOSOLOG", "TXT", 2048)]);
    let slot = {
        let mut v = Volumen::abrir(&mut d).unwrap();
        v.localizar(&n8("SOSOLOG"), &n3("TXT"), 2048).unwrap()
    };
    for s in 0..4 {
        d.sector_mut(slot.data_lba + s).fill(0);
    }
    let mut v = Volumen::abrir(&mut d).unwrap();
    assert!(
        v.existe(&n8("SOSOLOG"), &n3("TXT")),
        "a ceros el fichero sigue existiendo: hay que borrar la entrada"
    );
}

#[test]
fn borrar_tambien_vale_para_un_fichero_fragmentado() {
    // `localizar` lo rechaza porque no se puede sobrescribir por LBA, pero
    // borrarlo tiene que funcionar igual: la cadena se recorre, no se asume.
    let f = Formato::fat32();
    let mut fich = fichero("SOSOLOG", "TXT", 4096);
    fich.fragmentado = true;
    let mut d = volumen(&f, &[fich]);
    let mut v = Volumen::abrir(&mut d).unwrap();
    v.borrar(&n8("SOSOLOG"), &n3("TXT")).unwrap();
    assert!(!v.existe(&n8("SOSOLOG"), &n3("TXT")));
    for c in [3u32, 5, 7] {
        assert_eq!(fat_en(&d, &f, 0, c), 0, "cluster {c} sigue ocupado");
    }
}

#[test]
fn borrar_lo_que_no_existe_no_rompe_nada() {
    let f = Formato::fat32();
    let mut d = volumen(&f, &[fichero("SOSOLOG", "TXT", 1024)]);
    let antes = d.datos.clone();
    let mut v = Volumen::abrir(&mut d).unwrap();
    assert_eq!(v.borrar(&n8("NOHAY"), &n3("TXT")), Err(FatError::NoEncontrado));
    assert_eq!(d.datos, antes, "un borrado fallido no debe tocar el disco");
}

#[test]
fn una_cadena_ciclica_no_cuelga() {
    let f = Formato::fat32();
    let mut d = volumen(&f, &[fichero("SOSOLOG", "TXT", 2048)]);
    // Sabotaje: el último cluster de la cadena apunta al primero.
    {
        let cluster = {
            let mut v = Volumen::abrir(&mut d).unwrap();
            v.localizar(&n8("SOSOLOG"), &n3("TXT"), 2048).unwrap().cluster
        };
        let off = (cluster as u64 + 3) * 4;
        let lba = f.fat_lba(0) + off / SECTOR as u64;
        let pos = (off % SECTOR as u64) as usize;
        d.sector_mut(lba)[pos..pos + 4].copy_from_slice(&cluster.to_le_bytes());
    }
    let mut v = Volumen::abrir(&mut d).unwrap();
    // Termina: o libera el ciclo, o lo declara roto. Lo que no puede es girar.
    let _ = v.borrar(&n8("SOSOLOG"), &n3("TXT"));
    assert!(!v.existe(&n8("SOSOLOG"), &n3("TXT")));
}

#[test]
fn un_fallo_de_escritura_se_propaga() {
    let f = Formato::fat32();
    let mut d = volumen(&f, &[fichero("SOSOLOG", "TXT", 2048)]);
    d.fallar_desde = Some(f.raiz_lba());
    let mut v = Volumen::abrir(&mut d).unwrap();
    assert_eq!(v.borrar(&n8("SOSOLOG"), &n3("TXT")), Err(FatError::Io));
}
