//! Raíz de una ESP llena: el kernel tiene que **ampliarla**, no rendirse.
//!
//! En placa (HP, 2026-11-15) la ESP no pudo reponer `SOSOKRN.MET`:
//! `updslot: no pude reponer SOSOKRN.MET (RaizLlena)`. El directorio raíz de
//! FAT32 es una cadena de clústeres y este port devolvía `RaizLlena` al llegar
//! al final, igual que hace Linux antes de `fat_add_cluster`.
//!
//! Los volúmenes se fabrican a mano y con clúster de **un sector** (16
//! entradas), que es la geometría real del stick del HP.

use espfat_core::{FatError, Sectores, Volumen, SECTOR};

struct Disco {
    datos: Vec<u8>,
}

impl Disco {
    fn nuevo(sectores: usize) -> Self {
        Self { datos: vec![0u8; sectores * SECTOR] }
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
        let off = lba as usize * SECTOR;
        buf.copy_from_slice(&self.datos[off..off + buf.len()]);
        Ok(())
    }
    fn escribir(&mut self, lba: u64, buf: &[u8]) -> Result<(), FatError> {
        let off = lba as usize * SECTOR;
        self.datos[off..off + buf.len()].copy_from_slice(buf);
        Ok(())
    }
}

const RESERVADOS: u64 = 32;
const FATS: u64 = 2;
const SPF: u64 = 8;
const EOC: u32 = 0x0FFF_FFFF;

fn raiz_lba() -> u64 {
    RESERVADOS + FATS * SPF
}

fn cluster_lba(cluster: u32) -> u64 {
    raiz_lba() + (cluster as u64 - 2)
}

fn fat_lba(copia: u64) -> u64 {
    RESERVADOS + copia * SPF
}

fn poner_fat(d: &mut Disco, cluster: u32, valor: u32) {
    for copia in 0..FATS {
        let off = cluster as u64 * 4;
        let lba = fat_lba(copia) + off / SECTOR as u64;
        let pos = (off % SECTOR as u64) as usize;
        let sec = d.sector_mut(lba);
        sec[pos..pos + 4].copy_from_slice(&valor.to_le_bytes());
    }
}

fn leer_fat(d: &Disco, copia: u64, cluster: u32) -> u32 {
    let off = cluster as u64 * 4;
    let lba = fat_lba(copia) + off / SECTOR as u64;
    let pos = (off % SECTOR as u64) as usize;
    let sec = d.sector(lba);
    u32::from_le_bytes([sec[pos], sec[pos + 1], sec[pos + 2], sec[pos + 3]]) & 0x0FFF_FFFF
}

/// FAT32 con clúster de 1 sector y la raíz en el clúster 2.
fn volumen(total_sectores: u32) -> Disco {
    let mut d = Disco::nuevo(total_sectores as usize);
    {
        let sec = d.sector_mut(0);
        sec[11..13].copy_from_slice(&512u16.to_le_bytes());
        sec[13] = 1; // un sector por clúster → 16 entradas de directorio
        sec[14..16].copy_from_slice(&(RESERVADOS as u16).to_le_bytes());
        sec[16] = FATS as u8;
        sec[17..19].copy_from_slice(&0u16.to_le_bytes()); // raíz por cadena (FAT32)
        sec[32..36].copy_from_slice(&total_sectores.to_le_bytes());
        sec[36..40].copy_from_slice(&(SPF as u32).to_le_bytes());
        sec[44..48].copy_from_slice(&2u32.to_le_bytes());
        sec[510] = 0x55;
        sec[511] = 0xAA;
    }
    poner_fat(&mut d, 0, 0x0FFF_FFF8);
    poner_fat(&mut d, 1, EOC);
    poner_fat(&mut d, 2, EOC);
    d
}

/// Escribe `n` entradas 8.3 vivas seguidas desde `lba`, como los ficheros que
/// ya trae un stick: la entrada solo tiene que estar ocupada.
fn llenar_entradas(d: &mut Disco, lba: u64, n: usize, desde: u8) {
    let sec = d.sector_mut(lba);
    for i in 0..n {
        let off = i * 32;
        let nombre = [b'R', b'0' + (desde + i as u8) % 10, b'X', b'Y', b'Z', b'0', b'1', b'2'];
        sec[off..off + 8].copy_from_slice(&nombre);
        sec[off + 8..off + 11].copy_from_slice(b"BIN");
        sec[off + 11] = 0x20;
        sec[off + 26..off + 28].copy_from_slice(&0u16.to_le_bytes());
        sec[off + 28..off + 32].copy_from_slice(&0u32.to_le_bytes());
    }
}

fn n8(s: &str) -> [u8; 8] {
    let mut o = [b' '; 8];
    o[..s.len()].copy_from_slice(s.as_bytes());
    o
}

#[test]
fn una_raiz_llena_se_amplia_con_un_cluster_a_ceros() {
    let mut d = volumen(4096);
    // 16 entradas vivas = el único sector de la raíz, lleno.
    llenar_entradas(&mut d, raiz_lba(), 16, 0);

    {
        let mut v = Volumen::abrir(&mut d).unwrap();
        v.asegurar_contiguo(&n8("SOSOKRN"), b"MET", 512)
            .expect("la raíz llena debe ampliarse, no fallar con RaizLlena");
    }

    // La cadena de la raíz ahora es 2 → 3, en las dos copias de la FAT.
    for copia in 0..FATS {
        assert_eq!(leer_fat(&d, copia, 2), 3, "copia {copia}: 2 no apunta a 3");
        assert_eq!(leer_fat(&d, copia, 3), EOC, "copia {copia}: 3 no cierra");
    }

    // La entrada nace en la entrada 0 del clúster nuevo y el contenido va al
    // primer clúster que siga libre después de ampliar la raíz (el 3 ya es
    // de la raíz).
    let mut v = Volumen::abrir(&mut d).unwrap();
    let slot = v.localizar(&n8("SOSOKRN"), b"MET", 512).unwrap();
    assert_eq!(slot.cluster, 4);

    // El resto del clúster sigue a ceros: nada de entradas basura, que es lo
    // que dejaba la raíz «llena» sin estarlo para Linux.
    let sec = d.sector(cluster_lba(3));
    for i in 1..16 {
        assert_eq!(sec[i * 32], 0, "entrada {i} del clúster nuevo no está a cero");
    }
}

/// Volumen mínimo: reservados 8, 2 FAT de 1 sector, 54 clústeres de datos
/// (2..55). Los ficheros empiezan a partir del 2, así que la raíz es el 2 y
/// su primer sector coincide con la entrada de directorio.
fn volumen_minimo() -> Disco {
    let total = 64u32;
    let mut d = Disco::nuevo(total as usize);
    {
        let sec = d.sector_mut(0);
        sec[11..13].copy_from_slice(&512u16.to_le_bytes());
        sec[13] = 1;
        sec[14..16].copy_from_slice(&8u16.to_le_bytes());
        sec[16] = 2;
        sec[32..36].copy_from_slice(&total.to_le_bytes());
        sec[36..40].copy_from_slice(&1u32.to_le_bytes());
        sec[44..48].copy_from_slice(&2u32.to_le_bytes());
        sec[510] = 0x55;
        sec[511] = 0xAA;
    }
    d
}

/// Primer sector de datos del volumen mínimo: 8 reservados + 2 FAT de 1.
fn cluster_lba_min(cluster: u32) -> u64 {
    10 + (cluster as u64 - 2)
}

fn fat_minimo(d: &mut Disco, cluster: u32, valor: u32) {
    for copia in 0..2u64 {
        let off = cluster as u64 * 4;
        let lba = 8 + copia + off / SECTOR as u64;
        let pos = (off % SECTOR as u64) as usize;
        d.sector_mut(lba)[pos..pos + 4].copy_from_slice(&valor.to_le_bytes());
    }
}

fn leer_fat_minimo(d: &Disco, cluster: u32) -> u32 {
    let off = cluster as u64 * 4;
    let lba = 8 + off / SECTOR as u64;
    let pos = (off % SECTOR as u64) as usize;
    let sec = d.sector(lba);
    u32::from_le_bytes([sec[pos], sec[pos + 1], sec[pos + 2], sec[pos + 3]])
}

#[test]
fn sin_clusters_libres_sigue_siendo_sin_espacio() {
    let mut d = volumen_minimo();
    fat_minimo(&mut d, 0, 0x0FFF_FFF8);
    fat_minimo(&mut d, 1, EOC);
    // Del 2 al 55, todos ocupados: ni raíz ampliable ni datos que escribir.
    for c in 2..=55u32 {
        fat_minimo(&mut d, c, EOC);
    }
    llenar_entradas(&mut d, cluster_lba_min(2), 16, 0);

    let mut v = Volumen::abrir(&mut d).unwrap();
    assert_eq!(
        v.asegurar_contiguo(&n8("SOSOKRN"), b"MET", 512),
        Err(FatError::SinEspacio)
    );
}

#[test]
fn la_raiz_ampliada_y_el_fichero_no_comparten_cluster() {
    // Dos clústeres libres (54 y 55) y la raíz llena: ampliarla consume uno y
    // el fichero tiene que quedarse el otro. Si se elige primero el run de
    // datos, los dos acaban en el mismo clúster (directorio y fichero
    // enlazados) y el volumen queda corrupto sin que nadie proteste.
    let mut d = volumen_minimo();
    fat_minimo(&mut d, 0, 0x0FFF_FFF8);
    fat_minimo(&mut d, 1, EOC);
    fat_minimo(&mut d, 2, EOC);
    for c in 3..54u32 {
        fat_minimo(&mut d, c, EOC);
    }
    llenar_entradas(&mut d, cluster_lba_min(2), 16, 0);

    {
        let mut v = Volumen::abrir(&mut d).unwrap();
        v.asegurar_contiguo(&n8("SOSOKRN"), b"MET", 512).unwrap();
    }

    assert_eq!(leer_fat_minimo(&d, 2), 54, "la raíz se amplía con el 54");
    let mut v = Volumen::abrir(&mut d).unwrap();
    let slot = v.localizar(&n8("SOSOKRN"), b"MET", 512).unwrap();
    assert_eq!(slot.cluster, 55, "el fichero no puede caer en el clúster de la raíz");
}
